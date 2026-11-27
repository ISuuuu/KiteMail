//! Mail session state for KiteMail Tauri preview (`DemoStore` in Milestone A/B,
//! structured for `MailWorkerHandle` integration in Milestone C).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use kitemail_mail_core::demo::{
    DemoStore, DEMO_ARCHIVE_ID, DEMO_DRAFTS_ID, DEMO_INBOX_ID, DEMO_INBOX_TOTAL, DEMO_SENT_ID,
};
use kitemail_mail_core::search_query::SearchQuery;
use kitemail_mail_core::{
    start_of_today_ts, MessageDetail, MessageSort, MessageSummary, QuickFilterSet,
};
use tauri::Emitter;

use crate::dto::{
    AccountSummaryDto, AppMode, AttachmentMetaDto, FolderSummaryDto, IpcError, ListMessagesInput,
    ListMessagesOutput, LockStateEvent, MailChangedEvent, MainBootstrapDto, MainViewDto,
    MessageFilterDto, MessageSummaryDto, OpenMessageInput, OpenMessageOutput, ReaderPositionDto,
    RefreshMailInput, RefreshMailOutput, SearchScopeDto, SyncStatusDto, SyncStatusEvent,
    ThreadMessageDto, EVENT_LOCK_STATE, EVENT_MAIL_CHANGED, EVENT_SYNC_STATUS, MAIN_WINDOW_LABEL,
};
use crate::preferences::PreferencesStore;
use crate::reader::{ReaderAppearance, ReaderDocumentSpec, ReaderManager};

pub const MAX_PAGE_LIMIT: usize = 500;
pub const MAX_QUERY_LEN: usize = 1000;

/// Abstraction for emitting main-window scoped events (`main://*`).
pub trait MainEventEmitter: Send + Sync {
    fn emit_mail_changed(&self, event: MailChangedEvent);
    fn emit_sync_status(&self, event: SyncStatusEvent);
    #[allow(dead_code)]
    fn emit_lock_state(&self, event: LockStateEvent);
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
        let _ = self
            .app_handle
            .emit_to(MAIN_WINDOW_LABEL, EVENT_LOCK_STATE, event);
    }
}

#[cfg(test)]
pub struct NullEventEmitter;

#[cfg(test)]
impl MainEventEmitter for NullEventEmitter {
    fn emit_mail_changed(&self, _: MailChangedEvent) {}
    fn emit_sync_status(&self, _: SyncStatusEvent) {}
    fn emit_lock_state(&self, _: LockStateEvent) {}
}

struct DemoSessionState {
    store: DemoStore,
    accounts: Vec<AccountSummaryDto>,
    base_folders: Vec<FolderSummaryDto>,
    inbox: Vec<MessageSummary>,
    sent: Vec<MessageSummary>,
    drafts: Vec<MessageSummary>,
    archive: Vec<MessageSummary>,
}

impl DemoSessionState {
    fn new() -> Self {
        let store = DemoStore::new();
        let accounts = store.accounts.iter().map(AccountSummaryDto::from).collect();
        let base_folders = store.folders.iter().map(FolderSummaryDto::from).collect();
        let inbox = store
            .list_messages_page(DEMO_INBOX_ID, "", 0, DEMO_INBOX_TOTAL)
            .items;
        let sent = store.list_messages_page(DEMO_SENT_ID, "", 0, 1_000).items;
        let drafts = store.list_messages_page(DEMO_DRAFTS_ID, "", 0, 1_000).items;
        let archive = store
            .list_messages_page(DEMO_ARCHIVE_ID, "", 0, 1_000)
            .items;
        Self {
            store,
            accounts,
            base_folders,
            inbox,
            sent,
            drafts,
            archive,
        }
    }

    fn folders(&self) -> Vec<FolderSummaryDto> {
        self.base_folders
            .iter()
            .map(|f| {
                let mut cloned = f.clone();
                let slice: &[MessageSummary] = match f.id {
                    DEMO_INBOX_ID => &self.inbox,
                    DEMO_SENT_ID => &self.sent,
                    DEMO_DRAFTS_ID => &self.drafts,
                    DEMO_ARCHIVE_ID => &self.archive,
                    _ => &[],
                };
                cloned.unread_count = slice.iter().filter(|m| m.unread).count() as u32;
                cloned.total_count = slice.len();
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
                MainViewDto::Junk | MainViewDto::Trash | MainViewDto::Snoozed => Vec::new(),
                MainViewDto::AccountFolder { folder_id, .. } => {
                    let slice: &[MessageSummary] = match folder_id {
                        DEMO_INBOX_ID => &self.inbox,
                        DEMO_SENT_ID => &self.sent,
                        DEMO_DRAFTS_ID => &self.drafts,
                        DEMO_ARCHIVE_ID => &self.archive,
                        _ => &[],
                    };
                    self.filter_slice(slice, &parsed, quick, today_start)
                }
            }
        };

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
            .find(|m| m.account_id == account_id && m.id == message_id)
    }

    fn get_detail(&self, account_id: u32, message_id: u32) -> Option<MessageDetail> {
        let summary = self.find_summary(account_id, message_id)?;
        let mut detail = self.store.get_message_detail(message_id)?;
        detail.flagged = summary.flagged;
        if let Some(first) = detail.thread_messages.first_mut() {
            if first.id == summary.id {
                first.unread = summary.unread;
                first.flagged = summary.flagged;
            }
        }
        Some(detail)
    }
}

fn to_quick_filters(filter: MessageFilterDto) -> QuickFilterSet {
    QuickFilterSet {
        unread: filter.unread.unwrap_or(false),
        starred: filter.starred.unwrap_or(false),
        has_attachment: filter.has_attachment.unwrap_or(false),
        today: filter.today.unwrap_or(false),
        ..QuickFilterSet::default()
    }
}

fn matches_quick_filters(m: &MessageSummary, filters: QuickFilterSet, today_start_ts: i64) -> bool {
    (!filters.unread || m.unread)
        && (!filters.starred || m.flagged)
        && (!filters.has_attachment || m.has_attachment)
        && (!filters.today || m.date_ts >= today_start_ts)
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

pub struct MailSession {
    mode: AppMode,
    locked: AtomicBool,
    state_version: AtomicU64,
    read_mark_gen: AtomicU64,
    current_view: Mutex<MainViewDto>,
    sync_status: Mutex<SyncStatusDto>,
    demo: Mutex<DemoSessionState>,
    prefs: Arc<PreferencesStore>,
    reader: Arc<ReaderManager>,
    emitter: Arc<dyn MainEventEmitter>,
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
                message: Some("真实账号同步将在阶段 C 接入，请使用 --demo 启动预览".to_string()),
                last_sync_at: None,
            },
        };
        Self {
            mode,
            locked: AtomicBool::new(false),
            state_version: AtomicU64::new(1),
            read_mark_gen: AtomicU64::new(0),
            current_view: Mutex::new(MainViewDto::UnifiedInbox),
            sync_status: Mutex::new(sync_status),
            demo: Mutex::new(DemoSessionState::new()),
            prefs,
            reader,
            emitter,
        }
    }

    #[allow(dead_code)]
    pub fn mode(&self) -> AppMode {
        self.mode
    }

    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::SeqCst)
    }

    pub fn state_version(&self) -> u64 {
        self.state_version.load(Ordering::SeqCst)
    }

    fn bump_state_version(&self) -> u64 {
        self.state_version.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn bootstrap(&self) -> MainBootstrapDto {
        let locked = self.is_locked();
        let state_version = self.state_version();
        let current_view = *self
            .current_view
            .lock()
            .expect("current_view mutex poisoned");
        let preferences = self.prefs.snapshot_dto();
        let sync_status = self
            .sync_status
            .lock()
            .expect("sync_status mutex poisoned")
            .clone();

        if locked || self.mode == AppMode::Live {
            return MainBootstrapDto {
                mode: self.mode,
                locked,
                state_version,
                accounts: Vec::new(),
                folders: Vec::new(),
                current_view,
                preferences,
                sync_status,
            };
        }

        let demo = self.demo.lock().expect("demo mutex poisoned");
        MainBootstrapDto {
            mode: self.mode,
            locked,
            state_version,
            accounts: demo.accounts.clone(),
            folders: demo.folders(),
            current_view,
            preferences,
            sync_status,
        }
    }

    pub fn list_messages(&self, input: &ListMessagesInput) -> Result<ListMessagesOutput, IpcError> {
        let req_id = Some(input.request_id.clone());
        if self.is_locked() {
            return Err(IpcError::locked(req_id));
        }
        if self.mode == AppMode::Live {
            return Ok(ListMessagesOutput {
                request_id: input.request_id.clone(),
                state_version: self.state_version(),
                offset: input.offset,
                total: 0,
                has_more: false,
                items: Vec::new(),
            });
        }

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

    pub fn open_message(
        &self,
        input: &OpenMessageInput,
        reader_session_id: String,
    ) -> Result<OpenMessageOutput, IpcError> {
        let req_id = Some(input.request_id.clone());
        if self.is_locked() {
            return Err(IpcError::locked(req_id));
        }
        if self.mode == AppMode::Live {
            return Err(IpcError::unavailable(
                "阶段 C 将接入真实邮箱正文加载",
                req_id,
            ));
        }

        let raw_prefs = self.prefs.snapshot_raw();
        let pref_dto = self.prefs.snapshot_dto();
        let reader_enabled = pref_dto.reader_position != ReaderPositionDto::Off;

        let (detail, mark_changed_info) = {
            let mut demo = self.demo.lock().expect("demo mutex poisoned");
            let detail = demo
                .get_detail(input.account_id, input.message_id)
                .ok_or_else(|| IpcError::message_not_found(input.message_id, req_id.clone()))?;

            let mut changed_info = None;
            let gen = self.read_mark_gen.fetch_add(1, Ordering::SeqCst) + 1;
            let _ = gen;
            if reader_enabled && raw_prefs.read_mark_mode == "shown" {
                if let Some(summary) = demo.find_summary_mut(input.account_id, input.message_id) {
                    if summary.unread {
                        summary.unread = false;
                        changed_info = Some((summary.account_id, summary.folder_id, summary.id));
                    }
                }
            }
            (detail, changed_info)
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
        }

        let appearance = ReaderAppearance {
            theme_id: pref_dto.theme_id,
            dark: pref_dto.dark_mode.unwrap_or(false),
            zoom_percent: pref_dto.reader_zoom_percent,
            block_remote_images: raw_prefs.block_remote_images,
        };

        let body_status = self.reader.begin_session(
            ReaderDocumentSpec {
                session_id: reader_session_id.clone(),
                account_id: detail.account_id,
                message_id: detail.id,
                subject: detail.subject.clone(),
                sender: detail.sender.clone(),
                plain_text: detail.body_text.clone(),
                html_body: detail.body_html.clone(),
                has_thread: detail.thread_messages.len() > 1,
            },
            &appearance,
            reader_enabled,
        );

        let attachments = detail
            .attachments
            .iter()
            .map(AttachmentMetaDto::from)
            .collect();
        let thread_messages = detail
            .thread_messages
            .iter()
            .map(ThreadMessageDto::from)
            .collect();

        Ok(OpenMessageOutput {
            request_id: input.request_id.clone(),
            reader_session_id,
            account_id: detail.account_id,
            message_id: detail.id,
            subject: detail.subject,
            sender: detail.sender,
            recipients: detail.recipients,
            cc: detail.cc,
            date_display: detail.date_display,
            body_status,
            plain_text: detail.body_text,
            has_html: detail.body_html.is_some(),
            html_body: detail.body_html,
            flagged: detail.flagged,
            attachments,
            thread_messages,
        })
    }

    #[allow(dead_code)]
    pub fn set_demo_message_unread(
        &self,
        account_id: u32,
        message_id: u32,
        unread: bool,
    ) -> Result<u64, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        let folder_id = {
            let mut demo = self.demo.lock().expect("demo mutex poisoned");
            let summary = demo
                .find_summary_mut(account_id, message_id)
                .ok_or_else(|| IpcError::message_not_found(message_id, None))?;
            summary.unread = unread;
            summary.folder_id
        };
        let ver = self.bump_state_version();
        self.emitter.emit_mail_changed(MailChangedEvent {
            state_version: ver,
            reason: if unread {
                "markUnread".to_string()
            } else {
                "markRead".to_string()
            },
            account_id: Some(account_id),
            folder_id: Some(folder_id),
            message_id: Some(message_id),
        });
        Ok(ver)
    }

    #[allow(dead_code)]
    pub fn set_demo_message_flagged(
        &self,
        account_id: u32,
        message_id: u32,
        flagged: bool,
    ) -> Result<u64, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        let folder_id = {
            let mut demo = self.demo.lock().expect("demo mutex poisoned");
            let summary = demo
                .find_summary_mut(account_id, message_id)
                .ok_or_else(|| IpcError::message_not_found(message_id, None))?;
            summary.flagged = flagged;
            summary.folder_id
        };
        let ver = self.bump_state_version();
        self.emitter.emit_mail_changed(MailChangedEvent {
            state_version: ver,
            reason: "toggleFlag".to_string(),
            account_id: Some(account_id),
            folder_id: Some(folder_id),
            message_id: Some(message_id),
        });
        Ok(ver)
    }

    pub fn refresh_mail(&self, input: &RefreshMailInput) -> Result<RefreshMailOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        if self.mode == AppMode::Demo {
            self.demo
                .lock()
                .expect("demo mutex poisoned")
                .validate_view(input.view, None)?;
        }
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
        *self.sync_status.lock().expect("sync_status mutex poisoned") = syncing_status.clone();
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

    #[allow(dead_code)]
    pub fn set_locked(&self, locked: bool) -> u64 {
        self.locked.store(locked, Ordering::SeqCst);
        self.read_mark_gen.fetch_add(1, Ordering::SeqCst);
        self.reader.set_locked(locked);
        let ver = self.bump_state_version();
        self.emitter.emit_lock_state(LockStateEvent {
            state_version: ver,
            locked,
        });
        ver
    }

    pub fn on_preferences_updated(&self) {
        let raw_prefs = self.prefs.snapshot_raw();
        let dto = self.prefs.snapshot_dto();
        let enabled = dto.reader_position != ReaderPositionDto::Off;
        if !enabled {
            self.read_mark_gen.fetch_add(1, Ordering::SeqCst);
        }
        self.reader.set_reader_enabled(enabled);
        self.reader.update_appearance(&ReaderAppearance {
            theme_id: dto.theme_id,
            dark: dto.dark_mode.unwrap_or(false),
            zoom_percent: dto.reader_zoom_percent,
            block_remote_images: raw_prefs.block_remote_images,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::{BodyStatusDto, MessageSortDto};
    use crate::reader::NullReaderSink;

    #[derive(Default)]
    struct TestEmitter {
        mail_changed: Mutex<Vec<MailChangedEvent>>,
        sync_status: Mutex<Vec<SyncStatusEvent>>,
        lock_state: Mutex<Vec<LockStateEvent>>,
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
    }

    fn make_test_session() -> (MailSession, Arc<TestEmitter>, std::path::PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("kitemail-tauri-session-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let prefs = Arc::new(PreferencesStore::open_in(dir.clone()));
        let reader = Arc::new(ReaderManager::new(Arc::new(NullReaderSink)));
        let emitter = Arc::new(TestEmitter::default());
        let session = MailSession::new(AppMode::Demo, prefs, reader, emitter.clone());
        (session, emitter, dir)
    }

    #[test]
    fn demo_session_paginates_filters_sorts_and_searches() {
        let (session, _, dir) = make_test_session();
        let boot = session.bootstrap();
        assert_eq!(boot.mode, AppMode::Demo);
        assert_eq!(boot.accounts.len(), 1);
        assert_eq!(boot.folders[0].total_count, 10_000);
        // DemoStore inbox actually contains 2 pinned unread + 124 generated unread = 126.
        assert_eq!(boot.folders[0].unread_count, 126);

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
        assert_eq!(page0.total, 10_000);
        assert_eq!(page0.items.len(), 50);
        assert!(page0.has_more);

        let last = session
            .list_messages(&ListMessagesInput {
                request_id: "r2".to_string(),
                view: MainViewDto::UnifiedInbox,
                query: String::new(),
                scope: SearchScopeDto::ThisFolder,
                sort: MessageSortDto::DateNewest,
                filter: MessageFilterDto::default(),
                offset: 9_980,
                limit: 50,
            })
            .expect("last page");
        assert_eq!(last.items.len(), 20);
        assert!(!last.has_more);

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
        assert_eq!(unread.total, 126);
        assert!(unread.items.iter().all(|m| m.unread));

        // Cross-folder search finds Drafts/Sent/Archive/Inbox matches.
        let search = session
            .list_messages(&ListMessagesInput {
                request_id: "r4".to_string(),
                view: MainViewDto::UnifiedInbox,
                query: "归档".to_string(),
                scope: SearchScopeDto::AllFolders,
                sort: MessageSortDto::DateOldest,
                filter: MessageFilterDto::default(),
                offset: 0,
                limit: 50,
            })
            .expect("cross-folder search");
        assert_eq!(search.total, 24);
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
        assert_eq!(opened.thread_messages.len(), 2);
        assert!(!opened.attachments.is_empty());
        assert_eq!(opened.body_status, BodyStatusDto::Ready);

        // `read_mark_mode == "shown"` marks message #1 read and bumps stateVersion.
        let boot_after = session.bootstrap();
        assert_eq!(boot_after.folders[0].unread_count, 125);
        assert_eq!(emitter.mail_changed.lock().unwrap().len(), 1);

        // Explicitly toggle unread back on and verify unreadCount & Unread view reflect it.
        session
            .set_demo_message_unread(1, 1, true)
            .expect("mark #1 unread");
        assert_eq!(session.bootstrap().folders[0].unread_count, 126);

        let unread_view = session
            .list_messages(&ListMessagesInput {
                request_id: "unread-view".to_string(),
                view: MainViewDto::Unread,
                query: String::new(),
                scope: SearchScopeDto::ThisFolder,
                sort: MessageSortDto::UnreadFirst,
                filter: MessageFilterDto::default(),
                offset: 0,
                limit: 10,
            })
            .expect("unread view");
        assert_eq!(unread_view.total, 126);

        session
            .set_demo_message_flagged(1, 2, true)
            .expect("flag message #2");
        let opened2 = session
            .open_message(
                &OpenMessageInput {
                    request_id: "open-2".to_string(),
                    account_id: 1,
                    message_id: 2,
                },
                "rs-2".to_string(),
            )
            .expect("open message #2");
        assert!(opened2.flagged);

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
        let (session, emitter, dir) = make_test_session();
        session.set_locked(true);
        assert_eq!(emitter.lock_state.lock().unwrap().len(), 1);

        let boot = session.bootstrap();
        assert!(boot.locked);
        assert!(boot.accounts.is_empty());
        assert!(boot.folders.is_empty());

        let err = session
            .open_message(
                &OpenMessageInput {
                    request_id: "open-locked".to_string(),
                    account_id: 1,
                    message_id: 1,
                },
                "rs-locked".to_string(),
            )
            .unwrap_err();
        assert_eq!(err.code, "LOCKED");
        assert_eq!(err.request_id.as_deref(), Some("open-locked"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
