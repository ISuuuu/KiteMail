//! Core mail domain models, UI-worker command/event protocol, local SQLite cache,
//! Windows Credential Manager secrets, and IMAP/SMTP worker runtime.

pub mod cache;
pub mod config;
pub mod demo;
pub mod graph_client;
pub mod imap_client;
pub mod oauth;
pub mod secrets;
pub mod smtp_client;
pub mod worker;

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
    pub flagged: bool,
    pub attachments: Vec<AttachmentMeta>,
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

/// Server/cache-side filter applied to message list queries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MessageFilter {
    #[default]
    All,
    Unread,
    Starred,
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
        offset: usize,
        limit: usize,
    },
    LoadMessagesPage {
        folder_id: u32,
        query: String,
        filter: MessageFilter,
        offset: usize,
        limit: usize,
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
    SaveAttachment {
        attachment_id: u32,
        dest_path: std::path::PathBuf,
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
    Status {
        account_id: Option<u32>,
        text: String,
    },
    Error {
        account_id: Option<u32>,
        message: String,
    },
}
