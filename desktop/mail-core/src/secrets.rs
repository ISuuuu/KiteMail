//! Credential storage backed by Windows Credential Manager via `keyring`.
//!
//! All keyring operations run serially on a dedicated background thread so a
//! save followed immediately by a read or delete is never reordered.

use std::sync::mpsc;
use std::sync::OnceLock;

const SERVICE_NAME: &str = "co.hyprlab.Hylki.Windows";

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

/// Secrets associated with one mail account.
#[derive(Clone, Debug, Default)]
pub struct AccountSecrets {
    pub imap_password: String,
    pub smtp_password: Option<String>,
}

/// Save IMAP (and optional separate SMTP) credentials in Windows Credential Manager.
pub fn save_account_secrets(account_id: u32, secrets: AccountSecrets) -> Result<(), String> {
    run_on_keyring_thread(move || {
        let imap_entry = keyring::Entry::new(SERVICE_NAME, &imap_entry_name(account_id))
            .map_err(|e| format!("初始化 IMAP 凭据项失败: {e}"))?;
        imap_entry
            .set_password(&secrets.imap_password)
            .map_err(|e| format!("保存 IMAP 授权码失败: {e}"))?;

        let smtp_entry = keyring::Entry::new(SERVICE_NAME, &smtp_entry_name(account_id))
            .map_err(|e| format!("初始化 SMTP 凭据项失败: {e}"))?;
        match secrets.smtp_password {
            Some(pwd) if !pwd.is_empty() => smtp_entry
                .set_password(&pwd)
                .map_err(|e| format!("保存 SMTP 授权码失败: {e}"))?,
            _ => {
                let _ = smtp_entry.delete_credential();
            }
        }
        Ok(())
    })
}

/// Load IMAP (and optional separate SMTP) credentials from Windows Credential Manager.
pub fn load_account_secrets(account_id: u32) -> Result<AccountSecrets, String> {
    run_on_keyring_thread(move || {
        let imap_entry = keyring::Entry::new(SERVICE_NAME, &imap_entry_name(account_id))
            .map_err(|e| format!("初始化 IMAP 凭据项失败: {e}"))?;
        let imap_password = imap_entry
            .get_password()
            .map_err(|e| format!("读取 IMAP 授权码失败: {e}"))?;

        let smtp_entry = keyring::Entry::new(SERVICE_NAME, &smtp_entry_name(account_id))
            .map_err(|e| format!("初始化 SMTP 凭据项失败: {e}"))?;
        let smtp_password = match smtp_entry.get_password() {
            Ok(pwd) => Some(pwd),
            Err(keyring::Error::NoEntry) => None,
            Err(e) => return Err(format!("读取 SMTP 授权码失败: {e}")),
        };

        Ok(AccountSecrets {
            imap_password,
            smtp_password,
        })
    })
}

/// Remove saved credentials for `account_id` from Windows Credential Manager.
pub fn delete_account_secrets(account_id: u32) -> Result<(), String> {
    run_on_keyring_thread(move || {
        for key in [
            imap_entry_name(account_id),
            smtp_entry_name(account_id),
            oauth_refresh_entry_name(account_id),
        ] {
            if let Ok(entry) = keyring::Entry::new(SERVICE_NAME, &key) {
                let _ = entry.delete_credential();
            }
        }
        Ok(())
    })
}

/// Save an OAuth2 refresh token in Windows Credential Manager.
pub fn save_oauth_refresh_token(account_id: u32, refresh_token: String) -> Result<(), String> {
    run_on_keyring_thread(move || {
        let entry = keyring::Entry::new(SERVICE_NAME, &oauth_refresh_entry_name(account_id))
            .map_err(|e| format!("初始化 OAuth 凭据项失败: {e}"))?;
        entry
            .set_password(&refresh_token)
            .map_err(|e| format!("保存 OAuth 刷新令牌失败: {e}"))?;
        Ok(())
    })
}

/// Load an OAuth2 refresh token from Windows Credential Manager.
pub fn load_oauth_refresh_token(account_id: u32) -> Result<String, String> {
    run_on_keyring_thread(move || {
        let entry = keyring::Entry::new(SERVICE_NAME, &oauth_refresh_entry_name(account_id))
            .map_err(|e| format!("初始化 OAuth 凭据项失败: {e}"))?;
        entry
            .get_password()
            .map_err(|e| format!("读取 OAuth 刷新令牌失败（请重新执行 OAuth2 登录）: {e}"))
    })
}
