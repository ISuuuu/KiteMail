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
    /// Optional user-defined nickname shown in the sidebar and account list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Whether this account is enabled for syncing and shown in the sidebar.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Optional custom accent color (`#rrggbb`) for account badges and avatars.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Optional emoji shown in the account circle instead of initials.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
    /// Optional avatar picture file name stored under `avatars/`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    /// Whether this account's mail is included in unified views (All Inboxes, Starred, Sent, Archive).
    #[serde(default = "default_true")]
    pub in_unified: bool,
    /// Per-account push override (`None` follows global setting, `Some(true/false)` forces on/off).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub push_override: Option<bool>,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_security: SecurityMode,
    pub imap_username: String,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_security: SecurityMode,
    pub smtp_username: String,
    pub smtp_separate_credentials: bool,
    /// Optional plain-text email signature appended to newly composed / replied / forwarded mails.
    #[serde(default)]
    pub signature: String,
}

impl AccountConfig {
    /// Return the custom nickname when non-empty, otherwise fall back to email.
    pub fn display_label(&self) -> String {
        self.label
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(&self.email)
            .to_string()
    }

    /// Resolve effective push policy for this account given the global `push_enabled`.
    pub fn effective_push(&self, global_push: bool) -> bool {
        self.push_override.unwrap_or(global_push)
    }
}

impl Default for AccountConfig {
    fn default() -> Self {
        Self {
            id: 1,
            auth_backend: AuthBackend::Password,
            oauth_client_id: String::new(),
            display_name: String::new(),
            email: String::new(),
            label: None,
            enabled: true,
            color: None,
            emoji: None,
            avatar: None,
            in_unified: true,
            push_override: None,
            imap_host: String::new(),
            imap_port: 993,
            imap_security: SecurityMode::Tls,
            imap_username: String::new(),
            smtp_host: String::new(),
            smtp_port: 465,
            smtp_security: SecurityMode::Tls,
            smtp_username: String::new(),
            smtp_separate_credentials: false,
            signature: String::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct AccountsFile {
    #[serde(default)]
    accounts: Vec<AccountConfig>,
}

pub const DEFAULT_ATTACHMENT_KEYWORDS: &str =
    "附件, 附上, 随信, 见附, attachment, attached, enclose";

/// User interface & sync preferences persisted in `preferences.toml`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    /// Enable instant mail push (IMAP IDLE / fast Graph pulse).
    #[serde(default = "default_true")]
    pub push_enabled: bool,
    /// Block remote images in HTML mail bodies.
    pub block_remote_images: bool,
    /// Hide the window to the system tray instead of exiting when clicking close.
    pub close_to_minimize: bool,
    /// Hide the window to the system tray when minimizing.
    #[serde(default = "default_true")]
    pub minimize_to_tray: bool,
    /// Show desktop notifications when background sync finds new unread mail.
    #[serde(default = "default_true")]
    pub desktop_notifications: bool,
    /// Show sender and subject in desktop notifications.
    #[serde(default = "default_true")]
    pub notification_show_content: bool,
    /// UI text scale percentage (`90` / `100` / `110` / `120` / `135` / `150`).
    #[serde(default = "default_ui_text_scale_percent")]
    pub ui_text_scale_percent: u32,
    /// Whether the settings window opens directly to Mail Accounts.
    #[serde(default)]
    pub settings_open_accounts: bool,
    /// Always fold the right reader toolbar button group into overflow menu.
    #[serde(default)]
    pub reader_toolbar_fold_right: bool,
    /// Start in Focus Mode on launch.
    #[serde(default)]
    pub focus_start_enabled: bool,
    /// In Focus Mode, hide the left sidebar.
    #[serde(default = "default_true")]
    pub focus_hide_sidebar: bool,
    /// In Focus Mode, hide the message list pane.
    #[serde(default = "default_true")]
    pub focus_hide_list: bool,
    /// In Focus Mode, open messages in simplified Reader View.
    #[serde(default = "default_true")]
    pub focus_reader_view: bool,
    /// In Focus Mode, fold low-frequency reader toolbar buttons into overflow menu.
    #[serde(default = "default_true")]
    pub focus_fold_toolbar: bool,
    /// Show accounts section in the sidebar.
    #[serde(default = "default_true")]
    pub show_accounts: bool,
    /// Show Attachments entry in the sidebar footer.
    #[serde(default = "default_true")]
    pub sidebar_show_attachments: bool,
    /// Show Contacts entry in the sidebar footer.
    #[serde(default = "default_true")]
    pub sidebar_show_contacts: bool,
    /// Remember sidebar collapsed (icon rail) state across restarts.
    #[serde(default = "default_true")]
    pub remember_sidebar_collapsed: bool,
    /// Persisted sidebar collapsed (icon rail) state.
    #[serde(default)]
    pub sidebar_collapsed: bool,
    /// Unified sidebar rows visibility.
    #[serde(default = "default_true")]
    pub unified_show_inbox: bool,
    #[serde(default = "default_true")]
    pub unified_show_starred: bool,
    #[serde(default = "default_true")]
    pub unified_show_sent: bool,
    #[serde(default = "default_true")]
    pub unified_show_drafts: bool,
    #[serde(default = "default_true")]
    pub unified_show_archive: bool,
    /// Show unread dots instead of numeric counts in icon rail mode.
    #[serde(default = "default_true")]
    pub rail_unread_dots: bool,
    /// Show sender avatars in message list rows.
    #[serde(default = "default_true")]
    pub show_avatars: bool,
    /// Group message list rows by date headers.
    #[serde(default = "default_true")]
    pub list_group_by_date: bool,
    /// Keep the message list search bar always open.
    #[serde(default)]
    pub list_search_always_open: bool,
    /// Place the message list search bar at the bottom instead of top.
    #[serde(default)]
    pub list_search_bottom: bool,
    /// Place the multi-selection bulk bar at the bottom instead of top.
    #[serde(default)]
    pub list_bulk_bar_bottom: bool,
    /// Persisted window width in logical pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_width: Option<f32>,
    /// Persisted window height in logical pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_height: Option<f32>,
    /// Whether the window was maximized when last closed/exited.
    #[serde(default)]
    pub window_maximized: bool,
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
    #[serde(default)]
    pub thread_newest_first: bool,
    /// Automatically fold older messages when opening a multi-message thread.
    #[serde(default)]
    pub fold_older_messages: bool,
    /// Explicit thread fold mode: `"never"` / `"read"` / `"all_but_newest"`.
    /// When `None`, derived from legacy [`Self::fold_older_messages`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fold_messages_mode: Option<String>,
    /// Show attachments directly on each conversation message card.
    #[serde(default = "default_true")]
    pub card_attachments: bool,
    /// Automatic mark-as-read timing: `"shown"` / `"delay"` / `"manual"`.
    #[serde(default = "default_read_mark_mode")]
    pub read_mark_mode: String,
    /// Message body appearance theme: `"system"` / `"light"` / `"dark"`.
    #[serde(default = "default_message_theme")]
    pub message_theme: String,
    /// Show per-message Light/Dark toggle in the reader header.
    #[serde(default = "default_true")]
    pub reader_show_theme_switch: bool,
    /// Show account email pill in the reader header.
    #[serde(default = "default_true")]
    pub reader_show_account_chip: bool,
    /// Render standalone single messages inside a raised card.
    #[serde(default = "default_true")]
    pub single_message_card: bool,
    /// Always expand the recipients line in reader message cards.
    #[serde(default)]
    pub always_show_recipients: bool,
    /// Show simplified Reader View toggle button in the reader header.
    #[serde(default = "default_true")]
    pub reader_show_view_switch: bool,
    /// Default Reader View mode when opening messages: `"remember"` / `"on"` / `"off"`.
    #[serde(default = "default_reader_default_mode")]
    pub reader_default_mode: String,
    /// Default sender account email for new messages (`""` = auto / current account).
    #[serde(default)]
    pub compose_default_from: String,
    /// Place signature above quoted text when replying or forwarding.
    #[serde(default = "default_true")]
    pub signature_above_quote: bool,
    /// Insert standard `"-- "` separator before signatures.
    #[serde(default)]
    pub signature_dashes: bool,
    /// Undo send delay in seconds (`0` disables undo send).
    #[serde(default = "default_undo_send_secs")]
    pub undo_send_secs: u32,
    /// Custom reply attribution header template (`""` uses built-in Chinese header).
    #[serde(default)]
    pub reply_header_template: String,
    /// Comma-separated keywords that trigger missing-attachment reminder before sending.
    #[serde(default = "default_attachment_keywords")]
    pub attachment_reminder_keywords: String,
    /// Show blocked remote content banner in reader cards.
    #[serde(default = "default_true")]
    pub show_remote_banner: bool,
    /// Strip common tracking parameters from HTML links when rendering messages.
    #[serde(default = "default_true")]
    pub clean_links: bool,
    /// Date display arrangement: `"system"` / `"month_first"` / `"day_first"` / `"year_first"`.
    #[serde(default = "default_system_key")]
    pub date_style: String,
    /// Clock format: `"system"` / `"12h"` / `"24h"`.
    #[serde(default = "default_system_key")]
    pub clock_style: String,
    /// Launch main window in fullscreen mode.
    #[serde(default)]
    pub start_fullscreen: bool,
    /// Place the bottom status bar at the top of the main window.
    #[serde(default)]
    pub status_bar_top: bool,
    /// Lowercased sender email addresses always allowed to load remote images.
    #[serde(default)]
    pub allowed_remote_senders: Vec<String>,
    /// Reader body zoom percentage (`50..=250`, default `100`).
    #[serde(default = "default_reader_zoom_percent")]
    pub reader_zoom_percent: u32,
    /// Enable the Gmail-style single-key shortcuts (`j` / `r` / `a` ...);
    /// Ctrl combos always work regardless of this flag.
    #[serde(default = "default_true")]
    pub single_key_shortcuts: bool,
    /// L-2：左栏（侧边栏）宽度，逻辑像素（180~280）；`None` 用默认 240。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sidebar_width: Option<f32>,
    /// L-2：中栏邮件列表宽度，逻辑像素（≥280）；`None` 用默认 360。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list_width: Option<f32>,
    /// L-6：阅读器位置（`"right"` 右侧 / `"below"` 列表下方 / `"off"` 关闭）。
    #[serde(default = "default_reader_position")]
    pub reader_position: String,
    /// L-6：阅读器在下方时列表的高度，逻辑像素；`None` 用默认 360。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list_below_height: Option<f32>,
}

fn default_true() -> bool {
    true
}

fn default_ui_text_scale_percent() -> u32 {
    100
}

fn default_reader_zoom_percent() -> u32 {
    100
}

fn default_reader_position() -> String {
    "right".to_string()
}

fn default_read_mark_mode() -> String {
    "shown".to_string()
}

fn default_message_theme() -> String {
    "system".to_string()
}

fn default_reader_default_mode() -> String {
    "remember".to_string()
}

fn default_undo_send_secs() -> u32 {
    5
}

fn default_attachment_keywords() -> String {
    DEFAULT_ATTACHMENT_KEYWORDS.to_string()
}

fn default_system_key() -> String {
    "system".to_string()
}

/// Clamp reader zoom percentage to the supported `50..=250` range.
pub fn clamp_reader_zoom(percent: u32) -> u32 {
    percent.clamp(50, 250)
}

/// Supported interface text scale percentages.
pub const UI_TEXT_SCALE_STEPS: &[u32] = &[90, 100, 110, 120, 135, 150];

/// Snap/clamp UI text scale percentage to the nearest supported step (`90..=150`).
pub fn clamp_ui_text_scale(percent: u32) -> u32 {
    *UI_TEXT_SCALE_STEPS
        .iter()
        .min_by_key(|&&step| step.abs_diff(percent))
        .unwrap_or(&100)
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

    /// Remove `sender`'s normalized email address from [`Self::allowed_remote_senders`].
    /// Returns `true` if at least one entry was removed.
    pub fn remove_remote_sender(&mut self, sender: &str) -> bool {
        let email = normalize_sender_email(sender);
        let before = self.allowed_remote_senders.len();
        self.allowed_remote_senders
            .retain(|existing| !existing.eq_ignore_ascii_case(&email));
        self.allowed_remote_senders.len() != before
    }

    /// Resolve effective thread fold mode (`"never"`, `"read"`, or `"all_but_newest"`),
    /// migrating seamlessly from legacy [`Self::fold_older_messages`] when unset.
    pub fn effective_fold_messages_mode(&self) -> &'static str {
        match self.fold_messages_mode.as_deref() {
            Some("never") => "never",
            Some("read") => "read",
            Some("all_but_newest") => "all_but_newest",
            _ => {
                if self.fold_older_messages {
                    "all_but_newest"
                } else {
                    "never"
                }
            }
        }
    }

    /// Set thread fold mode (`"never"` / `"read"` / `"all_but_newest"`) and keep
    /// legacy [`Self::fold_older_messages`] synchronized.
    pub fn set_fold_messages_mode(&mut self, mode: &str) {
        let normalized = match mode {
            "never" => "never",
            "read" => "read",
            _ => "all_but_newest",
        };
        self.fold_messages_mode = Some(normalized.to_string());
        self.fold_older_messages = normalized != "never";
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
            push_enabled: true,
            block_remote_images: true,
            close_to_minimize: true,
            minimize_to_tray: true,
            desktop_notifications: true,
            notification_show_content: true,
            ui_text_scale_percent: 100,
            settings_open_accounts: false,
            reader_toolbar_fold_right: false,
            focus_start_enabled: false,
            focus_hide_sidebar: true,
            focus_hide_list: true,
            focus_reader_view: true,
            focus_fold_toolbar: true,
            show_accounts: true,
            sidebar_show_attachments: true,
            sidebar_show_contacts: true,
            remember_sidebar_collapsed: true,
            sidebar_collapsed: false,
            unified_show_inbox: true,
            unified_show_starred: true,
            unified_show_sent: true,
            unified_show_drafts: true,
            unified_show_archive: true,
            rail_unread_dots: true,
            show_avatars: true,
            list_group_by_date: true,
            list_search_always_open: false,
            list_search_bottom: false,
            list_bulk_bar_bottom: false,
            window_width: None,
            window_height: None,
            window_maximized: false,
            default_view: "unified".to_string(),
            last_view: None,
            last_folder: None,
            thread_newest_first: false,
            fold_older_messages: false,
            fold_messages_mode: None,
            card_attachments: true,
            read_mark_mode: "shown".to_string(),
            message_theme: "system".to_string(),
            reader_show_theme_switch: true,
            reader_show_account_chip: true,
            single_message_card: true,
            always_show_recipients: false,
            reader_show_view_switch: true,
            reader_default_mode: "remember".to_string(),
            compose_default_from: String::new(),
            signature_above_quote: true,
            signature_dashes: false,
            undo_send_secs: 5,
            reply_header_template: String::new(),
            attachment_reminder_keywords: DEFAULT_ATTACHMENT_KEYWORDS.to_string(),
            show_remote_banner: true,
            clean_links: true,
            date_style: "system".to_string(),
            clock_style: "system".to_string(),
            start_fullscreen: false,
            status_bar_top: false,
            allowed_remote_senders: Vec::new(),
            reader_zoom_percent: 100,
            single_key_shortcuts: true,
            sidebar_width: None,
            list_width: None,
            reader_position: "right".to_string(),
            list_below_height: None,
        }
    }
}

/// Atomically write UTF-8 contents via a same-directory temp file.
fn atomic_write_file(dir: &Path, target: &Path, content: &str, kind: &str) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("创建数据目录失败 ({}): {e}", dir.display()))?;
    let file_name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("config.toml");
    let tmp_path = dir.join(format!(".{file_name}.{}.tmp", std::process::id()));
    fs::write(&tmp_path, content)
        .map_err(|e| format!("写入临时{kind}失败 ({}): {e}", tmp_path.display()))?;
    if let Err(first_err) = fs::rename(&tmp_path, target) {
        // Windows `MoveFileEx` via `std::fs::rename` replaces existing files on modern NTFS,
        // but fallback gracefully if an existing target blocks direct rename.
        if target.exists() {
            let _ = fs::remove_file(target);
            if let Err(second_err) = fs::rename(&tmp_path, target) {
                let _ = fs::remove_file(&tmp_path);
                return Err(format!(
                    "替换{kind}失败 ({}): {second_err}",
                    target.display()
                ));
            }
        } else {
            let _ = fs::remove_file(&tmp_path);
            return Err(format!(
                "写入{kind}失败 ({}): {first_err}",
                target.display()
            ));
        }
    }
    Ok(())
}

/// Return the Hylki Windows local data directory (`%LOCALAPPDATA%\Hylki`).
pub fn data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Hylki")
}

/// Return the directory storing imported custom account avatars under `dir`.
pub fn avatars_dir_in(dir: &Path) -> PathBuf {
    dir.join("avatars")
}

/// Resolve a stored avatar file name inside `dir/avatars/`, rejecting path traversal.
pub fn avatar_path_in(dir: &Path, name: &str) -> Option<PathBuf> {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains("..")
        || trimmed.contains('\0')
    {
        return None;
    }
    let mut comps = Path::new(trimmed).components();
    match (comps.next(), comps.next()) {
        (Some(std::path::Component::Normal(_)), None) => {}
        _ => return None,
    }
    let p = avatars_dir_in(dir).join(trimmed);
    p.is_file().then_some(p)
}

/// Validate and copy an image file into `dir/avatars/avatar-<unix_ms>.<ext>`.
pub fn import_avatar_file_in(dir: &Path, src: &Path) -> Result<String, String> {
    const MAX_AVATAR_BYTES: u64 = 8 * 1024 * 1024;
    let meta =
        fs::metadata(src).map_err(|e| format!("无法读取所选图片文件 ({}): {e}", src.display()))?;
    if !meta.is_file() {
        return Err("请选择有效的图片文件".to_string());
    }
    if meta.len() > MAX_AVATAR_BYTES {
        return Err("头像图片不能超过 8 MB".to_string());
    }
    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    if !matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif") {
        return Err("仅支持 PNG、JPG、WebP 或 GIF 格式的图片".to_string());
    }
    let avatars_dir = avatars_dir_in(dir);
    fs::create_dir_all(&avatars_dir)
        .map_err(|e| format!("创建头像目录失败 ({}): {e}", avatars_dir.display()))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let mut name = format!("avatar-{stamp}.{ext}");
    let mut dest = avatars_dir.join(&name);
    let mut seq = 1u32;
    while dest.exists() {
        name = format!("avatar-{stamp}-{seq}.{ext}");
        dest = avatars_dir.join(&name);
        seq = seq.saturating_add(1);
    }
    fs::copy(src, &dest).map_err(|e| format!("复制头像文件失败 ({}): {e}", dest.display()))?;
    Ok(name)
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
    let path = preferences_file_in(dir);
    let text = toml::to_string_pretty(prefs).map_err(|e| format!("序列化偏好设置失败: {e}"))?;
    atomic_write_file(dir, &path, &text, "偏好设置")
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
    let path = accounts_file_in(dir);
    let data = AccountsFile {
        accounts: accounts.to_vec(),
    };
    let text = toml::to_string_pretty(&data).map_err(|e| format!("序列化账号配置失败: {e}"))?;
    atomic_write_file(dir, &path, &text, "账号配置")
}

/// Current portable settings backup bundle version.
pub const SETTINGS_BACKUP_VERSION: u32 = 1;

/// Portable TOML settings backup bundle (excludes passwords, OAuth tokens, and cached mail).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SettingsBackupBundle {
    pub version: u32,
    #[serde(default)]
    pub preferences: AppPreferences,
    #[serde(default)]
    pub accounts: Vec<AccountConfig>,
}

/// Export current `preferences.toml` and `accounts.toml` in `dir` as a TOML backup string.
pub fn export_settings_bundle_in(dir: &Path) -> Result<String, String> {
    let preferences = load_preferences_in(dir)?;
    let accounts = load_accounts_in(dir)?;
    let bundle = SettingsBackupBundle {
        version: SETTINGS_BACKUP_VERSION,
        preferences,
        accounts,
    };
    toml::to_string_pretty(&bundle).map_err(|e| format!("序列化设置备份失败: {e}"))
}

/// Parse and validate a settings backup TOML bundle (`version == 1`).
pub fn parse_settings_bundle(text: &str) -> Result<SettingsBackupBundle, String> {
    let bundle: SettingsBackupBundle =
        toml::from_str(text).map_err(|e| format!("解析设置备份文件失败: {e}"))?;
    if bundle.version != SETTINGS_BACKUP_VERSION {
        return Err(format!("不支持的备份版本: {}", bundle.version));
    }
    Ok(bundle)
}

/// Import a validated [`SettingsBackupBundle`] into `dir` without touching credentials.
/// Returns the number of restored accounts.
pub fn import_settings_bundle_in(
    dir: &Path,
    bundle: &SettingsBackupBundle,
) -> Result<usize, String> {
    if bundle.version != SETTINGS_BACKUP_VERSION {
        return Err(format!("不支持的备份版本: {}", bundle.version));
    }
    save_preferences_in(dir, &bundle.preferences)?;
    save_accounts_in(dir, &bundle.accounts)?;
    Ok(bundle.accounts.len())
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

/// Reorder saved accounts in `accounts.toml` according to `ordered_ids`.
/// Any existing accounts not listed in `ordered_ids` are preserved in order at the end.
pub fn reorder_accounts_in(dir: &Path, ordered_ids: &[u32]) -> Result<Vec<AccountConfig>, String> {
    let mut remaining = load_accounts_in(dir)?;
    let mut reordered = Vec::with_capacity(remaining.len());
    for &id in ordered_ids {
        if let Some(pos) = remaining.iter().position(|a| a.id == id) {
            reordered.push(remaining.remove(pos));
        }
    }
    reordered.extend(remaining);
    save_accounts_in(dir, &reordered)?;
    Ok(reordered)
}

/// Toggle `enabled` for `account_id` in `accounts.toml` inside `dir`.
pub fn set_account_enabled_in(
    dir: &Path,
    account_id: u32,
    enabled: bool,
) -> Result<Option<AccountConfig>, String> {
    let mut accounts = load_accounts_in(dir)?;
    let Some(acc) = accounts.iter_mut().find(|a| a.id == account_id) else {
        return Ok(None);
    };
    acc.enabled = enabled;
    let updated = acc.clone();
    save_accounts_in(dir, &accounts)?;
    Ok(Some(updated))
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
            label: Some("工作邮箱".to_string()),
            enabled: true,
            color: Some("#3584e4".to_string()),
            emoji: Some("🦊".to_string()),
            avatar: None,
            in_unified: true,
            push_override: Some(false),
            imap_host: "imap.example.com".to_string(),
            imap_port: 993,
            imap_security: SecurityMode::Tls,
            imap_username: "user@example.com".to_string(),
            smtp_host: "smtp.example.com".to_string(),
            smtp_port: 465,
            smtp_security: SecurityMode::Tls,
            smtp_username: "user@example.com".to_string(),
            smtp_separate_credentials: false,
            signature: "Best regards,\nTester".to_string(),
        };

        upsert_account_in(&base, account.clone()).expect("upsert should succeed");
        let loaded = load_accounts_in(&base).expect("load should succeed");
        assert_eq!(loaded, vec![account]);

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn accounts_missing_signature_defaults_to_empty() {
        let base = std::env::temp_dir().join("hylki test 旧账号无签名");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let legacy_toml = r#"
[[accounts]]
id = 1
display_name = "Legacy"
email = "legacy@example.com"
imap_host = "imap.example.com"
imap_port = 993
imap_security = "Tls"
imap_username = "legacy@example.com"
smtp_host = "smtp.example.com"
smtp_port = 465
smtp_security = "Tls"
smtp_username = "legacy@example.com"
smtp_separate_credentials = false
"#;
        fs::write(accounts_file_in(&base), legacy_toml).unwrap();
        let loaded = load_accounts_in(&base).expect("load legacy account");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].signature, "");
        assert!(loaded[0].enabled);
        assert!(loaded[0].in_unified);
        assert_eq!(loaded[0].label, None);
        assert_eq!(loaded[0].color, None);
        assert_eq!(loaded[0].emoji, None);
        assert_eq!(loaded[0].avatar, None);
        assert_eq!(loaded[0].push_override, None);
        assert_eq!(loaded[0].display_label(), "legacy@example.com");
        assert!(loaded[0].effective_push(true));
        assert!(!loaded[0].effective_push(false));
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn account_reorder_enable_and_avatar_import_work() {
        let base = std::env::temp_dir().join("hylki test 账号排序与头像");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();

        let a1 = AccountConfig {
            id: 1,
            email: "one@example.com".to_string(),
            label: Some("  工作号  ".to_string()),
            push_override: Some(false),
            ..AccountConfig::default()
        };
        let a2 = AccountConfig {
            id: 2,
            email: "two@example.com".to_string(),
            push_override: Some(true),
            ..AccountConfig::default()
        };
        let a3 = AccountConfig {
            id: 3,
            email: "three@example.com".to_string(),
            ..AccountConfig::default()
        };
        assert_eq!(a1.display_label(), "工作号");
        assert!(!a1.effective_push(true));
        assert!(a2.effective_push(false));

        save_accounts_in(&base, &[a1, a2, a3]).unwrap();
        let reordered = reorder_accounts_in(&base, &[3, 1]).unwrap();
        let ids: Vec<u32> = reordered.iter().map(|a| a.id).collect();
        assert_eq!(ids, vec![3, 1, 2]);

        let toggled = set_account_enabled_in(&base, 1, false).unwrap().unwrap();
        assert!(!toggled.enabled);
        let reloaded = load_accounts_in(&base).unwrap();
        assert!(!reloaded.iter().find(|a| a.id == 1).unwrap().enabled);

        // 头像导入与路径穿越防护。
        let src_png = base.join("sample.png");
        fs::write(&src_png, b"\x89PNG\r\n\x1a\n").unwrap();
        let saved_name = import_avatar_file_in(&base, &src_png).unwrap();
        assert!(saved_name.starts_with("avatar-"));
        assert!(saved_name.ends_with(".png"));
        assert!(avatar_path_in(&base, &saved_name).is_some());
        assert!(avatar_path_in(&base, "../sample.png").is_none());
        assert!(avatar_path_in(&base, "sub/file.png").is_none());
        assert!(avatar_path_in(&base, "sub\\file.png").is_none());
        assert!(avatar_path_in(&base, "").is_none());

        let bad_ext = base.join("script.exe");
        fs::write(&bad_ext, b"MZ").unwrap();
        assert!(import_avatar_file_in(&base, &bad_ext).is_err());

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
            minimize_to_tray: false,
            desktop_notifications: false,
            window_width: Some(1440.0),
            window_height: Some(900.0),
            window_maximized: true,
            default_view: "last".to_string(),
            last_view: None,
            last_folder: None,
            thread_newest_first: false,
            fold_older_messages: false,
            allowed_remote_senders: vec!["trusted@example.com".to_string()],
            reader_zoom_percent: 125,
            single_key_shortcuts: false,
            sidebar_width: None,
            list_width: None,
            reader_position: "right".to_string(),
            list_below_height: None,
            ..AppPreferences::default()
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
    fn preferences_tray_notifications_and_window_geometry_compat() {
        let base = std::env::temp_dir().join("hylki test 托盘与窗口几何");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();

        // 旧格式 TOML（缺少新字段）应自动填充向后兼容默认值。
        fs::write(
            preferences_file_in(&base),
            "theme_id = 'default'\nclose_to_minimize = true\n",
        )
        .expect("write legacy prefs");
        let legacy = load_preferences_in(&base).expect("load legacy file");
        assert!(legacy.minimize_to_tray);
        assert!(legacy.desktop_notifications);
        assert_eq!(legacy.window_width, None);
        assert_eq!(legacy.window_height, None);
        assert!(!legacy.window_maximized);
        // 阶段 3：旧文件缺 single_key_shortcuts 字段时默认开启。
        assert!(legacy.single_key_shortcuts);

        // 完整往返保存与读取。
        let custom = AppPreferences {
            minimize_to_tray: false,
            desktop_notifications: false,
            window_width: Some(1520.0),
            window_height: Some(920.0),
            window_maximized: true,
            ..AppPreferences::default()
        };
        save_preferences_in(&base, &custom).expect("save custom prefs");
        let reloaded = load_preferences_in(&base).expect("reload custom prefs");
        assert_eq!(reloaded, custom);

        let _ = fs::remove_dir_all(&base);
    }

    /// L-2/L-6：布局字段（宽度 / 阅读器位置 / 下方列表高度）默认值、旧文件兼容与往返。
    #[test]
    fn preferences_layout_fields_defaults_and_roundtrip() {
        let base = std::env::temp_dir().join("hylki test 偏好布局字段");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();

        // 旧格式文件（无布局字段）加载后回落默认值。
        fs::write(
            preferences_file_in(&base),
            "theme_id = 'default'\npage_size = 50\n",
        )
        .expect("write legacy prefs");
        let legacy = load_preferences_in(&base).expect("load legacy file");
        assert_eq!(legacy.sidebar_width, None);
        assert_eq!(legacy.list_width, None);
        assert_eq!(legacy.reader_position, "right");
        assert_eq!(legacy.list_below_height, None);

        // 往返：持久化全部布局字段。
        let prefs = AppPreferences {
            sidebar_width: Some(208.0),
            list_width: Some(640.0),
            reader_position: "below".to_string(),
            list_below_height: Some(420.0),
            ..AppPreferences::default()
        };
        save_preferences_in(&base, &prefs).expect("save should succeed");
        let loaded = load_preferences_in(&base).expect("load should succeed");
        assert_eq!(loaded, prefs);

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn remote_sender_whitelist_and_zoom_clamping_work() {
        let mut prefs = AppPreferences::default();
        // WP-04：对齐 Linux 默认值——旧邮件在上、不折叠会话。
        assert!(!prefs.thread_newest_first);
        assert!(!prefs.fold_older_messages);
        assert_eq!(prefs.reader_zoom_percent, 100);
        // 阶段 3：单键快捷键默认开启。
        assert!(prefs.single_key_shortcuts);
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

        // 字号范围钳制在 50..=250（WP-04 对齐 Linux READER_ZOOM_STEPS）。
        assert_eq!(clamp_reader_zoom(40), 50);
        assert_eq!(clamp_reader_zoom(50), 50);
        assert_eq!(clamp_reader_zoom(100), 100);
        assert_eq!(clamp_reader_zoom(220), 220);
        assert_eq!(clamp_reader_zoom(260), 250);

        // 移除发件人白名单（大小写不敏感与格式化邮箱）。
        assert!(prefs.remove_remote_sender("Alice <ALICE@EXAMPLE.COM>"));
        assert!(prefs.allowed_remote_senders.is_empty());
        assert!(!prefs.remove_remote_sender("missing@example.com"));
    }

    #[test]
    fn empty_preferences_file_yields_linux_aligned_defaults() {
        let base = std::env::temp_dir().join("hylki test 空偏好默认值");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();

        // 空文件走 serde 反序列化路径：全部字段按 serde/Default 回落，
        // 应与 `AppPreferences::default()` 完全一致（WP-04 对齐 Linux）。
        fs::write(preferences_file_in(&base), "").expect("write empty prefs");
        let parsed = load_preferences_in(&base).expect("load empty file");
        assert_eq!(parsed, AppPreferences::default());

        // 1. 会话内旧邮件在上（最新不置顶）。
        assert!(!parsed.thread_newest_first);
        // 2. 打开会话时不折叠较早邮件（"never"）。
        assert!(!parsed.fold_older_messages);
        assert_eq!(parsed.effective_fold_messages_mode(), "never");
        // 3. 收件人行默认不固定展开。
        assert!(!parsed.always_show_recipients);
        // 4. 签名默认位于引用内容上方。
        assert!(parsed.signature_above_quote);
        // 5. 签名前默认不插入 "-- " 分隔符。
        assert!(!parsed.signature_dashes);
        // 6. 专注模式默认开启简化阅读视图。
        assert!(parsed.focus_reader_view);
        // 7. 默认记住侧边栏折叠状态。
        assert!(parsed.remember_sidebar_collapsed);
        // 8. 窄栏模式默认以未读圆点代替计数。
        assert!(parsed.rail_unread_dots);
        // 9. 撤销发送默认 5 秒（档位 UI 提供 0/3/5/10/20/30）。
        assert_eq!(parsed.undo_send_secs, 5);
        // 10. 阅读器缩放默认 100%，档位范围扩展为 50..=250。
        assert_eq!(parsed.reader_zoom_percent, 100);
        assert_eq!(clamp_reader_zoom(250), 250);

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn fold_mode_migration_and_ui_text_scale_clamp() {
        let mut legacy_true = AppPreferences {
            fold_older_messages: true,
            fold_messages_mode: None,
            ..AppPreferences::default()
        };
        assert_eq!(legacy_true.effective_fold_messages_mode(), "all_but_newest");

        let legacy_false = AppPreferences {
            fold_older_messages: false,
            fold_messages_mode: None,
            ..AppPreferences::default()
        };
        assert_eq!(legacy_false.effective_fold_messages_mode(), "never");

        legacy_true.set_fold_messages_mode("read");
        assert_eq!(legacy_true.effective_fold_messages_mode(), "read");
        assert!(legacy_true.fold_older_messages);

        legacy_true.set_fold_messages_mode("never");
        assert_eq!(legacy_true.effective_fold_messages_mode(), "never");
        assert!(!legacy_true.fold_older_messages);

        assert_eq!(clamp_ui_text_scale(80), 90);
        assert_eq!(clamp_ui_text_scale(102), 100);
        assert_eq!(clamp_ui_text_scale(136), 135);
        assert_eq!(clamp_ui_text_scale(200), 150);
    }

    #[test]
    fn settings_backup_bundle_roundtrip_and_version_validation() {
        let base = std::env::temp_dir().join("hylki test 设置备份往返");
        let restore = std::env::temp_dir().join("hylki test 设置备份还原");
        let _ = fs::remove_dir_all(&base);
        let _ = fs::remove_dir_all(&restore);

        let mut prefs = AppPreferences::default();
        prefs.theme_id = "midnight".to_string();
        prefs.undo_send_secs = 10;
        prefs.date_style = "year_first".to_string();
        prefs.clock_style = "24h".to_string();
        save_preferences_in(&base, &prefs).expect("save prefs");
        upsert_account_in(
            &base,
            AccountConfig {
                id: 1,
                email: "alice@example.com".to_string(),
                display_name: "Alice".to_string(),
                ..AccountConfig::default()
            },
        )
        .expect("save account");

        let exported = export_settings_bundle_in(&base).expect("export bundle");
        let parsed = parse_settings_bundle(&exported).expect("parse exported bundle");
        assert_eq!(parsed.version, SETTINGS_BACKUP_VERSION);
        assert_eq!(parsed.preferences.theme_id, "midnight");
        assert_eq!(parsed.preferences.undo_send_secs, 10);
        assert_eq!(parsed.accounts.len(), 1);

        let count = import_settings_bundle_in(&restore, &parsed).expect("import bundle");
        assert_eq!(count, 1);
        assert_eq!(load_preferences_in(&restore).unwrap(), parsed.preferences);
        assert_eq!(load_accounts_in(&restore).unwrap(), parsed.accounts);

        // 版本不匹配或格式错误应拒绝。
        let bad_ver = exported.replace("version = 1", "version = 99");
        assert!(parse_settings_bundle(&bad_ver).is_err());
        assert!(parse_settings_bundle("not = [valid").is_err());

        let _ = fs::remove_dir_all(&base);
        let _ = fs::remove_dir_all(&restore);
    }
}
