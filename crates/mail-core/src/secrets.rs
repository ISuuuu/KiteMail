//! Credential storage backed by Windows Credential Manager via `keyring`.
//!
//! All keyring operations run serially on a dedicated background thread so a
//! save followed immediately by a read or delete is never reordered.

use std::path::Path;
use std::sync::mpsc;
use std::sync::OnceLock;

use sha2::{Digest, Sha256};

const DEFAULT_SERVICE_NAME: &str = "KiteMail.Windows";

/// 根据配置目录和自定义隔离标志推导 Windows 凭据管理器命名空间。
/// 安装版与便携版在同一台电脑上共享 `KiteMail.Windows` 命名空间，仅在显式指定自定义 `KITEMAIL_DATA_DIR` 时附加目录指纹隔离。
pub fn resolve_service_name_for(data_dir: &Path, isolated: bool) -> String {
    if !isolated {
        return DEFAULT_SERVICE_NAME.to_string();
    }
    let resolved = std::fs::canonicalize(data_dir).unwrap_or_else(|_| data_dir.to_path_buf());
    let raw = resolved.to_string_lossy();
    let trimmed = raw
        .strip_prefix(r"\\?\")
        .unwrap_or(&raw)
        .to_ascii_lowercase();
    let digest = Sha256::digest(trimmed.as_bytes());
    let hex12: String = digest.iter().take(6).map(|b| format!("{b:02x}")).collect();
    format!("{DEFAULT_SERVICE_NAME}.Portable.{hex12}")
}

fn current_service_name() -> String {
    if cfg!(test) {
        return format!("{DEFAULT_SERVICE_NAME}.Test.{}", std::process::id());
    }
    let has_env_override = std::env::var_os("KITEMAIL_DATA_DIR").is_some_and(|v| !v.is_empty());
    resolve_service_name_for(&crate::config::data_dir(), has_env_override)
}

type KeyringJob = Box<dyn FnOnce() + Send>;

fn keyring_sender() -> Option<&'static mpsc::Sender<KeyringJob>> {
    static SENDER: OnceLock<Option<mpsc::Sender<KeyringJob>>> = OnceLock::new();
    SENDER
        .get_or_init(|| {
            let (tx, rx) = mpsc::channel::<KeyringJob>();
            std::thread::Builder::new()
                .name("kitemail-win-keyring".to_string())
                .spawn(move || {
                    for job in rx {
                        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
                    }
                })
                .ok()
                .map(|_| tx)
        })
        .as_ref()
}

fn run_on_keyring_thread<T: Send + 'static>(
    job: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let Some(sender) = keyring_sender() else {
        return Err("无法启动系统凭据访问线程".to_string());
    };
    let (tx, rx) = mpsc::sync_channel(1);
    sender
        .send(Box::new(move || {
            let _ = tx.send(job());
        }))
        .map_err(|_| "系统凭据访问线程已停止".to_string())?;
    rx.recv()
        .map_err(|_| "系统凭据访问未返回结果".to_string())?
}

fn imap_entry_name(account_id: u32) -> String {
    format!("account-{account_id}-imap")
}

fn smtp_entry_name(account_id: u32) -> String {
    format!("account-{account_id}-smtp")
}

fn oauth_refresh_entry_name(account_id: u32) -> String {
    format!("account-{account_id}-oauth-refresh")
}

/// 规范化用于 Windows 凭据键名的邮箱地址（去首尾空白并转小写）。
pub fn normalize_credential_email(email: &str) -> String {
    email.trim().to_ascii_lowercase()
}

fn email_imap_entry_name(email: &str) -> Option<String> {
    let norm = normalize_credential_email(email);
    (!norm.is_empty()).then(|| format!("email-{norm}-imap"))
}

fn email_smtp_entry_name(email: &str) -> Option<String> {
    let norm = normalize_credential_email(email);
    (!norm.is_empty()).then(|| format!("email-{norm}-smtp"))
}

fn email_oauth_refresh_entry_name(email: &str) -> Option<String> {
    let norm = normalize_credential_email(email);
    (!norm.is_empty()).then(|| format!("email-{norm}-oauth-refresh"))
}

fn read_entry_optional(
    service_name: &str,
    key: &str,
    kind: &str,
    read_err_prefix: &str,
) -> Result<Option<String>, String> {
    let entry = keyring::Entry::new(service_name, key)
        .map_err(|e| format!("初始化 {kind} 凭据项失败: {e}"))?;
    match entry.get_password() {
        Ok(val) => Ok(Some(val)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("{read_err_prefix}: {e}")),
    }
}

fn write_entry(
    service_name: &str,
    key: &str,
    secret: &str,
    kind: &str,
    write_err_prefix: &str,
) -> Result<(), String> {
    let entry = keyring::Entry::new(service_name, key)
        .map_err(|e| format!("初始化 {kind} 凭据项失败: {e}"))?;
    entry
        .set_password(secret)
        .map_err(|e| format!("{write_err_prefix}: {e}"))
}

fn delete_entry(service_name: &str, key: &str) {
    if let Ok(entry) = keyring::Entry::new(service_name, key) {
        let _ = entry.delete_credential();
    }
}

/// Secrets associated with one mail account.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct AccountSecrets {
    pub imap_password: String,
    pub smtp_password: Option<String>,
}

/// 按邮箱地址优先（为空时回退账号 ID）保存 IMAP（及可选独立 SMTP）凭据到 Windows 凭据管理器。
pub fn save_account_secrets_for(
    account_id: u32,
    email: &str,
    secrets: AccountSecrets,
) -> Result<(), String> {
    let service_name = current_service_name();
    let imap_key = email_imap_entry_name(email).unwrap_or_else(|| imap_entry_name(account_id));
    let smtp_key = email_smtp_entry_name(email).unwrap_or_else(|| smtp_entry_name(account_id));
    run_on_keyring_thread(move || {
        write_entry(
            &service_name,
            &imap_key,
            &secrets.imap_password,
            "IMAP",
            "保存 IMAP 授权码失败",
        )?;
        match secrets.smtp_password {
            Some(pwd) if !pwd.is_empty() => {
                write_entry(
                    &service_name,
                    &smtp_key,
                    &pwd,
                    "SMTP",
                    "保存 SMTP 授权码失败",
                )?;
            }
            _ => delete_entry(&service_name, &smtp_key),
        }
        Ok(())
    })
}

/// Detect orphaned keys without migrating, modifying, or exposing their values.
pub fn has_account_credentials(account_id: u32, email: &str) -> Result<bool, String> {
    let mut keys = vec![
        imap_entry_name(account_id),
        smtp_entry_name(account_id),
        oauth_refresh_entry_name(account_id),
    ];
    keys.extend(
        [
            email_imap_entry_name(email),
            email_smtp_entry_name(email),
            email_oauth_refresh_entry_name(email),
        ]
        .into_iter()
        .flatten(),
    );
    keys.push(bound_key(email, "password"));
    keys.push(bound_key(email, "oauth"));
    let service_name = current_service_name();
    run_on_keyring_thread(move || {
        for key in keys {
            if read_entry_optional(&service_name, &key, "账号", "读取账号凭据失败")?.is_some()
            {
                return Ok(true);
            }
        }
        Ok(false)
    })
}

pub fn has_legacy_account_credentials(account_id: u32) -> Result<bool, String> {
    let keys = [
        imap_entry_name(account_id),
        smtp_entry_name(account_id),
        oauth_refresh_entry_name(account_id),
    ];
    let service_name = current_service_name();
    run_on_keyring_thread(move || {
        for key in keys {
            if read_entry_optional(&service_name, &key, "账号", "读取账号凭据失败")?.is_some()
            {
                return Ok(true);
            }
        }
        Ok(false)
    })
}

fn credential_config_dir() -> std::path::PathBuf {
    if cfg!(test) {
        std::env::temp_dir().join(format!("kitemail-credential-test-{}", std::process::id()))
    } else {
        crate::config::data_dir()
    }
}

fn endpoint_fingerprint(account: &crate::config::AccountConfig) -> Result<String, String> {
    let fields = (
        normalize_credential_email(&account.email),
        &account.auth_backend,
        crate::config::normalized_oauth_client_id(account),
        account.imap_host.trim().to_ascii_lowercase(),
        account.imap_port,
        &account.imap_security,
        account.imap_username.trim(),
        account.smtp_host.trim().to_ascii_lowercase(),
        account.smtp_port,
        &account.smtp_security,
        account.smtp_username.trim(),
        account.smtp_separate_credentials,
    );
    let encoded = serde_json::to_vec(&fields).map_err(|e| format!("生成凭据绑定失败: {e}"))?;
    Ok(format!("{:x}", Sha256::digest(encoded)))
}

fn legacy_v1_endpoint_fingerprint(
    account: &crate::config::AccountConfig,
) -> Result<String, String> {
    let fields = (
        normalize_credential_email(&account.email),
        &account.auth_backend,
        &account.oauth_client_id,
        account.imap_host.to_ascii_lowercase(),
        account.imap_port,
        &account.imap_security,
        &account.imap_username,
        account.smtp_host.to_ascii_lowercase(),
        account.smtp_port,
        &account.smtp_security,
        &account.smtp_username,
        account.smtp_separate_credentials,
    );
    let encoded = serde_json::to_vec(&fields).map_err(|e| format!("生成凭据绑定失败: {e}"))?;
    Ok(format!("{:x}", Sha256::digest(encoded)))
}

fn accepted_endpoint_fingerprints(
    account: &crate::config::AccountConfig,
) -> Result<Vec<String>, String> {
    let mut accepted = vec![endpoint_fingerprint(account)?];
    let legacy = legacy_v1_endpoint_fingerprint(account)?;
    if !accepted.contains(&legacy) {
        accepted.push(legacy);
    }
    if account.auth_backend == crate::config::AuthBackend::MicrosoftGraphOAuth2
        && account.oauth_client_id.trim() == crate::oauth::DEFAULT_MICROSOFT_CLIENT_ID
    {
        let mut legacy_empty = account.clone();
        legacy_empty.oauth_client_id.clear();
        let legacy_empty_fp = legacy_v1_endpoint_fingerprint(&legacy_empty)?;
        if !accepted.contains(&legacy_empty_fp) {
            accepted.push(legacy_empty_fp);
        }
    }
    Ok(accepted)
}

#[derive(serde::Serialize, serde::Deserialize)]
struct BoundCredential<T> {
    endpoint: String,
    secret: T,
}

#[derive(serde::Deserialize)]
struct BoundCredentialHeader {
    endpoint: String,
}

fn bound_key(email: &str, kind: &str) -> String {
    format!(
        "email-{}-bound-{kind}-v1",
        normalize_credential_email(email)
    )
}

fn has_conflicting_bound_in(
    service: &str,
    email: &str,
    accepted_endpoints: &[String],
) -> Result<bool, String> {
    if normalize_credential_email(email).is_empty() {
        return Ok(false);
    }
    for kind in ["password", "oauth"] {
        if let Some(raw) = read_entry_optional(
            service,
            &bound_key(email, kind),
            "绑定凭据",
            "读取绑定凭据失败",
        )? {
            let Ok(parsed) = serde_json::from_str::<BoundCredentialHeader>(&raw) else {
                return Ok(true);
            };
            if !accepted_endpoints.contains(&parsed.endpoint) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Return whether the keyring already holds a bound credential for `account.email`
/// whose endpoint does not match `account`, or is malformed. Legacy unbound entries
/// are ignored because they are never read or migrated.
pub fn has_conflicting_bound_credentials(
    account: &crate::config::AccountConfig,
) -> Result<bool, String> {
    if normalize_credential_email(&account.email).is_empty() {
        return Ok(false);
    }
    let accepted = accepted_endpoint_fingerprints(account)?;
    let service = current_service_name();
    let email = account.email.clone();
    run_on_keyring_thread(move || has_conflicting_bound_in(&service, &email, &accepted))
}

fn write_bound<T: serde::Serialize>(
    service: &str,
    account: &crate::config::AccountConfig,
    kind: &str,
    secret: T,
) -> Result<(), String> {
    if normalize_credential_email(&account.email).is_empty() {
        return Err("邮箱地址为空，无法绑定凭据".into());
    }
    let value = serde_json::to_string(&BoundCredential {
        endpoint: endpoint_fingerprint(account)?,
        secret,
    })
    .map_err(|e| format!("生成绑定凭据失败: {e}"))?;
    write_entry(
        service,
        &bound_key(&account.email, kind),
        &value,
        "绑定凭据",
        "保存绑定凭据失败",
    )
}

fn read_bound<T: serde::de::DeserializeOwned>(
    service: &str,
    account: &crate::config::AccountConfig,
    kind: &str,
) -> Result<T, String> {
    let value = read_entry_optional(
        service,
        &bound_key(&account.email, kind),
        "绑定凭据",
        "读取绑定凭据失败",
    )?
    .ok_or_else(|| "凭据未绑定当前服务器，请重新输入凭据或重新授权".to_string())?;
    let bound: BoundCredential<T> = serde_json::from_str(&value)
        .map_err(|_| "绑定凭据损坏，请重新输入凭据或重新授权".to_string())?;
    let accepted = accepted_endpoint_fingerprints(account)?;
    if !accepted.contains(&bound.endpoint) {
        return Err("凭据与当前连接端点不匹配，请重新输入凭据或重新授权".into());
    }
    Ok(bound.secret)
}

fn rotate_bound_oauth_in(
    service: &str,
    account: &crate::config::AccountConfig,
    previous: &str,
    token: &str,
) -> Result<(), String> {
    let key = bound_key(&account.email, "oauth");
    let value = read_entry_optional(service, &key, "绑定凭据", "读取绑定凭据失败")?
        .ok_or_else(|| "凭据未绑定当前服务器，请重新输入凭据或重新授权".to_string())?;
    let mut bound: BoundCredential<String> = serde_json::from_str(&value)
        .map_err(|_| "绑定凭据损坏，请重新输入凭据或重新授权".to_string())?;
    if !accepted_endpoint_fingerprints(account)?.contains(&bound.endpoint)
        || bound.secret != previous
    {
        return Err("凭据已更改或与当前连接端点不匹配，请重新授权".into());
    }
    bound.secret = token.to_string();
    let value = serde_json::to_string(&bound).map_err(|e| format!("生成绑定凭据失败: {e}"))?;
    write_entry(service, &key, &value, "绑定凭据", "保存绑定凭据失败")
}

pub fn rotate_bound_oauth_token_in(
    data_dir: Option<&Path>,
    account: &crate::config::AccountConfig,
    previous: &str,
    token: &str,
) -> Result<(), String> {
    let default_dir;
    let dir = match data_dir {
        Some(dir) => dir,
        None => {
            default_dir = credential_config_dir();
            &default_dir
        }
    };
    validate_account_config_in(dir, account)?;
    let service = current_service_name();
    let account = account.clone();
    let previous = previous.to_string();
    let token = token.to_string();
    run_on_keyring_thread(move || rotate_bound_oauth_in(&service, &account, &previous, &token))
}

fn validate_account_config_in(
    dir: &Path,
    account: &crate::config::AccountConfig,
) -> Result<(), String> {
    let saved = crate::config::load_accounts_in(dir)?
        .into_iter()
        .find(|a| a.id == account.id)
        .ok_or_else(|| "账号配置不存在，请重新输入凭据或重新授权".to_string())?;
    if endpoint_fingerprint(&saved)? != endpoint_fingerprint(account)? {
        return Err("账号配置与连接端点不匹配".into());
    }
    crate::config::validate_shared_credential_endpoint(dir, account)
}

pub fn save_bound_account_secrets(
    account: &crate::config::AccountConfig,
    secrets: AccountSecrets,
) -> Result<(), String> {
    let service = current_service_name();
    let account = account.clone();
    run_on_keyring_thread(move || write_bound(&service, &account, "password", secrets))
}

pub fn load_account_secrets_for_config_in(
    data_dir: &Path,
    account: &crate::config::AccountConfig,
) -> Result<AccountSecrets, String> {
    validate_account_config_in(data_dir, account)?;
    let service = current_service_name();
    let account = account.clone();
    run_on_keyring_thread(move || read_bound(&service, &account, "password"))
}

pub fn load_account_secrets_for_config(
    account: &crate::config::AccountConfig,
) -> Result<AccountSecrets, String> {
    load_account_secrets_for_config_in(&credential_config_dir(), account)
}

pub fn save_bound_oauth_token(
    account: &crate::config::AccountConfig,
    token: String,
) -> Result<(), String> {
    let service = current_service_name();
    let account = account.clone();
    run_on_keyring_thread(move || write_bound(&service, &account, "oauth", token))
}

pub fn load_bound_oauth_token_in(
    data_dir: &Path,
    account: &crate::config::AccountConfig,
) -> Result<String, String> {
    validate_account_config_in(data_dir, account)?;
    let service = current_service_name();
    let account = account.clone();
    run_on_keyring_thread(move || read_bound(&service, &account, "oauth"))
}

pub fn load_bound_oauth_token(account: &crate::config::AccountConfig) -> Result<String, String> {
    load_bound_oauth_token_in(&credential_config_dir(), account)
}

/// 按邮箱地址优先读取 IMAP（及可选独立 SMTP）凭据；若新邮箱键尚不存在，则回退读取旧版 `account-{id}-*` 并懒镜像到邮箱键。
pub fn load_account_secrets_for(_account_id: u32, _email: &str) -> Result<AccountSecrets, String> {
    Err("读取凭据需要完整账号配置，请重新输入凭据".into())
}

/// 选择性删除账号关联的凭据项：可分别控制是否删除邮箱键 `email-{lower_email}-*` 与旧版 `account-{id}-*`。
pub fn delete_account_secrets_selective(
    account_id: u32,
    email: &str,
    delete_email_keys: bool,
    delete_legacy_id_keys: bool,
) -> Result<(), String> {
    let mut keys = Vec::new();
    if delete_legacy_id_keys {
        keys.push(imap_entry_name(account_id));
        keys.push(smtp_entry_name(account_id));
        keys.push(oauth_refresh_entry_name(account_id));
    }
    if delete_email_keys {
        keys.push(bound_key(email, "password"));
        keys.push(bound_key(email, "oauth"));
        if let Some(k) = email_imap_entry_name(email) {
            keys.push(k);
        }
        if let Some(k) = email_smtp_entry_name(email) {
            keys.push(k);
        }
        if let Some(k) = email_oauth_refresh_entry_name(email) {
            keys.push(k);
        }
    }
    if keys.is_empty() {
        return Ok(());
    }
    let service_name = current_service_name();
    run_on_keyring_thread(move || {
        for key in &keys {
            delete_entry(&service_name, key);
        }
        Ok(())
    })
}

/// 删除指定账号关联的旧版 `account-{id}-*` 及邮箱键 `email-{lower_email}-*` 凭据。
pub fn delete_account_secrets_for(account_id: u32, email: &str) -> Result<(), String> {
    delete_account_secrets_selective(account_id, email, true, true)
}

/// 保存 OAuth2 刷新令牌到主键（优先邮箱键），并静默同步所有“已存在且与主键不同”的旧版 `account-{id}-oauth-refresh` 键。
pub fn save_oauth_refresh_token_and_sync_legacy(
    account_id: u32,
    email: &str,
    legacy_ids_to_sync: &[u32],
    refresh_token: String,
) -> Result<(), String> {
    let service_name = current_service_name();
    let primary_key = email_oauth_refresh_entry_name(email)
        .unwrap_or_else(|| oauth_refresh_entry_name(account_id));
    let mut seen = std::collections::HashSet::new();
    let mut legacy_keys = Vec::new();
    for &lid in legacy_ids_to_sync {
        if !seen.insert(lid) {
            continue;
        }
        let lkey = oauth_refresh_entry_name(lid);
        if lkey != primary_key {
            legacy_keys.push(lkey);
        }
    }

    run_on_keyring_thread(move || {
        write_entry(
            &service_name,
            &primary_key,
            &refresh_token,
            "OAuth",
            "保存 OAuth 刷新令牌失败",
        )?;
        for lkey in &legacy_keys {
            if let Ok(Some(_)) =
                read_entry_optional(&service_name, lkey, "OAuth", "读取旧版 OAuth 刷新令牌失败")
            {
                let _ = write_entry(
                    &service_name,
                    lkey,
                    &refresh_token,
                    "OAuth",
                    "同步旧版 OAuth 刷新令牌失败",
                );
            }
        }
        Ok(())
    })
}

/// 按邮箱地址优先保存 OAuth2 刷新令牌（邮箱为空时回退账号 ID）。
pub fn save_oauth_refresh_token_for(
    account_id: u32,
    email: &str,
    refresh_token: String,
) -> Result<(), String> {
    save_oauth_refresh_token_and_sync_legacy(account_id, email, &[], refresh_token)
}

/// 依次读取给定无冲突 `legacy_ids` 对应的旧版 `account-{id}-oauth-refresh`，跳过空值及与 `already_tried` 相同或重复的令牌。
pub fn load_distinct_legacy_oauth_refresh_tokens(
    legacy_ids: &[u32],
    already_tried: &str,
) -> Vec<String> {
    let mut seen_ids = std::collections::HashSet::new();
    let mut keys = Vec::new();
    for &lid in legacy_ids {
        if seen_ids.insert(lid) {
            keys.push(oauth_refresh_entry_name(lid));
        }
    }
    if keys.is_empty() {
        return Vec::new();
    }

    let service_name = current_service_name();
    let tried = already_tried.to_string();
    run_on_keyring_thread(move || {
        let mut seen_tokens = std::collections::HashSet::new();
        seen_tokens.insert(tried);
        let mut out = Vec::new();
        for key in &keys {
            if let Ok(Some(val)) =
                read_entry_optional(&service_name, key, "OAuth", "读取旧版 OAuth 刷新令牌失败")
            {
                if !val.is_empty() && seen_tokens.insert(val.clone()) {
                    out.push(val);
                }
            }
        }
        Ok(out)
    })
    .unwrap_or_default()
}

/// 按邮箱地址优先读取 OAuth2 刷新令牌；若新邮箱键尚不存在，则回退读取旧版 `account-{id}-oauth-refresh` 并懒镜像到邮箱键。
pub fn load_oauth_refresh_token_for(_account_id: u32, _email: &str) -> Result<String, String> {
    Err("读取 OAuth 凭据需要完整账号配置，请重新授权".into())
}

/// 将旧版 `account-{legacy_account_id}-*` 凭据一次性复制迁移到邮箱键 `email-{lower_email}-*`（不覆盖已有邮箱键，也不删除旧键）。
pub fn migrate_legacy_secret_to_email(_legacy_account_id: u32, _email: &str) -> Result<(), String> {
    Err("旧版凭据未绑定服务器，请重新输入凭据或重新授权".into())
}

/// Save IMAP (and optional separate SMTP) credentials in Windows Credential Manager.
pub fn save_account_secrets(account_id: u32, secrets: AccountSecrets) -> Result<(), String> {
    save_account_secrets_for(account_id, "", secrets)
}

/// Load IMAP (and optional separate SMTP) credentials from Windows Credential Manager.
pub fn load_account_secrets(account_id: u32) -> Result<AccountSecrets, String> {
    load_account_secrets_for(account_id, "")
}

/// Remove saved credentials for `account_id` from Windows Credential Manager.
pub fn delete_account_secrets(account_id: u32) -> Result<(), String> {
    delete_account_secrets_for(account_id, "")
}

/// Save an OAuth2 refresh token in Windows Credential Manager.
pub fn save_oauth_refresh_token(account_id: u32, refresh_token: String) -> Result<(), String> {
    save_oauth_refresh_token_for(account_id, "", refresh_token)
}

/// Load an OAuth2 refresh token from Windows Credential Manager.
pub fn load_oauth_refresh_token(account_id: u32) -> Result<String, String> {
    load_oauth_refresh_token_for(account_id, "")
}

const APP_LOCK_PASSCODE_KEY: &str = "app-lock-passcode-hash";

/// 将应用锁屏密码哈希持久化到 Windows 凭据管理器。
pub fn save_app_lock_hash(hash: &str) -> Result<(), String> {
    let service_name = current_service_name();
    let hash_owned = hash.to_string();
    run_on_keyring_thread(move || {
        write_entry(
            &service_name,
            APP_LOCK_PASSCODE_KEY,
            &hash_owned,
            "锁屏密码",
            "保存锁屏密码失败",
        )
    })
}

/// 从 Windows 凭据管理器读取应用锁屏密码哈希；未配置时返回 `Ok(None)`。
pub fn load_app_lock_hash() -> Result<Option<String>, String> {
    let service_name = current_service_name();
    run_on_keyring_thread(move || {
        read_entry_optional(
            &service_name,
            APP_LOCK_PASSCODE_KEY,
            "锁屏密码",
            "读取锁屏密码失败",
        )
    })
}

/// 从 Windows 凭据管理器删除应用锁屏密码哈希（不存在时幂等返回 `Ok(())`）。
pub fn delete_app_lock_hash() -> Result<(), String> {
    let service_name = current_service_name();
    run_on_keyring_thread(move || {
        let entry = keyring::Entry::new(&service_name, APP_LOCK_PASSCODE_KEY)
            .map_err(|e| format!("初始化 锁屏密码 凭据项失败: {e}"))?;
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("删除锁屏密码失败: {e}")),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[cfg(windows)]
    #[test]
    fn windows_credential_manager_bound_credentials_end_to_end() {
        let mut random = [0u8; 16];
        getrandom::getrandom(&mut random).unwrap();
        let suffix: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let service = format!("KiteMail.Test.Isolated.{suffix}");
        let mut account = crate::config::AccountConfig::default();
        account.id = 1;
        account.email = format!("{suffix}@example.invalid");
        account.imap_host = "imap.example.invalid".into();
        account.smtp_host = "smtp.example.invalid".into();
        struct Cleanup {
            service: String,
            keys: Vec<String>,
        }
        impl Drop for Cleanup {
            fn drop(&mut self) {
                for key in &self.keys {
                    delete_entry(&self.service, key);
                }
            }
        }
        let _cleanup = Cleanup {
            service: service.clone(),
            keys: vec![
                bound_key(&account.email, "password"),
                bound_key(&account.email, "oauth"),
                email_imap_entry_name(&account.email).unwrap(),
            ],
        };
        let password = AccountSecrets {
            imap_password: "synthetic-secret".into(),
            smtp_password: Some("synthetic-smtp".into()),
        };
        write_entry(
            &service,
            &email_imap_entry_name(&account.email).unwrap(),
            "old-secret",
            "test",
            "test",
        )
        .unwrap();
        assert!(read_bound::<AccountSecrets>(&service, &account, "password").is_err());
        write_entry(
            &service,
            &bound_key(&account.email, "password"),
            "orphan-binding",
            "test",
            "test",
        )
        .unwrap();
        assert!(read_bound::<AccountSecrets>(&service, &account, "password").is_err());
        write_bound(&service, &account, "password", password).unwrap();
        let read: AccountSecrets = read_bound(&service, &account, "password").unwrap();
        assert_eq!(read.imap_password, "synthetic-secret");
        assert_eq!(read.smtp_password.as_deref(), Some("synthetic-smtp"));
        let mut attacker = account.clone();
        attacker.imap_host = "attacker.invalid".into();
        assert!(read_bound::<AccountSecrets>(&service, &attacker, "password").is_err());
        write_bound(
            &service,
            &attacker,
            "password",
            AccountSecrets {
                imap_password: "new-secret".into(),
                smtp_password: None,
            },
        )
        .unwrap();
        assert!(read_bound::<AccountSecrets>(&service, &account, "password").is_err());
        assert_eq!(
            read_bound::<AccountSecrets>(&service, &attacker, "password")
                .unwrap()
                .imap_password,
            "new-secret"
        );
        account.auth_backend = crate::config::AuthBackend::MicrosoftGraphOAuth2;
        assert!(read_bound::<String>(&service, &account, "oauth").is_err());
        write_bound(&service, &account, "oauth", "synthetic-token").unwrap();
        assert_eq!(
            read_bound::<String>(&service, &account, "oauth").unwrap(),
            "synthetic-token"
        );
        account.oauth_client_id = "different-client".into();
        assert!(read_bound::<String>(&service, &account, "oauth").is_err());

        // 冲突探测：同端点允许，异端点与损坏条目拒绝，旧未绑定条目忽略。
        write_bound(
            &service,
            &account,
            "password",
            AccountSecrets {
                imap_password: "aligned-secret".into(),
                smtp_password: None,
            },
        )
        .unwrap();
        write_bound(&service, &account, "oauth", "aligned-token").unwrap();
        let accepted = accepted_endpoint_fingerprints(&account).unwrap();
        assert!(!has_conflicting_bound_in(&service, &account.email, &accepted).unwrap());
        let mut wrong_endpoint = account.clone();
        wrong_endpoint.imap_host = "other.invalid".into();
        let wrong_accepted = accepted_endpoint_fingerprints(&wrong_endpoint).unwrap();
        assert!(has_conflicting_bound_in(&service, &account.email, &wrong_accepted).unwrap());

        write_entry(
            &service,
            &bound_key(&account.email, "password"),
            "corrupted-json",
            "test",
            "test",
        )
        .unwrap();
        assert!(has_conflicting_bound_in(&service, &account.email, &accepted).unwrap());

        // 兼容 3f27df0e 旧版 v1 绑定指纹（默认微软客户端为空字符串、字段首尾带空格）且保持旧条目不变。
        delete_entry(&service, &bound_key(&account.email, "password"));
        let mut legacy_oauth = account.clone();
        legacy_oauth.auth_backend = crate::config::AuthBackend::MicrosoftGraphOAuth2;
        legacy_oauth.oauth_client_id.clear();
        let legacy_oauth_fp = legacy_v1_endpoint_fingerprint(&legacy_oauth).unwrap();
        let legacy_oauth_payload = serde_json::to_string(&BoundCredential {
            endpoint: legacy_oauth_fp.clone(),
            secret: "legacy-v1-rt".to_string(),
        })
        .unwrap();
        write_entry(
            &service,
            &bound_key(&legacy_oauth.email, "oauth"),
            &legacy_oauth_payload,
            "test",
            "test",
        )
        .unwrap();

        let mut explicit_default_oauth = legacy_oauth.clone();
        explicit_default_oauth.oauth_client_id = crate::oauth::DEFAULT_MICROSOFT_CLIENT_ID.into();
        assert!(!has_conflicting_bound_in(
            &service,
            &legacy_oauth.email,
            &accepted_endpoint_fingerprints(&legacy_oauth).unwrap(),
        )
        .unwrap());
        assert!(!has_conflicting_bound_in(
            &service,
            &explicit_default_oauth.email,
            &accepted_endpoint_fingerprints(&explicit_default_oauth).unwrap(),
        )
        .unwrap());
        let mut other_client = legacy_oauth.clone();
        other_client.oauth_client_id = "other-client".into();
        assert!(has_conflicting_bound_in(
            &service,
            &other_client.email,
            &accepted_endpoint_fingerprints(&other_client).unwrap(),
        )
        .unwrap());
        assert!(read_bound::<String>(&service, &other_client, "oauth").is_err());

        assert_eq!(
            read_bound::<String>(&service, &legacy_oauth, "oauth").unwrap(),
            "legacy-v1-rt"
        );
        let migrated_oauth_raw = read_entry_optional(
            &service,
            &bound_key(&legacy_oauth.email, "oauth"),
            "绑定凭据",
            "读取绑定凭据失败",
        )
        .unwrap()
        .unwrap();
        assert_eq!(migrated_oauth_raw, legacy_oauth_payload);
        let legacy_oauth_read: BoundCredential<String> =
            serde_json::from_str(&migrated_oauth_raw).unwrap();
        assert_eq!(legacy_oauth_read.endpoint, legacy_oauth_fp);
        assert_eq!(legacy_oauth_read.secret, "legacy-v1-rt");

        rotate_bound_oauth_in(&service, &legacy_oauth, "legacy-v1-rt", "rotated-rt").unwrap();
        let rotated_raw = read_entry_optional(
            &service,
            &bound_key(&legacy_oauth.email, "oauth"),
            "test",
            "test",
        )
        .unwrap()
        .unwrap();
        let rotated: BoundCredential<String> = serde_json::from_str(&rotated_raw).unwrap();
        assert_eq!(rotated.endpoint, legacy_oauth_fp);
        assert_eq!(rotated.secret, "rotated-rt");
        assert!(rotate_bound_oauth_in(&service, &other_client, "rotated-rt", "wrong").is_err());
        assert!(rotate_bound_oauth_in(&service, &legacy_oauth, "legacy-v1-rt", "stale").is_err());
        assert_eq!(
            read_entry_optional(
                &service,
                &bound_key(&legacy_oauth.email, "oauth"),
                "test",
                "test"
            )
            .unwrap()
            .unwrap(),
            rotated_raw
        );
        delete_entry(&service, &bound_key(&account.email, "oauth"));
        assert!(rotate_bound_oauth_in(&service, &legacy_oauth, "rotated-rt", "missing").is_err());
        let mut padded_pwd = crate::config::AccountConfig::default();
        padded_pwd.id = account.id;
        padded_pwd.email = account.email.clone();
        padded_pwd.imap_host = " imap.example.invalid ".into();
        padded_pwd.smtp_host = " smtp.example.invalid ".into();
        let legacy_pwd_fp = legacy_v1_endpoint_fingerprint(&padded_pwd).unwrap();
        let legacy_pwd_payload = serde_json::to_string(&BoundCredential {
            endpoint: legacy_pwd_fp.clone(),
            secret: AccountSecrets {
                imap_password: "legacy-v1-secret".into(),
                smtp_password: None,
            },
        })
        .unwrap();
        write_entry(
            &service,
            &bound_key(&padded_pwd.email, "password"),
            &legacy_pwd_payload,
            "test",
            "test",
        )
        .unwrap();
        assert!(!has_conflicting_bound_in(
            &service,
            &padded_pwd.email,
            &accepted_endpoint_fingerprints(&padded_pwd).unwrap(),
        )
        .unwrap());
        let read_legacy_pwd =
            read_bound::<AccountSecrets>(&service, &padded_pwd, "password").unwrap();
        assert_eq!(read_legacy_pwd.imap_password, "legacy-v1-secret");
        let migrated_pwd_raw = read_entry_optional(
            &service,
            &bound_key(&padded_pwd.email, "password"),
            "绑定凭据",
            "读取绑定凭据失败",
        )
        .unwrap()
        .unwrap();
        assert_eq!(migrated_pwd_raw, legacy_pwd_payload);
        let legacy_pwd_read: BoundCredential<AccountSecrets> =
            serde_json::from_str(&migrated_pwd_raw).unwrap();
        assert_eq!(legacy_pwd_read.endpoint, legacy_pwd_fp);
        assert_eq!(legacy_pwd_read.secret.imap_password, "legacy-v1-secret");
        let mut wrong_pwd_endpoint = padded_pwd.clone();
        wrong_pwd_endpoint.imap_host = "other.invalid".into();
        assert!(read_bound::<AccountSecrets>(&service, &wrong_pwd_endpoint, "password").is_err());
    }

    #[test]
    fn endpoint_binding_tracks_security_sensitive_fields() {
        let mut a = crate::config::AccountConfig::default();
        a.email = "Alice@example.com".into();
        a.imap_host = "imap.example.com".into();
        let original = endpoint_fingerprint(&a).unwrap();
        a.email = " alice@EXAMPLE.COM ".into();
        a.imap_host = " IMAP.EXAMPLE.COM ".into();
        assert_eq!(original, endpoint_fingerprint(&a).unwrap());
        a.imap_host = "attacker.invalid".into();
        assert_ne!(original, endpoint_fingerprint(&a).unwrap());
        a.imap_host = "imap.example.com".into();
        a.smtp_port += 1;
        assert_ne!(original, endpoint_fingerprint(&a).unwrap());
        a.smtp_port -= 1;
        a.oauth_client_id = "other-client".into();
        assert_ne!(original, endpoint_fingerprint(&a).unwrap());

        let mut oauth_empty = crate::config::AccountConfig {
            auth_backend: crate::config::AuthBackend::MicrosoftGraphOAuth2,
            email: "oauth@example.com".into(),
            ..Default::default()
        };
        let mut oauth_default = oauth_empty.clone();
        oauth_default.oauth_client_id = crate::oauth::DEFAULT_MICROSOFT_CLIENT_ID.into();
        assert_eq!(
            endpoint_fingerprint(&oauth_empty).unwrap(),
            endpoint_fingerprint(&oauth_default).unwrap()
        );
        oauth_empty.oauth_client_id = "custom".into();
        assert_ne!(
            endpoint_fingerprint(&oauth_empty).unwrap(),
            endpoint_fingerprint(&oauth_default).unwrap()
        );
    }

    #[test]
    fn validate_account_config_in_uses_explicit_data_dir() {
        let dir =
            std::env::temp_dir().join(format!("kitemail-validate-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut acc = crate::config::AccountConfig::default();
        acc.id = 42;
        acc.email = "custom-dir@example.invalid".into();
        acc.imap_host = "imap.example.invalid".into();
        crate::config::save_accounts_in(&dir, &[acc.clone()]).unwrap();

        assert!(validate_account_config_in(&dir, &acc).is_ok());
        let mut altered = acc.clone();
        altered.imap_host = "evil.invalid".into();
        assert!(validate_account_config_in(&dir, &altered).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn service_name_shares_installed_and_portable_and_isolates_env_override() {
        let installed =
            resolve_service_name_for(Path::new(r"C:\Users\Alice\AppData\Local\KiteMail"), false);
        let portable = resolve_service_name_for(Path::new(r"D:\PortableApps\KiteMail\data"), false);
        assert_eq!(installed, "KiteMail.Windows");
        assert_eq!(portable, installed);

        let dir_a = PathBuf::from(r"D:\PortableApps\KiteMailA\data");
        let dir_b = PathBuf::from(r"E:\USB\KiteMailB\data");
        let svc_a1 = resolve_service_name_for(&dir_a, true);
        let svc_a2 = resolve_service_name_for(&dir_a, true);
        let svc_b = resolve_service_name_for(&dir_b, true);

        assert!(svc_a1.starts_with("KiteMail.Windows.Portable."));
        assert_eq!(svc_a1.len(), "KiteMail.Windows.Portable.".len() + 12);
        assert_eq!(svc_a1, svc_a2);
        assert_ne!(svc_a1, svc_b);
    }

    #[test]
    fn email_entry_names_normalize_case_and_whitespace() {
        assert_eq!(
            normalize_credential_email("  Alice.User@Outlook.COM "),
            "alice.user@outlook.com"
        );
        assert_eq!(
            email_imap_entry_name("  Alice@Example.COM "),
            Some("email-alice@example.com-imap".to_string())
        );
        assert_eq!(
            email_smtp_entry_name("Alice@Example.COM"),
            Some("email-alice@example.com-smtp".to_string())
        );
        assert_eq!(
            email_oauth_refresh_entry_name("Alice@Example.COM"),
            Some("email-alice@example.com-oauth-refresh".to_string())
        );
        assert_eq!(email_imap_entry_name("   "), None);
        assert_eq!(email_smtp_entry_name(""), None);
        assert_eq!(email_oauth_refresh_entry_name(""), None);
    }

    #[cfg(windows)]
    #[test]
    fn app_lock_hash_roundtrip_and_delete() {
        struct Cleanup;
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = delete_app_lock_hash();
            }
        }
        let _cleanup = Cleanup;

        delete_app_lock_hash().expect("initial idempotent delete");
        assert_eq!(load_app_lock_hash().expect("load empty"), None);

        save_app_lock_hash("pbkdf2_sha256$200000$c2FsdA==$ZGs=").expect("save hash");
        assert_eq!(
            load_app_lock_hash().expect("load saved"),
            Some("pbkdf2_sha256$200000$c2FsdA==$ZGs=".to_string())
        );

        save_app_lock_hash("pbkdf2_sha256$200000$bmV3c2FsdA==$bmV3ZGs=").expect("update hash");
        assert_eq!(
            load_app_lock_hash().expect("load updated"),
            Some("pbkdf2_sha256$200000$bmV3c2FsdA==$bmV3ZGs=".to_string())
        );

        delete_app_lock_hash().expect("delete hash");
        assert_eq!(load_app_lock_hash().expect("load after delete"), None);
    }
}
