//! Core mail domain models, UI-worker command/event protocol, local SQLite cache,
//! Windows Credential Manager secrets, and IMAP/SMTP worker runtime.

pub mod cache;
pub mod config;
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

use crate::cache::{CachedAttachment, KnownContact};
use crate::config::AccountConfig;

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
    pub date_display: String,
    pub body_text: String,
    #[serde(default)]
    pub body_html: Option<String>,
    pub body_loaded: bool,
    pub flagged: bool,
    pub attachments: Vec<AttachmentMeta>,
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
    pub date_display: String,
    pub body_text: String,
    #[serde(default)]
    pub body_html: Option<String>,
    pub flagged: bool,
    pub attachments: Vec<AttachmentMeta>,
    /// Chronological messages belonging to the same conversation thread (empty or 1 item for standalone mails).
    #[serde(default)]
    pub thread_messages: Vec<ThreadMessage>,
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
    },
    RetryOutboxItem {
        account_id: u32,
        outbox_id: u32,
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
    /// Clear all cached message bodies (headers and accounts are kept).
    ClearBodyCaches,
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
    FoldersLoaded {
        account_id: u32,
        folders: Vec<FolderSummary>,
    },
    MessagesPageLoaded(MailPage),
    MessageBodyLoaded(MessageDetail),
    DraftsUpdated {
        drafts: Vec<LocalDraft>,
    },
    OutboxUpdated {
        account_id: u32,
        items: Vec<OutboxItem>,
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
    MessageArchived {
        account_id: u32,
        message_id: u32,
    },
    /// A message was moved to the account's trash folder (or permanently
    /// deleted) on the server and removed from the local cache.
    MessageDeleted {
        account_id: u32,
        message_id: u32,
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
    },
    /// A message was moved to the account's spam/junk folder on the server and
    /// removed from the old folder's local cache.
    MessageMarkedSpam {
        account_id: u32,
        message_id: u32,
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
    /// Result of fetching an older batch of messages from the remote server.
    OlderMessagesLoaded {
        folder_id: u32,
        fetched_count: usize,
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
}
