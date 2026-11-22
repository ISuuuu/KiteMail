//! Core mail domain models, UI-worker command/event protocol, local SQLite cache,
//! Windows Credential Manager secrets, and IMAP/SMTP worker runtime.

pub mod cache;
pub mod config;
pub mod contacts_io;
pub mod demo;
pub mod graph_client;
pub mod imap_client;
pub mod oauth;
pub mod search_query;
pub mod secrets;
pub mod smtp_client;
pub mod worker;

pub use search_query::SearchQuery;
use serde::{Deserialize, Serialize};

use crate::{
    cache::{CachedAttachment, CachedAttachmentInput, KnownContact},
    config::AccountConfig,
};

/// Summary of a configured mail account shown in the left sidebar.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountSummary {
    pub id: u32,
    pub display_name: String,
    pub email: String,
}

impl From<&AccountConfig> for AccountSummary {
    fn from(cfg: &AccountConfig) -> Self {
        Self {
            id: cfg.id,
            display_name: if cfg.display_name.is_empty() {
                cfg.email.clone()
            } else {
                cfg.display_name.clone()
            },
            email: cfg.email.clone(),
        }
    }
}

/// Summary of a mailbox folder belonging to an account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderSummary {
    pub id: u32,
    pub account_id: u32,
    pub name: String,
    pub path: String,
    pub unread_count: u32,
    pub total_count: usize,
    /// Whether the remote server still has older messages beyond what is cached locally.
    #[serde(default)]
    pub has_more_remote: bool,
}

/// Lightweight message row used for virtualized / paginated list rendering.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageSummary {
    pub id: u32,
    pub account_id: u32,
    pub folder_id: u32,
    pub folder_path: String,
    pub uid: u32,
    pub sender: String,
    pub subject: String,
    pub preview: String,
    #[serde(default)]
    pub date_ts: i64,
    pub date_display: String,
    pub unread: bool,
    pub flagged: bool,
    pub has_attachment: bool,
    /// 收件人（B-6：已发送 / 草稿视图列表行显示「致：…」）。
    #[serde(default)]
    pub recipients: String,
    /// 同一会话（按归一化主题聚合）的邮件总数，`> 1` 时列表行显示计数胶囊（B-4）。
    #[serde(default)]
    pub thread_count: u32,
}

/// Cached attachment metadata shown in the reading pane.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentMeta {
    pub id: u32,
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: usize,
}

/// One message card within a conversation thread shown in the reading pane.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadMessage {
    pub id: u32,
    #[serde(default)]
    pub rfc_message_id: String,
    #[serde(default)]
    pub subject: String,
    pub sender: String,
    pub recipients: String,
    #[serde(default)]
    pub date_ts: i64,
    pub date_display: String,
    pub body_text: String,
    #[serde(default)]
    pub body_html: Option<String>,
    pub body_loaded: bool,
    #[serde(default)]
    pub unread: bool,
    pub flagged: bool,
    pub attachments: Vec<AttachmentMeta>,
    /// 该会话卡片的 IMAP UID 与所在文件夹（供卡片 `...` 菜单的标记 / 导出操作）。
    #[serde(default)]
    pub uid: u32,
    #[serde(default)]
    pub folder_path: String,
}

/// Full message payload shown in the reading pane.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageDetail {
    pub id: u32,
    pub account_id: u32,
    pub folder_id: u32,
    pub folder_path: String,
    pub uid: u32,
    pub rfc_message_id: String,
    pub sender: String,
    pub recipients: String,
    pub subject: String,
    #[serde(default)]
    pub date_ts: i64,
    pub date_display: String,
    pub body_text: String,
    #[serde(default)]
    pub body_html: Option<String>,
    pub flagged: bool,
    pub attachments: Vec<AttachmentMeta>,
    /// Chronological messages belonging to the same conversation thread (empty or 1 item for standalone mails).
    #[serde(default)]
    pub thread_messages: Vec<ThreadMessage>,
    /// F-7 一键退订：`List-Unsubscribe` 头中的第一个可执行地址
    /// （http/https/mailto），正文抓取时随头解析并存入缓存；无该头时为 `None`。
    #[serde(default)]
    pub unsubscribe_url: Option<String>,
    /// F-7 一键退订：是否携带 `List-Unsubscribe-Post: List-Unsubscribe=One-Click`
    /// （RFC 8058），此时可静默 POST 而无需打开浏览器。
    #[serde(default)]
    pub unsubscribe_one_click: bool,
}

/// 一次完整正文抓取（IMAP `RFC822` / Graph 消息）解析出的载荷；
/// 退订头在抓取时一并读出并随正文落库（对齐原版「正文抓取时读句柄」）。
#[derive(Clone, Debug, Default)]
pub struct FetchedMessageBody {
    pub body_text: String,
    pub body_html: Option<String>,
    pub attachments: Vec<CachedAttachmentInput>,
    pub unsubscribe_url: Option<String>,
    pub unsubscribe_one_click: bool,
}

/// 用户手动维护的联系人（`user_contacts` 表行）；`id == 0` 表示待插入的新联系人。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserContact {
    pub id: u32,
    pub email: String,
    pub display_name: String,
    pub notes: String,
}

/// Persisted local compose draft recoverable across restarts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalDraft {
    pub id: u32,
    pub account_id: u32,
    pub to_addr: String,
    /// Comma-separated Cc addresses (empty when unset).
    pub cc_addr: String,
    /// Comma-separated Bcc addresses (empty when unset).
    pub bcc_addr: String,
    pub subject: String,
    pub body_text: String,
    pub in_reply_to: String,
    pub attachment_paths: Vec<String>,
    pub updated_at: i64,
}

/// One page of message summaries returned from the cache or demo store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MailPage {
    pub folder_id: u32,
    pub offset: usize,
    pub total: usize,
    pub items: Vec<MessageSummary>,
}

/// Draft fields submitted from the compose / reply pane.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComposeDraft {
    pub to: String,
    /// Comma-separated Cc addresses (empty when unset).
    pub cc: String,
    /// Comma-separated Bcc addresses (empty when unset).
    pub bcc: String,
    pub subject: String,
    pub body: String,
    pub in_reply_to: String,
    pub attachment_paths: Vec<String>,
}

/// Lifecycle state of a persisted Outbox entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutboxStatus {
    Queued,
    Sending,
    /// SMTP connection dropped or timed out before confirmation; never auto-resend.
    Uncertain,
    Failed,
}

impl OutboxStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Sending => "sending",
            Self::Uncertain => "uncertain",
            Self::Failed => "failed",
        }
    }

    pub fn from_str(value: &str) -> Self {
        match value {
            "sending" => Self::Sending,
            "uncertain" => Self::Uncertain,
            "failed" => Self::Failed,
            _ => Self::Queued,
        }
    }
}

/// Persisted outgoing message record in the local Outbox journal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutboxItem {
    pub id: u32,
    pub account_id: u32,
    pub to_addr: String,
    /// Comma-separated Cc addresses (empty when unset).
    pub cc_addr: String,
    /// Comma-separated Bcc addresses (empty when unset).
    pub bcc_addr: String,
    pub subject: String,
    pub body_text: String,
    pub in_reply_to: String,
    pub attachment_paths: Vec<String>,
    pub status: OutboxStatus,
    pub last_error: String,
}

/// Unified cross-account views backed by dedicated cache queries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnifiedView {
    Starred,
    Sent,
    Archive,
}

/// Server/cache-side single-choice filter (kept for backward compatibility).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageFilter {
    #[default]
    All,
    Unread,
    Starred,
}

/// Multi-select quick filter conditions applied to message list queries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuickFilterSet {
    pub unread: bool,
    pub starred: bool,
    pub has_attachment: bool,
    pub today: bool,
}

impl QuickFilterSet {
    /// Convert legacy single-choice [`MessageFilter`] into a [`QuickFilterSet`].
    pub fn from_message_filter(filter: MessageFilter) -> Self {
        match filter {
            MessageFilter::All => Self::default(),
            MessageFilter::Unread => Self {
                unread: true,
                ..Self::default()
            },
            MessageFilter::Starred => Self {
                starred: true,
                ..Self::default()
            },
        }
    }

    /// True when no quick filter condition is enabled.
    pub fn is_empty(self) -> bool {
        !self.unread && !self.starred && !self.has_attachment && !self.today
    }

    /// Number of active quick filter conditions.
    pub fn active_count(self) -> usize {
        usize::from(self.unread)
            + usize::from(self.starred)
            + usize::from(self.has_attachment)
            + usize::from(self.today)
    }

    /// Clear all active quick filter conditions.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Human-readable labels of active quick filters.
    pub fn active_labels(self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.unread {
            out.push("仅未读");
        }
        if self.starred {
            out.push("仅星标");
        }
        if self.has_attachment {
            out.push("含附件");
        }
        if self.today {
            out.push("今天");
        }
        out
    }
}

/// Sort order for message list queries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageSort {
    #[default]
    DateNewest,
    DateOldest,
    SenderAsc,
    SubjectAsc,
    UnreadFirst,
    StarredFirst,
}

impl MessageSort {
    /// All 6 sort orders in menu display order.
    pub const ALL: [Self; 6] = [
        Self::DateNewest,
        Self::DateOldest,
        Self::SenderAsc,
        Self::SubjectAsc,
        Self::UnreadFirst,
        Self::StarredFirst,
    ];

    /// Localized Chinese label for the sort dropdown.
    pub fn label(self) -> &'static str {
        match self {
            Self::DateNewest => "日期（最新优先）",
            Self::DateOldest => "日期（最早优先）",
            Self::SenderAsc => "发件人（A-Z）",
            Self::SubjectAsc => "主题（A-Z）",
            Self::UnreadFirst => "未读优先",
            Self::StarredFirst => "星标优先",
        }
    }

    /// Whether date-group headers (`今天` / `昨天` / `本周` / `更早`) should be shown.
    pub fn uses_date_groups(self) -> bool {
        matches!(self, Self::DateNewest | Self::DateOldest)
    }
}

/// Search scope for local message search.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchScope {
    #[default]
    ThisFolder,
    AllFolders,
}

/// Local Unix timestamp for the start of today (00:00:00 local time).
pub fn start_of_today_ts() -> i64 {
    use chrono::TimeZone;
    let today = chrono::Local::now().date_naive();
    today
        .and_hms_opt(0, 0, 0)
        .and_then(|dt| chrono::Local.from_local_datetime(&dt).earliest())
        .map(|dt| dt.timestamp())
        .unwrap_or(0)
}

/// Classify a message's Unix timestamp into a date-group header (`今天`, `昨天`, `本周`, `更早`)
/// relative to `now_ts` in the local timezone.
pub fn date_group_label(date_ts: i64, now_ts: i64) -> &'static str {
    if date_ts <= 0 {
        return "更早";
    }
    let Some(msg_dt) = chrono::DateTime::from_timestamp(date_ts, 0) else {
        return "更早";
    };
    let Some(now_dt) = chrono::DateTime::from_timestamp(now_ts, 0) else {
        return "更早";
    };
    let msg_day = msg_dt.with_timezone(&chrono::Local).date_naive();
    let today = now_dt.with_timezone(&chrono::Local).date_naive();
    let delta_days = today.signed_duration_since(msg_day).num_days();
    if delta_days <= 0 {
        "今天"
    } else if delta_days == 1 {
        "昨天"
    } else if delta_days <= 6 {
        "本周"
    } else {
        "更早"
    }
}

/// Commands sent from the GPUI frontend to the mail worker.
#[derive(Clone, Debug)]
pub enum MailRequest {
    VerifyAccount {
        config: AccountConfig,
        imap_password: String,
        smtp_password: Option<String>,
    },
    SaveAndSyncAccount {
        config: AccountConfig,
        imap_password: String,
        smtp_password: Option<String>,
    },
    StartMicrosoftOAuth {
        custom_client_id: String,
        editing_account_id: Option<u32>,
    },
    DeleteAccount {
        account_id: u32,
    },
    LoadAllCached,
    SyncAllAccounts,
    SyncAccount {
        account_id: u32,
    },
    SyncFolder {
        account_id: u32,
        folder_id: u32,
        folder_path: String,
    },
    LoadUnifiedInbox {
        query: String,
        filter: MessageFilter,
        quick_filters: QuickFilterSet,
        sort: MessageSort,
        offset: usize,
        limit: usize,
    },
    LoadMessagesPage {
        folder_id: u32,
        query: String,
        filter: MessageFilter,
        quick_filters: QuickFilterSet,
        sort: MessageSort,
        offset: usize,
        limit: usize,
    },
    /// Cross-account / cross-folder local search (excludes Trash/Junk by default, deduped by message_id).
    SearchAllFolders {
        query: String,
        quick_filters: QuickFilterSet,
        sort: MessageSort,
        offset: usize,
        limit: usize,
    },
    /// Online remote server search (IMAP UID SEARCH / Microsoft Graph `$search`).
    SearchServer {
        query: String,
        account_id: Option<u32>,
    },
    LoadMessageBody {
        account_id: u32,
        message_id: u32,
    },
    /// Fetch the complete raw RFC 822 message bytes for .eml export
    /// (IMAP `UID FETCH (BODY.PEEK[])` / Graph `/$value`).
    FetchRawMessage {
        account_id: u32,
        message_id: u32,
        uid: u32,
        folder_path: String,
    },
    SetSeen {
        account_id: u32,
        folder_path: String,
        message_id: u32,
        uid: u32,
        seen: bool,
    },
    SetFlagged {
        account_id: u32,
        folder_path: String,
        message_id: u32,
        uid: u32,
        flagged: bool,
    },
    ArchiveMessage {
        account_id: u32,
        message_id: u32,
    },
    DeleteMessage {
        account_id: u32,
        message_id: u32,
    },
    MoveMessage {
        account_id: u32,
        message_id: u32,
        target_folder_path: String,
    },
    /// F-3 撤销移动 / 删除 / 归档：在 `current_folder_path` 中按 RFC Message-ID
    /// 定位邮件并移回 `original_folder_path`（邮件已不在本地缓存，需服务端检索）。
    UndoMessageMove {
        account_id: u32,
        message_id: u32,
        rfc_message_id: String,
        current_folder_path: String,
        original_folder_path: String,
    },
    MarkSpam {
        account_id: u32,
        message_id: u32,
    },
    ExportForwardAttachments {
        message_id: u32,
    },
    SaveAttachment {
        attachment_id: u32,
        dest_path: std::path::PathBuf,
    },
    SaveAllAttachments {
        attachment_ids: Vec<u32>,
        dest_dir: std::path::PathBuf,
    },
    SaveLocalDraft {
        draft: LocalDraft,
    },
    DeleteLocalDraft {
        draft_id: u32,
    },
    SendDraft {
        account_id: u32,
        draft: ComposeDraft,
        local_draft_id: Option<u32>,
        /// 撤销发送客户端令牌（`send_at` 为 `Some` 时用于 `OutboxEnqueued` 回执匹配）。
        undo_token: u64,
        /// `Some(延迟秒数)` = 仅入队不投递（撤销发送缓冲，到期后由启动恢复 /
        /// 重新启用账号 / 显式重试投递）；`None` = 入队后立即投递。
        send_at: Option<i64>,
    },
    RetryOutboxItem {
        account_id: u32,
        outbox_id: u32,
    },
    /// 撤销发送：删除仍在排队（Queued）的发件箱记录；已被领取投递的记录不可撤销。
    DeleteOutboxItem {
        account_id: u32,
        outbox_id: u32,
    },
    PrepareQuit,
    UndoOutboxItem {
        account_id: u32,
        outbox_id: u32,
        token: u64,
    },
    LoadUnifiedView {
        view: UnifiedView,
        query: String,
        filter: MessageFilter,
        quick_filters: QuickFilterSet,
        sort: MessageSort,
        offset: usize,
        limit: usize,
    },
    ListCachedAttachments {
        query: String,
        offset: usize,
        limit: usize,
    },
    ListKnownContacts {
        query: String,
        offset: usize,
        limit: usize,
    },
    /// Fetch the next batch of older messages from the remote server for `folder_id`
    /// (`0` = all inbox folders across configured accounts).
    LoadOlderMessages {
        folder_id: u32,
        limit: usize,
    },
    /// 任务四后台历史索引：拉取一小批更早邮件入库（worker 自选下一个待索引的
    /// 统一收件箱文件夹）。单队列事件驱动：批次完成后由 app 端决定是否续排，
    /// 已入队的读信、搜索和发信请求自然插队，同一时刻最多一个在途批次。
    BackfillHistoryBatch,
    /// Clear all cached message bodies (headers and accounts are kept).
    ClearBodyCaches,
    /// Batch-mark every message in `folder_path` as read on the server
    /// (IMAP: chunked `UID STORE +FLAGS (\Seen)`; Graph: per-message update
    /// over cached unread messages), zero the folder's local unread count
    /// and emit refreshed folders.
    MarkFolderAllRead {
        account_id: u32,
        folder_id: u32,
        folder_path: String,
    },
    /// Empty a Trash / Junk folder on the server: IMAP marks every message
    /// `\Deleted` and expunges. Not supported for Graph accounts (dropped in
    /// the UI); clears the folder's local cache and emits refreshed folders.
    EmptyFolder {
        account_id: u32,
        folder_id: u32,
        folder_path: String,
    },
    /// F-4 草稿自动保存：静默变体（不回发「草稿已保存」状态行，仅回发
    /// `DraftsUpdated` 与 `DraftSaved`，避免打字期间状态栏抖动）。
    AutosaveLocalDraft {
        draft: LocalDraft,
    },
    /// F-5 用户联系人写入（`id == 0` 插入，其余按 id 更新；邮箱唯一冲突报错）。
    SaveUserContact {
        contact: UserContact,
    },
    /// F-5 删除用户联系人（仅 `user_contacts` 表中的自定义联系人可删）。
    DeleteUserContact {
        contact_id: u32,
    },
    /// F-5 批量导入用户联系人（按邮箱去重，已存在的跳过）。
    ImportUserContacts {
        contacts: Vec<UserContact>,
    },
    /// F-6 附件总览图片预览：读取本地缓存的附件字节。
    FetchAttachmentBytes {
        attachment_id: u32,
    },
    /// F-7 一键退订（RFC 8058）：对 https 句柄 POST `List-Unsubscribe=One-Click`。
    UnsubscribeOneClick {
        url: String,
    },
    /// Enable or disable background IMAP IDLE / Graph pulse push watchers.
    SetPushEnabled {
        enabled: bool,
    },
    /// Persist reordered account list in `accounts.toml`.
    ReorderAccounts {
        account_ids: Vec<u32>,
    },
    /// Enable or disable a single account in `accounts.toml` and update watchers/sync.
    SetAccountEnabled {
        account_id: u32,
        enabled: bool,
    },
}

/// Events emitted from the mail worker back to the GPUI frontend.
#[derive(Clone, Debug)]
pub enum WorkerEvent {
    AccountVerified {
        ok: bool,
        message: String,
    },
    AccountSaved {
        account: AccountConfig,
    },
    AccountDeleted {
        account_id: u32,
    },
    /// Full reordered/updated account list emitted after reorder or enable toggle.
    AccountsUpdated {
        accounts: Vec<AccountConfig>,
    },
    FoldersLoaded {
        account_id: u32,
        folders: Vec<FolderSummary>,
    },
    MessagesPageLoaded(MailPage),
    MessageBodyLoaded(MessageDetail),
    /// Complete raw MIME bytes fetched for one message (.eml export).
    RawMessageFetched {
        message_id: u32,
        bytes: Vec<u8>,
    },
    DraftsUpdated {
        drafts: Vec<LocalDraft>,
    },
    OutboxUpdated {
        account_id: u32,
        items: Vec<OutboxItem>,
    },
    /// 撤销发送缓冲：待发记录已持久化到发件箱（Queued + 到期时间），
    /// UI 据此绑定撤销 / 立即发送目标，并确认原草稿已交接落盘。
    QuitPrepared {
        result: Result<(), String>,
    },
    OutboxUndoResult {
        token: u64,
        result: Result<Option<u32>, String>,
    },
    OutboxEnqueueFailed {
        token: u64,
        message: String,
        recovered_draft_id: Option<u32>,
    },
    OutboxEnqueued {
        token: u64,
        account_id: u32,
        outbox_id: u32,
    },
    /// Microsoft OAuth 重授权返回了与编辑中账号不一致的邮箱：保留原账号、凭据与
    /// 缓存完全不动，提示用户重新选择原邮箱授权或通过「添加账号」流程接入新邮箱。
    OAuthRebindRejected {
        account_id: Option<u32>,
        returned_email: String,
    },
    DraftSent {
        account_id: u32,
        subject: String,
    },
    AttachmentSaved {
        path: std::path::PathBuf,
    },
    AttachmentsSaved {
        count: usize,
        dest_dir: std::path::PathBuf,
    },
    /// A message was moved to the account's archive folder on the server and
    /// removed from the local cache; the UI should refresh its lists.
    /// `dest_folder_path` / `rfc_message_id` 供撤销使用；`dest_folder_path` 为空
    /// 表示无法定位邮件（不提供撤销）。
    MessageArchived {
        account_id: u32,
        message_id: u32,
        dest_folder_path: String,
        rfc_message_id: String,
    },
    /// A message was moved to the account's trash folder (or permanently
    /// deleted) on the server and removed from the local cache.
    MessageDeleted {
        account_id: u32,
        message_id: u32,
        dest_folder_path: String,
        rfc_message_id: String,
    },
    /// A page of cross-folder local search results was loaded.
    AllFoldersSearchLoaded(MailPage),
    /// Online server search results (IMAP / Graph) for `query`.
    ServerSearchResults {
        query: String,
        items: Vec<MessageSummary>,
    },
    /// A message was moved to an explicit target folder on the server and
    /// removed from the old folder's local cache.
    MessageMoved {
        account_id: u32,
        message_id: u32,
        target_folder_path: String,
        rfc_message_id: String,
    },
    /// A message was moved to the account's spam/junk folder on the server and
    /// removed from the old folder's local cache.
    MessageMarkedSpam {
        account_id: u32,
        message_id: u32,
        dest_folder_path: String,
        rfc_message_id: String,
    },
    /// Cached attachments of a forwarded message were exported to a temp directory.
    ForwardAttachmentsExported {
        message_id: u32,
        paths: Vec<String>,
    },
    /// A page of one unified view (starred / sent / archive) was loaded.
    UnifiedViewLoaded {
        view: UnifiedView,
        page: MailPage,
    },
    /// A page of cached attachments was loaded for the gallery view.
    CachedAttachmentsLoaded {
        items: Vec<CachedAttachment>,
    },
    /// Aggregated known contacts were loaded for the contacts view.
    KnownContactsLoaded {
        items: Vec<KnownContact>,
    },
    /// F-4 草稿落库完成（含新增分配的本地 ID；写信页据此绑定 `compose_draft_id`
    /// 并在自动保存后显示「草稿已自动保存 HH:MM」）。
    DraftSaved {
        draft_id: u32,
    },
    /// F-5 批量导入结果（`skipped` 为因邮箱重复被跳过的条数）。
    ContactsImported {
        added: usize,
        skipped: usize,
    },
    /// F-6 附件总览图片预览所需的附件字节（仅图片附件点击时抓取）。
    AttachmentBytesFetched {
        attachment_id: u32,
        filename: String,
        mime_type: String,
        bytes: Vec<u8>,
    },
    /// Result of fetching an older batch of messages from the remote server.
    OlderMessagesLoaded {
        folder_id: u32,
        fetched_count: usize,
        has_more_remote: bool,
    },
    /// 任务四后台历史索引：单文件夹单批完成。`folder_id == 0` 表示已无待索引
    /// 目标（全部拉完或全部放弃），app 端据此停止续排；`has_more_remote` 为
    /// 处理后是否仍有待索引的收件箱（已过滤停用 / 退出统一视图 / 本会话放弃）。
    BackfillHistoryBatchDone {
        folder_id: u32,
        fetched_count: usize,
        body_only: bool,
        has_more_remote: bool,
    },
    Status {
        account_id: Option<u32>,
        text: String,
    },
    Error {
        account_id: Option<u32>,
        message: String,
    },
}

/// Format a Unix timestamp for message list rows according to `date_style` and `clock_style`.
///
/// When both styles are `"system"` and `fallback` is non-empty, preserves `fallback` verbatim so
/// existing display formatting stays untouched.
pub fn format_message_date(
    date_ts: i64,
    fallback: &str,
    date_style: &str,
    clock_style: &str,
    now_ts: i64,
) -> String {
    if date_style == "system" && clock_style == "system" && !fallback.is_empty() {
        return fallback.to_string();
    }
    if date_ts <= 0 {
        return fallback.to_string();
    }
    let Some(msg_dt) = chrono::DateTime::from_timestamp(date_ts, 0) else {
        return fallback.to_string();
    };
    let Some(now_dt) = chrono::DateTime::from_timestamp(now_ts, 0) else {
        return fallback.to_string();
    };
    let local_msg = msg_dt.with_timezone(&chrono::Local);
    let local_now = now_dt.with_timezone(&chrono::Local);
    if local_msg.date_naive() == local_now.date_naive() {
        return format_clock_part(&local_msg, clock_style);
    }
    use chrono::Datelike;
    if local_msg.year() == local_now.year() {
        return format_month_day_part(&local_msg, date_style);
    }
    format_year_month_day_part(&local_msg, date_style)
}

/// Format a Unix timestamp with date and time for reading-pane headers.
pub fn format_message_datetime(
    date_ts: i64,
    fallback: &str,
    date_style: &str,
    clock_style: &str,
) -> String {
    if date_style == "system" && clock_style == "system" && !fallback.is_empty() {
        return fallback.to_string();
    }
    if date_ts <= 0 {
        return fallback.to_string();
    }
    let Some(msg_dt) = chrono::DateTime::from_timestamp(date_ts, 0) else {
        return fallback.to_string();
    };
    let local_msg = msg_dt.with_timezone(&chrono::Local);
    let date_part = format_year_month_day_part(&local_msg, date_style);
    let time_part = format_clock_part(&local_msg, clock_style);
    format!("{date_part} {time_part}")
}

fn format_clock_part(dt: &chrono::DateTime<chrono::Local>, clock_style: &str) -> String {
    use chrono::Timelike;
    match clock_style {
        "12h" => {
            let (is_pm, hour12) = dt.hour12();
            let suffix = if is_pm { "PM" } else { "AM" };
            format!("{hour12}:{:02} {suffix}", dt.minute())
        }
        _ => dt.format("%H:%M").to_string(),
    }
}

fn month_abbr(month: u32) -> &'static str {
    match month {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        _ => "Dec",
    }
}

fn format_month_day_part(dt: &chrono::DateTime<chrono::Local>, date_style: &str) -> String {
    use chrono::Datelike;
    let m = dt.month();
    let d = dt.day();
    match date_style {
        "month_first" => format!("{} {d}", month_abbr(m)),
        "day_first" => format!("{d} {}", month_abbr(m)),
        "year_first" => format!("{m:02}-{d:02}"),
        _ => format!("{m:02}-{d:02}"),
    }
}

fn format_year_month_day_part(dt: &chrono::DateTime<chrono::Local>, date_style: &str) -> String {
    use chrono::Datelike;
    let y = dt.year();
    let m = dt.month();
    let d = dt.day();
    match date_style {
        "month_first" => format!("{} {d}, {y}", month_abbr(m)),
        "day_first" => format!("{d} {} {y}", month_abbr(m)),
        "year_first" => format!("{y}-{m:02}-{d:02}"),
        _ => format!("{y}-{m:02}-{d:02}"),
    }
}

/// Detect whether subject or user-authored compose body mentions an attachment keyword
/// without any attached files. Quoted reply/forward blocks and `>` quote lines are ignored.
pub fn detect_attachment_reminder(subject: &str, body: &str, keywords_csv: &str) -> Option<String> {
    let mut authored = String::with_capacity(subject.len() + body.len() + 2);
    authored.push_str(subject);
    authored.push('\n');
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed == "------------------ 原始邮件 ------------------"
            || trimmed == "------------------ 转发邮件 ------------------"
        {
            break;
        }
        if trimmed.starts_with('>') {
            continue;
        }
        authored.push_str(line);
        authored.push('\n');
    }
    let hay = authored.to_lowercase();
    keywords_csv
        .split([',', '，'])
        .map(str::trim)
        .filter(|kw| !kw.is_empty())
        .find(|kw| hay.contains(&kw.to_lowercase()))
        .map(|kw| kw.to_string())
}

/// Format the attribution header placed above quoted messages in replies.
///
/// An empty template returns the built-in Chinese attribution block; custom templates
/// substitute `{name}`, `{address}`, `{date}`, `{time}`, and `{subject}`.
pub fn format_reply_header(
    template: &str,
    sender: &str,
    date_display: &str,
    subject: &str,
) -> String {
    let tpl = template.trim();
    if tpl.is_empty() {
        return format!(
            "------------------ 原始邮件 ------------------\n发件人：{sender}\n时间：{date_display}\n主题：{subject}"
        );
    }
    let (name, address) = if let (Some(open), Some(close)) = (sender.find('<'), sender.rfind('>')) {
        if close > open {
            (
                sender[..open].trim().to_string(),
                sender[open + 1..close].trim().to_string(),
            )
        } else {
            (sender.trim().to_string(), sender.trim().to_string())
        }
    } else {
        (sender.trim().to_string(), sender.trim().to_string())
    };
    let name_out = if name.is_empty() {
        address.clone()
    } else {
        name
    };
    let (date_part, time_part) = match date_display.trim().rsplit_once(' ') {
        Some((d, t)) if t.contains(':') => (d.trim(), t.trim()),
        _ => (date_display.trim(), date_display.trim()),
    };
    tpl.replace("{name}", &name_out)
        .replace("{address}", &address)
        .replace("{date}", date_part)
        .replace("{time}", time_part)
        .replace("{subject}", subject)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_group_label_classifies_today_yesterday_week_and_older() {
        use chrono::TimeZone;
        // 固定一个本地中午基准时间，避免跨午夜或夏令时边界抖动。
        let base_day = chrono::NaiveDate::from_ymd_opt(2026, 6, 15).unwrap();
        let noon = |days_ago: i64| -> i64 {
            let day = base_day - chrono::Duration::days(days_ago);
            let dt = day.and_hms_opt(12, 0, 0).unwrap();
            chrono::Local
                .from_local_datetime(&dt)
                .earliest()
                .unwrap()
                .timestamp()
        };
        let now_ts = noon(0);
        assert_eq!(date_group_label(now_ts, now_ts), "今天");
        assert_eq!(date_group_label(now_ts + 3600, now_ts), "今天");
        assert_eq!(date_group_label(noon(1), now_ts), "昨天");
        assert_eq!(date_group_label(noon(2), now_ts), "本周");
        assert_eq!(date_group_label(noon(6), now_ts), "本周");
        assert_eq!(date_group_label(noon(7), now_ts), "更早");
        assert_eq!(date_group_label(noon(30), now_ts), "更早");
        assert_eq!(date_group_label(0, now_ts), "更早");
    }

    #[test]
    fn quick_filter_set_and_sort_metadata() {
        let mut qf = QuickFilterSet::default();
        assert!(qf.is_empty());
        assert_eq!(qf.active_count(), 0);
        qf.unread = true;
        qf.has_attachment = true;
        qf.today = true;
        assert!(!qf.is_empty());
        assert_eq!(qf.active_count(), 3);
        assert_eq!(qf.active_labels(), vec!["仅未读", "含附件", "今天"]);
        qf.clear();
        assert!(qf.is_empty());

        assert_eq!(MessageSort::ALL.len(), 6);
        assert!(MessageSort::DateNewest.uses_date_groups());
        assert!(MessageSort::DateOldest.uses_date_groups());
        assert!(!MessageSort::SenderAsc.uses_date_groups());
        assert!(!MessageSort::SubjectAsc.uses_date_groups());
        assert!(!MessageSort::UnreadFirst.uses_date_groups());
        assert!(!MessageSort::StarredFirst.uses_date_groups());
    }

    #[test]
    fn date_formatting_respects_system_fallback_and_explicit_styles() {
        use chrono::TimeZone;
        let dt = chrono::Local
            .with_ymd_and_hms(2025, 8, 23, 17, 40, 0)
            .single()
            .unwrap();
        let ts = dt.timestamp();
        let now_same_day = chrono::Local
            .with_ymd_and_hms(2025, 8, 23, 20, 0, 0)
            .single()
            .unwrap()
            .timestamp();
        let now_next_year = chrono::Local
            .with_ymd_and_hms(2026, 1, 10, 9, 0, 0)
            .single()
            .unwrap()
            .timestamp();

        assert_eq!(
            format_message_date(ts, "原始日期", "system", "system", now_next_year),
            "原始日期"
        );
        assert_eq!(
            format_message_date(ts, "fallback", "system", "12h", now_same_day),
            "5:40 PM"
        );
        assert_eq!(
            format_message_date(ts, "fallback", "system", "24h", now_same_day),
            "17:40"
        );
        assert_eq!(
            format_message_date(ts, "fallback", "month_first", "24h", now_next_year),
            "Aug 23, 2025"
        );
        assert_eq!(
            format_message_date(ts, "fallback", "day_first", "24h", now_next_year),
            "23 Aug 2025"
        );
        assert_eq!(
            format_message_date(ts, "fallback", "year_first", "24h", now_next_year),
            "2025-08-23"
        );
        assert_eq!(
            format_message_datetime(ts, "fallback", "day_first", "12h"),
            "23 Aug 2025 5:40 PM"
        );
    }

    #[test]
    fn attachment_reminder_detects_authored_text_and_ignores_quotes() {
        let kw = "附件, attachment, attached";
        assert_eq!(
            detect_attachment_reminder("汇报", "请查收附件。", kw).as_deref(),
            Some("附件")
        );
        assert_eq!(
            detect_attachment_reminder("See attached report", "Hello", kw).as_deref(),
            Some("attached")
        );
        let quoted_only = "好的，收到。\n\n> See attachment below\n------------------ 原始邮件 ------------------\n请查收附件";
        assert_eq!(
            detect_attachment_reminder("Re: 汇报", quoted_only, kw),
            None
        );
    }

    #[test]
    fn reply_header_formats_default_and_custom_templates() {
        let def = format_reply_header("", "Alice <a@example.com>", "2026-08-23 14:05", "Hi");
        assert!(def.contains("------------------ 原始邮件 ------------------"));
        assert!(def.contains("发件人：Alice <a@example.com>"));

        let custom = format_reply_header(
            "On {date} at {time}, {name} <{address}> wrote:",
            "Alice <a@example.com>",
            "2026-08-23 14:05",
            "Hi",
        );
        assert_eq!(
            custom,
            "On 2026-08-23 at 14:05, Alice <a@example.com> wrote:"
        );
    }
}
