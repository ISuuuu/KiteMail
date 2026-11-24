//! Credential storage backed by Windows Credential Manager via `keyring`.
//!
//! All keyring operations run serially on a dedicated background thread so a
//! save followed immediately by a read or delete is never reordered.

use std::path::Path;
use std::sync::mpsc;
use std::sync::OnceLock;

use sha2::{Digest, Sha256};

const DEFAULT_SERVICE_NAME: &str = "co.hyprlab.Hylki.Windows";

/// 根据配置目录和自定义隔离标志推导 Windows 凭据管理器命名空间。
/// 安装版与便携版在同一台电脑上共享 `co.hyprlab.Hylki.Windows` 命名空间，仅在显式指定自定义 `HYLKI_DATA_DIR` 时附加目录指纹隔离。
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
    let has_env_override = std::env::var_os("HYLKI_DATA_DIR").is_some_and(|v| !v.is_empty());
    resolve_service_name_for(&crate::config::data_dir(), has_env_override)
}

type KeyringJob = Box<dyn FnOnce() + Send>;

fn keyring_sender() -> Option<&'static mpsc::Sender<KeyringJob>> {
    static SENDER: OnceLock<Option<mpsc::Sender<KeyringJob>>> = OnceLock::new();
    SENDER
        .get_or_init(|| {
            let (tx, rx) = mpsc::channel::<KeyringJob>();
            std::thread::Builder::new()
                .name("hylki-win-keyring".to_string())
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

fn load_account_secrets_raw(
    service_name: &str,
    imap_key: &str,
    smtp_key: &str,
) -> Result<AccountSecrets, String> {
    let imap_entry = keyring::Entry::new(service_name, imap_key)
        .map_err(|e| format!("初始化 IMAP 凭据项失败: {e}"))?;
    let imap_password = imap_entry
        .get_password()
        .map_err(|e| format!("读取 IMAP 授权码失败: {e}"))?;
    let smtp_password =
        read_entry_optional(service_name, smtp_key, "SMTP", "读取 SMTP 授权码失败")?;
    Ok(AccountSecrets {
        imap_password,
        smtp_password,
    })
}

fn load_oauth_raw(service_name: &str, key: &str) -> Result<String, String> {
    let entry = keyring::Entry::new(service_name, key)
        .map_err(|e| format!("初始化 OAuth 凭据项失败: {e}"))?;
    entry
        .get_password()
        .map_err(|e| format!("读取 OAuth 刷新令牌失败（请重新执行 OAuth2 登录）: {e}"))
}

/// Secrets associated with one mail account.
#[derive(Clone, Debug, Default)]
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

/// 按邮箱地址优先读取 IMAP（及可选独立 SMTP）凭据；若新邮箱键尚不存在，则回退读取旧版 `account-{id}-*` 并懒镜像到邮箱键。
pub fn load_account_secrets_for(account_id: u32, email: &str) -> Result<AccountSecrets, String> {
    let service_name = current_service_name();
    let email_imap = email_imap_entry_name(email);
    let email_smtp = email_smtp_entry_name(email);
    let legacy_imap = imap_entry_name(account_id);
    let legacy_smtp = smtp_entry_name(account_id);
    run_on_keyring_thread(move || {
        if let (Some(e_imap), Some(e_smtp)) = (&email_imap, &email_smtp) {
            if let Some(imap_password) =
                read_entry_optional(&service_name, e_imap, "IMAP", "读取 IMAP 授权码失败")?
            {
                let smtp_password =
                    read_entry_optional(&service_name, e_smtp, "SMTP", "读取 SMTP 授权码失败")?;
                return Ok(AccountSecrets {
                    imap_password,
                    smtp_password,
                });
            }
            let secrets = load_account_secrets_raw(&service_name, &legacy_imap, &legacy_smtp)?;
            let _ = write_entry(
                &service_name,
                e_imap,
                &secrets.imap_password,
                "IMAP",
                "保存 IMAP 授权码失败",
            );
            if let Some(ref pwd) = secrets.smtp_password {
                if !pwd.is_empty() {
                    let _ = write_entry(&service_name, e_smtp, pwd, "SMTP", "保存 SMTP 授权码失败");
                }
            }
            return Ok(secrets);
        }

        load_account_secrets_raw(&service_name, &legacy_imap, &legacy_smtp)
    })
}

/// 删除指定账号关联的旧版 `account-{id}-*` 及邮箱键 `email-{lower_email}-*` 凭据。
pub fn delete_account_secrets_for(account_id: u32, email: &str) -> Result<(), String> {
    let service_name = current_service_name();
    let mut keys = vec![
        imap_entry_name(account_id),
        smtp_entry_name(account_id),
        oauth_refresh_entry_name(account_id),
    ];
    if let Some(k) = email_imap_entry_name(email) {
        keys.push(k);
    }
    if let Some(k) = email_smtp_entry_name(email) {
        keys.push(k);
    }
    if let Some(k) = email_oauth_refresh_entry_name(email) {
        keys.push(k);
    }
    run_on_keyring_thread(move || {
        for key in &keys {
            delete_entry(&service_name, key);
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
    let service_name = current_service_name();
    let key = email_oauth_refresh_entry_name(email)
        .unwrap_or_else(|| oauth_refresh_entry_name(account_id));
    run_on_keyring_thread(move || {
        write_entry(
            &service_name,
            &key,
            &refresh_token,
            "OAuth",
            "保存 OAuth 刷新令牌失败",
        )
    })
}

/// 按邮箱地址优先读取 OAuth2 刷新令牌；若新邮箱键尚不存在，则回退读取旧版 `account-{id}-oauth-refresh` 并懒镜像到邮箱键。
pub fn load_oauth_refresh_token_for(account_id: u32, email: &str) -> Result<String, String> {
    let service_name = current_service_name();
    let email_key = email_oauth_refresh_entry_name(email);
    let legacy_key = oauth_refresh_entry_name(account_id);
    run_on_keyring_thread(move || {
        if let Some(ref e_key) = email_key {
            if let Some(token) = read_entry_optional(
                &service_name,
                e_key,
                "OAuth",
                "读取 OAuth 刷新令牌失败（请重新执行 OAuth2 登录）",
            )? {
                return Ok(token);
            }
            let token = load_oauth_raw(&service_name, &legacy_key)?;
            let _ = write_entry(
                &service_name,
                e_key,
                &token,
                "OAuth",
                "保存 OAuth 刷新令牌失败",
            );
            return Ok(token);
        }

        load_oauth_raw(&service_name, &legacy_key)
    })
}

/// 将旧版 `account-{legacy_account_id}-*` 凭据一次性复制迁移到邮箱键 `email-{lower_email}-*`（不覆盖已有邮箱键，也不删除旧键）。
pub fn migrate_legacy_secret_to_email(legacy_account_id: u32, email: &str) -> Result<(), String> {
    let (Some(e_imap), Some(e_smtp), Some(e_oauth)) = (
        email_imap_entry_name(email),
        email_smtp_entry_name(email),
        email_oauth_refresh_entry_name(email),
    ) else {
        return Ok(());
    };
    let service_name = current_service_name();
    let pairs = [
        (
            imap_entry_name(legacy_account_id),
            e_imap,
            "IMAP",
            "保存 IMAP 授权码失败",
        ),
        (
            smtp_entry_name(legacy_account_id),
            e_smtp,
            "SMTP",
            "保存 SMTP 授权码失败",
        ),
        (
            oauth_refresh_entry_name(legacy_account_id),
            e_oauth,
            "OAuth",
            "保存 OAuth 刷新令牌失败",
        ),
    ];
    run_on_keyring_thread(move || {
        for (legacy_key, email_key, kind, write_err) in pairs {
            if read_entry_optional(&service_name, &email_key, kind, write_err)?.is_some() {
                continue;
            }
            if let Some(val) = read_entry_optional(&service_name, &legacy_key, kind, write_err)? {
                if !val.is_empty() {
                    write_entry(&service_name, &email_key, &val, kind, write_err)?;
                }
            }
        }
        Ok(())
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn service_name_shares_installed_and_portable_and_isolates_env_override() {
        let installed =
            resolve_service_name_for(Path::new(r"C:\Users\Alice\AppData\Local\Hylki"), false);
        let portable = resolve_service_name_for(Path::new(r"D:\PortableApps\Hylki\data"), false);
        assert_eq!(installed, "co.hyprlab.Hylki.Windows");
        assert_eq!(portable, installed);

        let dir_a = PathBuf::from(r"D:\PortableApps\HylkiA\data");
        let dir_b = PathBuf::from(r"E:\USB\HylkiB\data");
        let svc_a1 = resolve_service_name_for(&dir_a, true);
        let svc_a2 = resolve_service_name_for(&dir_a, true);
        let svc_b = resolve_service_name_for(&dir_b, true);

        assert!(svc_a1.starts_with("co.hyprlab.Hylki.Windows.Portable."));
        assert_eq!(
            svc_a1.len(),
            "co.hyprlab.Hylki.Windows.Portable.".len() + 12
        );
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
}
