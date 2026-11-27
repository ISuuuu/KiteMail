//! Frozen IPC DTOs and event payloads for the KiteMail Tauri main window.

use kitemail_mail_core::{
    AccountSummary, AttachmentMeta, FolderSummary, MessageSort, MessageSummary, SearchScope,
    ThreadMessage,
};
use serde::{Deserialize, Deserializer, Serialize};

pub const MAIN_WINDOW_LABEL: &str = "main";
pub const EVENT_MAIL_CHANGED: &str = "main://mail-changed";
pub const EVENT_SYNC_STATUS: &str = "main://sync-status";
pub const EVENT_READER_STATUS: &str = "main://reader-status";
#[allow(dead_code)]
pub const EVENT_LOCK_STATE: &str = "main://lock-state";

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
}

impl From<&AccountSummary> for AccountSummaryDto {
    fn from(value: &AccountSummary) -> Self {
        Self {
            id: value.id,
            display_name: value.display_name.clone(),
            email: value.email.clone(),
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
    pub state_version: u64,
    pub accounts: Vec<AccountSummaryDto>,
    pub folders: Vec<FolderSummaryDto>,
    pub current_view: MainViewDto,
    pub preferences: MainPreferencesDto,
    pub sync_status: SyncStatusDto,
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
    pub rfc_message_id: String,
    pub subject: String,
    pub sender: String,
    pub recipients: String,
    pub cc: String,
    pub date_ts: i64,
    pub date_display: String,
    pub body_text: String,
    pub has_html: bool,
    pub html_body: Option<String>,
    pub body_loaded: bool,
    pub unread: bool,
    pub flagged: bool,
    pub attachments: Vec<AttachmentMetaDto>,
    pub uid: u32,
    pub folder_path: String,
}

impl From<&ThreadMessage> for ThreadMessageDto {
    fn from(value: &ThreadMessage) -> Self {
        Self {
            id: value.id,
            rfc_message_id: value.rfc_message_id.clone(),
            subject: value.subject.clone(),
            sender: value.sender.clone(),
            recipients: value.recipients.clone(),
            cc: value.cc.clone(),
            date_ts: value.date_ts,
            date_display: value.date_display.clone(),
            body_text: value.body_text.clone(),
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
pub struct ReaderStatusEvent {
    pub reader_session_id: String,
    pub account_id: u32,
    pub message_id: u32,
    pub body_status: BodyStatusDto,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockStateEvent {
    pub state_version: u64,
    pub locked: bool,
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
            state_version: 3,
            accounts: vec![AccountSummaryDto {
                id: 1,
                display_name: "测试工作邮箱".to_string(),
                email: "demo@hylki.example".to_string(),
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
        };

        let val = serde_json::to_value(&dto).expect("serialize bootstrap");
        assert_eq!(val["mode"], "demo");
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
}
