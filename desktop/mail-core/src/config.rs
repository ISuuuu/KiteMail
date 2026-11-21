//! Non-sensitive account configuration and Windows local data directory paths.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Transport encryption mode for IMAP and SMTP connections.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SecurityMode {
    #[default]
    Tls,
    StartTls,
}

/// Authentication & mail transport backend for an account.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthBackend {
    #[default]
    Password,
    MicrosoftGraphOAuth2,
}

/// Non-sensitive configuration for one mail account.
/// Passwords and OAuth refresh tokens live in Windows Credential Manager.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountConfig {
    pub id: u32,
    #[serde(default)]
    pub auth_backend: AuthBackend,
    #[serde(default)]
    pub oauth_client_id: String,
    pub display_name: String,
    pub email: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_security: SecurityMode,
    pub imap_username: String,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_security: SecurityMode,
    pub smtp_username: String,
    pub smtp_separate_credentials: bool,
}

impl Default for AccountConfig {
    fn default() -> Self {
        Self {
            id: 1,
            auth_backend: AuthBackend::Password,
            oauth_client_id: String::new(),
            display_name: String::new(),
            email: String::new(),
            imap_host: String::new(),
            imap_port: 993,
            imap_security: SecurityMode::Tls,
            imap_username: String::new(),
            smtp_host: String::new(),
            smtp_port: 465,
            smtp_security: SecurityMode::Tls,
            smtp_username: String::new(),
            smtp_separate_credentials: false,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct AccountsFile {
    #[serde(default)]
    accounts: Vec<AccountConfig>,
}

/// User interface & sync preferences persisted in `preferences.toml`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppPreferences {
    /// Active theme identifier (e.g. `"default"`, `"rose"`).
    pub theme_id: String,
    /// `None` follows the OS setting; `Some(true/false)` forces dark/light.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dark_mode: Option<bool>,
    /// Denser list rows in the main window.
    pub compact_list: bool,
    /// Number of messages fetched per page.
    pub page_size: usize,
    /// Background sync interval in minutes; `0` means manual sync only.
    pub auto_sync_minutes: u32,
    /// Block remote images in HTML mail bodies.
    pub block_remote_images: bool,
    /// Minimize the window instead of exiting when clicking the titlebar close button.
    pub close_to_minimize: bool,
    /// View opened at startup: `"unified"` (unified inbox) or `"last"`
    /// (the folder selected in the previous session).
    pub default_view: String,
    /// Last browsed unified view (`"unified"` / `"starred"` / `"sent"` /
    /// `"drafts"` / `"archive"`); restored when `default_view == "last"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_view: Option<String>,
    /// Last browsed account folder `(account_id, folder_id)`; restored when
    /// `default_view == "last"` and takes precedence over [`Self::last_view`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_folder: Option<(u32, u32)>,
    /// Show newest replies first in conversation threads (`true` = descending).
    #[serde(default = "default_true")]
    pub thread_newest_first: bool,
    /// Automatically fold older messages when opening a multi-message thread.
    #[serde(default = "default_true")]
    pub fold_older_messages: bool,
    /// Lowercased sender email addresses always allowed to load remote images.
    #[serde(default)]
    pub allowed_remote_senders: Vec<String>,
    /// Reader body zoom percentage (`70..=160`, default `100`).
    #[serde(default = "default_reader_zoom_percent")]
    pub reader_zoom_percent: u32,
}

fn default_true() -> bool {
    true
}

fn default_reader_zoom_percent() -> u32 {
    100
}

/// Clamp reader zoom percentage to the supported `70..=160` range.
pub fn clamp_reader_zoom(percent: u32) -> u32 {
    percent.clamp(70, 160)
}

/// Extract a normalized (trimmed, lowercased) email address from a sender header
/// such as `"Alice <Alice@Example.com>"` or `"alice@example.com"`.
pub fn normalize_sender_email(sender: &str) -> String {
    let trimmed = sender.trim();
    if let (Some(open), Some(close)) = (trimmed.rfind('<'), trimmed.rfind('>')) {
        if open < close {
            return trimmed[open + 1..close].trim().to_ascii_lowercase();
        }
    }
    trimmed.to_ascii_lowercase()
}

impl AppPreferences {
    /// Return whether `sender` is in the persistent remote-image allowlist.
    pub fn is_remote_sender_allowed(&self, sender: &str) -> bool {
        let email = normalize_sender_email(sender);
        if email.is_empty() {
            return false;
        }
        self.allowed_remote_senders
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(&email))
    }

    /// Add `sender`'s normalized email address to [`Self::allowed_remote_senders`]
    /// if valid and not already present. Returns the normalized email on insert/hit.
    pub fn allow_remote_sender(&mut self, sender: &str) -> Option<String> {
        let email = normalize_sender_email(sender);
        if email.is_empty() || !email.contains('@') {
            return None;
        }
        if !self
            .allowed_remote_senders
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(&email))
        {
            self.allowed_remote_senders.push(email.clone());
        }
        Some(email)
    }
}

impl Default for AppPreferences {
    fn default() -> Self {
        Self {
            theme_id: "default".to_string(),
            dark_mode: None,
            compact_list: false,
            page_size: 100,
            auto_sync_minutes: 5,
            block_remote_images: true,
            close_to_minimize: true,
            default_view: "unified".to_string(),
            last_view: None,
            last_folder: None,
            thread_newest_first: true,
            fold_older_messages: true,
            allowed_remote_senders: Vec::new(),
            reader_zoom_percent: 100,
        }
    }
}

/// Return the Hylki Windows local data directory (`%LOCALAPPDATA%\Hylki`).
pub fn data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Hylki")
}

/// Return the path to `accounts.toml` inside `dir`.
pub fn accounts_file_in(dir: &Path) -> PathBuf {
    dir.join("accounts.toml")
}

/// Return the path to the SQLite cache database inside `dir`.
pub fn cache_db_in(dir: &Path) -> PathBuf {
    dir.join("hylki-cache.db")
}

/// Return the path to `preferences.toml` inside `dir`.
pub fn preferences_file_in(dir: &Path) -> PathBuf {
    dir.join("preferences.toml")
}

/// Load user preferences from `dir`; a missing file yields the defaults.
pub fn load_preferences_in(dir: &Path) -> Result<AppPreferences, String> {
    let path = preferences_file_in(dir);
    if !path.exists() {
        return Ok(AppPreferences::default());
    }
    let text = fs::read_to_string(&path)
        .map_err(|e| format!("读取偏好设置失败 ({}): {e}", path.display()))?;
    toml::from_str(&text).map_err(|e| format!("解析偏好设置失败 ({}): {e}", path.display()))
}

/// Persist user preferences into `dir`.
pub fn save_preferences_in(dir: &Path, prefs: &AppPreferences) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("创建数据目录失败 ({}): {e}", dir.display()))?;
    let path = preferences_file_in(dir);
    let text = toml::to_string_pretty(prefs).map_err(|e| format!("序列化偏好设置失败: {e}"))?;
    fs::write(&path, text).map_err(|e| format!("写入偏好设置失败 ({}): {e}", path.display()))?;
    Ok(())
}

/// Load saved non-sensitive account configurations from `dir`.
pub fn load_accounts_in(dir: &Path) -> Result<Vec<AccountConfig>, String> {
    let path = accounts_file_in(dir);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(&path)
        .map_err(|e| format!("读取账号配置失败 ({}): {e}", path.display()))?;
    let parsed: AccountsFile =
        toml::from_str(&text).map_err(|e| format!("解析账号配置失败 ({}): {e}", path.display()))?;
    Ok(parsed.accounts)
}

/// Persist non-sensitive account configurations into `dir`.
pub fn save_accounts_in(dir: &Path, accounts: &[AccountConfig]) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("创建数据目录失败 ({}): {e}", dir.display()))?;
    let path = accounts_file_in(dir);
    let data = AccountsFile {
        accounts: accounts.to_vec(),
    };
    let text = toml::to_string_pretty(&data).map_err(|e| format!("序列化账号配置失败: {e}"))?;
    fs::write(&path, text).map_err(|e| format!("写入账号配置失败 ({}): {e}", path.display()))?;
    Ok(())
}

/// Insert or replace an account in `accounts.toml` inside `dir`.
pub fn upsert_account_in(dir: &Path, account: AccountConfig) -> Result<Vec<AccountConfig>, String> {
    let mut accounts = load_accounts_in(dir)?;
    if let Some(existing) = accounts.iter_mut().find(|a| a.id == account.id) {
        *existing = account;
    } else {
        accounts.push(account);
    }
    save_accounts_in(dir, &accounts)?;
    Ok(accounts)
}

/// Remove `account_id` from `accounts.toml` inside `dir`.
pub fn delete_account_in(dir: &Path, account_id: u32) -> Result<Vec<AccountConfig>, String> {
    let mut accounts = load_accounts_in(dir)?;
    accounts.retain(|a| a.id != account_id);
    save_accounts_in(dir, &accounts)?;
    Ok(accounts)
}

/// Return the next unused account ID for `accounts`.
pub fn next_account_id(accounts: &[AccountConfig]) -> u32 {
    accounts.iter().map(|a| a.id).max().unwrap_or(0) + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accounts_roundtrip_handles_chinese_and_spaces_in_path() {
        let base = std::env::temp_dir().join("hylki test 测试目录");
        let _ = fs::remove_dir_all(&base);

        let account = AccountConfig {
            id: 1,
            auth_backend: AuthBackend::Password,
            oauth_client_id: String::new(),
            display_name: "测试账号".to_string(),
            email: "user@example.com".to_string(),
            imap_host: "imap.example.com".to_string(),
            imap_port: 993,
            imap_security: SecurityMode::Tls,
            imap_username: "user@example.com".to_string(),
            smtp_host: "smtp.example.com".to_string(),
            smtp_port: 465,
            smtp_security: SecurityMode::Tls,
            smtp_username: "user@example.com".to_string(),
            smtp_separate_credentials: false,
        };

        upsert_account_in(&base, account.clone()).expect("upsert should succeed");
        let loaded = load_accounts_in(&base).expect("load should succeed");
        assert_eq!(loaded, vec![account]);

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn preferences_roundtrip_in_temp_dir() {
        let base = std::env::temp_dir().join("hylki test 偏好目录");
        let _ = fs::remove_dir_all(&base);

        assert_eq!(preferences_file_in(&base), base.join("preferences.toml"));

        // Missing file yields defaults without error.
        let defaults = load_preferences_in(&base).expect("load missing file");
        assert_eq!(defaults, AppPreferences::default());
        assert_eq!(defaults.theme_id, "default");
        assert_eq!(defaults.dark_mode, None);
        assert_eq!(defaults.page_size, 100);
        assert_eq!(defaults.auto_sync_minutes, 5);
        assert!(defaults.block_remote_images);

        // Round-trip including dark_mode = None (skipped in TOML output).
        let prefs = AppPreferences {
            theme_id: "rose".to_string(),
            dark_mode: Some(true),
            compact_list: true,
            page_size: 50,
            auto_sync_minutes: 0,
            block_remote_images: false,
            close_to_minimize: false,
            default_view: "last".to_string(),
            last_view: None,
            last_folder: None,
            thread_newest_first: false,
            fold_older_messages: false,
            allowed_remote_senders: vec!["trusted@example.com".to_string()],
            reader_zoom_percent: 125,
        };
        save_preferences_in(&base, &prefs).expect("save should succeed");
        let loaded = load_preferences_in(&base).expect("load should succeed");
        assert_eq!(loaded, prefs);

        let none_dark = AppPreferences {
            dark_mode: None,
            ..prefs.clone()
        };
        save_preferences_in(&base, &none_dark).expect("save None dark_mode should succeed");
        assert_eq!(load_preferences_in(&base).expect("reload"), none_dark);

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn preferences_partial_file_fills_defaults() {
        let base = std::env::temp_dir().join("hylki test 偏好目录2");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        fs::write(
            preferences_file_in(&base),
            "theme_id = 'rose'\ncompact_list = true\n",
        )
        .expect("write partial prefs");

        let loaded = load_preferences_in(&base).expect("load partial file");
        assert_eq!(loaded.theme_id, "rose");
        assert!(loaded.compact_list);
        assert_eq!(loaded.dark_mode, None);
        assert_eq!(loaded.page_size, 100);
        assert_eq!(loaded.auto_sync_minutes, 5);
        assert!(loaded.block_remote_images);

        let _ = fs::remove_dir_all(&base);
    }

    /// 阶段 3：last_view / last_folder 字段往返 + 旧文件缺字段兼容。
    #[test]
    fn preferences_last_view_and_folder_roundtrip() {
        let base = std::env::temp_dir().join("hylki test 偏好上次视图");
        let _ = fs::remove_dir_all(&base);

        // 旧格式文件（无 last_view / last_folder 字段）加载后为 None。
        fs::create_dir_all(&base).unwrap();
        fs::write(
            preferences_file_in(&base),
            "theme_id = 'default'\ndefault_view = 'last'\n",
        )
        .expect("write legacy prefs");
        let legacy = load_preferences_in(&base).expect("load legacy file");
        assert_eq!(legacy.last_view, None);
        assert_eq!(legacy.last_folder, None);

        // 往返：同时持久化 last_view 与 last_folder。
        let prefs = AppPreferences {
            last_view: Some("sent".to_string()),
            last_folder: Some((3, 17)),
            ..AppPreferences::default()
        };
        save_preferences_in(&base, &prefs).expect("save should succeed");
        let loaded = load_preferences_in(&base).expect("load should succeed");
        assert_eq!(loaded.last_view.as_deref(), Some("sent"));
        assert_eq!(loaded.last_folder, Some((3, 17)));
        assert_eq!(loaded, prefs);

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn remote_sender_whitelist_and_zoom_clamping_work() {
        let mut prefs = AppPreferences::default();
        assert!(prefs.thread_newest_first);
        assert!(prefs.fold_older_messages);
        assert_eq!(prefs.reader_zoom_percent, 100);
        assert!(prefs.allowed_remote_senders.is_empty());

        assert!(!prefs.is_remote_sender_allowed("Alice <Alice@Example.COM>"));
        let added = prefs.allow_remote_sender("Alice <Alice@Example.COM>");
        assert_eq!(added.as_deref(), Some("alice@example.com"));
        assert_eq!(prefs.allowed_remote_senders, vec!["alice@example.com"]);

        // 重复添加不会产生重复项，且大小写不敏感匹配。
        let again = prefs.allow_remote_sender("alice@example.com");
        assert_eq!(again.as_deref(), Some("alice@example.com"));
        assert_eq!(prefs.allowed_remote_senders.len(), 1);
        assert!(prefs.is_remote_sender_allowed("ALICE@EXAMPLE.COM"));
        assert!(prefs.is_remote_sender_allowed("Display Name <alice@example.com>"));
        assert!(!prefs.is_remote_sender_allowed("bob@example.com"));

        // 无效发件人（不含 @）不写入白名单。
        assert_eq!(prefs.allow_remote_sender("(未知发件人)"), None);

        // 字号范围钳制在 70..=160。
        assert_eq!(clamp_reader_zoom(50), 70);
        assert_eq!(clamp_reader_zoom(100), 100);
        assert_eq!(clamp_reader_zoom(220), 160);
    }
}
