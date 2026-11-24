//! Dedicated Tokio worker thread orchestrating cache, credentials, IMAP, SMTP, and Graph OAuth2.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::cache::{AttachmentRecord, CacheStore};
use crate::config::{self, AccountConfig, AuthBackend};
use crate::search_query::SearchQuery;
use crate::secrets::{self, AccountSecrets};
use crate::smtp_client::SendOutcome;
use crate::{
    graph_client, imap_client, oauth, smtp_client, FetchedMessageBody, MailRequest, MessageDetail,
    MessageFilter, MessageSort, MessageSummary, OutboxItem, OutboxStatus, QuickFilterSet,
    UnifiedView, WorkerEvent,
};

type TokenCache = HashMap<u32, (String, Instant)>;
type IdleWatchers = HashMap<u32, tokio::task::JoinHandle<()>>;
type SharedAsyncCache = Arc<std::sync::Mutex<Option<CacheStore>>>;

#[derive(Clone, Default)]
struct SharedTokenCache {
    cache: Arc<tokio::sync::Mutex<TokenCache>>,
    refresh_locks: Arc<std::sync::Mutex<HashMap<u32, Arc<tokio::sync::Mutex<()>>>>>,
}

impl SharedTokenCache {
    async fn insert(&self, account_id: u32, token: String, expires_at: Instant) {
        let mut guard = self.cache.lock().await;
        guard.insert(account_id, (token, expires_at));
    }

    async fn remove(&self, account_id: u32) {
        {
            let mut guard = self.cache.lock().await;
            guard.remove(&account_id);
        }
        if let Ok(mut locks) = self.refresh_locks.lock() {
            locks.remove(&account_id);
        }
    }

    fn account_refresh_lock(&self, account_id: u32) -> Arc<tokio::sync::Mutex<()>> {
        let mut guard = self
            .refresh_locks
            .lock()
            .expect("refresh lock map poisoned");
        guard
            .entry(account_id)
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    async fn cached_valid_token(&self, account_id: u32) -> Option<String> {
        let guard = self.cache.lock().await;
        guard
            .get(&account_id)
            .filter(|(_, exp)| Instant::now() < *exp)
            .map(|(tok, _)| tok.clone())
    }

    async fn ensure_graph_token(&self, account: &AccountConfig) -> Result<String, String> {
        if let Some(tok) = self.cached_valid_token(account.id).await {
            return Ok(tok);
        }
        let refresh_lock = self.account_refresh_lock(account.id);
        let _refresh_guard = refresh_lock.lock().await;
        if let Some(tok) = self.cached_valid_token(account.id).await {
            return Ok(tok);
        }

        let refresh_token = secrets::load_oauth_refresh_token(account.id)?;
        let client_id = account.oauth_client_id.clone();
        let refreshed = tokio::task::spawn_blocking(move || {
            oauth::refresh_microsoft_token(&client_id, &refresh_token)
        })
        .await
        .map_err(|e| format!("刷新令牌任务异常: {e}"))??;

        if let Some(new_rt) = refreshed.refresh_token {
            let _ = secrets::save_oauth_refresh_token(account.id, new_rt);
        }
        let expires_at =
            Instant::now() + Duration::from_secs(refreshed.expires_in_secs.saturating_sub(120));
        self.insert(account.id, refreshed.access_token.clone(), expires_at)
            .await;
        Ok(refreshed.access_token)
    }
}

fn open_existing_or_create_cache(db_path: &Path) -> Result<CacheStore, String> {
    if db_path.exists() {
        if let Ok(c) = CacheStore::open_existing(db_path) {
            return Ok(c);
        }
    }
    CacheStore::open(db_path)
}

fn with_shared_cache<T>(
    shared: &SharedAsyncCache,
    data_dir: &Path,
    f: impl FnOnce(&mut CacheStore) -> Result<T, String>,
) -> Result<T, String> {
    let mut guard = shared.lock().map_err(|_| "异步缓存锁已损坏".to_string())?;
    if guard.is_none() {
        *guard = Some(open_existing_or_create_cache(&config::cache_db_in(
            data_dir,
        ))?);
    }
    let cache = guard
        .as_mut()
        .ok_or_else(|| "异步缓存初始化失败".to_string())?;
    let res = f(cache);
    if res.is_err() {
        *guard = None;
    }
    res
}

#[derive(Clone)]
pub(crate) struct EventSender {
    tx: mpsc::Sender<WorkerEvent>,
    on_event: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl EventSender {
    fn new(tx: mpsc::Sender<WorkerEvent>, on_event: Option<Arc<dyn Fn() + Send + Sync>>) -> Self {
        Self { tx, on_event }
    }

    fn noop() -> Self {
        let (tx, _rx) = mpsc::channel();
        Self { tx, on_event: None }
    }

    pub(crate) fn send(&self, evt: WorkerEvent) -> Result<(), mpsc::SendError<WorkerEvent>> {
        let res = self.tx.send(evt);
        if res.is_ok() {
            if let Some(ref cb) = self.on_event {
                cb();
            }
        }
        res
    }
}

struct InFlightBodyGuard {
    message_id: u32,
    set: Arc<std::sync::Mutex<HashSet<u32>>>,
}

impl Drop for InFlightBodyGuard {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.set.lock() {
            guard.remove(&self.message_id);
        }
    }
}

/// 任务四：后台历史索引每批拉取的邮件数上限。比前台滚动拉取的 100 小，
/// 批次在独立任务中执行，不占用交互请求（读信 / 搜索 / 发信）队列。
const BACKFILL_HISTORY_BATCH: usize = 25;
/// 任务四：后台索引连续失败达到该次数后放弃该文件夹本次会话（对齐 Linux
/// `BACKFILL_TRIES`，避免对总是失败的文件夹紧循环；游标保留，重启后续跑）。
const BACKFILL_HISTORY_TRIES: u32 = 3;

/// Handle owned by the UI to dispatch [`MailRequest`]s and poll [`WorkerEvent`]s.
pub struct MailWorkerHandle {
    req_tx: mpsc::Sender<MailRequest>,
    evt_rx: mpsc::Receiver<WorkerEvent>,
    /// 数据目录：退出时主线程直接打开缓存库落盘，不排队等待单线程 worker。
    data_dir: PathBuf,
    /// UI 线程只读 SQLite 连接：已缓存正文直接毫秒级读取，不进 worker 队列。
    read_cache: std::sync::Mutex<Option<CacheStore>>,
}

impl MailWorkerHandle {
    /// Spawn the background Tokio runtime and worker loop using `data_dir`.
    pub fn spawn(data_dir: PathBuf) -> Result<Self, String> {
        Self::spawn_with_notifier(data_dir, None)
    }

    /// Spawn the background worker with an optional callback invoked whenever a new
    /// [`WorkerEvent`] is queued for the UI.
    pub fn spawn_with_notifier(
        data_dir: PathBuf,
        on_event: Option<Arc<dyn Fn() + Send + Sync>>,
    ) -> Result<Self, String> {
        let (req_tx, req_rx) = mpsc::channel::<MailRequest>();
        let (raw_evt_tx, evt_rx) = mpsc::channel::<WorkerEvent>();
        let evt_tx = EventSender::new(raw_evt_tx, on_event);
        let worker_req_tx = req_tx.clone();
        let worker_data_dir = data_dir.clone();

        std::thread::Builder::new()
            .name("hylki-mail-worker".to_string())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .enable_all()
                    .worker_threads(2)
                    .build()
                    .expect("failed to start Tokio runtime");

                rt.block_on(worker_loop(worker_data_dir, worker_req_tx, req_rx, evt_tx));
            })
            .map_err(|e| format!("启动邮件后台线程失败: {e}"))?;

        Ok(Self {
            req_tx,
            evt_rx,
            data_dir,
            read_cache: std::sync::Mutex::new(None),
        })
    }

    /// Read already-cached message details directly on the UI thread via a read-only
    /// SQLite connection, bypassing the worker channel completely.
    pub fn try_load_cached_message(
        &self,
        message_id: u32,
        newest_first: bool,
    ) -> Result<Option<(MessageDetail, bool)>, String> {
        let Ok(mut guard) = self.read_cache.lock() else {
            return Ok(None);
        };
        if guard.is_none() {
            let db_path = config::cache_db_in(&self.data_dir);
            if !db_path.exists() {
                return Ok(None);
            }
            match CacheStore::open_read_only(&db_path) {
                Ok(store) => *guard = Some(store),
                Err(_) => return Ok(None),
            }
        }
        let Some(store) = guard.as_ref() else {
            return Ok(None);
        };
        match store.get_message_detail_ordered(message_id, newest_first) {
            Ok(Some((detail, _uid, body_loaded))) => Ok(Some((detail, body_loaded))),
            Ok(None) => Ok(None),
            Err(_) => {
                *guard = None;
                Ok(None)
            }
        }
    }

    /// 同步准备退出：不经 worker 队列，直接把仍排队的发件箱记录标记为到期，
    /// 使下次启动立即续发（对齐 Linux 主线程直接落盘的退出语义）。即使 worker
    /// 正在执行耗时的同步或超时重连也不会阻塞关窗。
    pub fn prepare_quit_now(&self) -> Result<(), String> {
        let mut cache = CacheStore::open(&config::cache_db_in(&self.data_dir))?;
        cache.expire_queued_outbox()
    }

    /// Send a request to the background mail worker.
    pub fn send(&self, req: MailRequest) {
        let _ = self.req_tx.send(req);
    }

    pub fn send_checked(&self, req: MailRequest) -> Result<(), String> {
        self.req_tx
            .send(req)
            .map_err(|_| "邮件后台线程已停止".to_string())
    }

    /// Non-blocking drain of all pending [`WorkerEvent`]s.
    pub fn drain_events(&self) -> Vec<WorkerEvent> {
        let mut out = Vec::new();
        while let Ok(evt) = self.evt_rx.try_recv() {
            out.push(evt);
        }
        out
    }
}

async fn worker_loop(
    data_dir: PathBuf,
    req_tx: mpsc::Sender<MailRequest>,
    req_rx: mpsc::Receiver<MailRequest>,
    evt_tx: EventSender,
) {
    let db_path = config::cache_db_in(&data_dir);
    let mut cache = match CacheStore::open(&db_path) {
        Ok(c) => c,
        Err(e) => {
            let _ = evt_tx.send(WorkerEvent::Error {
                account_id: None,
                message: e,
            });
            return;
        }
    };
    let tokens = SharedTokenCache::default();
    let async_cache: SharedAsyncCache = Arc::new(std::sync::Mutex::new(
        open_existing_or_create_cache(&db_path).ok(),
    ));
    let in_flight_bodies: Arc<std::sync::Mutex<HashSet<u32>>> =
        Arc::new(std::sync::Mutex::new(HashSet::new()));
    let mut watchers: IdleWatchers = HashMap::new();
    let background = BackgroundTasks::new(
        data_dir.clone(),
        db_path.clone(),
        tokens.clone(),
        evt_tx.clone(),
    );

    if let Ok(prefs) = config::load_preferences_in(&data_dir) {
        if prefs.keep_downloads_days > 0 {
            let keep_days = prefs.keep_downloads_days;
            let async_cache_cl = Arc::clone(&async_cache);
            let prune_data_dir = data_dir.clone();
            tokio::task::spawn_blocking(move || {
                let now_ts = chrono::Utc::now().timestamp();
                let _ = with_shared_cache(&async_cache_cl, &prune_data_dir, |c| {
                    c.prune_expired_downloads(keep_days, now_ts)
                });
            });
        }
    }

    if let Ok(accounts) = config::load_accounts_in(&data_dir) {
        cache.set_own_emails(accounts.iter().map(|a| a.email.clone()).collect());
        for acc in &accounts {
            refresh_account_watcher(&mut watchers, &data_dir, acc, &req_tx);
        }
        deliver_due_outbox(&data_dir, &mut cache, &tokens, &accounts, &evt_tx).await;
    }
    wake_due_snoozed_messages(&mut cache, &evt_tx);

    // 主循环不再无限阻塞等请求：按「下一条定时记录的到期时刻」限时等待（clamp 到
    // [`OUTBOX_TICK_MIN`]..[`OUTBOX_TICK_MAX`]，即最长 30s），醒来先投递已到期的
    // 定时件。这样「明天早上 8 点发」不依赖用户任何操作也能按时发出；同时间隔上限
    // 30s 也顺带承担了稍后提醒（snooze）的巡检粒度。
    loop {
        // 请求通道是标准库 mpsc（阻塞式），这里用 `recv_timeout` 替代直接
        // `recv`：worker 独占自己的 runtime 线程，限时阻塞不影响 IDLE 推送。
        let req = match req_rx.recv_timeout(next_outbox_wait(&cache)) {
            Ok(req) => Some(req),
            // 没有新请求：落到下面做一次到期巡检。
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            // 发送端已释放：退出主循环。
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };

        // 定时发送投递：内部先查有无到期记录，没有就立刻返回（不读盘），
        // 所以每次唤醒都调用也不影响交互请求的响应速度。
        deliver_due_outbox_tick(&data_dir, &mut cache, &tokens, &evt_tx).await;

        let Some(req) = req else {
            // 空闲唤醒才巡检稍后提醒，避免在交互请求路径上多加一次查库。
            wake_due_snoozed_messages(&mut cache, &evt_tx);
            continue;
        };
        if matches!(req, MailRequest::PrepareQuit) {
            let result = cache.expire_queued_outbox();
            let prepared = result.is_ok();
            let _ = evt_tx.send(WorkerEvent::QuitPrepared { result });
            if prepared {
                break;
            }
            continue;
        }
        // 后台任务（历史索引 / 账号与文件夹同步）改派到独立任务执行，不占用
        // 本条交互队列：否则点开一封未缓存正文的邮件时，正文请求要排在整批
        // 索引或同步之后（表现为点击后很久才有反应）。
        if background.dispatch_if_background(&req) {
            continue;
        }
        handle_request(
            &data_dir,
            &mut cache,
            &tokens,
            &async_cache,
            &in_flight_bodies,
            &mut watchers,
            &req_tx,
            req,
            &evt_tx,
        )
        .await;
    }

    for (_, handle) in watchers.drain() {
        handle.abort();
    }
}

/// 可改派到后台执行的任务：历史索引与账号 / 文件夹同步。
enum BackgroundJob {
    Backfill,
    SyncAllAccounts,
    SyncAccount(u32),
    SyncFolder {
        account_id: u32,
        folder_id: u32,
        folder_path: String,
    },
}

impl BackgroundJob {
    fn from_request(req: &MailRequest) -> Option<Self> {
        match req {
            MailRequest::BackfillHistoryBatch => Some(Self::Backfill),
            MailRequest::SyncAllAccounts => Some(Self::SyncAllAccounts),
            MailRequest::SyncAccount { account_id } => Some(Self::SyncAccount(*account_id)),
            MailRequest::SyncFolder {
                account_id,
                folder_id,
                folder_path,
            } => Some(Self::SyncFolder {
                account_id: *account_id,
                folder_id: *folder_id,
                folder_path: folder_path.clone(),
            }),
            _ => None,
        }
    }

    async fn run(
        self,
        data_dir: &PathBuf,
        tokens: &SharedTokenCache,
        backfill: Arc<tokio::sync::Mutex<BackfillIndexState>>,
        bg_cache: &Arc<tokio::sync::Mutex<Option<CacheStore>>>,
        db_path: &PathBuf,
        evt_tx: &EventSender,
    ) {
        let mut cache_guard = bg_cache.lock().await;
        if cache_guard.is_none() {
            match open_existing_or_create_cache(db_path) {
                Ok(c) => *cache_guard = Some(c),
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: None,
                        message: e,
                    });
                    if matches!(&self, Self::Backfill) {
                        // 同时回一个「无剩余候选」的批次完成事件，避免界面端的
                        // in_flight 标志永远卡住导致索引不再续排。
                        let _ = evt_tx.send(WorkerEvent::BackfillHistoryBatchDone {
                            folder_id: 0,
                            fetched_count: 0,
                            body_only: false,
                            has_more_remote: false,
                        });
                    }
                    return;
                }
            }
        }
        let Some(cache) = cache_guard.as_mut() else {
            return;
        };
        // 独立连接也要设置本机账号邮箱，保证「发给我」类筛选与主连接口径一致。
        let accounts = config::load_accounts_in(data_dir).unwrap_or_default();
        cache.set_own_emails(accounts.iter().map(|a| a.email.clone()).collect());
        match self {
            Self::Backfill => {
                let mut state = backfill.lock().await;
                backfill_history_batch_inner(data_dir, cache, tokens, &mut state, evt_tx).await;
            }
            Self::SyncAllAccounts => {
                for acc in accounts.into_iter().filter(|a| a.enabled) {
                    sync_account_inner(data_dir, cache, tokens, acc.id, evt_tx).await;
                }
                let page_size = pref_page_size(data_dir);
                let allowed = unified_account_ids(data_dir);
                if let Ok(page) = cache.list_unified_inbox_for_accounts(
                    &allowed,
                    "",
                    QuickFilterSet::default(),
                    MessageSort::DateNewest,
                    0,
                    page_size,
                ) {
                    let _ = evt_tx.send(WorkerEvent::MessagesPageLoaded(page));
                }
            }
            Self::SyncAccount(account_id) => {
                sync_account_inner(data_dir, cache, tokens, account_id, evt_tx).await;
            }
            Self::SyncFolder {
                account_id,
                folder_id,
                folder_path,
            } => {
                let Ok(account) = load_account_config(data_dir, account_id) else {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: Some(account_id),
                        message: format!("账号 #{account_id} 未找到配置"),
                    });
                    return;
                };
                let page_size = pref_page_size(data_dir);
                sync_folder_inner(
                    cache,
                    tokens,
                    &account,
                    folder_id,
                    &folder_path,
                    page_size,
                    evt_tx,
                )
                .await;
            }
        }
    }
}

/// 后台任务执行环境：在串行信号量内懒加载并复用一条 SQLite 连接，并与交互路径共享同一 Graph 令牌缓存。
struct BackgroundTasks {
    gate: Arc<tokio::sync::Semaphore>,
    backfill: Arc<tokio::sync::Mutex<BackfillIndexState>>,
    tokens: SharedTokenCache,
    cache: Arc<tokio::sync::Mutex<Option<CacheStore>>>,
    data_dir: PathBuf,
    db_path: PathBuf,
    evt_tx: EventSender,
}

impl BackgroundTasks {
    fn new(
        data_dir: PathBuf,
        db_path: PathBuf,
        tokens: SharedTokenCache,
        evt_tx: EventSender,
    ) -> Self {
        Self {
            gate: Arc::new(tokio::sync::Semaphore::new(1)),
            backfill: Arc::new(tokio::sync::Mutex::new(BackfillIndexState::default())),
            tokens,
            cache: Arc::new(tokio::sync::Mutex::new(None)),
            data_dir,
            db_path,
            evt_tx,
        }
    }

    /// 后台请求在此改派到独立任务并返回 true（调用方跳过交互队列）。
    fn dispatch_if_background(&self, req: &MailRequest) -> bool {
        let Some(job) = BackgroundJob::from_request(req) else {
            return false;
        };
        let gate = Arc::clone(&self.gate);
        let backfill = Arc::clone(&self.backfill);
        let tokens = self.tokens.clone();
        let bg_cache = Arc::clone(&self.cache);
        let data_dir = self.data_dir.clone();
        let db_path = self.db_path.clone();
        let evt_tx = self.evt_tx.clone();
        tokio::spawn(async move {
            let Ok(_permit) = gate.acquire_owned().await else {
                return;
            };
            job.run(&data_dir, &tokens, backfill, &bg_cache, &db_path, &evt_tx)
                .await;
        });
        true
    }
}

fn wake_due_snoozed_messages(cache: &mut CacheStore, evt_tx: &EventSender) {
    let now_ts = chrono::Utc::now().timestamp();
    if let Ok(woken) = cache.wake_due_snoozed(now_ts) {
        for (message_id, account_id, _subject) in woken {
            let _ = evt_tx.send(WorkerEvent::MessageSnoozed {
                account_id,
                message_id,
                until_ts: 0,
            });
        }
    }
}

/// Spawn or replace a background IMAP IDLE push watcher for `account`.
/// Supports both standard Password IMAP (163, 126, QQ, Gmail, iCloud, Exmail, Aliyun, etc.)
/// and Microsoft Outlook OAuth2 accounts (via `XOAUTH2` on `outlook.office365.com:993`).
/// For Microsoft OAuth2 accounts whose older refresh token has not yet been granted
/// `IMAP.AccessAsUser.All`, automatically falls back to a lightweight 20-second Graph inbox
/// unread/total check so push-like responsiveness works immediately without re-login.
fn spawn_account_idle_watcher(
    watchers: &mut IdleWatchers,
    account: AccountConfig,
    req_tx: mpsc::Sender<MailRequest>,
) {
    if let Some(old) = watchers.remove(&account.id) {
        old.abort();
    }
    let account_id = account.id;
    let handle = tokio::spawn(async move {
        run_account_push_watcher(account, req_tx).await;
    });
    watchers.insert(account_id, handle);
}

fn stop_account_idle_watcher(watchers: &mut IdleWatchers, account_id: u32) {
    if let Some(old) = watchers.remove(&account_id) {
        old.abort();
    }
}

/// 账号是否在统一视图范围内：已启用且加入统一收件箱。任务四起统一视图的
/// 列表、未读数、历史可用性与后台索引都以此为账号范围。
fn is_unified_scope_account(acc: &AccountConfig) -> bool {
    acc.enabled && acc.in_unified
}

fn unified_account_ids(data_dir: &std::path::Path) -> Vec<u32> {
    config::load_accounts_in(data_dir)
        .unwrap_or_default()
        .into_iter()
        .filter(is_unified_scope_account)
        .map(|acc| acc.id)
        .collect()
}

/// Microsoft OAuth 返回身份的归属判定结果（任务二：先用邮箱核对账号身份再换绑）。
#[derive(Debug, PartialEq, Eq)]
enum OAuthAccountMatch {
    /// 返回邮箱与账号身份一致：绑定到该账号 ID，更新令牌与配置。
    Bind(u32),
    /// 返回邮箱与正在编辑的原账号不一致：保留原账号、凭据与缓存完全不动。
    Rejected,
}

/// 判定 OAuth 授权返回的邮箱应绑定到哪个账号：
/// - 重新授权（`editing_account_id` 有值）：仅当返回邮箱与原账号一致（忽略大小写）
///   时更新原账号，否则拒绝换绑（无论该邮箱是否命中其他已有账号）。
/// - 添加账号流程（`editing_account_id` 为空）：返回邮箱命中已有账号时视为重复
///   添加并报错（不静默覆盖已有账号），否则分配新账号 ID。
fn resolve_oauth_account_match(
    editing_account_id: Option<u32>,
    existing: &[AccountConfig],
    returned_email: &str,
) -> Result<OAuthAccountMatch, String> {
    match editing_account_id {
        Some(edit_id) => {
            let same_as_editing = existing
                .iter()
                .find(|a| a.id == edit_id)
                .is_some_and(|a| a.email.eq_ignore_ascii_case(returned_email));
            if same_as_editing {
                Ok(OAuthAccountMatch::Bind(edit_id))
            } else {
                Ok(OAuthAccountMatch::Rejected)
            }
        }
        None => {
            if existing
                .iter()
                .any(|a| a.email.eq_ignore_ascii_case(returned_email))
            {
                return Err(format!("Account {returned_email} already exists"));
            }
            Ok(OAuthAccountMatch::Bind(config::next_account_id(existing)))
        }
    }
}

fn refresh_account_watcher(
    watchers: &mut IdleWatchers,
    data_dir: &std::path::Path,
    account: &AccountConfig,
    req_tx: &mpsc::Sender<MailRequest>,
) {
    let global_push = config::load_preferences_in(data_dir)
        .map(|p| p.push_enabled)
        .unwrap_or(true);
    if account.enabled && account.effective_push(global_push) {
        spawn_account_idle_watcher(watchers, account.clone(), req_tx.clone());
    } else {
        stop_account_idle_watcher(watchers, account.id);
    }
}

async fn run_account_push_watcher(account: AccountConfig, req_tx: mpsc::Sender<MailRequest>) {
    // Brief initial delay so startup sync finishes first without competing for connections.
    tokio::time::sleep(Duration::from_secs(5)).await;

    let mut backoff = Duration::from_secs(15);
    let mut cached_imap_token: Option<(String, Instant)> = None;

    loop {
        match account.auth_backend {
            AuthBackend::Password => {
                let Ok(sec) = secrets::load_account_secrets(account.id) else {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    continue;
                };
                match imap_client::connect_imap_password(&account, &sec.imap_password).await {
                    Ok(session) => {
                        backoff = Duration::from_secs(15);
                        let tx = req_tx.clone();
                        let aid = account.id;
                        let _ = imap_client::run_idle_inbox_session(session, "INBOX", move || {
                            let _ = tx.send(MailRequest::SyncAccount { account_id: aid });
                        })
                        .await;
                    }
                    Err(_) => {
                        tokio::time::sleep(backoff).await;
                        backoff = (backoff * 2).min(Duration::from_secs(300));
                        continue;
                    }
                }
            }
            AuthBackend::MicrosoftGraphOAuth2 => {
                // 1. Try to obtain an Exchange Online IMAP token from the saved refresh token
                //    so we can use genuine IMAP IDLE push on outlook.office365.com:993.
                let imap_token = match cached_imap_token.as_ref() {
                    Some((tok, exp)) if Instant::now() < *exp => Some(tok.clone()),
                    _ => {
                        let Ok(rt) = secrets::load_oauth_refresh_token(account.id) else {
                            tokio::time::sleep(Duration::from_secs(60)).await;
                            continue;
                        };
                        let cid = account.oauth_client_id.clone();
                        let refreshed = tokio::task::spawn_blocking(move || {
                            oauth::refresh_microsoft_imap_token(&cid, &rt)
                        })
                        .await
                        .ok()
                        .and_then(|r| r.ok());

                        if let Some(refreshed) = refreshed {
                            if let Some(new_rt) = refreshed.refresh_token {
                                let _ = secrets::save_oauth_refresh_token(account.id, new_rt);
                            }
                            let exp = Instant::now()
                                + Duration::from_secs(
                                    refreshed.expires_in_secs.saturating_sub(120),
                                );
                            cached_imap_token = Some((refreshed.access_token.clone(), exp));
                            Some(refreshed.access_token)
                        } else {
                            None
                        }
                    }
                };

                if let Some(token) = imap_token {
                    match imap_client::connect_imap_xoauth2(&account, &token).await {
                        Ok(session) => {
                            backoff = Duration::from_secs(15);
                            let tx = req_tx.clone();
                            let aid = account.id;
                            let _ =
                                imap_client::run_idle_inbox_session(session, "INBOX", move || {
                                    let _ = tx.send(MailRequest::SyncAccount { account_id: aid });
                                })
                                .await;
                            tokio::time::sleep(Duration::from_secs(5)).await;
                            continue;
                        }
                        Err(_) => {
                            cached_imap_token = None;
                        }
                    }
                }

                // 2. Fallback when an older refresh token has not yet been re-authorized with
                //    IMAP.AccessAsUser.All (or tenant blocks IMAP): lightweight 20-second Graph
                //    inbox unread/total pulse check; only triggers full sync when counts change.
                run_graph_fast_poll_fallback(&account, &req_tx).await;
            }
        }

        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(300));
    }
}

async fn run_graph_fast_poll_fallback(account: &AccountConfig, req_tx: &mpsc::Sender<MailRequest>) {
    let mut cached_graph_token: Option<(String, Instant)> = None;
    let mut last_inbox_sig: Option<(u32, usize)> = None;

    // Run for 10 minutes before re-checking whether the user re-authorized IMAP IDLE.
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_secs(20)).await;

        let token = match cached_graph_token.as_ref() {
            Some((tok, exp)) if Instant::now() < *exp => tok.clone(),
            _ => {
                let Ok(rt) = secrets::load_oauth_refresh_token(account.id) else {
                    return;
                };
                let cid = account.oauth_client_id.clone();
                let Ok(Ok(refreshed)) =
                    tokio::task::spawn_blocking(move || oauth::refresh_microsoft_token(&cid, &rt))
                        .await
                else {
                    return;
                };
                if let Some(new_rt) = refreshed.refresh_token {
                    let _ = secrets::save_oauth_refresh_token(account.id, new_rt);
                }
                let exp = Instant::now()
                    + Duration::from_secs(refreshed.expires_in_secs.saturating_sub(120));
                cached_graph_token = Some((refreshed.access_token.clone(), exp));
                refreshed.access_token
            }
        };

        let tok_for_call = token.clone();
        let Ok(Ok(folders)) =
            tokio::task::spawn_blocking(move || graph_client::fetch_folders(&tok_for_call)).await
        else {
            cached_graph_token = None;
            continue;
        };

        if let Some((inbox, _)) = folders
            .iter()
            .find(|(_, is_inbox)| *is_inbox)
            .or_else(|| folders.first())
        {
            let sig = (inbox.unread_count, inbox.total_count);
            if let Some(prev) = last_inbox_sig {
                if sig != prev {
                    let _ = req_tx.send(MailRequest::SyncAccount {
                        account_id: account.id,
                    });
                }
            }
            last_inbox_sig = Some(sig);
        }
    }
}

async fn handle_request(
    data_dir: &PathBuf,
    cache: &mut CacheStore,
    tokens: &SharedTokenCache,
    async_cache: &SharedAsyncCache,
    in_flight_bodies: &Arc<std::sync::Mutex<HashSet<u32>>>,
    watchers: &mut IdleWatchers,
    req_tx: &mpsc::Sender<MailRequest>,
    req: MailRequest,
    evt_tx: &EventSender,
) {
    match req {
        MailRequest::VerifyAccount {
            config,
            imap_password,
            smtp_password,
        } => {
            if config.auth_backend != AuthBackend::Password {
                let _ = evt_tx.send(WorkerEvent::AccountVerified {
                    ok: true,
                    message: "当前账号使用 OAuth2 浏览器授权，无需测试 IMAP/SMTP 密码".to_string(),
                });
                return;
            }
            let stored = if imap_password.is_empty()
                || (config.smtp_separate_credentials
                    && smtp_password.as_deref().is_none_or(str::is_empty))
            {
                secrets::load_account_secrets(config.id).ok()
            } else {
                None
            };
            let effective_imap = if imap_password.is_empty() {
                stored
                    .as_ref()
                    .map(|s| s.imap_password.clone())
                    .unwrap_or_default()
            } else {
                imap_password
            };
            if effective_imap.is_empty() {
                let _ = evt_tx.send(WorkerEvent::AccountVerified {
                    ok: false,
                    message: "请输入密码或客户端授权码".to_string(),
                });
                return;
            }
            let effective_smtp = if config.smtp_separate_credentials {
                let raw = smtp_password
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .or_else(|| {
                        stored
                            .as_ref()
                            .and_then(|s| s.smtp_password.clone())
                            .filter(|s| !s.is_empty())
                    });
                let Some(pwd) = raw else {
                    let _ = evt_tx.send(WorkerEvent::AccountVerified {
                        ok: false,
                        message: "启用独立 SMTP 凭据时请输入 SMTP 密码或授权码".to_string(),
                    });
                    return;
                };
                pwd
            } else {
                effective_imap.clone()
            };
            let imap_res = imap_client::verify_login(&config, &effective_imap).await;
            let smtp_res = smtp_client::verify_login(&config, &effective_smtp).await;
            match (imap_res, smtp_res) {
                (Ok(()), Ok(())) => {
                    let _ = evt_tx.send(WorkerEvent::AccountVerified {
                        ok: true,
                        message: "IMAP 与 SMTP 验证均通过".to_string(),
                    });
                }
                (Err(e), _) | (_, Err(e)) => {
                    let _ = evt_tx.send(WorkerEvent::AccountVerified {
                        ok: false,
                        message: e,
                    });
                }
            }
        }

        MailRequest::SaveAndSyncAccount {
            config,
            imap_password,
            smtp_password,
        } => {
            if config.auth_backend == AuthBackend::Password {
                let stored = if imap_password.is_empty()
                    || (config.smtp_separate_credentials
                        && smtp_password.as_deref().is_none_or(str::is_empty))
                {
                    secrets::load_account_secrets(config.id).ok()
                } else {
                    None
                };
                let effective_imap = if imap_password.is_empty() {
                    stored
                        .as_ref()
                        .map(|s| s.imap_password.clone())
                        .unwrap_or_default()
                } else {
                    imap_password
                };
                if effective_imap.is_empty() {
                    let msg = "请输入密码或客户端授权码".to_string();
                    let _ = evt_tx.send(WorkerEvent::AccountVerified {
                        ok: false,
                        message: msg.clone(),
                    });
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: Some(config.id),
                        message: msg,
                    });
                    return;
                }
                let effective_smtp = if config.smtp_separate_credentials {
                    let raw = smtp_password
                        .as_deref()
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .or_else(|| {
                            stored
                                .as_ref()
                                .and_then(|s| s.smtp_password.clone())
                                .filter(|s| !s.is_empty())
                        });
                    let Some(pwd) = raw else {
                        let msg = "启用独立 SMTP 凭据时请输入 SMTP 密码或授权码".to_string();
                        let _ = evt_tx.send(WorkerEvent::AccountVerified {
                            ok: false,
                            message: msg.clone(),
                        });
                        let _ = evt_tx.send(WorkerEvent::Error {
                            account_id: Some(config.id),
                            message: msg,
                        });
                        return;
                    };
                    Some(pwd)
                } else {
                    None
                };
                if let Err(e) = secrets::save_account_secrets(
                    config.id,
                    AccountSecrets {
                        imap_password: effective_imap,
                        smtp_password: effective_smtp,
                    },
                ) {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: Some(config.id),
                        message: e,
                    });
                    return;
                }
            }
            if let Err(e) = config::upsert_account_in(data_dir, config.clone()) {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: Some(config.id),
                    message: e,
                });
                return;
            }
            let account_id = config.id;
            let account_enabled = config.enabled;
            refresh_account_watcher(watchers, data_dir, &config, req_tx);
            let _ = evt_tx.send(WorkerEvent::AccountSaved { account: config });
            if account_enabled {
                sync_account_inner(data_dir, cache, tokens, account_id, evt_tx).await;
            }
            let page_size = pref_page_size(data_dir);
            let allowed = unified_account_ids(data_dir);
            if let Ok(page) = cache.list_unified_inbox_for_accounts(
                &allowed,
                "",
                QuickFilterSet::default(),
                MessageSort::DateNewest,
                0,
                page_size,
            ) {
                let _ = evt_tx.send(WorkerEvent::MessagesPageLoaded(page));
            }
        }

        MailRequest::StartMicrosoftOAuth {
            custom_client_id,
            login_hint_email,
            editing_account_id,
        } => {
            let _ = evt_tx.send(WorkerEvent::Status {
                account_id: None,
                text: "已打开浏览器，请在微软页面完成 Outlook 授权...".to_string(),
            });

            let sign_in = tokio::task::spawn_blocking(move || {
                oauth::run_microsoft_oauth_login(&custom_client_id, login_hint_email.as_deref())
            })
            .await;

            let result = match sign_in {
                Ok(Ok(res)) => res,
                Ok(Err(e)) => {
                    let _ = evt_tx.send(WorkerEvent::AccountVerified {
                        ok: false,
                        message: e.clone(),
                    });
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: None,
                        message: e,
                    });
                    return;
                }
                Err(e) => {
                    let msg = format!("OAuth2 登录任务异常: {e}");
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: None,
                        message: msg,
                    });
                    return;
                }
            };

            let existing = config::load_accounts_in(data_dir).unwrap_or_default();
            // 先用返回的邮箱核对账号身份：与编辑中账号一致才更新；选择了另一个
            // 邮箱时保留原账号、凭据与缓存完全不动，提示用户重新授权或走添加流程。
            // 新增流程授权到已存在的邮箱时直接报错，不静默覆盖已有账号。
            let account_id =
                match resolve_oauth_account_match(editing_account_id, &existing, &result.email) {
                    Ok(OAuthAccountMatch::Bind(id)) => id,
                    Ok(OAuthAccountMatch::Rejected) => {
                        let _ = evt_tx.send(WorkerEvent::OAuthRebindRejected {
                            account_id: editing_account_id,
                            returned_email: result.email.clone(),
                        });
                        return;
                    }
                    Err(message) => {
                        let _ = evt_tx.send(WorkerEvent::AccountVerified {
                            ok: false,
                            message: message.clone(),
                        });
                        let _ = evt_tx.send(WorkerEvent::Error {
                            account_id: None,
                            message,
                        });
                        return;
                    }
                };

            // 编辑已有账号重新授权：只把新令牌写入 Keyring 与内存令牌缓存，
            // 账号配置、缓存与同步完全不动，由 UI 保留编辑表单并提示成功。
            if editing_account_id.is_some() {
                if let Err(e) = secrets::save_oauth_refresh_token(account_id, result.refresh_token)
                {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: Some(account_id),
                        message: e,
                    });
                    return;
                }
                let expires_at = Instant::now()
                    + Duration::from_secs(result.expires_in_secs.saturating_sub(120));
                tokens
                    .insert(account_id, result.access_token, expires_at)
                    .await;
                let _ = evt_tx.send(WorkerEvent::OAuthReauthorized {
                    account_id,
                    email: result.email,
                });
                return;
            }

            let prev = existing.iter().find(|a| a.id == account_id);

            let account = AccountConfig {
                id: account_id,
                auth_backend: AuthBackend::MicrosoftGraphOAuth2,
                oauth_client_id: result.client_id,
                display_name: prev
                    .map(|p| p.display_name.clone())
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or(result.display_name),
                email: result.email.clone(),
                label: prev.and_then(|p| p.label.clone()),
                enabled: prev.map(|p| p.enabled).unwrap_or(true),
                color: prev.and_then(|p| p.color.clone()),
                emoji: prev.and_then(|p| p.emoji.clone()),
                avatar: prev.and_then(|p| p.avatar.clone()),
                in_unified: prev.map(|p| p.in_unified).unwrap_or(true),
                push_override: prev.and_then(|p| p.push_override),
                imap_host: "outlook.office365.com".to_string(),
                imap_port: 993,
                imap_security: config::SecurityMode::Tls,
                imap_username: result.email.clone(),
                smtp_host: "smtp.office365.com".to_string(),
                smtp_port: 587,
                smtp_security: config::SecurityMode::StartTls,
                smtp_username: result.email,
                smtp_separate_credentials: false,
                signature: prev.map(|p| p.signature.clone()).unwrap_or_default(),
                quota_used_bytes: prev.and_then(|p| p.quota_used_bytes),
                quota_total_bytes: prev.and_then(|p| p.quota_total_bytes),
                aliases: prev.map(|p| p.aliases.clone()).unwrap_or_default(),
                signatures: prev.map(|p| p.signatures.clone()).unwrap_or_default(),
                default_new_signature: prev.and_then(|p| p.default_new_signature),
                default_reply_signature: prev.and_then(|p| p.default_reply_signature),
                special_folders: prev.map(|p| p.special_folders.clone()).unwrap_or_else(|| {
                    config::SpecialFoldersConfig {
                        server_saves_sent: true,
                        ..Default::default()
                    }
                }),
                sync_interval_minutes: prev.and_then(|p| p.sync_interval_minutes),
                download_window_days: prev.map(|p| p.download_window_days).unwrap_or(0),
                auto_empty_trash_days: prev.map(|p| p.auto_empty_trash_days).unwrap_or(0),
                auto_empty_junk_days: prev.map(|p| p.auto_empty_junk_days).unwrap_or(0),
                hidden_folders: prev.map(|p| p.hidden_folders.clone()).unwrap_or_default(),
            };

            if let Err(e) = secrets::save_oauth_refresh_token(account_id, result.refresh_token) {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: Some(account_id),
                    message: e,
                });
                return;
            }
            if let Err(e) = config::upsert_account_in(data_dir, account.clone()) {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: Some(account_id),
                    message: e,
                });
                return;
            }

            let expires_at =
                Instant::now() + Duration::from_secs(result.expires_in_secs.saturating_sub(120));
            tokens
                .insert(account_id, result.access_token, expires_at)
                .await;

            let _ = evt_tx.send(WorkerEvent::AccountVerified {
                ok: true,
                message: "Outlook OAuth2 授权成功，正在同步邮件...".to_string(),
            });
            let account_enabled = account.enabled;
            refresh_account_watcher(watchers, data_dir, &account, req_tx);
            let _ = evt_tx.send(WorkerEvent::AccountSaved { account });
            if account_enabled {
                sync_account_inner(data_dir, cache, tokens, account_id, evt_tx).await;
            }
            let page_size = pref_page_size(data_dir);
            let allowed = unified_account_ids(data_dir);
            if let Ok(page) = cache.list_unified_inbox_for_accounts(
                &allowed,
                "",
                QuickFilterSet::default(),
                MessageSort::DateNewest,
                0,
                page_size,
            ) {
                let _ = evt_tx.send(WorkerEvent::MessagesPageLoaded(page));
            }
        }

        MailRequest::DeleteAccount { account_id } => {
            stop_account_idle_watcher(watchers, account_id);
            tokens.remove(account_id).await;
            let _ = secrets::delete_account_secrets(account_id);
            let _ = config::delete_account_in(data_dir, account_id);
            let _ = cache.delete_account_data(account_id);
            let _ = evt_tx.send(WorkerEvent::AccountDeleted { account_id });
            if let Ok(drafts) = cache.list_drafts() {
                let _ = evt_tx.send(WorkerEvent::DraftsUpdated { drafts });
            }
            if let Ok(tags) = cache.list_tags() {
                let _ = evt_tx.send(WorkerEvent::TagsLoaded { tags });
            }
            let page_size = pref_page_size(data_dir);
            let allowed = unified_account_ids(data_dir);
            if let Ok(page) = cache.list_unified_inbox_for_accounts(
                &allowed,
                "",
                QuickFilterSet::default(),
                MessageSort::DateNewest,
                0,
                page_size,
            ) {
                let _ = evt_tx.send(WorkerEvent::MessagesPageLoaded(page));
            }
        }

        MailRequest::LoadAllCached => {
            let accounts = config::load_accounts_in(data_dir).unwrap_or_default();
            for acc in &accounts {
                if let Ok(folders) = cache.list_folders(acc.id) {
                    let _ = evt_tx.send(WorkerEvent::FoldersLoaded {
                        account_id: acc.id,
                        folders,
                    });
                }
                if let Ok(items) = cache.list_outbox(acc.id) {
                    let _ = evt_tx.send(WorkerEvent::OutboxUpdated {
                        account_id: acc.id,
                        items,
                    });
                }
            }
            if let Ok(drafts) = cache.list_drafts() {
                let _ = evt_tx.send(WorkerEvent::DraftsUpdated { drafts });
            }
            if let Ok(tags) = cache.list_tags() {
                let _ = evt_tx.send(WorkerEvent::TagsLoaded { tags });
            }
            let page_size = pref_page_size(data_dir);
            let allowed = unified_account_ids(data_dir);
            if let Ok(page) = cache.list_unified_inbox_for_accounts(
                &allowed,
                "",
                QuickFilterSet::default(),
                MessageSort::DateNewest,
                0,
                page_size,
            ) {
                let _ = evt_tx.send(WorkerEvent::MessagesPageLoaded(page));
            }
        }

        // 后台任务（历史索引 / 账号与文件夹同步）在 worker_loop 中改派到独立
        // 任务执行，不占用本队列；这里保留空分支以满足穷尽匹配。
        MailRequest::BackfillHistoryBatch
        | MailRequest::SyncAllAccounts
        | MailRequest::SyncAccount { .. }
        | MailRequest::SyncFolder { .. } => {}

        MailRequest::LoadUnifiedInbox {
            query,
            filter,
            quick_filters,
            sort,
            offset,
            limit,
        } => {
            let effective = merge_filter_options(filter, quick_filters);
            let allowed = unified_account_ids(data_dir);
            match cache
                .list_unified_inbox_for_accounts(&allowed, &query, effective, sort, offset, limit)
            {
                Ok(page) => {
                    let _ = evt_tx.send(WorkerEvent::MessagesPageLoaded(page));
                }
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: None,
                        message: e,
                    });
                }
            }
        }

        MailRequest::LoadMessagesPage {
            folder_id,
            query,
            filter,
            quick_filters,
            sort,
            offset,
            limit,
        } => {
            let effective = merge_filter_options(filter, quick_filters);
            match cache
                .list_messages_page_with_options(folder_id, &query, effective, sort, offset, limit)
            {
                Ok(page) => {
                    let _ = evt_tx.send(WorkerEvent::MessagesPageLoaded(page));
                }
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: None,
                        message: e,
                    });
                }
            }
        }

        MailRequest::SearchAllFolders {
            query,
            quick_filters,
            sort,
            offset,
            limit,
        } => {
            // 跨文件夹搜索只覆盖已启用账号（对齐 Linux `folder_select`），
            // 禁用账号的本地缓存邮件不参与全局搜索。
            let enabled_account_ids: Vec<u32> = config::load_accounts_in(data_dir)
                .unwrap_or_default()
                .into_iter()
                .filter(|acc| acc.enabled)
                .map(|acc| acc.id)
                .collect();
            match cache.search_all_folders(
                &enabled_account_ids,
                &query,
                quick_filters,
                sort,
                offset,
                limit,
            ) {
                Ok(page) => {
                    let _ = evt_tx.send(WorkerEvent::AllFoldersSearchLoaded(page));
                }
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: None,
                        message: e,
                    });
                }
            }
        }

        MailRequest::SearchServer { query, account_id } => {
            search_server_inner(data_dir, cache, tokens, &query, account_id, evt_tx).await;
        }

        MailRequest::LoadMessageBody {
            account_id,
            message_id,
        } => {
            let newest_first = pref_thread_newest_first(data_dir);
            let Ok(Some((detail, uid, body_loaded))) =
                cache.get_message_detail_ordered(message_id, newest_first)
            else {
                return;
            };
            if body_loaded {
                let _ = evt_tx.send(WorkerEvent::MessageBodyLoaded(detail));
                return;
            }
            let Ok(account) = load_account_config(data_dir, account_id) else {
                let _ = evt_tx.send(WorkerEvent::MessageBodyLoaded(detail));
                return;
            };

            let (remote_id_opt, folder_path_opt) = match account.auth_backend {
                AuthBackend::MicrosoftGraphOAuth2 => {
                    let Some(remote_id) = cache.get_message_remote_id(message_id).ok().flatten()
                    else {
                        let _ = evt_tx.send(WorkerEvent::MessageBodyLoaded(detail));
                        return;
                    };
                    (Some(remote_id), None)
                }
                AuthBackend::Password => {
                    let Ok(Some((folder_path, _))) = cache.folder_meta(detail.folder_id) else {
                        return;
                    };
                    (None, Some(folder_path))
                }
            };

            {
                let Ok(mut set) = in_flight_bodies.lock() else {
                    return;
                };
                if !set.insert(message_id) {
                    return;
                }
            }

            let _ = evt_tx.send(WorkerEvent::Status {
                account_id: Some(account_id),
                text: format!("[{}] 正在获取邮件正文...", account.email),
            });

            let data_dir_cl = data_dir.clone();
            let tokens_cl = tokens.clone();
            let async_cache_cl = Arc::clone(async_cache);
            let in_flight_cl = Arc::clone(in_flight_bodies);
            let evt_tx_cl = evt_tx.clone();
            tokio::spawn(async move {
                let _guard = InFlightBodyGuard {
                    message_id,
                    set: in_flight_cl,
                };
                let fetch_res = match account.auth_backend {
                    AuthBackend::MicrosoftGraphOAuth2 => {
                        let remote_id = remote_id_opt.unwrap_or_default();
                        match tokens_cl.ensure_graph_token(&account).await {
                            Ok(token) => tokio::task::spawn_blocking(move || {
                                graph_client::fetch_message_body(&token, &remote_id)
                            })
                            .await
                            .unwrap_or_else(|e| Err(format!("Graph 任务异常: {e}"))),
                            Err(e) => Err(e),
                        }
                    }
                    AuthBackend::Password => {
                        let folder_path = folder_path_opt.unwrap_or_default();
                        match secrets::load_account_secrets(account_id) {
                            Ok(sec) => {
                                imap_client::fetch_message_body(
                                    &account,
                                    &sec.imap_password,
                                    &folder_path,
                                    uid,
                                )
                                .await
                            }
                            Err(e) => Err(e),
                        }
                    }
                };

                match fetch_res {
                    Ok(fetched) => {
                        let db_res = tokio::task::spawn_blocking(move || {
                            with_shared_cache(&async_cache_cl, &data_dir_cl, |c| {
                                c.store_message_body_with_html(
                                    message_id,
                                    &fetched.body_text,
                                    fetched.body_html.as_deref(),
                                    &fetched.attachments,
                                    fetched.unsubscribe_url.as_deref(),
                                    fetched.unsubscribe_one_click,
                                    fetched.auth_results.as_deref(),
                                )?;
                                c.get_message_detail_ordered(message_id, newest_first)
                            })
                        })
                        .await
                        .unwrap_or_else(|e| Err(format!("写入正文缓存任务异常: {e}")));

                        match db_res {
                            Ok(Some((updated, _, _))) => {
                                let _ = evt_tx_cl.send(WorkerEvent::MessageBodyLoaded(updated));
                            }
                            Ok(None) => {}
                            Err(e) => {
                                let _ = evt_tx_cl.send(WorkerEvent::Error {
                                    account_id: Some(account_id),
                                    message: e,
                                });
                            }
                        }
                    }
                    Err(e) => {
                        let _ = evt_tx_cl.send(WorkerEvent::Error {
                            account_id: Some(account_id),
                            message: format!("[{}] {e}", account.email),
                        });
                    }
                }
            });
        }

        MailRequest::FetchRawMessage {
            account_id,
            message_id,
            uid,
            folder_path,
        } => {
            let Ok(account) = load_account_config(data_dir, account_id) else {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: Some(account_id),
                    message: format!("账号 #{account_id} 未找到配置"),
                });
                return;
            };
            let _ = evt_tx.send(WorkerEvent::Status {
                account_id: Some(account_id),
                text: format!("[{}] 正在获取邮件原文...", account.email),
            });
            let res = match account.auth_backend {
                AuthBackend::MicrosoftGraphOAuth2 => {
                    // Graph 路径：按本地缓存的远端字符串 ID 取完整 MIME（/$value）。
                    match cache.get_message_remote_id(message_id) {
                        Ok(Some(remote_id)) => match ensure_graph_token(tokens, &account).await {
                            Ok(token) => tokio::task::spawn_blocking(move || {
                                graph_client::fetch_raw_message(&token, &remote_id)
                            })
                            .await
                            .unwrap_or_else(|e| Err(format!("Graph 任务异常: {e}"))),
                            Err(e) => Err(e),
                        },
                        Ok(None) => Err("本地缓存中缺少该邮件的远端 ID，无法导出原文".to_string()),
                        Err(e) => Err(e),
                    }
                }
                AuthBackend::Password => match secrets::load_account_secrets(account_id) {
                    Ok(sec) => {
                        imap_client::fetch_raw_message(
                            &account,
                            &sec.imap_password,
                            &folder_path,
                            uid,
                        )
                        .await
                    }
                    Err(e) => Err(e),
                },
            };
            match res {
                Ok(bytes) => {
                    let _ = evt_tx.send(WorkerEvent::RawMessageFetched { message_id, bytes });
                }
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: Some(account_id),
                        message: format!("[{}] {e}", account.email),
                    });
                }
            }
        }

        MailRequest::SetSeen {
            account_id,
            folder_path,
            message_id,
            uid,
            seen,
        } => {
            let _ = cache.set_message_seen(message_id, seen);
            if let Ok(folders) = cache.list_folders(account_id) {
                let _ = evt_tx.send(WorkerEvent::FoldersLoaded {
                    account_id,
                    folders,
                });
            }
            if let Ok(account) = load_account_config(data_dir, account_id) {
                let remote_id_opt = if account.auth_backend == AuthBackend::MicrosoftGraphOAuth2 {
                    cache.get_message_remote_id(message_id).ok().flatten()
                } else {
                    None
                };
                let tokens_cl = tokens.clone();
                let evt_tx_cl = evt_tx.clone();
                tokio::spawn(async move {
                    let res = match account.auth_backend {
                        AuthBackend::MicrosoftGraphOAuth2 => {
                            if let Some(remote_id) = remote_id_opt {
                                match tokens_cl.ensure_graph_token(&account).await {
                                    Ok(token) => tokio::task::spawn_blocking(move || {
                                        graph_client::set_seen(&token, &remote_id, seen)
                                    })
                                    .await
                                    .unwrap_or_else(|e| Err(format!("Graph 任务异常: {e}"))),
                                    Err(e) => Err(e),
                                }
                            } else {
                                Ok(())
                            }
                        }
                        AuthBackend::Password => match secrets::load_account_secrets(account_id) {
                            Ok(sec) => {
                                imap_client::set_flag(
                                    &account,
                                    &sec.imap_password,
                                    &folder_path,
                                    uid,
                                    "\\Seen",
                                    seen,
                                )
                                .await
                            }
                            Err(e) => Err(e),
                        },
                    };
                    if let Err(e) = res {
                        let _ = evt_tx_cl.send(WorkerEvent::Error {
                            account_id: Some(account_id),
                            message: format!("[{}] {e}", account.email),
                        });
                    }
                });
            }
        }

        MailRequest::SetFlagged {
            account_id,
            folder_path,
            message_id,
            uid,
            flagged,
        } => {
            let _ = cache.set_message_flagged(message_id, flagged);
            let newest_first = pref_thread_newest_first(data_dir);
            if let Ok(Some((updated, _, _))) =
                cache.get_message_detail_ordered(message_id, newest_first)
            {
                let _ = evt_tx.send(WorkerEvent::MessageBodyLoaded(updated));
            }
            if let Ok(account) = load_account_config(data_dir, account_id) {
                let remote_id_opt = if account.auth_backend == AuthBackend::MicrosoftGraphOAuth2 {
                    cache.get_message_remote_id(message_id).ok().flatten()
                } else {
                    None
                };
                let tokens_cl = tokens.clone();
                let evt_tx_cl = evt_tx.clone();
                tokio::spawn(async move {
                    let res = match account.auth_backend {
                        AuthBackend::MicrosoftGraphOAuth2 => {
                            if let Some(remote_id) = remote_id_opt {
                                match tokens_cl.ensure_graph_token(&account).await {
                                    Ok(token) => tokio::task::spawn_blocking(move || {
                                        graph_client::set_flagged(&token, &remote_id, flagged)
                                    })
                                    .await
                                    .unwrap_or_else(|e| Err(format!("Graph 任务异常: {e}"))),
                                    Err(e) => Err(e),
                                }
                            } else {
                                Ok(())
                            }
                        }
                        AuthBackend::Password => match secrets::load_account_secrets(account_id) {
                            Ok(sec) => {
                                imap_client::set_flag(
                                    &account,
                                    &sec.imap_password,
                                    &folder_path,
                                    uid,
                                    "\\Flagged",
                                    flagged,
                                )
                                .await
                            }
                            Err(e) => Err(e),
                        },
                    };
                    if let Err(e) = res {
                        let _ = evt_tx_cl.send(WorkerEvent::Error {
                            account_id: Some(account_id),
                            message: format!("[{}] {e}", account.email),
                        });
                    }
                });
            }
        }

        MailRequest::MarkFolderAllRead {
            account_id,
            folder_id,
            folder_path,
        } => {
            // 先落本地：未读归零并即时回发文件夹计数，再同步服务器（失败走 Error 事件）。
            let _ = cache.set_folder_all_seen(folder_id);
            if let Ok(folders) = cache.list_folders(account_id) {
                let _ = evt_tx.send(WorkerEvent::FoldersLoaded {
                    account_id,
                    folders,
                });
            }
            let Ok(account) = load_account_config(data_dir, account_id) else {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: Some(account_id),
                    message: format!("账号 #{account_id} 未找到配置"),
                });
                return;
            };
            let res = match account.auth_backend {
                AuthBackend::MicrosoftGraphOAuth2 => {
                    // graph_client 无批量接口：对本文件夹缓存内的未读邮件逐封置已读。
                    let unread = match cache.folder_unread_remote_ids(folder_id) {
                        Ok(list) => list,
                        Err(e) => {
                            let _ = evt_tx.send(WorkerEvent::Error {
                                account_id: Some(account_id),
                                message: e,
                            });
                            return;
                        }
                    };
                    match ensure_graph_token(tokens, &account).await {
                        Ok(token) => {
                            let mut last = Ok(());
                            for (_, remote_id) in unread {
                                let token = token.clone();
                                last = tokio::task::spawn_blocking(move || {
                                    graph_client::set_seen(&token, &remote_id, true)
                                })
                                .await
                                .unwrap_or_else(|e| Err(format!("Graph 任务异常: {e}")));
                                if last.is_err() {
                                    break;
                                }
                            }
                            last
                        }
                        Err(e) => Err(e),
                    }
                }
                AuthBackend::Password => match secrets::load_account_secrets(account_id) {
                    Ok(sec) => {
                        imap_client::mark_folder_all_read(
                            &account,
                            &sec.imap_password,
                            &folder_path,
                        )
                        .await
                    }
                    Err(e) => Err(e),
                },
            };
            match res {
                Ok(()) => {
                    let _ = evt_tx.send(WorkerEvent::Status {
                        account_id: Some(account_id),
                        text: format!("[{}] 已将文件夹全部标为已读", account.email),
                    });
                }
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: Some(account_id),
                        message: format!("[{}] {e}", account.email),
                    });
                }
            }
        }

        MailRequest::EmptyFolder {
            account_id,
            folder_id,
            folder_path,
        } => {
            let Ok(account) = load_account_config(data_dir, account_id) else {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: Some(account_id),
                    message: format!("账号 #{account_id} 未找到配置"),
                });
                return;
            };
            // Graph（Outlook）账号：graph_client 无批量删除 / 清空能力，降级不支持。
            if account.auth_backend == AuthBackend::MicrosoftGraphOAuth2 {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: Some(account_id),
                    message: format!(
                        "[{}] Graph (Outlook) 账号暂不支持清空文件夹，请在网页端操作",
                        account.email
                    ),
                });
                return;
            }
            let res = match secrets::load_account_secrets(account_id) {
                Ok(sec) => {
                    imap_client::empty_folder(&account, &sec.imap_password, &folder_path).await
                }
                Err(e) => Err(e),
            };
            match res {
                Ok(()) => {
                    // 服务器已 EXPUNGE：同步清空该文件夹本地缓存并刷新计数。
                    let _ = cache.clear_folder(folder_id);
                    if let Ok(folders) = cache.list_folders(account_id) {
                        let _ = evt_tx.send(WorkerEvent::FoldersLoaded {
                            account_id,
                            folders,
                        });
                    }
                    let _ = evt_tx.send(WorkerEvent::Status {
                        account_id: Some(account_id),
                        text: format!("[{}] 文件夹已清空", account.email),
                    });
                }
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: Some(account_id),
                        message: format!("[{}] {e}", account.email),
                    });
                }
            }
        }

        // F-4 草稿自动保存：静默变体，不回发状态行（避免打字期间状态栏抖动），
        // 仅回发草稿列表与 `DraftSaved`（携带本地草稿 ID 供写信页绑定）。
        MailRequest::AutosaveLocalDraft { mut draft } => {
            draft.updated_at = chrono::Utc::now().timestamp();
            match cache.upsert_draft(&draft) {
                Ok(draft_id) => {
                    if let Ok(drafts) = cache.list_drafts() {
                        let _ = evt_tx.send(WorkerEvent::DraftsUpdated { drafts });
                    }
                    let _ = evt_tx.send(WorkerEvent::DraftSaved { draft_id });
                }
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: Some(draft.account_id),
                        message: format!("自动保存草稿失败: {e}"),
                    });
                }
            }
        }

        // F-5 用户联系人：写库后统一回发合并后的联系人列表（含聚合结果）。
        MailRequest::SaveUserContact { contact } => match cache.upsert_user_contact(&contact) {
            Ok(_) => {
                let items = cache.list_known_contacts("", 0, 500).unwrap_or_default();
                let _ = evt_tx.send(WorkerEvent::KnownContactsLoaded { items });
                let _ = evt_tx.send(WorkerEvent::Status {
                    account_id: None,
                    text: "联系人已保存".to_string(),
                });
            }
            Err(e) => {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: None,
                    message: e,
                });
            }
        },

        MailRequest::DeleteUserContact { contact_id } => {
            match cache.delete_user_contact(contact_id) {
                Ok(_) => {
                    let items = cache.list_known_contacts("", 0, 500).unwrap_or_default();
                    let _ = evt_tx.send(WorkerEvent::KnownContactsLoaded { items });
                    let _ = evt_tx.send(WorkerEvent::Status {
                        account_id: None,
                        text: "联系人已删除".to_string(),
                    });
                }
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: None,
                        message: e,
                    });
                }
            }
        }

        MailRequest::ImportUserContacts { contacts } => {
            match cache.import_user_contacts(&contacts) {
                Ok((added, skipped)) => {
                    let items = cache.list_known_contacts("", 0, 500).unwrap_or_default();
                    let _ = evt_tx.send(WorkerEvent::KnownContactsLoaded { items });
                    let _ = evt_tx.send(WorkerEvent::ContactsImported { added, skipped });
                }
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: None,
                        message: e,
                    });
                }
            }
        }

        // F-6 附件预览/字节获取：已落盘直接返回，大附件按需下载后再缓存返回。
        MailRequest::FetchAttachmentBytes { attachment_id } => {
            match cache.get_attachment_record(attachment_id) {
                Ok(Some(AttachmentRecord::Local {
                    filename,
                    mime_type,
                    bytes,
                    ..
                })) => {
                    let _ = evt_tx.send(WorkerEvent::AttachmentBytesFetched {
                        attachment_id,
                        filename,
                        mime_type,
                        bytes,
                    });
                }
                Ok(Some(pending @ AttachmentRecord::PendingRemote { .. })) => {
                    let data_dir_cl = data_dir.clone();
                    let tokens_cl = tokens.clone();
                    let async_cache_cl = Arc::clone(async_cache);
                    let evt_tx_cl = evt_tx.clone();
                    tokio::spawn(async move {
                        match download_and_cache_pending_attachment(
                            &data_dir_cl,
                            &tokens_cl,
                            &async_cache_cl,
                            attachment_id,
                            pending,
                            &evt_tx_cl,
                        )
                        .await
                        {
                            Ok((filename, mime_type, bytes)) => {
                                let _ = evt_tx_cl.send(WorkerEvent::AttachmentBytesFetched {
                                    attachment_id,
                                    filename,
                                    mime_type,
                                    bytes,
                                });
                            }
                            Err(e) => {
                                let _ = evt_tx_cl.send(WorkerEvent::Error {
                                    account_id: None,
                                    message: e,
                                });
                            }
                        }
                    });
                }
                Ok(None) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: None,
                        message: "本地缓存中未找到该附件".to_string(),
                    });
                }
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: None,
                        message: e,
                    });
                }
            }
        }

        // F-7 RFC 8058 一键退订：POST `List-Unsubscribe=One-Click` 表单体；
        // 重定向不跟随（重定向后的 200 不代表退订成功，对齐原版 #284）。
        MailRequest::UnsubscribeOneClick { url } => {
            let result = tokio::task::spawn_blocking(move || unsubscribe_one_click_post(&url))
                .await
                .unwrap_or_else(|e| Err(format!("退订任务异常: {e}")));
            match result {
                Ok(()) => {
                    let _ = evt_tx.send(WorkerEvent::Status {
                        account_id: None,
                        text: "退订请求已发送成功".to_string(),
                    });
                }
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: None,
                        message: format!("一键退订失败: {e}"),
                    });
                }
            }
        }

        MailRequest::ArchiveMessage {
            account_id,
            message_id,
        } => {
            move_message_inner(
                data_dir,
                cache,
                tokens,
                account_id,
                message_id,
                MoveTarget::Archive,
                evt_tx,
            )
            .await;
        }

        MailRequest::DeleteMessage {
            account_id,
            message_id,
        } => {
            move_message_inner(
                data_dir,
                cache,
                tokens,
                account_id,
                message_id,
                MoveTarget::Trash,
                evt_tx,
            )
            .await;
        }

        MailRequest::MoveMessage {
            account_id,
            message_id,
            target_folder_path,
        } => {
            move_message_inner(
                data_dir,
                cache,
                tokens,
                account_id,
                message_id,
                MoveTarget::Folder(target_folder_path),
                evt_tx,
            )
            .await;
        }

        MailRequest::MarkSpam {
            account_id,
            message_id,
        } => {
            move_message_inner(
                data_dir,
                cache,
                tokens,
                account_id,
                message_id,
                MoveTarget::Spam,
                evt_tx,
            )
            .await;
        }

        MailRequest::UndoMessageMove {
            account_id,
            message_id: _,
            rfc_message_id,
            current_folder_path,
            original_folder_path,
        } => {
            let Ok(account) = load_account_config(data_dir, account_id) else {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: Some(account_id),
                    message: format!("账号 #{account_id} 未找到配置"),
                });
                return;
            };
            let _ = evt_tx.send(WorkerEvent::Status {
                account_id: Some(account_id),
                text: format!("[{}] 正在撤销移动，移回原文件夹...", account.email),
            });
            let res = match account.auth_backend {
                AuthBackend::MicrosoftGraphOAuth2 => {
                    // 闭包会接管参数，撤销成功后还要用 original_folder_path 重新同步原文件夹。
                    let cur = current_folder_path.clone();
                    let rfc = rfc_message_id.clone();
                    let orig = original_folder_path.clone();
                    match ensure_graph_token(tokens, &account).await {
                        Ok(token) => tokio::task::spawn_blocking(move || {
                            graph_client::undo_move_by_message_id(&token, &cur, &rfc, &orig)
                        })
                        .await
                        .unwrap_or_else(|e| Err(format!("Graph 任务异常: {e}"))),
                        Err(e) => Err(e),
                    }
                }
                AuthBackend::Password => match secrets::load_account_secrets(account_id) {
                    Ok(sec) => {
                        imap_client::undo_move_by_message_id(
                            &account,
                            &sec.imap_password,
                            &current_folder_path,
                            &rfc_message_id,
                            &original_folder_path,
                        )
                        .await
                    }
                    Err(e) => Err(e),
                },
            };
            match res {
                Ok(()) => {
                    // 撤销成功：重新同步原文件夹，把邮件拉回本地缓存并刷新计数。
                    if let Ok(folders) = cache.list_folders(account_id) {
                        let _ = evt_tx.send(WorkerEvent::FoldersLoaded {
                            account_id,
                            folders: folders.clone(),
                        });
                        if let Some(folder) =
                            folders.iter().find(|f| f.path == original_folder_path)
                        {
                            let page_size = pref_page_size(data_dir);
                            sync_folder_inner(
                                cache,
                                tokens,
                                &account,
                                folder.id,
                                &folder.path.clone(),
                                page_size,
                                evt_tx,
                            )
                            .await;
                        }
                    }
                    let _ = evt_tx.send(WorkerEvent::Status {
                        account_id: Some(account_id),
                        text: format!("[{}] 已撤销：邮件已移回原文件夹", account.email),
                    });
                }
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: Some(account_id),
                        message: format!("[{}] {e}", account.email),
                    });
                }
            }
        }

        MailRequest::ExportForwardAttachments { message_id } => {
            let dest_dir = std::env::temp_dir().join(format!("hylki-forward-{message_id}"));
            let att_ids = match cache.list_message_attachment_ids(message_id) {
                Ok(ids) => ids,
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: None,
                        message: format!("查询转发附件失败: {e}"),
                    });
                    return;
                }
            };
            let mut pending: Vec<(u32, AttachmentRecord)> = Vec::new();
            for att_id in att_ids {
                if let Ok(Some(rec @ AttachmentRecord::PendingRemote { .. })) =
                    cache.get_attachment_record(att_id)
                {
                    pending.push((att_id, rec));
                }
            }
            if pending.is_empty() {
                match cache.export_message_attachments(message_id, &dest_dir) {
                    Ok(paths) => {
                        let paths = paths
                            .into_iter()
                            .map(|p| p.to_string_lossy().to_string())
                            .collect();
                        let _ = evt_tx
                            .send(WorkerEvent::ForwardAttachmentsExported { message_id, paths });
                    }
                    Err(e) => {
                        let _ = evt_tx.send(WorkerEvent::Error {
                            account_id: None,
                            message: format!("导出转发附件失败: {e}"),
                        });
                    }
                }
            } else {
                let data_dir_cl = data_dir.clone();
                let tokens_cl = tokens.clone();
                let async_cache_cl = Arc::clone(async_cache);
                let evt_tx_cl = evt_tx.clone();
                tokio::spawn(async move {
                    for (att_id, rec) in pending {
                        if let Err(e) = download_and_cache_pending_attachment(
                            &data_dir_cl,
                            &tokens_cl,
                            &async_cache_cl,
                            att_id,
                            rec,
                            &evt_tx_cl,
                        )
                        .await
                        {
                            let _ = evt_tx_cl.send(WorkerEvent::Error {
                                account_id: None,
                                message: format!("下载转发附件失败: {e}"),
                            });
                            return;
                        }
                    }
                    let exported = tokio::task::spawn_blocking(move || {
                        with_shared_cache(&async_cache_cl, &data_dir_cl, |c| {
                            c.export_message_attachments(message_id, &dest_dir)
                        })
                    })
                    .await
                    .unwrap_or_else(|e| Err(format!("导出转发附件任务异常: {e}")));
                    match exported {
                        Ok(paths) => {
                            let paths = paths
                                .into_iter()
                                .map(|p| p.to_string_lossy().to_string())
                                .collect();
                            let _ = evt_tx_cl.send(WorkerEvent::ForwardAttachmentsExported {
                                message_id,
                                paths,
                            });
                        }
                        Err(e) => {
                            let _ = evt_tx_cl.send(WorkerEvent::Error {
                                account_id: None,
                                message: format!("导出转发附件失败: {e}"),
                            });
                        }
                    }
                });
            }
        }

        MailRequest::SaveAttachment {
            attachment_id,
            dest_path,
        } => match cache.get_attachment_record(attachment_id) {
            Ok(Some(AttachmentRecord::Local { bytes, .. })) => {
                match std::fs::write(&dest_path, bytes) {
                    Ok(()) => {
                        let _ = evt_tx.send(WorkerEvent::AttachmentSaved { path: dest_path });
                    }
                    Err(e) => {
                        let _ = evt_tx.send(WorkerEvent::Error {
                            account_id: None,
                            message: format!("保存附件失败: {e}"),
                        });
                    }
                }
            }
            Ok(Some(pending @ AttachmentRecord::PendingRemote { .. })) => {
                let data_dir_cl = data_dir.clone();
                let tokens_cl = tokens.clone();
                let async_cache_cl = Arc::clone(async_cache);
                let evt_tx_cl = evt_tx.clone();
                tokio::spawn(async move {
                    match download_and_cache_pending_attachment(
                        &data_dir_cl,
                        &tokens_cl,
                        &async_cache_cl,
                        attachment_id,
                        pending,
                        &evt_tx_cl,
                    )
                    .await
                    {
                        Ok((_name, _mime, bytes)) => match std::fs::write(&dest_path, bytes) {
                            Ok(()) => {
                                let _ = evt_tx_cl
                                    .send(WorkerEvent::AttachmentSaved { path: dest_path });
                            }
                            Err(e) => {
                                let _ = evt_tx_cl.send(WorkerEvent::Error {
                                    account_id: None,
                                    message: format!("保存附件失败: {e}"),
                                });
                            }
                        },
                        Err(e) => {
                            let _ = evt_tx_cl.send(WorkerEvent::Error {
                                account_id: None,
                                message: e,
                            });
                        }
                    }
                });
            }
            Ok(None) => {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: None,
                    message: "本地缓存中未找到该附件".to_string(),
                });
            }
            Err(e) => {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: None,
                    message: e,
                });
            }
        },

        MailRequest::SaveAllAttachments {
            attachment_ids,
            dest_dir,
        } => {
            if let Err(e) = std::fs::create_dir_all(&dest_dir) {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: None,
                    message: format!("创建附件导出目录失败: {e}"),
                });
                return;
            }
            let mut records = Vec::with_capacity(attachment_ids.len());
            let mut any_pending = false;
            for att_id in attachment_ids {
                match cache.get_attachment_record(att_id) {
                    Ok(Some(rec)) => {
                        if matches!(rec, AttachmentRecord::PendingRemote { .. }) {
                            any_pending = true;
                        }
                        records.push((att_id, rec));
                    }
                    Ok(None) => {}
                    Err(e) => {
                        let _ = evt_tx.send(WorkerEvent::Error {
                            account_id: None,
                            message: e,
                        });
                    }
                }
            }

            if !any_pending {
                let mut saved_count = 0usize;
                for (_att_id, rec) in records {
                    if let AttachmentRecord::Local {
                        filename, bytes, ..
                    } = rec
                    {
                        let dest_path = crate::cache::unique_attachment_path(&dest_dir, &filename);
                        match std::fs::write(&dest_path, bytes) {
                            Ok(()) => saved_count += 1,
                            Err(e) => {
                                let _ = evt_tx.send(WorkerEvent::Error {
                                    account_id: None,
                                    message: format!("保存附件 {filename} 失败: {e}"),
                                });
                            }
                        }
                    }
                }
                let _ = evt_tx.send(WorkerEvent::AttachmentsSaved {
                    count: saved_count,
                    dest_dir,
                });
            } else {
                let data_dir_cl = data_dir.clone();
                let tokens_cl = tokens.clone();
                let async_cache_cl = Arc::clone(async_cache);
                let evt_tx_cl = evt_tx.clone();
                tokio::spawn(async move {
                    let mut saved_count = 0usize;
                    for (att_id, rec) in records {
                        let resolved = match rec {
                            AttachmentRecord::Local {
                                filename, bytes, ..
                            } => Ok((filename, bytes)),
                            pending @ AttachmentRecord::PendingRemote { .. } => {
                                download_and_cache_pending_attachment(
                                    &data_dir_cl,
                                    &tokens_cl,
                                    &async_cache_cl,
                                    att_id,
                                    pending,
                                    &evt_tx_cl,
                                )
                                .await
                                .map(|(name, _mime, bytes)| (name, bytes))
                            }
                        };
                        match resolved {
                            Ok((name, bytes)) => {
                                let dest_path =
                                    crate::cache::unique_attachment_path(&dest_dir, &name);
                                match std::fs::write(&dest_path, bytes) {
                                    Ok(()) => saved_count += 1,
                                    Err(e) => {
                                        let _ = evt_tx_cl.send(WorkerEvent::Error {
                                            account_id: None,
                                            message: format!("保存附件 {name} 失败: {e}"),
                                        });
                                    }
                                }
                            }
                            Err(e) => {
                                let _ = evt_tx_cl.send(WorkerEvent::Error {
                                    account_id: None,
                                    message: e,
                                });
                            }
                        }
                    }
                    let _ = evt_tx_cl.send(WorkerEvent::AttachmentsSaved {
                        count: saved_count,
                        dest_dir,
                    });
                });
            }
        }

        MailRequest::StripAttachment {
            account_id,
            message_id,
            attachment_id,
            attachment_name,
            size_bytes,
        } => {
            let Ok(account) = load_account_config(data_dir, account_id) else {
                let _ = evt_tx.send(WorkerEvent::AttachmentStripFailed {
                    account_id,
                    message_id,
                    attachment_id,
                    message: format!("账号 #{account_id} 未找到配置"),
                });
                return;
            };
            if account.auth_backend == AuthBackend::MicrosoftGraphOAuth2 {
                let _ = evt_tx.send(WorkerEvent::AttachmentStripFailed {
                    account_id,
                    message_id,
                    attachment_id,
                    message: "Microsoft Graph 账号暂不支持服务端剥离附件".to_string(),
                });
                return;
            }
            let loc = match cache.get_message_location(message_id) {
                Ok(Some(loc)) => loc,
                Ok(None) => {
                    let _ = evt_tx.send(WorkerEvent::AttachmentStripFailed {
                        account_id,
                        message_id,
                        attachment_id,
                        message: "本地缓存中未找到该邮件".to_string(),
                    });
                    return;
                }
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::AttachmentStripFailed {
                        account_id,
                        message_id,
                        attachment_id,
                        message: e,
                    });
                    return;
                }
            };
            let (folder_path, uid) = loc;
            let res = match secrets::load_account_secrets(account_id) {
                Ok(sec) => {
                    imap_client::strip_imap_attachment(
                        &account,
                        &sec.imap_password,
                        &folder_path,
                        uid,
                        &attachment_name,
                        size_bytes as u64,
                    )
                    .await
                }
                Err(e) => Err(e),
            };
            match res {
                Ok((new_uid, _stripped)) => {
                    match cache.delete_attachment_and_update_uid(message_id, attachment_id, new_uid)
                    {
                        Ok(attachments) => {
                            let _ = evt_tx.send(WorkerEvent::AttachmentStripped {
                                account_id,
                                message_id,
                                attachment_id,
                                attachments,
                            });
                            let _ = evt_tx.send(WorkerEvent::Status {
                                account_id: Some(account_id),
                                text: format!("已从服务器剥离附件：{attachment_name}"),
                            });
                        }
                        Err(e) => {
                            let _ = evt_tx.send(WorkerEvent::AttachmentStripFailed {
                                account_id,
                                message_id,
                                attachment_id,
                                message: e,
                            });
                        }
                    }
                }
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::AttachmentStripFailed {
                        account_id,
                        message_id,
                        attachment_id,
                        message: e,
                    });
                }
            }
        }

        MailRequest::SaveLocalDraft { mut draft } => {
            draft.updated_at = chrono::Utc::now().timestamp();
            match cache.upsert_draft(&draft) {
                Ok(draft_id) => {
                    if let Ok(drafts) = cache.list_drafts() {
                        let _ = evt_tx.send(WorkerEvent::DraftsUpdated { drafts });
                    }
                    // F-4：写信页据此绑定新分配的本地草稿 ID。
                    let _ = evt_tx.send(WorkerEvent::DraftSaved { draft_id });
                    let _ = evt_tx.send(WorkerEvent::Status {
                        account_id: Some(draft.account_id),
                        text: "草稿已保存至本地".to_string(),
                    });
                }
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: Some(draft.account_id),
                        message: e,
                    });
                }
            }
        }

        MailRequest::DeleteLocalDraft { draft_id } => {
            let _ = cache.delete_draft(draft_id);
            if let Ok(drafts) = cache.list_drafts() {
                let _ = evt_tx.send(WorkerEvent::DraftsUpdated { drafts });
            }
            if let Ok(tags) = cache.list_tags() {
                let _ = evt_tx.send(WorkerEvent::TagsLoaded { tags });
            }
        }

        // WP-10-5 写信模板：保存 / 删除都回推整份列表，UI 无需自行合并。
        MailRequest::SaveTemplate { template } => match cache.upsert_template(&template) {
            Ok(_) => {
                if let Ok(templates) = cache.list_templates() {
                    let _ = evt_tx.send(WorkerEvent::TemplatesUpdated { templates });
                }
            }
            Err(message) => {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: None,
                    message,
                });
            }
        },
        MailRequest::DeleteTemplate { template_id } => {
            if let Err(message) = cache.delete_template(template_id) {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: None,
                    message,
                });
            } else if let Ok(templates) = cache.list_templates() {
                let _ = evt_tx.send(WorkerEvent::TemplatesUpdated { templates });
            }
        }
        MailRequest::LoadTemplates => {
            if let Ok(templates) = cache.list_templates() {
                let _ = evt_tx.send(WorkerEvent::TemplatesUpdated { templates });
            }
        }

        MailRequest::SendDraft {
            account_id,
            draft,
            local_draft_id,
            undo_token,
            send_at,
        } => {
            let now = chrono::Utc::now().timestamp();
            let outbox_id = match cache.enqueue_outbox_from_draft(
                account_id,
                &draft.to,
                &draft.cc,
                &draft.bcc,
                &draft.subject,
                &draft.body,
                &draft.in_reply_to,
                &draft.from_addr,
                &draft.reply_to,
                draft.body_html.as_deref(),
                &draft.attachment_paths,
                now,
                send_at.map(|delay| now + delay),
                local_draft_id,
            ) {
                Ok(id) => id,
                Err(e) => {
                    let recovered_draft_id = cache
                        .upsert_draft(&crate::LocalDraft {
                            id: local_draft_id.unwrap_or(0),
                            account_id,
                            to_addr: draft.to.clone(),
                            cc_addr: draft.cc.clone(),
                            bcc_addr: draft.bcc.clone(),
                            subject: draft.subject.clone(),
                            body_text: draft.body.clone(),
                            in_reply_to: draft.in_reply_to.clone(),
                            from_addr: draft.from_addr.clone(),
                            reply_to: draft.reply_to.clone(),
                            body_format: draft.body_format.clone(),
                            attachment_paths: draft.attachment_paths.clone(),
                            updated_at: now,
                        })
                        .ok();
                    let _ = evt_tx.send(WorkerEvent::OutboxEnqueueFailed {
                        token: undo_token,
                        message: e,
                        recovered_draft_id,
                    });
                    if let Ok(drafts) = cache.list_drafts() {
                        let _ = evt_tx.send(WorkerEvent::DraftsUpdated { drafts });
                    }
                    return;
                }
            };
            // 原草稿交接给发件箱：记录落盘成功后立即删除，保证内容不丢也不重复。
            if local_draft_id.is_some() {
                if let Ok(drafts) = cache.list_drafts() {
                    let _ = evt_tx.send(WorkerEvent::DraftsUpdated { drafts });
                }
            }
            if send_at.is_some() {
                // 撤销发送缓冲：仅入队不投递（Queued + 到期时间），等待撤销或到期触发。
                let _ = evt_tx.send(WorkerEvent::OutboxEnqueued {
                    token: undo_token,
                    account_id,
                    outbox_id,
                });
            } else {
                send_outbox_item(data_dir, cache, tokens, account_id, outbox_id, evt_tx).await;
            }
            if let Ok(outbox) = cache.list_outbox(account_id) {
                let _ = evt_tx.send(WorkerEvent::OutboxUpdated {
                    account_id,
                    items: outbox,
                });
            }
        }

        MailRequest::RetryOutboxItem {
            account_id,
            outbox_id,
        } => {
            retry_outbox_item(data_dir, cache, tokens, account_id, outbox_id, evt_tx).await;
        }

        MailRequest::DeleteOutboxItem {
            account_id,
            outbox_id,
        } => {
            // 撤销发送：仅删除仍在排队（Queued）的记录；已被领取投递的不可撤销。
            match cache.delete_queued_outbox(outbox_id) {
                Ok(true) => {
                    if let Ok(outbox) = cache.list_outbox(account_id) {
                        let _ = evt_tx.send(WorkerEvent::OutboxUpdated {
                            account_id,
                            items: outbox,
                        });
                    }
                }
                Ok(false) => {}
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: Some(account_id),
                        message: e,
                    });
                }
            }
        }

        MailRequest::PrepareQuit => unreachable!("退出准备由 worker_loop 处理"),

        MailRequest::UndoOutboxItem {
            account_id,
            outbox_id,
            token,
        } => {
            let result = cache.undo_queued_outbox(account_id, outbox_id);
            let success = matches!(result, Ok(Some(_)));
            let _ = evt_tx.send(WorkerEvent::OutboxUndoResult { token, result });
            if success {
                if let Ok(drafts) = cache.list_drafts() {
                    let _ = evt_tx.send(WorkerEvent::DraftsUpdated { drafts });
                }
                if let Ok(items) = cache.list_outbox(account_id) {
                    let _ = evt_tx.send(WorkerEvent::OutboxUpdated { account_id, items });
                }
            }
        }

        MailRequest::LoadUnifiedView {
            view,
            query,
            filter,
            quick_filters,
            sort,
            offset,
            limit,
        } => {
            let effective = merge_filter_options(filter, quick_filters);
            let allowed = unified_account_ids(data_dir);
            let result = match view {
                UnifiedView::Unread => cache.list_unread_messages_for_accounts(
                    &allowed, &query, effective, sort, offset, limit,
                ),
                UnifiedView::Starred => cache.list_starred_messages_for_accounts(
                    &allowed, &query, effective, sort, offset, limit,
                ),
                UnifiedView::Sent => cache.list_sent_messages_for_accounts(
                    &allowed, &query, effective, sort, offset, limit,
                ),
                UnifiedView::Archive => cache.list_archived_messages_for_accounts(
                    &allowed, &query, effective, sort, offset, limit,
                ),
                UnifiedView::Junk => cache.list_junk_messages_for_accounts(
                    &allowed, &query, effective, sort, offset, limit,
                ),
                UnifiedView::Trash => cache.list_trash_messages_for_accounts(
                    &allowed, &query, effective, sort, offset, limit,
                ),
                UnifiedView::Snoozed => cache.list_snoozed_messages_for_accounts(
                    &allowed, &query, effective, sort, offset, limit,
                ),
            };
            match result {
                Ok(page) => {
                    let _ = evt_tx.send(WorkerEvent::UnifiedViewLoaded { view, page });
                }
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: None,
                        message: e,
                    });
                }
            }
        }

        MailRequest::ListCachedAttachments {
            query,
            offset,
            limit,
        } => match cache.list_cached_attachments(&query, offset, limit) {
            Ok(items) => {
                let _ = evt_tx.send(WorkerEvent::CachedAttachmentsLoaded { items });
            }
            Err(e) => {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: None,
                    message: e,
                });
            }
        },

        MailRequest::ListKnownContacts {
            query,
            offset,
            limit,
        } => match cache.list_known_contacts(&query, offset, limit) {
            Ok(items) => {
                let _ = evt_tx.send(WorkerEvent::KnownContactsLoaded { items });
            }
            Err(e) => {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: None,
                    message: e,
                });
            }
        },

        MailRequest::LoadOlderMessages { folder_id, limit } => {
            load_older_messages_inner(data_dir, cache, tokens, folder_id, limit, evt_tx).await;
        }

        MailRequest::ClearBodyCaches => match cache.clear_message_bodies() {
            Ok(_) => {
                let _ = evt_tx.send(WorkerEvent::Status {
                    account_id: None,
                    text: "正文缓存已清理".to_string(),
                });
            }
            Err(e) => {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: None,
                    message: e,
                });
            }
        },

        MailRequest::PruneExpiredDownloads { keep_days } => {
            match cache.prune_expired_downloads(keep_days, chrono::Utc::now().timestamp()) {
                Ok(pruned) if pruned > 0 => {
                    let _ = evt_tx.send(WorkerEvent::Status {
                        account_id: None,
                        text: format!("已清理 {pruned} 封过期离线正文与附件缓存"),
                    });
                }
                Ok(_) => {}
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: None,
                        message: e,
                    });
                }
            }
        }

        MailRequest::SetPushEnabled { enabled } => {
            if let Ok(accounts) = config::load_accounts_in(data_dir) {
                for acc in accounts {
                    if acc.enabled && acc.effective_push(enabled) {
                        if !watchers.contains_key(&acc.id) {
                            spawn_account_idle_watcher(watchers, acc, req_tx.clone());
                        }
                    } else {
                        stop_account_idle_watcher(watchers, acc.id);
                    }
                }
            }
            let _ = evt_tx.send(WorkerEvent::Status {
                account_id: None,
                text: if enabled {
                    "即时推送收信已开启".to_string()
                } else {
                    "即时推送收信已关闭（将按定时同步）".to_string()
                },
            });
        }

        MailRequest::ReorderAccounts { account_ids } => {
            match config::reorder_accounts_in(data_dir, &account_ids) {
                Ok(accounts) => {
                    let _ = evt_tx.send(WorkerEvent::AccountsUpdated { accounts });
                }
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: None,
                        message: e,
                    });
                }
            }
        }

        MailRequest::SetAccountEnabled {
            account_id,
            enabled,
        } => match config::set_account_enabled_in(data_dir, account_id, enabled) {
            Ok(Some(updated)) => {
                refresh_account_watcher(watchers, data_dir, &updated, req_tx);
                if let Ok(accounts) = config::load_accounts_in(data_dir) {
                    let _ = evt_tx.send(WorkerEvent::AccountsUpdated { accounts });
                }
                let page_size = pref_page_size(data_dir);
                let allowed = unified_account_ids(data_dir);
                if let Ok(page) = cache.list_unified_inbox_for_accounts(
                    &allowed,
                    "",
                    QuickFilterSet::default(),
                    MessageSort::DateNewest,
                    0,
                    page_size,
                ) {
                    let _ = evt_tx.send(WorkerEvent::MessagesPageLoaded(page));
                }
                if enabled {
                    // 自动续发（任务一）：重新启用账号后只触发该账号已到期的 Queued
                    // 邮件；Failed / Uncertain 不参与。领取为原子转换，反复启停
                    // 账号时同一条记录只会被领取一次。
                    let now = chrono::Utc::now().timestamp();
                    let due = cache
                        .list_due_queued_ids(Some(account_id), now)
                        .unwrap_or_default();
                    for (_, outbox_id) in due {
                        send_outbox_item(data_dir, cache, tokens, account_id, outbox_id, evt_tx)
                            .await;
                    }
                    sync_account_inner(data_dir, cache, tokens, account_id, evt_tx).await;
                    let allowed = unified_account_ids(data_dir);
                    if let Ok(page) = cache.list_unified_inbox_for_accounts(
                        &allowed,
                        "",
                        QuickFilterSet::default(),
                        MessageSort::DateNewest,
                        0,
                        page_size,
                    ) {
                        let _ = evt_tx.send(WorkerEvent::MessagesPageLoaded(page));
                    }
                }
            }
            Ok(None) => {}
            Err(e) => {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: Some(account_id),
                    message: e,
                });
            }
        },

        MailRequest::LoadThreadChildren {
            account_id,
            message_id,
            subject,
        } => match cache.list_thread_summaries(account_id, message_id, &subject) {
            Ok(children) => {
                let _ = evt_tx.send(WorkerEvent::ThreadChildrenLoaded {
                    account_id,
                    head_message_id: message_id,
                    children,
                });
            }
            Err(e) => {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: Some(account_id),
                    message: e,
                });
            }
        },

        MailRequest::SetMessageTags {
            account_id,
            message_id,
            tags,
        } => match cache.set_message_tags(message_id, &tags) {
            Ok(()) => {
                let _ = evt_tx.send(WorkerEvent::MessageTagsUpdated {
                    account_id,
                    message_id,
                    tags,
                });
            }
            Err(e) => {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: Some(account_id),
                    message: e,
                });
            }
        },

        MailRequest::SnoozeMessage {
            account_id,
            message_id,
            until_ts,
        } => match cache.snooze_message(message_id, until_ts) {
            Ok(()) => {
                let _ = evt_tx.send(WorkerEvent::MessageSnoozed {
                    account_id,
                    message_id,
                    until_ts,
                });
            }
            Err(e) => {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: Some(account_id),
                    message: e,
                });
            }
        },

        MailRequest::CopyMessage {
            account_id,
            message_id,
            target_account_id,
            target_folder_path,
        } => match cache.copy_message_to_folder(message_id, target_account_id, &target_folder_path)
        {
            Ok(()) => {
                let _ = evt_tx.send(WorkerEvent::MessageCopied {
                    message_id,
                    target_account_id,
                    target_folder_path,
                });
            }
            Err(e) => {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: Some(account_id),
                    message: e,
                });
            }
        },

        MailRequest::ListTags => match cache.list_tags() {
            Ok(tags) => {
                let _ = evt_tx.send(WorkerEvent::TagsLoaded { tags });
            }
            Err(e) => {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: None,
                    message: e,
                });
            }
        },

        MailRequest::CreateFolder {
            account_id,
            folder_path,
        } => {
            let trimmed = folder_path.trim().to_string();
            if trimmed.is_empty() {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: Some(account_id),
                    message: "文件夹名称不能为空".to_string(),
                });
                return;
            }
            let account = match load_account_config(data_dir, account_id) {
                Ok(acc) => acc,
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: Some(account_id),
                        message: e,
                    });
                    return;
                }
            };
            if account.auth_backend == AuthBackend::MicrosoftGraphOAuth2 {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: Some(account_id),
                    message: "Microsoft Graph 账号暂不支持远端新建文件夹".to_string(),
                });
                return;
            }
            let res = match secrets::load_account_secrets(account_id) {
                Ok(sec) => imap_client::create_folder(&account, &sec.imap_password, &trimmed).await,
                Err(e) => Err(e),
            };
            match res {
                Ok(()) => {
                    if let Ok(sec) = secrets::load_account_secrets(account_id) {
                        if let Ok(remote) =
                            imap_client::fetch_folders(&account, &sec.imap_password).await
                        {
                            if let Ok(folders) = cache.sync_folders(account_id, &remote) {
                                let _ = evt_tx.send(WorkerEvent::FoldersLoaded {
                                    account_id,
                                    folders,
                                });
                            }
                        }
                    }
                    let _ = evt_tx.send(WorkerEvent::Status {
                        account_id: Some(account_id),
                        text: format!("已创建文件夹「{trimmed}」"),
                    });
                }
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: Some(account_id),
                        message: e,
                    });
                }
            }
        }

        MailRequest::RenameFolder {
            account_id,
            folder_id,
            old_path,
            new_name,
        } => {
            let trimmed = new_name.trim().to_string();
            if trimmed.is_empty() {
                return;
            }
            let account = match load_account_config(data_dir, account_id) {
                Ok(acc) => acc,
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: Some(account_id),
                        message: e,
                    });
                    return;
                }
            };
            if account.auth_backend == AuthBackend::MicrosoftGraphOAuth2 {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: Some(account_id),
                    message: "Microsoft Graph 账号暂不支持远端重命名文件夹".to_string(),
                });
                return;
            }
            let res = match secrets::load_account_secrets(account_id) {
                Ok(sec) => {
                    imap_client::rename_folder(&account, &sec.imap_password, &old_path, &trimmed)
                        .await
                }
                Err(e) => Err(e),
            };
            match res {
                Ok(None) => {}
                Ok(Some((new_path, delimiter))) => {
                    let leaf_name = trimmed;
                    match cache.rename_folder_entry(
                        account_id,
                        folder_id,
                        &old_path,
                        &new_path,
                        &leaf_name,
                        delimiter.as_deref(),
                    ) {
                        Ok(folders) => {
                            let _ = evt_tx.send(WorkerEvent::FoldersLoaded {
                                account_id,
                                folders,
                            });
                        }
                        Err(e) => {
                            let _ = evt_tx.send(WorkerEvent::Error {
                                account_id: Some(account_id),
                                message: format!("远端文件夹已重命名，但更新本地缓存失败: {e}"),
                            });
                            return;
                        }
                    }
                    let _ = evt_tx.send(WorkerEvent::Status {
                        account_id: Some(account_id),
                        text: format!("已将文件夹重命名为「{leaf_name}」"),
                    });
                }
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: Some(account_id),
                        message: e,
                    });
                }
            }
        }

        MailRequest::DeleteFolder {
            account_id,
            folder_id,
            folder_path,
        } => {
            let account = match load_account_config(data_dir, account_id) {
                Ok(acc) => acc,
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: Some(account_id),
                        message: e,
                    });
                    return;
                }
            };
            if account.auth_backend == AuthBackend::MicrosoftGraphOAuth2 {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: Some(account_id),
                    message: "Microsoft Graph 账号暂不支持远端删除文件夹".to_string(),
                });
                return;
            }
            let res = match secrets::load_account_secrets(account_id) {
                Ok(sec) => {
                    imap_client::delete_folder(&account, &sec.imap_password, &folder_path).await
                }
                Err(e) => Err(e),
            };
            match res {
                Ok(()) => {
                    if let Ok(folders) = cache.delete_folder_entry(account_id, folder_id) {
                        let _ = evt_tx.send(WorkerEvent::FoldersLoaded {
                            account_id,
                            folders,
                        });
                    }
                    let _ = evt_tx.send(WorkerEvent::Status {
                        account_id: Some(account_id),
                        text: format!("已删除文件夹「{folder_path}」"),
                    });
                }
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: Some(account_id),
                        message: e,
                    });
                }
            }
        }
    }
}

/// Destination folder role or explicit path for server-side message moves.
#[derive(Clone, Debug, PartialEq, Eq)]
enum MoveTarget {
    Archive,
    Trash,
    Spam,
    Folder(String),
}

/// Move a message to the account's archive, trash, spam, or explicit target
/// folder on the server, then drop it from the local cache and emit a
/// completion event. Failures (missing account config, missing remote ID,
/// no target folder, remote error) are reported through the regular error
/// event channel.
async fn move_message_inner(
    data_dir: &PathBuf,
    cache: &mut CacheStore,
    tokens: &SharedTokenCache,
    account_id: u32,
    message_id: u32,
    target: MoveTarget,
    evt_tx: &EventSender,
) {
    let Some((detail, uid, _)) = cache.get_message_detail(message_id).ok().flatten() else {
        let _ = evt_tx.send(WorkerEvent::Error {
            account_id: Some(account_id),
            message: "本地缓存中未找到该邮件，无法移动或删除".to_string(),
        });
        return;
    };
    let Ok(account) = load_account_config(data_dir, account_id) else {
        let _ = evt_tx.send(WorkerEvent::Error {
            account_id: Some(account_id),
            message: format!("账号 #{account_id} 未找到配置"),
        });
        return;
    };

    if let MoveTarget::Folder(ref target_path) = target {
        if target_path.trim().is_empty() {
            let _ = evt_tx.send(WorkerEvent::Error {
                account_id: Some(account_id),
                message: "目标文件夹路径不能为空".to_string(),
            });
            return;
        }
        if detail.folder_path == *target_path {
            let _ = evt_tx.send(WorkerEvent::Status {
                account_id: Some(account_id),
                text: "邮件已在目标文件夹中".to_string(),
            });
            return;
        }
    }

    let op_name = match target {
        MoveTarget::Archive => "归档",
        MoveTarget::Trash => "删除",
        MoveTarget::Spam => "标记为垃圾邮件",
        MoveTarget::Folder(_) => "移动",
    };

    // F-3：记录移动目标（撤销时按 Message-ID 在该文件夹检索）；IMAP 无回收站的
    // 永久删除（\Deleted + EXPUNGE）不可撤销，保持 None。
    let mut dest_for_undo: Option<String> = None;
    let res = match account.auth_backend {
        AuthBackend::MicrosoftGraphOAuth2 => {
            let Ok(Some(remote_id)) = cache.get_message_remote_id(message_id) else {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: Some(account_id),
                    message: format!("[{}] 缺少远端 ID，无法{}该邮件", account.email, op_name),
                });
                return;
            };
            let dest_folder = match &target {
                // Graph accepts well-known folder names directly.
                MoveTarget::Trash => Ok("deleteditems".to_string()),
                MoveTarget::Spam => match cache.find_spam_folder(account_id) {
                    Ok(Some((_, path))) => Ok(path),
                    Ok(None) => Ok("junkemail".to_string()),
                    Err(e) => Err(e),
                },
                MoveTarget::Archive => match cache.find_archive_folder(account_id) {
                    Ok(Some((_, path))) => Ok(path),
                    Ok(None) => Err("该账号没有归档文件夹，无法归档邮件".to_string()),
                    Err(e) => Err(e),
                },
                MoveTarget::Folder(path) => Ok(path.clone()),
            };
            match dest_folder {
                Ok(dest) => {
                    dest_for_undo = Some(dest.clone());
                    match ensure_graph_token(tokens, &account).await {
                        Ok(token) => tokio::task::spawn_blocking(move || {
                            graph_client::move_message(&token, &remote_id, &dest)
                        })
                        .await
                        .unwrap_or_else(|e| Err(format!("Graph 任务异常: {e}"))),
                        Err(e) => Err(e),
                    }
                }
                Err(e) => Err(e),
            }
        }
        AuthBackend::Password => {
            let dest_folder = match &target {
                MoveTarget::Archive => match cache.find_archive_folder(account_id) {
                    Ok(Some((_, path))) => Ok(Some(path)),
                    Ok(None) => Err("该账号没有归档文件夹，无法归档邮件".to_string()),
                    Err(e) => Err(e),
                },
                // No trash folder: fall back to \Deleted + EXPUNGE.
                MoveTarget::Trash => Ok(cache
                    .find_trash_folder(account_id)
                    .ok()
                    .flatten()
                    .map(|(_, path)| path)),
                MoveTarget::Spam => match cache.find_spam_folder(account_id) {
                    Ok(Some((_, path))) => Ok(Some(path)),
                    Ok(None) => Err("该账号没有垃圾邮件文件夹，无法标记为垃圾邮件".to_string()),
                    Err(e) => Err(e),
                },
                MoveTarget::Folder(path) => Ok(Some(path.clone())),
            };
            match dest_folder {
                Ok(Some(dest)) => {
                    dest_for_undo = Some(dest.clone());
                    match secrets::load_account_secrets(account_id) {
                        Ok(sec) => {
                            imap_client::move_message(
                                &account,
                                &sec.imap_password,
                                &detail.folder_path,
                                uid,
                                &dest,
                            )
                            .await
                        }
                        Err(e) => Err(e),
                    }
                }
                Ok(None) => match secrets::load_account_secrets(account_id) {
                    Ok(sec) => {
                        imap_client::delete_message(
                            &account,
                            &sec.imap_password,
                            &detail.folder_path,
                            uid,
                        )
                        .await
                    }
                    Err(e) => Err(e),
                },
                Err(e) => Err(e),
            }
        }
    };

    match res {
        Ok(()) => {
            // Drop the message from the old folder's cache; the UI refreshes
            // its lists when it receives the completion event.
            let _ = cache.remove_message(message_id);
            if let Ok(folders) = cache.list_folders(account_id) {
                let _ = evt_tx.send(WorkerEvent::FoldersLoaded {
                    account_id,
                    folders,
                });
            }
            let dest_folder_path = dest_for_undo.unwrap_or_default();
            let rfc_message_id = detail.rfc_message_id.clone();
            let _ = evt_tx.send(match target {
                MoveTarget::Archive => WorkerEvent::MessageArchived {
                    account_id,
                    message_id,
                    dest_folder_path,
                    rfc_message_id,
                },
                MoveTarget::Trash => WorkerEvent::MessageDeleted {
                    account_id,
                    message_id,
                    dest_folder_path,
                    rfc_message_id,
                },
                MoveTarget::Spam => WorkerEvent::MessageMarkedSpam {
                    account_id,
                    message_id,
                    dest_folder_path,
                    rfc_message_id,
                },
                MoveTarget::Folder(target_folder_path) => WorkerEvent::MessageMoved {
                    account_id,
                    message_id,
                    target_folder_path,
                    rfc_message_id,
                },
            });
            let _ = evt_tx.send(WorkerEvent::Status {
                account_id: Some(account_id),
                text: format!("[{}] 邮件{}完成", account.email, op_name),
            });
        }
        Err(e) => {
            let _ = evt_tx.send(WorkerEvent::Error {
                account_id: Some(account_id),
                message: format!("[{}] {e}", account.email),
            });
        }
    }
}

async fn ensure_graph_token(
    tokens: &SharedTokenCache,
    account: &AccountConfig,
) -> Result<String, String> {
    tokens.ensure_graph_token(account).await
}

async fn download_and_cache_pending_attachment(
    data_dir: &PathBuf,
    tokens: &SharedTokenCache,
    async_cache: &SharedAsyncCache,
    attachment_id: u32,
    record: AttachmentRecord,
    evt_tx: &EventSender,
) -> Result<(String, String, Vec<u8>), String> {
    let AttachmentRecord::PendingRemote {
        message_id,
        account_id,
        folder_path,
        uid,
        remote_msg_id,
        filename,
        mime_type,
        remote_part,
        transfer_encoding,
        ..
    } = record
    else {
        return Err("附件状态异常".to_string());
    };

    let account = load_account_config(data_dir, account_id)?;
    let _ = evt_tx.send(WorkerEvent::Status {
        account_id: Some(account_id),
        text: format!("正在下载附件 {filename}..."),
    });

    let bytes = match account.auth_backend {
        AuthBackend::MicrosoftGraphOAuth2 => {
            if remote_msg_id.is_empty() || remote_part.is_empty() {
                return Err(format!("缺少远端附件定位信息 ({filename})"));
            }
            let token = tokens.ensure_graph_token(&account).await?;
            tokio::task::spawn_blocking(move || {
                graph_client::fetch_attachment_content(&token, &remote_msg_id, &remote_part)
            })
            .await
            .map_err(|e| format!("下载 Graph 附件任务异常: {e}"))??
        }
        AuthBackend::Password => {
            let sec = secrets::load_account_secrets(account_id)?;
            imap_client::fetch_attachment_section(
                &account,
                &sec.imap_password,
                &folder_path,
                uid,
                &remote_part,
                &transfer_encoding,
                &filename,
            )
            .await?
        }
    };

    let data_dir_cl = data_dir.clone();
    let async_cache_cl = Arc::clone(async_cache);
    let bytes_for_db = bytes.clone();
    let stored_size = tokio::task::spawn_blocking(move || {
        with_shared_cache(&async_cache_cl, &data_dir_cl, |c| {
            c.store_downloaded_attachment_bytes(attachment_id, &bytes_for_db)
        })
    })
    .await
    .map_err(|e| format!("保存已下载附件任务异常: {e}"))??;

    let _ = evt_tx.send(WorkerEvent::AttachmentDownloaded {
        message_id,
        attachment_id,
        size_bytes: stored_size,
    });
    Ok((filename, mime_type, bytes))
}

async fn sync_account_inner(
    data_dir: &PathBuf,
    cache: &mut CacheStore,
    tokens: &SharedTokenCache,
    account_id: u32,
    evt_tx: &EventSender,
) {
    let Ok(account) = load_account_config(data_dir, account_id) else {
        let _ = evt_tx.send(WorkerEvent::Error {
            account_id: Some(account_id),
            message: format!("账号 #{account_id} 未找到配置"),
        });
        return;
    };
    if !account.enabled {
        return;
    }

    let _ = evt_tx.send(WorkerEvent::Status {
        account_id: Some(account_id),
        text: format!("[{}] 正在同步文件夹与邮件头...", account.email),
    });

    let folders = match account.auth_backend {
        AuthBackend::MicrosoftGraphOAuth2 => {
            let token = match ensure_graph_token(tokens, &account).await {
                Ok(t) => t,
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: Some(account_id),
                        message: format!("[{}] {e}", account.email),
                    });
                    return;
                }
            };
            let remote = tokio::task::spawn_blocking(move || graph_client::fetch_folders(&token))
                .await
                .unwrap_or_else(|e| Err(format!("Graph 文件夹任务异常: {e}")));
            match remote {
                Ok(list) => match cache.sync_folders_with_flags(account_id, &list) {
                    Ok(f) => f,
                    Err(e) => {
                        let _ = evt_tx.send(WorkerEvent::Error {
                            account_id: Some(account_id),
                            message: format!("[{}] {e}", account.email),
                        });
                        return;
                    }
                },
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: Some(account_id),
                        message: format!("[{}] {e}", account.email),
                    });
                    return;
                }
            }
        }
        AuthBackend::Password => {
            let Ok(sec) = secrets::load_account_secrets(account_id) else {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: Some(account_id),
                    message: format!("[{}] 缺少系统凭据", account.email),
                });
                return;
            };
            let remote_folders =
                match imap_client::fetch_folders(&account, &sec.imap_password).await {
                    Ok(f) => f,
                    Err(e) => {
                        let _ = evt_tx.send(WorkerEvent::Error {
                            account_id: Some(account_id),
                            message: format!("[{}] {e}", account.email),
                        });
                        return;
                    }
                };
            match cache.sync_folders(account_id, &remote_folders) {
                Ok(f) => f,
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: Some(account_id),
                        message: format!("[{}] {e}", account.email),
                    });
                    return;
                }
            }
        }
    };

    if let Some(inbox) = folders
        .iter()
        .find(|f| f.path.eq_ignore_ascii_case("INBOX") || f.name == "收件箱")
        .or_else(|| folders.first())
    {
        let page_size = pref_page_size(data_dir);
        sync_folder_inner(
            cache,
            tokens,
            &account,
            inbox.id,
            &inbox.path,
            page_size,
            evt_tx,
        )
        .await;
    }

    if let Ok(updated_folders) = cache.list_folders(account_id) {
        let _ = evt_tx.send(WorkerEvent::FoldersLoaded {
            account_id,
            folders: updated_folders,
        });
    }
}

fn pref_page_size(data_dir: &PathBuf) -> usize {
    config::load_preferences_in(data_dir)
        .map(|p| p.page_size)
        .unwrap_or(100)
}

fn pref_thread_newest_first(data_dir: &PathBuf) -> bool {
    config::load_preferences_in(data_dir)
        .map(|p| p.thread_newest_first)
        .unwrap_or(true)
}

async fn sync_folder_inner(
    cache: &mut CacheStore,
    tokens: &SharedTokenCache,
    account: &AccountConfig,
    folder_id: u32,
    folder_path: &str,
    page_size: usize,
    evt_tx: &EventSender,
) {
    let sync_res: Result<(), String> = match account.auth_backend {
        AuthBackend::MicrosoftGraphOAuth2 => {
            let token = match ensure_graph_token(tokens, account).await {
                Ok(t) => t,
                Err(e) => {
                    let _ = evt_tx.send(WorkerEvent::Error {
                        account_id: Some(account.id),
                        message: format!("[{}] {e}", account.email),
                    });
                    return;
                }
            };
            let prev_cached = cache
                .folder_remote_cursor(folder_id)
                .ok()
                .flatten()
                .map(|(_, _, _, _, _, cnt)| cnt)
                .unwrap_or(0);
            let fp = folder_path.to_string();
            let fetched = tokio::task::spawn_blocking(move || {
                graph_client::fetch_folder_headers(&token, &fp)
            })
            .await
            .unwrap_or_else(|e| Err(format!("Graph 拉取邮件头任务异常: {e}")));
            match fetched {
                Ok((items, next_link)) => {
                    // 传入 None 避免误删用户此前已预加载到本地的更早历史邮件。
                    let commit_res = cache
                        .commit_folder_headers_with_remote(account.id, folder_id, 1, &items, None);
                    if commit_res.is_ok() && prev_cached <= 100 {
                        let _ = cache.set_folder_remote_cursor(
                            folder_id,
                            next_link.is_some(),
                            next_link.as_deref().unwrap_or(""),
                        );
                    }
                    commit_res
                }
                Err(e) => Err(e),
            }
        }
        AuthBackend::Password => match secrets::load_account_secrets(account.id) {
            Ok(sec) => {
                match tokio::time::timeout(
                    Duration::from_secs(60),
                    imap_client::fetch_folder_headers_with_flags(
                        account,
                        &sec.imap_password,
                        folder_path,
                    ),
                )
                .await
                .unwrap_or_else(|_| Err("同步文件夹超时".to_string()))
                {
                    Ok((uid_validity, headers, live_uids, flag_snapshot)) => {
                        let commit_res = cache.commit_folder_headers(
                            account.id,
                            folder_id,
                            uid_validity,
                            &headers,
                            Some(&live_uids),
                        );
                        if commit_res.is_ok() {
                            if let Some(flags) = flag_snapshot {
                                let _ = cache.reconcile_folder_flags(
                                    folder_id,
                                    &flags.flagged_uids,
                                    &flags.unseen_uids,
                                );
                            }
                            let min_cached_uid = cache
                                .folder_remote_cursor(folder_id)
                                .ok()
                                .flatten()
                                .and_then(|(_, _, _, _, min_uid, _)| min_uid);
                            let has_more = live_uids
                                .first()
                                .zip(min_cached_uid)
                                .is_some_and(|(&min_remote, min_cached)| min_remote < min_cached);
                            let _ = cache.set_folder_remote_cursor(folder_id, has_more, "");
                        }
                        commit_res
                    }
                    Err(e) => Err(e),
                }
            }
            Err(e) => Err(e),
        },
    };

    match sync_res {
        Ok(()) => {
            if let Ok(page) = cache.list_messages_page(folder_id, "", 0, page_size) {
                let _ = evt_tx.send(WorkerEvent::MessagesPageLoaded(page));
            }
            if let Ok(folders) = cache.list_folders(account.id) {
                let _ = evt_tx.send(WorkerEvent::FoldersLoaded {
                    account_id: account.id,
                    folders,
                });
            }
            let _ = evt_tx.send(WorkerEvent::Status {
                account_id: Some(account.id),
                text: format!("[{}] 同步完成", account.email),
            });
        }
        Err(e) => {
            let _ = evt_tx.send(WorkerEvent::Error {
                account_id: Some(account.id),
                message: format!("[{}] {e}", account.email),
            });
        }
    }
}

/// 任务四：单文件夹历史批次失败信息（游标未推进，错误呈现由调用方决定）。
struct FolderBatchError {
    account_id: u32,
    account_email: String,
    message: String,
}

/// 任务四：单文件夹单批历史拉取（双协议共用，前台分页与后台索引都走这里）。
/// 读游标 → 载账号 → 拉取 → 提交缓存与游标 → 发 `FoldersLoaded` 刷新侧栏。
/// 返回 `Ok(None)` = 无需拉取（游标缺失 / 已无更早 / 账号不在允许范围）；
/// `Ok(Some((条数, 是否仍有更早)))` = 批次已提交；`Err` = 失败且游标未推进。
async fn fetch_older_batch_for_folder(
    data_dir: &PathBuf,
    cache: &mut CacheStore,
    tokens: &SharedTokenCache,
    fid: u32,
    batch: usize,
    enforce_unified_scope: bool,
    uid_snapshot: &mut Option<(u32, Vec<u32>)>,
    evt_tx: &EventSender,
) -> Result<Option<(usize, bool)>, FolderBatchError> {
    let Some((account_id, folder_path, has_more_flag, next_link, min_cached_uid, _)) =
        cache.folder_remote_cursor(fid).ok().flatten()
    else {
        return Ok(None);
    };
    if !has_more_flag {
        return Ok(None);
    }
    let Ok(account) = load_account_config(data_dir, account_id) else {
        return Ok(None);
    };
    // folder_id = 0 的统一视图路径与后台索引只覆盖 `enabled && in_unified` 的账号；
    // 指定 folder_id 的独立文件夹浏览不受限（退出统一视图的账号继续独立同步）。
    if enforce_unified_scope && !is_unified_scope_account(&account) {
        return Ok(None);
    }

    let res: Result<(usize, bool), String> = match account.auth_backend {
        AuthBackend::MicrosoftGraphOAuth2 => {
            if next_link.is_empty() {
                let _ = cache.set_folder_remote_cursor(fid, false, "");
                Ok((0, false))
            } else {
                match ensure_graph_token(tokens, &account).await {
                    Ok(token) => {
                        let url = next_link.clone();
                        let fetched = tokio::task::spawn_blocking(move || {
                            graph_client::fetch_folder_headers_next(&token, &url)
                        })
                        .await
                        .unwrap_or_else(|e| Err(format!("Graph 拉取历史邮件任务异常: {e}")));
                        match fetched {
                            Ok((items, new_next)) => {
                                let count = items.len();
                                let more = new_next.is_some();
                                match cache.commit_folder_headers_with_remote(
                                    account.id, fid, 1, &items, None,
                                ) {
                                    Ok(()) => {
                                        let _ = cache.set_folder_remote_cursor(
                                            fid,
                                            more,
                                            new_next.as_deref().unwrap_or(""),
                                        );
                                        Ok((count, more))
                                    }
                                    Err(e) => Err(e),
                                }
                            }
                            Err(e) => Err(e),
                        }
                    }
                    Err(e) => Err(e),
                }
            }
        }
        AuthBackend::Password => match min_cached_uid {
            None => {
                let _ = cache.set_folder_remote_cursor(fid, false, "");
                // 游标已收口（无更早历史），与 Graph 空游标分支同样视为完成。
                Ok((0, false))
            }
            Some(before_uid) => match secrets::load_account_secrets(account.id) {
                Ok(sec) => match imap_client::fetch_older_folder_headers(
                    &account,
                    &sec.imap_password,
                    &folder_path,
                    before_uid,
                    cache
                        .folder_meta(fid)
                        .ok()
                        .flatten()
                        .map(|(_, validity)| validity)
                        .unwrap_or(0),
                    batch,
                    uid_snapshot,
                )
                .await
                {
                    Ok((uid_validity, headers, has_more)) => {
                        let count = headers.len();
                        match cache.commit_folder_headers(
                            account.id,
                            fid,
                            uid_validity,
                            &headers,
                            None,
                        ) {
                            Ok(()) => {
                                let _ = cache.set_folder_remote_cursor(fid, has_more, "");
                                Ok((count, has_more))
                            }
                            Err(e) => Err(e),
                        }
                    }
                    Err(e) if e == "UIDVALIDITY_CHANGED" => {
                        let background_events = EventSender::noop();
                        sync_folder_inner(
                            cache,
                            tokens,
                            &account,
                            fid,
                            &folder_path,
                            batch,
                            &background_events,
                        )
                        .await;
                        Ok((0, true))
                    }
                    Err(e) => Err(e),
                },
                Err(e) => Err(e),
            },
        },
    };

    match res {
        Ok((count, more)) => {
            if let Ok(folders) = cache.list_folders(account.id) {
                let _ = evt_tx.send(WorkerEvent::FoldersLoaded {
                    account_id: account.id,
                    folders,
                });
            }
            Ok(Some((count, more)))
        }
        Err(e) => Err(FolderBatchError {
            account_id: account.id,
            account_email: account.email.clone(),
            message: e,
        }),
    }
}

/// Fetch the next older batch (up to `limit`, default 100) from the remote server for `folder_id`
/// (`0` = all unified-scope inbox folders that still have older remote messages).
async fn load_older_messages_inner(
    data_dir: &PathBuf,
    cache: &mut CacheStore,
    tokens: &SharedTokenCache,
    folder_id: u32,
    limit: usize,
    evt_tx: &EventSender,
) {
    let batch = if limit == 0 { 100 } else { limit };
    let targets: Vec<u32> = if folder_id == 0 {
        cache
            .list_inbox_folders_with_more_remote()
            .unwrap_or_default()
            .into_iter()
            .map(|(fid, _, _)| fid)
            .collect()
    } else {
        vec![folder_id]
    };

    let mut total_fetched = 0usize;
    let mut any_more_remote = false;

    for fid in targets {
        match fetch_older_batch_for_folder(
            data_dir,
            cache,
            tokens,
            fid,
            batch,
            // 任务四：folder_id = 0 的统一分页只覆盖 `enabled && in_unified` 账号。
            folder_id == 0,
            &mut None,
            evt_tx,
        )
        .await
        {
            Ok(Some((count, more))) => {
                total_fetched += count;
                if more {
                    any_more_remote = true;
                }
            }
            Ok(None) => {}
            Err(err) => {
                let _ = evt_tx.send(WorkerEvent::Error {
                    account_id: Some(err.account_id),
                    message: format!("[{}] 加载更早邮件失败: {}", err.account_email, err.message),
                });
            }
        }
    }

    let _ = evt_tx.send(WorkerEvent::OlderMessagesLoaded {
        folder_id,
        fetched_count: total_fetched,
        has_more_remote: any_more_remote,
    });
}

/// 任务四：后台历史索引的会话内状态（worker 进程生命周期；重启自动清零，
/// 持久化游标续跑，对齐 Linux 本会话放弃语义）。
#[derive(Default)]
struct BackfillIndexState {
    /// folder_id -> 连续失败次数（成功一批即清零）。
    failures: HashMap<u32, u32>,
    /// 连续失败达 [`BACKFILL_HISTORY_TRIES`] 的文件夹：本次会话不再索引。
    abandoned: std::collections::HashSet<u32>,
    /// 上一批处理的文件夹：候选轮转的起点，避免大文件夹饿死其他账号。
    last_folder: Option<u32>,
    uid_snapshots: HashMap<u32, Option<(u32, Vec<u32>)>>,
}

impl BackfillIndexState {
    /// 登记一次批次失败；连续失败达 [`BACKFILL_HISTORY_TRIES`] 时放弃该文件夹
    /// 本次会话（游标保留），返回是否刚刚被放弃。
    fn record_failure(&mut self, folder_id: u32) -> bool {
        let count = self
            .failures
            .entry(folder_id)
            .and_modify(|c| *c += 1)
            .or_insert(1);
        if *count >= BACKFILL_HISTORY_TRIES {
            self.abandoned.insert(folder_id);
            self.failures.remove(&folder_id);
            return true;
        }
        false
    }

    /// 登记一次批次成功：清零该文件夹的连续失败计数。
    fn record_success(&mut self, folder_id: u32) {
        self.failures.remove(&folder_id);
    }
}

/// 任务四：从缓存候选中选出下一个后台索引批次目标。过滤账号不在统一视图
/// 范围（停用 / 退出统一视图）与本会话已放弃的文件夹；候选按文件夹 ID 升序，
/// 从 `last_folder` 之后轮转，避免大文件夹饿死其他账号。
fn select_backfill_target(
    candidates: &[(u32, u32, String)],
    allowed: &std::collections::HashSet<u32>,
    state: &BackfillIndexState,
) -> Option<u32> {
    let in_scope: Vec<u32> = candidates
        .iter()
        .filter(|(_, account_id, _)| allowed.contains(account_id))
        .filter(|(fid, _, _)| !state.abandoned.contains(fid))
        .map(|(fid, _, _)| *fid)
        .collect();
    match state.last_folder {
        Some(last) => in_scope
            .iter()
            .copied()
            .find(|fid| *fid > last)
            .or_else(|| in_scope.first().copied()),
        None => in_scope.first().copied(),
    }
}

/// 任务四：后台历史索引批次。每批只处理一个收件箱文件夹的
/// [`BACKFILL_HISTORY_BATCH`] 条；由 worker_loop 放到独立任务里执行（自带
/// SQLite 连接），不阻塞交互请求队列。完成后发 `BackfillHistoryBatchDone`，
/// 由 app 端决定是否续排下一批。失败时游标保留，连续失败 3 次放弃该文件夹
/// 本次会话并日志说明（不打扰界面）。
async fn backfill_history_batch_inner(
    data_dir: &PathBuf,
    cache: &mut CacheStore,
    tokens: &SharedTokenCache,
    state: &mut BackfillIndexState,
    evt_tx: &EventSender,
) {
    let accounts = config::load_accounts_in(data_dir).unwrap_or_default();
    let allowed: std::collections::HashSet<u32> = accounts
        .iter()
        .filter(|a| a.enabled)
        .map(|a| a.id)
        .collect();
    let folders = cache.list_backfill_folders().unwrap_or_default();
    let candidates: Vec<_> = folders
        .iter()
        .map(|(id, account, path, _)| (*id, *account, path.clone()))
        .collect();
    let Some(fid) = select_backfill_target(&candidates, &allowed, state) else {
        // 已无待索引目标（全部拉完 / 放弃 / 范围外）：通知 app 端停止续排。
        let _ = evt_tx.send(WorkerEvent::BackfillHistoryBatchDone {
            folder_id: 0,
            fetched_count: 0,
            body_only: false,
            has_more_remote: false,
        });
        return;
    };
    state.last_folder = Some(fid);

    if let Some((_, account_id, path, true)) = folders.iter().find(|(id, _, _, _)| *id == fid) {
        if let Some(account) = accounts.iter().find(|a| a.id == *account_id && a.enabled) {
            let background_events = EventSender::noop();
            sync_folder_inner(
                cache,
                tokens,
                account,
                fid,
                path,
                BACKFILL_HISTORY_BATCH,
                &background_events,
            )
            .await;
            if let Ok(folders) = cache.list_folders(account.id) {
                let _ = evt_tx.send(WorkerEvent::FoldersLoaded {
                    account_id: account.id,
                    folders,
                });
            }
        }
        let still_uninitialized = cache
            .list_backfill_folders()
            .unwrap_or_default()
            .iter()
            .any(|(id, _, _, uninitialized)| *id == fid && *uninitialized);
        if still_uninitialized {
            state.record_failure(fid);
        } else {
            state.record_success(fid);
        }
        let remaining = cache.list_backfill_folders().unwrap_or_default();
        let candidates: Vec<_> = remaining
            .into_iter()
            .map(|(id, account, path, _)| (id, account, path))
            .collect();
        let _ = evt_tx.send(WorkerEvent::BackfillHistoryBatchDone {
            folder_id: fid,
            fetched_count: 1,
            body_only: false,
            has_more_remote: select_backfill_target(&candidates, &allowed, state).is_some(),
        });
        return;
    }

    // 一次请求在同一 IMAP 会话内批量补一批正文（最多 10 封），落盘成功后由界面续排下一批。
    let has_older = cache
        .folder_remote_cursor(fid)
        .ok()
        .flatten()
        .is_some_and(|(_, _, more, _, _, _)| more);
    let missing = if has_older {
        Vec::new()
    } else {
        cache.list_unindexed_messages(fid, 10).unwrap_or_default()
    };
    if !missing.is_empty() {
        let Some((_, account_id, path, _)) = folders.iter().find(|(id, _, _, _)| *id == fid) else {
            return;
        };
        let Some(account) = accounts.iter().find(|a| a.id == *account_id && a.enabled) else {
            return;
        };
        let validity = cache
            .folder_meta(fid)
            .ok()
            .flatten()
            .map(|(_, validity)| validity)
            .unwrap_or(0);
        let results: Vec<(u32, Result<(String, Option<FetchedMessageBody>), String>)> =
            match account.auth_backend {
                AuthBackend::Password => match secrets::load_account_secrets(account.id) {
                    Ok(sec) => {
                        let uids: Vec<u32> = missing.iter().map(|(_, uid, _)| *uid).collect();
                        match imap_client::fetch_index_texts_batch(
                            account,
                            &sec.imap_password,
                            path,
                            &uids,
                            validity,
                        )
                        .await
                        {
                            Ok(batch) => {
                                let mut by_uid: std::collections::HashMap<
                                    u32,
                                    (String, Option<FetchedMessageBody>),
                                > = batch
                                    .into_iter()
                                    .map(|(uid, text, full)| (uid, (text, full)))
                                    .collect();
                                missing
                                    .into_iter()
                                    .map(|(message_id, uid, _)| {
                                        let result = match by_uid.remove(&uid) {
                                            Some(pair) => Ok(pair),
                                            None => Err("服务器未返回索引正文".to_string()),
                                        };
                                        (message_id, result)
                                    })
                                    .collect()
                            }
                            Err(e) => missing
                                .into_iter()
                                .map(|(message_id, _, _)| (message_id, Err(e.clone())))
                                .collect(),
                        }
                    }
                    Err(e) => missing
                        .into_iter()
                        .map(|(message_id, _, _)| (message_id, Err(e.clone())))
                        .collect(),
                },
                AuthBackend::MicrosoftGraphOAuth2 => {
                    match ensure_graph_token(tokens, account).await {
                        Ok(token) => {
                            let mut out = Vec::new();
                            for (message_id, _uid, remote_id) in missing {
                                if remote_id.is_empty() {
                                    out.push((message_id, Err("缺少 Graph 邮件标识".to_string())));
                                    continue;
                                }
                                let token = token.clone();
                                let result = tokio::task::spawn_blocking(move || {
                                    graph_client::fetch_index_text(&token, &remote_id)
                                })
                                .await
                                .unwrap_or_else(|e| Err(format!("Graph 索引任务异常: {e}")));
                                out.push((message_id, result));
                            }
                            out
                        }
                        Err(e) => missing
                            .into_iter()
                            .map(|(message_id, _, _)| (message_id, Err(e.clone())))
                            .collect(),
                    }
                }
            };
        let mut stored_count = 0usize;
        let mut needs_resync = false;
        for (message_id, result) in &results {
            match result {
                Ok((_text, Some(full))) => {
                    if cache
                        .store_message_body_with_html(
                            *message_id,
                            &full.body_text,
                            full.body_html.as_deref(),
                            &full.attachments,
                            full.unsubscribe_url.as_deref(),
                            full.unsubscribe_one_click,
                            full.auth_results.as_deref(),
                        )
                        .is_ok()
                    {
                        stored_count += 1;
                    }
                }
                Ok((text, None)) => {
                    if cache.store_index_text(*message_id, text).is_ok() {
                        stored_count += 1;
                    }
                }
                Err(e) if e == "UIDVALIDITY_CHANGED" || e == "服务器未返回索引正文" => {
                    needs_resync = true;
                }
                Err(e) => {
                    eprintln!("后台正文索引失败（文件夹 {fid}）: {e}");
                }
            }
        }
        if needs_resync {
            state.uid_snapshots.remove(&fid);
            let background_events = EventSender::noop();
            sync_folder_inner(
                cache,
                tokens,
                account,
                fid,
                path,
                BACKFILL_HISTORY_BATCH,
                &background_events,
            )
            .await;
        }
        if stored_count > 0 {
            state.record_success(fid);
        } else {
            state.record_failure(fid);
        }
        let remaining: Vec<_> = cache
            .list_backfill_folders()
            .unwrap_or_default()
            .into_iter()
            .map(|(id, account, path, _)| (id, account, path))
            .collect();
        let _ = evt_tx.send(WorkerEvent::BackfillHistoryBatchDone {
            folder_id: fid,
            fetched_count: stored_count,
            body_only: true,
            has_more_remote: select_backfill_target(&remaining, &allowed, state).is_some(),
        });
        return;
    }

    let outcome = fetch_older_batch_for_folder(
        data_dir,
        cache,
        tokens,
        fid,
        BACKFILL_HISTORY_BATCH,
        false,
        state.uid_snapshots.entry(fid).or_default(),
        evt_tx,
    )
    .await;

    let exclude_current = matches!(outcome, Ok(None));
    let mut fetched_count = 0usize;
    match outcome {
        Ok(Some((0, true))) => {
            // 空批次但服务端仍声称有更早历史：无实质进展，按失败计数防止紧循环空转。
            if state.record_failure(fid) {
                eprintln!(
                    "后台历史索引: 文件夹 {fid} 连续 {} 次空批次，本次会话放弃（游标保留，重启后自动续跑）",
                    BACKFILL_HISTORY_TRIES
                );
            }
        }
        Ok(Some((count, _more))) => {
            state.record_success(fid);
            fetched_count = count;
            if count > 0 {
                eprintln!("后台历史索引: 文件夹 {fid} 本批入库 {count} 封");
            }
        }
        Ok(None) => {
            // 游标缺失 / 账号已停用或退出统一视图：无工作量，游标保持不动。
        }
        Err(err) => {
            if state.record_failure(fid) {
                eprintln!(
                    "后台历史索引: 文件夹 {fid} 连续失败 {} 次，本次会话放弃（游标保留，重启后自动续跑）: {}",
                    BACKFILL_HISTORY_TRIES, err.message
                );
            } else {
                eprintln!(
                    "后台历史索引: 文件夹 {fid} 本批失败，游标保留待重试: {}",
                    err.message
                );
            }
        }
    }

    // 批次后是否仍有待索引目标（已过滤范围外与放弃的文件夹）。本批无实质进展
    // （游标缺失 / 账号停用或退出统一视图）时把该文件夹从回信中排除，避免 app
    // 与 worker 对同一候选空转；下一次触发（如账号重新启用）会重新纳入。
    let remaining: Vec<_> = cache
        .list_backfill_folders()
        .unwrap_or_default()
        .into_iter()
        .map(|(id, account, path, _)| (id, account, path))
        .collect();
    let has_more_remote = select_backfill_target(&remaining, &allowed, state)
        .is_some_and(|next| !exclude_current || next != fid);
    let _ = evt_tx.send(WorkerEvent::BackfillHistoryBatchDone {
        folder_id: fid,
        fetched_count,
        body_only: false,
        has_more_remote,
    });
}

/// 启动恢复（对齐 Linux 退出落盘语义）：扫描发件箱中已到期的 Queued 记录直接进入
/// 投递；Sending / Uncertain / Failed 保持不动，避免重复投递或让用户误以为已投递。
/// 停用账号的记录保持 Queued 排队，等重新启用后由 `SetAccountEnabled` 触发续发。
/// 定时发送巡检的最长间隔：没有待发记录时，主循环仍每 30 秒醒一次。
const OUTBOX_TICK_MAX: Duration = Duration::from_secs(30);
/// 巡检的最短间隔：到期却没能投递（账号停用等）的记录会把「下次到期时间」压在
/// 过去，给个下限避免主循环空转。
const OUTBOX_TICK_MIN: Duration = Duration::from_secs(1);

/// WP-10-5：主循环本次该等多久——等到下一条定时记录到期，最多
/// [`OUTBOX_TICK_MAX`]。
fn next_outbox_wait(cache: &CacheStore) -> Duration {
    let now = chrono::Utc::now().timestamp();
    let wait = match cache.next_queued_send_at(now) {
        // 等到那一刻；已过期却没发出去的记录（例如账号被停用）不会出现在这里，
        // 于是自然退回固定间隔，避免空转查库。
        Ok(Some(at)) => Duration::from_secs((at - now).max(1) as u64),
        _ => OUTBOX_TICK_MAX,
    };
    wait.clamp(OUTBOX_TICK_MIN, OUTBOX_TICK_MAX)
}

/// WP-10-5：主循环每次醒来做一次到期巡检。只有确实存在到期记录时才读账号配置
/// 并投递，避免每 30 秒白读一次磁盘。
async fn deliver_due_outbox_tick(
    data_dir: &PathBuf,
    cache: &mut CacheStore,
    tokens: &SharedTokenCache,
    evt_tx: &EventSender,
) {
    let now = chrono::Utc::now().timestamp();
    let has_due = cache
        .list_due_queued_ids(None, now)
        .map(|ids| !ids.is_empty())
        .unwrap_or(false);
    if !has_due {
        return;
    }
    let accounts = config::load_accounts_in(data_dir).unwrap_or_default();
    deliver_due_outbox(data_dir, cache, tokens, &accounts, evt_tx).await;
    for acc in accounts.iter().filter(|a| a.enabled) {
        if let Ok(items) = cache.list_outbox(acc.id) {
            let _ = evt_tx.send(WorkerEvent::OutboxUpdated {
                account_id: acc.id,
                items,
            });
        }
    }
}

async fn deliver_due_outbox(
    data_dir: &PathBuf,
    cache: &mut CacheStore,
    tokens: &SharedTokenCache,
    accounts: &[AccountConfig],
    evt_tx: &EventSender,
) {
    let now = chrono::Utc::now().timestamp();
    let enabled: std::collections::HashSet<u32> = accounts
        .iter()
        .filter(|a| a.enabled)
        .map(|a| a.id)
        .collect();
    let due = cache.list_due_queued_ids(None, now).unwrap_or_default();
    for (account_id, outbox_id) in due {
        if !enabled.contains(&account_id) {
            continue;
        }
        send_outbox_item(data_dir, cache, tokens, account_id, outbox_id, evt_tx).await;
    }
}

/// 自动投递路径（SendDraft 立即发送 / 启动恢复 / 重新启用账号续发）：
/// 仅领取仍处于 Queued 且已到期的记录，已被领取或未到期则静默跳过。
async fn send_outbox_item(
    data_dir: &PathBuf,
    cache: &mut CacheStore,
    tokens: &SharedTokenCache,
    account_id: u32,
    outbox_id: u32,
    evt_tx: &EventSender,
) {
    let now = chrono::Utc::now().timestamp();
    let item = match cache.claim_outbox_for_delivery(outbox_id, now) {
        Ok(Some(item)) => item,
        Ok(None) => return,
        Err(e) => {
            let _ = evt_tx.send(WorkerEvent::Error {
                account_id: Some(account_id),
                message: e,
            });
            return;
        }
    };
    deliver_outbox_item(data_dir, cache, tokens, account_id, outbox_id, item, evt_tx).await;
}

/// 用户显式触发路径（手动重发 / 撤销倒计时到期的立即发送）：
/// Queued / Failed 均可领取，绕过到期检查。
async fn retry_outbox_item(
    data_dir: &PathBuf,
    cache: &mut CacheStore,
    tokens: &SharedTokenCache,
    account_id: u32,
    outbox_id: u32,
    evt_tx: &EventSender,
) {
    match load_account_config(data_dir, account_id) {
        Ok(account) if account.enabled => {}
        Ok(_) => {
            let _ = evt_tx.send(WorkerEvent::Error {
                account_id: Some(account_id),
                message: "账号已停用，无法重试发送".to_string(),
            });
            return;
        }
        Err(e) => {
            let _ = evt_tx.send(WorkerEvent::Error {
                account_id: Some(account_id),
                message: e.to_string(),
            });
            return;
        }
    }
    let item = match cache.claim_outbox_for_retry(outbox_id) {
        Ok(Some(item)) => item,
        Ok(None) => return,
        Err(e) => {
            let _ = evt_tx.send(WorkerEvent::Error {
                account_id: Some(account_id),
                message: e,
            });
            return;
        }
    };
    deliver_outbox_item(data_dir, cache, tokens, account_id, outbox_id, item, evt_tx).await;
}

/// Determine whether an SMTP-sent message needs an explicit IMAP `APPEND (\Seen)`
/// to the Sent folder and resolve its target path.
fn resolve_imap_sent_append_target(
    account: &AccountConfig,
    discovered_sent: Option<String>,
) -> Option<String> {
    if account.auth_backend != AuthBackend::Password || account.special_folders.server_saves_sent {
        return None;
    }
    let configured = account.special_folders.sent.trim();
    if !configured.is_empty() {
        return Some(configured.to_string());
    }
    discovered_sent.filter(|p| !p.trim().is_empty())
}

/// 投递一条已领取（Sending）的发件箱记录。
async fn deliver_outbox_item(
    data_dir: &PathBuf,
    cache: &mut CacheStore,
    tokens: &SharedTokenCache,
    account_id: u32,
    outbox_id: u32,
    item: OutboxItem,
    evt_tx: &EventSender,
) {
    let Ok(account) = load_account_config(data_dir, account_id) else {
        let _ = cache.update_outbox_status(outbox_id, OutboxStatus::Failed, "缺少账号配置");
        return;
    };

    // 停用账号：恢复 Queued 保持不动（不标 Failed），保留发件身份与正文，
    // 等重新启用账号后自动续发。
    if !account.enabled {
        let _ = cache.update_outbox_status(outbox_id, OutboxStatus::Queued, "账号已停用，暂缓投递");
        if let Ok(outbox) = cache.list_outbox(account_id) {
            let _ = evt_tx.send(WorkerEvent::OutboxUpdated {
                account_id,
                items: outbox,
            });
        }
        return;
    }

    if let Ok(outbox) = cache.list_outbox(account_id) {
        let _ = evt_tx.send(WorkerEvent::OutboxUpdated {
            account_id,
            items: outbox,
        });
    }

    let (outcome, sent_raw_rfc822, imap_password_for_sent) = match account.auth_backend {
        AuthBackend::MicrosoftGraphOAuth2 => match ensure_graph_token(tokens, &account).await {
            Ok(token) => {
                let to_addr = item.to_addr.clone();
                let cc_addrs = item.cc_addr.clone();
                let bcc_addrs = item.bcc_addr.clone();
                let reply_to = item.reply_to.clone();
                let subject = item.subject.clone();
                let body = item.body_text.clone();
                let body_html = item.body_html.clone();
                let atts = item.attachment_paths.clone();
                let out = tokio::task::spawn_blocking(move || {
                    graph_client::send_message(
                        &token,
                        &to_addr,
                        &cc_addrs,
                        &bcc_addrs,
                        (!reply_to.trim().is_empty()).then_some(reply_to.as_str()),
                        &subject,
                        &body,
                        body_html.as_deref(),
                        &atts,
                    )
                })
                .await
                .unwrap_or_else(|e| SendOutcome::Uncertain(format!("Graph 发信任务异常: {e}")));
                (out, None, None)
            }
            Err(e) => (SendOutcome::Failed(e), None, None),
        },
        AuthBackend::Password => match secrets::load_account_secrets(account_id) {
            Ok(secrets) => {
                let smtp_pwd = if account.smtp_separate_credentials {
                    secrets
                        .smtp_password
                        .as_deref()
                        .unwrap_or(&secrets.imap_password)
                } else {
                    secrets.imap_password.as_str()
                };
                let (out, raw) = smtp_client::send_message_with_raw(
                    &account,
                    smtp_pwd,
                    &item.to_addr,
                    &item.cc_addr,
                    &item.bcc_addr,
                    &item.subject,
                    &item.body_text,
                    &item.in_reply_to,
                    &item.from_addr,
                    &item.reply_to,
                    item.body_html.as_deref(),
                    &item.attachment_paths,
                )
                .await;
                (out, raw, Some(secrets.imap_password))
            }
            Err(e) => (SendOutcome::Failed(e), None, None),
        },
    };

    match outcome {
        SendOutcome::Accepted => {
            let _ = cache.delete_outbox(outbox_id);
            let _ = evt_tx.send(WorkerEvent::DraftSent {
                account_id,
                subject: item.subject,
            });
            // 移植方案阶段 2：服务器确认接收后，保存已发副本属于独立操作；
            // 若 `server_saves_sent` 为 true（Gmail/Outlook/iCloud 等）则跳过 APPEND 避免重复；
            // 若 APPEND 失败仅发温和状态提示，绝不将已发邮件标记为失败或重发。
            if let (Some(raw), Some(imap_pwd)) = (sent_raw_rfc822, imap_password_for_sent) {
                let discovered_sent = cache
                    .find_sent_folder(account_id)
                    .ok()
                    .flatten()
                    .map(|(_, path)| path);
                if let Some(sent_path) = resolve_imap_sent_append_target(&account, discovered_sent)
                {
                    if let Err(e) =
                        imap_client::append_sent_message(&account, &imap_pwd, &sent_path, &raw)
                            .await
                    {
                        let _ = evt_tx.send(WorkerEvent::Status {
                            account_id: Some(account_id),
                            text: format!(
                                "[{}] 邮件已发出，但保存已发副本失败：{e}",
                                account.email
                            ),
                        });
                    }
                }
            }
        }
        SendOutcome::Failed(err) => {
            let _ = cache.update_outbox_status(outbox_id, OutboxStatus::Failed, &err);
            let _ = evt_tx.send(WorkerEvent::Error {
                account_id: Some(account_id),
                message: format!("[{}] {err}", account.email),
            });
        }
        SendOutcome::Uncertain(err) => {
            let _ = cache.update_outbox_status(outbox_id, OutboxStatus::Uncertain, &err);
            let _ = evt_tx.send(WorkerEvent::Error {
                account_id: Some(account_id),
                message: format!("[{}] {err}", account.email),
            });
        }
    }

    if let Ok(outbox) = cache.list_outbox(account_id) {
        let _ = evt_tx.send(WorkerEvent::OutboxUpdated {
            account_id,
            items: outbox,
        });
    }
}

fn merge_filter_options(
    filter: MessageFilter,
    mut quick_filters: QuickFilterSet,
) -> QuickFilterSet {
    match filter {
        MessageFilter::All => quick_filters,
        MessageFilter::Unread => {
            quick_filters.unread = true;
            quick_filters
        }
        MessageFilter::Starred => {
            quick_filters.starred = true;
            quick_filters
        }
    }
}

/// F-7 RFC 8058 一键退订 POST（阻塞，须在后台线程执行）：
/// 以 `application/x-www-form-urlencoded` 提交 `List-Unsubscribe=One-Click`，
/// 仅 2xx 视为退订成功（对齐原版 `unsubscribe::action::one_click_post`）。
fn unsubscribe_one_click_post(url: &str) -> Result<(), String> {
    if !url
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("https://")
    {
        return Err("退订句柄不是 https 地址，无法执行一键退订".to_string());
    }
    let agent = ureq::AgentBuilder::new().redirects(0).build();
    let status = match agent
        .post(url)
        .set("User-Agent", "Hylki/0.1")
        .timeout(std::time::Duration::from_secs(20))
        .send_form(&[("List-Unsubscribe", "One-Click")])
    {
        Ok(resp) => resp.status(),
        Err(ureq::Error::Status(code, _)) => code,
        Err(e) => return Err(e.to_string()),
    };
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(format!("退订服务器返回 HTTP {status}"))
    }
}

/// Search remote IMAP / Graph servers for `query` with per-account timeout protection,
/// committing matched headers to the local cache so the user can open any hit immediately.
async fn search_server_inner(
    data_dir: &PathBuf,
    cache: &mut CacheStore,
    tokens: &SharedTokenCache,
    raw_query: &str,
    target_account_id: Option<u32>,
    evt_tx: &EventSender,
) {
    let parsed = SearchQuery::parse(raw_query);
    let search_text = {
        let s = parsed.server_search_text();
        if s.is_empty() {
            raw_query.trim().to_string()
        } else {
            s
        }
    };
    if search_text.chars().count() < 2 {
        let _ = evt_tx.send(WorkerEvent::ServerSearchResults {
            query: raw_query.to_string(),
            items: Vec::new(),
        });
        return;
    }

    let accounts: Vec<AccountConfig> = config::load_accounts_in(data_dir)
        .unwrap_or_default()
        .into_iter()
        .filter(|a| a.enabled && target_account_id.is_none_or(|id| a.id == id))
        .collect();

    if accounts.is_empty() {
        let _ = evt_tx.send(WorkerEvent::ServerSearchResults {
            query: raw_query.to_string(),
            items: Vec::new(),
        });
        return;
    }

    let _ = evt_tx.send(WorkerEvent::Status {
        account_id: target_account_id,
        text: format!("正在服务端在线搜索「{search_text}」..."),
    });

    let mut all_hits: Vec<MessageSummary> = Vec::new();
    let mut seen_keys: std::collections::HashSet<(u32, u32)> = std::collections::HashSet::new();

    for account in accounts {
        let inbox = cache.find_inbox_folder(account.id).ok().flatten();
        let Some((inbox_id, inbox_path)) = inbox else {
            continue;
        };

        match account.auth_backend {
            AuthBackend::MicrosoftGraphOAuth2 => {
                let Ok(token) = ensure_graph_token(tokens, &account).await else {
                    continue;
                };
                let q = search_text.clone();
                let res = tokio::task::spawn_blocking(move || {
                    graph_client::search_messages(&token, &q, 50)
                })
                .await
                .unwrap_or_else(|e| Err(format!("Graph 搜索任务异常: {e}")));

                if let Ok(items) = res {
                    let uids: Vec<u32> = items.iter().map(|(h, _)| h.uid).collect();
                    let _ = cache
                        .commit_folder_headers_with_remote(account.id, inbox_id, 1, &items, None);
                    if let Ok(summaries) = cache.list_summaries_by_uids(inbox_id, &uids) {
                        for item in summaries {
                            if seen_keys.insert((item.account_id, item.uid)) {
                                all_hits.push(item);
                            }
                        }
                    }
                }
            }
            AuthBackend::Password => {
                let Ok(sec) = secrets::load_account_secrets(account.id) else {
                    continue;
                };
                let res = imap_client::search_folder_messages(
                    &account,
                    &sec.imap_password,
                    &inbox_path,
                    &search_text,
                    50,
                )
                .await;
                if let Ok(headers) = res {
                    let uids: Vec<u32> = headers.iter().map(|h| h.uid).collect();
                    let uid_validity = cache
                        .folder_meta(inbox_id)
                        .ok()
                        .flatten()
                        .map(|(_, v)| v)
                        .unwrap_or(1)
                        .max(1);
                    let _ = cache.commit_folder_headers(
                        account.id,
                        inbox_id,
                        uid_validity,
                        &headers,
                        None,
                    );
                    if let Ok(summaries) = cache.list_summaries_by_uids(inbox_id, &uids) {
                        for item in summaries {
                            if seen_keys.insert((item.account_id, item.uid)) {
                                all_hits.push(item);
                            }
                        }
                    }
                }
            }
        }
    }

    all_hits.sort_by(|a, b| b.date_ts.cmp(&a.date_ts).then_with(|| b.uid.cmp(&a.uid)));
    let count = all_hits.len();
    let _ = evt_tx.send(WorkerEvent::ServerSearchResults {
        query: raw_query.to_string(),
        items: all_hits,
    });
    let _ = evt_tx.send(WorkerEvent::Status {
        account_id: target_account_id,
        text: format!("服务端搜索完成，命中 {count} 封邮件"),
    });
}

fn load_account_config(data_dir: &PathBuf, account_id: u32) -> Result<AccountConfig, String> {
    let accounts = config::load_accounts_in(data_dir)?;
    accounts
        .into_iter()
        .find(|a| a.id == account_id)
        .ok_or_else(|| format!("找不到账号 ID {account_id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(id: u32, email: &str) -> AccountConfig {
        AccountConfig {
            id,
            email: email.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn graceful_quit_makes_queued_mail_due_and_stops_processing_requests() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hylki-quit-{}-{unique}", std::process::id()));
        let db_path = config::cache_db_in(&dir);
        let mut cache = CacheStore::open(&db_path).unwrap();
        let id = cache
            .enqueue_outbox(
                1,
                "to@example.com",
                "",
                "",
                "待发",
                "正文",
                "",
                &[],
                1,
                Some(i64::MAX),
            )
            .unwrap();
        drop(cache);

        let worker = MailWorkerHandle::spawn(dir.clone()).unwrap();
        worker.send_checked(MailRequest::PrepareQuit).unwrap();
        worker.send(MailRequest::DeleteOutboxItem {
            account_id: 1,
            outbox_id: id,
        });
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut prepared = false;
        loop {
            prepared |= worker
                .drain_events()
                .into_iter()
                .any(|event| matches!(event, WorkerEvent::QuitPrepared { result: Ok(()) }));
            if prepared && worker.send_checked(MailRequest::LoadAllCached).is_err() {
                break;
            }
            assert!(Instant::now() < deadline, "后台线程应在完成退出准备后停止");
            std::thread::sleep(Duration::from_millis(10));
        }

        let cache = CacheStore::open(&db_path).unwrap();
        assert_eq!(cache.list_due_queued_ids(None, 1).unwrap(), vec![(1, id)]);
        drop(cache);
        std::fs::remove_file(&db_path).unwrap();
        std::fs::remove_dir(&dir).unwrap();
    }

    #[test]
    fn oauth_reauthorize_same_email_binds_editing_account() {
        // 任务二：同邮箱重授权 -> 更新原账号（忽略大小写）。
        let existing = vec![account(3, "User@Outlook.com")];
        let resolved = resolve_oauth_account_match(Some(3), &existing, "user@outlook.com");
        assert_eq!(resolved, Ok(OAuthAccountMatch::Bind(3)));
    }

    #[test]
    fn oauth_reauthorize_different_email_is_rejected() {
        // 任务二：异邮箱重授权（无论是否命中其他账号）-> 保留原账号不动。
        let existing = vec![account(3, "a@outlook.com"), account(7, "b@outlook.com")];
        // 返回邮箱命中了另一个已有账号：仍拒绝换绑，避免把 A 的身份改成 B。
        assert_eq!(
            resolve_oauth_account_match(Some(3), &existing, "b@outlook.com"),
            Ok(OAuthAccountMatch::Rejected)
        );
        // 返回邮箱是本地没有的新邮箱：同样拒绝换绑。
        assert_eq!(
            resolve_oauth_account_match(Some(3), &existing, "new@gmail.com"),
            Ok(OAuthAccountMatch::Rejected)
        );
    }

    #[test]
    fn oauth_add_flow_rejects_existing_email_or_allocates_new_id() {
        // 添加账号流程：返回邮箱命中已有账号时报“已存在”（不静默覆盖），
        // 全新邮箱则分配新账号 ID。
        let existing = vec![account(3, "a@outlook.com")];
        assert_eq!(
            resolve_oauth_account_match(None, &existing, "A@OUTLOOK.COM"),
            Err("Account A@OUTLOOK.COM already exists".to_string())
        );
        assert_eq!(
            resolve_oauth_account_match(None, &existing, "fresh@outlook.com"),
            Ok(OAuthAccountMatch::Bind(4))
        );
    }

    /// 任务四：统一视图账号范围 = 已启用且加入统一收件箱。
    #[test]
    fn unified_scope_requires_enabled_and_in_unified() {
        let mut acc = account(1, "a@qq.com");
        assert!(is_unified_scope_account(&acc));
        acc.enabled = false;
        assert!(!is_unified_scope_account(&acc), "停用账号不在统一视图范围");
        acc.enabled = true;
        acc.in_unified = false;
        assert!(
            !is_unified_scope_account(&acc),
            "退出统一视图的账号不在范围"
        );
    }

    /// 任务四：连续失败达 BACKFILL_HISTORY_TRIES 次放弃文件夹本次会话；
    /// 成功一批即清零计数，不会误伤偶发失败的文件夹。
    #[test]
    fn backfill_gives_up_folder_after_consecutive_failures() {
        let mut state = BackfillIndexState::default();
        assert!(!state.record_failure(7), "第 1 次失败不放弃");
        assert!(!state.record_failure(7), "第 2 次失败不放弃");
        assert!(state.record_failure(7), "第 3 次连续失败放弃该文件夹");
        assert!(state.abandoned.contains(&7));

        // 放弃后不再计数；成功清零的文件夹重新从头计数。
        assert!(!state.record_failure(7), "已放弃的文件夹重新计数");
        state.record_success(7);
        assert!(!state.record_failure(7), "成功后计数从零开始");
        assert!(!state.record_failure(7));
        assert!(state.record_failure(7));
    }

    /// 任务四：批次目标在候选间轮转，跳过范围外账号与本会话放弃的文件夹。
    #[test]
    fn backfill_target_rotates_and_skips_out_of_scope_folders() {
        let mut state = BackfillIndexState::default();
        let candidates = vec![
            (1u32, 10u32, "INBOX".to_string()),
            (2, 11, "INBOX".to_string()),
            (3, 10, "INBOX".to_string()),
        ];
        // 账号 11 停用 / 退出统一视图：只有账号 10 的文件夹（1、3）在范围。
        let allowed: std::collections::HashSet<u32> = [10u32].into_iter().collect();

        assert_eq!(
            select_backfill_target(&candidates, &allowed, &state),
            Some(1)
        );
        state.last_folder = Some(1);
        assert_eq!(
            select_backfill_target(&candidates, &allowed, &state),
            Some(3)
        );
        state.last_folder = Some(3);
        // 轮转到末尾后回到队首（避免大文件夹独占后台索引）。
        assert_eq!(
            select_backfill_target(&candidates, &allowed, &state),
            Some(1)
        );

        // 放弃文件夹 1 后只剩 3；全部放弃 / 范围外时无目标。
        state.abandoned.insert(1);
        state.last_folder = Some(3);
        assert_eq!(
            select_backfill_target(&candidates, &allowed, &state),
            Some(3)
        );
        state.abandoned.insert(3);
        assert_eq!(select_backfill_target(&candidates, &allowed, &state), None);
    }

    #[test]
    fn resolve_imap_sent_append_target_respects_server_saves_sent_and_override() {
        let mut account = AccountConfig::default();
        account.auth_backend = AuthBackend::Password;
        account.special_folders.server_saves_sent = true;
        assert_eq!(
            resolve_imap_sent_append_target(&account, Some("Sent".to_string())),
            None
        );

        account.special_folders.server_saves_sent = false;
        assert_eq!(
            resolve_imap_sent_append_target(&account, Some("Sent".to_string())),
            Some("Sent".to_string())
        );

        account.special_folders.sent = "Custom/Sent".to_string();
        assert_eq!(
            resolve_imap_sent_append_target(&account, Some("Sent".to_string())),
            Some("Custom/Sent".to_string())
        );

        account.auth_backend = AuthBackend::MicrosoftGraphOAuth2;
        assert_eq!(
            resolve_imap_sent_append_target(&account, Some("Sent".to_string())),
            None
        );
    }

    #[test]
    fn shared_token_cache_inserts_reads_removes_and_isolates_refresh_locks() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let tokens = SharedTokenCache::default();
            tokens
                .insert(
                    1,
                    "valid-tok".to_string(),
                    Instant::now() + Duration::from_secs(300),
                )
                .await;
            tokens
                .insert(
                    2,
                    "expired-tok".to_string(),
                    Instant::now() - Duration::from_secs(1),
                )
                .await;

            assert_eq!(
                tokens.cached_valid_token(1).await.as_deref(),
                Some("valid-tok")
            );
            assert_eq!(tokens.cached_valid_token(2).await, None);

            let lock_1a = tokens.account_refresh_lock(1);
            let lock_1b = tokens.account_refresh_lock(1);
            let lock_2 = tokens.account_refresh_lock(2);
            assert!(Arc::ptr_eq(&lock_1a, &lock_1b));
            assert!(!Arc::ptr_eq(&lock_1a, &lock_2));

            tokens.remove(1).await;
            assert_eq!(tokens.cached_valid_token(1).await, None);
        });
    }
}
