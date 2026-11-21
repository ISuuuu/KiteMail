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
}
