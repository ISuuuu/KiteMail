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

/// WP-06：发信别名配置（对齐 Linux `src/config/account.rs` AliasConfig）。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SendAsAlias {
    pub name: String,
    pub email: String,
    pub reply_to: String,
    pub is_default: bool,
}

/// WP-06：多命名 HTML/文本签名配置（对齐 Linux `NamedSignature`）。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NamedSignature {
    pub name: String,
    pub body: String,
    pub is_html: bool,
}

/// WP-06：账号级特殊文件夹映射与 `server_saves_sent` 策略。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpecialFoldersConfig {
    pub sent: String,
    pub drafts: String,
    pub trash: String,
    pub junk: String,
    pub archive: String,
    pub server_saves_sent: bool,
}

/// 根据邮箱域名判断 `server_saves_sent` 默认值（Gmail/Outlook/iCloud 默认 true，自建 IMAP 默认 false）。
pub fn default_server_saves_sent_for_email(email: &str) -> bool {
    let domain = email
        .trim()
        .rsplit('@')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        domain.as_str(),
        "gmail.com"
            | "googlemail.com"
            | "outlook.com"
            | "hotmail.com"
            | "live.com"
            | "msn.com"
            | "icloud.com"
            | "me.com"
            | "mac.com"
    )
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
    /// WP-06：缓存的 IMAP `GETQUOTAROOT` 已用字节数（`None` 表示服务端未返回配额）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_used_bytes: Option<u64>,
    /// WP-06：缓存的 IMAP `GETQUOTAROOT` 总配额字节数。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota_total_bytes: Option<u64>,
    /// WP-06：发信别名列表。
    #[serde(default)]
    pub aliases: Vec<SendAsAlias>,
    /// WP-06：多命名 HTML/文本签名列表。
    #[serde(default)]
    pub signatures: Vec<NamedSignature>,
    /// WP-06：新邮件默认签名索引。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_new_signature: Option<usize>,
    /// WP-06：回复/转发默认签名索引。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_reply_signature: Option<usize>,
    /// WP-06：特殊文件夹映射与服务器自动保存已发送邮件开关。
    #[serde(default)]
    pub special_folders: SpecialFoldersConfig,
    /// WP-06：账号级轮询周期覆盖（`None` 跟随全局，`Some(0)` 仅手动，`Some(n)` 每 n 分钟）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_interval_minutes: Option<u32>,
    /// WP-06：`Download messages from` 天数窗口（`0` = 全部，`30` / `90` / `365`）。
    #[serde(default)]
    pub download_window_days: u32,
    /// WP-06：自动清空废纸篓天数（`0` = 从不，`7` / `14` / `30`）。
    #[serde(default)]
    pub auto_empty_trash_days: u32,
    /// WP-06：自动清空垃圾邮件天数（`0` = 从不，`7` / `14` / `30`）。
    #[serde(default)]
    pub auto_empty_junk_days: u32,
    /// WP-06：账号级隐藏文件夹路径列表。
    #[serde(default)]
    pub hidden_folders: Vec<String>,
    /// 自定义的账号本地邮件目录名（位于 `mail_data_dir` 下；`None` 时按 `email` 自动归一化推导）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_dir: Option<String>,
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

    fn resolve_signature(&self, index: Option<usize>) -> (String, bool) {
        if !self.signatures.is_empty() {
            return index
                .and_then(|idx| self.signatures.get(idx))
                .map(|sig| (sig.body.clone(), sig.is_html))
                .unwrap_or_default();
        }
        if !self.signature.trim().is_empty() {
            return (self.signature.clone(), false);
        }
        (String::new(), false)
    }

    /// 新邮件使用的签名正文，以及它是不是 HTML（对齐 Linux `AccountConfig::signature_set`）。
    /// `default_new_signature == None` 表示「不使用签名」；仅在尚未迁移命名签名列表时退回旧单签名 `signature`。
    pub fn primary_signature(&self) -> (String, bool) {
        self.resolve_signature(self.default_new_signature)
    }

    /// 回复/转发使用的签名正文，以及它是不是 HTML。
    /// `default_reply_signature == None` 表示「不使用签名」。
    pub fn reply_signature(&self) -> (String, bool) {
        self.resolve_signature(self.default_reply_signature)
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
            quota_used_bytes: None,
            quota_total_bytes: None,
            aliases: Vec::new(),
            signatures: Vec::new(),
            default_new_signature: None,
            default_reply_signature: None,
            special_folders: SpecialFoldersConfig::default(),
            sync_interval_minutes: None,
            download_window_days: 0,
            auto_empty_trash_days: 0,
            auto_empty_junk_days: 0,
            hidden_folders: Vec::new(),
            storage_dir: None,
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

/// 侧边栏收藏 / 隐藏文件夹的引用（账号 id + IMAP 路径；WP-07）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SidebarFolderRef {
    pub account_id: u32,
    pub path: String,
}

/// 用户为自定义文件夹指定的特殊角色（WP-07「Assign Special Role」；
/// `role` 取 `inbox` / `drafts` / `sent` / `archive` / `junk` / `trash`）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SidebarFolderRole {
    pub account_id: u32,
    pub path: String,
    pub role: String,
}

/// WP-06：OpenPGP 密钥元数据条目（Windows 端持久化密钥元信息与签名/加密策略）。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PgpKeyEntry {
    pub id: u32,
    pub name: String,
    pub email: String,
    pub fingerprint: String,
    pub has_secret: bool,
    pub trusted: bool,
    pub sign_by_default: bool,
    pub encrypt_by_default: bool,
    pub created_at: String,
}

/// WP-06：云存储账号配置（Nextcloud / WebDAV / S3 / Dropbox）。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CloudStorageAccount {
    pub id: u32,
    pub provider: String,
    pub name: String,
    pub server_url: String,
    pub username: String,
    pub bucket_or_path: String,
    pub enabled: bool,
}

/// WP-06：通讯录同步源配置（CardDAV / iCloud / Nextcloud / Fastmail）。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ContactSourceEntry {
    pub id: u32,
    pub provider: String,
    pub name: String,
    pub server_url: String,
    pub username: String,
    pub enabled: bool,
}

/// WP-06：邮件过滤器匹配字段。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterField {
    #[default]
    From,
    To,
    Subject,
    Body,
}

/// WP-06：邮件过滤器匹配方式。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterMatch {
    #[default]
    Contains,
    Equals,
    StartsWith,
    EndsWith,
}

/// WP-06：自动化过滤器规则。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FilterRule {
    pub id: u32,
    pub name: String,
    pub account_email: String,
    pub enabled: bool,
    pub match_all: bool,
    pub field: FilterField,
    pub matcher: FilterMatch,
    pub value: String,
    pub move_to_folder: String,
    pub archive: bool,
    pub star: bool,
    pub mark_read: bool,
    pub apply_tag: String,
    pub forward_to: String,
}

/// WP-06：智能文件夹配置。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SmartFolderConfig {
    pub id: u32,
    pub name: String,
    pub icon: String,
    pub query: String,
    pub account_email: String,
}

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
    /// Check GitHub releases for a newer Windows build shortly after launch.
    #[serde(default = "default_true")]
    pub check_updates: bool,
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
    pub unified_show_unread: bool,
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
    /// Whether new messages open in the main window (true) or their own window (false).
    #[serde(default = "default_true")]
    pub compose_inline: bool,
    /// Whether reply / reply-all / forward open in the main window (true) or their own window (false).
    #[serde(default = "default_true")]
    pub reply_inline: bool,
    /// Place signature above quoted text when replying or forwarding.
    #[serde(default = "default_true")]
    pub signature_above_quote: bool,
    /// One-time migration marker bringing pre-WP-04 installs (`signature_above_quote = false`)
    /// up to the default (`true`) without overriding subsequent user choices.
    #[serde(default)]
    pub signature_above_quote_default_applied: bool,
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
    // ---- WP-05：专注模式（对齐 Linux `src/config/focus.rs` FocusMode，
    //      兼容映射保留既有 5 个 `focus_*` 旧字段，不迁移不丢数据） ----
    /// 专注模式下折叠列表头部（搜索 / 筛选 / 计数 / 排序收进 ⋯ 菜单）。
    #[serde(default = "default_true")]
    pub focus_list_header: bool,
    /// 专注模式下隐藏侧边栏账号分区。
    #[serde(default = "default_true")]
    pub focus_hide_accounts: bool,
    /// 专注模式下折叠统一视图行。
    #[serde(default = "default_true")]
    pub focus_fold_unified: bool,
    /// 专注模式下隐藏发件人头像。
    #[serde(default = "default_true")]
    pub focus_hide_avatars: bool,
    /// 专注模式下隐藏主题（标签胶囊随之隐藏）。
    #[serde(default)]
    pub focus_hide_subject: bool,
    /// 专注模式单行布局显示列标题。
    #[serde(default)]
    pub focus_column_headings: bool,
    /// 专注模式下隐藏星标。
    #[serde(default = "default_true")]
    pub focus_hide_stars: bool,
    /// 专注模式下隐藏附件图标。
    #[serde(default = "default_true")]
    pub focus_hide_attachment_icons: bool,
    /// 专注模式下侧边栏折叠为图标栏。
    #[serde(default = "default_true")]
    pub focus_rail_sidebar: bool,
    /// 专注模式图标栏以未读圆点代替计数。
    #[serde(default = "default_true")]
    pub focus_rail_dots: bool,
    /// 专注模式下悬停图标栏滑出完整侧边栏。
    #[serde(default)]
    pub focus_hover_expand: bool,
    /// 专注模式下折叠账号为头像。
    #[serde(default = "default_true")]
    pub focus_fold_accounts: bool,
    /// 专注模式下单击账号即折叠。
    #[serde(default)]
    pub focus_click_folds: bool,
    /// 专注模式统一视图行显示未读计数。
    #[serde(default = "default_true")]
    pub focus_unread_counts: bool,
    /// 专注模式保留「收藏」分区。
    #[serde(default = "default_true")]
    pub focus_show_favorites: bool,
    /// 专注模式保留「筛选文件夹」分区。
    #[serde(default = "default_true")]
    pub focus_show_filters: bool,
    /// 专注模式保留「标签」分区。
    #[serde(default = "default_true")]
    pub focus_show_tags: bool,
    /// 专注模式保留「联系人」分区。
    #[serde(default = "default_true")]
    pub focus_show_contacts: bool,
    /// 专注模式保留「附件」分区。
    #[serde(default = "default_true")]
    pub focus_show_attachments: bool,
    /// 专注模式保留「统一收件箱」行。
    #[serde(default = "default_true")]
    pub focus_show_inboxes: bool,
    /// 专注模式下标签胶囊置于文本下方。
    #[serde(default)]
    pub focus_tags_below: bool,
    /// 专注模式统一视图各行的显隐（星标 / 已发送 / 草稿 / 归档 / 垃圾邮件 / 废纸篓 / 稍后处理）。
    #[serde(default = "default_true")]
    pub focus_unified_starred: bool,
    #[serde(default = "default_true")]
    pub focus_unified_sent: bool,
    #[serde(default = "default_true")]
    pub focus_unified_drafts: bool,
    #[serde(default = "default_true")]
    pub focus_unified_archive: bool,
    #[serde(default = "default_true")]
    pub focus_unified_junk: bool,
    #[serde(default = "default_true")]
    pub focus_unified_trash: bool,
    #[serde(default = "default_true")]
    pub focus_unified_snoozed: bool,
    /// 专注模式消息列表摘要行数（0..=3，对齐 Linux `preview_line_count`）。
    #[serde(default = "default_focus_preview_lines")]
    pub focus_preview_lines: u32,
    /// 专注模式消息列表布局：`"cards"` / `"single_line"` / `"automatic"`（对齐 Linux `ListLayout`）。
    #[serde(default = "default_focus_layout")]
    pub focus_layout: String,
    // ---- WP-05：侧边栏 ----
    /// 统一视图导航项：显示「垃圾邮件」。
    #[serde(default = "default_true")]
    pub unified_show_junk: bool,
    /// 统一视图导航项：显示「废纸篓」。
    #[serde(default = "default_true")]
    pub unified_show_trash: bool,
    /// 统一视图导航项：显示「稍后处理」。
    #[serde(default = "default_true")]
    pub unified_show_snoozed: bool,
    /// 侧边栏显示「筛选文件夹」分区。
    #[serde(default)]
    pub sidebar_show_filtered: bool,
    /// 侧边栏显示「智能文件夹」分区。
    #[serde(default = "default_true")]
    pub sidebar_show_smart: bool,
    /// 侧边栏显示「标签」分区。
    #[serde(default = "default_true")]
    pub sidebar_show_tags: bool,
    // ---- WP-05：消息列表 ----
    /// 列表布局：`"cards"` 经典双行 / `"wide"` 宽表列视图（渲染接线随 WP-08 宽表落地）。
    #[serde(default = "default_list_layout_key")]
    pub list_layout: String,
    /// 列表摘要行数（1 / 2 / 3 行；1 行为现状，多行渲染随 WP-08 接线）。
    #[serde(default = "default_list_preview_lines")]
    pub list_preview_lines: u32,
    /// 单行宽表显示的列（`ListColumn` key 顺序表；主题列恒在）。
    #[serde(default = "default_list_columns")]
    pub list_columns: Vec<String>,
    /// 宽表模式显示列标题栏。
    #[serde(default = "default_true")]
    pub list_show_headings: bool,
    // ---- WP-05：外观 ----
    /// 阅读器工具栏左组按钮（`ToolbarItem` key 顺序表，最多 7 个）。
    #[serde(default = "default_toolbar_left")]
    pub reader_toolbar_left: Vec<String>,
    /// 阅读器工具栏右组按钮（窄窗折叠进 ⋯ 菜单）。
    #[serde(default = "default_toolbar_right")]
    pub reader_toolbar_right: Vec<String>,
    /// 应用图标变体 id（当前资源库仅内置默认图标，其余选择回退默认）。
    #[serde(default = "default_app_icon")]
    pub app_icon: String,
    // ---- WP-05：常规 ----
    /// 按流量计费的网络连接下省流：仅收取新邮件，不预取其余内容。
    #[serde(default = "default_true")]
    pub metered_save_data: bool,
    /// 新邮件提示音开关（音频后端落地前仅存配置）。
    #[serde(default)]
    pub notification_sound_enabled: bool,
    /// 新邮件提示音（`BUILTIN_SOUNDS` 之一）。
    #[serde(default = "default_notification_sound")]
    pub notification_sound: String,
    /// 发送邮件提示音开关。
    #[serde(default)]
    pub send_sound_enabled: bool,
    /// 通知操作按钮（`NotificationButton` key，最多 3 项）。
    #[serde(default = "default_notification_buttons")]
    pub notification_buttons: Vec<String>,
    // ---- WP-05：发件人 ----
    /// 屏蔽发件人黑名单（完整邮箱或域名如 `example.com`，命中邮件待过滤点接线）。
    #[serde(default)]
    pub blocked_senders: Vec<String>,
    // ---- WP-05：隐私 ----
    /// 发件人域名与地址显示不一致时警告（仿冒横幅）。
    #[serde(default = "default_true")]
    pub show_spoof_banner: bool,
    /// 联系人无头像时向 Gravatar 查询（发送地址哈希到第三方）。
    #[serde(default)]
    pub use_gravatar: bool,
    /// 允许抓取发件人站点图标填充头像圈。
    #[serde(default)]
    pub sender_logos: bool,
    /// 已读回执策略：`"ask"` / `"always"` / `"never"`。
    #[serde(default = "default_read_receipt_policy")]
    pub read_receipt_policy: String,
    /// 本地数据库加密（SQLCipher 后端暂缺，仅存配置，不伪造加密行为）。
    #[serde(default)]
    pub encrypt_stored_mail: bool,
    // ---- WP-05：系统 ----
    /// 已下载邮件保留天数（`0` = 永久；对齐 Linux `[0, 365, 90, 30]`）。
    #[serde(default)]
    pub keep_downloads_days: u32,
    /// 登录时自启动（Windows 注册表 `HKCU\...\Run`，见 `autostart` 模块）。
    #[serde(default)]
    pub start_at_login: bool,
    /// 邮件内链接打开方式：`"system"` / `"edge"` / `"chrome"` / `"firefox"`。
    #[serde(default = "default_link_open_mode")]
    pub link_open_mode: String,
    /// WP-07：收藏文件夹（Favorites 分区；按收藏顺序排列）。
    #[serde(default)]
    pub sidebar_favorite_folders: Vec<SidebarFolderRef>,
    /// WP-07：隐藏文件夹（不渲染在侧边栏账号树中）。
    #[serde(default)]
    pub sidebar_hidden_folders: Vec<SidebarFolderRef>,
    /// WP-07：自定义文件夹的特殊角色指定（覆盖按路径推断的角色分类）。
    #[serde(default)]
    pub sidebar_special_roles: Vec<SidebarFolderRole>,
    // ---- WP-06：8 个新增设置一级分类持久化字段 ----
    /// WP-06 OpenPGP：是否通过 WKD 查询收件人公钥。
    #[serde(default)]
    pub openpgp_wkd_lookup: bool,
    /// WP-06 OpenPGP：是否查询 keys.openpgp.org。
    #[serde(default)]
    pub openpgp_keyserver_lookup: bool,
    /// WP-06 OpenPGP：本地登记的 PGP 密钥列表与邮箱绑定策略。
    #[serde(default)]
    pub pgp_keys: Vec<PgpKeyEntry>,
    /// WP-06 云存储：超大附件自动转存云盘阈值（MB，默认 25）。
    #[serde(default = "default_cloud_threshold_mb")]
    pub cloud_auto_upload_threshold_mb: u32,
    /// WP-06 云存储：云盘账号列表。
    #[serde(default)]
    pub cloud_accounts: Vec<CloudStorageAccount>,
    /// WP-06 通讯录源：自动收集已发送/回复邮件联系人。
    #[serde(default = "default_true")]
    pub contacts_auto_collect_senders: bool,
    /// WP-06 通讯录源：CardDAV 远程通讯录同步源列表。
    #[serde(default)]
    pub contact_sources: Vec<ContactSourceEntry>,
    /// WP-06 过滤器：自动化邮件过滤规则列表。
    #[serde(default)]
    pub filter_rules: Vec<FilterRule>,
    /// WP-06 智能文件夹：基于搜索表达式的动态虚拟文件夹列表。
    #[serde(default)]
    pub smart_folders: Vec<SmartFolderConfig>,
    /// WP-06 翻译：翻译引擎（`"off"` / `"libretranslate"` / `"deepl"` / `"google"` / `"local_llm"`）。
    #[serde(default = "default_translation_engine")]
    pub translation_engine: String,
    /// WP-06 翻译：服务端 URL（LibreTranslate / 本地 LLM）。
    #[serde(default)]
    pub translation_api_url: String,
    /// WP-06 翻译：目标语言代码（`"system"` / `"zh-CN"` / `"en"` / `"ja"` / `"ko"` / `"fr"` / `"de"` / `"es"`）。
    #[serde(default = "default_system_key")]
    pub translation_target_language: String,
    /// WP-06 翻译：阅读器内提供一键翻译按钮。
    #[serde(default = "default_true")]
    pub translation_offer_reader: bool,
    /// WP-06 翻译：写信器内提供一键翻译按钮。
    #[serde(default)]
    pub translation_offer_composer: bool,
    /// WP-06 MCP 服务：是否启用本地 MCP Server。
    #[serde(default)]
    pub mcp_enabled: bool,
    /// WP-06 MCP 服务：Windows 命名管道 / Socket 路径。
    #[serde(default = "default_mcp_socket_path")]
    pub mcp_socket_path: String,
    /// WP-06 MCP 服务：允许 AI 助手读取邮件内容。
    #[serde(default = "default_true")]
    pub mcp_allow_read: bool,
    /// WP-06 MCP 服务：允许 AI 助手搜索邮件。
    #[serde(default = "default_true")]
    pub mcp_allow_search: bool,
    /// WP-06 MCP 服务：允许 AI 助手创建草稿。
    #[serde(default)]
    pub mcp_allow_draft: bool,
    /// WP-06 MCP 服务：允许 AI 助手标记已读/星标/标签。
    #[serde(default)]
    pub mcp_allow_modify_flags: bool,
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

// ---- WP-05 serde 默认值 ----

fn default_focus_preview_lines() -> u32 {
    0
}

fn default_focus_layout() -> String {
    "automatic".to_string()
}

fn default_list_layout_key() -> String {
    "cards".to_string()
}

fn default_list_preview_lines() -> u32 {
    1
}

fn default_list_columns() -> Vec<String> {
    ListColumn::to_keys(&ListColumn::DEFAULT)
}

fn default_toolbar_left() -> Vec<String> {
    ReaderToolbarLayout::default()
        .left
        .iter()
        .map(|i| i.key().to_string())
        .collect()
}

fn default_toolbar_right() -> Vec<String> {
    ReaderToolbarLayout::default()
        .right
        .iter()
        .map(|i| i.key().to_string())
        .collect()
}

fn default_app_icon() -> String {
    "default".to_string()
}

fn default_notification_sound() -> String {
    BUILTIN_SOUNDS[0].to_string()
}

fn default_notification_buttons() -> Vec<String> {
    NotificationButtons::default().to_list()
}

fn default_read_receipt_policy() -> String {
    "ask".to_string()
}

fn default_link_open_mode() -> String {
    "system".to_string()
}

fn default_cloud_threshold_mb() -> u32 {
    25
}

fn default_translation_engine() -> String {
    "off".to_string()
}

fn default_mcp_socket_path() -> String {
    r"\\.\pipe\kitemail-mcp".to_string()
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

    /// WP-05：解析阅读器工具栏布局（左右两组 + 未显示池）。
    pub fn reader_toolbar(&self) -> ReaderToolbarLayout {
        ReaderToolbarLayout::from_keys(&self.reader_toolbar_left, &self.reader_toolbar_right)
    }

    /// WP-05：解析单行宽表显示的列（去重、跳过未知名、主题列恒在）。
    pub fn list_columns_shown(&self) -> Vec<ListColumn> {
        ListColumn::from_keys(&self.list_columns)
    }

    /// WP-05：`sender` 是否被黑名单屏蔽。完整邮箱精确匹配；裸域名（如
    /// `example.com`）匹配该域及其子域（对齐 Linux `app/filters.rs::is_blacklisted`）。
    pub fn is_sender_blocked(&self, sender: &str) -> bool {
        let addr = normalize_sender_email(sender);
        if addr.is_empty() {
            return false;
        }
        let domain = addr.rsplit('@').next().unwrap_or("");
        self.blocked_senders.iter().any(|entry| {
            let entry = entry.trim().to_ascii_lowercase();
            if entry.is_empty() {
                return false;
            }
            if entry.contains('@') {
                addr == entry
            } else {
                domain == entry || domain.ends_with(&format!(".{entry}"))
            }
        })
    }

    /// WP-05：加入黑名单（接受完整邮箱、`Name <addr>` 形式或裸域名；大小写
    /// 归一并去重）。返回归一化后的条目；空串或非法邮箱返回 `None`。
    pub fn block_sender(&mut self, entry: &str) -> Option<String> {
        let trimmed = entry.trim().to_ascii_lowercase();
        if trimmed.is_empty() {
            return None;
        }
        // `Name <addr>` 形式：取尖括号内的地址。
        let normalized = if let (Some(open), Some(close)) = (trimmed.rfind('<'), trimmed.rfind('>'))
        {
            if open < close {
                trimmed[open + 1..close].trim().to_string()
            } else {
                trimmed
            }
        } else {
            trimmed
        };
        if normalized.is_empty() {
            return None;
        }
        // 完整邮箱需含 @ 且 @ 后非空；裸域名则不允许含 @。
        if normalized.contains('@') {
            let (_, domain) = normalized.rsplit_once('@')?;
            if domain.is_empty() {
                return None;
            }
        } else if normalized.contains('/') || normalized.contains('\\') {
            return None;
        }
        if !self
            .blocked_senders
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(&normalized))
        {
            self.blocked_senders.push(normalized.clone());
        }
        Some(normalized)
    }

    /// WP-05：从黑名单移除（大小写不敏感）。返回是否确有移除。
    pub fn unblock_sender(&mut self, entry: &str) -> bool {
        let normalized = entry.trim().to_ascii_lowercase();
        let before = self.blocked_senders.len();
        self.blocked_senders
            .retain(|existing| !existing.eq_ignore_ascii_case(&normalized));
        self.blocked_senders.len() != before
    }
}

impl Default for AppPreferences {
    fn default() -> Self {
        Self {
            theme_id: "system".to_string(),
            dark_mode: None,
            compact_list: false,
            page_size: 100,
            auto_sync_minutes: 5,
            push_enabled: true,
            block_remote_images: true,
            close_to_minimize: true,
            minimize_to_tray: true,
            desktop_notifications: true,
            check_updates: true,
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
            unified_show_unread: true,
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
            compose_inline: true,
            reply_inline: true,
            signature_above_quote: true,
            signature_above_quote_default_applied: true,
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
            focus_list_header: true,
            focus_hide_accounts: true,
            focus_fold_unified: true,
            focus_hide_avatars: true,
            focus_hide_subject: false,
            focus_column_headings: false,
            focus_hide_stars: true,
            focus_hide_attachment_icons: true,
            focus_rail_sidebar: true,
            focus_rail_dots: true,
            focus_hover_expand: false,
            focus_fold_accounts: true,
            focus_click_folds: false,
            focus_unread_counts: true,
            focus_show_favorites: true,
            focus_show_filters: true,
            focus_show_tags: true,
            focus_show_contacts: true,
            focus_show_attachments: true,
            focus_show_inboxes: true,
            focus_tags_below: false,
            focus_unified_starred: true,
            focus_unified_sent: true,
            focus_unified_drafts: true,
            focus_unified_archive: true,
            focus_unified_junk: true,
            focus_unified_trash: true,
            focus_unified_snoozed: true,
            focus_preview_lines: default_focus_preview_lines(),
            focus_layout: default_focus_layout(),
            unified_show_junk: true,
            unified_show_trash: true,
            unified_show_snoozed: true,
            sidebar_show_filtered: false,
            sidebar_show_smart: true,
            sidebar_show_tags: true,
            list_layout: default_list_layout_key(),
            list_preview_lines: default_list_preview_lines(),
            list_columns: default_list_columns(),
            list_show_headings: true,
            reader_toolbar_left: default_toolbar_left(),
            reader_toolbar_right: default_toolbar_right(),
            app_icon: default_app_icon(),
            metered_save_data: true,
            notification_sound_enabled: false,
            notification_sound: default_notification_sound(),
            send_sound_enabled: false,
            notification_buttons: default_notification_buttons(),
            blocked_senders: Vec::new(),
            show_spoof_banner: true,
            use_gravatar: false,
            sender_logos: false,
            read_receipt_policy: default_read_receipt_policy(),
            encrypt_stored_mail: false,
            keep_downloads_days: 0,
            start_at_login: false,
            link_open_mode: default_link_open_mode(),
            sidebar_favorite_folders: Vec::new(),
            sidebar_hidden_folders: Vec::new(),
            sidebar_special_roles: Vec::new(),
            openpgp_wkd_lookup: false,
            openpgp_keyserver_lookup: false,
            pgp_keys: Vec::new(),
            cloud_auto_upload_threshold_mb: default_cloud_threshold_mb(),
            cloud_accounts: Vec::new(),
            contacts_auto_collect_senders: true,
            contact_sources: Vec::new(),
            filter_rules: Vec::new(),
            smart_folders: Vec::new(),
            translation_engine: default_translation_engine(),
            translation_api_url: String::new(),
            translation_target_language: default_system_key(),
            translation_offer_reader: true,
            translation_offer_composer: false,
            mcp_enabled: false,
            mcp_socket_path: default_mcp_socket_path(),
            mcp_allow_read: true,
            mcp_allow_search: true,
            mcp_allow_draft: false,
            mcp_allow_modify_flags: false,
        }
    }
}

// ---------------------------------------------------------------------------
// WP-05：移植 Linux 的可枚举配置（工具栏按钮 / 列表列 / 通知按钮）与辅助统计。
// ---------------------------------------------------------------------------

/// 内置新邮件提示音名（对齐 Linux `BUILTIN_SOUNDS`；无音频后端时仅存配置）。
pub const BUILTIN_SOUNDS: [&str; 4] = ["click", "hum", "string", "swing"];

/// 阅读器工具栏上的一个按钮（对齐 Linux `src/config/toolbar.rs::ToolbarItem`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ToolbarItem {
    Reply,
    ReplyAll,
    Forward,
    Star,
    Archive,
    Snooze,
    Delete,
    Spam,
    ReadUnread,
    Tags,
    MoveTo,
    Find,
    Print,
}

impl ToolbarItem {
    pub const ALL: [ToolbarItem; 13] = [
        ToolbarItem::Reply,
        ToolbarItem::ReplyAll,
        ToolbarItem::Forward,
        ToolbarItem::Star,
        ToolbarItem::Archive,
        ToolbarItem::Snooze,
        ToolbarItem::Delete,
        ToolbarItem::Spam,
        ToolbarItem::ReadUnread,
        ToolbarItem::Tags,
        ToolbarItem::MoveTo,
        ToolbarItem::Find,
        ToolbarItem::Print,
    ];

    /// 写入配置的稳定名。
    pub fn key(self) -> &'static str {
        match self {
            ToolbarItem::Reply => "reply",
            ToolbarItem::ReplyAll => "reply_all",
            ToolbarItem::Forward => "forward",
            ToolbarItem::Star => "star",
            ToolbarItem::Archive => "archive",
            ToolbarItem::Snooze => "snooze",
            ToolbarItem::Delete => "delete",
            ToolbarItem::Spam => "spam",
            ToolbarItem::ReadUnread => "read_unread",
            ToolbarItem::Tags => "tags",
            ToolbarItem::MoveTo => "move_to",
            ToolbarItem::Find => "find",
            ToolbarItem::Print => "print",
        }
    }

    pub fn from_key(key: &str) -> Option<ToolbarItem> {
        ToolbarItem::ALL.iter().copied().find(|i| i.key() == key)
    }

    /// 设置页芯片的中文标签。
    pub fn label(self) -> &'static str {
        match self {
            ToolbarItem::Reply => "回复",
            ToolbarItem::ReplyAll => "全部回复",
            ToolbarItem::Forward => "转发",
            ToolbarItem::Star => "星标",
            ToolbarItem::Archive => "归档",
            ToolbarItem::Snooze => "稍后处理",
            ToolbarItem::Delete => "删除",
            ToolbarItem::Spam => "垃圾邮件",
            ToolbarItem::ReadUnread => "已读 / 未读",
            ToolbarItem::Tags => "标签",
            ToolbarItem::MoveTo => "移动到",
            ToolbarItem::Find => "查找",
            ToolbarItem::Print => "打印",
        }
    }
}

/// 阅读器工具栏布局：左组（始终显示，最多 [`ReaderToolbarLayout::LEFT_MAX`] 个）、
/// 右组（窄窗折叠进 ⋯）与未显示池（两个列表都不含即隐藏）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReaderToolbarLayout {
    pub left: Vec<ToolbarItem>,
    pub right: Vec<ToolbarItem>,
}

impl Default for ReaderToolbarLayout {
    fn default() -> Self {
        ReaderToolbarLayout {
            left: vec![
                ToolbarItem::Reply,
                ToolbarItem::ReplyAll,
                ToolbarItem::Forward,
                ToolbarItem::Star,
                ToolbarItem::Archive,
                ToolbarItem::Spam,
                ToolbarItem::Delete,
            ],
            right: vec![
                ToolbarItem::Snooze,
                ToolbarItem::Tags,
                ToolbarItem::ReadUnread,
                ToolbarItem::MoveTo,
                ToolbarItem::Find,
                ToolbarItem::Print,
            ],
        }
    }
}

impl ReaderToolbarLayout {
    /// 左组最多容纳的按钮数（对齐 Linux `TOOLBAR_LEFT_MAX`）。
    pub const LEFT_MAX: usize = 7;

    /// 从配置 key 列表重建（去重、跳过未知名、左组截断到上限）。
    pub fn from_keys(left: &[String], right: &[String]) -> Self {
        let parse = |keys: &[String]| -> Vec<ToolbarItem> {
            keys.iter()
                .filter_map(|k| ToolbarItem::from_key(k))
                .collect()
        };
        let mut layout = ReaderToolbarLayout {
            left: parse(left),
            right: parse(right),
        };
        layout.sanitize();
        layout
    }

    /// 按钮在哪一组（不在两组即隐藏）。
    pub fn side(&self, item: ToolbarItem) -> Option<ToolbarSide> {
        if self.left.contains(&item) {
            Some(ToolbarSide::Left)
        } else if self.right.contains(&item) {
            Some(ToolbarSide::Right)
        } else {
            None
        }
    }

    /// 未显示的按钮（规范顺序）。
    pub fn hidden(&self) -> Vec<ToolbarItem> {
        ToolbarItem::ALL
            .iter()
            .copied()
            .filter(|i| self.side(*i).is_none())
            .collect()
    }

    /// 把 `item` 放到 `side` 的 `index` 处（钳制）；`None` 隐藏。左组满员时不动。
    pub fn place(&mut self, item: ToolbarItem, side: Option<ToolbarSide>, index: usize) {
        if side == Some(ToolbarSide::Left)
            && !self.left.contains(&item)
            && self.left.len() >= Self::LEFT_MAX
        {
            return;
        }
        self.left.retain(|i| *i != item);
        self.right.retain(|i| *i != item);
        let list = match side {
            Some(ToolbarSide::Left) => &mut self.left,
            Some(ToolbarSide::Right) => &mut self.right,
            None => return,
        };
        let index = index.min(list.len());
        list.insert(index, item);
    }

    /// 在所属组内把 `item` 上移（`-1`）或下移（`1`）；越界即不动。
    pub fn move_within(&mut self, item: ToolbarItem, delta: isize) {
        let list = match self.side(item) {
            Some(ToolbarSide::Left) => &mut self.left,
            Some(ToolbarSide::Right) => &mut self.right,
            None => return,
        };
        if let Some(pos) = list.iter().position(|i| *i == item) {
            let next = pos as isize + delta;
            if next >= 0 && (next as usize) < list.len() {
                list.remove(pos);
                list.insert(next as usize, item);
            }
        }
    }

    /// 去重并把左组截断到上限。
    fn sanitize(&mut self) {
        let mut seen = std::collections::HashSet::new();
        self.left.retain(|i| seen.insert(*i));
        self.right.retain(|i| seen.insert(*i));
        self.left.truncate(Self::LEFT_MAX);
    }
}

/// 工具栏按钮所在侧。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolbarSide {
    Left,
    Right,
}

/// 单行宽表消息列表的一列（对齐 Linux `src/config/look.rs::ListColumn` 12 列）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ListColumn {
    Star,
    Replied,
    Sender,
    Recipients,
    Correspondents,
    Subject,
    Tags,
    Attachment,
    Importance,
    Account,
    Due,
    Date,
}

impl ListColumn {
    pub const ALL: [ListColumn; 12] = [
        ListColumn::Star,
        ListColumn::Replied,
        ListColumn::Sender,
        ListColumn::Recipients,
        ListColumn::Correspondents,
        ListColumn::Subject,
        ListColumn::Tags,
        ListColumn::Attachment,
        ListColumn::Importance,
        ListColumn::Account,
        ListColumn::Due,
        ListColumn::Date,
    ];

    /// 单行列表默认显示的列（对齐 Linux `ListColumn::DEFAULT`）。
    pub const DEFAULT: [ListColumn; 7] = [
        ListColumn::Star,
        ListColumn::Replied,
        ListColumn::Sender,
        ListColumn::Subject,
        ListColumn::Tags,
        ListColumn::Attachment,
        ListColumn::Date,
    ];

    pub fn key(self) -> &'static str {
        match self {
            ListColumn::Star => "star",
            ListColumn::Replied => "replied",
            ListColumn::Sender => "sender",
            ListColumn::Recipients => "recipients",
            ListColumn::Correspondents => "correspondents",
            ListColumn::Subject => "subject",
            ListColumn::Tags => "tags",
            ListColumn::Attachment => "attachment",
            ListColumn::Importance => "importance",
            ListColumn::Account => "account",
            ListColumn::Due => "due",
            ListColumn::Date => "date",
        }
    }

    pub fn from_key(key: &str) -> Option<ListColumn> {
        ListColumn::ALL.iter().copied().find(|c| c.key() == key)
    }

    /// 设置页芯片的中文标签。
    pub fn label(self) -> &'static str {
        match self {
            ListColumn::Star => "星标",
            ListColumn::Replied => "已回复",
            ListColumn::Sender => "发件人",
            ListColumn::Recipients => "收件人",
            ListColumn::Correspondents => "往来联系人",
            ListColumn::Subject => "主题",
            ListColumn::Tags => "标签",
            ListColumn::Attachment => "附件",
            ListColumn::Importance => "重要性",
            ListColumn::Account => "账号",
            ListColumn::Due => "截止日期",
            ListColumn::Date => "日期",
        }
    }

    /// 从保存的 key 列表还原显示列：跳过未知名并去重；主题列是占满剩余
    /// 宽度的弹性列，因此始终保留（缺省插到发件人之后）。
    pub fn from_keys(keys: &[String]) -> Vec<ListColumn> {
        let mut out: Vec<ListColumn> = Vec::new();
        for c in keys.iter().filter_map(|k| ListColumn::from_key(k)) {
            if !out.contains(&c) {
                out.push(c);
            }
        }
        if !out.contains(&ListColumn::Subject) {
            let at = out
                .iter()
                .position(|c| *c == ListColumn::Sender)
                .map(|i| i + 1)
                .unwrap_or(0);
            out.insert(at, ListColumn::Subject);
        }
        out
    }

    pub fn to_keys(columns: &[ListColumn]) -> Vec<String> {
        columns.iter().map(|c| c.key().to_string()).collect()
    }
}

/// 通知可携带的一个操作按钮（对齐 Linux `NotificationButton`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationButton {
    MarkRead,
    Archive,
    Delete,
    Reply,
    Forward,
    Spam,
}

impl NotificationButton {
    pub const ALL: [NotificationButton; 6] = [
        NotificationButton::MarkRead,
        NotificationButton::Archive,
        NotificationButton::Delete,
        NotificationButton::Reply,
        NotificationButton::Forward,
        NotificationButton::Spam,
    ];

    pub fn key(self) -> &'static str {
        match self {
            NotificationButton::MarkRead => "mark_read",
            NotificationButton::Archive => "archive",
            NotificationButton::Delete => "delete",
            NotificationButton::Reply => "reply",
            NotificationButton::Forward => "forward",
            NotificationButton::Spam => "spam",
        }
    }

    pub fn from_key(key: &str) -> Option<NotificationButton> {
        NotificationButton::ALL
            .iter()
            .copied()
            .find(|b| b.key() == key)
    }

    /// 设置页行的中文标签。
    pub fn label(self) -> &'static str {
        match self {
            NotificationButton::MarkRead => "标记为已读",
            NotificationButton::Archive => "归档",
            NotificationButton::Delete => "删除",
            NotificationButton::Reply => "回复",
            NotificationButton::Forward => "转发",
            NotificationButton::Spam => "标记为垃圾邮件",
        }
    }
}

/// 新邮件通知携带的操作按钮集合：六个中最多三个（对齐 Linux 上限，
/// Windows 通知 portal 同样只显示三条）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotificationButtons {
    pub mark_read: bool,
    pub archive: bool,
    pub delete: bool,
    pub reply: bool,
    pub forward: bool,
    pub spam: bool,
}

impl Default for NotificationButtons {
    fn default() -> Self {
        // 对齐 Linux：默认携带 标记已读 / 归档 / 删除。
        NotificationButtons {
            mark_read: true,
            archive: true,
            delete: true,
            reply: false,
            forward: false,
            spam: false,
        }
    }
}

impl NotificationButtons {
    pub const MAX: usize = 3;

    /// 全关集合（[`Self::from_list`] 的解析起点）。
    pub const NONE: Self = Self {
        mark_read: false,
        archive: false,
        delete: false,
        reply: false,
        forward: false,
        spam: false,
    };

    pub fn get(self, button: NotificationButton) -> bool {
        match button {
            NotificationButton::MarkRead => self.mark_read,
            NotificationButton::Archive => self.archive,
            NotificationButton::Delete => self.delete,
            NotificationButton::Reply => self.reply,
            NotificationButton::Forward => self.forward,
            NotificationButton::Spam => self.spam,
        }
    }

    pub fn set(&mut self, button: NotificationButton, on: bool) {
        match button {
            NotificationButton::MarkRead => self.mark_read = on,
            NotificationButton::Archive => self.archive = on,
            NotificationButton::Delete => self.delete = on,
            NotificationButton::Reply => self.reply = on,
            NotificationButton::Forward => self.forward = on,
            NotificationButton::Spam => self.spam = on,
        }
    }

    pub fn count(self) -> usize {
        NotificationButton::ALL
            .into_iter()
            .filter(|b| self.get(*b))
            .count()
    }

    /// 集已满，不能再开启下一个按钮。
    pub fn full(self) -> bool {
        self.count() >= Self::MAX
    }

    /// 某按钮的开关行当前是否可操作：已开启的随时可关；未开启的需未满员。
    pub fn row_sensitive(self, button: NotificationButton) -> bool {
        self.get(button) || !self.full()
    }

    /// 从保存的 key 列表还原（未知名跳过，超过 3 个按 ALL 顺序截断）。
    pub fn from_list(names: &[String]) -> Self {
        let mut b = Self::NONE;
        for button in NotificationButton::ALL {
            if !b.full() && names.iter().any(|n| n == button.key()) {
                b.set(button, true);
            }
        }
        b
    }

    pub fn to_list(self) -> Vec<String> {
        NotificationButton::ALL
            .into_iter()
            .filter(|b| self.get(*b))
            .map(|b| b.key().to_string())
            .collect()
    }
}

/// 本地数据目录占用统计：总字节数与文件数（纯 Rust 目录遍历，实时计算）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StorageStats {
    pub total_bytes: u64,
    pub file_count: u64,
}

/// 单个账号在邮件存储目录中的空间占用统计。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountStorageStat {
    pub account_id: u32,
    pub email: String,
    pub dir_name: String,
    pub stats: StorageStats,
}

/// 程序配置目录与独立邮件存储目录的组合空间统计摘要。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppStorageSummary {
    pub app_data_dir: PathBuf,
    pub mail_data_dir: PathBuf,
    pub is_portable: bool,
    pub is_custom_mail_dir: bool,
    pub config_stats: StorageStats,
    pub mail_stats: StorageStats,
    pub per_account: Vec<AccountStorageStat>,
}

/// 独立邮件存储配置（存于 `<app_data_dir>\storage.toml`）。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageConfig {
    /// 自定义邮件数据根目录；`None` 或空串表示使用默认的 `<app_data_dir>\mail`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mail_data_dir: Option<String>,
}

/// 统计 `dir` 及其子目录的磁盘占用（跳过无法访问的条目）。`dir` 不存在时为零值。
pub fn storage_stats_in(dir: &Path) -> StorageStats {
    storage_stats_excluding_in(dir, None)
}

fn storage_stats_excluding_in(dir: &Path, skip_dir: Option<&Path>) -> StorageStats {
    let mut stats = StorageStats {
        total_bytes: 0,
        file_count: 0,
    };
    let canonical_skip =
        skip_dir.and_then(|p| fs::canonicalize(p).ok().or_else(|| Some(p.to_path_buf())));
    let Ok(entries) = fs::read_dir(dir) else {
        return stats;
    };
    let mut stack = vec![entries];
    while let Some(entries) = stack.pop() {
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.file_type() {
                Ok(ft) if ft.is_dir() => {
                    if let Some(ref skip) = canonical_skip {
                        let cmp_path = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
                        if &cmp_path == skip {
                            continue;
                        }
                    }
                    if let Ok(sub) = fs::read_dir(&path) {
                        stack.push(sub);
                    }
                }
                Ok(ft) if ft.is_file() => {
                    if let Ok(meta) = entry.metadata() {
                        stats.total_bytes += meta.len();
                        stats.file_count += 1;
                    }
                }
                _ => {}
            }
        }
    }
    stats
}

/// 把字节数格式化为人类可读的容量（`B` / `KB` / `MB` / `GB`，二进制换算）。
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
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

/// 解析程序配置与全局数据根目录：
/// `KITEMAIL_DATA_DIR` > 便携版（`<exe_dir>\data`）> 安装版（`%LOCALAPPDATA%\KiteMail`）。
pub fn resolve_data_dir(
    env_override: Option<&str>,
    is_portable: bool,
    exe_dir: Option<&Path>,
    local_app_data: Option<&Path>,
) -> PathBuf {
    if let Some(raw) = env_override.map(str::trim).filter(|s| !s.is_empty()) {
        return PathBuf::from(raw);
    }
    if is_portable {
        if let Some(dir) = exe_dir {
            return dir.join("data");
        }
    }
    local_app_data
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("KiteMail")
}

/// 返回当前运行时生效的 KiteMail 本地配置与全局数据库根目录。
/// 安装版为 `%LOCALAPPDATA%\KiteMail`；便携版（同级存在 `portable.flag`）为 `<exe_dir>\data`。
pub fn data_dir() -> PathBuf {
    #[cfg(not(test))]
    {
        static CACHED_DATA_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
        return CACHED_DATA_DIR.get_or_init(compute_data_dir).clone();
    }
    #[cfg(test)]
    {
        compute_data_dir()
    }
}

fn compute_data_dir() -> PathBuf {
    let env_override = std::env::var("KITEMAIL_DATA_DIR").ok();
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    let local_app_data = dirs::data_local_dir();
    resolve_data_dir(
        env_override.as_deref(),
        crate::update::is_portable(),
        exe_dir.as_deref(),
        local_app_data.as_deref(),
    )
}

/// 返回 `dir/storage.toml` 的路径。
pub fn storage_config_file_in(dir: &Path) -> PathBuf {
    dir.join("storage.toml")
}

/// 从 `dir` 加载邮件存储路径配置；文件缺失时返回默认值。
pub fn load_storage_config_in(dir: &Path) -> Result<StorageConfig, String> {
    let path = storage_config_file_in(dir);
    if !path.exists() {
        return Ok(StorageConfig::default());
    }
    let text = fs::read_to_string(&path)
        .map_err(|e| format!("读取存储路径配置失败 ({}): {e}", path.display()))?;
    toml::from_str(&text).map_err(|e| format!("解析存储路径配置失败 ({}): {e}", path.display()))
}

/// 将独立邮件存储路径配置原子写入 `dir/storage.toml`。
pub fn save_storage_config_in(dir: &Path, cfg: &StorageConfig) -> Result<(), String> {
    let normalized = StorageConfig {
        mail_data_dir: cfg
            .mail_data_dir
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
    };
    let path = storage_config_file_in(dir);
    let text =
        toml::to_string_pretty(&normalized).map_err(|e| format!("序列化存储路径配置失败: {e}"))?;
    atomic_write_file(dir, &path, &text, "存储路径配置")
}

/// 返回配置目录对应的默认独立邮件存储路径（`<app_data_dir>\mail`）。
pub fn default_mail_data_dir_in(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("mail")
}

/// 解析当前生效的独立邮件存储目录；若 `storage.toml` 配置了相对路径则相对于 `app_data_dir` 解析。
pub fn resolve_mail_data_dir_in(app_data_dir: &Path) -> PathBuf {
    if let Ok(cfg) = load_storage_config_in(app_data_dir) {
        if let Some(raw) = cfg
            .mail_data_dir
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let path = PathBuf::from(raw);
            return if path.is_absolute() {
                path
            } else {
                app_data_dir.join(path)
            };
        }
    }
    default_mail_data_dir_in(app_data_dir)
}

/// 判断目录名是否为单一合法的普通子目录名（防路径穿越及 Windows 保留设备名）。
pub fn is_valid_single_dir_name(name: &str) -> bool {
    let trimmed = name.trim();
    if trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".."
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains("..")
        || trimmed.contains('\0')
        || trimmed.ends_with('.')
        || trimmed.ends_with(' ')
    {
        return false;
    }
    let mut comps = Path::new(trimmed).components();
    match (comps.next(), comps.next()) {
        (Some(std::path::Component::Normal(_)), None) => {}
        _ => return false,
    }
    !is_windows_reserved_name(trimmed)
}

fn is_windows_reserved_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).to_ascii_lowercase();
    matches!(
        stem.as_str(),
        "con"
            | "prn"
            | "aux"
            | "nul"
            | "com1"
            | "com2"
            | "com3"
            | "com4"
            | "com5"
            | "com6"
            | "com7"
            | "com8"
            | "com9"
            | "lpt1"
            | "lpt2"
            | "lpt3"
            | "lpt4"
            | "lpt5"
            | "lpt6"
            | "lpt7"
            | "lpt8"
            | "lpt9"
    )
}

/// 将邮箱地址归一化为安全可读的 Windows 账号目录名（如 `user@outlook.com`）。
pub fn normalize_email_storage_dir(email: &str) -> String {
    let cleaned: String = email
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() || c.is_whitespace() => '_',
            other => other,
        })
        .collect();
    let trimmed = cleaned.trim_matches(|c| c == '.' || c == ' ');
    let mut safe = if trimmed.is_empty() {
        "account".to_string()
    } else if trimmed.chars().count() > 120 {
        trimmed
            .chars()
            .take(120)
            .collect::<String>()
            .trim_end_matches(|c| c == '.' || c == ' ')
            .to_string()
    } else {
        trimmed.to_string()
    };
    if safe.is_empty() {
        safe = "account".to_string();
    }
    if is_windows_reserved_name(&safe) {
        safe = format!("_{safe}");
    }
    safe
}

/// 为指定账号确定不冲突的邮件存储子目录名：优先采用显式且合法的 `account.storage_dir`，
/// 否则按 `email` 归一化；若同一客户端配置了多个同邮箱账号则追加 `_2`、`_3`。
pub fn resolve_account_storage_dir(
    account: &AccountConfig,
    all_accounts: &[AccountConfig],
) -> String {
    if let Some(ref custom) = account.storage_dir {
        let trimmed = custom.trim();
        if is_valid_single_dir_name(trimmed) {
            return trimmed.to_string();
        }
    }
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    for acc in all_accounts {
        if let Some(ref custom) = acc.storage_dir {
            let trimmed = custom.trim();
            if is_valid_single_dir_name(trimmed) {
                used.insert(trimmed.to_ascii_lowercase());
            }
        }
    }
    for acc in all_accounts {
        let dir_name = if let Some(ref custom) = acc.storage_dir {
            let trimmed = custom.trim();
            if is_valid_single_dir_name(trimmed) {
                trimmed.to_string()
            } else {
                String::new()
            }
        } else {
            String::new()
        };
        let resolved = if !dir_name.is_empty() {
            dir_name
        } else {
            let base = normalize_email_storage_dir(&acc.email);
            let mut candidate = base.clone();
            let mut seq = 2u32;
            while used.contains(&candidate.to_ascii_lowercase()) {
                candidate = format!("{base}_{seq}");
                seq = seq.saturating_add(1);
            }
            used.insert(candidate.to_ascii_lowercase());
            candidate
        };
        if acc.id == account.id {
            return resolved;
        }
    }
    normalize_email_storage_dir(&account.email)
}

/// 拼接并校验账号独立邮件目录（`<mail_data_dir>\<storage_dir>`）。
pub fn account_mail_dir_in(mail_data_dir: &Path, storage_dir: &str) -> Result<PathBuf, String> {
    let trimmed = storage_dir.trim();
    if !is_valid_single_dir_name(trimmed) {
        return Err(format!("非法的账号存储目录名: {storage_dir}"));
    }
    Ok(mail_data_dir.join(trimmed))
}

/// 返回全局元数据 SQLite 库路径（`<app_data_dir>\global.db`）。
pub fn global_db_in(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("global.db")
}

/// 把路径规整为可用于等价/包含判断的规范键（解析已有祖先符号链接并去掉 Windows `\\?\` 前缀）。
pub(crate) fn normalized_path_key(path: &Path) -> String {
    let resolved = if path.exists() {
        fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    } else {
        // 目标末级目录尚未创建时，向上回溯到最近存在的祖先以消除大小写/盘符/符号链接差异。
        let mut cursor = Some(path);
        let mut suffix: Vec<std::ffi::OsString> = Vec::new();
        let mut base = None;
        while let Some(cur) = cursor {
            if cur.exists() {
                if let Ok(canon) = fs::canonicalize(cur) {
                    base = Some(canon);
                    break;
                }
            }
            if let Some(name) = cur.file_name() {
                suffix.push(name.to_os_string());
                cursor = cur.parent();
            } else {
                break;
            }
        }
        if let Some(mut rebuilt) = base {
            for comp in suffix.into_iter().rev() {
                rebuilt.push(comp);
            }
            rebuilt
        } else {
            path.to_path_buf()
        }
    };
    let s = resolved.to_string_lossy();
    let stripped = s.strip_prefix(r"\\?\").unwrap_or(&s);
    stripped
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

/// 计算程序配置目录与独立邮件存储目录的分项磁盘占用摘要。
pub fn app_storage_summary_in(app_data_dir: &Path) -> AppStorageSummary {
    let default_mail_dir = default_mail_data_dir_in(app_data_dir);
    let mail_data_dir = resolve_mail_data_dir_in(app_data_dir);
    let is_custom_mail_dir =
        normalized_path_key(&mail_data_dir) != normalized_path_key(&default_mail_dir);
    let config_stats = storage_stats_excluding_in(app_data_dir, Some(&mail_data_dir));
    let mail_stats = storage_stats_in(&mail_data_dir);
    let accounts = load_accounts_in(app_data_dir).unwrap_or_default();
    let per_account = accounts
        .iter()
        .map(|acc| {
            let dir_name = resolve_account_storage_dir(acc, &accounts);
            let stats = storage_stats_in(&mail_data_dir.join(&dir_name));
            AccountStorageStat {
                account_id: acc.id,
                email: acc.email.clone(),
                dir_name,
                stats,
            }
        })
        .collect();
    AppStorageSummary {
        app_data_dir: app_data_dir.to_path_buf(),
        mail_data_dir,
        is_portable: crate::update::is_portable(),
        is_custom_mail_dir,
        config_stats,
        mail_stats,
        per_account,
    }
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
    dir.join("kitemail-cache.db")
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
    let mut prefs: AppPreferences =
        toml::from_str(&text).map_err(|e| format!("解析偏好设置失败 ({}): {e}", path.display()))?;
    // WP-04 之前生成的配置文件会把旧默认值 signature_above_quote = false 落盘；
    // 仅迁移一次，之后用户显式改回「在引用内容下方」会保留。
    if !prefs.signature_above_quote_default_applied {
        prefs.signature_above_quote_default_applied = true;
        prefs.signature_above_quote = true;
        let _ = save_preferences_in(dir, &prefs);
    }
    Ok(prefs)
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

const KNOWN_INSTANCES_FILE: &str = "known_instances.toml";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct KnownInstancesFile {
    data_dirs: Vec<String>,
}

/// 同机其他配置目录中对目标账号邮箱及旧版数字 ID 凭据的占用情况。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PeerSecretUsage {
    pub email_in_use_elsewhere: bool,
    pub legacy_id_in_use_by_peer: bool,
}

pub(crate) fn has_env_data_dir_override() -> bool {
    std::env::var_os("KITEMAIL_DATA_DIR").is_some_and(|v| !v.is_empty())
}

fn default_installed_registry_dir() -> Option<PathBuf> {
    dirs::data_local_dir().map(|d| d.join("KiteMail"))
}

fn read_known_instances_raw(registry_dir: &Path) -> Vec<String> {
    let path = registry_dir.join(KNOWN_INSTANCES_FILE);
    if !path.exists() {
        return Vec::new();
    }
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| toml::from_str::<KnownInstancesFile>(&s).ok())
        .map(|f| f.data_dirs)
        .unwrap_or_default()
}

/// 在指定注册目录（通常为 `%LOCALAPPDATA%\Hylki`）中登记当前活跃配置目录，并自动清理已不存在的旧目录。
pub(crate) fn register_known_data_dir_in(
    registry_dir: &Path,
    data_dir: &Path,
) -> Result<(), String> {
    let raw_dirs = read_known_instances_raw(registry_dir);
    let mut seen = std::collections::HashSet::new();
    let mut deduped = Vec::new();

    for raw in &raw_dirs {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let candidate = Path::new(trimmed);
        if !candidate.exists() {
            continue;
        }
        let key = normalized_path_key(candidate);
        if seen.insert(key) {
            deduped.push(trimmed.to_string());
        }
    }

    let current_str = data_dir.to_string_lossy().trim().to_string();
    if !current_str.is_empty() {
        let key = normalized_path_key(data_dir);
        if seen.insert(key) {
            deduped.push(current_str);
        }
    }

    let reg_file = registry_dir.join(KNOWN_INSTANCES_FILE);
    if reg_file.exists() && deduped == raw_dirs {
        return Ok(());
    }

    let text = toml::to_string_pretty(&KnownInstancesFile { data_dirs: deduped })
        .map_err(|e| format!("序列化实例注册表失败: {e}"))?;
    atomic_write_file(registry_dir, &reg_file, &text, "实例注册表")
}

/// 在运行时将当前生效的 `data_dir` 登记到同机共享实例注册表（单测与 `KITEMAIL_DATA_DIR` 隔离模式跳过）。
pub fn register_known_data_dir(data_dir: &Path) {
    if cfg!(test) || has_env_data_dir_override() {
        return;
    }
    if let Some(reg_dir) = default_installed_registry_dir() {
        let _ = register_known_data_dir_in(&reg_dir, data_dir);
    }
}

/// 汇总同机所有已知有效配置目录（含当前目录、默认安装目录与注册表内现存目录），按规范路径键去重。
pub(crate) fn list_known_data_dirs_in(
    registry_dir: &Path,
    current_data_dir: &Path,
) -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();

    let mut push_if_new = |dir: PathBuf| {
        if dir.as_os_str().is_empty() {
            return;
        }
        let key = normalized_path_key(&dir);
        if seen.insert(key) {
            out.push(dir);
        }
    };

    push_if_new(current_data_dir.to_path_buf());
    if registry_dir.exists() {
        push_if_new(registry_dir.to_path_buf());
    }
    for raw in read_known_instances_raw(registry_dir) {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let path = PathBuf::from(trimmed);
        if path.exists() {
            push_if_new(path);
        }
    }
    out
}

/// 统计删除 `(account_id, email)` 时，本目录其他账号及同机其他实例对其邮箱键与旧版 ID 键的占用状态。
pub(crate) fn collect_peer_secret_usage_in(
    registry_dir: &Path,
    current_data_dir: &Path,
    account_id: u32,
    email: &str,
) -> PeerSecretUsage {
    let trimmed_email = email.trim();
    let current_key = normalized_path_key(current_data_dir);
    let mut usage = PeerSecretUsage::default();

    for dir in list_known_data_dirs_in(registry_dir, current_data_dir) {
        let accounts = load_accounts_in(&dir).unwrap_or_default();
        if normalized_path_key(&dir) == current_key {
            if !trimmed_email.is_empty()
                && accounts
                    .iter()
                    .any(|a| a.id != account_id && a.email.eq_ignore_ascii_case(trimmed_email))
            {
                usage.email_in_use_elsewhere = true;
            }
            continue;
        }
        if !trimmed_email.is_empty()
            && accounts
                .iter()
                .any(|a| a.email.eq_ignore_ascii_case(trimmed_email))
        {
            usage.email_in_use_elsewhere = true;
        }
        if accounts.iter().any(|a| a.id == account_id) {
            usage.legacy_id_in_use_by_peer = true;
        }
    }
    usage
}

/// 运行时检查删除账号前是否仍需保留共享邮箱键或旧版 ID 键。
pub(crate) fn collect_peer_secret_usage(
    current_data_dir: &Path,
    account_id: u32,
    email: &str,
) -> PeerSecretUsage {
    if !cfg!(test) && !has_env_data_dir_override() {
        if let Some(reg_dir) = default_installed_registry_dir() {
            return collect_peer_secret_usage_in(&reg_dir, current_data_dir, account_id, email);
        }
    }
    collect_peer_secret_usage_in(current_data_dir, current_data_dir, account_id, email)
}

/// 收集同机所有已知配置目录中使用 `email` 且不与其他邮箱冲突的旧版 `account_id` 列表。
pub(crate) fn non_conflicting_legacy_ids_for_email_in(
    registry_dir: &Path,
    current_data_dir: &Path,
    email: &str,
) -> Vec<u32> {
    let trimmed = email.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let mut owned = std::collections::BTreeSet::new();
    let mut conflicting = std::collections::HashSet::new();

    for dir in list_known_data_dirs_in(registry_dir, current_data_dir) {
        for acc in load_accounts_in(&dir).unwrap_or_default() {
            let acc_email = acc.email.trim();
            if acc_email.eq_ignore_ascii_case(trimmed) {
                owned.insert(acc.id);
            } else if !acc_email.is_empty() {
                conflicting.insert(acc.id);
            }
        }
    }

    owned
        .into_iter()
        .filter(|id| !conflicting.contains(id))
        .collect()
}

/// 运行时收集当前及同机关联目录中属于 `email` 的无冲突旧版 `account_id` 列表。
pub(crate) fn non_conflicting_legacy_ids_for_email(
    current_data_dir: &Path,
    email: &str,
) -> Vec<u32> {
    if !cfg!(test) && !has_env_data_dir_override() {
        if let Some(reg_dir) = default_installed_registry_dir() {
            return non_conflicting_legacy_ids_for_email_in(&reg_dir, current_data_dir, email);
        }
    }
    non_conflicting_legacy_ids_for_email_in(current_data_dir, current_data_dir, email)
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
            ..Default::default()
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
        // WP-07：新用户默认主题对齐 Linux 首位 `system`。
        assert_eq!(defaults.theme_id, "system");
        assert!(defaults.sidebar_favorite_folders.is_empty());
        assert!(defaults.sidebar_hidden_folders.is_empty());
        assert!(defaults.sidebar_special_roles.is_empty());
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

    /// WP-07：侧边栏收藏 / 隐藏 / 特殊角色字段的 TOML 往返。
    #[test]
    fn preferences_sidebar_folder_refs_roundtrip() {
        let base = std::env::temp_dir().join("hylki test 侧边栏文件夹引用");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();

        let prefs = AppPreferences {
            sidebar_favorite_folders: vec![
                SidebarFolderRef {
                    account_id: 1,
                    path: "INBOX/Newsletters".to_string(),
                },
                SidebarFolderRef {
                    account_id: 2,
                    path: "Archive".to_string(),
                },
            ],
            sidebar_hidden_folders: vec![SidebarFolderRef {
                account_id: 1,
                path: "INBOX/LocalFolders/Old".to_string(),
            }],
            sidebar_special_roles: vec![SidebarFolderRole {
                account_id: 2,
                path: "Mail/Outbox mirror".to_string(),
                role: "sent".to_string(),
            }],
            ..AppPreferences::default()
        };
        save_preferences_in(&base, &prefs).expect("save should succeed");
        let loaded = load_preferences_in(&base).expect("load should succeed");
        assert_eq!(
            loaded.sidebar_favorite_folders,
            prefs.sidebar_favorite_folders
        );
        assert_eq!(loaded.sidebar_hidden_folders, prefs.sidebar_hidden_folders);
        assert_eq!(loaded.sidebar_special_roles, prefs.sidebar_special_roles);

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
    fn compose_and_reply_inline_preferences_roundtrip() {
        let defaults = AppPreferences::default();
        assert!(defaults.compose_inline);
        assert!(defaults.reply_inline);

        let toml_str = "compose_inline = false\nreply_inline = false\n";
        let parsed: AppPreferences = toml::from_str(toml_str).expect("parse toml");
        assert!(!parsed.compose_inline);
        assert!(!parsed.reply_inline);
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

    // ---- WP-05 ----

    /// WP-05：空偏好文件反序列化后应与 `AppPreferences::default()` 完全一致。
    #[test]
    fn wp05_empty_preferences_file_yields_defaults() {
        let base = std::env::temp_dir().join("hylki test wp05 空偏好默认");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        fs::write(preferences_file_in(&base), "").expect("write empty prefs");
        let parsed = load_preferences_in(&base).expect("load empty file");
        assert_eq!(parsed, AppPreferences::default());
        let _ = fs::remove_dir_all(&base);
    }

    /// WP-05：专注模式全部布尔默认值对齐 Linux `FocusMode::default()`
    ///（含 2 个 Windows 专属行为位），共 34 项。
    #[test]
    fn wp05_focus_mode_defaults_match_linux() {
        let p = AppPreferences::default();
        // 分组 1 专注模式（enabled 为运行时状态不持久化）。
        assert!(!p.focus_start_enabled); // start_focused
                                         // Windows 专属：隐藏侧边栏 / 隐藏列表窗格。
        assert!(p.focus_hide_sidebar);
        assert!(p.focus_hide_list);
        // 分组 2 侧边栏。
        assert!(p.focus_rail_sidebar);
        assert!(p.focus_unread_counts);
        assert!(p.focus_rail_dots);
        assert!(!p.focus_hover_expand);
        assert!(p.focus_hide_accounts);
        assert!(p.focus_fold_accounts);
        assert!(!p.focus_click_folds);
        assert!(p.focus_fold_unified);
        // 分组 3 统一视图。
        assert!(p.focus_show_inboxes);
        assert!(p.focus_unified_starred);
        assert!(p.focus_unified_sent);
        assert!(p.focus_unified_drafts);
        assert!(p.focus_unified_archive);
        assert!(p.focus_unified_junk);
        assert!(p.focus_unified_trash);
        assert!(p.focus_unified_snoozed);
        // 分组 4 分区。
        assert!(p.focus_show_favorites);
        assert!(p.focus_show_filters);
        assert!(p.focus_show_tags);
        assert!(p.focus_show_contacts);
        assert!(p.focus_show_attachments);
        // 分组 5 消息列表。
        assert_eq!(p.focus_layout, "automatic");
        assert!(!p.focus_column_headings);
        assert_eq!(p.focus_preview_lines, 0);
        assert!(p.focus_list_header);
        assert!(p.focus_hide_avatars);
        assert!(!p.focus_hide_subject);
        assert!(!p.focus_tags_below);
        assert!(p.focus_hide_stars);
        assert!(p.focus_hide_attachment_icons);
        // 分组 6 阅读（旧字段复用：reader_toolbar / reader_view）。
        assert!(p.focus_fold_toolbar); // reader_toolbar
        assert!(p.focus_reader_view); // reader_view
    }

    /// WP-05：旧版 preferences.toml（仅有 5 个旧专注字段、无新字段）加载后
    /// 新字段按默认值补齐，旧值保持不变（兼容映射不丢用户数据）。
    #[test]
    fn wp05_legacy_focus_fields_keep_values_and_fill_new_defaults() {
        let base = std::env::temp_dir().join("hylki test wp05 旧专注字段");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        fs::write(
            preferences_file_in(&base),
            "focus_start_enabled = true\nfocus_reader_view = false\nfocus_hide_sidebar = false\n",
        )
        .expect("write legacy prefs");
        let p = load_preferences_in(&base).expect("load legacy file");
        assert!(p.focus_start_enabled);
        assert!(!p.focus_reader_view);
        assert!(!p.focus_hide_sidebar);
        // 新字段回落默认值。
        assert!(p.focus_list_header);
        assert!(p.focus_unified_starred);
        assert_eq!(p.focus_layout, "automatic");
        let _ = fs::remove_dir_all(&base);
    }

    /// WP-05：黑名单添加 / 移除与邮箱、域名（含子域）匹配语义。
    #[test]
    fn wp05_blocked_senders_add_remove_and_matching() {
        let mut p = AppPreferences::default();
        assert!(p.blocked_senders.is_empty());
        assert!(!p.is_sender_blocked("News <news@spam.example.com>"));

        // 完整邮箱：大小写归一去重。
        assert_eq!(
            p.block_sender("Spam <News@SPAM.example.com>").as_deref(),
            Some("news@spam.example.com")
        );
        assert_eq!(p.blocked_senders.len(), 1);
        assert_eq!(
            p.block_sender("news@spam.example.com").as_deref(),
            Some("news@spam.example.com")
        );
        assert_eq!(p.blocked_senders.len(), 1, "重复添加不产生重复项");

        // 完整邮箱只精确匹配该地址，不放宽到整个域。
        assert!(p.is_sender_blocked("News@Spam.example.com"));
        assert!(!p.is_sender_blocked("bob@spam.example.com"));

        // 裸域名：匹配该域及子域，不匹配相似域。
        assert_eq!(
            p.block_sender("Spam <dan@SPAM.example.com>").as_deref(),
            Some("dan@spam.example.com")
        );
        assert_eq!(
            p.block_sender("spam.example.com").as_deref(),
            Some("spam.example.com")
        );
        assert_eq!(
            p.block_sender("Example.ORG").as_deref(),
            Some("example.org")
        );
        assert!(p.is_sender_blocked("bob@spam.example.com"));
        assert!(p.is_sender_blocked("alice@sub.spam.example.com"));
        assert!(p.is_sender_blocked("dan@example.ORG"));
        assert!(!p.is_sender_blocked("eve@notspam.example.com"));
        assert!(!p.is_sender_blocked("bad@example.org.evil.com"));

        // 无效条目拒绝。
        assert_eq!(p.block_sender("   "), None);
        assert_eq!(p.block_sender("user@"), None);
        assert_eq!(p.block_sender("path/to"), None);
        assert!(!p.is_sender_blocked(""));

        // 移除。
        assert!(p.unblock_sender("News@Spam.example.com"));
        assert!(!p.unblock_sender("missing@example.com"));
        assert_eq!(
            p.blocked_senders,
            vec!["dan@spam.example.com", "spam.example.com", "example.org"]
        );
        assert!(!p.is_sender_blocked("dan@other.org"));
    }

    /// WP-05：工具栏布局 place / move_within / sanitize 与配置往返。
    #[test]
    fn wp05_reader_toolbar_layout_place_and_roundtrip() {
        let mut t = ReaderToolbarLayout::default();
        // 默认左组已满：加入左组无效。
        t.place(ToolbarItem::Print, Some(ToolbarSide::Left), 0);
        assert_eq!(t.left.first(), Some(&ToolbarItem::Reply));
        assert!(t.right.contains(&ToolbarItem::Print));
        // 隐藏再放回左组首位。
        t.place(ToolbarItem::Star, None, 0);
        assert_eq!(t.side(ToolbarItem::Star), None);
        assert_eq!(t.hidden(), vec![ToolbarItem::Star]);
        t.place(ToolbarItem::Star, Some(ToolbarSide::Left), 0);
        assert_eq!(t.left.first(), Some(&ToolbarItem::Star));
        assert_eq!(t.left.len(), ReaderToolbarLayout::LEFT_MAX);
        // 组内移动。
        t.move_within(ToolbarItem::Star, 1);
        assert_eq!(t.left[1], ToolbarItem::Star);
        // 越界不动。
        t.move_within(ToolbarItem::Star, -5);
        assert_eq!(t.left[1], ToolbarItem::Star);
        t.move_within(ToolbarItem::Star, -1);
        assert_eq!(t.left.first(), Some(&ToolbarItem::Star));
        // 未知名跳过 + 左组截断。
        let mut keys: Vec<String> = ToolbarItem::ALL
            .iter()
            .map(|i| i.key().to_string())
            .collect();
        keys.insert(0, "bogus".to_string());
        let layout = ReaderToolbarLayout::from_keys(&keys, &[]);
        assert_eq!(layout.left.len(), ReaderToolbarLayout::LEFT_MAX);
        assert!(layout.right.is_empty());
        assert_eq!(layout.hidden().len(), 6);
        // 往返：left/right keys 再解析保持不变。
        let defaults = ReaderToolbarLayout::default();
        let left_keys: Vec<String> = defaults.left.iter().map(|i| i.key().to_string()).collect();
        let right_keys: Vec<String> = defaults.right.iter().map(|i| i.key().to_string()).collect();
        assert_eq!(
            ReaderToolbarLayout::from_keys(&left_keys, &right_keys),
            defaults
        );
    }

    /// WP-05：列表列 from_keys 始终保留主题列并去重。
    #[test]
    fn wp05_list_columns_from_keys_keeps_subject() {
        let defaults = ListColumn::to_keys(&ListColumn::DEFAULT);
        assert_eq!(ListColumn::from_keys(&defaults).len(), 7);
        // 无主题列时插到发件人之后。
        let keys = vec!["star".to_string(), "sender".to_string(), "date".to_string()];
        let shown = ListColumn::from_keys(&keys);
        assert_eq!(
            shown,
            vec![
                ListColumn::Star,
                ListColumn::Sender,
                ListColumn::Subject,
                ListColumn::Date,
            ]
        );
        // 未知名跳过 + 去重。
        let dup = vec!["star".to_string(), "star".to_string(), "bogus".to_string()];
        let shown = ListColumn::from_keys(&dup);
        assert_eq!(shown, vec![ListColumn::Subject, ListColumn::Star]);
    }

    /// WP-05：通知按钮最多 3 个，from_list 截断与往返。
    #[test]
    fn wp05_notification_buttons_max_three() {
        let d = NotificationButtons::default();
        assert_eq!(d.count(), 3);
        assert!(d.full());
        assert!(d.row_sensitive(NotificationButton::MarkRead));
        assert!(!d.row_sensitive(NotificationButton::Reply));

        // 超过 3 个按 ALL 顺序截断。
        let names: Vec<String> = NotificationButton::ALL
            .iter()
            .map(|b| b.key().to_string())
            .collect();
        let parsed = NotificationButtons::from_list(&names);
        assert_eq!(parsed.count(), 3);
        assert!(parsed.get(NotificationButton::MarkRead));
        assert!(parsed.get(NotificationButton::Archive));
        assert!(parsed.get(NotificationButton::Delete));
        assert!(!parsed.get(NotificationButton::Reply));

        // to_list 往返。
        let mut custom = NotificationButtons::default();
        custom.set(NotificationButton::Archive, false);
        custom.set(NotificationButton::Reply, true);
        let back = NotificationButtons::from_list(&custom.to_list());
        assert_eq!(back, custom);
        // 未知名跳过。
        let junk = vec!["bogus".to_string(), "reply".to_string()];
        let parsed = NotificationButtons::from_list(&junk);
        assert!(parsed.get(NotificationButton::Reply));
        assert_eq!(parsed.count(), 1);
    }

    /// WP-05：本地数据目录占用统计与容量格式化。
    #[test]
    fn wp05_storage_stats_and_format_bytes() {
        let base = std::env::temp_dir().join("hylki test wp05 存储统计");
        let _ = fs::remove_dir_all(&base);
        assert_eq!(
            storage_stats_in(&base),
            StorageStats {
                total_bytes: 0,
                file_count: 0
            }
        );
        fs::create_dir_all(base.join("avatars")).unwrap();
        fs::write(base.join("kitemail-cache.db"), [0u8; 1024]).unwrap();
        fs::write(base.join("avatars").join("a.png"), [0u8; 512]).unwrap();
        let stats = storage_stats_in(&base);
        assert_eq!(stats.file_count, 2);
        assert_eq!(stats.total_bytes, 1536);
        let _ = fs::remove_dir_all(&base);

        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
    }

    /// WP-06：账号高阶配置与 8 个新增设置分类配置序列化往返及旧配置向后兼容。
    #[test]
    fn wp06_account_advanced_and_new_prefs_roundtrip_and_compat() {
        assert!(default_server_saves_sent_for_email("user@gmail.com"));
        assert!(default_server_saves_sent_for_email("user@outlook.com"));
        assert!(!default_server_saves_sent_for_email("user@company.example"));

        let mut acc = AccountConfig::default();
        acc.quota_used_bytes = Some(1_200_000_000);
        acc.quota_total_bytes = Some(15_000_000_000);
        acc.aliases.push(SendAsAlias {
            name: "Sales".to_string(),
            email: "sales@example.com".to_string(),
            reply_to: "support@example.com".to_string(),
            is_default: true,
        });
        acc.signatures = vec![
            NamedSignature {
                name: "商务".to_string(),
                body: "<b>Best</b>".to_string(),
                is_html: true,
            },
            NamedSignature {
                name: "简短".to_string(),
                body: "Thx".to_string(),
                is_html: false,
            },
        ];
        acc.default_new_signature = Some(0);
        acc.default_reply_signature = Some(1);
        acc.special_folders = SpecialFoldersConfig {
            sent: "Sent".to_string(),
            drafts: "Drafts".to_string(),
            trash: "Trash".to_string(),
            junk: "Junk".to_string(),
            archive: "Archive".to_string(),
            server_saves_sent: true,
        };
        acc.sync_interval_minutes = Some(5);
        acc.download_window_days = 90;
        acc.auto_empty_trash_days = 14;
        acc.auto_empty_junk_days = 7;
        acc.hidden_folders = vec!["[Gmail]/Important".to_string()];

        let toml_str = toml::to_string(&acc).unwrap();
        let back: AccountConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(back, acc);

        let mut prefs = AppPreferences::default();
        prefs.openpgp_wkd_lookup = true;
        prefs.cloud_auto_upload_threshold_mb = 50;
        prefs.translation_engine = "deepl".to_string();
        prefs.mcp_enabled = true;
        prefs.mcp_allow_draft = true;
        let prefs_toml = toml::to_string(&prefs).unwrap();
        let prefs_back: AppPreferences = toml::from_str(&prefs_toml).unwrap();
        assert_eq!(prefs_back, prefs);

        // 空 TOML 应能以默认值反序列化，不破坏旧版配置。
        let legacy: AppPreferences = toml::from_str("").unwrap();
        assert_eq!(legacy.cloud_auto_upload_threshold_mb, 25);
        assert_eq!(legacy.mcp_socket_path, r"\\.\pipe\kitemail-mcp");
    }

    #[test]
    fn resolve_data_dir_handles_env_portable_and_installed_modes() {
        let exe_dir = Path::new(r"D:\Tools\KiteMailPortable");
        let local_app_data = Path::new(r"C:\Users\Alice\AppData\Local");

        assert_eq!(
            resolve_data_dir(
                Some(r"E:\CustomData"),
                true,
                Some(exe_dir),
                Some(local_app_data),
            ),
            PathBuf::from(r"E:\CustomData")
        );
        assert_eq!(
            resolve_data_dir(Some("   "), true, Some(exe_dir), Some(local_app_data)),
            PathBuf::from(r"D:\Tools\KiteMailPortable\data")
        );
        assert_eq!(
            resolve_data_dir(None, false, Some(exe_dir), Some(local_app_data)),
            PathBuf::from(r"C:\Users\Alice\AppData\Local\KiteMail")
        );
    }

    #[test]
    fn storage_config_and_account_dir_normalization_work() {
        let base = std::env::temp_dir().join(format!("hylki-storage-cfg-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);

        // 缺失 storage.toml 时默认使用 `<app_data_dir>/mail`。
        assert_eq!(resolve_mail_data_dir_in(&base), base.join("mail"));

        // 相对路径相对 app_data_dir 解析。
        save_storage_config_in(
            &base,
            &StorageConfig {
                mail_data_dir: Some("custom-mail".to_string()),
            },
        )
        .unwrap();
        assert_eq!(resolve_mail_data_dir_in(&base), base.join("custom-mail"));

        // 绝对路径原样生效，空值回退默认。
        let custom_abs = base.join("external_mail_root");
        save_storage_config_in(
            &base,
            &StorageConfig {
                mail_data_dir: Some(custom_abs.to_string_lossy().into_owned()),
            },
        )
        .unwrap();
        assert_eq!(resolve_mail_data_dir_in(&base), custom_abs);

        save_storage_config_in(
            &base,
            &StorageConfig {
                mail_data_dir: None,
            },
        )
        .unwrap();
        assert_eq!(resolve_mail_data_dir_in(&base), base.join("mail"));

        // 账号目录名规范化与防冲突测试。
        assert_eq!(
            normalize_email_storage_dir("  User.Name@Outlook.COM "),
            "user.name@outlook.com"
        );
        assert_eq!(normalize_email_storage_dir("a:b/c*d"), "a_b_c_d");
        assert_eq!(normalize_email_storage_dir("CON"), "_con");
        assert!(!is_valid_single_dir_name("../escape"));
        assert!(!is_valid_single_dir_name("a/b"));
        assert!(!is_valid_single_dir_name("aux"));
        assert!(is_valid_single_dir_name("zjlkim@outlook.com"));

        let mut a1 = AccountConfig::default();
        a1.id = 1;
        a1.email = "dup@example.com".to_string();
        let mut a2 = AccountConfig::default();
        a2.id = 2;
        a2.email = "DUP@example.com".to_string();
        let all = vec![a1.clone(), a2.clone()];
        assert_eq!(resolve_account_storage_dir(&a1, &all), "dup@example.com");
        assert_eq!(resolve_account_storage_dir(&a2, &all), "dup@example.com_2");

        // 组合空间统计在默认嵌套目录下不应重复计算 mail 子目录。
        let acc_dir = resolve_mail_data_dir_in(&base).join("dup@example.com");
        fs::create_dir_all(&acc_dir).unwrap();
        fs::write(global_db_in(&base), [0u8; 400]).unwrap();
        fs::write(acc_dir.join("mail.db"), [0u8; 1024]).unwrap();
        save_accounts_in(&base, &[a1.clone()]).unwrap();

        let summary = app_storage_summary_in(&base);
        assert!(!summary.is_custom_mail_dir);
        assert_eq!(summary.mail_stats.total_bytes, 1024);
        assert_eq!(summary.per_account.len(), 1);
        assert_eq!(summary.per_account[0].stats.total_bytes, 1024);
        assert!(summary.config_stats.total_bytes < 1024);

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn known_instances_registry_dedupes_prunes_and_guards_bidirectional_secrets() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!(
            "hylki-known-instances-{}-{unique}",
            std::process::id()
        ));
        let installed = base.join("installed");
        let portable_a = base.join("portable_a");
        let portable_b = base.join("portable_b");
        let stale = base.join("deleted_portable");
        fs::create_dir_all(&installed).unwrap();
        fs::create_dir_all(&portable_a).unwrap();
        fs::create_dir_all(&portable_b).unwrap();
        fs::create_dir_all(&stale).unwrap();

        let make_acc = |id: u32, email: &str| AccountConfig {
            id,
            email: email.to_string(),
            ..Default::default()
        };

        // 1. 注册并去重，且无变化时不重写（或保持内容一致），不存在目录会被自动清理。
        register_known_data_dir_in(&installed, &stale).unwrap();
        register_known_data_dir_in(&installed, &portable_a).unwrap();
        register_known_data_dir_in(&installed, &portable_a).unwrap();
        register_known_data_dir_in(&installed, &portable_b).unwrap();
        fs::remove_dir_all(&stale).unwrap();
        register_known_data_dir_in(&installed, &portable_a).unwrap();

        let dirs = list_known_data_dirs_in(&installed, &installed);
        assert_eq!(dirs.len(), 3);
        let raw_after = read_known_instances_raw(&installed);
        assert_eq!(raw_after.len(), 2);

        // 2. 配置账号：
        // - installed: id=1 (alice), id=2 (shared_id_diff_email)
        // - portable_a: id=3 (alice), id=2 (bob) -> id=2 与 installed 邮箱冲突
        // - portable_b: id=4 (alice)
        save_accounts_in(
            &installed,
            &[
                make_acc(1, "Alice@Outlook.com"),
                make_acc(2, "InstalledOnly@Example.com"),
            ],
        )
        .unwrap();
        save_accounts_in(
            &portable_a,
            &[
                make_acc(3, "alice@outlook.com"),
                make_acc(2, "bob@test.com"),
            ],
        )
        .unwrap();
        save_accounts_in(&portable_b, &[make_acc(4, "ALICE@OUTLOOK.COM")]).unwrap();

        // 安装版删除 alice(1) -> portable_a/portable_b 仍用 alice
        assert_eq!(
            collect_peer_secret_usage_in(&installed, &installed, 1, "alice@outlook.com"),
            PeerSecretUsage {
                email_in_use_elsewhere: true,
                legacy_id_in_use_by_peer: false,
            }
        );

        // 便携版 B 删除 alice(4) -> installed/portable_a 仍用 alice
        assert_eq!(
            collect_peer_secret_usage_in(&installed, &portable_b, 4, "alice@outlook.com"),
            PeerSecretUsage {
                email_in_use_elsewhere: true,
                legacy_id_in_use_by_peer: false,
            }
        );

        // 安装版删除 InstalledOnly(2) -> 邮箱无人共用，但 portable_a 也使用了 account_id=2
        assert_eq!(
            collect_peer_secret_usage_in(&installed, &installed, 2, "InstalledOnly@Example.com"),
            PeerSecretUsage {
                email_in_use_elsewhere: false,
                legacy_id_in_use_by_peer: true,
            }
        );

        // 只在 installed(1) 与 portable_a(3)、portable_b(4) 中存在且无冲突的 alice 旧版 ID
        assert_eq!(
            non_conflicting_legacy_ids_for_email_in(&installed, &portable_a, "alice@outlook.com"),
            vec![1, 3, 4]
        );
        // id=2 同时被两个不同邮箱使用，因此 InstalledOnly 和 bob 的无冲突旧 ID 列表都必须排除 id=2
        assert!(non_conflicting_legacy_ids_for_email_in(
            &installed,
            &installed,
            "InstalledOnly@Example.com"
        )
        .is_empty());

        // 若从 portable_a 与 portable_b 移除 alice 后再删 installed(1)，应彻底可删
        save_accounts_in(&portable_a, &[make_acc(2, "bob@test.com")]).unwrap();
        save_accounts_in(&portable_b, &[]).unwrap();
        assert_eq!(
            collect_peer_secret_usage_in(&installed, &installed, 1, "alice@outlook.com"),
            PeerSecretUsage {
                email_in_use_elsewhere: false,
                legacy_id_in_use_by_peer: false,
            }
        );

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn account_default_signature_respects_none_and_reply_selection() {
        let sigs = vec![
            NamedSignature {
                name: "商务".to_string(),
                body: "商务签名".to_string(),
                is_html: false,
            },
            NamedSignature {
                name: "简短".to_string(),
                body: "<p>简短签名</p>".to_string(),
                is_html: true,
            },
        ];

        // 即使存在签名列表（甚至旧字段非空），选择「不使用签名」(None) 也必须返回空。
        let none_acc = AccountConfig {
            signature: "旧签名".to_string(),
            signatures: sigs.clone(),
            default_new_signature: None,
            default_reply_signature: None,
            ..Default::default()
        };
        assert_eq!(none_acc.primary_signature(), (String::new(), false));
        assert_eq!(none_acc.reply_signature(), (String::new(), false));

        // 分别指定新邮件与回复/转发默认签名。
        let split_acc = AccountConfig {
            signatures: sigs.clone(),
            default_new_signature: None,
            default_reply_signature: Some(1),
            ..Default::default()
        };
        assert_eq!(split_acc.primary_signature(), (String::new(), false));
        assert_eq!(
            split_acc.reply_signature(),
            ("<p>简短签名</p>".to_string(), true)
        );

        let new_only_acc = AccountConfig {
            signatures: sigs,
            default_new_signature: Some(0),
            default_reply_signature: None,
            ..Default::default()
        };
        assert_eq!(
            new_only_acc.primary_signature(),
            ("商务签名".to_string(), false)
        );
        assert_eq!(new_only_acc.reply_signature(), (String::new(), false));

        // 尚未迁移命名签名列表时，保留旧的单签名回退。
        let legacy_acc = AccountConfig {
            signature: "旧纯文本签名".to_string(),
            ..Default::default()
        };
        assert_eq!(
            legacy_acc.primary_signature(),
            ("旧纯文本签名".to_string(), false)
        );
        assert_eq!(
            legacy_acc.reply_signature(),
            ("旧纯文本签名".to_string(), false)
        );
    }

    #[test]
    fn legacy_signature_above_quote_false_migrates_once_then_preserves_choice() {
        let base = std::env::temp_dir().join("hylki test 签名位置一次性迁移");
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();

        // 模拟 WP-04 之前生成的配置：显式落盘了 signature_above_quote = false 且无迁移标记。
        fs::write(
            preferences_file_in(&base),
            "theme_id = \"default\"\nsignature_above_quote = false\n",
        )
        .expect("write legacy prefs");

        let migrated = load_preferences_in(&base).expect("load legacy prefs");
        assert!(migrated.signature_above_quote);
        assert!(migrated.signature_above_quote_default_applied);

        // 迁移后若用户再次显式改回「在引用内容下方」，后续加载必须保留用户选择。
        let mut user_below = migrated.clone();
        user_below.signature_above_quote = false;
        save_preferences_in(&base, &user_below).expect("save user choice");

        let reloaded = load_preferences_in(&base).expect("reload user choice");
        assert!(!reloaded.signature_above_quote);
        assert!(reloaded.signature_above_quote_default_applied);

        let _ = fs::remove_dir_all(&base);
    }
}
