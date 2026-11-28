//! Mail session state for KiteMail Tauri preview.
//!
//! Supports both `AppMode::Demo` (in-memory `DemoStore`) and `AppMode::Live`
//! (offline-first `CacheStore::open_for_data_dir_read_only` + background `MailWorkerHandle`),
//! with in-flight body fetch deduplication, `read_mark_mode` (`shown` / `delay` / `manual`),
//! and `kitemail_mail_core::app_lock` locking/cooldown state machine.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::{Duration, Instant};

use kitemail_mail_core::app_lock::{
    self, AutoLockTickInput, AutoLockTracker, AutoLockTrigger, BackgroundLockPolicy,
};
use kitemail_mail_core::cache::CacheStore;
use kitemail_mail_core::config::{self, AccountConfig, AppPreferences, AuthBackend};
use kitemail_mail_core::demo::{
    DemoStore, DEMO_ARCHIVE_ID, DEMO_DRAFTS_ID, DEMO_INBOX_ID, DEMO_JUNK_ID, DEMO_SENT_ID,
    DEMO_TRASH_ID,
};
use kitemail_mail_core::imap_client::strip_remote_images_for_webview;
use kitemail_mail_core::search_query::SearchQuery;
use kitemail_mail_core::worker::{MailWorkerHandle, MailWorkerSender, PageQueryRequest};
use kitemail_mail_core::{
    secrets, start_of_today_ts, LocalDraft, MailPage, MailRequest, MessageDetail, MessageFilter,
    MessageSort, MessageSummary, OutboxItem, QuickFilterSet, UndoOutboxOutcome, UnifiedView,
    UserContact, WorkerEvent,
};
use kitemail_windows_mail_reader::WebViewAction;
use tauri::Emitter;

use crate::dto::{
    AccountEvent, AccountEventKindDto, AccountSummaryDto, AllowRemoteImagesInput,
    AllowRemoteImagesOutput, AppMode, AttachmentMetaDto, BodyStatusDto, DraftSavedEvent,
    FolderSummaryDto, GetReaderViewOptionsInput, IpcError, ListMessagesInput, ListMessagesOutput,
    ListThreadSummariesInput, ListThreadSummariesOutput, LoadOlderMessagesInput,
    LoadOlderMessagesOutput, LockAppOutput, LockStateEvent, MailChangedEvent, MailTemplateDto,
    MainBootstrapDto, MainViewDto, MessageActionInput, MessageFilterDto, MessageMoveOutput,
    MessageSummaryDto, MoveMessageInput, OpenMessageInput, OpenMessageOutput, OutboxEventKindDto,
    OutboxItemDto, OutboxUpdatedEvent, ReaderFindInput, ReaderFindOutput, ReaderFindResultEvent,
    ReaderPositionDto, ReaderStatusEvent, ReaderViewOptionsOutput, RefreshMailInput,
    RefreshMailOutput, SaveAllAttachmentsInput, SaveAttachmentInput, SearchScopeDto,
    SetMessageFlaggedInput, SetMessageFlaggedOutput, SetMessageSeenInput, SetMessageSeenOutput,
    SetMessageTagsInput, SetMessageTagsOutput, SetReaderViewOptionsInput, SyncStatusDto,
    SyncStatusEvent, TagItemDto, ThreadCardActionEvent, ThreadCardActionKindDto, ThreadMessageDto,
    UndoMessageMoveInput, UndoMessageMoveOutput, UnlockAppInput, UnlockAppOutput,
    VerifyAccountOutput, EVENT_ACCOUNT_EVENT, EVENT_DRAFT_SAVED, EVENT_LOCK_STATE,
    EVENT_MAIL_CHANGED, EVENT_OUTBOX_UPDATED, EVENT_READER_FIND_RESULT, EVENT_SYNC_STATUS,
    EVENT_THREAD_CARD_ACTION, MAIN_WINDOW_LABEL,
};
use crate::preferences::{view_from_last_str, PreferencesStore};
use crate::reader::{
    ReaderAppearance, ReaderDocumentSpec, ReaderManager, ThreadActionHandler, ThreadRenderOptions,
};

pub const MAX_PAGE_LIMIT: usize = 500;
pub const MAX_QUERY_LEN: usize = 1000;
const BODY_CACHE_CAPACITY: usize = 64;
const DEFAULT_BODY_FETCH_TIMEOUT: Duration = Duration::from_secs(20);
const DEFAULT_READ_MARK_DELAY: Duration = Duration::from_secs(2);
const AUTO_SYNC_CHECK_INTERVAL: Duration = Duration::from_secs(30);

/// Abstraction for emitting main-window scoped events (`main://*`).
pub trait MainEventEmitter: Send + Sync {
    fn emit_mail_changed(&self, event: MailChangedEvent);
    fn emit_sync_status(&self, event: SyncStatusEvent);
    fn emit_lock_state(&self, event: LockStateEvent);
    fn emit_outbox_updated(&self, event: OutboxUpdatedEvent);
    fn emit_draft_saved(&self, event: DraftSavedEvent);
    fn emit_account_event(&self, event: AccountEvent);
    fn emit_thread_card_action(&self, _event: ThreadCardActionEvent) {}
    fn emit_reader_find_result(&self, _event: ReaderFindResultEvent) {}
}

pub struct TauriMainEventEmitter {
    app_handle: tauri::AppHandle,
}

impl TauriMainEventEmitter {
    pub fn new(app_handle: tauri::AppHandle) -> Self {
        Self { app_handle }
    }
}

impl MainEventEmitter for TauriMainEventEmitter {
    fn emit_mail_changed(&self, event: MailChangedEvent) {
        let _ = self
            .app_handle
            .emit_to(MAIN_WINDOW_LABEL, EVENT_MAIL_CHANGED, event);
    }

    fn emit_sync_status(&self, event: SyncStatusEvent) {
        let _ = self
            .app_handle
            .emit_to(MAIN_WINDOW_LABEL, EVENT_SYNC_STATUS, event);
    }

    fn emit_lock_state(&self, event: LockStateEvent) {
        let locked = event.locked;
        let _ = self
            .app_handle
            .emit_to(MAIN_WINDOW_LABEL, EVENT_LOCK_STATE, event.clone());
        let _ = self.app_handle.emit_to(
            crate::settings_window::SETTINGS_WINDOW_LABEL,
            EVENT_LOCK_STATE,
            event,
        );
        if locked {
            crate::settings_window::close_settings_window(&self.app_handle);
        }
    }

    fn emit_outbox_updated(&self, event: OutboxUpdatedEvent) {
        let _ = self
            .app_handle
            .emit_to(MAIN_WINDOW_LABEL, EVENT_OUTBOX_UPDATED, event);
    }

    fn emit_draft_saved(&self, event: DraftSavedEvent) {
        let _ = self
            .app_handle
            .emit_to(MAIN_WINDOW_LABEL, EVENT_DRAFT_SAVED, event);
    }

    fn emit_account_event(&self, event: AccountEvent) {
        let _ = self
            .app_handle
            .emit_to(MAIN_WINDOW_LABEL, EVENT_ACCOUNT_EVENT, event.clone());
        let _ = self.app_handle.emit_to(
            crate::settings_window::SETTINGS_WINDOW_LABEL,
            EVENT_ACCOUNT_EVENT,
            event,
        );
    }

    fn emit_thread_card_action(&self, event: ThreadCardActionEvent) {
        let _ = self
            .app_handle
            .emit_to(MAIN_WINDOW_LABEL, EVENT_THREAD_CARD_ACTION, event);
    }

    fn emit_reader_find_result(&self, event: ReaderFindResultEvent) {
        let _ = self
            .app_handle
            .emit_to(MAIN_WINDOW_LABEL, EVENT_READER_FIND_RESULT, event);
    }
}

/// Abstraction over `MailWorkerHandle` so Live session logic can be tested deterministically.
pub trait LiveWorkerSink: Send + Sync {
    fn send(&self, req: MailRequest);
    fn prepare_quit(&self);
    fn query_page(&self, _req: PageQueryRequest) -> Option<Result<MailPage, String>> {
        None
    }
    fn is_async_worker(&self) -> bool {
        false
    }
}

struct RealMailWorkerSink {
    sender: MailWorkerSender,
}

impl LiveWorkerSink for RealMailWorkerSink {
    fn send(&self, req: MailRequest) {
        self.sender.send(req);
    }

    fn prepare_quit(&self) {
        let _ = self.sender.prepare_quit_now();
    }

    fn query_page(&self, req: PageQueryRequest) -> Option<Result<MailPage, String>> {
        Some(self.sender.query_page(req))
    }

    fn is_async_worker(&self) -> bool {
        true
    }
}

#[cfg(test)]
pub struct NullEventEmitter;

#[cfg(test)]
impl MainEventEmitter for NullEventEmitter {
    fn emit_mail_changed(&self, _: MailChangedEvent) {}
    fn emit_sync_status(&self, _: SyncStatusEvent) {}
    fn emit_lock_state(&self, _: LockStateEvent) {}
    fn emit_outbox_updated(&self, _: OutboxUpdatedEvent) {}
    fn emit_draft_saved(&self, _: DraftSavedEvent) {}
    fn emit_account_event(&self, _: AccountEvent) {}
}

pub(crate) struct DemoOutboxEntry {
    pub(crate) item: OutboxItem,
    pub(crate) undo_token: u64,
}

pub(crate) struct DemoSessionState {
    pub(crate) store: DemoStore,
    pub(crate) account_configs: Vec<AccountConfig>,
    pub(crate) account_has_password: HashMap<u32, bool>,
    pub(crate) next_account_id: u32,
    pub(crate) accounts: Vec<AccountSummaryDto>,
    pub(crate) base_folders: Vec<FolderSummaryDto>,
    pub(crate) inbox: Vec<MessageSummary>,
    pub(crate) sent: Vec<MessageSummary>,
    pub(crate) drafts: Vec<MessageSummary>,
    pub(crate) archive: Vec<MessageSummary>,
    pub(crate) junk: Vec<MessageSummary>,
    pub(crate) trash: Vec<MessageSummary>,
    pub(crate) local_drafts: Vec<LocalDraft>,
    pub(crate) next_local_draft_id: u32,
    pub(crate) outbox: Vec<DemoOutboxEntry>,
    pub(crate) next_outbox_id: u32,
    pub(crate) next_undo_token: u64,
    pub(crate) send_idempotency: HashMap<(u32, String), (u32, u64, Option<i64>)>,
    pub(crate) next_sent_msg_id: u32,
    pub(crate) user_contacts: Vec<UserContact>,
    pub(crate) next_user_contact_id: u32,
}

impl DemoSessionState {
    fn new() -> Self {
        let store = DemoStore::new();
        let account_configs: Vec<AccountConfig> = store
            .accounts
            .iter()
            .map(|a| {
                let (label, color) = match a.id {
                    1 => (Some("Work".to_string()), Some("#3584e4".to_string())),
                    2 => (Some("Hyprlab".to_string()), Some("#9141ac".to_string())),
                    3 => (Some("Personal".to_string()), Some("#26a269".to_string())),
                    _ => (None, None),
                };
                AccountConfig {
                    id: a.id,
                    display_name: a.display_name.clone(),
                    email: a.email.clone(),
                    label,
                    color,
                    imap_host: "imap.hylki.example".to_string(),
                    imap_port: 993,
                    imap_username: a.email.clone(),
                    smtp_host: "smtp.hylki.example".to_string(),
                    smtp_port: 465,
                    smtp_username: a.email.clone(),
                    ..AccountConfig::default()
                }
            })
            .collect();
        let account_has_password = account_configs.iter().map(|a| (a.id, true)).collect();
        let next_account_id = account_configs
            .iter()
            .map(|a| a.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        let accounts = account_configs
            .iter()
            .map(AccountSummaryDto::from)
            .collect();
        let base_folders: Vec<FolderSummaryDto> =
            store.folders.iter().map(FolderSummaryDto::from).collect();
        let inbox = store.all_inbox_messages().to_vec();
        let sent = store.all_sent_messages().to_vec();
        let drafts = store.all_draft_messages().to_vec();
        let archive = store.all_archive_messages().to_vec();
        let junk = store.all_junk_messages().to_vec();
        Self {
            store,
            account_configs,
            account_has_password,
            next_account_id,
            accounts,
            base_folders,
            inbox,
            sent,
            drafts,
            archive,
            junk,
            trash: Vec::new(),
            local_drafts: Vec::new(),
            next_local_draft_id: 1,
            outbox: Vec::new(),
            next_outbox_id: 1,
            next_undo_token: 1,
            send_idempotency: HashMap::new(),
            next_sent_msg_id: 100_000,
            user_contacts: vec![UserContact {
                id: 1,
                email: "sophie@studio.dev".to_string(),
                display_name: "Sophie Turner".to_string(),
                notes: "产品设计与路线图负责人（演示）".to_string(),
                ..UserContact::default()
            }],
            next_user_contact_id: 2,
        }
    }

    pub(crate) fn sync_account_summaries(&mut self) {
        self.accounts = self
            .account_configs
            .iter()
            .filter(|a| a.enabled)
            .map(AccountSummaryDto::from)
            .collect();
    }

    pub(crate) fn outbox_dtos(&self) -> Vec<OutboxItemDto> {
        let mut items: Vec<OutboxItemDto> = self
            .outbox
            .iter()
            .map(|e| OutboxItemDto::from(&e.item))
            .collect();
        items.sort_by(|a, b| {
            let aid: u32 = a.id.parse().unwrap_or(0);
            let bid: u32 = b.id.parse().unwrap_or(0);
            bid.cmp(&aid)
        });
        items
    }

    fn slice_for_path(&self, path: &str) -> &[MessageSummary] {
        match path.to_ascii_uppercase().as_str() {
            "INBOX" => &self.inbox,
            "SENT" => &self.sent,
            "DRAFTS" => &self.drafts,
            "ARCHIVE" => &self.archive,
            "JUNK" | "SPAM" => &self.junk,
            "TRASH" | "DELETED" => &self.trash,
            _ => &[],
        }
    }

    fn folders(&self) -> Vec<FolderSummaryDto> {
        self.base_folders
            .iter()
            .map(|f| {
                let mut cloned = f.clone();
                let slice = self.slice_for_path(&f.path);
                let mut unread = 0u32;
                let mut total = 0usize;
                for m in slice {
                    if m.account_id == f.account_id && m.folder_id == f.id {
                        total += 1;
                        if m.unread {
                            unread += 1;
                        }
                    }
                }
                cloned.unread_count = unread;
                cloned.total_count = total;
                cloned
            })
            .collect()
    }

    fn validate_view(&self, view: MainViewDto, request_id: Option<&str>) -> Result<(), IpcError> {
        if let MainViewDto::AccountFolder {
            account_id,
            folder_id,
        } = view
        {
            let exists = self
                .base_folders
                .iter()
                .any(|f| f.account_id == account_id && f.id == folder_id);
            if !exists {
                return Err(IpcError::invalid_input(
                    format!("无效的账号或文件夹: accountId={account_id}, folderId={folder_id}"),
                    request_id.map(str::to_string),
                ));
            }
        }
        Ok(())
    }

    fn folder_name_for(&self, folder_id: u32) -> &str {
        self.base_folders
            .iter()
            .find(|f| f.id == folder_id)
            .map(|f| f.name.as_str())
            .unwrap_or("")
    }

    fn list_messages(
        &self,
        input: &ListMessagesInput,
    ) -> Result<(usize, Vec<MessageSummaryDto>), IpcError> {
        self.validate_view(input.view, Some(&input.request_id))?;

        let parsed = SearchQuery::parse(&input.query);
        let quick = to_quick_filters(input.filter);
        let sort: MessageSort = input.sort.into();
        let today_start = start_of_today_ts();
        let cross_folder =
            !input.query.trim().is_empty() && input.scope == SearchScopeDto::AllFolders;

        let mut matched: Vec<&MessageSummary> = if cross_folder {
            self.inbox
                .iter()
                .chain(self.sent.iter())
                .chain(self.archive.iter())
                .filter(|m| matches_quick_filters(m, quick, today_start))
                .filter(|m| {
                    parsed.is_empty()
                        || parsed.matches_summary(
                            m,
                            self.folder_name_for(m.folder_id),
                            "demo@hylki.example",
                            None,
                        )
                })
                .collect()
        } else {
            match input.view {
                MainViewDto::Unread => self
                    .inbox
                    .iter()
                    .chain(self.sent.iter())
                    .chain(self.archive.iter())
                    .filter(|m| m.unread && matches_quick_filters(m, quick, today_start))
                    .filter(|m| {
                        parsed.is_empty()
                            || parsed.matches_summary(
                                m,
                                self.folder_name_for(m.folder_id),
                                "demo@hylki.example",
                                None,
                            )
                    })
                    .collect(),
                MainViewDto::Starred => self
                    .inbox
                    .iter()
                    .chain(self.sent.iter())
                    .chain(self.archive.iter())
                    .filter(|m| m.flagged && matches_quick_filters(m, quick, today_start))
                    .filter(|m| {
                        parsed.is_empty()
                            || parsed.matches_summary(
                                m,
                                self.folder_name_for(m.folder_id),
                                "demo@hylki.example",
                                None,
                            )
                    })
                    .collect(),
                MainViewDto::UnifiedInbox => {
                    self.filter_slice(&self.inbox, &parsed, quick, today_start)
                }
                MainViewDto::Sent => self.filter_slice(&self.sent, &parsed, quick, today_start),
                MainViewDto::Drafts => self.filter_slice(&self.drafts, &parsed, quick, today_start),
                MainViewDto::Archive => {
                    self.filter_slice(&self.archive, &parsed, quick, today_start)
                }
                MainViewDto::Junk => self.filter_slice(&self.junk, &parsed, quick, today_start),
                MainViewDto::Trash => self.filter_slice(&self.trash, &parsed, quick, today_start),
                MainViewDto::Snoozed => Vec::new(),
                MainViewDto::Filters => self
                    .inbox
                    .iter()
                    .chain(self.sent.iter())
                    .chain(self.archive.iter())
                    .filter(|m| {
                        (m.unread || m.flagged || m.has_attachment)
                            && matches_quick_filters(m, quick, today_start)
                    })
                    .filter(|m| {
                        parsed.is_empty()
                            || parsed.matches_summary(
                                m,
                                self.folder_name_for(m.folder_id),
                                "demo@hylki.example",
                                None,
                            )
                    })
                    .collect(),
                MainViewDto::Tags => self
                    .inbox
                    .iter()
                    .chain(self.sent.iter())
                    .chain(self.archive.iter())
                    .filter(|m| !m.tags.is_empty() && matches_quick_filters(m, quick, today_start))
                    .filter(|m| {
                        parsed.is_empty()
                            || parsed.matches_summary(
                                m,
                                self.folder_name_for(m.folder_id),
                                "demo@hylki.example",
                                None,
                            )
                    })
                    .collect(),
                MainViewDto::AccountFolder {
                    account_id,
                    folder_id,
                } => {
                    let path = self
                        .base_folders
                        .iter()
                        .find(|f| f.account_id == account_id && f.id == folder_id)
                        .map(|f| f.path.as_str())
                        .unwrap_or("");
                    let slice = self.slice_for_path(path);
                    self.filter_slice(slice, &parsed, quick, today_start)
                        .into_iter()
                        .filter(|m| m.account_id == account_id && m.folder_id == folder_id)
                        .collect()
                }
            }
        };

        if let Some(account_id) = input.filter.account_id {
            matched.retain(|m| m.account_id == account_id);
        }

        sort_summary_refs(&mut matched, sort);

        let total = matched.len();
        let limit = input.limit.min(MAX_PAGE_LIMIT);
        let start = input.offset.min(total);
        let end = start.saturating_add(limit).min(total);
        let items = matched[start..end]
            .iter()
            .map(|m| MessageSummaryDto::from(*m))
            .collect();
        Ok((total, items))
    }

    fn filter_slice<'a>(
        &'a self,
        slice: &'a [MessageSummary],
        parsed: &SearchQuery,
        quick: QuickFilterSet,
        today_start: i64,
    ) -> Vec<&'a MessageSummary> {
        slice
            .iter()
            .filter(|m| matches_quick_filters(m, quick, today_start))
            .filter(|m| {
                parsed.is_empty()
                    || parsed.matches_summary(
                        m,
                        self.folder_name_for(m.folder_id),
                        "demo@hylki.example",
                        None,
                    )
            })
            .collect()
    }

    fn find_summary(&self, account_id: u32, message_id: u32) -> Option<&MessageSummary> {
        self.inbox
            .iter()
            .chain(self.sent.iter())
            .chain(self.drafts.iter())
            .chain(self.archive.iter())
            .chain(self.junk.iter())
            .chain(self.trash.iter())
            .find(|m| m.account_id == account_id && m.id == message_id)
    }

    fn find_summary_mut(
        &mut self,
        account_id: u32,
        message_id: u32,
    ) -> Option<&mut MessageSummary> {
        self.inbox
            .iter_mut()
            .chain(self.sent.iter_mut())
            .chain(self.drafts.iter_mut())
            .chain(self.archive.iter_mut())
            .chain(self.junk.iter_mut())
            .chain(self.trash.iter_mut())
            .find(|m| m.account_id == account_id && m.id == message_id)
    }

    fn get_detail(&self, account_id: u32, message_id: u32) -> Option<MessageDetail> {
        if let Some(summary) = self.find_summary(account_id, message_id) {
            let mut detail = self.store.get_message_detail(message_id)?;
            detail.folder_id = summary.folder_id;
            detail.folder_path = summary.folder_path.clone();
            detail.flagged = summary.flagged;
            if let Some(first) = detail.thread_messages.first_mut() {
                if first.id == summary.id {
                    first.unread = summary.unread;
                    first.flagged = summary.flagged;
                    first.folder_path = summary.folder_path.clone();
                }
            }
            return Some(detail);
        }
        self.store
            .get_message_detail(message_id)
            .filter(|d| d.account_id == account_id)
    }

    fn resolve_folder_by_path(&self, account_id: u32, target: &str) -> Option<(u32, String)> {
        let trimmed = target.trim();
        self.base_folders
            .iter()
            .find(|f| {
                f.account_id == account_id
                    && (f.path.eq_ignore_ascii_case(trimmed)
                        || f.name.eq_ignore_ascii_case(trimmed))
            })
            .map(|f| (f.id, f.path.clone()))
    }

    fn slice_mut_for_folder_id(&mut self, folder_id: u32) -> Option<&mut Vec<MessageSummary>> {
        let path = self
            .base_folders
            .iter()
            .find(|f| f.id == folder_id)
            .map(|f| f.path.to_ascii_uppercase())?;
        match path.as_str() {
            "INBOX" => Some(&mut self.inbox),
            "SENT" => Some(&mut self.sent),
            "DRAFTS" => Some(&mut self.drafts),
            "ARCHIVE" => Some(&mut self.archive),
            "JUNK" | "SPAM" => Some(&mut self.junk),
            "TRASH" | "DELETED" => Some(&mut self.trash),
            _ => None,
        }
    }

    fn take_message_from_any(
        &mut self,
        account_id: u32,
        message_id: u32,
        rfc_message_id: Option<&str>,
        preferred_folder_path: Option<&str>,
    ) -> Option<MessageSummary> {
        let preferred_fid = preferred_folder_path
            .and_then(|p| self.resolve_folder_by_path(account_id, p))
            .map(|(fid, _)| fid);
        let mut order: Vec<u32> = self
            .base_folders
            .iter()
            .filter(|f| f.account_id == account_id)
            .map(|f| f.id)
            .collect();
        if order.is_empty() {
            order = vec![
                DEMO_INBOX_ID,
                DEMO_SENT_ID,
                DEMO_DRAFTS_ID,
                DEMO_ARCHIVE_ID,
                DEMO_JUNK_ID,
                DEMO_TRASH_ID,
            ];
        }
        if let Some(fid) = preferred_fid {
            order.retain(|&id| id != fid);
            order.insert(0, fid);
        }
        for fid in order {
            if let Some(list) = self.slice_mut_for_folder_id(fid) {
                if let Some(idx) = list.iter().position(|m| {
                    m.account_id == account_id
                        && (m.id == message_id
                            || rfc_message_id
                                .is_some_and(|rfc| format!("<demo-{}@hylki.example>", m.id) == rfc))
                }) {
                    return Some(list.remove(idx));
                }
            }
        }
        None
    }

    fn move_message_to(
        &mut self,
        account_id: u32,
        message_id: u32,
        target_folder_path: Option<&str>,
    ) -> Result<(String, String, String, Option<u32>), IpcError> {
        let Some(existing) = self.find_summary(account_id, message_id).cloned() else {
            return Err(IpcError::message_not_found(message_id, None));
        };
        let rfc_message_id = format!("<demo-{message_id}@hylki.example>");
        let original_folder_path = existing.folder_path.clone();

        let Some(dest_raw) = target_folder_path else {
            let _ = self.take_message_from_any(account_id, message_id, None, None);
            return Ok((
                rfc_message_id,
                original_folder_path,
                String::new(),
                Some(existing.folder_id),
            ));
        };

        let Some((dest_fid, dest_path)) = self.resolve_folder_by_path(account_id, dest_raw) else {
            return Err(IpcError::invalid_input(
                format!("无效的目标文件夹: {dest_raw}"),
                None,
            ));
        };
        if dest_path.eq_ignore_ascii_case(&original_folder_path) {
            return Ok((
                rfc_message_id,
                original_folder_path,
                dest_path,
                Some(dest_fid),
            ));
        }

        let mut msg = self
            .take_message_from_any(account_id, message_id, None, Some(&original_folder_path))
            .ok_or_else(|| IpcError::message_not_found(message_id, None))?;
        msg.folder_id = dest_fid;
        msg.folder_path = dest_path.clone();
        if let Some(dest_list) = self.slice_mut_for_folder_id(dest_fid) {
            dest_list.push(msg);
        }
        Ok((
            rfc_message_id,
            original_folder_path,
            dest_path,
            Some(dest_fid),
        ))
    }

    fn undo_move(
        &mut self,
        account_id: u32,
        message_id: u32,
        rfc_message_id: &str,
        current_folder_path: &str,
        original_folder_path: &str,
    ) -> Result<(bool, Option<u32>), IpcError> {
        let Some((orig_fid, orig_path)) =
            self.resolve_folder_by_path(account_id, original_folder_path)
        else {
            return Err(IpcError::invalid_input(
                format!("无效的原文件夹: {original_folder_path}"),
                None,
            ));
        };
        let Some(mut msg) = self.take_message_from_any(
            account_id,
            message_id,
            Some(rfc_message_id),
            Some(current_folder_path),
        ) else {
            return Ok((false, None));
        };
        msg.folder_id = orig_fid;
        msg.folder_path = orig_path;
        if let Some(dest_list) = self.slice_mut_for_folder_id(orig_fid) {
            dest_list.push(msg);
        }
        Ok((true, Some(orig_fid)))
    }
}

struct InFlightBodyEntry {
    generation: u64,
    reader_session_id: String,
    appearance: ReaderAppearance,
    reader_enabled: bool,
}

#[derive(Default)]
pub(crate) struct LiveWaiters {
    pub(crate) draft_saved: VecDeque<mpsc::Sender<Result<u32, String>>>,
    pub(crate) send_enqueued: HashMap<u64, mpsc::Sender<Result<(u32, Option<i64>, bool), String>>>,
    pub(crate) undo_result: HashMap<u64, mpsc::Sender<Result<UndoOutboxOutcome, String>>>,
    pub(crate) outbox_deleted: HashMap<u32, mpsc::Sender<Result<bool, String>>>,
    pub(crate) verify_account: VecDeque<mpsc::Sender<VerifyAccountOutput>>,
    pub(crate) save_account: HashMap<u32, mpsc::Sender<Result<AccountConfig, String>>>,
    pub(crate) delete_account: HashMap<u32, mpsc::Sender<()>>,
    pub(crate) set_account_enabled: VecDeque<mpsc::Sender<()>>,
    pub(crate) attachment_bytes:
        HashMap<u32, mpsc::Sender<Result<(String, String, Vec<u8>), String>>>,
    pub(crate) attachment_saved: VecDeque<mpsc::Sender<Result<PathBuf, String>>>,
}

pub(crate) struct LiveSessionState {
    pub(crate) data_dir: PathBuf,
    pub(crate) worker: Option<Arc<dyn LiveWorkerSink>>,
    pub(crate) account_configs: Vec<AccountConfig>,
    pub(crate) accounts: Vec<AccountSummaryDto>,
    pub(crate) folders_by_account: HashMap<u32, Vec<FolderSummaryDto>>,
    pub(crate) drafts_cache: Vec<LocalDraft>,
    pub(crate) outbox_cache: HashMap<u32, Vec<OutboxItem>>,
    pub(crate) known_has_password: HashMap<u32, bool>,
    pub(crate) next_undo_token: u64,
    pub(crate) waiters: LiveWaiters,
    pub(crate) cache_ready: bool,
    pub(crate) snapshots_hydrated: bool,
    pub(crate) syncing_accounts: HashSet<u32>,
    pub(crate) synced_on_open_folders: HashSet<(u32, u32)>,
    pub(crate) mock_send_idempotency: HashMap<(u32, String), (u32, u64, Option<i64>)>,
    in_flight_bodies: HashMap<(u32, u32), InFlightBodyEntry>,
    in_flight_gen: u64,
    pub(crate) body_cache: HashMap<(u32, u32), MessageDetail>,
    body_cache_order: VecDeque<(u32, u32)>,
    active_reader_target: Option<(String, u32, u32)>,
    pending_mark_thread_seen: Option<u32>,
}

impl LiveSessionState {
    pub(crate) fn empty() -> Self {
        Self {
            data_dir: PathBuf::new(),
            worker: None,
            account_configs: Vec::new(),
            accounts: Vec::new(),
            folders_by_account: HashMap::new(),
            drafts_cache: Vec::new(),
            outbox_cache: HashMap::new(),
            known_has_password: HashMap::new(),
            next_undo_token: 1,
            waiters: LiveWaiters::default(),
            cache_ready: false,
            snapshots_hydrated: false,
            syncing_accounts: HashSet::new(),
            synced_on_open_folders: HashSet::new(),
            mock_send_idempotency: HashMap::new(),
            in_flight_bodies: HashMap::new(),
            in_flight_gen: 0,
            body_cache: HashMap::new(),
            body_cache_order: VecDeque::new(),
            active_reader_target: None,
            pending_mark_thread_seen: None,
        }
    }

    pub(crate) fn combined_outbox_dtos(&self) -> Vec<OutboxItemDto> {
        let mut items: Vec<&OutboxItem> = self.outbox_cache.values().flatten().collect();
        items.sort_by(|a, b| b.id.cmp(&a.id));
        items.into_iter().map(OutboxItemDto::from).collect()
    }

    pub(crate) fn refresh_account_outbox_from_db(&mut self, account_id: u32) {
        if config::global_db_in(&self.data_dir).exists() {
            if let Ok(cache) = CacheStore::open_for_data_dir_read_only(&self.data_dir) {
                if let Ok(items) = cache.list_outbox(account_id) {
                    self.outbox_cache.insert(account_id, items);
                }
            }
        }
    }

    pub(crate) fn refresh_drafts_from_db(&mut self) {
        if config::global_db_in(&self.data_dir).exists() {
            if let Ok(cache) = CacheStore::open_for_data_dir_read_only(&self.data_dir) {
                if let Ok(drafts) = cache.list_drafts() {
                    self.drafts_cache = drafts;
                }
            }
        }
    }

    fn all_folders(&self) -> Vec<FolderSummaryDto> {
        let mut out = Vec::new();
        for acc in &self.account_configs {
            if let Some(list) = self.folders_by_account.get(&acc.id) {
                out.extend(list.iter().cloned());
            }
        }
        for (&acc_id, list) in &self.folders_by_account {
            if !self.account_configs.iter().any(|a| a.id == acc_id) {
                out.extend(list.iter().cloned());
            }
        }
        out
    }

    fn enabled_account_ids(&self) -> Vec<u32> {
        self.account_configs
            .iter()
            .filter(|a| a.enabled)
            .map(|a| a.id)
            .collect()
    }

    fn unified_account_ids(&self) -> Vec<u32> {
        self.account_configs
            .iter()
            .filter(|a| a.enabled && a.in_unified)
            .map(|a| a.id)
            .collect()
    }

    fn cache_detail(&mut self, detail: MessageDetail) {
        let key = (detail.account_id, detail.id);
        if self.body_cache.contains_key(&key) {
            self.body_cache.insert(key, detail);
            return;
        }
        if self.body_cache_order.len() >= BODY_CACHE_CAPACITY {
            if let Some(evicted) = self.body_cache_order.pop_front() {
                self.body_cache.remove(&evicted);
            }
        }
        self.body_cache_order.push_back(key);
        self.body_cache.insert(key, detail);
    }

    fn remove_cached_detail(&mut self, account_id: u32, message_id: u32) {
        let key = (account_id, message_id);
        self.body_cache.remove(&key);
        self.body_cache_order.retain(|k| *k != key);
        if self
            .active_reader_target
            .as_ref()
            .is_some_and(|(_, acc, msg)| *acc == account_id && *msg == message_id)
        {
            self.active_reader_target = None;
        }
    }

    fn clear_sensitive_caches(&mut self) {
        self.in_flight_bodies.clear();
        self.body_cache.clear();
        self.body_cache_order.clear();
        self.active_reader_target = None;
        self.pending_mark_thread_seen = None;
    }

    fn total_inbox_unread(&self) -> u32 {
        self.folders_by_account
            .values()
            .flatten()
            .filter(|f| f.path.eq_ignore_ascii_case("INBOX") || f.name.contains("收件箱"))
            .map(|f| f.unread_count)
            .sum()
    }
}

pub(crate) struct AppLockState {
    pub(crate) passcode_hash: Option<String>,
    pub(crate) locked: bool,
    pub(crate) locked_reason: Option<String>,
    pub(crate) persist_dir: Option<PathBuf>,
    pub(crate) lock_on_startup: bool,
    pub(crate) persist_keyring: bool,
    pub(crate) failed_attempts: u32,
    pub(crate) cooldown_until: Option<Instant>,
    pub(crate) tracker: AutoLockTracker,
}

impl AppLockState {
    fn new(passcode_hash: Option<String>, locked: bool, persist_dir: Option<PathBuf>) -> Self {
        let lock_on_startup = persist_dir
            .as_deref()
            .is_some_and(crate::preferences::is_startup_lock_enabled_in)
            && passcode_hash.is_some();
        Self {
            passcode_hash,
            locked,
            locked_reason: locked.then(|| "startup".to_string()),
            persist_dir,
            lock_on_startup,
            persist_keyring: false,
            failed_attempts: 0,
            cooldown_until: None,
            tracker: AutoLockTracker::new(Instant::now()),
        }
    }

    fn has_passcode(&self) -> bool {
        self.passcode_hash.is_some()
    }
}

fn to_quick_filters(filter: MessageFilterDto) -> QuickFilterSet {
    QuickFilterSet {
        unread: filter.unread.unwrap_or(false),
        starred: filter.starred.unwrap_or(false),
        has_attachment: filter.has_attachment.unwrap_or(false),
        today: filter.today.unwrap_or(false),
        to_me: filter.to_me.unwrap_or(false),
        from_contacts: filter.from_contacts.unwrap_or(false),
        high_importance: filter.high_importance.unwrap_or(false),
        tagged: filter.tagged.unwrap_or(false),
        date_from_ts: filter.date_from_ts,
        date_to_ts: filter.date_to_ts,
    }
}

fn matches_quick_filters(m: &MessageSummary, filters: QuickFilterSet, today_start_ts: i64) -> bool {
    (!filters.unread || m.unread)
        && (!filters.starred || m.flagged)
        && (!filters.has_attachment || m.has_attachment)
        && (!filters.today || m.date_ts >= today_start_ts)
        && (!filters.to_me || !m.recipients.trim().is_empty())
        && (!filters.high_importance || m.importance > 0)
        && (!filters.tagged || !m.tags.is_empty())
        && filters.date_from_ts.is_none_or(|from| m.date_ts >= from)
        && filters.date_to_ts.is_none_or(|to| m.date_ts < to)
}

fn sort_summary_refs(items: &mut [&MessageSummary], sort: MessageSort) {
    match sort {
        MessageSort::DateNewest => {
            items.sort_by(|a, b| b.date_ts.cmp(&a.date_ts).then_with(|| b.uid.cmp(&a.uid)));
        }
        MessageSort::DateOldest => {
            items.sort_by(|a, b| a.date_ts.cmp(&b.date_ts).then_with(|| a.uid.cmp(&b.uid)));
        }
        MessageSort::SenderAsc => {
            items.sort_by(|a, b| {
                a.sender
                    .to_lowercase()
                    .cmp(&b.sender.to_lowercase())
                    .then_with(|| b.date_ts.cmp(&a.date_ts))
            });
        }
        MessageSort::SenderDesc => {
            items.sort_by(|a, b| {
                b.sender
                    .to_lowercase()
                    .cmp(&a.sender.to_lowercase())
                    .then_with(|| b.date_ts.cmp(&a.date_ts))
            });
        }
        MessageSort::SubjectAsc => {
            items.sort_by(|a, b| {
                a.subject
                    .to_lowercase()
                    .cmp(&b.subject.to_lowercase())
                    .then_with(|| b.date_ts.cmp(&a.date_ts))
            });
        }
        MessageSort::SubjectDesc => {
            items.sort_by(|a, b| {
                b.subject
                    .to_lowercase()
                    .cmp(&a.subject.to_lowercase())
                    .then_with(|| b.date_ts.cmp(&a.date_ts))
            });
        }
        MessageSort::UnreadFirst => {
            items.sort_by(|a, b| {
                b.unread
                    .cmp(&a.unread)
                    .then_with(|| b.date_ts.cmp(&a.date_ts))
            });
        }
        MessageSort::StarredFirst => {
            items.sort_by(|a, b| {
                b.flagged
                    .cmp(&a.flagged)
                    .then_with(|| b.date_ts.cmp(&a.date_ts))
            });
        }
        MessageSort::AttachmentFirst => {
            items.sort_by(|a, b| {
                b.has_attachment
                    .cmp(&a.has_attachment)
                    .then_with(|| b.date_ts.cmp(&a.date_ts))
            });
        }
        MessageSort::ImportanceFirst => {
            items.sort_by(|a, b| {
                b.importance
                    .cmp(&a.importance)
                    .then_with(|| b.date_ts.cmp(&a.date_ts))
            });
        }
        MessageSort::AccountAsc => {
            items.sort_by(|a, b| {
                a.account_id
                    .cmp(&b.account_id)
                    .then_with(|| b.date_ts.cmp(&a.date_ts))
            });
        }
        MessageSort::AccountDesc => {
            items.sort_by(|a, b| {
                b.account_id
                    .cmp(&a.account_id)
                    .then_with(|| b.date_ts.cmp(&a.date_ts))
            });
        }
        MessageSort::TagsAsc => {
            items.sort_by(|a, b| {
                a.tags
                    .join(",")
                    .to_lowercase()
                    .cmp(&b.tags.join(",").to_lowercase())
                    .then_with(|| b.date_ts.cmp(&a.date_ts))
            });
        }
    }
}

/// Collect unread thread targets in the same folder (`(id, uid, folder_path)`).
pub fn collect_same_folder_unread_thread_targets(
    detail: &MessageDetail,
) -> Vec<(u32, u32, String)> {
    if detail.folder_path.is_empty() {
        return Vec::new();
    }
    detail
        .thread_messages
        .iter()
        .filter(|tm| {
            tm.id != detail.id
                && tm.unread
                && tm.folder_path.eq_ignore_ascii_case(&detail.folder_path)
        })
        .map(|tm| (tm.id, tm.uid, tm.folder_path.clone()))
        .collect()
}

fn resolve_initial_live_view(
    prefs: &kitemail_mail_core::config::AppPreferences,
    accounts: &[AccountConfig],
    folders_by_account: &HashMap<u32, Vec<FolderSummaryDto>>,
    cache_ready: bool,
) -> MainViewDto {
    if prefs.default_view != "last" {
        return MainViewDto::UnifiedInbox;
    }
    if let Some((account_id, folder_id)) = prefs.last_folder {
        let account_ok = accounts.iter().any(|a| a.id == account_id && a.enabled);
        if account_ok {
            if !cache_ready {
                return MainViewDto::AccountFolder {
                    account_id,
                    folder_id,
                };
            }
            let folder_ok = folders_by_account
                .get(&account_id)
                .is_some_and(|folders| folders.iter().any(|f| f.id == folder_id));
            if folder_ok {
                return MainViewDto::AccountFolder {
                    account_id,
                    folder_id,
                };
            }
        }
        return MainViewDto::UnifiedInbox;
    }
    view_from_last_str(prefs.last_view.as_deref()).unwrap_or(MainViewDto::UnifiedInbox)
}

fn html_has_remote_images(html: &str) -> bool {
    strip_remote_images_for_webview(html).1 > 0
}

pub(crate) fn is_message_remote_images_blocked(
    detail: &MessageDetail,
    raw_prefs: &AppPreferences,
    allowed_msgs: &HashSet<(u32, u32)>,
) -> bool {
    if !raw_prefs.block_remote_images {
        return false;
    }
    if detail.thread_messages.len() > 1 {
        return detail.thread_messages.iter().any(|tm| {
            let allowed = raw_prefs.is_remote_sender_allowed(&tm.sender)
                || allowed_msgs.contains(&(detail.account_id, tm.id))
                || allowed_msgs.contains(&(detail.account_id, detail.id));
            !allowed && tm.body_html.as_deref().is_some_and(html_has_remote_images)
        });
    }
    let allowed = raw_prefs.is_remote_sender_allowed(&detail.sender)
        || allowed_msgs.contains(&(detail.account_id, detail.id));
    !allowed
        && detail
            .body_html
            .as_deref()
            .is_some_and(html_has_remote_images)
}

fn compute_default_collapsed_cards(
    detail: &MessageDetail,
    thread_newest_first: bool,
    fold_mode: &str,
) -> HashSet<u32> {
    if fold_mode == "never" || detail.thread_messages.len() < 2 {
        return HashSet::new();
    }
    let newest_id = if thread_newest_first {
        detail.thread_messages.first().map(|m| m.id)
    } else {
        detail.thread_messages.last().map(|m| m.id)
    };
    detail
        .thread_messages
        .iter()
        .filter(|m| {
            let keep_open = m.id == detail.id || Some(m.id) == newest_id;
            if keep_open {
                return false;
            }
            if fold_mode == "read" {
                !m.unread
            } else {
                true
            }
        })
        .map(|m| m.id)
        .collect()
}

pub(crate) fn build_open_message_output(
    request_id: String,
    reader_session_id: String,
    detail: &MessageDetail,
    body_status: BodyStatusDto,
    include_body: bool,
    remote_images_blocked: Option<bool>,
) -> OpenMessageOutput {
    let attachments = detail
        .attachments
        .iter()
        .map(AttachmentMetaDto::from)
        .collect();
    let thread_messages = detail
        .thread_messages
        .iter()
        .map(|tm| ThreadMessageDto::from_core(detail.account_id, tm))
        .collect();
    let rfc_message_id =
        (!detail.rfc_message_id.trim().is_empty()).then(|| detail.rfc_message_id.clone());
    let folder_path = (!detail.folder_path.trim().is_empty()).then(|| detail.folder_path.clone());

    OpenMessageOutput {
        request_id,
        reader_session_id,
        account_id: detail.account_id,
        message_id: detail.id,
        subject: detail.subject.clone(),
        sender: detail.sender.clone(),
        recipients: detail.recipients.clone(),
        cc: detail.cc.clone(),
        date_display: detail.date_display.clone(),
        body_status,
        plain_text: if include_body {
            detail.body_text.clone()
        } else {
            String::new()
        },
        has_html: include_body && (detail.body_html.is_some() || detail.thread_messages.len() > 1),
        html_body: if include_body {
            detail.body_html.clone()
        } else {
            None
        },
        flagged: detail.flagged,
        attachments,
        thread_messages,
        remote_images_blocked,
        rfc_message_id,
        folder_path,
    }
}

pub struct MailSession {
    pub(crate) mode: AppMode,
    pub(crate) locked: AtomicBool,
    pub(crate) quitting: AtomicBool,
    state_version: AtomicU64,
    read_mark_gen: AtomicU64,
    pub(crate) active_selected_msg: Mutex<Option<(u32, u32)>>,
    pub(crate) active_reader_session_id: Mutex<Option<String>>,
    pub(crate) allowed_remote_messages: Mutex<HashSet<(u32, u32)>>,
    pub(crate) per_message_dark: Mutex<HashMap<(u32, u32), bool>>,
    pub(crate) reading_view: AtomicBool,
    thread_collapsed_cards: Mutex<(Option<(u32, u32)>, HashSet<u32>)>,
    thread_raw_cards: Mutex<(Option<(u32, u32)>, HashSet<u32>)>,
    current_view: Mutex<MainViewDto>,
    sync_status: Mutex<SyncStatusDto>,
    pub(crate) lock_state: Mutex<AppLockState>,
    pub(crate) demo: Mutex<DemoSessionState>,
    pub(crate) live: Mutex<LiveSessionState>,
    read_cache: Mutex<Option<CacheStore>>,
    pub(crate) prefs: Arc<PreferencesStore>,
    reader: Arc<ReaderManager>,
    pub(crate) emitter: Arc<dyn MainEventEmitter>,
    pub(crate) self_weak: Mutex<Weak<MailSession>>,
    tray: Mutex<Option<Arc<crate::tray::TrayHandle>>>,
    auto_sync_signal: Arc<(Mutex<bool>, Condvar)>,
    last_auto_sync_at: Mutex<Instant>,
    body_fetch_timeout: Duration,
    read_mark_delay: Duration,
}

impl MailSession {
    pub fn new(
        mode: AppMode,
        prefs: Arc<PreferencesStore>,
        reader: Arc<ReaderManager>,
        emitter: Arc<dyn MainEventEmitter>,
    ) -> Self {
        let sync_status = match mode {
            AppMode::Demo => SyncStatusDto {
                syncing: false,
                message: None,
                last_sync_at: Some(chrono::Utc::now().timestamp()),
            },
            AppMode::Live => SyncStatusDto {
                syncing: false,
                message: None,
                last_sync_at: None,
            },
        };
        Self {
            mode,
            locked: AtomicBool::new(false),
            quitting: AtomicBool::new(false),
            state_version: AtomicU64::new(1),
            read_mark_gen: AtomicU64::new(0),
            active_selected_msg: Mutex::new(None),
            active_reader_session_id: Mutex::new(None),
            allowed_remote_messages: Mutex::new(HashSet::new()),
            per_message_dark: Mutex::new(HashMap::new()),
            reading_view: AtomicBool::new(false),
            thread_collapsed_cards: Mutex::new((None, HashSet::new())),
            thread_raw_cards: Mutex::new((None, HashSet::new())),
            current_view: Mutex::new(MainViewDto::UnifiedInbox),
            sync_status: Mutex::new(sync_status),
            lock_state: Mutex::new(AppLockState::new(None, false, None)),
            demo: Mutex::new(DemoSessionState::new()),
            live: Mutex::new(LiveSessionState::empty()),
            read_cache: Mutex::new(None),
            prefs,
            reader,
            emitter,
            self_weak: Mutex::new(Weak::new()),
            tray: Mutex::new(None),
            auto_sync_signal: Arc::new((Mutex::new(false), Condvar::new())),
            last_auto_sync_at: Mutex::new(Instant::now()),
            body_fetch_timeout: DEFAULT_BODY_FETCH_TIMEOUT,
            read_mark_delay: DEFAULT_READ_MARK_DELAY,
        }
    }

    /// Build and boot a Live-mode session without blocking startup on heavy SQLite folder/draft/outbox reads.
    pub fn new_live(
        data_dir: PathBuf,
        prefs: Arc<PreferencesStore>,
        reader: Arc<ReaderManager>,
        emitter: Arc<dyn MainEventEmitter>,
    ) -> Arc<Self> {
        let total_started = Instant::now();
        let accounts_started = Instant::now();
        let accounts = config::load_accounts_in(&data_dir).unwrap_or_default();
        let account_dtos: Vec<AccountSummaryDto> =
            accounts.iter().map(AccountSummaryDto::from).collect();
        let accounts_ms = accounts_started.elapsed().as_secs_f64() * 1000.0;

        let lock_started = Instant::now();
        let (passcode_hash, initial_locked) = match secrets::load_app_lock_hash() {
            Ok(Some(hash)) => {
                let persisted = app_lock::is_lock_persisted_in(&data_dir)
                    || crate::preferences::is_startup_lock_enabled_in(&data_dir);
                (Some(hash), persisted)
            }
            Ok(None) => {
                let _ = app_lock::set_lock_persisted_in(&data_dir, false);
                let _ = crate::preferences::set_startup_lock_enabled_in(&data_dir, false);
                (None, false)
            }
            Err(_) => (None, false),
        };
        if initial_locked {
            reader.set_locked(true);
        }
        let lock_ms = lock_started.elapsed().as_secs_f64() * 1000.0;

        // Defer folder/draft/outbox snapshot reads off the window reveal critical path;
        // `bootstrap()` hydrates folders once lock state is confirmed unlocked, while
        // the background worker warms up drafts/outbox/first page asynchronously.
        let folders_by_account: HashMap<u32, Vec<FolderSummaryDto>> = HashMap::new();
        let raw_prefs = prefs.snapshot_raw();
        let initial_view =
            resolve_initial_live_view(&raw_prefs, &accounts, &folders_by_account, false);

        let enabled_ids: HashSet<u32> = accounts
            .iter()
            .filter(|a| a.enabled)
            .map(|a| a.id)
            .collect();
        let initial_syncing = !enabled_ids.is_empty() && !initial_locked;
        let sync_status = SyncStatusDto {
            syncing: initial_syncing,
            message: if initial_syncing {
                Some("正在同步邮箱...".to_string())
            } else {
                None
            },
            last_sync_at: None,
        };

        let mut live_state = LiveSessionState::empty();
        live_state.data_dir = data_dir.clone();
        live_state.account_configs = accounts;
        live_state.accounts = account_dtos;
        live_state.folders_by_account = folders_by_account;
        live_state.cache_ready = false;
        live_state.snapshots_hydrated = false;
        live_state.syncing_accounts = if initial_locked {
            HashSet::new()
        } else {
            enabled_ids
        };

        let session = Arc::new(Self {
            mode: AppMode::Live,
            locked: AtomicBool::new(initial_locked),
            quitting: AtomicBool::new(false),
            state_version: AtomicU64::new(1),
            read_mark_gen: AtomicU64::new(0),
            active_selected_msg: Mutex::new(None),
            active_reader_session_id: Mutex::new(None),
            allowed_remote_messages: Mutex::new(HashSet::new()),
            per_message_dark: Mutex::new(HashMap::new()),
            reading_view: AtomicBool::new(false),
            thread_collapsed_cards: Mutex::new((None, HashSet::new())),
            thread_raw_cards: Mutex::new((None, HashSet::new())),
            current_view: Mutex::new(initial_view),
            sync_status: Mutex::new(sync_status),
            lock_state: Mutex::new({
                let mut ls =
                    AppLockState::new(passcode_hash, initial_locked, Some(data_dir.clone()));
                ls.persist_keyring = true;
                ls
            }),
            demo: Mutex::new(DemoSessionState::new()),
            live: Mutex::new(live_state),
            read_cache: Mutex::new(None),
            prefs,
            reader,
            emitter,
            self_weak: Mutex::new(Weak::new()),
            tray: Mutex::new(None),
            auto_sync_signal: Arc::new((Mutex::new(false), Condvar::new())),
            last_auto_sync_at: Mutex::new(Instant::now()),
            body_fetch_timeout: DEFAULT_BODY_FETCH_TIMEOUT,
            read_mark_delay: DEFAULT_READ_MARK_DELAY,
        });
        session.bind_self(&session);
        let worker_started = Instant::now();
        session.spawn_live_worker(data_dir);
        let worker_ms = worker_started.elapsed().as_secs_f64() * 1000.0;
        eprintln!(
            "[startup] new_live: locked={}, accounts_ms={accounts_ms:.2}, lock_ms={lock_ms:.2}, worker_spawn_ms={worker_ms:.2}, total_ms={:.2}",
            initial_locked,
            total_started.elapsed().as_secs_f64() * 1000.0
        );
        session
    }

    pub fn bind_self(&self, arc: &Arc<MailSession>) {
        *self.self_weak.lock().expect("self_weak mutex poisoned") = Arc::downgrade(arc);
    }

    pub fn attach_tray(&self, tray: Arc<crate::tray::TrayHandle>) {
        tray.set_lock_enabled(self.has_passcode());
        let unread = self.total_unread_count();
        tray.set_tooltip(crate::tray::format_tray_tooltip(unread));
        *self.tray.lock().expect("tray mutex poisoned") = Some(tray);
    }

    fn spawn_live_worker(&self, data_dir: PathBuf) {
        let (notify_tx, notify_rx) = mpsc::channel::<()>();
        let worker_handle = match MailWorkerHandle::spawn_with_notifier(
            data_dir,
            Some(Arc::new(move || {
                let _ = notify_tx.send(());
            })),
        ) {
            Ok(w) => w,
            Err(err) => {
                let mut status = self.sync_status.lock().expect("sync_status mutex poisoned");
                status.syncing = false;
                status.message = Some(format!("启动邮件后台线程失败: {err}"));
                return;
            }
        };

        let (sender, events_rx) = worker_handle.split();
        let worker = Arc::new(RealMailWorkerSink { sender });

        let has_accounts = {
            let mut live = self.live.lock().expect("live mutex poisoned");
            live.worker = Some(worker.clone());
            !live.account_configs.is_empty()
        };

        if has_accounts && !self.is_locked() {
            // Startup defers local SQLite snapshots off the critical path; warm them up in background
            // once lock state is confirmed unlocked.
            worker.send(MailRequest::LoadAllCached);
            worker.send(MailRequest::SyncAllAccounts {
                user_initiated: false,
            });
        }

        let weak_session = self
            .self_weak
            .lock()
            .expect("self_weak mutex poisoned")
            .clone();
        let weak_for_events = weak_session.clone();
        let _ = std::thread::Builder::new()
            .name("kitemail-tauri-worker-events".to_string())
            .spawn(move || loop {
                let events = events_rx.drain_events();
                if !events.is_empty() {
                    if let Some(session) = weak_for_events.upgrade() {
                        session.process_worker_events(events);
                    } else {
                        break;
                    }
                    continue;
                }
                if notify_rx.recv().is_err() {
                    break;
                }
            });

        let auto_sync_signal = Arc::clone(&self.auto_sync_signal);
        let _ = std::thread::Builder::new()
            .name("kitemail-tauri-auto-sync".to_string())
            .spawn(move || loop {
                let (lock, cvar) = &*auto_sync_signal;
                let guard = lock.lock().expect("auto_sync_signal mutex poisoned");
                if *guard {
                    break;
                }
                let (guard, _) = cvar
                    .wait_timeout(guard, AUTO_SYNC_CHECK_INTERVAL)
                    .expect("auto_sync_signal mutex poisoned");
                if *guard {
                    break;
                }
                drop(guard);

                let Some(session) = weak_session.upgrade() else {
                    break;
                };
                if session.quitting.load(Ordering::SeqCst) {
                    break;
                }
                session.try_auto_sync_at(Instant::now());
            });
    }

    pub fn try_auto_sync_at(&self, now: Instant) -> bool {
        if self.mode != AppMode::Live || self.quitting.load(Ordering::SeqCst) || self.is_locked() {
            return false;
        }
        let minutes = self.prefs.snapshot_raw().auto_sync_minutes;
        if minutes == 0 {
            *self
                .last_auto_sync_at
                .lock()
                .expect("last_auto_sync_at mutex poisoned") = now;
            return false;
        }
        let interval = Duration::from_secs(u64::from(minutes) * 60);
        {
            let mut last = self
                .last_auto_sync_at
                .lock()
                .expect("last_auto_sync_at mutex poisoned");
            if now.saturating_duration_since(*last) < interval {
                return false;
            }
            *last = now;
        }

        let (worker, has_enabled) = {
            let live = self.live.lock().expect("live mutex poisoned");
            (live.worker.clone(), !live.enabled_account_ids().is_empty())
        };
        if !has_enabled {
            return false;
        }
        if let Some(w) = worker {
            w.send(MailRequest::SyncAllAccounts {
                user_initiated: false,
            });
            return true;
        }
        false
    }

    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::SeqCst)
    }

    pub fn has_passcode(&self) -> bool {
        self.lock_state
            .lock()
            .expect("lock_state mutex poisoned")
            .has_passcode()
    }

    pub(crate) fn is_lock_on_startup(&self) -> bool {
        let guard = self.lock_state.lock().expect("lock_state mutex poisoned");
        guard.has_passcode() && guard.lock_on_startup
    }

    pub(crate) fn set_lock_on_startup(&self, enabled: bool) -> Result<bool, IpcError> {
        let mut guard = self.lock_state.lock().expect("lock_state mutex poisoned");
        if enabled && !guard.has_passcode() {
            return Err(IpcError::invalid_input(
                "请先设置锁屏口令后再开启启动锁定",
                None,
            ));
        }
        let next = guard.has_passcode() && enabled;
        if let Some(ref dir) = guard.persist_dir {
            crate::preferences::set_startup_lock_enabled_in(dir, next)
                .map_err(|e| IpcError::internal(format!("保存启动锁配置失败: {e}"), None))?;
        }
        guard.lock_on_startup = next;
        Ok(next)
    }

    pub(crate) fn reset_read_cache(&self) {
        if let Ok(mut guard) = self.read_cache.lock() {
            *guard = None;
        }
    }

    pub(crate) fn clear_live_body_cache(&self) {
        if self.mode == AppMode::Live {
            let mut live = self.live.lock().expect("live mutex poisoned");
            live.body_cache.clear();
            live.body_cache_order.clear();
            drop(live);
            self.reset_read_cache();
        }
    }

    pub(crate) fn refresh_tray_lock_enabled(&self) {
        if let Some(ref tray) = *self.tray.lock().expect("tray mutex poisoned") {
            tray.set_lock_enabled(self.has_passcode());
        }
    }

    pub fn current_view(&self) -> MainViewDto {
        *self
            .current_view
            .lock()
            .expect("current_view mutex poisoned")
    }

    pub fn state_version(&self) -> u64 {
        self.state_version.load(Ordering::SeqCst)
    }

    pub(crate) fn bump_state_version(&self) -> u64 {
        self.state_version.fetch_add(1, Ordering::SeqCst) + 1
    }

    fn total_unread_count(&self) -> u32 {
        match self.mode {
            AppMode::Demo => {
                let demo = self.demo.lock().expect("demo mutex poisoned");
                demo.folders()
                    .iter()
                    .filter(|f| f.path.eq_ignore_ascii_case("INBOX"))
                    .map(|f| f.unread_count)
                    .sum()
            }
            AppMode::Live => {
                let live = self.live.lock().expect("live mutex poisoned");
                live.total_inbox_unread()
            }
        }
    }

    pub(crate) fn refresh_tray_tooltip(&self) {
        if let Some(ref tray) = *self.tray.lock().expect("tray mutex poisoned") {
            tray.set_tooltip(crate::tray::format_tray_tooltip(self.total_unread_count()));
        }
    }

    pub fn bootstrap(&self) -> MainBootstrapDto {
        let total_started = Instant::now();
        let locked = self.is_locked();
        let has_passcode = self.has_passcode();
        let state_version = self.state_version();
        let mut current_view = self.current_view();
        let preferences = self.prefs.snapshot_dto();
        let sync_status = self
            .sync_status
            .lock()
            .expect("sync_status mutex poisoned")
            .clone();

        if locked {
            eprintln!(
                "[startup] session.bootstrap: locked=true elapsed_ms={:.2}",
                total_started.elapsed().as_secs_f64() * 1000.0
            );
            return MainBootstrapDto {
                mode: self.mode,
                locked: true,
                has_passcode,
                cache_ready: false,
                state_version,
                accounts: Vec::new(),
                folders: Vec::new(),
                current_view,
                preferences,
                sync_status,
                tags: Vec::new(),
                templates: Vec::new(),
            };
        }

        match self.mode {
            AppMode::Demo => {
                let demo = self.demo.lock().expect("demo mutex poisoned");
                let dto = MainBootstrapDto {
                    mode: self.mode,
                    locked: false,
                    has_passcode,
                    cache_ready: true,
                    state_version,
                    accounts: demo.accounts.clone(),
                    folders: demo.folders(),
                    current_view,
                    preferences,
                    sync_status,
                    tags: Self::demo_tag_catalog(),
                    templates: Vec::new(),
                };
                eprintln!(
                    "[startup] session.bootstrap: mode=demo elapsed_ms={:.2}",
                    total_started.elapsed().as_secs_f64() * 1000.0
                );
                dto
            }
            AppMode::Live => {
                let (data_dir, account_configs, needs_folder_hydration) = {
                    let live = self.live.lock().expect("live mutex poisoned");
                    (
                        live.data_dir.clone(),
                        live.account_configs.clone(),
                        !live.snapshots_hydrated,
                    )
                };

                let mut folders_ms = 0.0;
                let mut tags_templates_ms = 0.0;
                let mut hydrated_folders: Option<HashMap<u32, Vec<FolderSummaryDto>>> = None;
                let (tags, templates) = if config::global_db_in(&data_dir).exists() {
                    if let Ok(cache) = CacheStore::open_for_data_dir_read_only(&data_dir) {
                        if needs_folder_hydration {
                            let f_started = Instant::now();
                            let mut map = HashMap::new();
                            for acc in &account_configs {
                                if let Ok(folders) = cache.list_folders(acc.id) {
                                    map.insert(
                                        acc.id,
                                        folders.iter().map(FolderSummaryDto::from).collect(),
                                    );
                                }
                            }
                            hydrated_folders = Some(map);
                            folders_ms = f_started.elapsed().as_secs_f64() * 1000.0;
                        }
                        let tt_started = Instant::now();
                        let tags = cache
                            .list_tags()
                            .unwrap_or_default()
                            .iter()
                            .map(TagItemDto::from)
                            .collect();
                        let templates = cache
                            .list_templates()
                            .unwrap_or_default()
                            .iter()
                            .map(MailTemplateDto::from)
                            .collect();
                        tags_templates_ms = tt_started.elapsed().as_secs_f64() * 1000.0;
                        (tags, templates)
                    } else {
                        (Vec::new(), Vec::new())
                    }
                } else {
                    (Vec::new(), Vec::new())
                };

                let (cache_ready, accounts, folders) = {
                    let mut live = self.live.lock().expect("live mutex poisoned");
                    if let Some(map) = hydrated_folders {
                        for (acc_id, list) in map {
                            live.folders_by_account.entry(acc_id).or_insert(list);
                        }
                        live.cache_ready = true;
                        live.snapshots_hydrated = true;
                        let raw_prefs = self.prefs.snapshot_raw();
                        let validated_view = resolve_initial_live_view(
                            &raw_prefs,
                            &live.account_configs,
                            &live.folders_by_account,
                            true,
                        );
                        *self
                            .current_view
                            .lock()
                            .expect("current_view mutex poisoned") = validated_view;
                        current_view = validated_view;
                    }
                    (live.cache_ready, live.accounts.clone(), live.all_folders())
                };
                eprintln!(
                    "[startup] session.bootstrap: mode=live cache_ready={} folders={} folders_ms={folders_ms:.2} tags_templates_ms={tags_templates_ms:.2} total_ms={:.2}",
                    cache_ready,
                    folders.len(),
                    total_started.elapsed().as_secs_f64() * 1000.0
                );
                MainBootstrapDto {
                    mode: self.mode,
                    locked: false,
                    has_passcode,
                    cache_ready,
                    state_version,
                    accounts,
                    folders,
                    current_view,
                    preferences,
                    sync_status,
                    tags,
                    templates,
                }
            }
        }
    }

    pub fn list_messages(&self, input: &ListMessagesInput) -> Result<ListMessagesOutput, IpcError> {
        let req_id = Some(input.request_id.clone());
        if self.is_locked() {
            return Err(IpcError::locked(req_id));
        }

        match self.mode {
            AppMode::Demo => {
                let demo = self.demo.lock().expect("demo mutex poisoned");
                let (total, items) = demo.list_messages(input)?;
                *self
                    .current_view
                    .lock()
                    .expect("current_view mutex poisoned") = input.view;
                let offset = input.offset.min(total);
                let has_more = offset.saturating_add(items.len()) < total;
                Ok(ListMessagesOutput {
                    request_id: input.request_id.clone(),
                    state_version: self.state_version(),
                    offset,
                    total,
                    has_more,
                    items,
                })
            }
            AppMode::Live => {
                let page = self.query_live_messages_page(input)?;
                let prev_view = {
                    let mut cur = self
                        .current_view
                        .lock()
                        .expect("current_view mutex poisoned");
                    let prev = *cur;
                    *cur = input.view;
                    prev
                };
                self.prefs.update_last_location(input.view);
                if let MainViewDto::AccountFolder {
                    account_id,
                    folder_id,
                } = input.view
                {
                    // Switching into a specific account folder (or opening a restored non-inbox
                    // folder for the first time after startup) schedules a deduplicated background
                    // sync so Outlook Graph / IMAP subfolders load remote headers immediately.
                    if input.offset == 0 {
                        let (worker, folder_path, first_open_sync) = {
                            let mut live = self.live.lock().expect("live mutex poisoned");
                            let first_open =
                                live.synced_on_open_folders.insert((account_id, folder_id));
                            let path = live
                                .folders_by_account
                                .get(&account_id)
                                .and_then(|fs| fs.iter().find(|f| f.id == folder_id))
                                .map(|f| f.path.clone());
                            (live.worker.clone(), path, first_open)
                        };
                        if prev_view != input.view || first_open_sync {
                            if let (Some(w), Some(path)) = (worker, folder_path) {
                                w.send(MailRequest::SyncFolder {
                                    account_id,
                                    folder_id,
                                    folder_path: path,
                                    user_initiated: false,
                                });
                            }
                        }
                    }
                }
                let offset = input.offset.min(page.total);
                let has_more = offset.saturating_add(page.items.len()) < page.total;
                let items = page.items.iter().map(MessageSummaryDto::from).collect();
                Ok(ListMessagesOutput {
                    request_id: input.request_id.clone(),
                    state_version: self.state_version(),
                    offset,
                    total: page.total,
                    has_more,
                    items,
                })
            }
        }
    }

    fn query_live_messages_page(&self, input: &ListMessagesInput) -> Result<MailPage, IpcError> {
        let req_id = Some(input.request_id.clone());
        let (data_dir, cache_ready, enabled_ids, unified_ids, folders, worker) = {
            let live = self.live.lock().expect("live mutex poisoned");
            (
                live.data_dir.clone(),
                live.cache_ready,
                live.enabled_account_ids(),
                live.unified_account_ids(),
                live.all_folders(),
                live.worker.clone(),
            )
        };

        if let MainViewDto::AccountFolder {
            account_id,
            folder_id,
        } = input.view
        {
            if cache_ready
                && !folders.is_empty()
                && !folders
                    .iter()
                    .any(|f| f.account_id == account_id && f.id == folder_id)
            {
                return Err(IpcError::invalid_input(
                    format!("无效的账号或文件夹: accountId={account_id}, folderId={folder_id}"),
                    req_id,
                ));
            }
        }

        let quick = to_quick_filters(input.filter);
        let sort: MessageSort = input.sort.into();
        let limit = input.limit.min(MAX_PAGE_LIMIT);
        let cross_folder = input.scope == SearchScopeDto::AllFolders
            && (!input.query.trim().is_empty() || !quick.is_empty());
        let scoped_unified_ids = if let Some(acc_id) = input.filter.account_id {
            vec![acc_id]
        } else {
            unified_ids.clone()
        };
        let scoped_enabled_ids = if let Some(acc_id) = input.filter.account_id {
            vec![acc_id]
        } else {
            enabled_ids.clone()
        };

        let worker_query = if input.filter.account_id.is_some() {
            None
        } else if cross_folder {
            Some(PageQueryRequest::SearchAllFolders {
                query: input.query.clone(),
                quick_filters: quick,
                sort,
                offset: input.offset,
                limit,
            })
        } else {
            match input.view {
                MainViewDto::UnifiedInbox => Some(PageQueryRequest::UnifiedInbox {
                    query: input.query.clone(),
                    filter: MessageFilter::All,
                    quick_filters: quick,
                    sort,
                    offset: input.offset,
                    limit,
                }),
                MainViewDto::Unread => Some(PageQueryRequest::UnifiedView {
                    view: UnifiedView::Unread,
                    query: input.query.clone(),
                    filter: MessageFilter::All,
                    quick_filters: quick,
                    sort,
                    offset: input.offset,
                    limit,
                }),
                MainViewDto::Starred => Some(PageQueryRequest::UnifiedView {
                    view: UnifiedView::Starred,
                    query: input.query.clone(),
                    filter: MessageFilter::All,
                    quick_filters: quick,
                    sort,
                    offset: input.offset,
                    limit,
                }),
                MainViewDto::Sent => Some(PageQueryRequest::UnifiedView {
                    view: UnifiedView::Sent,
                    query: input.query.clone(),
                    filter: MessageFilter::All,
                    quick_filters: quick,
                    sort,
                    offset: input.offset,
                    limit,
                }),
                MainViewDto::Archive => Some(PageQueryRequest::UnifiedView {
                    view: UnifiedView::Archive,
                    query: input.query.clone(),
                    filter: MessageFilter::All,
                    quick_filters: quick,
                    sort,
                    offset: input.offset,
                    limit,
                }),
                MainViewDto::Junk => Some(PageQueryRequest::UnifiedView {
                    view: UnifiedView::Junk,
                    query: input.query.clone(),
                    filter: MessageFilter::All,
                    quick_filters: quick,
                    sort,
                    offset: input.offset,
                    limit,
                }),
                MainViewDto::Trash => Some(PageQueryRequest::UnifiedView {
                    view: UnifiedView::Trash,
                    query: input.query.clone(),
                    filter: MessageFilter::All,
                    quick_filters: quick,
                    sort,
                    offset: input.offset,
                    limit,
                }),
                MainViewDto::Snoozed => Some(PageQueryRequest::UnifiedView {
                    view: UnifiedView::Snoozed,
                    query: input.query.clone(),
                    filter: MessageFilter::All,
                    quick_filters: quick,
                    sort,
                    offset: input.offset,
                    limit,
                }),
                MainViewDto::Filters => Some(PageQueryRequest::SearchAllFolders {
                    query: input.query.clone(),
                    quick_filters: quick,
                    sort,
                    offset: input.offset,
                    limit,
                }),
                MainViewDto::Tags => Some(PageQueryRequest::SearchAllFolders {
                    query: input.query.clone(),
                    quick_filters: QuickFilterSet {
                        tagged: true,
                        ..quick
                    },
                    sort,
                    offset: input.offset,
                    limit,
                }),
                MainViewDto::AccountFolder { folder_id, .. } => {
                    Some(PageQueryRequest::MessagesPage {
                        folder_id,
                        query: input.query.clone(),
                        filter: MessageFilter::All,
                        quick_filters: quick,
                        sort,
                        offset: input.offset,
                        limit,
                    })
                }
                MainViewDto::Drafts => None,
            }
        };

        if let (Some(w), Some(page_req)) = (worker.as_ref(), worker_query) {
            if !w.is_async_worker() {
                if let Some(res) = w.query_page(page_req) {
                    return res
                        .map_err(|e| IpcError::internal(format!("查询邮件列表失败: {e}"), req_id));
                }
            }
        }

        if !config::global_db_in(&data_dir).exists() {
            return Ok(MailPage {
                folder_id: 0,
                offset: input.offset,
                total: 0,
                items: Vec::new(),
            });
        }

        let mut cache_guard = self.read_cache.lock().expect("read_cache mutex poisoned");
        if cache_guard.is_none() {
            let opened = CacheStore::open_for_data_dir_read_only(&data_dir).map_err(|e| {
                IpcError::internal(format!("打开本地只读缓存失败: {e}"), req_id.clone())
            })?;
            *cache_guard = Some(opened);
        }

        let run_query = |cache: &CacheStore| {
            execute_live_page_query(
                cache,
                input,
                cross_folder,
                &scoped_enabled_ids,
                &scoped_unified_ids,
                quick,
                sort,
                limit,
            )
        };

        let first = run_query(cache_guard.as_ref().expect("read_cache initialized"));
        let res = match first {
            Ok(page) => Ok(page),
            Err(_) => {
                // Reopen read-only cache once in case worker migration or schema refresh invalidated the handle.
                *cache_guard = None;
                let reopened = CacheStore::open_for_data_dir_read_only(&data_dir).map_err(|e| {
                    IpcError::internal(format!("打开本地只读缓存失败: {e}"), req_id.clone())
                })?;
                *cache_guard = Some(reopened);
                run_query(cache_guard.as_ref().expect("read_cache reopened"))
            }
        };

        res.map_err(|e| IpcError::internal(format!("查询缓存列表失败: {e}"), req_id))
    }

    fn build_reader_spec(
        &self,
        reader_session_id: String,
        detail: &MessageDetail,
        raw_prefs: &AppPreferences,
        base_appearance: &ReaderAppearance,
    ) -> (ReaderDocumentSpec, ReaderAppearance, bool) {
        let allowed_msgs = self
            .allowed_remote_messages
            .lock()
            .expect("allowed_remote_messages mutex poisoned")
            .clone();
        let remote_blocked = is_message_remote_images_blocked(detail, raw_prefs, &allowed_msgs);
        let has_thread = detail.thread_messages.len() > 1;
        let mut effective_appearance = base_appearance.clone();
        effective_appearance.block_remote_images = raw_prefs.block_remote_images;

        let key = (detail.account_id, detail.id);
        let collapsed_cards = if has_thread {
            let mut guard = self
                .thread_collapsed_cards
                .lock()
                .expect("thread_collapsed_cards mutex poisoned");
            if guard.0 != Some(key) {
                guard.0 = Some(key);
                guard.1 = compute_default_collapsed_cards(
                    detail,
                    raw_prefs.thread_newest_first,
                    raw_prefs.effective_fold_messages_mode(),
                );
            }
            guard.1.clone()
        } else {
            HashSet::new()
        };
        let raw_cards = if has_thread {
            let mut guard = self
                .thread_raw_cards
                .lock()
                .expect("thread_raw_cards mutex poisoned");
            if guard.0 != Some(key) {
                guard.0 = Some(key);
                guard.1.clear();
            }
            guard.1.clone()
        } else {
            HashSet::new()
        };
        let mut allowed_remote_cards: HashSet<u32> = detail
            .thread_messages
            .iter()
            .filter(|tm| {
                allowed_msgs.contains(&(detail.account_id, tm.id))
                    || allowed_msgs.contains(&(detail.account_id, detail.id))
            })
            .map(|tm| tm.id)
            .collect();
        if allowed_msgs.contains(&(detail.account_id, detail.id)) {
            allowed_remote_cards.insert(detail.id);
        }
        let per_message_dark = {
            let guard = self
                .per_message_dark
                .lock()
                .expect("per_message_dark mutex poisoned");
            let mut map = HashMap::new();
            if let Some(&dark) = guard.get(&(detail.account_id, detail.id)) {
                map.insert(detail.id, dark);
            }
            for tm in &detail.thread_messages {
                if let Some(&dark) = guard.get(&(detail.account_id, tm.id)) {
                    map.insert(tm.id, dark);
                }
            }
            map
        };
        let card_tags = self.resolve_card_tags_map(detail);
        let thread = Some(ThreadRenderOptions {
            detail: detail.clone(),
            collapsed_cards,
            raw_cards,
            allowed_remote_cards,
            allowed_remote_senders: raw_prefs.allowed_remote_senders.clone(),
            ui_text_scale_percent: raw_prefs.ui_text_scale_percent,
            date_style: raw_prefs.date_style.clone(),
            clock_style: raw_prefs.clock_style.clone(),
            always_show_recipients: raw_prefs.always_show_recipients,
            demo_mode: self.mode == AppMode::Demo,
            read_receipt_policy: raw_prefs.read_receipt_policy.clone(),
            initial_anchor: None,
            reading_view: self.reading_view.load(Ordering::SeqCst),
            per_message_dark,
            card_tags,
        });

        let spec = ReaderDocumentSpec {
            session_id: reader_session_id,
            account_id: detail.account_id,
            message_id: detail.id,
            subject: detail.subject.clone(),
            sender: detail.sender.clone(),
            plain_text: detail.body_text.clone(),
            html_body: detail.body_html.clone(),
            has_thread,
            thread,
        };
        (spec, effective_appearance, remote_blocked)
    }

    pub fn list_thread_summaries(
        &self,
        input: &ListThreadSummariesInput,
    ) -> Result<ListThreadSummariesOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        if input.account_id == 0 || input.message_id == 0 {
            return Err(IpcError::invalid_input(
                "accountId 与 messageId 必须为正整数",
                None,
            ));
        }
        let items: Vec<MessageSummaryDto> = match self.mode {
            AppMode::Demo => {
                let demo = self.demo.lock().expect("demo mutex poisoned");
                demo.store
                    .list_thread_summaries(input.message_id)
                    .iter()
                    .map(MessageSummaryDto::from)
                    .collect()
            }
            AppMode::Live => {
                let data_dir = {
                    let live = self.live.lock().expect("live mutex poisoned");
                    live.data_dir.clone()
                };
                if !config::global_db_in(&data_dir).exists() {
                    Vec::new()
                } else {
                    let cache =
                        CacheStore::open_for_data_dir_read_only(&data_dir).map_err(|e| {
                            IpcError::internal(format!("打开本地只读缓存失败: {e}"), None)
                        })?;
                    cache
                        .list_thread_summaries(input.account_id, input.message_id, &input.subject)
                        .map_err(|e| IpcError::internal(format!("加载会话子邮件失败: {e}"), None))?
                        .iter()
                        .map(MessageSummaryDto::from)
                        .collect()
                }
            }
        };
        Ok(ListThreadSummariesOutput {
            account_id: input.account_id,
            message_id: input.message_id,
            items,
        })
    }

    pub fn open_message(
        &self,
        input: &OpenMessageInput,
        reader_session_id: String,
    ) -> Result<OpenMessageOutput, IpcError> {
        let req_id = Some(input.request_id.clone());
        if self.is_locked() {
            return Err(IpcError::locked(req_id));
        }

        let gen = self.read_mark_gen.fetch_add(1, Ordering::SeqCst) + 1;
        *self
            .active_selected_msg
            .lock()
            .expect("active_selected_msg mutex poisoned") =
            Some((input.account_id, input.message_id));
        *self
            .active_reader_session_id
            .lock()
            .expect("active_reader_session_id mutex poisoned") = Some(reader_session_id.clone());

        let raw_prefs = self.prefs.snapshot_raw();
        let pref_dto = self.prefs.snapshot_dto();
        let reader_enabled = pref_dto.reader_position != ReaderPositionDto::Off;
        let appearance = ReaderAppearance {
            theme_id: pref_dto.theme_id,
            dark: pref_dto.dark_mode.unwrap_or(false),
            zoom_percent: pref_dto.reader_zoom_percent,
            block_remote_images: raw_prefs.block_remote_images,
        };

        match self.mode {
            AppMode::Demo => self.open_demo_message(
                input,
                reader_session_id,
                gen,
                &raw_prefs,
                reader_enabled,
                &appearance,
            ),
            AppMode::Live => self.open_live_message(
                input,
                reader_session_id,
                gen,
                &raw_prefs,
                reader_enabled,
                appearance,
            ),
        }
    }

    fn open_demo_message(
        &self,
        input: &OpenMessageInput,
        reader_session_id: String,
        gen: u64,
        raw_prefs: &AppPreferences,
        reader_enabled: bool,
        appearance: &ReaderAppearance,
    ) -> Result<OpenMessageOutput, IpcError> {
        let req_id = Some(input.request_id.clone());
        let (detail, unread_now, mark_changed_info) = {
            let mut demo = self.demo.lock().expect("demo mutex poisoned");
            let detail = demo
                .get_detail(input.account_id, input.message_id)
                .ok_or_else(|| IpcError::message_not_found(input.message_id, req_id.clone()))?;

            let was_unread = demo
                .find_summary(input.account_id, input.message_id)
                .is_some_and(|s| s.unread);
            let mut changed_info = None;
            if reader_enabled && raw_prefs.read_mark_mode == "shown" && was_unread {
                if let Some(summary) = demo.find_summary_mut(input.account_id, input.message_id) {
                    summary.unread = false;
                    changed_info = Some((summary.account_id, summary.folder_id, summary.id));
                }
            }
            (detail, was_unread, changed_info)
        };

        if let Some((account_id, folder_id, message_id)) = mark_changed_info {
            let ver = self.bump_state_version();
            self.emitter.emit_mail_changed(MailChangedEvent {
                state_version: ver,
                reason: "markRead".to_string(),
                account_id: Some(account_id),
                folder_id: Some(folder_id),
                message_id: Some(message_id),
            });
            self.refresh_tray_tooltip();
        } else if reader_enabled && raw_prefs.read_mark_mode == "delay" && unread_now {
            self.schedule_delayed_read_mark(
                gen,
                detail.account_id,
                detail.id,
                detail.uid,
                detail.folder_path.clone(),
            );
        }

        let (spec, effective_appearance, remote_blocked) =
            self.build_reader_spec(reader_session_id.clone(), &detail, raw_prefs, appearance);
        let body_status = self
            .reader
            .begin_session(spec, &effective_appearance, reader_enabled);

        Ok(build_open_message_output(
            input.request_id.clone(),
            reader_session_id,
            &detail,
            body_status,
            true,
            Some(remote_blocked),
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn open_live_message(
        &self,
        input: &OpenMessageInput,
        reader_session_id: String,
        gen: u64,
        raw_prefs: &AppPreferences,
        reader_enabled: bool,
        appearance: ReaderAppearance,
    ) -> Result<OpenMessageOutput, IpcError> {
        let req_id = Some(input.request_id.clone());
        let data_dir = {
            let live = self.live.lock().expect("live mutex poisoned");
            live.data_dir.clone()
        };

        // 1. Check in-memory body cache first (fast A->B->A switching).
        let cached_in_mem = {
            let live = self.live.lock().expect("live mutex poisoned");
            live.body_cache
                .get(&(input.account_id, input.message_id))
                .cloned()
        };

        let (detail, body_loaded, is_unread) = if let Some(detail) = cached_in_mem {
            let unread = is_detail_unread(&data_dir, &detail);
            (detail, true, unread)
        } else {
            let cache = CacheStore::open_for_data_dir_read_only(&data_dir).map_err(|e| {
                IpcError::unavailable(format!("本地邮件缓存暂不可用: {e}"), req_id.clone())
            })?;
            let Some((detail, _uid, body_loaded)) = cache
                .get_message_detail_ordered(input.message_id, raw_prefs.thread_newest_first)
                .map_err(|e| {
                    IpcError::internal(format!("读取邮件详情失败: {e}"), req_id.clone())
                })?
            else {
                return Err(IpcError::message_not_found(input.message_id, req_id));
            };
            if detail.account_id != input.account_id {
                return Err(IpcError::message_not_found(input.message_id, req_id));
            }
            let unread = is_detail_unread_with_cache(&cache, &detail);
            if body_loaded {
                let mut live = self.live.lock().expect("live mutex poisoned");
                live.cache_detail(detail.clone());
            }
            (detail, body_loaded, unread)
        };

        if body_loaded {
            {
                let mut live = self.live.lock().expect("live mutex poisoned");
                live.active_reader_target =
                    Some((reader_session_id.clone(), detail.account_id, detail.id));
                live.pending_mark_thread_seen = None;
            }
            self.apply_live_read_mark_on_open(
                gen,
                &detail,
                is_unread,
                &raw_prefs.read_mark_mode,
                reader_enabled,
                true,
            );
            let (spec, effective_appearance, remote_blocked) =
                self.build_reader_spec(reader_session_id.clone(), &detail, raw_prefs, &appearance);
            let body_status =
                self.reader
                    .begin_session(spec, &effective_appearance, reader_enabled);
            return Ok(build_open_message_output(
                input.request_id.clone(),
                reader_session_id,
                &detail,
                body_status,
                true,
                Some(remote_blocked),
            ));
        }

        // Body is not cached locally yet -> enqueue deduplicated LoadMessageBody request.
        let (worker_to_send, timeout_gen) = {
            let mut live = self.live.lock().expect("live mutex poisoned");
            live.active_reader_target =
                Some((reader_session_id.clone(), detail.account_id, detail.id));
            if reader_enabled && raw_prefs.read_mark_mode == "shown" && is_unread {
                live.pending_mark_thread_seen = Some(detail.id);
            } else {
                live.pending_mark_thread_seen = None;
            }

            let key = (detail.account_id, detail.id);
            if let Some(existing) = live.in_flight_bodies.get_mut(&key) {
                existing.reader_session_id = reader_session_id.clone();
                existing.appearance = appearance;
                existing.reader_enabled = reader_enabled;
                (None, None)
            } else {
                live.in_flight_gen = live.in_flight_gen.wrapping_add(1);
                let g = live.in_flight_gen;
                live.in_flight_bodies.insert(
                    key,
                    InFlightBodyEntry {
                        generation: g,
                        reader_session_id: reader_session_id.clone(),
                        appearance,
                        reader_enabled,
                    },
                );
                (live.worker.clone(), Some(g))
            }
        };

        if let (Some(worker), Some(in_flight_gen)) = (worker_to_send, timeout_gen) {
            worker.send(MailRequest::LoadMessageBody {
                account_id: detail.account_id,
                message_id: detail.id,
            });
            self.spawn_body_timeout_watcher(detail.account_id, detail.id, in_flight_gen);
        }

        self.apply_live_read_mark_on_open(
            gen,
            &detail,
            is_unread,
            &raw_prefs.read_mark_mode,
            reader_enabled,
            false,
        );

        Ok(build_open_message_output(
            input.request_id.clone(),
            reader_session_id,
            &detail,
            BodyStatusDto::Loading,
            false,
            None,
        ))
    }

    fn apply_live_read_mark_on_open(
        &self,
        gen: u64,
        detail: &MessageDetail,
        is_unread: bool,
        read_mark_mode: &str,
        reader_enabled: bool,
        body_ready: bool,
    ) {
        if !reader_enabled || !is_unread {
            return;
        }
        match read_mark_mode {
            "shown" => {
                self.send_live_set_seen(
                    detail.account_id,
                    detail.id,
                    detail.uid,
                    &detail.folder_path,
                    true,
                );
                if body_ready {
                    self.mark_live_thread_same_folder_unread_seen(detail);
                }
            }
            "delay" => {
                self.schedule_delayed_read_mark(
                    gen,
                    detail.account_id,
                    detail.id,
                    detail.uid,
                    detail.folder_path.clone(),
                );
            }
            _ => {}
        }
    }

    fn mark_live_thread_same_folder_unread_seen(&self, detail: &MessageDetail) {
        for (tm_id, tm_uid, tm_folder_path) in collect_same_folder_unread_thread_targets(detail) {
            self.send_live_set_seen(detail.account_id, tm_id, tm_uid, &tm_folder_path, true);
        }
    }

    fn send_live_set_seen(
        &self,
        account_id: u32,
        message_id: u32,
        uid: u32,
        folder_path: &str,
        seen: bool,
    ) {
        let worker = {
            let mut live = self.live.lock().expect("live mutex poisoned");
            if let Some(cached) = live.body_cache.get_mut(&(account_id, message_id)) {
                for tm in &mut cached.thread_messages {
                    if tm.id == message_id {
                        tm.unread = !seen;
                    }
                }
            }
            live.worker.clone()
        };
        if let Some(w) = worker {
            w.send(MailRequest::SetSeen {
                account_id,
                folder_path: folder_path.to_string(),
                message_id,
                uid,
                seen,
            });
        }
    }

    fn schedule_delayed_read_mark(
        &self,
        generation: u64,
        account_id: u32,
        message_id: u32,
        uid: u32,
        folder_path: String,
    ) {
        let weak = self
            .self_weak
            .lock()
            .expect("self_weak mutex poisoned")
            .clone();
        if weak.strong_count() == 0 {
            return;
        }
        let delay = self.read_mark_delay;
        let _ = std::thread::Builder::new()
            .name("kitemail-read-mark-delay".to_string())
            .spawn(move || {
                std::thread::sleep(delay);
                if let Some(session) = weak.upgrade() {
                    session.execute_delayed_read_mark(
                        generation,
                        account_id,
                        message_id,
                        uid,
                        &folder_path,
                    );
                }
            });
    }

    fn execute_delayed_read_mark(
        &self,
        generation: u64,
        account_id: u32,
        message_id: u32,
        uid: u32,
        folder_path: &str,
    ) {
        if self.is_locked() {
            return;
        }
        if self.read_mark_gen.load(Ordering::SeqCst) != generation {
            return;
        }
        let active = *self
            .active_selected_msg
            .lock()
            .expect("active_selected_msg mutex poisoned");
        if active != Some((account_id, message_id)) {
            return;
        }
        let pref_dto = self.prefs.snapshot_dto();
        let raw_prefs = self.prefs.snapshot_raw();
        if pref_dto.reader_position == ReaderPositionDto::Off || raw_prefs.read_mark_mode != "delay"
        {
            return;
        }

        match self.mode {
            AppMode::Demo => {
                let changed = {
                    let mut demo = self.demo.lock().expect("demo mutex poisoned");
                    if let Some(summary) = demo.find_summary_mut(account_id, message_id) {
                        if summary.unread {
                            summary.unread = false;
                            Some(summary.folder_id)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                };
                if let Some(folder_id) = changed {
                    let ver = self.bump_state_version();
                    self.emitter.emit_mail_changed(MailChangedEvent {
                        state_version: ver,
                        reason: "markRead".to_string(),
                        account_id: Some(account_id),
                        folder_id: Some(folder_id),
                        message_id: Some(message_id),
                    });
                    self.refresh_tray_tooltip();
                }
            }
            AppMode::Live => {
                self.send_live_set_seen(account_id, message_id, uid, folder_path, true);
                let maybe_detail = {
                    let live = self.live.lock().expect("live mutex poisoned");
                    live.body_cache.get(&(account_id, message_id)).cloned()
                };
                if let Some(detail) = maybe_detail {
                    self.mark_live_thread_same_folder_unread_seen(&detail);
                } else {
                    let mut live = self.live.lock().expect("live mutex poisoned");
                    live.pending_mark_thread_seen = Some(message_id);
                }
                let ver = self.bump_state_version();
                self.emitter.emit_mail_changed(MailChangedEvent {
                    state_version: ver,
                    reason: "markRead".to_string(),
                    account_id: Some(account_id),
                    folder_id: None,
                    message_id: Some(message_id),
                });
            }
        }
    }

    fn spawn_body_timeout_watcher(&self, account_id: u32, message_id: u32, generation: u64) {
        let weak = self
            .self_weak
            .lock()
            .expect("self_weak mutex poisoned")
            .clone();
        if weak.strong_count() == 0 {
            return;
        }
        let timeout = self.body_fetch_timeout;
        let _ = std::thread::Builder::new()
            .name("kitemail-body-timeout".to_string())
            .spawn(move || {
                std::thread::sleep(timeout);
                if let Some(session) = weak.upgrade() {
                    session.handle_body_fetch_timeout(account_id, message_id, generation);
                }
            });
    }

    fn handle_body_fetch_timeout(&self, account_id: u32, message_id: u32, generation: u64) {
        let timed_out_entry = {
            let mut live = self.live.lock().expect("live mutex poisoned");
            let key = (account_id, message_id);
            let matches_gen = live
                .in_flight_bodies
                .get(&key)
                .is_some_and(|e| e.generation == generation);
            if !matches_gen {
                return;
            }
            let entry = live.in_flight_bodies.remove(&key);
            let still_active = live
                .active_reader_target
                .as_ref()
                .is_some_and(|(sid, acc, msg)| {
                    *acc == account_id
                        && *msg == message_id
                        && entry.as_ref().is_some_and(|e| e.reader_session_id == *sid)
                });
            (entry, still_active)
        };

        if let (Some(entry), true) = timed_out_entry {
            if !self.is_locked() {
                let msg = "邮件正文拉取超时，请检查网络后重试".to_string();
                self.reader.emit_reader_status(ReaderStatusEvent {
                    reader_session_id: entry.reader_session_id,
                    account_id,
                    message_id,
                    body_status: BodyStatusDto::Unavailable,
                    detail: Some(msg.clone()),
                    message: Some(msg),
                    retryable: Some(true),
                    plain_text: None,
                    has_html: None,
                    html_body: None,
                    layout_applied: None,
                    layout_visible: None,
                    stage: Some("bodyTimeout".to_string()),
                });
            }
        }
    }

    pub fn process_worker_events(&self, events: Vec<WorkerEvent>) {
        for evt in events {
            match evt {
                WorkerEvent::AccountVerified { ok, message } => {
                    {
                        let mut live = self.live.lock().expect("live mutex poisoned");
                        if let Some(tx) = live.waiters.verify_account.pop_front() {
                            let _ = tx.send(VerifyAccountOutput {
                                ok,
                                message: message.clone(),
                            });
                        }
                    }
                    let ver = self.bump_state_version();
                    self.emitter.emit_account_event(AccountEvent {
                        state_version: ver,
                        kind: AccountEventKindDto::Verified,
                        ok,
                        account_id: None,
                        email: None,
                        message,
                    });
                }
                WorkerEvent::AccountSaved { account } => {
                    let acc_id = account.id;
                    let email = account.email.clone();
                    self.reset_read_cache();
                    {
                        let mut live = self.live.lock().expect("live mutex poisoned");
                        if let Some(existing) =
                            live.account_configs.iter_mut().find(|a| a.id == acc_id)
                        {
                            *existing = account.clone();
                        } else {
                            live.account_configs.push(account.clone());
                        }
                        live.accounts = live
                            .account_configs
                            .iter()
                            .map(AccountSummaryDto::from)
                            .collect();
                        live.cache_ready = true;
                        if let Some(tx) = live.waiters.save_account.remove(&acc_id) {
                            let _ = tx.send(Ok(account));
                        }
                    }
                    let ver = self.bump_state_version();
                    self.emitter.emit_account_event(AccountEvent {
                        state_version: ver,
                        kind: AccountEventKindDto::Saved,
                        ok: true,
                        account_id: Some(acc_id.to_string()),
                        email: Some(email),
                        message: "账号配置已保存".to_string(),
                    });
                    self.emitter.emit_mail_changed(MailChangedEvent {
                        state_version: ver,
                        reason: "accountsUpdated".to_string(),
                        account_id: Some(acc_id),
                        folder_id: None,
                        message_id: None,
                    });
                }
                WorkerEvent::AccountDeleted { account_id } => {
                    self.reset_read_cache();
                    {
                        let mut live = self.live.lock().expect("live mutex poisoned");
                        live.account_configs.retain(|a| a.id != account_id);
                        live.accounts = live
                            .account_configs
                            .iter()
                            .map(AccountSummaryDto::from)
                            .collect();
                        live.folders_by_account.remove(&account_id);
                        live.outbox_cache.remove(&account_id);
                        live.known_has_password.remove(&account_id);
                        if let Some(tx) = live.waiters.delete_account.remove(&account_id) {
                            let _ = tx.send(());
                        }
                    }
                    self.refresh_tray_tooltip();
                    let ver = self.bump_state_version();
                    self.emitter.emit_account_event(AccountEvent {
                        state_version: ver,
                        kind: AccountEventKindDto::Deleted,
                        ok: true,
                        account_id: Some(account_id.to_string()),
                        email: None,
                        message: "账号已删除".to_string(),
                    });
                    self.emitter.emit_mail_changed(MailChangedEvent {
                        state_version: ver,
                        reason: "accountsUpdated".to_string(),
                        account_id: Some(account_id),
                        folder_id: None,
                        message_id: None,
                    });
                }
                WorkerEvent::OAuthReauthorized { account_id, email } => {
                    let ver = self.bump_state_version();
                    self.emitter.emit_account_event(AccountEvent {
                        state_version: ver,
                        kind: AccountEventKindDto::OauthReauthorized,
                        ok: true,
                        account_id: Some(account_id.to_string()),
                        email: Some(email),
                        message: "Microsoft OAuth2 重新授权成功".to_string(),
                    });
                }
                WorkerEvent::OAuthRebindRejected {
                    account_id,
                    returned_email,
                } => {
                    let ver = self.bump_state_version();
                    self.emitter.emit_account_event(AccountEvent {
                        state_version: ver,
                        kind: AccountEventKindDto::OauthRebindRejected,
                        ok: false,
                        account_id: account_id.map(|id| id.to_string()),
                        email: Some(returned_email.clone()),
                        message: format!(
                            "授权邮箱 {returned_email} 与当前编辑账号不匹配，已拒绝重绑"
                        ),
                    });
                }
                WorkerEvent::AccountsUpdated { accounts } => {
                    self.reset_read_cache();
                    let dtos: Vec<AccountSummaryDto> =
                        accounts.iter().map(AccountSummaryDto::from).collect();
                    {
                        let mut live = self.live.lock().expect("live mutex poisoned");
                        live.account_configs = accounts;
                        live.accounts = dtos;
                        live.cache_ready = true;
                        if let Some(tx) = live.waiters.set_account_enabled.pop_front() {
                            let _ = tx.send(());
                        }
                    }
                    let ver = self.bump_state_version();
                    self.emitter.emit_account_event(AccountEvent {
                        state_version: ver,
                        kind: AccountEventKindDto::Updated,
                        ok: true,
                        account_id: None,
                        email: None,
                        message: "账号状态已更新".to_string(),
                    });
                    self.emitter.emit_mail_changed(MailChangedEvent {
                        state_version: ver,
                        reason: "accountsUpdated".to_string(),
                        account_id: None,
                        folder_id: None,
                        message_id: None,
                    });
                }
                WorkerEvent::DraftsUpdated { drafts } => {
                    {
                        let mut live = self.live.lock().expect("live mutex poisoned");
                        live.drafts_cache = drafts;
                        live.cache_ready = true;
                    }
                    let ver = self.bump_state_version();
                    self.emitter.emit_mail_changed(MailChangedEvent {
                        state_version: ver,
                        reason: "draftsUpdated".to_string(),
                        account_id: None,
                        folder_id: None,
                        message_id: None,
                    });
                }
                WorkerEvent::DraftSaved { draft_id } => {
                    {
                        let mut live = self.live.lock().expect("live mutex poisoned");
                        if let Some(tx) = live.waiters.draft_saved.pop_front() {
                            let _ = tx.send(Ok(draft_id));
                        }
                    }
                    let ver = self.bump_state_version();
                    self.emitter.emit_draft_saved(DraftSavedEvent {
                        state_version: ver,
                        draft_id: draft_id.to_string(),
                    });
                }
                WorkerEvent::OutboxEnqueued {
                    token,
                    account_id,
                    outbox_id,
                    send_at,
                    scheduled,
                } => {
                    let items = {
                        let mut live = self.live.lock().expect("live mutex poisoned");
                        live.refresh_account_outbox_from_db(account_id);
                        if let Some(tx) = live.waiters.send_enqueued.remove(&token) {
                            let _ = tx.send(Ok((outbox_id, send_at, scheduled)));
                        }
                        live.combined_outbox_dtos()
                    };
                    let ver = self.bump_state_version();
                    self.emitter.emit_outbox_updated(OutboxUpdatedEvent {
                        state_version: ver,
                        items,
                        event_kind: OutboxEventKindDto::Enqueued,
                        undo_token: Some(token.to_string()),
                        outbox_id: Some(outbox_id.to_string()),
                        message: None,
                    });
                }
                WorkerEvent::OutboxUpdated { account_id, items } => {
                    let all_items = {
                        let mut live = self.live.lock().expect("live mutex poisoned");
                        live.outbox_cache.insert(account_id, items);
                        live.combined_outbox_dtos()
                    };
                    let ver = self.bump_state_version();
                    self.emitter.emit_outbox_updated(OutboxUpdatedEvent {
                        state_version: ver,
                        items: all_items,
                        event_kind: OutboxEventKindDto::Updated,
                        undo_token: None,
                        outbox_id: None,
                        message: None,
                    });
                }
                WorkerEvent::OutboxUndoResult {
                    token,
                    account_id,
                    outbox_id,
                    result,
                } => {
                    let restored_draft_id = match &result {
                        Ok(UndoOutboxOutcome::Restored { draft_id, .. }) => Some(*draft_id),
                        _ => None,
                    };
                    let items = {
                        let mut live = self.live.lock().expect("live mutex poisoned");
                        if restored_draft_id.is_some() {
                            live.refresh_account_outbox_from_db(account_id);
                            live.refresh_drafts_from_db();
                        }
                        if let Some(tx) = live.waiters.undo_result.remove(&token) {
                            let _ = tx.send(result);
                        }
                        live.combined_outbox_dtos()
                    };
                    if let Some(draft_id) = restored_draft_id {
                        let ver = self.bump_state_version();
                        self.emitter.emit_outbox_updated(OutboxUpdatedEvent {
                            state_version: ver,
                            items,
                            event_kind: OutboxEventKindDto::Undone,
                            undo_token: Some(token.to_string()),
                            outbox_id: Some(outbox_id.to_string()),
                            message: Some("已撤销发送，邮件已恢复为草稿".to_string()),
                        });
                        self.emitter.emit_draft_saved(DraftSavedEvent {
                            state_version: ver,
                            draft_id: draft_id.to_string(),
                        });
                    }
                }
                WorkerEvent::OutboxItemDeleted {
                    account_id,
                    outbox_id,
                    deleted,
                    message,
                } => {
                    let items = {
                        let mut live = self.live.lock().expect("live mutex poisoned");
                        if deleted {
                            if let Some(list) = live.outbox_cache.get_mut(&account_id) {
                                list.retain(|item| item.id != outbox_id);
                            }
                            live.refresh_account_outbox_from_db(account_id);
                        }
                        if let Some(tx) = live.waiters.outbox_deleted.remove(&outbox_id) {
                            let result = match message {
                                Some(err) if !deleted => Err(err),
                                _ => Ok(deleted),
                            };
                            let _ = tx.send(result);
                        }
                        live.combined_outbox_dtos()
                    };
                    if deleted {
                        let ver = self.bump_state_version();
                        self.emitter.emit_outbox_updated(OutboxUpdatedEvent {
                            state_version: ver,
                            items,
                            event_kind: OutboxEventKindDto::Updated,
                            undo_token: None,
                            outbox_id: Some(outbox_id.to_string()),
                            message: None,
                        });
                    }
                }
                WorkerEvent::OutboxEnqueueFailed {
                    token,
                    message,
                    recovered_draft_id,
                } => {
                    let items = {
                        let mut live = self.live.lock().expect("live mutex poisoned");
                        if recovered_draft_id.is_some() {
                            live.refresh_drafts_from_db();
                        }
                        if let Some(tx) = live.waiters.send_enqueued.remove(&token) {
                            let _ = tx.send(Err(message.clone()));
                        }
                        live.combined_outbox_dtos()
                    };
                    let ver = self.bump_state_version();
                    self.emitter.emit_outbox_updated(OutboxUpdatedEvent {
                        state_version: ver,
                        items,
                        event_kind: OutboxEventKindDto::Failed,
                        undo_token: Some(token.to_string()),
                        outbox_id: None,
                        message: Some(message),
                    });
                    if let Some(draft_id) = recovered_draft_id {
                        self.emitter.emit_draft_saved(DraftSavedEvent {
                            state_version: ver,
                            draft_id: draft_id.to_string(),
                        });
                    }
                }
                WorkerEvent::DraftSent {
                    account_id,
                    outbox_id,
                    subject,
                } => {
                    let items = {
                        let mut live = self.live.lock().expect("live mutex poisoned");
                        if let Some(list) = live.outbox_cache.get_mut(&account_id) {
                            list.retain(|item| item.id != outbox_id);
                        }
                        live.refresh_account_outbox_from_db(account_id);
                        live.combined_outbox_dtos()
                    };
                    let ver = self.bump_state_version();
                    self.emitter.emit_outbox_updated(OutboxUpdatedEvent {
                        state_version: ver,
                        items,
                        event_kind: OutboxEventKindDto::Sent,
                        undo_token: None,
                        outbox_id: Some(outbox_id.to_string()),
                        message: Some(format!("邮件「{subject}」已发送")),
                    });
                    self.emitter.emit_mail_changed(MailChangedEvent {
                        state_version: ver,
                        reason: "draftSent".to_string(),
                        account_id: Some(account_id),
                        folder_id: None,
                        message_id: None,
                    });
                }
                WorkerEvent::SentCopySaveFailed {
                    account_id,
                    outbox_id,
                    subject,
                    message,
                } => {
                    let items = {
                        let mut live = self.live.lock().expect("live mutex poisoned");
                        if let Some(list) = live.outbox_cache.get_mut(&account_id) {
                            list.retain(|item| item.id != outbox_id);
                        }
                        live.refresh_account_outbox_from_db(account_id);
                        live.combined_outbox_dtos()
                    };
                    let ver = self.bump_state_version();
                    self.emitter.emit_outbox_updated(OutboxUpdatedEvent {
                        state_version: ver,
                        items,
                        event_kind: OutboxEventKindDto::Sent,
                        undo_token: None,
                        outbox_id: Some(outbox_id.to_string()),
                        message: Some(format!(
                            "邮件「{subject}」已发送，但保存到已发送文件夹失败: {message}"
                        )),
                    });
                    self.emitter.emit_mail_changed(MailChangedEvent {
                        state_version: ver,
                        reason: "sentCopySaveFailed".to_string(),
                        account_id: Some(account_id),
                        folder_id: None,
                        message_id: None,
                    });
                }
                WorkerEvent::OutboxDeliveryFailed {
                    account_id,
                    outbox_id,
                    status,
                    subject,
                    message,
                } => {
                    let items = {
                        let mut live = self.live.lock().expect("live mutex poisoned");
                        if let Some(list) = live.outbox_cache.get_mut(&account_id) {
                            if let Some(entry) = list.iter_mut().find(|i| i.id == outbox_id) {
                                entry.status = status;
                                entry.last_error = message.clone();
                            }
                        }
                        live.refresh_account_outbox_from_db(account_id);
                        live.combined_outbox_dtos()
                    };
                    let ver = self.bump_state_version();
                    self.emitter.emit_outbox_updated(OutboxUpdatedEvent {
                        state_version: ver,
                        items,
                        event_kind: OutboxEventKindDto::Failed,
                        undo_token: None,
                        outbox_id: Some(outbox_id.to_string()),
                        message: Some(format!("发送「{subject}」失败: {message}")),
                    });
                    self.emitter.emit_mail_changed(MailChangedEvent {
                        state_version: ver,
                        reason: "outboxDeliveryFailed".to_string(),
                        account_id: Some(account_id),
                        folder_id: None,
                        message_id: None,
                    });
                }
                WorkerEvent::FoldersLoaded {
                    account_id,
                    folders,
                } => {
                    let dtos: Vec<FolderSummaryDto> =
                        folders.iter().map(FolderSummaryDto::from).collect();
                    let fallback_to_unified = {
                        let mut live = self.live.lock().expect("live mutex poisoned");
                        live.folders_by_account.insert(account_id, dtos.clone());
                        live.cache_ready = true;
                        let cur = self.current_view();
                        matches!(
                            cur,
                            MainViewDto::AccountFolder {
                                account_id: cur_acc,
                                folder_id: cur_fid
                            } if cur_acc == account_id && !dtos.iter().any(|f| f.id == cur_fid)
                        )
                    };
                    if fallback_to_unified {
                        *self
                            .current_view
                            .lock()
                            .expect("current_view mutex poisoned") = MainViewDto::UnifiedInbox;
                        self.prefs.update_last_location(MainViewDto::UnifiedInbox);
                    }
                    self.refresh_tray_tooltip();
                    let ver = self.bump_state_version();
                    self.emitter.emit_mail_changed(MailChangedEvent {
                        state_version: ver,
                        reason: "foldersLoaded".to_string(),
                        account_id: Some(account_id),
                        folder_id: None,
                        message_id: None,
                    });
                }
                WorkerEvent::FolderSynced {
                    account_id,
                    folder_id,
                    ..
                } => {
                    {
                        let mut live = self.live.lock().expect("live mutex poisoned");
                        live.cache_ready = true;
                    }
                    let ver = self.bump_state_version();
                    self.emitter.emit_mail_changed(MailChangedEvent {
                        state_version: ver,
                        reason: "folderSynced".to_string(),
                        account_id: Some(account_id),
                        folder_id: Some(folder_id),
                        message_id: None,
                    });
                }
                WorkerEvent::MessagesPageLoaded(page) => {
                    {
                        let mut live = self.live.lock().expect("live mutex poisoned");
                        live.cache_ready = true;
                    }
                    let ver = self.bump_state_version();
                    self.emitter.emit_mail_changed(MailChangedEvent {
                        state_version: ver,
                        reason: "messagesLoaded".to_string(),
                        account_id: None,
                        folder_id: Some(page.folder_id),
                        message_id: None,
                    });
                }
                WorkerEvent::AccountSyncFinished {
                    account_id,
                    failed_folders,
                    ..
                } => {
                    let all_done = {
                        let mut live = self.live.lock().expect("live mutex poisoned");
                        live.syncing_accounts.remove(&account_id);
                        live.cache_ready = true;
                        live.syncing_accounts.is_empty()
                    };
                    let ver = self.bump_state_version();
                    let next_status = {
                        let mut status =
                            self.sync_status.lock().expect("sync_status mutex poisoned");
                        if all_done {
                            status.syncing = false;
                            status.last_sync_at = Some(chrono::Utc::now().timestamp());
                        }
                        if !failed_folders.is_empty() {
                            let summary = failed_folders
                                .iter()
                                .map(|(f, e)| format!("{f}: {e}"))
                                .collect::<Vec<_>>()
                                .join("；");
                            status.message = Some(format!("部分文件夹同步失败: {summary}"));
                        } else if all_done {
                            status.message = None;
                        }
                        status.clone()
                    };
                    self.emitter.emit_sync_status(SyncStatusEvent {
                        state_version: ver,
                        sync_status: next_status,
                    });
                    self.emitter.emit_mail_changed(MailChangedEvent {
                        state_version: ver,
                        reason: "accountSyncFinished".to_string(),
                        account_id: Some(account_id),
                        folder_id: None,
                        message_id: None,
                    });
                }
                WorkerEvent::MessageBodyLoaded(detail) => {
                    self.handle_worker_body_loaded(detail);
                }
                WorkerEvent::MessageBodyLoadFailed {
                    account_id,
                    message_id,
                    message,
                } => {
                    self.handle_worker_body_failed(account_id, message_id, message);
                }
                WorkerEvent::OlderMessagesLoaded {
                    folder_id,
                    fetched_count: _,
                    has_more_remote,
                } => {
                    {
                        let mut live = self.live.lock().expect("live mutex poisoned");
                        live.cache_ready = true;
                        if folder_id == 0 {
                            // 统一收件箱按文件夹分别回写游标：单账号成功结果已由 `FoldersLoaded`
                            // 按文件夹更新；仅当全部统一账号均确认无更多且无失败时才整体收口。
                            if !has_more_remote {
                                let unified_ids = live.unified_account_ids();
                                let enforce_scope = !live.account_configs.is_empty();
                                for folders in live.folders_by_account.values_mut() {
                                    for f in folders.iter_mut() {
                                        if is_inbox_folder(f)
                                            && (!enforce_scope
                                                || unified_ids.contains(&f.account_id))
                                        {
                                            f.has_more_remote = false;
                                        }
                                    }
                                }
                            }
                        } else {
                            for folders in live.folders_by_account.values_mut() {
                                for f in folders.iter_mut() {
                                    if f.id == folder_id {
                                        f.has_more_remote = has_more_remote;
                                    }
                                }
                            }
                        }
                    }
                    let ver = self.bump_state_version();
                    self.emitter.emit_mail_changed(MailChangedEvent {
                        state_version: ver,
                        reason: "olderMessagesLoaded".to_string(),
                        account_id: None,
                        folder_id: (folder_id != 0).then_some(folder_id),
                        message_id: None,
                    });
                }
                WorkerEvent::MessageArchived {
                    account_id,
                    message_id,
                    ..
                } => {
                    self.handle_live_message_moved_event(account_id, message_id, "messageArchived");
                }
                WorkerEvent::MessageDeleted {
                    account_id,
                    message_id,
                    ..
                } => {
                    self.handle_live_message_moved_event(account_id, message_id, "messageDeleted");
                }
                WorkerEvent::MessageMoved {
                    account_id,
                    message_id,
                    ..
                } => {
                    self.handle_live_message_moved_event(account_id, message_id, "messageMoved");
                }
                WorkerEvent::MessageMarkedSpam {
                    account_id,
                    message_id,
                    ..
                } => {
                    self.handle_live_message_moved_event(
                        account_id,
                        message_id,
                        "messageMarkedSpam",
                    );
                }
                WorkerEvent::MessageTagsUpdated {
                    account_id,
                    message_id,
                    tags,
                } => {
                    let ver = self.bump_state_version();
                    self.emitter.emit_mail_changed(MailChangedEvent {
                        state_version: ver,
                        reason: "tagsUpdated".to_string(),
                        account_id: Some(account_id),
                        folder_id: None,
                        message_id: Some(message_id),
                    });
                    let tag_pairs = self.decorate_tag_pairs(&tags);
                    let raw_prefs = self.prefs.snapshot_raw();
                    let appearance = self.current_reader_appearance(raw_prefs.block_remote_images);
                    self.reader.rerender_matching_document(
                        account_id,
                        message_id,
                        &appearance,
                        |spec| {
                            if let Some(ref mut thread) = spec.thread {
                                thread.card_tags.insert(message_id, tag_pairs);
                            }
                        },
                    );
                }
                WorkerEvent::AttachmentBytesFetched {
                    attachment_id,
                    filename,
                    mime_type,
                    bytes,
                } => {
                    let mut live = self.live.lock().expect("live mutex poisoned");
                    if let Some(tx) = live.waiters.attachment_bytes.remove(&attachment_id) {
                        let _ = tx.send(Ok((filename, mime_type, bytes)));
                    }
                }
                WorkerEvent::AttachmentSaved { path } => {
                    let mut live = self.live.lock().expect("live mutex poisoned");
                    if let Some(tx) = live.waiters.attachment_saved.pop_front() {
                        let _ = tx.send(Ok(path));
                    }
                }
                WorkerEvent::FolderSyncFailed {
                    user_initiated,
                    removed_from_server,
                    message,
                    ..
                } => {
                    if user_initiated || removed_from_server {
                        let any_syncing = !self
                            .live
                            .lock()
                            .expect("live mutex poisoned")
                            .syncing_accounts
                            .is_empty();
                        let ver = self.bump_state_version();
                        let next_status = {
                            let mut status =
                                self.sync_status.lock().expect("sync_status mutex poisoned");
                            if !any_syncing {
                                status.syncing = false;
                            }
                            status.message = Some(message);
                            status.clone()
                        };
                        self.emitter.emit_sync_status(SyncStatusEvent {
                            state_version: ver,
                            sync_status: next_status,
                        });
                    }
                }
                WorkerEvent::Error {
                    account_id,
                    message,
                } => {
                    let (handled_save_waiter, all_done) = {
                        let mut live = self.live.lock().expect("live mutex poisoned");
                        if let Some(acc_id) = account_id {
                            live.syncing_accounts.remove(&acc_id);
                        } else {
                            live.syncing_accounts.clear();
                        }
                        if !live.account_configs.is_empty()
                            || config::global_db_in(&live.data_dir).exists()
                        {
                            live.cache_ready = true;
                        }
                        let handled = if let Some(acc_id) = account_id {
                            if let Some(tx) = live.waiters.save_account.remove(&acc_id) {
                                let _ = tx.send(Err(message.clone()));
                                true
                            } else {
                                false
                            }
                        } else {
                            if let Some(tx) = live.waiters.attachment_saved.pop_front() {
                                let _ = tx.send(Err(message.clone()));
                            }
                            false
                        };
                        (handled, live.syncing_accounts.is_empty())
                    };
                    let ver = self.bump_state_version();
                    if handled_save_waiter {
                        self.emitter.emit_account_event(AccountEvent {
                            state_version: ver,
                            kind: AccountEventKindDto::Saved,
                            ok: false,
                            account_id: account_id.map(|id| id.to_string()),
                            email: None,
                            message: message.clone(),
                        });
                    }
                    let next_status = {
                        let mut status =
                            self.sync_status.lock().expect("sync_status mutex poisoned");
                        if all_done {
                            status.syncing = false;
                        }
                        status.message = Some(message);
                        status.clone()
                    };
                    self.emitter.emit_sync_status(SyncStatusEvent {
                        state_version: ver,
                        sync_status: next_status,
                    });
                }
                _ => {}
            }
        }
    }

    fn handle_live_message_moved_event(&self, account_id: u32, message_id: u32, reason: &str) {
        let data_dir = {
            let mut live = self.live.lock().expect("live mutex poisoned");
            live.remove_cached_detail(account_id, message_id);
            live.data_dir.clone()
        };
        if config::global_db_in(&data_dir).exists() {
            if let Ok(cache) = CacheStore::open_for_data_dir_read_only(&data_dir) {
                if let Ok(folders) = cache.list_folders(account_id) {
                    let dtos: Vec<FolderSummaryDto> =
                        folders.iter().map(FolderSummaryDto::from).collect();
                    let mut live = self.live.lock().expect("live mutex poisoned");
                    live.folders_by_account.insert(account_id, dtos);
                }
            }
        }
        self.refresh_tray_tooltip();
        let ver = self.bump_state_version();
        self.emitter.emit_mail_changed(MailChangedEvent {
            state_version: ver,
            reason: reason.to_string(),
            account_id: Some(account_id),
            folder_id: None,
            message_id: Some(message_id),
        });
    }

    fn handle_worker_body_loaded(&self, detail: MessageDetail) {
        if self.is_locked() {
            let mut live = self.live.lock().expect("live mutex poisoned");
            live.in_flight_bodies
                .remove(&(detail.account_id, detail.id));
            return;
        }

        let (entry_opt, is_active, mark_thread_seen) = {
            let mut live = self.live.lock().expect("live mutex poisoned");
            live.cache_detail(detail.clone());
            let entry = live
                .in_flight_bodies
                .remove(&(detail.account_id, detail.id));
            let active = live
                .active_reader_target
                .as_ref()
                .is_some_and(|(_, acc, msg)| *acc == detail.account_id && *msg == detail.id);
            let mark_thread = active && live.pending_mark_thread_seen == Some(detail.id);
            if mark_thread {
                live.pending_mark_thread_seen = None;
            }
            (entry, active, mark_thread)
        };

        if !is_active {
            return;
        }

        if mark_thread_seen {
            self.mark_live_thread_same_folder_unread_seen(&detail);
        }

        let Some(entry) = entry_opt else {
            return;
        };

        let raw_prefs = self.prefs.snapshot_raw();
        let (spec, effective_appearance, _) = self.build_reader_spec(
            entry.reader_session_id,
            &detail,
            &raw_prefs,
            &entry.appearance,
        );
        self.reader
            .begin_session(spec, &effective_appearance, entry.reader_enabled);
    }

    fn handle_worker_body_failed(&self, account_id: u32, message_id: u32, message: String) {
        let entry_opt = {
            let mut live = self.live.lock().expect("live mutex poisoned");
            let entry = live.in_flight_bodies.remove(&(account_id, message_id));
            if live.pending_mark_thread_seen == Some(message_id) {
                live.pending_mark_thread_seen = None;
            }
            let active = live
                .active_reader_target
                .as_ref()
                .is_some_and(|(_, acc, msg)| *acc == account_id && *msg == message_id);
            if active {
                entry
            } else {
                None
            }
        };

        if let Some(entry) = entry_opt {
            if !self.is_locked() {
                self.reader.emit_reader_status(ReaderStatusEvent {
                    reader_session_id: entry.reader_session_id,
                    account_id,
                    message_id,
                    body_status: BodyStatusDto::Unavailable,
                    detail: Some(message.clone()),
                    message: Some(message),
                    retryable: Some(true),
                    plain_text: None,
                    has_html: None,
                    html_body: None,
                    layout_applied: None,
                    layout_visible: None,
                    stage: Some("bodyFailed".to_string()),
                });
            }
        }
    }

    pub fn set_message_seen(
        &self,
        input: &SetMessageSeenInput,
    ) -> Result<SetMessageSeenOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        if input.account_id == 0 || input.message_id == 0 {
            return Err(IpcError::invalid_input(
                "accountId 与 messageId 必须为正整数",
                None,
            ));
        }

        let unread = !input.seen;
        let reason = if input.seen { "markRead" } else { "markUnread" };

        let folder_id = match self.mode {
            AppMode::Demo => {
                let mut demo = self.demo.lock().expect("demo mutex poisoned");
                let summary = demo
                    .find_summary_mut(input.account_id, input.message_id)
                    .ok_or_else(|| IpcError::message_not_found(input.message_id, None))?;
                summary.unread = unread;
                Some(summary.folder_id)
            }
            AppMode::Live => {
                let data_dir = {
                    let live = self.live.lock().expect("live mutex poisoned");
                    live.data_dir.clone()
                };
                let cache = CacheStore::open_for_data_dir_read_only(&data_dir)
                    .map_err(|e| IpcError::unavailable(format!("本地缓存暂不可用: {e}"), None))?;
                let (folder_path, uid) = cache
                    .get_message_location(input.message_id)
                    .map_err(|e| IpcError::internal(format!("查询邮件位置失败: {e}"), None))?
                    .ok_or_else(|| IpcError::message_not_found(input.message_id, None))?;
                self.send_live_set_seen(
                    input.account_id,
                    input.message_id,
                    uid,
                    &folder_path,
                    input.seen,
                );
                None
            }
        };

        let ver = self.bump_state_version();
        self.emitter.emit_mail_changed(MailChangedEvent {
            state_version: ver,
            reason: reason.to_string(),
            account_id: Some(input.account_id),
            folder_id,
            message_id: Some(input.message_id),
        });
        self.refresh_tray_tooltip();

        Ok(SetMessageSeenOutput {
            account_id: input.account_id,
            message_id: input.message_id,
            unread,
            state_version: ver,
        })
    }

    pub fn set_message_flagged(
        &self,
        input: &SetMessageFlaggedInput,
    ) -> Result<SetMessageFlaggedOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        if input.account_id == 0 || input.message_id == 0 {
            return Err(IpcError::invalid_input(
                "accountId 与 messageId 必须为正整数",
                None,
            ));
        }

        let folder_id = match self.mode {
            AppMode::Demo => {
                let mut demo = self.demo.lock().expect("demo mutex poisoned");
                let summary = demo
                    .find_summary_mut(input.account_id, input.message_id)
                    .ok_or_else(|| IpcError::message_not_found(input.message_id, None))?;
                summary.flagged = input.flagged;
                Some(summary.folder_id)
            }
            AppMode::Live => {
                let data_dir = {
                    let live = self.live.lock().expect("live mutex poisoned");
                    live.data_dir.clone()
                };
                let cache = CacheStore::open_for_data_dir_read_only(&data_dir)
                    .map_err(|e| IpcError::unavailable(format!("本地缓存暂不可用: {e}"), None))?;
                let (folder_path, uid) = cache
                    .get_message_location(input.message_id)
                    .map_err(|e| IpcError::internal(format!("查询邮件位置失败: {e}"), None))?
                    .ok_or_else(|| IpcError::message_not_found(input.message_id, None))?;

                let worker = {
                    let mut live = self.live.lock().expect("live mutex poisoned");
                    if let Some(cached) = live
                        .body_cache
                        .get_mut(&(input.account_id, input.message_id))
                    {
                        cached.flagged = input.flagged;
                        for tm in &mut cached.thread_messages {
                            if tm.id == input.message_id {
                                tm.flagged = input.flagged;
                            }
                        }
                    }
                    live.worker.clone()
                };
                if let Some(w) = worker {
                    w.send(MailRequest::SetFlagged {
                        account_id: input.account_id,
                        folder_path,
                        message_id: input.message_id,
                        uid,
                        flagged: input.flagged,
                    });
                }
                None
            }
        };

        let ver = self.bump_state_version();
        self.emitter.emit_mail_changed(MailChangedEvent {
            state_version: ver,
            reason: "toggleFlag".to_string(),
            account_id: Some(input.account_id),
            folder_id,
            message_id: Some(input.message_id),
        });

        Ok(SetMessageFlaggedOutput {
            account_id: input.account_id,
            message_id: input.message_id,
            flagged: input.flagged,
            state_version: ver,
        })
    }

    #[cfg(test)]
    pub fn set_demo_message_unread(
        &self,
        account_id: u32,
        message_id: u32,
        unread: bool,
    ) -> Result<u64, IpcError> {
        self.set_message_seen(&SetMessageSeenInput {
            account_id,
            message_id,
            seen: !unread,
        })
        .map(|o| o.state_version)
    }

    #[cfg(test)]
    pub fn set_demo_message_flagged(
        &self,
        account_id: u32,
        message_id: u32,
        flagged: bool,
    ) -> Result<u64, IpcError> {
        self.set_message_flagged(&SetMessageFlaggedInput {
            account_id,
            message_id,
            flagged,
        })
        .map(|o| o.state_version)
    }

    pub fn refresh_mail(&self, input: &RefreshMailInput) -> Result<RefreshMailOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }

        match self.mode {
            AppMode::Demo => {
                self.demo
                    .lock()
                    .expect("demo mutex poisoned")
                    .validate_view(input.view, None)?;
                *self
                    .current_view
                    .lock()
                    .expect("current_view mutex poisoned") = input.view;

                let starting_ver = self.state_version();
                let syncing_status = SyncStatusDto {
                    syncing: true,
                    message: Some("正在刷新演示邮箱...".to_string()),
                    last_sync_at: self
                        .sync_status
                        .lock()
                        .expect("sync_status mutex poisoned")
                        .last_sync_at,
                };
                *self.sync_status.lock().expect("sync_status mutex poisoned") =
                    syncing_status.clone();
                self.emitter.emit_sync_status(SyncStatusEvent {
                    state_version: starting_ver,
                    sync_status: syncing_status,
                });

                let ver = self.bump_state_version();
                let done_status = SyncStatusDto {
                    syncing: false,
                    message: None,
                    last_sync_at: Some(chrono::Utc::now().timestamp()),
                };
                *self.sync_status.lock().expect("sync_status mutex poisoned") = done_status.clone();
                self.emitter.emit_mail_changed(MailChangedEvent {
                    state_version: ver,
                    reason: "refresh".to_string(),
                    account_id: None,
                    folder_id: None,
                    message_id: None,
                });
                self.emitter.emit_sync_status(SyncStatusEvent {
                    state_version: ver,
                    sync_status: done_status,
                });

                Ok(RefreshMailOutput {
                    accepted: true,
                    state_version: ver,
                })
            }
            AppMode::Live => {
                let (worker, req, acc_ids) = {
                    let mut live = self.live.lock().expect("live mutex poisoned");
                    let req = match input.view {
                        MainViewDto::AccountFolder {
                            account_id,
                            folder_id,
                        } => {
                            let folder_path = live
                                .folders_by_account
                                .get(&account_id)
                                .and_then(|fs| fs.iter().find(|f| f.id == folder_id))
                                .map(|f| f.path.clone())
                                .ok_or_else(|| {
                                    IpcError::invalid_input(
                                        format!(
                                            "无效的账号或文件夹: accountId={account_id}, folderId={folder_id}"
                                        ),
                                        None,
                                    )
                                })?;
                            live.syncing_accounts.insert(account_id);
                            MailRequest::SyncFolder {
                                account_id,
                                folder_id,
                                folder_path,
                                user_initiated: true,
                            }
                        }
                        _ => {
                            let enabled = live.enabled_account_ids();
                            for id in enabled {
                                live.syncing_accounts.insert(id);
                            }
                            MailRequest::SyncAllAccounts {
                                user_initiated: true,
                            }
                        }
                    };
                    let syncing = live.syncing_accounts.is_empty();
                    (live.worker.clone(), req, syncing)
                };

                *self
                    .current_view
                    .lock()
                    .expect("current_view mutex poisoned") = input.view;
                self.prefs.update_last_location(input.view);

                let ver = self.bump_state_version();
                if acc_ids {
                    return Ok(RefreshMailOutput {
                        accepted: false,
                        state_version: ver,
                    });
                }

                let syncing_status = {
                    let mut status = self.sync_status.lock().expect("sync_status mutex poisoned");
                    status.syncing = true;
                    status.message = Some("正在同步邮箱...".to_string());
                    status.clone()
                };
                self.emitter.emit_sync_status(SyncStatusEvent {
                    state_version: ver,
                    sync_status: syncing_status,
                });

                if let Some(w) = worker {
                    w.send(req);
                }

                Ok(RefreshMailOutput {
                    accepted: true,
                    state_version: ver,
                })
            }
        }
    }

    pub fn lock_app(&self, reason: Option<&str>) -> LockAppOutput {
        let mut guard = self.lock_state.lock().expect("lock_state mutex poisoned");
        if guard.passcode_hash.is_none() {
            return LockAppOutput { locked: false };
        }
        if guard.locked {
            return LockAppOutput { locked: true };
        }
        guard.locked = true;
        guard.locked_reason = reason.map(str::to_string);
        guard.tracker.reset_after_unlock(Instant::now());
        if let Some(ref dir) = guard.persist_dir {
            let _ = app_lock::set_lock_persisted_in(dir, true);
        }
        let has_passcode = guard.has_passcode();
        drop(guard);

        self.locked.store(true, Ordering::SeqCst);
        self.read_mark_gen.fetch_add(1, Ordering::SeqCst);
        *self
            .active_selected_msg
            .lock()
            .expect("active_selected_msg mutex poisoned") = None;
        self.allowed_remote_messages
            .lock()
            .expect("allowed_remote_messages mutex poisoned")
            .clear();
        *self
            .thread_collapsed_cards
            .lock()
            .expect("thread_collapsed_cards mutex poisoned") = (None, HashSet::new());
        *self
            .thread_raw_cards
            .lock()
            .expect("thread_raw_cards mutex poisoned") = (None, HashSet::new());
        if self.mode == AppMode::Live {
            self.live
                .lock()
                .expect("live mutex poisoned")
                .clear_sensitive_caches();
        }
        self.reader.set_locked(true);

        let ver = self.bump_state_version();
        self.emitter.emit_lock_state(LockStateEvent {
            state_version: ver,
            locked: true,
            has_passcode,
            locked_reason: reason.map(str::to_string),
        });
        LockAppOutput { locked: true }
    }

    pub fn unlock_app(&self, input: &UnlockAppInput) -> UnlockAppOutput {
        let now = Instant::now();
        let (stored_hash, persist_dir) = {
            let mut guard = self.lock_state.lock().expect("lock_state mutex poisoned");
            if !guard.locked {
                return UnlockAppOutput {
                    unlocked: true,
                    locked: false,
                    error: None,
                    retry_after_secs: None,
                };
            }
            if let Some(until) = guard.cooldown_until {
                if until > now {
                    let d = until.saturating_duration_since(now);
                    let secs = d
                        .as_secs()
                        .saturating_add(if d.subsec_nanos() > 0 { 1 } else { 0 });
                    return UnlockAppOutput {
                        unlocked: false,
                        locked: true,
                        error: Some(format!("密码错误次数过多，请 {secs} 秒后再试")),
                        retry_after_secs: Some(secs),
                    };
                }
                guard.cooldown_until = None;
            }
            if input.passcode.is_empty() {
                return UnlockAppOutput {
                    unlocked: false,
                    locked: true,
                    error: Some("请输入锁屏密码".to_string()),
                    retry_after_secs: None,
                };
            }
            let Some(hash) = guard.passcode_hash.clone() else {
                guard.locked = false;
                guard.locked_reason = None;
                self.locked.store(false, Ordering::SeqCst);
                self.reader.set_locked(false);
                return UnlockAppOutput {
                    unlocked: true,
                    locked: false,
                    error: None,
                    retry_after_secs: None,
                };
            };
            (hash, guard.persist_dir.clone())
        };

        let matched = app_lock::verify_passcode(&input.passcode, &stored_hash);
        if matched {
            let has_passcode = {
                let mut guard = self.lock_state.lock().expect("lock_state mutex poisoned");
                guard.locked = false;
                guard.locked_reason = None;
                guard.failed_attempts = 0;
                guard.cooldown_until = None;
                guard.tracker.reset_after_unlock(Instant::now());
                guard.has_passcode()
            };
            if let Some(ref dir) = persist_dir {
                let _ = app_lock::set_lock_persisted_in(dir, false);
            }
            self.locked.store(false, Ordering::SeqCst);
            self.reader.set_locked(false);
            if self.mode == AppMode::Live {
                let (worker, should_warm) = {
                    let live = self.live.lock().expect("live mutex poisoned");
                    (
                        live.worker.clone(),
                        !live.snapshots_hydrated && !live.account_configs.is_empty(),
                    )
                };
                if should_warm {
                    if let Some(w) = worker {
                        w.send(MailRequest::LoadAllCached);
                        w.send(MailRequest::SyncAllAccounts {
                            user_initiated: false,
                        });
                    }
                }
            }
            let ver = self.bump_state_version();
            self.emitter.emit_lock_state(LockStateEvent {
                state_version: ver,
                locked: false,
                has_passcode,
                locked_reason: None,
            });
            UnlockAppOutput {
                unlocked: true,
                locked: false,
                error: None,
                retry_after_secs: None,
            }
        } else {
            let (error, retry_after_secs) = {
                let mut guard = self.lock_state.lock().expect("lock_state mutex poisoned");
                guard.failed_attempts = guard.failed_attempts.saturating_add(1);
                let cooldown = app_lock::unlock_cooldown_secs(guard.failed_attempts);
                if cooldown > 0 {
                    guard.cooldown_until = Some(Instant::now() + Duration::from_secs(cooldown));
                    (
                        Some(format!("密码错误次数过多，请 {cooldown} 秒后再试")),
                        Some(cooldown),
                    )
                } else {
                    (Some("锁屏密码错误，请重新输入".to_string()), None)
                }
            };
            UnlockAppOutput {
                unlocked: false,
                locked: true,
                error,
                retry_after_secs,
            }
        }
    }

    pub fn auto_lock_tick(&self, in_foreground: bool, system_idle: Option<Duration>) {
        let raw_prefs = self.prefs.snapshot_raw();
        let trigger = {
            let mut guard = self.lock_state.lock().expect("lock_state mutex poisoned");
            if guard.locked || guard.passcode_hash.is_none() {
                return;
            }
            let bg_policy =
                BackgroundLockPolicy::from_pref_str(&raw_prefs.app_lock_background_timeout);
            guard.tracker.evaluate(AutoLockTickInput {
                now: Instant::now(),
                has_passcode: true,
                is_locked: false,
                in_foreground,
                file_dialog_open: false,
                system_idle,
                idle_minutes: raw_prefs.app_lock_idle_minutes,
                background_policy: bg_policy,
            })
        };
        if let Some(trigger) = trigger {
            let reason = match trigger {
                AutoLockTrigger::Idle => "idle",
                AutoLockTrigger::Background => "background",
            };
            self.lock_app(Some(reason));
        }
    }

    pub fn on_preferences_updated(&self) {
        let raw_prefs = self.prefs.snapshot_raw();
        let dto = self.prefs.snapshot_dto();
        let enabled = dto.reader_position != ReaderPositionDto::Off;
        if !enabled {
            self.read_mark_gen.fetch_add(1, Ordering::SeqCst);
        }
        self.reader.set_reader_enabled(enabled);
        let active = *self
            .active_selected_msg
            .lock()
            .expect("active_selected_msg mutex poisoned");
        let block_remote_images = match active {
            Some(key)
                if self
                    .allowed_remote_messages
                    .lock()
                    .expect("allowed_remote_messages mutex poisoned")
                    .contains(&key) =>
            {
                false
            }
            _ => raw_prefs.block_remote_images,
        };
        self.reader.update_appearance(&ReaderAppearance {
            theme_id: dto.theme_id,
            dark: dto.dark_mode.unwrap_or(false),
            zoom_percent: dto.reader_zoom_percent,
            block_remote_images,
        });
    }

    pub fn prepare_quit(&self) {
        self.quitting.store(true, Ordering::SeqCst);
        self.read_mark_gen.fetch_add(1, Ordering::SeqCst);
        {
            let (lock, cvar) = &*self.auto_sync_signal;
            *lock.lock().expect("auto_sync_signal mutex poisoned") = true;
            cvar.notify_all();
        }
        if self.mode == AppMode::Live {
            self.reset_read_cache();
            let worker = self
                .live
                .lock()
                .expect("live mutex poisoned")
                .worker
                .clone();
            if let Some(w) = worker {
                w.prepare_quit();
            }
        }
    }

    fn current_reader_appearance(&self, block_remote_images: bool) -> ReaderAppearance {
        let dto = self.prefs.snapshot_dto();
        ReaderAppearance {
            theme_id: dto.theme_id,
            dark: dto.dark_mode.unwrap_or(false),
            zoom_percent: dto.reader_zoom_percent,
            block_remote_images,
        }
    }

    pub(crate) fn load_detail_for_action(
        &self,
        account_id: u32,
        message_id: u32,
    ) -> Result<MessageDetail, IpcError> {
        match self.mode {
            AppMode::Demo => self
                .demo
                .lock()
                .expect("demo mutex poisoned")
                .get_detail(account_id, message_id)
                .ok_or_else(|| IpcError::message_not_found(message_id, None)),
            AppMode::Live => {
                let (data_dir, cached) = {
                    let live = self.live.lock().expect("live mutex poisoned");
                    (
                        live.data_dir.clone(),
                        live.body_cache.get(&(account_id, message_id)).cloned(),
                    )
                };
                if let Some(detail) = cached {
                    return Ok(detail);
                }
                let cache = CacheStore::open_for_data_dir_read_only(&data_dir)
                    .map_err(|e| IpcError::unavailable(format!("本地缓存暂不可用: {e}"), None))?;
                let raw_prefs = self.prefs.snapshot_raw();
                let Some((detail, _, _)) = cache
                    .get_message_detail_ordered(message_id, raw_prefs.thread_newest_first)
                    .map_err(|e| IpcError::internal(format!("读取邮件详情失败: {e}"), None))?
                else {
                    return Err(IpcError::message_not_found(message_id, None));
                };
                if detail.account_id != account_id {
                    return Err(IpcError::message_not_found(message_id, None));
                }
                Ok(detail)
            }
        }
    }

    pub fn allow_remote_images(
        &self,
        input: &AllowRemoteImagesInput,
    ) -> Result<AllowRemoteImagesOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        if input.account_id == 0 || input.message_id == 0 {
            return Err(IpcError::invalid_input(
                "accountId 与 messageId 必须为正整数",
                None,
            ));
        }

        let detail = self.load_detail_for_action(input.account_id, input.message_id)?;
        let target_sender = detail
            .thread_messages
            .iter()
            .find(|tm| tm.id == input.message_id)
            .map(|tm| tm.sender.clone())
            .unwrap_or_else(|| detail.sender.clone());

        {
            let mut allowed = self
                .allowed_remote_messages
                .lock()
                .expect("allowed_remote_messages mutex poisoned");
            allowed.insert((input.account_id, input.message_id));
            if input.message_id == detail.id && detail.thread_messages.len() > 1 {
                for tm in &detail.thread_messages {
                    allowed.insert((input.account_id, tm.id));
                }
            }
        }

        let mut sender_trusted = false;
        if input.trust_sender {
            let mut persisted_trusted = false;
            self.prefs
                .mutate_and_save(|prefs| {
                    let prev_len = prefs.allowed_remote_senders.len();
                    if prefs.allow_remote_sender(&target_sender).is_some() {
                        persisted_trusted = true;
                    }
                    prefs.allowed_remote_senders.len() != prev_len
                })
                .map_err(|e| IpcError::internal(format!("保存信任发件人失败: {e}"), None))?;
            sender_trusted = persisted_trusted;
        }

        let raw_prefs = self.prefs.snapshot_raw();
        let allowed_senders = raw_prefs.allowed_remote_senders.clone();
        let is_thread = detail.thread_messages.len() > 1;
        let thread_ids: Vec<u32> = detail.thread_messages.iter().map(|tm| tm.id).collect();
        let appearance = self.current_reader_appearance(if is_thread {
            raw_prefs.block_remote_images
        } else {
            false
        });
        self.reader
            .rerender_matching_document(input.account_id, detail.id, &appearance, |spec| {
                if let Some(ref mut thread) = spec.thread {
                    thread.allowed_remote_cards.insert(input.message_id);
                    if input.message_id == detail.id {
                        for id in &thread_ids {
                            thread.allowed_remote_cards.insert(*id);
                        }
                    }
                    thread.allowed_remote_senders = allowed_senders.clone();
                }
            });

        let ver = self.bump_state_version();
        Ok(AllowRemoteImagesOutput {
            allowed: true,
            sender_trusted,
            state_version: ver,
        })
    }

    pub fn load_older_messages(
        &self,
        input: &LoadOlderMessagesInput,
    ) -> Result<LoadOlderMessagesOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        if input.limit == 0 {
            return Err(IpcError::invalid_input("limit 必须大于 0", None));
        }

        match self.mode {
            AppMode::Demo => {
                self.demo
                    .lock()
                    .expect("demo mutex poisoned")
                    .validate_view(input.view, None)?;
                Ok(LoadOlderMessagesOutput {
                    accepted: false,
                    has_more_remote: false,
                    state_version: self.state_version(),
                })
            }
            AppMode::Live => {
                let (worker, target_folder_id, has_more_remote) = {
                    let live = self.live.lock().expect("live mutex poisoned");
                    match input.view {
                        MainViewDto::UnifiedInbox => {
                            let more = live
                                .folders_by_account
                                .values()
                                .flatten()
                                .any(|f| is_inbox_folder(f) && f.has_more_remote);
                            (live.worker.clone(), Some(0u32), more)
                        }
                        MainViewDto::AccountFolder {
                            account_id,
                            folder_id,
                        } => {
                            let folder = live
                                .folders_by_account
                                .get(&account_id)
                                .and_then(|fs| fs.iter().find(|f| f.id == folder_id))
                                .ok_or_else(|| {
                                    IpcError::invalid_input(
                                        format!(
                                            "无效的账号或文件夹: accountId={account_id}, folderId={folder_id}"
                                        ),
                                        None,
                                    )
                                })?;
                            (live.worker.clone(), Some(folder_id), folder.has_more_remote)
                        }
                        _ => (live.worker.clone(), None, false),
                    }
                };

                let Some(folder_id) = target_folder_id else {
                    return Ok(LoadOlderMessagesOutput {
                        accepted: false,
                        has_more_remote: false,
                        state_version: self.state_version(),
                    });
                };

                let accepted = if let Some(w) = worker {
                    w.send(MailRequest::LoadOlderMessages {
                        folder_id,
                        limit: input.limit.min(MAX_PAGE_LIMIT),
                    });
                    true
                } else {
                    false
                };

                Ok(LoadOlderMessagesOutput {
                    accepted,
                    has_more_remote,
                    state_version: self.state_version(),
                })
            }
        }
    }

    pub fn archive_message(
        &self,
        input: &MessageActionInput,
    ) -> Result<MessageMoveOutput, IpcError> {
        self.execute_message_move(input.account_id, input.message_id, LiveMoveKind::Archive)
    }

    pub fn delete_message(
        &self,
        input: &MessageActionInput,
    ) -> Result<MessageMoveOutput, IpcError> {
        self.execute_message_move(input.account_id, input.message_id, LiveMoveKind::Delete)
    }

    pub fn mark_message_spam(
        &self,
        input: &MessageActionInput,
    ) -> Result<MessageMoveOutput, IpcError> {
        self.execute_message_move(input.account_id, input.message_id, LiveMoveKind::Spam)
    }

    pub fn move_message(&self, input: &MoveMessageInput) -> Result<MessageMoveOutput, IpcError> {
        if input.target_folder_path.trim().is_empty() {
            return Err(IpcError::invalid_input("targetFolderPath 不能为空", None));
        }
        self.execute_message_move(
            input.account_id,
            input.message_id,
            LiveMoveKind::Folder(input.target_folder_path.trim().to_string()),
        )
    }

    pub fn unmark_message_spam(
        &self,
        input: &MessageActionInput,
    ) -> Result<MessageMoveOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        if input.account_id == 0 || input.message_id == 0 {
            return Err(IpcError::invalid_input(
                "accountId 与 messageId 必须为正整数",
                None,
            ));
        }
        let detail = self.load_detail_for_action(input.account_id, input.message_id)?;
        let is_in_spam = {
            let p = detail.folder_path.trim().to_ascii_lowercase();
            p == "junk"
                || p == "spam"
                || p == "junkemail"
                || p.contains("spam")
                || p.contains("junk")
        };
        if !is_in_spam {
            return Err(IpcError::invalid_input(
                "仅垃圾邮件文件夹中的邮件可执行恢复操作",
                None,
            ));
        }
        let target_inbox = match self.mode {
            AppMode::Demo => "INBOX".to_string(),
            AppMode::Live => {
                let data_dir = self
                    .live
                    .lock()
                    .expect("live mutex poisoned")
                    .data_dir
                    .clone();
                let cache = CacheStore::open_for_data_dir_read_only(&data_dir)
                    .map_err(|e| IpcError::unavailable(format!("本地缓存暂不可用: {e}"), None))?;
                cache
                    .find_inbox_folder(input.account_id)
                    .map_err(|e| IpcError::internal(format!("查询收件箱失败: {e}"), None))?
                    .map(|(_, path)| path)
                    .unwrap_or_else(|| "INBOX".to_string())
            }
        };
        self.execute_message_move(
            input.account_id,
            input.message_id,
            LiveMoveKind::Folder(target_inbox),
        )
    }

    pub fn list_tags(&self) -> Result<Vec<TagItemDto>, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        match self.mode {
            AppMode::Demo => Ok(Self::demo_tag_catalog()),
            AppMode::Live => {
                let data_dir = self
                    .live
                    .lock()
                    .expect("live mutex poisoned")
                    .data_dir
                    .clone();
                if !config::global_db_in(&data_dir).exists() {
                    return Ok(Vec::new());
                }
                let cache = CacheStore::open_for_data_dir_read_only(&data_dir)
                    .map_err(|e| IpcError::unavailable(format!("本地缓存暂不可用: {e}"), None))?;
                let items = cache
                    .list_tags()
                    .map_err(|e| IpcError::internal(format!("读取标签列表失败: {e}"), None))?;
                Ok(items
                    .into_iter()
                    .map(|t| TagItemDto {
                        id: t.id,
                        name: t.name,
                        color: t.color,
                        shortcut: t.shortcut,
                    })
                    .collect())
            }
        }
    }

    fn demo_tag_catalog() -> Vec<TagItemDto> {
        vec![
            TagItemDto {
                id: 1,
                name: "Work".to_string(),
                color: "#3584e4".to_string(),
                shortcut: Some(1),
            },
            TagItemDto {
                id: 2,
                name: "Personal".to_string(),
                color: "#26a269".to_string(),
                shortcut: Some(2),
            },
            TagItemDto {
                id: 3,
                name: "重要".to_string(),
                color: "#e01b24".to_string(),
                shortcut: Some(3),
            },
            TagItemDto {
                id: 4,
                name: "工作".to_string(),
                color: "#3584e4".to_string(),
                shortcut: Some(4),
            },
            TagItemDto {
                id: 5,
                name: "待办".to_string(),
                color: "#e66100".to_string(),
                shortcut: Some(5),
            },
            TagItemDto {
                id: 6,
                name: "财务".to_string(),
                color: "#10b981".to_string(),
                shortcut: Some(6),
            },
        ]
    }

    fn decorate_tag_pairs(&self, tags: &[String]) -> Vec<(String, String)> {
        let catalog = self.list_tags().unwrap_or_default();
        tags.iter()
            .map(|name| {
                let color = catalog
                    .iter()
                    .find(|t| t.name == *name)
                    .map(|t| t.color.clone())
                    .unwrap_or_else(|| "#6366f1".to_string());
                (name.clone(), color)
            })
            .collect()
    }

    fn resolve_card_tags_map(&self, detail: &MessageDetail) -> HashMap<u32, Vec<(String, String)>> {
        let mut out = HashMap::new();
        let catalog = self.list_tags().unwrap_or_default();
        let to_pairs = |names: &[String]| -> Vec<(String, String)> {
            names
                .iter()
                .map(|name| {
                    let color = catalog
                        .iter()
                        .find(|t| t.name == *name)
                        .map(|t| t.color.clone())
                        .unwrap_or_else(|| "#6366f1".to_string());
                    (name.clone(), color)
                })
                .collect()
        };
        match self.mode {
            AppMode::Demo => {
                let demo = self.demo.lock().expect("demo mutex poisoned");
                if let Some(s) = demo.find_summary(detail.account_id, detail.id) {
                    if !s.tags.is_empty() {
                        out.insert(detail.id, to_pairs(&s.tags));
                    }
                }
                for tm in &detail.thread_messages {
                    if let Some(s) = demo.find_summary(detail.account_id, tm.id) {
                        if !s.tags.is_empty() {
                            out.insert(tm.id, to_pairs(&s.tags));
                        }
                    }
                }
            }
            AppMode::Live => {
                let data_dir = self
                    .live
                    .lock()
                    .expect("live mutex poisoned")
                    .data_dir
                    .clone();
                if config::global_db_in(&data_dir).exists() {
                    if let Ok(cache) = CacheStore::open_for_data_dir_read_only(&data_dir) {
                        let mut uids = vec![detail.uid];
                        for tm in &detail.thread_messages {
                            if tm.uid != 0 && tm.folder_path == detail.folder_path {
                                uids.push(tm.uid);
                            }
                        }
                        if let Ok(summaries) = cache.list_summaries_by_uids(detail.folder_id, &uids)
                        {
                            for s in summaries {
                                if !s.tags.is_empty() {
                                    out.insert(s.id, to_pairs(&s.tags));
                                }
                            }
                        }
                    }
                }
            }
        }
        out
    }

    pub fn set_message_tags(
        &self,
        input: &SetMessageTagsInput,
    ) -> Result<SetMessageTagsOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        if input.account_id == 0 || input.message_id == 0 {
            return Err(IpcError::invalid_input(
                "accountId 与 messageId 必须为正整数",
                None,
            ));
        }
        let mut normalized = Vec::new();
        for raw in &input.tags {
            let trimmed = raw.trim();
            if !trimmed.is_empty() && !normalized.iter().any(|existing| existing == trimmed) {
                normalized.push(trimmed.to_string());
            }
        }

        let folder_id = match self.mode {
            AppMode::Demo => {
                let mut demo = self.demo.lock().expect("demo mutex poisoned");
                demo.store
                    .set_message_tags(input.message_id, normalized.clone());
                let summary = demo
                    .find_summary_mut(input.account_id, input.message_id)
                    .ok_or_else(|| IpcError::message_not_found(input.message_id, None))?;
                summary.tags = normalized.clone();
                Some(summary.folder_id)
            }
            AppMode::Live => {
                let (data_dir, worker) = {
                    let live = self.live.lock().expect("live mutex poisoned");
                    (live.data_dir.clone(), live.worker.clone())
                };
                let mut cache = CacheStore::open_for_data_dir(&data_dir)
                    .map_err(|e| IpcError::unavailable(format!("本地缓存暂不可用: {e}"), None))?;
                cache
                    .set_message_tags(input.message_id, &normalized)
                    .map_err(|e| IpcError::internal(format!("更新标签失败: {e}"), None))?;
                if let Some(w) = worker {
                    w.send(MailRequest::SetMessageTags {
                        account_id: input.account_id,
                        message_id: input.message_id,
                        tags: normalized.clone(),
                    });
                }
                None
            }
        };

        let tag_pairs = self.decorate_tag_pairs(&normalized);
        let raw_prefs = self.prefs.snapshot_raw();
        let appearance = self.current_reader_appearance(raw_prefs.block_remote_images);
        self.reader.rerender_matching_document(
            input.account_id,
            input.message_id,
            &appearance,
            |spec| {
                if let Some(ref mut thread) = spec.thread {
                    thread.card_tags.insert(input.message_id, tag_pairs);
                }
            },
        );

        let ver = self.bump_state_version();
        self.emitter.emit_mail_changed(MailChangedEvent {
            state_version: ver,
            reason: "tagsUpdated".to_string(),
            account_id: Some(input.account_id),
            folder_id,
            message_id: Some(input.message_id),
        });
        Ok(SetMessageTagsOutput {
            account_id: input.account_id,
            message_id: input.message_id,
            tags: normalized,
            state_version: ver,
        })
    }

    pub fn get_reader_view_options(
        &self,
        input: &GetReaderViewOptionsInput,
    ) -> Result<ReaderViewOptionsOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        let per_message_dark = self
            .per_message_dark
            .lock()
            .expect("per_message_dark mutex poisoned")
            .get(&(input.account_id, input.message_id))
            .copied();
        Ok(ReaderViewOptionsOutput {
            account_id: input.account_id,
            message_id: input.message_id,
            per_message_dark,
            reading_view: self.reading_view.load(Ordering::SeqCst),
        })
    }

    pub fn set_reader_view_options(
        &self,
        input: &SetReaderViewOptionsInput,
    ) -> Result<ReaderViewOptionsOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        if input.account_id == 0 || input.message_id == 0 {
            return Err(IpcError::invalid_input(
                "accountId 与 messageId 必须为正整数",
                None,
            ));
        }
        let next_dark = {
            let mut guard = self
                .per_message_dark
                .lock()
                .expect("per_message_dark mutex poisoned");
            if let Some(dark_opt) = input.per_message_dark {
                match dark_opt {
                    Some(v) => {
                        guard.insert((input.account_id, input.message_id), v);
                    }
                    None => {
                        guard.remove(&(input.account_id, input.message_id));
                    }
                }
            }
            guard.get(&(input.account_id, input.message_id)).copied()
        };
        let next_reading = if let Some(rv) = input.reading_view {
            self.reading_view.store(rv, Ordering::SeqCst);
            rv
        } else {
            self.reading_view.load(Ordering::SeqCst)
        };

        let raw_prefs = self.prefs.snapshot_raw();
        let appearance = self.current_reader_appearance(raw_prefs.block_remote_images);
        self.reader.rerender_matching_document(
            input.account_id,
            input.message_id,
            &appearance,
            |spec| {
                if let Some(ref mut thread) = spec.thread {
                    thread.reading_view = next_reading;
                    if let Some(dark_opt) = input.per_message_dark {
                        match dark_opt {
                            Some(v) => {
                                thread.per_message_dark.insert(input.message_id, v);
                                for tm in &thread.detail.thread_messages {
                                    thread.per_message_dark.insert(tm.id, v);
                                }
                            }
                            None => {
                                thread.per_message_dark.remove(&input.message_id);
                                for tm in &thread.detail.thread_messages {
                                    thread.per_message_dark.remove(&tm.id);
                                }
                            }
                        }
                    }
                }
            },
        );

        Ok(ReaderViewOptionsOutput {
            account_id: input.account_id,
            message_id: input.message_id,
            per_message_dark: next_dark,
            reading_view: next_reading,
        })
    }

    pub fn find_in_reader(&self, input: &ReaderFindInput) -> Result<ReaderFindOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        let dispatched =
            self.reader
                .find_in_reader(&input.reader_session_id, &input.query, input.step)?;
        Ok(ReaderFindOutput { dispatched })
    }

    fn execute_message_move(
        &self,
        account_id: u32,
        message_id: u32,
        kind: LiveMoveKind,
    ) -> Result<MessageMoveOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        if account_id == 0 || message_id == 0 {
            return Err(IpcError::invalid_input(
                "accountId 与 messageId 必须为正整数",
                None,
            ));
        }

        match self.mode {
            AppMode::Demo => {
                let (rfc_message_id, original_folder_path, target_folder_path, folder_id) = {
                    let mut demo = self.demo.lock().expect("demo mutex poisoned");
                    let target = match &kind {
                        LiveMoveKind::Archive => Some("Archive"),
                        LiveMoveKind::Spam => Some("Junk"),
                        LiveMoveKind::Delete => {
                            let in_trash = demo
                                .find_summary(account_id, message_id)
                                .is_some_and(|m| m.folder_path.eq_ignore_ascii_case("Trash"));
                            if in_trash {
                                None
                            } else {
                                Some("Trash")
                            }
                        }
                        LiveMoveKind::Folder(path) => Some(path.as_str()),
                    };
                    demo.move_message_to(account_id, message_id, target)?
                };
                let reason = match &kind {
                    LiveMoveKind::Archive => "messageArchived",
                    LiveMoveKind::Delete => "messageDeleted",
                    LiveMoveKind::Spam => "messageMarkedSpam",
                    LiveMoveKind::Folder(_) => "messageMoved",
                };
                let ver = self.bump_state_version();
                self.emitter.emit_mail_changed(MailChangedEvent {
                    state_version: ver,
                    reason: reason.to_string(),
                    account_id: Some(account_id),
                    folder_id,
                    message_id: Some(message_id),
                });
                self.refresh_tray_tooltip();
                Ok(MessageMoveOutput {
                    account_id,
                    message_id,
                    rfc_message_id,
                    original_folder_path,
                    target_folder_path,
                    state_version: ver,
                })
            }
            AppMode::Live => {
                let (data_dir, auth_backend, worker) = {
                    let live = self.live.lock().expect("live mutex poisoned");
                    let auth = live
                        .account_configs
                        .iter()
                        .find(|a| a.id == account_id)
                        .map(|a| a.auth_backend)
                        .unwrap_or(AuthBackend::Password);
                    (live.data_dir.clone(), auth, live.worker.clone())
                };
                let cache = CacheStore::open_for_data_dir_read_only(&data_dir)
                    .map_err(|e| IpcError::unavailable(format!("本地缓存暂不可用: {e}"), None))?;
                let Some((detail, _, _)) = cache
                    .get_message_detail(message_id)
                    .map_err(|e| IpcError::internal(format!("查询邮件详情失败: {e}"), None))?
                else {
                    return Err(IpcError::message_not_found(message_id, None));
                };
                if detail.account_id != account_id {
                    return Err(IpcError::message_not_found(message_id, None));
                }

                let target_folder_path = match &kind {
                    LiveMoveKind::Archive => match cache.find_archive_folder(account_id) {
                        Ok(Some((_, path))) => path,
                        Ok(None) => {
                            return Err(IpcError::unavailable(
                                "该账号没有归档文件夹，无法归档邮件",
                                None,
                            ))
                        }
                        Err(e) => {
                            return Err(IpcError::internal(
                                format!("查询归档文件夹失败: {e}"),
                                None,
                            ))
                        }
                    },
                    LiveMoveKind::Delete => match cache.find_trash_folder(account_id) {
                        Ok(Some((_, path))) => path,
                        Ok(None) => {
                            if auth_backend == AuthBackend::MicrosoftGraphOAuth2 {
                                "deleteditems".to_string()
                            } else {
                                String::new()
                            }
                        }
                        Err(e) => {
                            return Err(IpcError::internal(format!("查询废纸篓失败: {e}"), None))
                        }
                    },
                    LiveMoveKind::Spam => match cache.find_spam_folder(account_id) {
                        Ok(Some((_, path))) => path,
                        Ok(None) => {
                            if auth_backend == AuthBackend::MicrosoftGraphOAuth2 {
                                "junkemail".to_string()
                            } else {
                                return Err(IpcError::unavailable(
                                    "该账号没有垃圾邮件文件夹，无法标记为垃圾邮件",
                                    None,
                                ));
                            }
                        }
                        Err(e) => {
                            return Err(IpcError::internal(
                                format!("查询垃圾邮件文件夹失败: {e}"),
                                None,
                            ))
                        }
                    },
                    LiveMoveKind::Folder(path) => path.clone(),
                };

                {
                    let mut live = self.live.lock().expect("live mutex poisoned");
                    live.remove_cached_detail(account_id, message_id);
                }

                let req = match kind {
                    LiveMoveKind::Archive => MailRequest::ArchiveMessage {
                        account_id,
                        message_id,
                    },
                    LiveMoveKind::Delete => MailRequest::DeleteMessage {
                        account_id,
                        message_id,
                    },
                    LiveMoveKind::Spam => MailRequest::MarkSpam {
                        account_id,
                        message_id,
                    },
                    LiveMoveKind::Folder(ref path) => MailRequest::MoveMessage {
                        account_id,
                        message_id,
                        target_folder_path: path.clone(),
                    },
                };
                if let Some(w) = worker {
                    w.send(req);
                }

                let ver = self.bump_state_version();
                Ok(MessageMoveOutput {
                    account_id,
                    message_id,
                    rfc_message_id: detail.rfc_message_id,
                    original_folder_path: detail.folder_path,
                    target_folder_path,
                    state_version: ver,
                })
            }
        }
    }

    pub fn undo_message_move(
        &self,
        input: &UndoMessageMoveInput,
    ) -> Result<UndoMessageMoveOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        if input.account_id == 0 || input.message_id == 0 {
            return Err(IpcError::invalid_input(
                "accountId 与 messageId 必须为正整数",
                None,
            ));
        }
        if input.current_folder_path.trim().is_empty()
            || input.original_folder_path.trim().is_empty()
        {
            return Err(IpcError::invalid_input(
                "currentFolderPath 与 originalFolderPath 不能为空",
                None,
            ));
        }

        match self.mode {
            AppMode::Demo => {
                let (accepted, folder_id) = {
                    let mut demo = self.demo.lock().expect("demo mutex poisoned");
                    demo.undo_move(
                        input.account_id,
                        input.message_id,
                        &input.rfc_message_id,
                        &input.current_folder_path,
                        &input.original_folder_path,
                    )?
                };
                let ver = if accepted {
                    let v = self.bump_state_version();
                    self.emitter.emit_mail_changed(MailChangedEvent {
                        state_version: v,
                        reason: "undoMove".to_string(),
                        account_id: Some(input.account_id),
                        folder_id,
                        message_id: Some(input.message_id),
                    });
                    self.refresh_tray_tooltip();
                    v
                } else {
                    self.state_version()
                };
                Ok(UndoMessageMoveOutput {
                    accepted,
                    state_version: ver,
                })
            }
            AppMode::Live => {
                let worker = {
                    let live = self.live.lock().expect("live mutex poisoned");
                    live.worker.clone()
                };
                let accepted = if let Some(w) = worker {
                    w.send(MailRequest::UndoMessageMove {
                        account_id: input.account_id,
                        message_id: input.message_id,
                        rfc_message_id: input.rfc_message_id.clone(),
                        current_folder_path: input.current_folder_path.clone(),
                        original_folder_path: input.original_folder_path.clone(),
                    });
                    true
                } else {
                    false
                };
                let ver = self.bump_state_version();
                Ok(UndoMessageMoveOutput {
                    accepted,
                    state_version: ver,
                })
            }
        }
    }
}

enum LiveMoveKind {
    Archive,
    Delete,
    Spam,
    Folder(String),
}

impl ThreadActionHandler for MailSession {
    fn handle_thread_action(&self, account_id: u32, message_id: u32, action: WebViewAction) {
        if self.is_locked() {
            return;
        }
        match action {
            WebViewAction::ScrollAnchor { msg_id, offset_px } => {
                self.reader
                    .update_scroll_anchor(account_id, message_id, msg_id, offset_px);
            }
            WebViewAction::CardFold { msg_id, collapsed } => {
                {
                    let mut guard = self
                        .thread_collapsed_cards
                        .lock()
                        .expect("thread_collapsed_cards mutex poisoned");
                    if guard.0 != Some((account_id, message_id)) {
                        guard.0 = Some((account_id, message_id));
                        guard.1.clear();
                    }
                    if collapsed {
                        guard.1.insert(msg_id);
                    } else {
                        guard.1.remove(&msg_id);
                    }
                }
                let raw_prefs = self.prefs.snapshot_raw();
                let appearance = self.current_reader_appearance(raw_prefs.block_remote_images);
                self.reader.rerender_matching_document(
                    account_id,
                    message_id,
                    &appearance,
                    |spec| {
                        if let Some(ref mut thread) = spec.thread {
                            if collapsed {
                                thread.collapsed_cards.insert(msg_id);
                            } else {
                                thread.collapsed_cards.remove(&msg_id);
                            }
                        }
                    },
                );
            }
            WebViewAction::CardToggleRaw(msg_id) => {
                let show_raw = {
                    let mut guard = self
                        .thread_raw_cards
                        .lock()
                        .expect("thread_raw_cards mutex poisoned");
                    if guard.0 != Some((account_id, message_id)) {
                        guard.0 = Some((account_id, message_id));
                        guard.1.clear();
                    }
                    if !guard.1.remove(&msg_id) {
                        guard.1.insert(msg_id);
                        true
                    } else {
                        false
                    }
                };
                if show_raw {
                    let mut guard = self
                        .thread_collapsed_cards
                        .lock()
                        .expect("thread_collapsed_cards mutex poisoned");
                    if guard.0 == Some((account_id, message_id)) {
                        guard.1.remove(&msg_id);
                    }
                }
                let raw_prefs = self.prefs.snapshot_raw();
                let appearance = self.current_reader_appearance(raw_prefs.block_remote_images);
                self.reader.rerender_matching_document(
                    account_id,
                    message_id,
                    &appearance,
                    |spec| {
                        if let Some(ref mut thread) = spec.thread {
                            if show_raw {
                                thread.raw_cards.insert(msg_id);
                                thread.collapsed_cards.remove(&msg_id);
                            } else {
                                thread.raw_cards.remove(&msg_id);
                            }
                        }
                    },
                );
            }
            WebViewAction::CardAllowRemote(msg_id) => {
                let _ = self.allow_remote_images(&AllowRemoteImagesInput {
                    account_id,
                    message_id: msg_id,
                    trust_sender: false,
                });
            }
            WebViewAction::CardToggleFlag(msg_id) => {
                let current_flagged = self
                    .load_detail_for_action(account_id, message_id)
                    .ok()
                    .and_then(|d| {
                        d.thread_messages
                            .iter()
                            .find(|tm| tm.id == msg_id)
                            .map(|tm| tm.flagged)
                            .or_else(|| (d.id == msg_id).then_some(d.flagged))
                    })
                    .unwrap_or(false);
                let next_flagged = !current_flagged;
                if self
                    .set_message_flagged(&SetMessageFlaggedInput {
                        account_id,
                        message_id: msg_id,
                        flagged: next_flagged,
                    })
                    .is_ok()
                {
                    let raw_prefs = self.prefs.snapshot_raw();
                    let appearance = self.current_reader_appearance(raw_prefs.block_remote_images);
                    self.reader.rerender_matching_document(
                        account_id,
                        message_id,
                        &appearance,
                        |spec| {
                            if let Some(ref mut thread) = spec.thread {
                                if thread.detail.id == msg_id {
                                    thread.detail.flagged = next_flagged;
                                }
                                for tm in &mut thread.detail.thread_messages {
                                    if tm.id == msg_id {
                                        tm.flagged = next_flagged;
                                    }
                                }
                            }
                        },
                    );
                }
            }
            WebViewAction::CardMarkUnread(msg_id) => {
                let _ = self.set_message_seen(&SetMessageSeenInput {
                    account_id,
                    message_id: msg_id,
                    seen: false,
                });
            }
            WebViewAction::CardArchive(msg_id) => {
                let _ = self.archive_message(&MessageActionInput {
                    account_id,
                    message_id: msg_id,
                });
            }
            WebViewAction::CardDelete(msg_id) => {
                let _ = self.delete_message(&MessageActionInput {
                    account_id,
                    message_id: msg_id,
                });
            }
            WebViewAction::CardLoadBody(msg_id) => {
                if self.mode == AppMode::Live {
                    if let Some(w) = self
                        .live
                        .lock()
                        .expect("live mutex poisoned")
                        .worker
                        .clone()
                    {
                        w.send(MailRequest::LoadMessageBody {
                            account_id,
                            message_id: msg_id,
                        });
                    }
                }
            }
            WebViewAction::CardUnsubscribe(msg_id) => {
                if self.mode == AppMode::Live && msg_id == message_id {
                    if let Ok(detail) = self.load_detail_for_action(account_id, message_id) {
                        if detail.unsubscribe_one_click {
                            if let Some(url) = detail.unsubscribe_url {
                                if let Some(w) = self
                                    .live
                                    .lock()
                                    .expect("live mutex poisoned")
                                    .worker
                                    .clone()
                                {
                                    w.send(MailRequest::UnsubscribeOneClick { url });
                                }
                            }
                        }
                    }
                }
            }
            WebViewAction::CardIcsReply { msg_id, status } => {
                if let Ok(detail) = self.load_detail_for_action(account_id, message_id) {
                    if let Some(tm) = detail.thread_messages.iter().find(|m| m.id == msg_id) {
                        if tm.body_text.contains("BEGIN:VCALENDAR") {
                            if let Some(inv) = kitemail_mail_core::invite::parse(&tm.body_text) {
                                let _ = kitemail_mail_core::invite::reply_ics(&inv, "", "", status);
                            }
                        }
                    }
                }
            }
            WebViewAction::CardReply(msg_id) => {
                self.emitter.emit_thread_card_action(ThreadCardActionEvent {
                    kind: ThreadCardActionKindDto::Reply,
                    account_id,
                    message_id,
                    target_id: Some(msg_id),
                    text: None,
                });
            }
            WebViewAction::CardReplyAll(msg_id) => {
                self.emitter.emit_thread_card_action(ThreadCardActionEvent {
                    kind: ThreadCardActionKindDto::ReplyAll,
                    account_id,
                    message_id,
                    target_id: Some(msg_id),
                    text: None,
                });
            }
            WebViewAction::CardForward(msg_id) => {
                self.emitter.emit_thread_card_action(ThreadCardActionEvent {
                    kind: ThreadCardActionKindDto::Forward,
                    account_id,
                    message_id,
                    target_id: Some(msg_id),
                    text: None,
                });
            }
            WebViewAction::CardOpenAttachment(att_id) => {
                self.emitter.emit_thread_card_action(ThreadCardActionEvent {
                    kind: ThreadCardActionKindDto::OpenAttachment,
                    account_id,
                    message_id,
                    target_id: Some(att_id),
                    text: None,
                });
            }
            WebViewAction::CardSaveAttachment(att_id) => {
                let text = match self.save_attachment(&SaveAttachmentInput {
                    attachment_id: att_id,
                    dest_path: None,
                }) {
                    Ok(out) => Some(out.message),
                    Err(err) => Some(err.message),
                };
                self.emitter.emit_thread_card_action(ThreadCardActionEvent {
                    kind: ThreadCardActionKindDto::SaveAttachment,
                    account_id,
                    message_id,
                    target_id: Some(att_id),
                    text,
                });
            }
            WebViewAction::CardSaveAllAttachments(msg_id) => {
                let att_ids: Vec<u32> = self
                    .load_detail_for_action(account_id, message_id)
                    .ok()
                    .map(|detail| {
                        detail
                            .thread_messages
                            .iter()
                            .find(|tm| tm.id == msg_id)
                            .map(|tm| tm.attachments.iter().map(|a| a.id).collect())
                            .unwrap_or_else(|| {
                                if detail.id == msg_id {
                                    detail.attachments.iter().map(|a| a.id).collect()
                                } else {
                                    Vec::new()
                                }
                            })
                    })
                    .unwrap_or_default();
                let text = if att_ids.is_empty() {
                    Some("当前卡片没有可保存的附件".to_string())
                } else {
                    match self.save_all_attachments(&SaveAllAttachmentsInput {
                        attachment_ids: att_ids,
                        dest_dir: None,
                    }) {
                        Ok(out) => Some(out.message),
                        Err(err) => Some(err.message),
                    }
                };
                self.emitter.emit_thread_card_action(ThreadCardActionEvent {
                    kind: ThreadCardActionKindDto::SaveAllAttachments,
                    account_id,
                    message_id,
                    target_id: Some(msg_id),
                    text,
                });
            }
            WebViewAction::CardCopyBody(msg_id) => {
                let body = self
                    .load_detail_for_action(account_id, message_id)
                    .ok()
                    .and_then(|detail| {
                        detail
                            .thread_messages
                            .iter()
                            .find(|tm| tm.id == msg_id)
                            .map(|tm| tm.body_text.clone())
                            .or_else(|| (detail.id == msg_id).then_some(detail.body_text))
                    });
                self.emitter.emit_thread_card_action(ThreadCardActionEvent {
                    kind: ThreadCardActionKindDto::CopyBody,
                    account_id,
                    message_id,
                    target_id: Some(msg_id),
                    text: body,
                });
            }
            WebViewAction::CardExportEml(msg_id) => {
                self.emitter.emit_thread_card_action(ThreadCardActionEvent {
                    kind: ThreadCardActionKindDto::ExportEml,
                    account_id,
                    message_id,
                    target_id: Some(msg_id),
                    text: None,
                });
            }
            WebViewAction::CardMdn { msg_id, send } => {
                self.emitter.emit_thread_card_action(ThreadCardActionEvent {
                    kind: ThreadCardActionKindDto::Mdn,
                    account_id,
                    message_id,
                    target_id: Some(msg_id),
                    text: Some(if send { "accept" } else { "ignore" }.to_string()),
                });
            }
            WebViewAction::FindCount { current, total } => {
                let reader_session_id = self
                    .active_reader_session_id
                    .lock()
                    .expect("active_reader_session_id mutex poisoned")
                    .clone()
                    .unwrap_or_default();
                self.emitter.emit_reader_find_result(ReaderFindResultEvent {
                    reader_session_id,
                    current,
                    total,
                });
            }
        }
    }
}

fn is_detail_unread_with_cache(cache: &CacheStore, detail: &MessageDetail) -> bool {
    if let Some(tm) = detail.thread_messages.iter().find(|tm| tm.id == detail.id) {
        return tm.unread;
    }
    cache
        .list_summaries_by_uids(detail.folder_id, &[detail.uid])
        .ok()
        .and_then(|list| list.into_iter().find(|m| m.id == detail.id))
        .is_some_and(|m| m.unread)
}

fn is_detail_unread(data_dir: &Path, detail: &MessageDetail) -> bool {
    if let Some(tm) = detail.thread_messages.iter().find(|tm| tm.id == detail.id) {
        return tm.unread;
    }
    if let Ok(cache) = CacheStore::open_for_data_dir_read_only(data_dir) {
        return is_detail_unread_with_cache(&cache, detail);
    }
    false
}

fn is_inbox_folder(folder: &FolderSummaryDto) -> bool {
    let name = folder.name.trim();
    folder.path.eq_ignore_ascii_case("INBOX")
        || name == "收件箱"
        || name.eq_ignore_ascii_case("Inbox")
}

#[allow(clippy::too_many_arguments)]
fn execute_live_page_query(
    cache: &CacheStore,
    input: &ListMessagesInput,
    cross_folder: bool,
    scoped_enabled_ids: &[u32],
    scoped_unified_ids: &[u32],
    quick: QuickFilterSet,
    sort: MessageSort,
    limit: usize,
) -> Result<MailPage, String> {
    if cross_folder {
        return cache.search_all_folders(
            scoped_enabled_ids,
            &input.query,
            quick,
            sort,
            input.offset,
            limit,
        );
    }
    match input.view {
        MainViewDto::UnifiedInbox => cache.list_unified_inbox_for_accounts(
            scoped_unified_ids,
            &input.query,
            quick,
            sort,
            input.offset,
            limit,
        ),
        MainViewDto::Unread => cache.list_unread_messages_for_accounts(
            scoped_unified_ids,
            &input.query,
            quick,
            sort,
            input.offset,
            limit,
        ),
        MainViewDto::Starred => cache.list_starred_messages_for_accounts(
            scoped_unified_ids,
            &input.query,
            quick,
            sort,
            input.offset,
            limit,
        ),
        MainViewDto::Sent => cache.list_sent_messages_for_accounts(
            scoped_unified_ids,
            &input.query,
            quick,
            sort,
            input.offset,
            limit,
        ),
        MainViewDto::Archive => cache.list_archived_messages_for_accounts(
            scoped_unified_ids,
            &input.query,
            quick,
            sort,
            input.offset,
            limit,
        ),
        MainViewDto::Junk => cache.list_junk_messages_for_accounts(
            scoped_unified_ids,
            &input.query,
            quick,
            sort,
            input.offset,
            limit,
        ),
        MainViewDto::Trash => cache.list_trash_messages_for_accounts(
            scoped_unified_ids,
            &input.query,
            quick,
            sort,
            input.offset,
            limit,
        ),
        MainViewDto::Snoozed => cache.list_snoozed_messages_for_accounts(
            scoped_unified_ids,
            &input.query,
            quick,
            sort,
            input.offset,
            limit,
        ),
        MainViewDto::Filters => cache.search_all_folders(
            scoped_enabled_ids,
            &input.query,
            quick,
            sort,
            input.offset,
            limit,
        ),
        MainViewDto::Tags => cache.search_all_folders(
            scoped_enabled_ids,
            &input.query,
            QuickFilterSet {
                tagged: true,
                ..quick
            },
            sort,
            input.offset,
            limit,
        ),
        MainViewDto::Drafts => list_live_drafts_page(cache, &input.query, input.offset, limit),
        MainViewDto::AccountFolder { folder_id, .. } => cache.list_messages_page_with_options(
            folder_id,
            &input.query,
            quick,
            sort,
            input.offset,
            limit,
        ),
    }
}

fn list_live_drafts_page(
    cache: &CacheStore,
    query: &str,
    offset: usize,
    limit: usize,
) -> Result<MailPage, String> {
    let drafts = cache.list_drafts()?;
    let q = query.trim().to_ascii_lowercase();
    let mut items: Vec<MessageSummary> = drafts
        .into_iter()
        .filter(|d| {
            q.is_empty()
                || d.subject.to_ascii_lowercase().contains(&q)
                || d.to_addr.to_ascii_lowercase().contains(&q)
                || d.body_text.to_ascii_lowercase().contains(&q)
        })
        .map(|d| MessageSummary {
            id: d.id,
            account_id: d.account_id,
            folder_id: 0,
            folder_path: "Drafts".to_string(),
            uid: d.id,
            sender: if d.from_addr.trim().is_empty() {
                "草稿".to_string()
            } else {
                d.from_addr
            },
            subject: if d.subject.trim().is_empty() {
                "(无主题)".to_string()
            } else {
                d.subject
            },
            preview: d.body_text.chars().take(120).collect(),
            date_ts: d.updated_at,
            date_display: String::new(),
            unread: false,
            flagged: false,
            has_attachment: !d.attachment_paths.is_empty(),
            recipients: d.to_addr,
            thread_count: 1,
            importance: 0,
            tags: Vec::new(),
            answered: false,
            forwarded: false,
            snoozed_until_ts: 0,
        })
        .collect();
    items.sort_by(|a, b| b.date_ts.cmp(&a.date_ts));
    let total = items.len();
    let start = offset.min(total);
    let end = start.saturating_add(limit).min(total);
    Ok(MailPage {
        folder_id: 0,
        offset: start,
        total,
        items: items[start..end].to_vec(),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::dto::{MainPreferencesPatch, MessageSortDto};
    use crate::reader::NullReaderSink;
    use kitemail_mail_core::cache::{CachedHeaderInput, RemoteFolder};
    use kitemail_mail_core::config::{save_accounts_in, AccountConfig};

    #[derive(Default)]
    pub(crate) struct TestEmitter {
        pub(crate) mail_changed: Mutex<Vec<MailChangedEvent>>,
        pub(crate) sync_status: Mutex<Vec<SyncStatusEvent>>,
        pub(crate) lock_state: Mutex<Vec<LockStateEvent>>,
        pub(crate) outbox_updated: Mutex<Vec<OutboxUpdatedEvent>>,
        pub(crate) draft_saved: Mutex<Vec<DraftSavedEvent>>,
        pub(crate) account_events: Mutex<Vec<AccountEvent>>,
        pub(crate) thread_card_actions: Mutex<Vec<ThreadCardActionEvent>>,
        pub(crate) reader_find_results: Mutex<Vec<ReaderFindResultEvent>>,
    }

    impl MainEventEmitter for TestEmitter {
        fn emit_mail_changed(&self, event: MailChangedEvent) {
            self.mail_changed.lock().unwrap().push(event);
        }

        fn emit_sync_status(&self, event: SyncStatusEvent) {
            self.sync_status.lock().unwrap().push(event);
        }

        fn emit_lock_state(&self, event: LockStateEvent) {
            self.lock_state.lock().unwrap().push(event);
        }

        fn emit_outbox_updated(&self, event: OutboxUpdatedEvent) {
            self.outbox_updated.lock().unwrap().push(event);
        }

        fn emit_draft_saved(&self, event: DraftSavedEvent) {
            self.draft_saved.lock().unwrap().push(event);
        }

        fn emit_account_event(&self, event: AccountEvent) {
            self.account_events.lock().unwrap().push(event);
        }

        fn emit_thread_card_action(&self, event: ThreadCardActionEvent) {
            self.thread_card_actions.lock().unwrap().push(event);
        }

        fn emit_reader_find_result(&self, event: ReaderFindResultEvent) {
            self.reader_find_results.lock().unwrap().push(event);
        }
    }

    #[derive(Default)]
    pub(crate) struct RecordingReaderSink {
        pub(crate) events: Mutex<Vec<ReaderStatusEvent>>,
    }

    impl crate::reader::ReaderPlatformSink for RecordingReaderSink {
        fn is_native_supported(&self) -> bool {
            false
        }
        fn load_html(
            &self,
            _: String,
            _: u32,
            _: u32,
            _: Vec<u8>,
            _: bool,
            _: kitemail_windows_mail_reader::DocumentMode,
        ) {
        }
        fn apply_layout(
            &self,
            _: crate::window_lifecycle::PhysicalRect,
            _: crate::window_lifecycle::PhysicalRect,
            _: bool,
        ) -> Result<(), String> {
            Ok(())
        }
        fn set_visible(&self, _: bool) {}
        fn set_zoom(&self, _: u32) {}
        fn close(&self) {}
        fn emit_status(&self, event: ReaderStatusEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    #[derive(Default)]
    pub(crate) struct RecordingWorkerSink {
        pub(crate) requests: Mutex<Vec<MailRequest>>,
    }

    impl LiveWorkerSink for RecordingWorkerSink {
        fn send(&self, req: MailRequest) {
            self.requests.lock().unwrap().push(req);
        }
        fn prepare_quit(&self) {}
    }

    pub(crate) fn make_test_session() -> (MailSession, Arc<TestEmitter>, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("kitemail-tauri-session-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let prefs = Arc::new(PreferencesStore::open_in(dir.clone()));
        let reader = Arc::new(ReaderManager::new(Arc::new(NullReaderSink)));
        let emitter = Arc::new(TestEmitter::default());
        let session = MailSession::new(AppMode::Demo, prefs, reader, emitter.clone());
        (session, emitter, dir)
    }

    pub(crate) fn make_live_test_session(
        with_cache: bool,
        passcode: Option<&str>,
    ) -> (
        Arc<MailSession>,
        Arc<TestEmitter>,
        Arc<RecordingReaderSink>,
        Arc<RecordingWorkerSink>,
        PathBuf,
    ) {
        let dir =
            std::env::temp_dir().join(format!("kitemail-tauri-live-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let acc = AccountConfig {
            id: 1,
            display_name: "测试账号".to_string(),
            email: "tester@example.com".to_string(),
            ..AccountConfig::default()
        };
        save_accounts_in(&dir, std::slice::from_ref(&acc)).unwrap();

        let mut folders_by_account = HashMap::new();
        let mut cache_ready = false;
        if with_cache {
            let mut store = CacheStore::open_for_data_dir(&dir).unwrap();
            let folders = store
                .sync_folders(
                    1,
                    &[RemoteFolder {
                        name: "收件箱".to_string(),
                        path: "INBOX".to_string(),
                        unread_count: 2,
                        total_count: 2,
                    }],
                )
                .unwrap();
            let inbox_id = folders[0].id;
            store
                .commit_folder_headers(
                    1,
                    inbox_id,
                    1,
                    &[
                        CachedHeaderInput {
                            uid: 10,
                            message_id: "<m10@example.com>".to_string(),
                            in_reply_to: String::new(),
                            references_header: String::new(),
                            sender: "Alice <alice@example.com>".to_string(),
                            recipients: "tester@example.com".to_string(),
                            cc: String::new(),
                            subject: "未缓存正文的邮件 A".to_string(),
                            preview: "预览 A".to_string(),
                            date_ts: 1_700_000_200,
                            date_display: "10:00".to_string(),
                            unread: true,
                            flagged: false,
                            has_attachment: false,
                        },
                        CachedHeaderInput {
                            uid: 11,
                            message_id: "<m11@example.com>".to_string(),
                            in_reply_to: String::new(),
                            references_header: String::new(),
                            sender: "Bob <bob@example.com>".to_string(),
                            recipients: "tester@example.com".to_string(),
                            cc: String::new(),
                            subject: "已缓存正文的邮件 B".to_string(),
                            preview: "预览 B".to_string(),
                            date_ts: 1_700_000_100,
                            date_display: "09:00".to_string(),
                            unread: true,
                            flagged: false,
                            has_attachment: false,
                        },
                    ],
                    None,
                )
                .unwrap();
            let page = store.list_messages_page(inbox_id, "", 0, 10).unwrap();
            let msg_b_id = page.items.iter().find(|m| m.uid == 11).unwrap().id;
            store
                .store_message_body_with_html(
                    msg_b_id,
                    "这是邮件 B 的缓存正文",
                    None,
                    &[],
                    None,
                    false,
                    None,
                )
                .unwrap();
            store.checkpoint_and_close_all();

            let ro = CacheStore::open_for_data_dir_read_only(&dir).unwrap();
            folders_by_account.insert(
                1,
                ro.list_folders(1)
                    .unwrap()
                    .iter()
                    .map(FolderSummaryDto::from)
                    .collect(),
            );
            cache_ready = true;
        }

        let prefs = Arc::new(PreferencesStore::open_in(dir.clone()));
        let reader_sink = Arc::new(RecordingReaderSink::default());
        let reader = Arc::new(ReaderManager::new(reader_sink.clone()));
        let emitter = Arc::new(TestEmitter::default());
        let worker = Arc::new(RecordingWorkerSink::default());

        let passcode_hash = passcode.map(|p| app_lock::hash_passcode(p).unwrap());
        let persisted_lock = passcode_hash.is_some() && app_lock::is_lock_persisted_in(&dir);
        if persisted_lock {
            reader.set_locked(true);
        }

        let mut live = LiveSessionState::empty();
        live.data_dir = dir.clone();
        live.worker = Some(worker.clone());
        live.account_configs = vec![acc.clone()];
        live.accounts = vec![AccountSummaryDto::from(&acc)];
        live.folders_by_account = folders_by_account;
        live.cache_ready = cache_ready;
        live.snapshots_hydrated = cache_ready;

        let session = Arc::new(MailSession {
            mode: AppMode::Live,
            locked: AtomicBool::new(persisted_lock),
            quitting: AtomicBool::new(false),
            state_version: AtomicU64::new(1),
            read_mark_gen: AtomicU64::new(0),
            active_selected_msg: Mutex::new(None),
            active_reader_session_id: Mutex::new(None),
            allowed_remote_messages: Mutex::new(HashSet::new()),
            per_message_dark: Mutex::new(HashMap::new()),
            reading_view: AtomicBool::new(false),
            thread_collapsed_cards: Mutex::new((None, HashSet::new())),
            thread_raw_cards: Mutex::new((None, HashSet::new())),
            current_view: Mutex::new(MainViewDto::UnifiedInbox),
            sync_status: Mutex::new(SyncStatusDto::default()),
            lock_state: Mutex::new(AppLockState::new(
                passcode_hash,
                persisted_lock,
                Some(dir.clone()),
            )),
            demo: Mutex::new(DemoSessionState::new()),
            live: Mutex::new(live),
            read_cache: Mutex::new(None),
            prefs,
            reader,
            emitter: emitter.clone(),
            self_weak: Mutex::new(Weak::new()),
            tray: Mutex::new(None),
            auto_sync_signal: Arc::new((Mutex::new(false), Condvar::new())),
            last_auto_sync_at: Mutex::new(Instant::now()),
            body_fetch_timeout: Duration::from_millis(120),
            read_mark_delay: Duration::from_millis(80),
        });
        session.bind_self(&session);
        (session, emitter, reader_sink, worker, dir)
    }

    #[test]
    fn demo_session_paginates_filters_sorts_and_searches() {
        let (session, _, dir) = make_test_session();
        let boot = session.bootstrap();
        assert_eq!(boot.mode, AppMode::Demo);
        assert!(boot.cache_ready);
        assert_eq!(boot.accounts.len(), 3);
        assert_eq!(boot.accounts[0].display_name, "Jason M.");
        assert_eq!(boot.accounts[1].display_name, "Hyprlab");
        assert_eq!(boot.accounts[2].display_name, "Jason (Personal)");

        let inbox1 = boot
            .folders
            .iter()
            .find(|f| f.account_id == 1 && f.path == "INBOX")
            .expect("account 1 inbox");
        assert_eq!(inbox1.total_count, 10);
        assert_eq!(inbox1.unread_count, 6);

        let page0 = session
            .list_messages(&ListMessagesInput {
                request_id: "r1".to_string(),
                view: MainViewDto::UnifiedInbox,
                query: String::new(),
                scope: SearchScopeDto::ThisFolder,
                sort: MessageSortDto::DateNewest,
                filter: MessageFilterDto::default(),
                offset: 0,
                limit: 50,
            })
            .expect("page 0");
        assert_eq!(page0.total, 18);
        assert_eq!(page0.items.len(), 18);
        assert!(!page0.has_more);

        let page_limited = session
            .list_messages(&ListMessagesInput {
                request_id: "r2".to_string(),
                view: MainViewDto::UnifiedInbox,
                query: String::new(),
                scope: SearchScopeDto::ThisFolder,
                sort: MessageSortDto::DateNewest,
                filter: MessageFilterDto::default(),
                offset: 10,
                limit: 5,
            })
            .expect("middle page");
        assert_eq!(page_limited.items.len(), 5);
        assert!(page_limited.has_more);

        let acct2_inbox = session
            .list_messages(&ListMessagesInput {
                request_id: "r2-acct2".to_string(),
                view: MainViewDto::AccountFolder {
                    account_id: 2,
                    folder_id: 11,
                },
                query: String::new(),
                scope: SearchScopeDto::ThisFolder,
                sort: MessageSortDto::DateNewest,
                filter: MessageFilterDto::default(),
                offset: 0,
                limit: 50,
            })
            .expect("account 2 inbox");
        assert_eq!(acct2_inbox.total, 4);
        assert!(acct2_inbox.items.iter().all(|m| m.account_id == 2));

        let unread = session
            .list_messages(&ListMessagesInput {
                request_id: "r3".to_string(),
                view: MainViewDto::UnifiedInbox,
                query: String::new(),
                scope: SearchScopeDto::ThisFolder,
                sort: MessageSortDto::DateNewest,
                filter: MessageFilterDto {
                    unread: Some(true),
                    ..Default::default()
                },
                offset: 0,
                limit: 50,
            })
            .expect("unread filter");
        assert_eq!(unread.total, 12);
        assert!(unread.items.iter().all(|m| m.unread));

        let search = session
            .list_messages(&ListMessagesInput {
                request_id: "r4".to_string(),
                view: MainViewDto::UnifiedInbox,
                query: "roadmap".to_string(),
                scope: SearchScopeDto::AllFolders,
                sort: MessageSortDto::DateOldest,
                filter: MessageFilterDto::default(),
                offset: 0,
                limit: 50,
            })
            .expect("cross-folder search");
        assert_eq!(search.total, 2);
        assert!(search.items[0].date_ts <= search.items[1].date_ts);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn demo_session_open_message_marks_read_updates_counts_and_flag_state() {
        let (session, emitter, dir) = make_test_session();

        let opened = session
            .open_message(
                &OpenMessageInput {
                    request_id: "open-1".to_string(),
                    account_id: 1,
                    message_id: 1,
                },
                "rs-1".to_string(),
            )
            .expect("open message #1");
        assert!(opened.has_html);
        assert_eq!(opened.thread_messages.len(), 6);
        assert!(!opened.attachments.is_empty());
        assert_eq!(opened.body_status, BodyStatusDto::Ready);

        let q3 = session
            .open_message(
                &OpenMessageInput {
                    request_id: "open-q3".to_string(),
                    account_id: 1,
                    message_id: 8,
                },
                "rs-q3".to_string(),
            )
            .expect("open Q3 roadmap review");
        assert_eq!(q3.subject, "Q3 roadmap review");
        assert_eq!(q3.thread_messages.len(), 5);
        assert_eq!(q3.attachments.len(), 3);

        let boot_after = session.bootstrap();
        assert_eq!(boot_after.folders[0].unread_count, 4);
        assert_eq!(emitter.mail_changed.lock().unwrap().len(), 2);

        session
            .set_demo_message_unread(1, 1, true)
            .expect("mark #1 unread");
        assert_eq!(session.bootstrap().folders[0].unread_count, 5);

        let unread_view = session
            .list_messages(&ListMessagesInput {
                request_id: "unread-view".to_string(),
                view: MainViewDto::Unread,
                query: String::new(),
                scope: SearchScopeDto::ThisFolder,
                sort: MessageSortDto::UnreadFirst,
                filter: MessageFilterDto::default(),
                offset: 0,
                limit: 50,
            })
            .expect("unread view");
        assert_eq!(unread_view.total, 11);

        session
            .set_demo_message_flagged(1, 10, true)
            .expect("flag message #10");
        let opened10 = session
            .open_message(
                &OpenMessageInput {
                    request_id: "open-10".to_string(),
                    account_id: 1,
                    message_id: 10,
                },
                "rs-10".to_string(),
            )
            .expect("open message #10");
        assert!(opened10.flagged);

        let refreshed = session
            .refresh_mail(&RefreshMailInput {
                view: MainViewDto::UnifiedInbox,
            })
            .expect("refresh");
        assert!(refreshed.accepted);
        assert_eq!(emitter.sync_status.lock().unwrap().len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn locked_session_redacts_bootstrap_and_rejects_sensitive_commands() {
        let (session, emitter, _, _, dir) = make_live_test_session(true, Some("1234"));
        let locked = session.lock_app(Some("manual"));
        assert!(locked.locked);
        assert_eq!(emitter.lock_state.lock().unwrap().len(), 1);
        assert!(app_lock::is_lock_persisted_in(&dir));

        let boot = session.bootstrap();
        assert!(boot.locked);
        assert!(boot.has_passcode);
        assert!(boot.accounts.is_empty());
        assert!(boot.folders.is_empty());

        let err_open = session
            .open_message(
                &OpenMessageInput {
                    request_id: "open-locked".to_string(),
                    account_id: 1,
                    message_id: 1,
                },
                "rs-locked".to_string(),
            )
            .unwrap_err();
        assert_eq!(err_open.code, "LOCKED");

        let err_seen = session
            .set_message_seen(&SetMessageSeenInput {
                account_id: 1,
                message_id: 1,
                seen: true,
            })
            .unwrap_err();
        assert_eq!(err_seen.code, "LOCKED");

        let err_flag = session
            .set_message_flagged(&SetMessageFlaggedInput {
                account_id: 1,
                message_id: 1,
                flagged: true,
            })
            .unwrap_err();
        assert_eq!(err_flag.code, "LOCKED");

        let bad_unlock = session.unlock_app(&UnlockAppInput {
            passcode: "wrong".to_string(),
        });
        assert!(!bad_unlock.unlocked);
        assert!(bad_unlock.locked);

        let good_unlock = session.unlock_app(&UnlockAppInput {
            passcode: "1234".to_string(),
        });
        assert!(good_unlock.unlocked);
        assert!(!good_unlock.locked);
        assert!(!app_lock::is_lock_persisted_in(&dir));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn live_offline_cache_first_screen_and_empty_cache_ready_flag() {
        let (empty_session, _, _, _, empty_dir) = make_live_test_session(false, None);
        let empty_boot = empty_session.bootstrap();
        assert_eq!(empty_boot.mode, AppMode::Live);
        assert!(!empty_boot.cache_ready);
        assert!(empty_boot.folders.is_empty());
        let _ = std::fs::remove_dir_all(&empty_dir);

        let (session, _, _, _, dir) = make_live_test_session(true, None);
        let boot = session.bootstrap();
        assert!(boot.cache_ready);
        assert_eq!(boot.folders.len(), 1);

        let page = session
            .list_messages(&ListMessagesInput {
                request_id: "live-p0".to_string(),
                view: MainViewDto::UnifiedInbox,
                query: String::new(),
                scope: SearchScopeDto::ThisFolder,
                sort: MessageSortDto::DateNewest,
                filter: MessageFilterDto::default(),
                offset: 0,
                limit: 50,
            })
            .expect("live cached list");
        assert_eq!(page.total, 2);
        assert_eq!(page.items.len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn live_open_message_deduplicates_in_flight_body_fetches_and_reuses_cache_on_quick_switch() {
        let (session, _, reader_sink, worker, dir) = make_live_test_session(true, None);
        let list = session
            .list_messages(&ListMessagesInput {
                request_id: "l1".to_string(),
                view: MainViewDto::UnifiedInbox,
                query: String::new(),
                scope: SearchScopeDto::ThisFolder,
                sort: MessageSortDto::DateNewest,
                filter: MessageFilterDto::default(),
                offset: 0,
                limit: 10,
            })
            .unwrap();
        let msg_a = list
            .items
            .iter()
            .find(|m| m.subject.contains("邮件 A"))
            .unwrap();
        let msg_b = list
            .items
            .iter()
            .find(|m| m.subject.contains("邮件 B"))
            .unwrap();

        // Open uncached A twice -> only one LoadMessageBody should be dispatched.
        let open_a1 = session
            .open_message(
                &OpenMessageInput {
                    request_id: "open-a1".to_string(),
                    account_id: 1,
                    message_id: msg_a.id,
                },
                "rs-a1".to_string(),
            )
            .unwrap();
        assert_eq!(open_a1.body_status, BodyStatusDto::Loading);

        let open_a2 = session
            .open_message(
                &OpenMessageInput {
                    request_id: "open-a2".to_string(),
                    account_id: 1,
                    message_id: msg_a.id,
                },
                "rs-a2".to_string(),
            )
            .unwrap();
        assert_eq!(open_a2.body_status, BodyStatusDto::Loading);

        let load_reqs = worker
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| matches!(r, MailRequest::LoadMessageBody { message_id, .. } if *message_id == msg_a.id))
            .count();
        assert_eq!(
            load_reqs, 1,
            "in-flight LoadMessageBody must be deduplicated"
        );

        // Switch to cached B -> immediately Ready without LoadMessageBody.
        let open_b = session
            .open_message(
                &OpenMessageInput {
                    request_id: "open-b".to_string(),
                    account_id: 1,
                    message_id: msg_b.id,
                },
                "rs-b".to_string(),
            )
            .unwrap();
        assert_eq!(open_b.body_status, BodyStatusDto::Ready);
        assert_eq!(open_b.plain_text, "这是邮件 B 的缓存正文");

        // Worker finishes A while user is on B -> cached in memory, must not overwrite B's reader.
        let mut store = CacheStore::open_for_data_dir(&dir).unwrap();
        store
            .store_message_body_with_html(
                msg_a.id,
                "这是邮件 A 的远端正文",
                None,
                &[],
                None,
                false,
                None,
            )
            .unwrap();
        let (detail_a, _, _) = store
            .get_message_detail_ordered(msg_a.id, true)
            .unwrap()
            .unwrap();
        store.checkpoint_and_close_all();

        let events_before = reader_sink.events.lock().unwrap().len();
        session.process_worker_events(vec![WorkerEvent::MessageBodyLoaded(detail_a)]);
        assert_eq!(
            reader_sink.events.lock().unwrap().len(),
            events_before,
            "stale A completion must not hijack active reader B"
        );

        // Quick switch back B -> A reuses cached body immediately with zero extra worker fetch.
        let open_a3 = session
            .open_message(
                &OpenMessageInput {
                    request_id: "open-a3".to_string(),
                    account_id: 1,
                    message_id: msg_a.id,
                },
                "rs-a3".to_string(),
            )
            .unwrap();
        assert_eq!(open_a3.body_status, BodyStatusDto::Ready);
        assert_eq!(open_a3.plain_text, "这是邮件 A 的远端正文");

        let total_load_reqs_a = worker
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| matches!(r, MailRequest::LoadMessageBody { message_id, .. } if *message_id == msg_a.id))
            .count();
        assert_eq!(total_load_reqs_a, 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn live_body_fetch_timeout_emits_retryable_unavailable_and_allows_retry() {
        let (session, _, reader_sink, worker, dir) = make_live_test_session(true, None);
        let list = session
            .list_messages(&ListMessagesInput {
                request_id: "l-to".to_string(),
                view: MainViewDto::UnifiedInbox,
                query: String::new(),
                scope: SearchScopeDto::ThisFolder,
                sort: MessageSortDto::DateNewest,
                filter: MessageFilterDto::default(),
                offset: 0,
                limit: 10,
            })
            .unwrap();
        let msg_a = list
            .items
            .iter()
            .find(|m| m.subject.contains("邮件 A"))
            .unwrap();

        let open = session
            .open_message(
                &OpenMessageInput {
                    request_id: "open-to".to_string(),
                    account_id: 1,
                    message_id: msg_a.id,
                },
                "rs-to".to_string(),
            )
            .unwrap();
        assert_eq!(open.body_status, BodyStatusDto::Loading);

        std::thread::sleep(Duration::from_millis(220));
        let events = reader_sink.events.lock().unwrap().clone();
        assert!(
            events.iter().any(|e| {
                e.reader_session_id == "rs-to"
                    && e.body_status == BodyStatusDto::Unavailable
                    && e.retryable == Some(true)
            }),
            "timeout must emit retryable unavailable status"
        );

        // Retry after timeout must send a fresh LoadMessageBody.
        let retry_open = session
            .open_message(
                &OpenMessageInput {
                    request_id: "open-retry".to_string(),
                    account_id: 1,
                    message_id: msg_a.id,
                },
                "rs-retry".to_string(),
            )
            .unwrap();
        assert_eq!(retry_open.body_status, BodyStatusDto::Loading);
        let count = worker
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| matches!(r, MailRequest::LoadMessageBody { message_id, .. } if *message_id == msg_a.id))
            .count();
        assert_eq!(count, 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delayed_read_mark_cancels_on_switch_reader_off_and_lock() {
        let (session, _, _, worker, dir) = make_live_test_session(true, Some("1234"));
        session
            .prefs
            .mutate_and_save(|p| {
                p.read_mark_mode = "delay".to_string();
                true
            })
            .unwrap();

        let list = session
            .list_messages(&ListMessagesInput {
                request_id: "l-delay".to_string(),
                view: MainViewDto::UnifiedInbox,
                query: String::new(),
                scope: SearchScopeDto::ThisFolder,
                sort: MessageSortDto::DateNewest,
                filter: MessageFilterDto::default(),
                offset: 0,
                limit: 10,
            })
            .unwrap();
        let msg_a = list
            .items
            .iter()
            .find(|m| m.subject.contains("邮件 A"))
            .unwrap();
        let msg_b = list
            .items
            .iter()
            .find(|m| m.subject.contains("邮件 B"))
            .unwrap();

        // 1. Open A, then turn reader off before 80ms delay -> SetSeen must NOT fire.
        session
            .open_message(
                &OpenMessageInput {
                    request_id: "d1".to_string(),
                    account_id: 1,
                    message_id: msg_a.id,
                },
                "rs-d1".to_string(),
            )
            .unwrap();
        session
            .prefs
            .update_patch(&MainPreferencesPatch {
                reader_position: Some(ReaderPositionDto::Off),
                ..Default::default()
            })
            .unwrap();
        session.on_preferences_updated();
        std::thread::sleep(Duration::from_millis(140));
        assert!(
            !worker
                .requests
                .lock()
                .unwrap()
                .iter()
                .any(|r| matches!(r, MailRequest::SetSeen { .. })),
            "reader off must cancel delayed read mark"
        );

        // 2. Restore reader, open A then lock before delay -> SetSeen must NOT fire.
        session
            .prefs
            .update_patch(&MainPreferencesPatch {
                reader_position: Some(ReaderPositionDto::Right),
                ..Default::default()
            })
            .unwrap();
        session.on_preferences_updated();
        session
            .open_message(
                &OpenMessageInput {
                    request_id: "d2".to_string(),
                    account_id: 1,
                    message_id: msg_a.id,
                },
                "rs-d2".to_string(),
            )
            .unwrap();
        session.lock_app(Some("manual"));
        std::thread::sleep(Duration::from_millis(140));
        assert!(
            !worker
                .requests
                .lock()
                .unwrap()
                .iter()
                .any(|r| matches!(r, MailRequest::SetSeen { .. })),
            "lock_app must cancel delayed read mark"
        );

        // 3. Unlock, open A then switch quickly to B -> only B gets SetSeen after delay!
        session.unlock_app(&UnlockAppInput {
            passcode: "1234".to_string(),
        });
        session
            .open_message(
                &OpenMessageInput {
                    request_id: "d3".to_string(),
                    account_id: 1,
                    message_id: msg_a.id,
                },
                "rs-d3".to_string(),
            )
            .unwrap();
        session
            .open_message(
                &OpenMessageInput {
                    request_id: "d4".to_string(),
                    account_id: 1,
                    message_id: msg_b.id,
                },
                "rs-d4".to_string(),
            )
            .unwrap();
        std::thread::sleep(Duration::from_millis(160));
        let seen_ids: Vec<u32> = worker
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter_map(|r| match r {
                MailRequest::SetSeen {
                    message_id,
                    seen: true,
                    ..
                } => Some(*message_id),
                _ => None,
            })
            .collect();
        assert_eq!(seen_ids, vec![msg_b.id]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn live_set_message_seen_and_flagged_dispatch_worker_requests() {
        let (session, emitter, _, worker, dir) = make_live_test_session(true, None);
        let list = session
            .list_messages(&ListMessagesInput {
                request_id: "l-flags".to_string(),
                view: MainViewDto::UnifiedInbox,
                query: String::new(),
                scope: SearchScopeDto::ThisFolder,
                sort: MessageSortDto::DateNewest,
                filter: MessageFilterDto::default(),
                offset: 0,
                limit: 10,
            })
            .unwrap();
        let msg_b = list
            .items
            .iter()
            .find(|m| m.subject.contains("邮件 B"))
            .unwrap();

        let seen_out = session
            .set_message_seen(&SetMessageSeenInput {
                account_id: 1,
                message_id: msg_b.id,
                seen: true,
            })
            .expect("set_message_seen");
        assert!(!seen_out.unread);

        let flag_out = session
            .set_message_flagged(&SetMessageFlaggedInput {
                account_id: 1,
                message_id: msg_b.id,
                flagged: true,
            })
            .expect("set_message_flagged");
        assert!(flag_out.flagged);

        let reqs = worker.requests.lock().unwrap();
        assert!(reqs.iter().any(|r| matches!(
            r,
            MailRequest::SetSeen { account_id: 1, message_id, seen: true, .. } if *message_id == msg_b.id
        )));
        assert!(reqs.iter().any(|r| matches!(
            r,
            MailRequest::SetFlagged { account_id: 1, message_id, flagged: true, .. } if *message_id == msg_b.id
        )));
        assert_eq!(emitter.mail_changed.lock().unwrap().len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn live_last_view_restores_and_falls_back_when_folder_missing() {
        let mut prefs = kitemail_mail_core::config::AppPreferences::default();
        prefs.default_view = "last".to_string();
        prefs.last_folder = Some((1, 999));

        let acc = AccountConfig {
            id: 1,
            enabled: true,
            ..AccountConfig::default()
        };
        let mut folders = HashMap::new();
        folders.insert(
            1,
            vec![FolderSummaryDto {
                id: 101,
                account_id: 1,
                name: "收件箱".to_string(),
                path: "INBOX".to_string(),
                unread_count: 0,
                total_count: 0,
                has_more_remote: false,
            }],
        );
        // Folder 999 does not exist -> fall back to UnifiedInbox.
        assert_eq!(
            resolve_initial_live_view(&prefs, std::slice::from_ref(&acc), &folders, true),
            MainViewDto::UnifiedInbox
        );

        // Existing folder 101 restores AccountFolder.
        prefs.last_folder = Some((1, 101));
        assert_eq!(
            resolve_initial_live_view(&prefs, std::slice::from_ref(&acc), &folders, true),
            MainViewDto::AccountFolder {
                account_id: 1,
                folder_id: 101
            }
        );

        // Unified last_view="starred" restores Starred.
        prefs.last_folder = None;
        prefs.last_view = Some("starred".to_string());
        assert_eq!(
            resolve_initial_live_view(&prefs, &[acc], &folders, true),
            MainViewDto::Starred
        );
    }

    #[test]
    fn lock_cooldown_enforces_retry_after_and_background_auto_lock_triggers() {
        let (session, emitter, _, _, dir) = make_live_test_session(true, Some("1234"));
        session
            .prefs
            .mutate_and_save(|p| {
                p.app_lock_background_timeout = "immediate".to_string();
                true
            })
            .unwrap();

        // Seed background_since 2 seconds in the past, then verify auto_lock_tick locks after grace.
        {
            let mut guard = session.lock_state.lock().unwrap();
            let past = Instant::now() - Duration::from_secs(2);
            assert!(guard
                .tracker
                .evaluate(AutoLockTickInput {
                    now: past,
                    has_passcode: true,
                    is_locked: false,
                    in_foreground: false,
                    file_dialog_open: false,
                    system_idle: None,
                    idle_minutes: 15,
                    background_policy: BackgroundLockPolicy::Immediate,
                })
                .is_none());
        }
        session.auto_lock_tick(false, None);
        assert!(session.is_locked());
        assert_eq!(
            emitter
                .lock_state
                .lock()
                .unwrap()
                .last()
                .and_then(|e| e.locked_reason.as_deref()),
            Some("background")
        );

        // 5 consecutive wrong passcodes trigger 5s cooldown.
        for attempt in 1..=5 {
            let out = session.unlock_app(&UnlockAppInput {
                passcode: "bad-pass".to_string(),
            });
            assert!(!out.unlocked);
            if attempt < 5 {
                assert_eq!(out.retry_after_secs, None);
            } else {
                assert_eq!(out.retry_after_secs, Some(5));
            }
        }

        // Even the right passcode is blocked while cooldown is active.
        let during_cooldown = session.unlock_app(&UnlockAppInput {
            passcode: "1234".to_string(),
        });
        assert!(!during_cooldown.unlocked);
        assert!(during_cooldown.retry_after_secs.is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn demo_archive_delete_spam_move_and_undo_update_counts_and_emit_events() {
        let (session, emitter, dir) = make_test_session();
        let boot_init = session.bootstrap();
        let init_inbox_total = boot_init.folders[0].total_count;
        let init_inbox_unread = boot_init.folders[0].unread_count;
        let init_archive_total = boot_init.folders[3].total_count;

        // Archive unread message #1 from INBOX -> Archive.
        let archived = session
            .archive_message(&MessageActionInput {
                account_id: 1,
                message_id: 1,
            })
            .expect("archive #1");
        assert_eq!(archived.original_folder_path, "INBOX");
        assert_eq!(archived.target_folder_path, "Archive");
        assert_eq!(archived.rfc_message_id, "<demo-1@hylki.example>");

        let boot_arch = session.bootstrap();
        assert_eq!(boot_arch.folders[0].total_count, init_inbox_total - 1);
        assert_eq!(boot_arch.folders[0].unread_count, init_inbox_unread - 1);
        assert_eq!(boot_arch.folders[3].total_count, init_archive_total + 1);

        // Undo archive -> INBOX counts restored.
        let undo_arch = session
            .undo_message_move(&UndoMessageMoveInput {
                account_id: 1,
                message_id: 1,
                rfc_message_id: archived.rfc_message_id.clone(),
                current_folder_path: archived.target_folder_path.clone(),
                original_folder_path: archived.original_folder_path.clone(),
            })
            .expect("undo archive #1");
        assert!(undo_arch.accepted);
        let boot_restored = session.bootstrap();
        assert_eq!(boot_restored.folders[0].total_count, init_inbox_total);
        assert_eq!(boot_restored.folders[0].unread_count, init_inbox_unread);

        // Delete #7 -> moves to Trash, then delete again -> permanently removed.
        let deleted = session
            .delete_message(&MessageActionInput {
                account_id: 1,
                message_id: 7,
            })
            .expect("delete #7");
        assert_eq!(deleted.target_folder_path, "Trash");
        let trash_page = session
            .list_messages(&ListMessagesInput {
                request_id: "trash-1".to_string(),
                view: MainViewDto::Trash,
                query: String::new(),
                scope: SearchScopeDto::ThisFolder,
                sort: MessageSortDto::DateNewest,
                filter: MessageFilterDto::default(),
                offset: 0,
                limit: 10,
            })
            .expect("list trash");
        assert_eq!(trash_page.total, 1);
        assert_eq!(trash_page.items[0].id, 7);

        let perm_deleted = session
            .delete_message(&MessageActionInput {
                account_id: 1,
                message_id: 7,
            })
            .expect("permanent delete #7");
        assert_eq!(perm_deleted.target_folder_path, "");

        // Mark #8 as spam -> Junk folder.
        let init_junk_total = session
            .list_messages(&ListMessagesInput {
                request_id: "junk-0".to_string(),
                view: MainViewDto::Junk,
                query: String::new(),
                scope: SearchScopeDto::ThisFolder,
                sort: MessageSortDto::DateNewest,
                filter: MessageFilterDto::default(),
                offset: 0,
                limit: 10,
            })
            .expect("init junk")
            .total;
        let spammed = session
            .mark_message_spam(&MessageActionInput {
                account_id: 1,
                message_id: 8,
            })
            .expect("spam #8");
        assert_eq!(spammed.target_folder_path, "Junk");
        let junk_page = session
            .list_messages(&ListMessagesInput {
                request_id: "junk-1".to_string(),
                view: MainViewDto::Junk,
                query: String::new(),
                scope: SearchScopeDto::ThisFolder,
                sort: MessageSortDto::DateNewest,
                filter: MessageFilterDto::default(),
                offset: 0,
                limit: 10,
            })
            .expect("list junk");
        assert_eq!(junk_page.total, init_junk_total + 1);

        // Move #10 -> Sent, then undo back to INBOX.
        let moved = session
            .move_message(&MoveMessageInput {
                account_id: 1,
                message_id: 10,
                target_folder_path: "Sent".to_string(),
            })
            .expect("move #10");
        assert_eq!(moved.target_folder_path, "Sent");
        let undo_move = session
            .undo_message_move(&UndoMessageMoveInput {
                account_id: 1,
                message_id: 10,
                rfc_message_id: moved.rfc_message_id,
                current_folder_path: "Sent".to_string(),
                original_folder_path: "INBOX".to_string(),
            })
            .expect("undo move #10");
        assert!(undo_move.accepted);

        assert!(emitter.mail_changed.lock().unwrap().len() >= 6);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn allow_remote_images_detects_blocked_images_and_persists_allowed_remote_senders() {
        let (session, _, dir) = make_test_session();

        let opened = session
            .open_message(
                &OpenMessageInput {
                    request_id: "open-remote-1".to_string(),
                    account_id: 1,
                    message_id: 1,
                },
                "rs-remote-1".to_string(),
            )
            .expect("open #1");
        assert_eq!(opened.remote_images_blocked, Some(true));
        assert_eq!(
            opened.rfc_message_id.as_deref(),
            Some("<demo-1@hylki.example>")
        );
        assert_eq!(opened.folder_path.as_deref(), Some("INBOX"));

        // Allow once (trustSender = false) unblocks for session without persisting sender.
        let once = session
            .allow_remote_images(&AllowRemoteImagesInput {
                account_id: 1,
                message_id: 1,
                trust_sender: false,
            })
            .expect("allow once");
        assert!(once.allowed);
        assert!(!once.sender_trusted);
        assert!(PreferencesStore::open_in(dir.clone())
            .snapshot_raw()
            .allowed_remote_senders
            .is_empty());

        let reopened_once = session
            .open_message(
                &OpenMessageInput {
                    request_id: "open-remote-2".to_string(),
                    account_id: 1,
                    message_id: 1,
                },
                "rs-remote-2".to_string(),
            )
            .expect("reopen #1");
        assert_eq!(reopened_once.remote_images_blocked, Some(false));

        // Trust sender (trustSender = true) persists normalized email into preferences.toml.
        let trusted = session
            .allow_remote_images(&AllowRemoteImagesInput {
                account_id: 1,
                message_id: 1,
                trust_sender: true,
            })
            .expect("trust sender");
        assert!(trusted.allowed);
        assert!(trusted.sender_trusted);

        let reloaded_prefs = PreferencesStore::open_in(dir.clone());
        assert_eq!(
            reloaded_prefs.snapshot_raw().allowed_remote_senders,
            vec!["marcus@studio.dev".to_string()]
        );

        // Fresh session using same preferences dir sees sender already trusted.
        let fresh_session = MailSession::new(
            AppMode::Demo,
            Arc::new(reloaded_prefs),
            Arc::new(ReaderManager::new(Arc::new(NullReaderSink))),
            Arc::new(TestEmitter::default()),
        );
        let opened_fresh = fresh_session
            .open_message(
                &OpenMessageInput {
                    request_id: "open-remote-3".to_string(),
                    account_id: 1,
                    message_id: 1,
                },
                "rs-remote-3".to_string(),
            )
            .expect("open #1 in fresh session");
        assert_eq!(opened_fresh.remote_images_blocked, Some(false));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn live_move_archive_spam_delete_undo_and_load_older_dispatch_and_handle_events() {
        let (session, emitter, _, worker, dir) = make_live_test_session(true, Some("1234"));

        // Seed Archive/Trash/Junk folders and mark INBOX has_more_remote = true.
        {
            let mut store = CacheStore::open_for_data_dir(&dir).unwrap();
            let folders = store
                .sync_folders(
                    1,
                    &[
                        RemoteFolder {
                            name: "收件箱".to_string(),
                            path: "INBOX".to_string(),
                            unread_count: 2,
                            total_count: 2,
                        },
                        RemoteFolder {
                            name: "归档".to_string(),
                            path: "Archive".to_string(),
                            unread_count: 0,
                            total_count: 0,
                        },
                        RemoteFolder {
                            name: "已删除".to_string(),
                            path: "Trash".to_string(),
                            unread_count: 0,
                            total_count: 0,
                        },
                        RemoteFolder {
                            name: "垃圾邮件".to_string(),
                            path: "Junk".to_string(),
                            unread_count: 0,
                            total_count: 0,
                        },
                    ],
                )
                .unwrap();
            store.checkpoint_and_close_all();
            let mut dtos: Vec<FolderSummaryDto> =
                folders.iter().map(FolderSummaryDto::from).collect();
            dtos[0].has_more_remote = true;
            session
                .live
                .lock()
                .unwrap()
                .folders_by_account
                .insert(1, dtos);
        }

        let list = session
            .list_messages(&ListMessagesInput {
                request_id: "l-ops".to_string(),
                view: MainViewDto::UnifiedInbox,
                query: String::new(),
                scope: SearchScopeDto::ThisFolder,
                sort: MessageSortDto::DateNewest,
                filter: MessageFilterDto::default(),
                offset: 0,
                limit: 10,
            })
            .unwrap();
        let msg_b_id = list
            .items
            .iter()
            .find(|m| m.subject.contains("邮件 B"))
            .unwrap()
            .id;
        let inbox_id = session.bootstrap().folders[0].id;

        // Lock check: all new commands reject when locked.
        session.lock_app(Some("manual"));
        assert_eq!(
            session
                .archive_message(&MessageActionInput {
                    account_id: 1,
                    message_id: msg_b_id
                })
                .unwrap_err()
                .code,
            "LOCKED"
        );
        assert_eq!(
            session
                .delete_message(&MessageActionInput {
                    account_id: 1,
                    message_id: msg_b_id
                })
                .unwrap_err()
                .code,
            "LOCKED"
        );
        assert_eq!(
            session
                .mark_message_spam(&MessageActionInput {
                    account_id: 1,
                    message_id: msg_b_id
                })
                .unwrap_err()
                .code,
            "LOCKED"
        );
        assert_eq!(
            session
                .move_message(&MoveMessageInput {
                    account_id: 1,
                    message_id: msg_b_id,
                    target_folder_path: "Archive".to_string()
                })
                .unwrap_err()
                .code,
            "LOCKED"
        );
        assert_eq!(
            session
                .undo_message_move(&UndoMessageMoveInput {
                    account_id: 1,
                    message_id: msg_b_id,
                    rfc_message_id: "<m11@example.com>".to_string(),
                    current_folder_path: "Archive".to_string(),
                    original_folder_path: "INBOX".to_string()
                })
                .unwrap_err()
                .code,
            "LOCKED"
        );
        assert_eq!(
            session
                .load_older_messages(&LoadOlderMessagesInput {
                    view: MainViewDto::UnifiedInbox,
                    limit: 50
                })
                .unwrap_err()
                .code,
            "LOCKED"
        );
        assert_eq!(
            session
                .allow_remote_images(&AllowRemoteImagesInput {
                    account_id: 1,
                    message_id: msg_b_id,
                    trust_sender: false
                })
                .unwrap_err()
                .code,
            "LOCKED"
        );

        session.unlock_app(&UnlockAppInput {
            passcode: "1234".to_string(),
        });

        let arch_out = session
            .archive_message(&MessageActionInput {
                account_id: 1,
                message_id: msg_b_id,
            })
            .expect("live archive");
        assert_eq!(arch_out.original_folder_path, "INBOX");
        assert_eq!(arch_out.target_folder_path, "Archive");
        assert_eq!(arch_out.rfc_message_id, "<m11@example.com>");

        let del_out = session
            .delete_message(&MessageActionInput {
                account_id: 1,
                message_id: msg_b_id,
            })
            .expect("live delete");
        assert_eq!(del_out.target_folder_path, "Trash");

        let spam_out = session
            .mark_message_spam(&MessageActionInput {
                account_id: 1,
                message_id: msg_b_id,
            })
            .expect("live spam");
        assert_eq!(spam_out.target_folder_path, "Junk");

        let move_out = session
            .move_message(&MoveMessageInput {
                account_id: 1,
                message_id: msg_b_id,
                target_folder_path: "Custom/Work".to_string(),
            })
            .expect("live move");
        assert_eq!(move_out.target_folder_path, "Custom/Work");

        let undo_out = session
            .undo_message_move(&UndoMessageMoveInput {
                account_id: 1,
                message_id: msg_b_id,
                rfc_message_id: "<m11@example.com>".to_string(),
                current_folder_path: "Archive".to_string(),
                original_folder_path: "INBOX".to_string(),
            })
            .expect("live undo");
        assert!(undo_out.accepted);

        let older_unified = session
            .load_older_messages(&LoadOlderMessagesInput {
                view: MainViewDto::UnifiedInbox,
                limit: 50,
            })
            .expect("load older unified");
        assert!(older_unified.accepted);
        assert!(older_unified.has_more_remote);

        let older_folder = session
            .load_older_messages(&LoadOlderMessagesInput {
                view: MainViewDto::AccountFolder {
                    account_id: 1,
                    folder_id: inbox_id,
                },
                limit: 30,
            })
            .expect("load older folder");
        assert!(older_folder.accepted);

        {
            let reqs = worker.requests.lock().unwrap();
            assert!(reqs
                .iter()
                .any(|r| matches!(r, MailRequest::ArchiveMessage { message_id, .. } if *message_id == msg_b_id)));
            assert!(reqs
                .iter()
                .any(|r| matches!(r, MailRequest::DeleteMessage { message_id, .. } if *message_id == msg_b_id)));
            assert!(reqs.iter().any(
                |r| matches!(r, MailRequest::MarkSpam { message_id, .. } if *message_id == msg_b_id)
            ));
            assert!(reqs.iter().any(|r| matches!(
                r,
                MailRequest::MoveMessage { message_id, target_folder_path, .. }
                if *message_id == msg_b_id && target_folder_path == "Custom/Work"
            )));
            assert!(reqs.iter().any(|r| matches!(
                r,
                MailRequest::UndoMessageMove { current_folder_path, original_folder_path, .. }
                if current_folder_path == "Archive" && original_folder_path == "INBOX"
            )));
            assert!(reqs.iter().any(|r| matches!(
                r,
                MailRequest::LoadOlderMessages {
                    folder_id: 0,
                    limit: 50
                }
            )));
            assert!(reqs.iter().any(|r| matches!(
                r,
                MailRequest::LoadOlderMessages {
                    folder_id,
                    limit: 30
                } if *folder_id == inbox_id
            )));
        }

        session.process_worker_events(vec![
            WorkerEvent::OlderMessagesLoaded {
                folder_id: 0,
                fetched_count: 10,
                has_more_remote: false,
            },
            WorkerEvent::MessageArchived {
                account_id: 1,
                message_id: msg_b_id,
                dest_folder_path: "Archive".to_string(),
                rfc_message_id: "<m11@example.com>".to_string(),
            },
            WorkerEvent::MessageDeleted {
                account_id: 1,
                message_id: msg_b_id,
                dest_folder_path: "Trash".to_string(),
                rfc_message_id: "<m11@example.com>".to_string(),
            },
            WorkerEvent::MessageMoved {
                account_id: 1,
                message_id: msg_b_id,
                target_folder_path: "Custom/Work".to_string(),
                rfc_message_id: "<m11@example.com>".to_string(),
            },
            WorkerEvent::MessageMarkedSpam {
                account_id: 1,
                message_id: msg_b_id,
                dest_folder_path: "Junk".to_string(),
                rfc_message_id: "<m11@example.com>".to_string(),
            },
        ]);

        let boot_after = session.bootstrap();
        assert!(!boot_after.folders[0].has_more_remote);
        let reasons: Vec<String> = emitter
            .mail_changed
            .lock()
            .unwrap()
            .iter()
            .map(|e| e.reason.clone())
            .collect();
        assert!(reasons.contains(&"olderMessagesLoaded".to_string()));
        assert!(reasons.contains(&"messageArchived".to_string()));
        assert!(reasons.contains(&"messageDeleted".to_string()));
        assert!(reasons.contains(&"messageMoved".to_string()));
        assert!(reasons.contains(&"messageMarkedSpam".to_string()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn live_auto_sync_respects_interval_lock_and_quit_signal() {
        let (session, _, _, worker, dir) = make_live_test_session(true, Some("1234"));
        let start = *session.last_auto_sync_at.lock().unwrap();

        // Not elapsed yet -> no sync.
        assert!(!session.try_auto_sync_at(start + Duration::from_secs(60)));

        // Elapsed past default 5 minutes -> dispatches SyncAllAccounts { user_initiated: false }.
        assert!(session.try_auto_sync_at(start + Duration::from_secs(301)));
        assert_eq!(
            worker
                .requests
                .lock()
                .unwrap()
                .iter()
                .filter(|r| matches!(
                    r,
                    MailRequest::SyncAllAccounts {
                        user_initiated: false
                    }
                ))
                .count(),
            1
        );

        // auto_sync_minutes == 0 -> disabled.
        session
            .prefs
            .mutate_and_save(|p| {
                p.auto_sync_minutes = 0;
                true
            })
            .unwrap();
        assert!(!session.try_auto_sync_at(start + Duration::from_secs(900)));

        // Re-enable to 1 minute, but lock app -> no sync while locked.
        session
            .prefs
            .mutate_and_save(|p| {
                p.auto_sync_minutes = 1;
                true
            })
            .unwrap();
        session.lock_app(Some("manual"));
        assert!(!session.try_auto_sync_at(start + Duration::from_secs(1_000)));

        // prepare_quit wakes quit signal and stops auto-sync.
        session.unlock_app(&UnlockAppInput {
            passcode: "1234".to_string(),
        });
        session.prepare_quit();
        assert!(*session.auto_sync_signal.0.lock().unwrap());
        assert!(!session.try_auto_sync_at(start + Duration::from_secs(1_200)));

        let _ = std::fs::remove_dir_all(&dir);
    }

    struct AsyncWorkerMock {
        requests: Mutex<Vec<MailRequest>>,
        query_calls: AtomicU64,
    }

    impl AsyncWorkerMock {
        fn new() -> Self {
            Self {
                requests: Mutex::new(Vec::new()),
                query_calls: AtomicU64::new(0),
            }
        }
    }

    impl LiveWorkerSink for AsyncWorkerMock {
        fn send(&self, req: MailRequest) {
            self.requests.lock().unwrap().push(req);
        }
        fn prepare_quit(&self) {}
        fn query_page(&self, _req: PageQueryRequest) -> Option<Result<MailPage, String>> {
            self.query_calls.fetch_add(1, Ordering::SeqCst);
            Some(Err(
                "should not route list query through async worker".to_string()
            ))
        }
        fn is_async_worker(&self) -> bool {
            true
        }
    }

    #[test]
    fn live_read_cache_bypasses_async_worker_and_resets_cleanly() {
        let (session, _, _, _, dir) = make_live_test_session(true, None);
        let async_worker = Arc::new(AsyncWorkerMock::new());
        let folder_id = {
            let mut live = session.live.lock().unwrap();
            live.worker = Some(async_worker.clone());
            live.folders_by_account
                .get(&1)
                .and_then(|fs| fs.first())
                .expect("inbox")
                .id
        };

        let out1 = session
            .list_messages(&ListMessagesInput {
                request_id: "r1".to_string(),
                view: MainViewDto::AccountFolder {
                    account_id: 1,
                    folder_id,
                },
                query: String::new(),
                scope: SearchScopeDto::ThisFolder,
                sort: crate::dto::MessageSortDto::DateNewest,
                filter: MessageFilterDto::default(),
                offset: 0,
                limit: 50,
            })
            .expect("first live query succeeds via read_cache");
        assert_eq!(out1.total, 2);
        assert_eq!(async_worker.query_calls.load(Ordering::SeqCst), 0);
        assert!(session.read_cache.lock().unwrap().is_some());

        // Switching into AccountFolder at offset=0 triggers a single background SyncFolder.
        {
            let reqs = async_worker.requests.lock().unwrap();
            assert_eq!(
                reqs.iter()
                    .filter(|r| matches!(
                        r,
                        MailRequest::SyncFolder {
                            account_id: 1,
                            folder_id: fid,
                            user_initiated: false,
                            ..
                        } if *fid == folder_id
                    ))
                    .count(),
                1
            );
        }

        // Reloading the same AccountFolder must NOT trigger a second SyncFolder loop.
        let out2 = session
            .list_messages(&ListMessagesInput {
                request_id: "r2".to_string(),
                view: MainViewDto::AccountFolder {
                    account_id: 1,
                    folder_id,
                },
                query: String::new(),
                scope: SearchScopeDto::ThisFolder,
                sort: crate::dto::MessageSortDto::DateNewest,
                filter: MessageFilterDto::default(),
                offset: 0,
                limit: 50,
            })
            .expect("second live query reuses read_cache");
        assert_eq!(out2.total, 2);
        assert_eq!(
            async_worker
                .requests
                .lock()
                .unwrap()
                .iter()
                .filter(|r| matches!(r, MailRequest::SyncFolder { .. }))
                .count(),
            1
        );

        session.reset_read_cache();
        assert!(session.read_cache.lock().unwrap().is_none());
        let out3 = session
            .list_messages(&ListMessagesInput {
                request_id: "r3".to_string(),
                view: MainViewDto::UnifiedInbox,
                query: String::new(),
                scope: SearchScopeDto::ThisFolder,
                sort: crate::dto::MessageSortDto::DateNewest,
                filter: MessageFilterDto::default(),
                offset: 0,
                limit: 50,
            })
            .expect("query reopens read_cache after reset");
        assert_eq!(out3.total, 2);

        session.prepare_quit();
        assert!(session.read_cache.lock().unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn live_worker_error_and_folder_sync_failed_update_sync_state() {
        let (session, emitter, _, _, dir) = make_live_test_session(false, None);
        {
            let mut live = session.live.lock().unwrap();
            live.syncing_accounts = [1u32, 2u32].into_iter().collect();
            live.cache_ready = false;
        }
        {
            let mut st = session.sync_status.lock().unwrap();
            st.syncing = true;
            st.message = Some("正在同步邮箱...".to_string());
        }

        // One account errors while account 2 is still syncing -> syncing stays true.
        session.process_worker_events(vec![WorkerEvent::Error {
            account_id: Some(1),
            message: "Outlook 令牌过期".to_string(),
        }]);
        {
            let st = session.sync_status.lock().unwrap();
            assert!(st.syncing);
            assert_eq!(st.message.as_deref(), Some("Outlook 令牌过期"));
            let live = session.live.lock().unwrap();
            assert!(!live.syncing_accounts.contains(&1));
            assert!(live.syncing_accounts.contains(&2));
            assert!(live.cache_ready);
        }

        // Second account errors -> syncing_accounts empty, syncing becomes false.
        session.process_worker_events(vec![WorkerEvent::Error {
            account_id: Some(2),
            message: "网络超时".to_string(),
        }]);
        {
            let st = session.sync_status.lock().unwrap();
            assert!(!st.syncing);
            assert_eq!(st.message.as_deref(), Some("网络超时"));
        }

        // User-initiated FolderSyncFailed surfaces message in sync_status.
        session.process_worker_events(vec![WorkerEvent::FolderSyncFailed {
            account_id: 1,
            folder_id: 10,
            folder_path: "AAMkGraphId".to_string(),
            folder_name: "收件箱".to_string(),
            user_initiated: true,
            removed_from_server: false,
            message: "文件夹同步失败".to_string(),
        }]);
        assert_eq!(
            session.sync_status.lock().unwrap().message.as_deref(),
            Some("文件夹同步失败")
        );
        assert!(emitter.sync_status.lock().unwrap().len() >= 3);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn live_older_messages_matches_outlook_opaque_inbox_path() {
        let (session, _, _, worker, dir) = make_live_test_session(false, None);
        {
            let mut live = session.live.lock().unwrap();
            live.folders_by_account.insert(
                1,
                vec![FolderSummaryDto {
                    account_id: 1,
                    id: 99,
                    name: "收件箱".to_string(),
                    path: "AQMkADAwATNiZmYAZC04".to_string(),
                    unread_count: 10,
                    total_count: 50,
                    has_more_remote: true,
                }],
            );
            live.cache_ready = true;
        }

        let older = session
            .load_older_messages(&LoadOlderMessagesInput {
                view: MainViewDto::UnifiedInbox,
                limit: 50,
            })
            .expect("load_older_messages on outlook inbox");
        assert!(older.accepted);
        assert!(older.has_more_remote);
        assert!(worker.requests.lock().unwrap().iter().any(|r| matches!(
            r,
            MailRequest::LoadOlderMessages {
                folder_id: 0,
                limit: 50
            }
        )));

        session.process_worker_events(vec![WorkerEvent::OlderMessagesLoaded {
            folder_id: 0,
            fetched_count: 10,
            has_more_remote: false,
        }]);
        let boot = session.bootstrap();
        assert!(!boot.folders[0].has_more_remote);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restored_non_inbox_folder_triggers_single_first_sync_and_unified_pagination_preserves_retry()
    {
        let (session, _, _, worker, dir) = make_live_test_session(true, None);
        let custom_fid = 77u32;
        {
            let mut live = session.live.lock().unwrap();
            live.account_configs.push(AccountConfig {
                id: 2,
                email: "second@example.com".to_string(),
                enabled: true,
                in_unified: true,
                ..AccountConfig::default()
            });
            live.folders_by_account
                .entry(1)
                .or_default()
                .push(FolderSummaryDto {
                    account_id: 1,
                    id: custom_fid,
                    name: "Archive/2026".to_string(),
                    path: "Archive/2026".to_string(),
                    unread_count: 0,
                    total_count: 0,
                    has_more_remote: true,
                });
            live.folders_by_account.insert(
                2,
                vec![FolderSummaryDto {
                    account_id: 2,
                    id: 201,
                    name: "收件箱".to_string(),
                    path: "INBOX".to_string(),
                    unread_count: 0,
                    total_count: 0,
                    has_more_remote: true,
                }],
            );
        }

        // Simulate startup restoring `current_view` to the non-inbox AccountFolder.
        let restored = MainViewDto::AccountFolder {
            account_id: 1,
            folder_id: custom_fid,
        };
        *session.current_view.lock().unwrap() = restored;

        session
            .list_messages(&ListMessagesInput {
                request_id: "restored-1".to_string(),
                view: restored,
                query: String::new(),
                scope: SearchScopeDto::ThisFolder,
                sort: crate::dto::MessageSortDto::DateNewest,
                filter: MessageFilterDto::default(),
                offset: 0,
                limit: 50,
            })
            .expect("first open of restored non-inbox folder");

        // Reloading the same restored view must NOT trigger a second SyncFolder.
        session
            .list_messages(&ListMessagesInput {
                request_id: "restored-2".to_string(),
                view: restored,
                query: String::new(),
                scope: SearchScopeDto::ThisFolder,
                sort: crate::dto::MessageSortDto::DateNewest,
                filter: MessageFilterDto::default(),
                offset: 0,
                limit: 50,
            })
            .expect("second open of restored non-inbox folder");

        let sync_count = worker
            .requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| {
                matches!(
                    r,
                    MailRequest::SyncFolder {
                        account_id: 1,
                        folder_id,
                        ..
                    } if *folder_id == custom_fid
                )
            })
            .count();
        assert_eq!(sync_count, 1);

        // Unified pagination: Account 1 finishes (`FoldersLoaded` has_more_remote=false),
        // Account 2 fails (`OlderMessagesLoaded` has_more_remote=true).
        // Account 1 stays false while Account 2 stays true and retry remains available.
        let acc1_inbox_id = session.live.lock().unwrap().folders_by_account[&1][0].id;
        session.process_worker_events(vec![
            WorkerEvent::FoldersLoaded {
                account_id: 1,
                folders: vec![kitemail_mail_core::FolderSummary {
                    id: acc1_inbox_id,
                    account_id: 1,
                    name: "收件箱".to_string(),
                    path: "INBOX".to_string(),
                    unread_count: 0,
                    total_count: 2,
                    has_more_remote: false,
                }],
            },
            WorkerEvent::OlderMessagesLoaded {
                folder_id: 0,
                fetched_count: 0,
                has_more_remote: true,
            },
        ]);

        let live = session.live.lock().unwrap();
        assert!(!live.folders_by_account[&1][0].has_more_remote);
        assert!(live.folders_by_account[&2][0].has_more_remote);
        drop(live);

        let retry_out = session
            .load_older_messages(&LoadOlderMessagesInput {
                view: MainViewDto::UnifiedInbox,
                limit: 50,
            })
            .expect("unified retry remains available");
        assert!(retry_out.has_more_remote);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bootstrap_hydrates_folders_only_when_unlocked_and_skips_when_locked() {
        let (session, _, _, _, dir) = make_live_test_session(true, Some("1234"));
        {
            let mut live = session.live.lock().unwrap();
            live.folders_by_account.clear();
            live.cache_ready = false;
            live.snapshots_hydrated = false;
        }

        // While locked, bootstrap must return immediately with empty folders and NOT hydrate cache.
        session.lock_app(Some("manual"));
        let locked_boot = session.bootstrap();
        assert!(locked_boot.locked);
        assert!(!locked_boot.cache_ready);
        assert!(locked_boot.folders.is_empty());
        assert!(!session.live.lock().unwrap().snapshots_hydrated);

        // Unlock first, then bootstrap hydrates folders from read-only cache.
        let unlocked = session.unlock_app(&UnlockAppInput {
            passcode: "1234".to_string(),
        });
        assert!(unlocked.unlocked);
        let unlocked_boot = session.bootstrap();
        assert!(!unlocked_boot.locked);
        assert!(unlocked_boot.cache_ready);
        assert_eq!(unlocked_boot.folders.len(), 1);
        assert!(session.live.lock().unwrap().snapshots_hydrated);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
