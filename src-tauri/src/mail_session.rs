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
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use kitemail_mail_core::app_lock::{
    self, AutoLockTickInput, AutoLockTracker, AutoLockTrigger, BackgroundLockPolicy,
};
use kitemail_mail_core::cache::CacheStore;
use kitemail_mail_core::config::{self, AccountConfig};
use kitemail_mail_core::demo::{
    DemoStore, DEMO_ARCHIVE_ID, DEMO_DRAFTS_ID, DEMO_INBOX_ID, DEMO_INBOX_TOTAL, DEMO_SENT_ID,
};
use kitemail_mail_core::search_query::SearchQuery;
use kitemail_mail_core::{
    secrets, start_of_today_ts, MailPage, MailRequest, MessageDetail, MessageSort, MessageSummary,
    QuickFilterSet, WorkerEvent,
};
use tauri::Emitter;

use crate::dto::{
    AccountSummaryDto, AppMode, AttachmentMetaDto, BodyStatusDto, FolderSummaryDto, IpcError,
    ListMessagesInput, ListMessagesOutput, LockAppOutput, LockStateEvent, MailChangedEvent,
    MainBootstrapDto, MainViewDto, MessageFilterDto, MessageSummaryDto, OpenMessageInput,
    OpenMessageOutput, ReaderPositionDto, ReaderStatusEvent, RefreshMailInput, RefreshMailOutput,
    SearchScopeDto, SetMessageFlaggedInput, SetMessageFlaggedOutput, SetMessageSeenInput,
    SetMessageSeenOutput, SyncStatusDto, SyncStatusEvent, ThreadMessageDto, UnlockAppInput,
    UnlockAppOutput, EVENT_LOCK_STATE, EVENT_MAIL_CHANGED, EVENT_SYNC_STATUS, MAIN_WINDOW_LABEL,
};
use crate::preferences::{view_from_last_str, PreferencesStore};
use crate::reader::{ReaderAppearance, ReaderDocumentSpec, ReaderManager};

pub const MAX_PAGE_LIMIT: usize = 500;
pub const MAX_QUERY_LEN: usize = 1000;
const BODY_CACHE_CAPACITY: usize = 64;
const DEFAULT_BODY_FETCH_TIMEOUT: Duration = Duration::from_secs(20);
const DEFAULT_READ_MARK_DELAY: Duration = Duration::from_secs(2);

/// Abstraction for emitting main-window scoped events (`main://*`).
pub trait MainEventEmitter: Send + Sync {
    fn emit_mail_changed(&self, event: MailChangedEvent);
    fn emit_sync_status(&self, event: SyncStatusEvent);
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

/// Abstraction over `MailWorkerHandle` so Live session logic can be tested deterministically.
pub trait LiveWorkerSink: Send + Sync {
    fn send(&self, req: MailRequest);
    fn prepare_quit(&self);
}

struct RealMailWorkerSink {
    handle: Mutex<kitemail_mail_core::worker::MailWorkerHandle>,
}

impl LiveWorkerSink for RealMailWorkerSink {
    fn send(&self, req: MailRequest) {
        if let Ok(guard) = self.handle.lock() {
            guard.send(req);
        }
    }

    fn prepare_quit(&self) {
        if let Ok(guard) = self.handle.lock() {
            let _ = guard.prepare_quit_now();
        }
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

struct InFlightBodyEntry {
    generation: u64,
    reader_session_id: String,
    appearance: ReaderAppearance,
    reader_enabled: bool,
}

struct LiveSessionState {
    data_dir: PathBuf,
    worker: Option<Arc<dyn LiveWorkerSink>>,
    account_configs: Vec<AccountConfig>,
    accounts: Vec<AccountSummaryDto>,
    folders_by_account: HashMap<u32, Vec<FolderSummaryDto>>,
    cache_ready: bool,
    syncing_accounts: HashSet<u32>,
    in_flight_bodies: HashMap<(u32, u32), InFlightBodyEntry>,
    in_flight_gen: u64,
    body_cache: HashMap<(u32, u32), MessageDetail>,
    body_cache_order: VecDeque<(u32, u32)>,
    active_reader_target: Option<(String, u32, u32)>,
    pending_mark_thread_seen: Option<u32>,
}

impl LiveSessionState {
    fn empty() -> Self {
        Self {
            data_dir: PathBuf::new(),
            worker: None,
            account_configs: Vec::new(),
            accounts: Vec::new(),
            folders_by_account: HashMap::new(),
            cache_ready: false,
            syncing_accounts: HashSet::new(),
            in_flight_bodies: HashMap::new(),
            in_flight_gen: 0,
            body_cache: HashMap::new(),
            body_cache_order: VecDeque::new(),
            active_reader_target: None,
            pending_mark_thread_seen: None,
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

struct AppLockState {
    passcode_hash: Option<String>,
    locked: bool,
    locked_reason: Option<String>,
    persist_dir: Option<PathBuf>,
    failed_attempts: u32,
    cooldown_until: Option<Instant>,
    tracker: AutoLockTracker,
}

impl AppLockState {
    fn new(passcode_hash: Option<String>, locked: bool, persist_dir: Option<PathBuf>) -> Self {
        Self {
            passcode_hash,
            locked,
            locked_reason: locked.then(|| "startup".to_string()),
            persist_dir,
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

fn build_open_message_output(
    request_id: String,
    reader_session_id: String,
    detail: &MessageDetail,
    body_status: BodyStatusDto,
    include_body: bool,
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
        has_html: include_body && detail.body_html.is_some(),
        html_body: if include_body {
            detail.body_html.clone()
        } else {
            None
        },
        flagged: detail.flagged,
        attachments,
        thread_messages,
    }
}

pub struct MailSession {
    mode: AppMode,
    locked: AtomicBool,
    state_version: AtomicU64,
    read_mark_gen: AtomicU64,
    active_selected_msg: Mutex<Option<(u32, u32)>>,
    current_view: Mutex<MainViewDto>,
    sync_status: Mutex<SyncStatusDto>,
    lock_state: Mutex<AppLockState>,
    demo: Mutex<DemoSessionState>,
    live: Mutex<LiveSessionState>,
    prefs: Arc<PreferencesStore>,
    reader: Arc<ReaderManager>,
    emitter: Arc<dyn MainEventEmitter>,
    self_weak: Mutex<Weak<MailSession>>,
    tray: Mutex<Option<Arc<crate::tray::TrayHandle>>>,
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
            state_version: AtomicU64::new(1),
            read_mark_gen: AtomicU64::new(0),
            active_selected_msg: Mutex::new(None),
            current_view: Mutex::new(MainViewDto::UnifiedInbox),
            sync_status: Mutex::new(sync_status),
            lock_state: Mutex::new(AppLockState::new(None, false, None)),
            demo: Mutex::new(DemoSessionState::new()),
            live: Mutex::new(LiveSessionState::empty()),
            prefs,
            reader,
            emitter,
            self_weak: Mutex::new(Weak::new()),
            tray: Mutex::new(None),
            body_fetch_timeout: DEFAULT_BODY_FETCH_TIMEOUT,
            read_mark_delay: DEFAULT_READ_MARK_DELAY,
        }
    }

    /// Build and boot a Live-mode session: read-only cache first screen + `MailWorkerHandle` event loop.
    pub fn new_live(
        data_dir: PathBuf,
        prefs: Arc<PreferencesStore>,
        reader: Arc<ReaderManager>,
        emitter: Arc<dyn MainEventEmitter>,
    ) -> Arc<Self> {
        let accounts = config::load_accounts_in(&data_dir).unwrap_or_default();
        let account_dtos: Vec<AccountSummaryDto> =
            accounts.iter().map(AccountSummaryDto::from).collect();

        let (passcode_hash, initial_locked) = match secrets::load_app_lock_hash() {
            Ok(Some(hash)) => {
                let persisted = app_lock::is_lock_persisted_in(&data_dir);
                (Some(hash), persisted)
            }
            Ok(None) => {
                let _ = app_lock::set_lock_persisted_in(&data_dir, false);
                (None, false)
            }
            Err(_) => (None, false),
        };
        if initial_locked {
            reader.set_locked(true);
        }

        let mut folders_by_account: HashMap<u32, Vec<FolderSummaryDto>> = HashMap::new();
        let cache_ready = if config::global_db_in(&data_dir).exists() {
            match CacheStore::open_for_data_dir_read_only(&data_dir) {
                Ok(cache) => {
                    for acc in &accounts {
                        if let Ok(folders) = cache.list_folders(acc.id) {
                            folders_by_account.insert(
                                acc.id,
                                folders.iter().map(FolderSummaryDto::from).collect(),
                            );
                        }
                    }
                    true
                }
                Err(_) => false,
            }
        } else {
            false
        };

        let raw_prefs = prefs.snapshot_raw();
        let initial_view =
            resolve_initial_live_view(&raw_prefs, &accounts, &folders_by_account, cache_ready);

        let enabled_ids: HashSet<u32> = accounts
            .iter()
            .filter(|a| a.enabled)
            .map(|a| a.id)
            .collect();
        let initial_syncing = !enabled_ids.is_empty();
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
        live_state.cache_ready = cache_ready;
        live_state.syncing_accounts = enabled_ids;

        let session = Arc::new(Self {
            mode: AppMode::Live,
            locked: AtomicBool::new(initial_locked),
            state_version: AtomicU64::new(1),
            read_mark_gen: AtomicU64::new(0),
            active_selected_msg: Mutex::new(None),
            current_view: Mutex::new(initial_view),
            sync_status: Mutex::new(sync_status),
            lock_state: Mutex::new(AppLockState::new(
                passcode_hash,
                initial_locked,
                Some(data_dir.clone()),
            )),
            demo: Mutex::new(DemoSessionState::new()),
            live: Mutex::new(live_state),
            prefs,
            reader,
            emitter,
            self_weak: Mutex::new(Weak::new()),
            tray: Mutex::new(None),
            body_fetch_timeout: DEFAULT_BODY_FETCH_TIMEOUT,
            read_mark_delay: DEFAULT_READ_MARK_DELAY,
        });
        session.bind_self(&session);
        session.spawn_live_worker(data_dir);
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
        let worker = match kitemail_mail_core::worker::MailWorkerHandle::spawn_with_notifier(
            data_dir,
            Some(Arc::new(move || {
                let _ = notify_tx.send(());
            })),
        ) {
            Ok(w) => Arc::new(RealMailWorkerSink {
                handle: Mutex::new(w),
            }),
            Err(err) => {
                let mut status = self.sync_status.lock().expect("sync_status mutex poisoned");
                status.syncing = false;
                status.message = Some(format!("启动邮件后台线程失败: {err}"));
                return;
            }
        };

        let has_accounts = {
            let mut live = self.live.lock().expect("live mutex poisoned");
            live.worker = Some(worker.clone());
            !live.account_configs.is_empty()
        };

        if has_accounts {
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
        let worker_for_events = Arc::clone(&worker);
        let _ = std::thread::Builder::new()
            .name("kitemail-tauri-worker-events".to_string())
            .spawn(move || loop {
                let events = worker_for_events
                    .handle
                    .lock()
                    .map(|w| w.drain_events())
                    .unwrap_or_default();
                if !events.is_empty() {
                    if let Some(session) = weak_session.upgrade() {
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

    pub fn current_view(&self) -> MainViewDto {
        *self
            .current_view
            .lock()
            .expect("current_view mutex poisoned")
    }

    pub fn state_version(&self) -> u64 {
        self.state_version.load(Ordering::SeqCst)
    }

    fn bump_state_version(&self) -> u64 {
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

    fn refresh_tray_tooltip(&self) {
        if let Some(ref tray) = *self.tray.lock().expect("tray mutex poisoned") {
            tray.set_tooltip(crate::tray::format_tray_tooltip(self.total_unread_count()));
        }
    }

    pub fn bootstrap(&self) -> MainBootstrapDto {
        let locked = self.is_locked();
        let has_passcode = self.has_passcode();
        let state_version = self.state_version();
        let current_view = self.current_view();
        let preferences = self.prefs.snapshot_dto();
        let sync_status = self
            .sync_status
            .lock()
            .expect("sync_status mutex poisoned")
            .clone();

        if locked {
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
            };
        }

        match self.mode {
            AppMode::Demo => {
                let demo = self.demo.lock().expect("demo mutex poisoned");
                MainBootstrapDto {
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
                }
            }
            AppMode::Live => {
                let live = self.live.lock().expect("live mutex poisoned");
                MainBootstrapDto {
                    mode: self.mode,
                    locked: false,
                    has_passcode,
                    cache_ready: live.cache_ready,
                    state_version,
                    accounts: live.accounts.clone(),
                    folders: live.all_folders(),
                    current_view,
                    preferences,
                    sync_status,
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
                *self
                    .current_view
                    .lock()
                    .expect("current_view mutex poisoned") = input.view;
                self.prefs.update_last_location(input.view);
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

    // [MERGE-POINT: core-worker-query-page]
    // 待 c-core-worker 的 `MailRequest::QueryPage` / `WorkerEvent::PageQueried` 合并后，
    // 可将此处直接只读 CacheStore 查询切换为单次 worker 查询接口，保留相同的视图校验与 DTO 映射。
    fn query_live_messages_page(&self, input: &ListMessagesInput) -> Result<MailPage, IpcError> {
        let req_id = Some(input.request_id.clone());
        let (data_dir, cache_ready, enabled_ids, unified_ids, folders) = {
            let live = self.live.lock().expect("live mutex poisoned");
            (
                live.data_dir.clone(),
                live.cache_ready,
                live.enabled_account_ids(),
                live.unified_account_ids(),
                live.all_folders(),
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

        if !config::global_db_in(&data_dir).exists() {
            return Ok(MailPage {
                folder_id: 0,
                offset: input.offset,
                total: 0,
                items: Vec::new(),
            });
        }

        let cache = CacheStore::open_for_data_dir_read_only(&data_dir).map_err(|e| {
            IpcError::internal(format!("打开本地只读缓存失败: {e}"), req_id.clone())
        })?;

        let quick = to_quick_filters(input.filter);
        let sort: MessageSort = input.sort.into();
        let limit = input.limit.min(MAX_PAGE_LIMIT);
        let cross_folder =
            !input.query.trim().is_empty() && input.scope == SearchScopeDto::AllFolders;

        let res = if cross_folder {
            cache.search_all_folders(&enabled_ids, &input.query, quick, sort, input.offset, limit)
        } else {
            match input.view {
                MainViewDto::UnifiedInbox => cache.list_unified_inbox_for_accounts(
                    &unified_ids,
                    &input.query,
                    quick,
                    sort,
                    input.offset,
                    limit,
                ),
                MainViewDto::Unread => cache.list_unread_messages_for_accounts(
                    &unified_ids,
                    &input.query,
                    quick,
                    sort,
                    input.offset,
                    limit,
                ),
                MainViewDto::Starred => cache.list_starred_messages_for_accounts(
                    &unified_ids,
                    &input.query,
                    quick,
                    sort,
                    input.offset,
                    limit,
                ),
                MainViewDto::Sent => cache.list_sent_messages_for_accounts(
                    &unified_ids,
                    &input.query,
                    quick,
                    sort,
                    input.offset,
                    limit,
                ),
                MainViewDto::Archive => cache.list_archived_messages_for_accounts(
                    &unified_ids,
                    &input.query,
                    quick,
                    sort,
                    input.offset,
                    limit,
                ),
                MainViewDto::Junk => cache.list_junk_messages_for_accounts(
                    &unified_ids,
                    &input.query,
                    quick,
                    sort,
                    input.offset,
                    limit,
                ),
                MainViewDto::Trash => cache.list_trash_messages_for_accounts(
                    &unified_ids,
                    &input.query,
                    quick,
                    sort,
                    input.offset,
                    limit,
                ),
                MainViewDto::Snoozed => cache.list_snoozed_messages_for_accounts(
                    &unified_ids,
                    &input.query,
                    quick,
                    sort,
                    input.offset,
                    limit,
                ),
                MainViewDto::Drafts => {
                    list_live_drafts_page(&cache, &input.query, input.offset, limit)
                }
                MainViewDto::AccountFolder { folder_id, .. } => cache
                    .list_messages_page_with_options(
                        folder_id,
                        &input.query,
                        quick,
                        sort,
                        input.offset,
                        limit,
                    ),
            }
        };

        res.map_err(|e| IpcError::internal(format!("查询缓存列表失败: {e}"), req_id))
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
                &raw_prefs.read_mark_mode,
                reader_enabled,
                &appearance,
            ),
            AppMode::Live => self.open_live_message(
                input,
                reader_session_id,
                gen,
                raw_prefs.thread_newest_first,
                &raw_prefs.read_mark_mode,
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
        read_mark_mode: &str,
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
            if reader_enabled && read_mark_mode == "shown" && was_unread {
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
        } else if reader_enabled && read_mark_mode == "delay" && unread_now {
            self.schedule_delayed_read_mark(
                gen,
                detail.account_id,
                detail.id,
                detail.uid,
                detail.folder_path.clone(),
            );
        }

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
            appearance,
            reader_enabled,
        );

        Ok(build_open_message_output(
            input.request_id.clone(),
            reader_session_id,
            &detail,
            body_status,
            true,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn open_live_message(
        &self,
        input: &OpenMessageInput,
        reader_session_id: String,
        gen: u64,
        newest_first: bool,
        read_mark_mode: &str,
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
                .get_message_detail_ordered(input.message_id, newest_first)
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
                read_mark_mode,
                reader_enabled,
                true,
            );
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
            return Ok(build_open_message_output(
                input.request_id.clone(),
                reader_session_id,
                &detail,
                body_status,
                true,
            ));
        }

        // Body is not cached locally yet -> enqueue deduplicated LoadMessageBody request.
        let (worker_to_send, timeout_gen) = {
            let mut live = self.live.lock().expect("live mutex poisoned");
            live.active_reader_target =
                Some((reader_session_id.clone(), detail.account_id, detail.id));
            if reader_enabled && read_mark_mode == "shown" && is_unread {
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
            read_mark_mode,
            reader_enabled,
            false,
        );

        Ok(build_open_message_output(
            input.request_id.clone(),
            reader_session_id,
            &detail,
            BodyStatusDto::Loading,
            false,
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
                });
            }
        }
    }

    pub fn process_worker_events(&self, events: Vec<WorkerEvent>) {
        for evt in events {
            match evt {
                WorkerEvent::AccountsUpdated { accounts } => {
                    let dtos: Vec<AccountSummaryDto> =
                        accounts.iter().map(AccountSummaryDto::from).collect();
                    {
                        let mut live = self.live.lock().expect("live mutex poisoned");
                        live.account_configs = accounts;
                        live.accounts = dtos;
                        live.cache_ready = true;
                    }
                    let ver = self.bump_state_version();
                    self.emitter.emit_mail_changed(MailChangedEvent {
                        state_version: ver,
                        reason: "accountsUpdated".to_string(),
                        account_id: None,
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
                WorkerEvent::Error {
                    account_id: _,
                    message,
                } => {
                    let ver = self.bump_state_version();
                    let next_status = {
                        let mut status =
                            self.sync_status.lock().expect("sync_status mutex poisoned");
                        status.syncing = false;
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

        self.reader.begin_session(
            ReaderDocumentSpec {
                session_id: entry.reader_session_id,
                account_id: detail.account_id,
                message_id: detail.id,
                subject: detail.subject,
                sender: detail.sender,
                plain_text: detail.body_text,
                html_body: detail.body_html,
                has_thread: detail.thread_messages.len() > 1,
            },
            &entry.appearance,
            entry.reader_enabled,
        );
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
        self.reader.update_appearance(&ReaderAppearance {
            theme_id: dto.theme_id,
            dark: dto.dark_mode.unwrap_or(false),
            zoom_percent: dto.reader_zoom_percent,
            block_remote_images: raw_prefs.block_remote_images,
        });
    }

    pub fn prepare_quit(&self) {
        self.read_mark_gen.fetch_add(1, Ordering::SeqCst);
        if self.mode == AppMode::Live {
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
mod tests {
    use super::*;
    use crate::dto::{MainPreferencesPatch, MessageSortDto};
    use crate::reader::NullReaderSink;
    use kitemail_mail_core::cache::{CachedHeaderInput, RemoteFolder};
    use kitemail_mail_core::config::{save_accounts_in, AccountConfig};

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

    #[derive(Default)]
    struct RecordingReaderSink {
        events: Mutex<Vec<ReaderStatusEvent>>,
    }

    impl crate::reader::ReaderPlatformSink for RecordingReaderSink {
        fn is_native_supported(&self) -> bool {
            false
        }
        fn load_html(&self, _: String, _: u32, _: u32, _: Vec<u8>, _: bool) {}
        fn apply_layout(
            &self,
            _: crate::window_lifecycle::PhysicalRect,
            _: crate::window_lifecycle::PhysicalRect,
            _: bool,
        ) {
        }
        fn set_visible(&self, _: bool) {}
        fn set_zoom(&self, _: u32) {}
        fn close(&self) {}
        fn emit_status(&self, event: ReaderStatusEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    #[derive(Default)]
    struct RecordingWorkerSink {
        requests: Mutex<Vec<MailRequest>>,
    }

    impl LiveWorkerSink for RecordingWorkerSink {
        fn send(&self, req: MailRequest) {
            self.requests.lock().unwrap().push(req);
        }
        fn prepare_quit(&self) {}
    }

    fn make_test_session() -> (MailSession, Arc<TestEmitter>, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("kitemail-tauri-session-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let prefs = Arc::new(PreferencesStore::open_in(dir.clone()));
        let reader = Arc::new(ReaderManager::new(Arc::new(NullReaderSink)));
        let emitter = Arc::new(TestEmitter::default());
        let session = MailSession::new(AppMode::Demo, prefs, reader, emitter.clone());
        (session, emitter, dir)
    }

    fn make_live_test_session(
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

        let session = Arc::new(MailSession {
            mode: AppMode::Live,
            locked: AtomicBool::new(persisted_lock),
            state_version: AtomicU64::new(1),
            read_mark_gen: AtomicU64::new(0),
            active_selected_msg: Mutex::new(None),
            current_view: Mutex::new(MainViewDto::UnifiedInbox),
            sync_status: Mutex::new(SyncStatusDto::default()),
            lock_state: Mutex::new(AppLockState::new(
                passcode_hash,
                persisted_lock,
                Some(dir.clone()),
            )),
            demo: Mutex::new(DemoSessionState::new()),
            live: Mutex::new(live),
            prefs,
            reader,
            emitter: emitter.clone(),
            self_weak: Mutex::new(Weak::new()),
            tray: Mutex::new(None),
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
        assert_eq!(boot.accounts.len(), 1);
        assert_eq!(boot.folders[0].total_count, 10_000);
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

        let boot_after = session.bootstrap();
        assert_eq!(boot_after.folders[0].unread_count, 125);
        assert_eq!(emitter.mail_changed.lock().unwrap().len(), 1);

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
}
