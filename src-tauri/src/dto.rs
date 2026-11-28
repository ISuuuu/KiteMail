//! Frozen IPC DTOs and event payloads for the KiteMail Tauri main window.

use kitemail_mail_core::cache::TagItem;
use kitemail_mail_core::{
    AccountSummary, AttachmentMeta, FolderSummary, LocalDraft, MailTemplate, MessageSort,
    MessageSummary, OutboxItem, OutboxStatus, SearchScope, ThreadMessage,
};
use serde::{Deserialize, Deserializer, Serialize};

pub const MAIN_WINDOW_LABEL: &str = "main";
pub const EVENT_MAIL_CHANGED: &str = "main://mail-changed";
pub const EVENT_SYNC_STATUS: &str = "main://sync-status";
pub const EVENT_READER_STATUS: &str = "main://reader-status";
pub const EVENT_LOCK_STATE: &str = "main://lock-state";
pub const EVENT_OUTBOX_UPDATED: &str = "main://outbox-updated";
pub const EVENT_DRAFT_SAVED: &str = "main://draft-saved";
pub const EVENT_ACCOUNT_EVENT: &str = "main://account-event";
pub const EVENT_THREAD_CARD_ACTION: &str = "main://thread-card-action";
pub const EVENT_READER_FIND_RESULT: &str = "main://reader-find-result";

/// Structured IPC error returned by all main-window Tauri commands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IpcError {
    pub code: String,
    pub message: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}

impl IpcError {
    pub fn invalid_input(message: impl Into<String>, request_id: Option<String>) -> Self {
        Self {
            code: "INVALID_INPUT".to_string(),
            message: message.into(),
            retryable: false,
            request_id,
        }
    }

    pub fn locked(request_id: Option<String>) -> Self {
        Self {
            code: "LOCKED".to_string(),
            message: "应用已锁定，请先解锁后再访问邮件内容".to_string(),
            retryable: false,
            request_id,
        }
    }

    pub fn message_not_found(message_id: u32, request_id: Option<String>) -> Self {
        Self {
            code: "MESSAGE_NOT_FOUND".to_string(),
            message: format!("未找到邮件 #{message_id}"),
            retryable: false,
            request_id,
        }
    }

    pub fn unavailable(message: impl Into<String>, request_id: Option<String>) -> Self {
        Self {
            code: "UNAVAILABLE".to_string(),
            message: message.into(),
            retryable: true,
            request_id,
        }
    }

    #[allow(dead_code)]
    pub fn render_failed(message: impl Into<String>, request_id: Option<String>) -> Self {
        Self {
            code: "RENDER_FAILED".to_string(),
            message: message.into(),
            retryable: true,
            request_id,
        }
    }

    pub fn internal(message: impl Into<String>, request_id: Option<String>) -> Self {
        Self {
            code: "INTERNAL_ERROR".to_string(),
            message: message.into(),
            retryable: true,
            request_id,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AppMode {
    Demo,
    Live,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountSummaryDto {
    pub id: u32,
    pub display_name: String,
    pub email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
}

impl From<&AccountSummary> for AccountSummaryDto {
    fn from(value: &AccountSummary) -> Self {
        Self {
            id: value.id,
            display_name: value.display_name.clone(),
            email: value.email.clone(),
            label: None,
            color: None,
            emoji: None,
        }
    }
}

impl From<&kitemail_mail_core::config::AccountConfig> for AccountSummaryDto {
    fn from(cfg: &kitemail_mail_core::config::AccountConfig) -> Self {
        let summary = AccountSummary::from(cfg);
        Self {
            id: summary.id,
            display_name: summary.display_name,
            email: summary.email,
            label: cfg.label.clone(),
            color: cfg.color.clone(),
            emoji: cfg.emoji.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TagItemDto {
    pub id: u32,
    pub name: String,
    pub color: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortcut: Option<u8>,
}

impl From<&TagItem> for TagItemDto {
    fn from(value: &TagItem) -> Self {
        Self {
            id: value.id,
            name: value.name.clone(),
            color: value.color.clone(),
            shortcut: value.shortcut,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailTemplateDto {
    pub id: u32,
    pub name: String,
    pub to_addr: String,
    pub cc_addr: String,
    pub bcc_addr: String,
    pub subject: String,
    pub body_text: String,
    pub body_format: String,
    pub attachment_paths: Vec<String>,
    pub created_at: i64,
}

impl From<&MailTemplate> for MailTemplateDto {
    fn from(value: &MailTemplate) -> Self {
        Self {
            id: value.id,
            name: value.name.clone(),
            to_addr: value.to_addr.clone(),
            cc_addr: value.cc_addr.clone(),
            bcc_addr: value.bcc_addr.clone(),
            subject: value.subject.clone(),
            body_text: value.body_text.clone(),
            body_format: value.body_format.clone(),
            attachment_paths: value.attachment_paths.clone(),
            created_at: value.created_at,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderSummaryDto {
    pub id: u32,
    pub account_id: u32,
    pub name: String,
    pub path: String,
    pub unread_count: u32,
    pub total_count: usize,
    pub has_more_remote: bool,
}

impl From<&FolderSummary> for FolderSummaryDto {
    fn from(value: &FolderSummary) -> Self {
        Self {
            id: value.id,
            account_id: value.account_id,
            name: value.name.clone(),
            path: value.path.clone(),
            unread_count: value.unread_count,
            total_count: value.total_count,
            has_more_remote: value.has_more_remote,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum MainViewDto {
    UnifiedInbox,
    Unread,
    Starred,
    Sent,
    Drafts,
    Archive,
    Junk,
    Trash,
    Snoozed,
    Filters,
    Tags,
    AccountFolder { account_id: u32, folder_id: u32 },
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReaderPositionDto {
    #[default]
    Right,
    Below,
    Off,
}

impl ReaderPositionDto {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Right => "right",
            Self::Below => "below",
            Self::Off => "off",
        }
    }

    pub fn from_pref_str(value: &str) -> Self {
        match value.trim() {
            "below" => Self::Below,
            "off" => Self::Off,
            _ => Self::Right,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MainPreferencesDto {
    pub theme_id: String,
    pub dark_mode: Option<bool>,
    pub reader_position: ReaderPositionDto,
    pub sidebar_width: f32,
    pub list_width: f32,
    pub list_below_height: f32,
    pub sidebar_collapsed: bool,
    pub compact_list: bool,
    pub show_avatars: bool,
    pub list_group_by_date: bool,
    pub reader_zoom_percent: u32,
}

/// Deserialize a JSON field where omitted => `None`, `null` => `Some(None)`, and value => `Some(Some(v))`.
fn deserialize_double_option<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Deserialize::deserialize(deserializer).map(Some)
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MainPreferencesPatch {
    pub theme_id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub dark_mode: Option<Option<bool>>,
    pub reader_position: Option<ReaderPositionDto>,
    pub sidebar_width: Option<f32>,
    pub list_width: Option<f32>,
    pub list_below_height: Option<f32>,
    pub sidebar_collapsed: Option<bool>,
    pub compact_list: Option<bool>,
    pub show_avatars: Option<bool>,
    pub list_group_by_date: Option<bool>,
    pub reader_zoom_percent: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatusDto {
    pub syncing: bool,
    pub message: Option<String>,
    pub last_sync_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MainBootstrapDto {
    pub mode: AppMode,
    pub locked: bool,
    pub has_passcode: bool,
    pub cache_ready: bool,
    pub state_version: u64,
    pub accounts: Vec<AccountSummaryDto>,
    pub folders: Vec<FolderSummaryDto>,
    pub current_view: MainViewDto,
    pub preferences: MainPreferencesDto,
    pub sync_status: SyncStatusDto,
    #[serde(default)]
    pub tags: Vec<TagItemDto>,
    #[serde(default)]
    pub templates: Vec<MailTemplateDto>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchScopeDto {
    #[default]
    ThisFolder,
    AllFolders,
}

impl From<SearchScopeDto> for SearchScope {
    fn from(value: SearchScopeDto) -> Self {
        match value {
            SearchScopeDto::ThisFolder => SearchScope::ThisFolder,
            SearchScopeDto::AllFolders => SearchScope::AllFolders,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MessageSortDto {
    #[default]
    DateNewest,
    DateOldest,
    SenderAsc,
    SenderDesc,
    SubjectAsc,
    SubjectDesc,
    UnreadFirst,
    StarredFirst,
    AttachmentFirst,
    ImportanceFirst,
}

impl From<MessageSortDto> for MessageSort {
    fn from(value: MessageSortDto) -> Self {
        match value {
            MessageSortDto::DateNewest => MessageSort::DateNewest,
            MessageSortDto::DateOldest => MessageSort::DateOldest,
            MessageSortDto::SenderAsc => MessageSort::SenderAsc,
            MessageSortDto::SenderDesc => MessageSort::SenderDesc,
            MessageSortDto::SubjectAsc => MessageSort::SubjectAsc,
            MessageSortDto::SubjectDesc => MessageSort::SubjectDesc,
            MessageSortDto::UnreadFirst => MessageSort::UnreadFirst,
            MessageSortDto::StarredFirst => MessageSort::StarredFirst,
            MessageSortDto::AttachmentFirst => MessageSort::AttachmentFirst,
            MessageSortDto::ImportanceFirst => MessageSort::ImportanceFirst,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MessageFilterDto {
    pub unread: Option<bool>,
    pub starred: Option<bool>,
    pub has_attachment: Option<bool>,
    pub today: Option<bool>,
    pub to_me: Option<bool>,
    pub from_contacts: Option<bool>,
    pub high_importance: Option<bool>,
    pub tagged: Option<bool>,
    pub date_from_ts: Option<i64>,
    pub date_to_ts: Option<i64>,
    pub account_id: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListThreadSummariesInput {
    pub account_id: u32,
    pub message_id: u32,
    #[serde(default)]
    pub subject: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListThreadSummariesOutput {
    pub account_id: u32,
    pub message_id: u32,
    pub items: Vec<MessageSummaryDto>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListMessagesInput {
    pub request_id: String,
    pub view: MainViewDto,
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub scope: SearchScopeDto,
    #[serde(default)]
    pub sort: MessageSortDto,
    #[serde(default)]
    pub filter: MessageFilterDto,
    pub offset: usize,
    pub limit: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageSummaryDto {
    pub id: u32,
    pub account_id: u32,
    pub folder_id: u32,
    pub folder_path: String,
    pub sender: String,
    pub recipients: String,
    pub subject: String,
    pub preview: String,
    pub date_ts: i64,
    pub date_display: String,
    pub unread: bool,
    pub flagged: bool,
    pub has_attachment: bool,
    pub thread_count: u32,
    pub importance: i8,
    pub tags: Vec<String>,
    pub answered: bool,
    pub forwarded: bool,
}

impl From<&MessageSummary> for MessageSummaryDto {
    fn from(value: &MessageSummary) -> Self {
        Self {
            id: value.id,
            account_id: value.account_id,
            folder_id: value.folder_id,
            folder_path: value.folder_path.clone(),
            sender: value.sender.clone(),
            recipients: value.recipients.clone(),
            subject: value.subject.clone(),
            preview: value.preview.clone(),
            date_ts: value.date_ts,
            date_display: value.date_display.clone(),
            unread: value.unread,
            flagged: value.flagged,
            has_attachment: value.has_attachment,
            thread_count: value.thread_count,
            importance: value.importance,
            tags: value.tags.clone(),
            answered: value.answered,
            forwarded: value.forwarded,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListMessagesOutput {
    pub request_id: String,
    pub state_version: u64,
    pub offset: usize,
    pub total: usize,
    pub has_more: bool,
    pub items: Vec<MessageSummaryDto>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenMessageInput {
    pub request_id: String,
    pub account_id: u32,
    pub message_id: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BodyStatusDto {
    Loading,
    Ready,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentMetaDto {
    pub id: u32,
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: usize,
    pub downloaded: bool,
}

impl From<&AttachmentMeta> for AttachmentMetaDto {
    fn from(value: &AttachmentMeta) -> Self {
        Self {
            id: value.id,
            filename: value.filename.clone(),
            mime_type: value.mime_type.clone(),
            size_bytes: value.size_bytes,
            downloaded: value.downloaded,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadMessageDto {
    pub id: u32,
    pub account_id: u32,
    pub rfc_message_id: String,
    pub subject: String,
    pub sender: String,
    pub recipients: String,
    pub cc: String,
    pub date_ts: i64,
    pub date_display: String,
    pub preview: String,
    pub body_text: String,
    pub plain_text: String,
    pub has_html: bool,
    pub html_body: Option<String>,
    pub body_loaded: bool,
    pub unread: bool,
    pub flagged: bool,
    pub attachments: Vec<AttachmentMetaDto>,
    pub uid: u32,
    pub folder_path: String,
}

impl ThreadMessageDto {
    pub fn from_core(account_id: u32, value: &ThreadMessage) -> Self {
        let preview = value
            .body_text
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or("")
            .chars()
            .take(160)
            .collect();
        Self {
            id: value.id,
            account_id,
            rfc_message_id: value.rfc_message_id.clone(),
            subject: value.subject.clone(),
            sender: value.sender.clone(),
            recipients: value.recipients.clone(),
            cc: value.cc.clone(),
            date_ts: value.date_ts,
            date_display: value.date_display.clone(),
            preview,
            body_text: value.body_text.clone(),
            plain_text: value.body_text.clone(),
            has_html: value.body_html.is_some(),
            html_body: value.body_html.clone(),
            body_loaded: value.body_loaded,
            unread: value.unread,
            flagged: value.flagged,
            attachments: value
                .attachments
                .iter()
                .map(AttachmentMetaDto::from)
                .collect(),
            uid: value.uid,
            folder_path: value.folder_path.clone(),
        }
    }
}

impl From<&ThreadMessage> for ThreadMessageDto {
    fn from(value: &ThreadMessage) -> Self {
        Self::from_core(0, value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenMessageOutput {
    pub request_id: String,
    pub reader_session_id: String,
    pub account_id: u32,
    pub message_id: u32,
    pub subject: String,
    pub sender: String,
    pub recipients: String,
    pub cc: String,
    pub date_display: String,
    pub body_status: BodyStatusDto,
    pub plain_text: String,
    pub has_html: bool,
    pub html_body: Option<String>,
    pub flagged: bool,
    pub attachments: Vec<AttachmentMetaDto>,
    pub thread_messages: Vec<ThreadMessageDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote_images_blocked: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rfc_message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder_path: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RectDto {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetReaderLayoutInput {
    pub reader_session_id: String,
    pub layout_revision: u64,
    pub content_rect: RectDto,
    pub clip_rect: RectDto,
    pub visible: bool,
    #[serde(default)]
    pub device_pixel_ratio: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetReaderLayoutOutput {
    pub applied: bool,
    pub reader_session_id: String,
    pub layout_revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshMailInput {
    pub view: MainViewDto,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshMailOutput {
    pub accepted: bool,
    pub state_version: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailChangedEvent {
    pub state_version: u64,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatusEvent {
    pub state_version: u64,
    pub sync_status: SyncStatusDto,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnlockAppInput {
    pub passcode: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnlockAppOutput {
    pub unlocked: bool,
    pub locked: bool,
    pub error: Option<String>,
    pub retry_after_secs: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockAppOutput {
    pub locked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetMessageSeenInput {
    pub account_id: u32,
    pub message_id: u32,
    pub seen: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetMessageSeenOutput {
    pub account_id: u32,
    pub message_id: u32,
    pub unread: bool,
    pub state_version: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetMessageFlaggedInput {
    pub account_id: u32,
    pub message_id: u32,
    pub flagged: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetMessageFlaggedOutput {
    pub account_id: u32,
    pub message_id: u32,
    pub flagged: bool,
    pub state_version: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AllowRemoteImagesInput {
    pub account_id: u32,
    pub message_id: u32,
    pub trust_sender: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AllowRemoteImagesOutput {
    pub allowed: bool,
    pub sender_trusted: bool,
    pub state_version: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadOlderMessagesInput {
    pub view: MainViewDto,
    pub limit: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadOlderMessagesOutput {
    pub accepted: bool,
    pub has_more_remote: bool,
    pub state_version: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageActionInput {
    pub account_id: u32,
    pub message_id: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveMessageInput {
    pub account_id: u32,
    pub message_id: u32,
    pub target_folder_path: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageMoveOutput {
    pub account_id: u32,
    pub message_id: u32,
    pub rfc_message_id: String,
    pub original_folder_path: String,
    pub target_folder_path: String,
    pub state_version: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoMessageMoveInput {
    pub account_id: u32,
    pub message_id: u32,
    pub rfc_message_id: String,
    pub current_folder_path: String,
    pub original_folder_path: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoMessageMoveOutput {
    pub accepted: bool,
    pub state_version: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderStatusEvent {
    pub reader_session_id: String,
    pub account_id: u32,
    pub message_id: u32,
    pub body_status: BodyStatusDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plain_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_html: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html_body: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockStateEvent {
    pub state_version: u64,
    pub locked: bool,
    pub has_passcode: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locked_reason: Option<String>,
}

/// Deserialize either a JSON string or integer into a `String` ID.
pub fn deserialize_id_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    struct IdVisitor;
    impl<'de> serde::de::Visitor<'de> for IdVisitor {
        type Value = String;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a string or non-negative integer ID")
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(v.to_string())
        }

        fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(v)
        }

        fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(v.to_string())
        }

        fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            if v < 0 {
                return Err(E::custom("ID cannot be negative"));
            }
            Ok(v.to_string())
        }
    }
    deserializer.deserialize_any(IdVisitor)
}

/// Deserialize `null`, omitted, string, or integer into `Option<String>`.
pub fn deserialize_opt_id_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    struct OptIdVisitor;
    impl<'de> serde::de::Visitor<'de> for OptIdVisitor {
        type Value = Option<String>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("null, a string, or non-negative integer ID")
        }

        fn visit_none<E>(self) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(None)
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(None)
        }

        fn visit_some<D2>(self, deserializer: D2) -> Result<Self::Value, D2::Error>
        where
            D2: Deserializer<'de>,
        {
            deserialize_id_string(deserializer).map(Some)
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(Some(v.to_string()))
        }

        fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(Some(v))
        }

        fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(Some(v.to_string()))
        }

        fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            if v < 0 {
                return Err(E::custom("ID cannot be negative"));
            }
            Ok(Some(v.to_string()))
        }
    }
    deserializer.deserialize_option(OptIdVisitor)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BodyFormatDto {
    #[default]
    Plain,
    Markdown,
    Html,
}

impl BodyFormatDto {
    pub fn as_core_str(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Markdown => "markdown",
            Self::Html => "html",
        }
    }

    pub fn from_core_str(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "markdown" | "md" => Self::Markdown,
            "html" | "rich" => Self::Html,
            _ => Self::Plain,
        }
    }
}

fn non_empty_opt(s: &str) -> Option<String> {
    let trimmed = s.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalDraftDto {
    pub id: String,
    pub account_id: String,
    pub to_addr: String,
    pub cc_addr: String,
    pub bcc_addr: String,
    pub subject: String,
    pub body_text: String,
    pub in_reply_to: Option<String>,
    pub from_addr: Option<String>,
    pub reply_to: Option<String>,
    pub body_format: BodyFormatDto,
    pub attachment_paths: Vec<String>,
    pub updated_at: i64,
}

impl From<&LocalDraft> for LocalDraftDto {
    fn from(value: &LocalDraft) -> Self {
        Self {
            id: value.id.to_string(),
            account_id: value.account_id.to_string(),
            to_addr: value.to_addr.clone(),
            cc_addr: value.cc_addr.clone(),
            bcc_addr: value.bcc_addr.clone(),
            subject: value.subject.clone(),
            body_text: value.body_text.clone(),
            in_reply_to: non_empty_opt(&value.in_reply_to),
            from_addr: non_empty_opt(&value.from_addr),
            reply_to: non_empty_opt(&value.reply_to),
            body_format: BodyFormatDto::from_core_str(&value.body_format),
            attachment_paths: value.attachment_paths.clone(),
            updated_at: value.updated_at,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListDraftsOutput {
    pub drafts: Vec<LocalDraftDto>,
    pub state_version: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveDraftInput {
    #[serde(default, deserialize_with = "deserialize_opt_id_string")]
    pub id: Option<String>,
    #[serde(deserialize_with = "deserialize_id_string")]
    pub account_id: String,
    #[serde(default)]
    pub to_addr: String,
    #[serde(default)]
    pub cc_addr: String,
    #[serde(default)]
    pub bcc_addr: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub body_text: String,
    #[serde(default)]
    pub in_reply_to: Option<String>,
    #[serde(default)]
    pub from_addr: Option<String>,
    #[serde(default)]
    pub reply_to: Option<String>,
    #[serde(default)]
    pub body_format: Option<BodyFormatDto>,
    #[serde(default)]
    pub attachment_paths: Vec<String>,
    #[serde(default)]
    pub silent: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveDraftOutput {
    pub draft: LocalDraftDto,
    pub state_version: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteDraftInput {
    #[serde(deserialize_with = "deserialize_id_string")]
    pub draft_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteDraftOutput {
    pub deleted: bool,
    pub state_version: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendMailInput {
    #[serde(deserialize_with = "deserialize_id_string")]
    pub account_id: String,
    pub to: String,
    #[serde(default)]
    pub cc: String,
    #[serde(default)]
    pub bcc: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub in_reply_to: Option<String>,
    #[serde(default)]
    pub from_addr: Option<String>,
    #[serde(default)]
    pub reply_to: Option<String>,
    #[serde(default)]
    pub body_format: Option<BodyFormatDto>,
    #[serde(default)]
    pub body_html: Option<String>,
    #[serde(default)]
    pub attachment_paths: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_opt_id_string")]
    pub local_draft_id: Option<String>,
    #[serde(default)]
    pub undo_delay_secs: Option<u32>,
    #[serde(default)]
    pub scheduled_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendMailOutput {
    pub accepted: bool,
    pub undo_token: String,
    pub outbox_id: String,
    pub undo_window_secs: u32,
    pub scheduled_at: Option<i64>,
    pub state_version: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoSendMailInput {
    #[serde(deserialize_with = "deserialize_id_string")]
    pub account_id: String,
    #[serde(deserialize_with = "deserialize_id_string")]
    pub outbox_id: String,
    #[serde(deserialize_with = "deserialize_id_string")]
    pub undo_token: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoSendMailOutput {
    pub undone: bool,
    pub message: String,
    pub state_version: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OutboxStatusDto {
    Queued,
    Sending,
    Uncertain,
    Failed,
}

impl From<OutboxStatus> for OutboxStatusDto {
    fn from(value: OutboxStatus) -> Self {
        match value {
            OutboxStatus::Queued => Self::Queued,
            OutboxStatus::Sending => Self::Sending,
            OutboxStatus::Uncertain => Self::Uncertain,
            OutboxStatus::Failed => Self::Failed,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutboxItemDto {
    pub id: String,
    pub account_id: String,
    pub to_addr: String,
    pub subject: String,
    pub status: OutboxStatusDto,
    pub last_error: Option<String>,
    pub send_at: i64,
}

impl From<&OutboxItem> for OutboxItemDto {
    fn from(value: &OutboxItem) -> Self {
        Self {
            id: value.id.to_string(),
            account_id: value.account_id.to_string(),
            to_addr: value.to_addr.clone(),
            subject: value.subject.clone(),
            status: OutboxStatusDto::from(value.status),
            last_error: non_empty_opt(&value.last_error),
            send_at: value.send_at.unwrap_or(0),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListOutboxOutput {
    pub items: Vec<OutboxItemDto>,
    pub state_version: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutboxItemActionInput {
    #[serde(deserialize_with = "deserialize_id_string")]
    pub account_id: String,
    #[serde(deserialize_with = "deserialize_id_string")]
    pub outbox_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutboxItemActionOutput {
    pub accepted: bool,
    pub state_version: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OutboxEventKindDto {
    Enqueued,
    Sent,
    Failed,
    Undone,
    Updated,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutboxUpdatedEvent {
    pub state_version: u64,
    pub items: Vec<OutboxItemDto>,
    pub event_kind: OutboxEventKindDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undo_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outbox_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftSavedEvent {
    pub state_version: u64,
    pub draft_id: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AuthBackendDto {
    #[default]
    Password,
    #[serde(rename = "microsoftGraphOAuth2")]
    MicrosoftGraphOAuth2,
}

impl From<kitemail_mail_core::config::AuthBackend> for AuthBackendDto {
    fn from(value: kitemail_mail_core::config::AuthBackend) -> Self {
        match value {
            kitemail_mail_core::config::AuthBackend::Password => Self::Password,
            kitemail_mail_core::config::AuthBackend::MicrosoftGraphOAuth2 => {
                Self::MicrosoftGraphOAuth2
            }
        }
    }
}

impl From<AuthBackendDto> for kitemail_mail_core::config::AuthBackend {
    fn from(value: AuthBackendDto) -> Self {
        match value {
            AuthBackendDto::Password => Self::Password,
            AuthBackendDto::MicrosoftGraphOAuth2 => Self::MicrosoftGraphOAuth2,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SecurityModeDto {
    #[default]
    Tls,
    StartTls,
}

impl From<kitemail_mail_core::config::SecurityMode> for SecurityModeDto {
    fn from(value: kitemail_mail_core::config::SecurityMode) -> Self {
        match value {
            kitemail_mail_core::config::SecurityMode::Tls => Self::Tls,
            kitemail_mail_core::config::SecurityMode::StartTls => Self::StartTls,
        }
    }
}

impl From<SecurityModeDto> for kitemail_mail_core::config::SecurityMode {
    fn from(value: SecurityModeDto) -> Self {
        match value {
            SecurityModeDto::Tls => Self::Tls,
            SecurityModeDto::StartTls => Self::StartTls,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountConfigDto {
    pub id: String,
    pub auth_backend: AuthBackendDto,
    pub oauth_client_id: Option<String>,
    pub display_name: String,
    pub email: String,
    pub label: String,
    pub enabled: bool,
    pub in_unified: bool,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_security: SecurityModeDto,
    pub imap_username: String,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_security: SecurityModeDto,
    pub smtp_username: String,
    pub smtp_separate_credentials: bool,
    pub signature: String,
    pub has_password: bool,
}

impl AccountConfigDto {
    pub fn from_config(
        cfg: &kitemail_mail_core::config::AccountConfig,
        has_password: bool,
    ) -> Self {
        Self {
            id: cfg.id.to_string(),
            auth_backend: AuthBackendDto::from(cfg.auth_backend),
            oauth_client_id: non_empty_opt(&cfg.oauth_client_id),
            display_name: cfg.display_name.clone(),
            email: cfg.email.clone(),
            label: cfg.label.clone().unwrap_or_default(),
            enabled: cfg.enabled,
            in_unified: cfg.in_unified,
            imap_host: cfg.imap_host.clone(),
            imap_port: cfg.imap_port,
            imap_security: SecurityModeDto::from(cfg.imap_security),
            imap_username: cfg.imap_username.clone(),
            smtp_host: cfg.smtp_host.clone(),
            smtp_port: cfg.smtp_port,
            smtp_security: SecurityModeDto::from(cfg.smtp_security),
            smtp_username: cfg.smtp_username.clone(),
            smtp_separate_credentials: cfg.smtp_separate_credentials,
            signature: cfg.signature.clone(),
            has_password,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderPresetDto {
    pub id: String,
    pub name: String,
    pub domains: Vec<String>,
    pub auth_backend: AuthBackendDto,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_security: SecurityModeDto,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_security: SecurityModeDto,
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListAccountsOutput {
    pub accounts: Vec<AccountConfigDto>,
    pub presets: Vec<ProviderPresetDto>,
    pub state_version: u64,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveAccountInput {
    #[serde(default, deserialize_with = "deserialize_opt_id_string")]
    pub id: Option<String>,
    #[serde(default)]
    pub auth_backend: AuthBackendDto,
    #[serde(default)]
    pub oauth_client_id: Option<String>,
    #[serde(default)]
    pub display_name: String,
    pub email: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub in_unified: Option<bool>,
    #[serde(default)]
    pub imap_host: String,
    #[serde(default)]
    pub imap_port: u16,
    #[serde(default)]
    pub imap_security: SecurityModeDto,
    #[serde(default)]
    pub imap_username: String,
    #[serde(default)]
    pub smtp_host: String,
    #[serde(default)]
    pub smtp_port: u16,
    #[serde(default)]
    pub smtp_security: SecurityModeDto,
    #[serde(default)]
    pub smtp_username: String,
    #[serde(default)]
    pub smtp_separate_credentials: bool,
    #[serde(default)]
    pub signature: String,
    #[serde(default)]
    pub imap_password: Option<String>,
    #[serde(default)]
    pub smtp_password: Option<String>,
}

impl std::fmt::Debug for SaveAccountInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SaveAccountInput")
            .field("id", &self.id)
            .field("auth_backend", &self.auth_backend)
            .field("oauth_client_id", &self.oauth_client_id)
            .field("display_name", &self.display_name)
            .field("email", &self.email)
            .field("label", &self.label)
            .field("enabled", &self.enabled)
            .field("in_unified", &self.in_unified)
            .field("imap_host", &self.imap_host)
            .field("imap_port", &self.imap_port)
            .field("imap_security", &self.imap_security)
            .field("imap_username", &self.imap_username)
            .field("smtp_host", &self.smtp_host)
            .field("smtp_port", &self.smtp_port)
            .field("smtp_security", &self.smtp_security)
            .field("smtp_username", &self.smtp_username)
            .field("smtp_separate_credentials", &self.smtp_separate_credentials)
            .field("signature", &self.signature)
            .field(
                "imap_password",
                &self.imap_password.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "smtp_password",
                &self.smtp_password.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerifyAccountOutput {
    pub ok: bool,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveAccountOutput {
    pub account: AccountConfigDto,
    pub state_version: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteAccountInput {
    #[serde(deserialize_with = "deserialize_id_string")]
    pub account_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteAccountOutput {
    pub deleted: bool,
    pub state_version: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetAccountEnabledInput {
    #[serde(deserialize_with = "deserialize_id_string")]
    pub account_id: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetAccountEnabledOutput {
    pub account_id: String,
    pub enabled: bool,
    pub state_version: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartMicrosoftOAuthInput {
    #[serde(default)]
    pub custom_client_id: Option<String>,
    #[serde(default)]
    pub login_hint_email: Option<String>,
    #[serde(default, deserialize_with = "deserialize_opt_id_string")]
    pub editing_account_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartMicrosoftOAuthOutput {
    pub started: bool,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AccountEventKindDto {
    Verified,
    Saved,
    Deleted,
    Updated,
    OauthReauthorized,
    OauthRebindRejected,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountEvent {
    pub state_version: u64,
    pub kind: AccountEventKindDto,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    pub message: String,
}

// ---- Wave 3: Attachments, Contacts, and Full Settings DTOs ----

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ListCachedAttachmentsInput {
    pub query: Option<String>,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedAttachmentDto {
    pub id: u32,
    pub message_id: u32,
    pub account_id: u32,
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: usize,
    pub sender: String,
    pub subject: String,
    pub date_ts: i64,
    pub date_display: String,
    pub downloaded: bool,
    pub is_image: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListCachedAttachmentsOutput {
    pub items: Vec<CachedAttachmentDto>,
    pub total: usize,
    pub has_more: bool,
    pub state_version: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchAttachmentPreviewInput {
    pub attachment_id: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchAttachmentPreviewOutput {
    pub attachment_id: u32,
    pub filename: String,
    pub mime_type: String,
    pub data_url: String,
    pub size_bytes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveAttachmentInput {
    pub attachment_id: u32,
    #[serde(default)]
    pub dest_path: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveAttachmentOutput {
    pub saved: bool,
    pub saved_path: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveAllAttachmentsInput {
    pub attachment_ids: Vec<u32>,
    #[serde(default)]
    pub dest_dir: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveAllAttachmentsOutput {
    pub saved_count: usize,
    pub dest_dir: String,
    pub saved_paths: Vec<String>,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetMessagePrintHtmlInput {
    pub account_id: u32,
    pub message_id: u32,
    #[serde(default)]
    pub include_header: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetMessagePrintHtmlOutput {
    pub subject: String,
    pub html_document: String,
    pub plain_text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenMessageWindowInput {
    pub account_id: u32,
    pub message_id: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenMessageWindowOutput {
    pub window_label: String,
    pub opened: bool,
    pub message: OpenMessageOutput,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ListContactsInput {
    pub query: Option<String>,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactDto {
    pub id: Option<u32>,
    pub name: String,
    pub email: String,
    pub notes: String,
    pub phone: String,
    pub organization: String,
    pub job_title: String,
    pub groups: Vec<String>,
    pub is_user_contact: bool,
    pub message_count: usize,
    pub last_contact_ts: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListContactsOutput {
    pub items: Vec<ContactDto>,
    pub total: usize,
    pub has_more: bool,
    pub state_version: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveContactInput {
    #[serde(default)]
    pub id: Option<u32>,
    pub name: String,
    pub email: String,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default)]
    pub organization: Option<String>,
    #[serde(default)]
    pub job_title: Option<String>,
    #[serde(default)]
    pub groups: Option<Vec<String>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveContactOutput {
    pub contact: ContactDto,
    pub state_version: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteContactInput {
    pub contact_id: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteContactOutput {
    pub deleted: bool,
    pub state_version: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImportContactsFormatDto {
    #[default]
    Auto,
    Vcard,
    Csv,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportContactsInput {
    #[serde(default)]
    pub format: ImportContactsFormatDto,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportContactsOutput {
    pub added: usize,
    pub skipped: usize,
    pub total_parsed: usize,
    pub state_version: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ExportContactsFormatDto {
    Vcard,
    Csv,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportContactsInput {
    pub format: ExportContactsFormatDto,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportContactsOutput {
    pub format: ExportContactsFormatDto,
    pub filename: String,
    pub content: String,
    pub count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageStatsDto {
    pub total_bytes: u64,
    pub mail_bytes: u64,
    pub config_bytes: u64,
    pub file_count: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FullPreferencesDto {
    pub theme_id: String,
    pub dark_mode: Option<bool>,
    pub reader_position: ReaderPositionDto,
    pub compact_list: bool,
    pub show_avatars: bool,
    pub list_group_by_date: bool,
    pub reader_zoom_percent: u32,
    pub ui_text_scale_percent: u32,
    pub auto_sync_minutes: u32,
    pub push_enabled: bool,
    pub desktop_notifications: bool,
    pub notification_show_content: bool,
    pub close_to_minimize: bool,
    pub minimize_to_tray: bool,
    pub check_updates: bool,
    pub block_remote_images: bool,
    pub show_remote_banner: bool,
    pub clean_links: bool,
    pub read_mark_mode: String,
    pub thread_newest_first: bool,
    pub undo_send_secs: u32,
    pub signature_above_quote: bool,
    pub signature_dashes: bool,
    pub attachment_reminder_keywords: String,
    pub reply_header_template: String,
    pub single_key_shortcuts: bool,
    pub default_view: String,
    pub app_lock_enabled: bool,
    pub app_lock_idle_minutes: u32,
    pub app_lock_on_startup: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct FullPreferencesPatchDto {
    pub theme_id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub dark_mode: Option<Option<bool>>,
    pub reader_position: Option<ReaderPositionDto>,
    pub compact_list: Option<bool>,
    pub show_avatars: Option<bool>,
    pub list_group_by_date: Option<bool>,
    pub reader_zoom_percent: Option<u32>,
    pub ui_text_scale_percent: Option<u32>,
    pub auto_sync_minutes: Option<u32>,
    pub push_enabled: Option<bool>,
    pub desktop_notifications: Option<bool>,
    pub notification_show_content: Option<bool>,
    pub close_to_minimize: Option<bool>,
    pub minimize_to_tray: Option<bool>,
    pub check_updates: Option<bool>,
    pub block_remote_images: Option<bool>,
    pub show_remote_banner: Option<bool>,
    pub clean_links: Option<bool>,
    pub read_mark_mode: Option<String>,
    pub thread_newest_first: Option<bool>,
    pub undo_send_secs: Option<u32>,
    pub signature_above_quote: Option<bool>,
    pub signature_dashes: Option<bool>,
    pub attachment_reminder_keywords: Option<String>,
    pub reply_header_template: Option<String>,
    pub single_key_shortcuts: Option<bool>,
    pub default_view: Option<String>,
    pub app_lock_enabled: Option<bool>,
    pub app_lock_idle_minutes: Option<u32>,
    pub app_lock_on_startup: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetFullPreferencesOutput {
    pub preferences: FullPreferencesDto,
    pub has_passcode: bool,
    pub allowed_remote_senders: Vec<String>,
    pub data_dir: String,
    pub mail_data_dir: String,
    pub storage_stats: StorageStatsDto,
    pub app_version: String,
    pub is_portable: bool,
    pub state_version: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateFullPreferencesOutput {
    pub preferences: FullPreferencesDto,
    pub state_version: u64,
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SetAppLockPasscodeInput {
    pub current_passcode: Option<String>,
    pub new_passcode: Option<String>,
    pub enable_on_startup: Option<bool>,
    pub idle_minutes: Option<u32>,
}

impl std::fmt::Debug for SetAppLockPasscodeInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SetAppLockPasscodeInput")
            .field(
                "current_passcode",
                &self.current_passcode.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "new_passcode",
                &self.new_passcode.as_ref().map(|_| "[REDACTED]"),
            )
            .field("enable_on_startup", &self.enable_on_startup)
            .field("idle_minutes", &self.idle_minutes)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetAppLockPasscodeOutput {
    pub has_passcode: bool,
    pub app_lock_enabled: bool,
    pub message: String,
    pub state_version: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TrustedSenderActionDto {
    List,
    Add,
    Remove,
    Clear,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManageTrustedSendersInput {
    pub action: TrustedSenderActionDto,
    #[serde(default)]
    pub sender: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManageTrustedSendersOutput {
    pub senders: Vec<String>,
    pub state_version: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ClearBodyCachesInput {
    pub keep_days: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearBodyCachesOutput {
    pub accepted: bool,
    pub message: String,
    pub storage_stats: StorageStatsDto,
    pub state_version: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckAppUpdateOutput {
    pub checked: bool,
    pub update_available: bool,
    pub current_version: String,
    pub target: String,
    pub latest_version: Option<String>,
    pub notes: Option<String>,
    pub pub_date: Option<String>,
    pub download_url: Option<String>,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetMessageTagsInput {
    pub account_id: u32,
    pub message_id: u32,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetMessageTagsOutput {
    pub account_id: u32,
    pub message_id: u32,
    pub tags: Vec<String>,
    pub state_version: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GetReaderViewOptionsInput {
    pub account_id: u32,
    pub message_id: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetReaderViewOptionsInput {
    pub account_id: u32,
    pub message_id: u32,
    #[serde(default)]
    pub per_message_dark: Option<Option<bool>>,
    #[serde(default)]
    pub reading_view: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderViewOptionsOutput {
    pub account_id: u32,
    pub message_id: u32,
    pub per_message_dark: Option<bool>,
    pub reading_view: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderFindInput {
    pub reader_session_id: String,
    pub query: String,
    #[serde(default)]
    pub step: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderFindOutput {
    pub dispatched: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderFindResultEvent {
    pub reader_session_id: String,
    pub current: u32,
    pub total: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ThreadCardActionKindDto {
    Reply,
    ReplyAll,
    Forward,
    OpenAttachment,
    SaveAttachment,
    SaveAllAttachments,
    CopyBody,
    ExportEml,
    Mdn,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadCardActionEvent {
    pub kind: ThreadCardActionKindDto,
    pub account_id: u32,
    pub message_id: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAttachmentInput {
    pub attachment_id: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAttachmentOutput {
    pub opened: bool,
    pub path: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PickDirectoryOutput {
    pub cancelled: bool,
    pub path: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bootstrap_and_view_dto_serialize_camel_case_contract() {
        let dto = MainBootstrapDto {
            mode: AppMode::Demo,
            locked: false,
            has_passcode: true,
            cache_ready: true,
            state_version: 3,
            accounts: vec![AccountSummaryDto {
                id: 1,
                display_name: "测试工作邮箱".to_string(),
                email: "demo@hylki.example".to_string(),
                label: None,
                color: None,
                emoji: None,
            }],
            folders: vec![FolderSummaryDto {
                id: 1,
                account_id: 1,
                name: "收件箱".to_string(),
                path: "INBOX".to_string(),
                unread_count: 128,
                total_count: 10_000,
                has_more_remote: false,
            }],
            current_view: MainViewDto::AccountFolder {
                account_id: 2,
                folder_id: 17,
            },
            preferences: MainPreferencesDto {
                theme_id: "system".to_string(),
                dark_mode: None,
                reader_position: ReaderPositionDto::Right,
                sidebar_width: 240.0,
                list_width: 360.0,
                list_below_height: 360.0,
                sidebar_collapsed: false,
                compact_list: false,
                show_avatars: true,
                list_group_by_date: true,
                reader_zoom_percent: 100,
            },
            sync_status: SyncStatusDto {
                syncing: false,
                message: None,
                last_sync_at: Some(1_700_000_000),
            },
            tags: Vec::new(),
            templates: Vec::new(),
        };

        let val = serde_json::to_value(&dto).expect("serialize bootstrap");
        assert_eq!(val["mode"], "demo");
        assert_eq!(val["hasPasscode"], true);
        assert_eq!(val["cacheReady"], true);
        assert_eq!(val["stateVersion"], 3);
        assert_eq!(val["accounts"][0]["displayName"], "测试工作邮箱");
        assert_eq!(val["folders"][0]["accountId"], 1);
        assert_eq!(val["folders"][0]["unreadCount"], 128);
        assert_eq!(val["folders"][0]["totalCount"], 10_000);
        assert_eq!(val["folders"][0]["hasMoreRemote"], false);
        assert_eq!(
            val["currentView"],
            json!({ "kind": "accountFolder", "accountId": 2, "folderId": 17 })
        );
        assert_eq!(val["preferences"]["readerPosition"], "right");
        assert_eq!(val["preferences"]["sidebarWidth"], 240.0);
        assert_eq!(val["preferences"]["readerZoomPercent"], 100);
        assert_eq!(val["syncStatus"]["lastSyncAt"], 1_700_000_000);
    }

    #[test]
    fn list_open_layout_and_error_dtos_match_contract() {
        let input_json = json!({
            "requestId": "req-1",
            "view": { "kind": "unifiedInbox" },
            "query": "from:test",
            "scope": "allFolders",
            "sort": "importanceFirst",
            "filter": { "unread": true, "hasAttachment": true },
            "offset": 0,
            "limit": 50
        });
        let parsed: ListMessagesInput =
            serde_json::from_value(input_json).expect("deserialize ListMessagesInput");
        assert_eq!(parsed.scope, SearchScopeDto::AllFolders);
        assert_eq!(parsed.sort, MessageSortDto::ImportanceFirst);
        assert_eq!(parsed.filter.unread, Some(true));
        assert_eq!(parsed.filter.has_attachment, Some(true));

        let err = IpcError::message_not_found(42, Some("req-2".to_string()));
        let err_val = serde_json::to_value(&err).expect("serialize IpcError");
        assert_eq!(
            err_val,
            json!({
                "code": "MESSAGE_NOT_FOUND",
                "message": "未找到邮件 #42",
                "retryable": false,
                "requestId": "req-2"
            })
        );

        let err_no_req = IpcError::locked(None);
        let err_no_req_val = serde_json::to_value(&err_no_req).expect("serialize locked error");
        assert!(err_no_req_val.get("requestId").is_none());

        let layout_json = json!({
            "readerSessionId": "sess-1",
            "layoutRevision": 9,
            "contentRect": { "x": 600.0, "y": 48.0, "width": 680.0, "height": 752.0 },
            "clipRect": { "x": 600.0, "y": 48.0, "width": 680.0, "height": 752.0 },
            "visible": true,
            "devicePixelRatio": 1.25
        });
        let layout: SetReaderLayoutInput =
            serde_json::from_value(layout_json).expect("deserialize SetReaderLayoutInput");
        assert_eq!(layout.layout_revision, 9);
        assert_eq!(layout.device_pixel_ratio, Some(1.25));
    }

    #[test]
    fn preferences_patch_distinguishes_omitted_vs_null_dark_mode() {
        let omitted: MainPreferencesPatch =
            serde_json::from_value(json!({ "sidebarWidth": 220.0 })).expect("omitted patch");
        assert_eq!(omitted.dark_mode, None);
        assert_eq!(omitted.sidebar_width, Some(220.0));

        let explicit_null: MainPreferencesPatch =
            serde_json::from_value(json!({ "darkMode": null })).expect("null darkMode patch");
        assert_eq!(explicit_null.dark_mode, Some(None));

        let explicit_true: MainPreferencesPatch =
            serde_json::from_value(json!({ "darkMode": true })).expect("true darkMode patch");
        assert_eq!(explicit_true.dark_mode, Some(Some(true)));

        let unknown =
            serde_json::from_value::<MainPreferencesPatch>(json!({ "unknownField": 123 }));
        assert!(unknown.is_err(), "non-whitelisted patch field must fail");
    }

    #[test]
    fn milestone_c_lock_and_flag_dtos_match_frozen_contract() {
        let unlock_in: UnlockAppInput =
            serde_json::from_value(json!({ "passcode": "1234" })).expect("UnlockAppInput");
        assert_eq!(unlock_in.passcode, "1234");

        let unlock_out = UnlockAppOutput {
            unlocked: false,
            locked: true,
            error: Some("密码错误".to_string()),
            retry_after_secs: Some(5),
        };
        let unlock_val = serde_json::to_value(&unlock_out).expect("UnlockAppOutput");
        assert_eq!(unlock_val["unlocked"], false);
        assert_eq!(unlock_val["locked"], true);
        assert_eq!(unlock_val["retryAfterSecs"], 5);

        let seen_in: SetMessageSeenInput =
            serde_json::from_value(json!({ "accountId": 1, "messageId": 99, "seen": true }))
                .expect("SetMessageSeenInput");
        assert!(seen_in.seen);

        let seen_out = SetMessageSeenOutput {
            account_id: 1,
            message_id: 99,
            unread: false,
            state_version: 12,
        };
        let seen_val = serde_json::to_value(&seen_out).expect("SetMessageSeenOutput");
        assert_eq!(seen_val["accountId"], 1);
        assert_eq!(seen_val["messageId"], 99);
        assert_eq!(seen_val["unread"], false);
        assert_eq!(seen_val["stateVersion"], 12);

        let flag_in: SetMessageFlaggedInput =
            serde_json::from_value(json!({ "accountId": 2, "messageId": 50, "flagged": true }))
                .expect("SetMessageFlaggedInput");
        assert!(flag_in.flagged);

        let lock_ev = LockStateEvent {
            state_version: 4,
            locked: true,
            has_passcode: true,
            locked_reason: Some("idle".to_string()),
        };
        let lock_val = serde_json::to_value(&lock_ev).expect("LockStateEvent");
        assert_eq!(lock_val["locked"], true);
        assert_eq!(lock_val["hasPasscode"], true);
        assert_eq!(lock_val["lockedReason"], "idle");
    }

    #[test]
    fn wave1_seven_new_command_dtos_match_frozen_contract() {
        let allow_in: AllowRemoteImagesInput = serde_json::from_value(json!({
            "accountId": 1,
            "messageId": 42,
            "trustSender": true
        }))
        .expect("AllowRemoteImagesInput");
        assert!(allow_in.trust_sender);

        let allow_out = serde_json::to_value(AllowRemoteImagesOutput {
            allowed: true,
            sender_trusted: true,
            state_version: 9,
        })
        .expect("AllowRemoteImagesOutput");
        assert_eq!(
            allow_out,
            json!({ "allowed": true, "senderTrusted": true, "stateVersion": 9 })
        );

        let older_in: LoadOlderMessagesInput = serde_json::from_value(json!({
            "view": { "kind": "accountFolder", "accountId": 1, "folderId": 3 },
            "limit": 50
        }))
        .expect("LoadOlderMessagesInput");
        assert_eq!(older_in.limit, 50);

        let older_out = serde_json::to_value(LoadOlderMessagesOutput {
            accepted: true,
            has_more_remote: true,
            state_version: 10,
        })
        .expect("LoadOlderMessagesOutput");
        assert_eq!(
            older_out,
            json!({ "accepted": true, "hasMoreRemote": true, "stateVersion": 10 })
        );

        let action_in: MessageActionInput = serde_json::from_value(json!({
            "accountId": 1,
            "messageId": 7
        }))
        .expect("MessageActionInput");
        assert_eq!(action_in.message_id, 7);

        let move_in: MoveMessageInput = serde_json::from_value(json!({
            "accountId": 1,
            "messageId": 7,
            "targetFolderPath": "Archive"
        }))
        .expect("MoveMessageInput");
        assert_eq!(move_in.target_folder_path, "Archive");

        let move_out = serde_json::to_value(MessageMoveOutput {
            account_id: 1,
            message_id: 7,
            rfc_message_id: "<m7@example.com>".to_string(),
            original_folder_path: "INBOX".to_string(),
            target_folder_path: "Archive".to_string(),
            state_version: 11,
        })
        .expect("MessageMoveOutput");
        assert_eq!(
            move_out,
            json!({
                "accountId": 1,
                "messageId": 7,
                "rfcMessageId": "<m7@example.com>",
                "originalFolderPath": "INBOX",
                "targetFolderPath": "Archive",
                "stateVersion": 11
            })
        );

        let undo_in: UndoMessageMoveInput = serde_json::from_value(json!({
            "accountId": 1,
            "messageId": 7,
            "rfcMessageId": "<m7@example.com>",
            "currentFolderPath": "Archive",
            "originalFolderPath": "INBOX"
        }))
        .expect("UndoMessageMoveInput");
        assert_eq!(undo_in.original_folder_path, "INBOX");

        let undo_out = serde_json::to_value(UndoMessageMoveOutput {
            accepted: true,
            state_version: 12,
        })
        .expect("UndoMessageMoveOutput");
        assert_eq!(undo_out, json!({ "accepted": true, "stateVersion": 12 }));

        let open_out = serde_json::to_value(OpenMessageOutput {
            request_id: "r1".to_string(),
            reader_session_id: "s1".to_string(),
            account_id: 1,
            message_id: 7,
            subject: "Subj".to_string(),
            sender: "a@b.com".to_string(),
            recipients: "c@d.com".to_string(),
            cc: String::new(),
            date_display: "10:00".to_string(),
            body_status: BodyStatusDto::Ready,
            plain_text: "hi".to_string(),
            has_html: true,
            html_body: Some("<p>hi</p>".to_string()),
            flagged: false,
            attachments: Vec::new(),
            thread_messages: Vec::new(),
            remote_images_blocked: Some(true),
            rfc_message_id: Some("<m7@example.com>".to_string()),
            folder_path: Some("INBOX".to_string()),
        })
        .expect("OpenMessageOutput");
        assert_eq!(open_out["remoteImagesBlocked"], true);
        assert_eq!(open_out["rfcMessageId"], "<m7@example.com>");
        assert_eq!(open_out["folderPath"], "INBOX");
    }

    #[test]
    fn wave2_compose_and_accounts_dtos_match_frozen_contract() {
        let draft_in: SaveDraftInput = serde_json::from_value(json!({
            "id": 3,
            "accountId": "1",
            "toAddr": "a@example.com",
            "subject": "主题",
            "bodyText": "正文",
            "bodyFormat": "markdown",
            "silent": true
        }))
        .expect("SaveDraftInput");
        assert_eq!(draft_in.id.as_deref(), Some("3"));
        assert_eq!(draft_in.account_id, "1");
        assert!(draft_in.silent);
        assert_eq!(draft_in.body_format, Some(BodyFormatDto::Markdown));

        let draft_dto = serde_json::to_value(LocalDraftDto {
            id: "3".to_string(),
            account_id: "1".to_string(),
            to_addr: "a@example.com".to_string(),
            cc_addr: String::new(),
            bcc_addr: String::new(),
            subject: "主题".to_string(),
            body_text: "正文".to_string(),
            in_reply_to: None,
            from_addr: None,
            reply_to: None,
            body_format: BodyFormatDto::Markdown,
            attachment_paths: vec![],
            updated_at: 1_700_000_000,
        })
        .expect("LocalDraftDto");
        assert_eq!(draft_dto["id"], "3");
        assert_eq!(draft_dto["accountId"], "1");
        assert_eq!(draft_dto["bodyFormat"], "markdown");

        let outbox_ev = serde_json::to_value(OutboxUpdatedEvent {
            state_version: 5,
            items: vec![OutboxItemDto {
                id: "9".to_string(),
                account_id: "1".to_string(),
                to_addr: "b@example.com".to_string(),
                subject: "Sub".to_string(),
                status: OutboxStatusDto::Queued,
                last_error: None,
                send_at: 1_700_000_050,
            }],
            event_kind: OutboxEventKindDto::Enqueued,
            undo_token: Some("42".to_string()),
            outbox_id: Some("9".to_string()),
            message: None,
        })
        .expect("OutboxUpdatedEvent");
        assert_eq!(outbox_ev["eventKind"], "enqueued");
        assert_eq!(outbox_ev["undoToken"], "42");
        assert_eq!(outbox_ev["items"][0]["status"], "queued");

        let acc_in: SaveAccountInput = serde_json::from_value(json!({
            "id": 2,
            "authBackend": "microsoftGraphOAuth2",
            "displayName": "Work",
            "email": "me@outlook.com",
            "imapHost": "outlook.office365.com",
            "imapPort": 993,
            "imapSecurity": "tls",
            "imapUsername": "me@outlook.com",
            "smtpHost": "smtp.office365.com",
            "smtpPort": 587,
            "smtpSecurity": "startTls",
            "smtpUsername": "me@outlook.com"
        }))
        .expect("SaveAccountInput");
        assert_eq!(acc_in.id.as_deref(), Some("2"));
        assert_eq!(acc_in.auth_backend, AuthBackendDto::MicrosoftGraphOAuth2);
        assert_eq!(acc_in.smtp_security, SecurityModeDto::StartTls);

        let acc_ev = serde_json::to_value(AccountEvent {
            state_version: 8,
            kind: AccountEventKindDto::OauthReauthorized,
            ok: true,
            account_id: Some("2".to_string()),
            email: Some("me@outlook.com".to_string()),
            message: "ok".to_string(),
        })
        .expect("AccountEvent");
        assert_eq!(acc_ev["kind"], "oauthReauthorized");
        assert_eq!(acc_ev["accountId"], "2");
    }

    #[test]
    fn wave3_attachments_contacts_and_settings_dtos_match_frozen_contract() {
        let att_in: ListCachedAttachmentsInput =
            serde_json::from_value(json!({ "query": "pdf", "offset": 10, "limit": 20 }))
                .expect("ListCachedAttachmentsInput");
        assert_eq!(att_in.offset, Some(10));
        assert_eq!(att_in.limit, Some(20));

        let att_dto = serde_json::to_value(CachedAttachmentDto {
            id: 12,
            message_id: 1,
            account_id: 1,
            filename: "shot.png".to_string(),
            mime_type: "image/png".to_string(),
            size_bytes: 1024,
            sender: "a@b.com".to_string(),
            subject: "Sub".to_string(),
            date_ts: 1_700_000_000,
            date_display: "10:00".to_string(),
            downloaded: true,
            is_image: true,
        })
        .expect("CachedAttachmentDto");
        assert_eq!(att_dto["isImage"], true);
        assert_eq!(att_dto["sizeBytes"], 1024);

        let contact_null_id = serde_json::to_value(ContactDto {
            id: None,
            name: "Aggregated".to_string(),
            email: "agg@example.com".to_string(),
            notes: String::new(),
            phone: String::new(),
            organization: String::new(),
            job_title: String::new(),
            groups: vec![],
            is_user_contact: false,
            message_count: 3,
            last_contact_ts: 0,
        })
        .expect("ContactDto");
        assert_eq!(contact_null_id["id"], serde_json::Value::Null);
        assert_eq!(contact_null_id["isUserContact"], false);
        assert_eq!(contact_null_id["lastContactTs"], 0);

        let patch_null_dark: FullPreferencesPatchDto =
            serde_json::from_value(json!({ "darkMode": null, "uiTextScalePercent": 110 }))
                .expect("FullPreferencesPatchDto");
        assert_eq!(patch_null_dark.dark_mode, Some(None));
        assert_eq!(patch_null_dark.ui_text_scale_percent, Some(110));

        let unknown_patch =
            serde_json::from_value::<FullPreferencesPatchDto>(json!({ "unexpectedField": true }));
        assert!(unknown_patch.is_err());

        let passcode_in: SetAppLockPasscodeInput = serde_json::from_value(json!({
            "currentPasscode": "1234",
            "newPasscode": null,
            "enableOnStartup": false,
            "idleMinutes": 5
        }))
        .expect("SetAppLockPasscodeInput");
        assert_eq!(passcode_in.new_passcode, None);
        assert_eq!(passcode_in.idle_minutes, Some(5));
    }
}
