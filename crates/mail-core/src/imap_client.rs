//! Async IMAP client supporting TLS and STARTTLS with MIME header/body parsing.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use async_imap::imap_proto::types::{
    BodyContentCommon, BodyContentSinglePart, BodyParams, BodyStructure, ContentEncoding,
    SectionPath,
};
use async_imap::types::NameAttribute;
use async_native_tls::TlsConnector;
use futures::StreamExt;
use mail_parser::{MessageParser, MimeHeaders};
use tokio::net::TcpStream;

use crate::cache::{CachedAttachmentInput, CachedHeaderInput, RemoteFolder};
use crate::config::{AccountConfig, SecurityMode};
use crate::{FetchedMessageBody, AUTO_CACHE_ATTACHMENT_MAX_BYTES};

const IO_TIMEOUT: Duration = Duration::from_secs(20);
/// 单封正文拉取的超时：附件较大的邮件给更宽的窗口，但必须封顶，
/// 否则卡住的连接会一直占住发信 / 读信请求的处理队列。
const BODY_FETCH_TIMEOUT: Duration = Duration::from_secs(60);
/// 按需下载大附件的超时窗口。
const ATTACHMENT_FETCH_TIMEOUT: Duration = Duration::from_secs(180);
/// 内联 `cid:` 图片允许自动随正文一并缓存的上限。
const MAX_INLINE_CID_BYTES: usize = 5 * 1024 * 1024;
const MAX_INITIAL_FETCH: usize = 100;

type TlsStream = async_native_tls::TlsStream<TcpStream>;
pub(crate) type ImapSession = async_imap::Session<TlsStream>;

/// XOAUTH2 SASL authenticator for async-imap (`AUTHENTICATE XOAUTH2`).
struct XOAuth2 {
    user: String,
    token: String,
    step: u8,
}

impl async_imap::Authenticator for XOAuth2 {
    type Response = Vec<u8>;
    fn process(&mut self, challenge: &[u8]) -> Self::Response {
        self.step += 1;
        if self.step == 1 {
            format!("user={}\x01auth=Bearer {}\x01\x01", self.user, self.token).into_bytes()
        } else {
            let _ = challenge;
            Vec::new()
        }
    }
}

async fn connect_imap_client(
    account: &AccountConfig,
) -> Result<async_imap::Client<TlsStream>, String> {
    if account.imap_host.trim().is_empty() {
        return Err("IMAP 主机地址不能为空".to_string());
    }
    let addr = (account.imap_host.as_str(), account.imap_port);
    let tcp = tokio::time::timeout(IO_TIMEOUT, TcpStream::connect(addr))
        .await
        .map_err(|_| {
            format!(
                "连接 IMAP 服务器 {}:{} 超时",
                account.imap_host, account.imap_port
            )
        })?
        .map_err(|e| format!("连接 IMAP 服务器失败: {e}"))?;

    let tls = TlsConnector::new();
    let client = match account.imap_security {
        SecurityMode::StartTls => {
            let mut plain = async_imap::Client::new(tcp);
            let _ = tokio::time::timeout(IO_TIMEOUT, plain.read_response()).await;
            tokio::time::timeout(IO_TIMEOUT, plain.run_command_and_check_ok("STARTTLS", None))
                .await
                .map_err(|_| "IMAP STARTTLS 握手超时".to_string())?
                .map_err(|e| format!("IMAP STARTTLS 失败: {e}"))?;
            let stream = tokio::time::timeout(
                IO_TIMEOUT,
                tls.connect(account.imap_host.as_str(), plain.into_inner()),
            )
            .await
            .map_err(|_| "IMAP TLS 升级超时".to_string())?
            .map_err(|e| format!("IMAP TLS 证书校验或握手失败: {e}"))?;
            async_imap::Client::new(stream)
        }
        SecurityMode::Tls => {
            let stream =
                tokio::time::timeout(IO_TIMEOUT, tls.connect(account.imap_host.as_str(), tcp))
                    .await
                    .map_err(|_| "IMAP TLS 握手超时".to_string())?
                    .map_err(|e| format!("IMAP TLS 证书校验或握手失败: {e}"))?;
            let mut c = async_imap::Client::new(stream);
            let _ = tokio::time::timeout(IO_TIMEOUT, c.read_response()).await;
            c
        }
    };

    Ok(client)
}

fn imap_username_for(account: &AccountConfig) -> &str {
    if account.imap_username.trim().is_empty() {
        account.email.as_str()
    } else {
        account.imap_username.as_str()
    }
}

async fn connect_imap(account: &AccountConfig, password: &str) -> Result<ImapSession, String> {
    let client = connect_imap_client(account).await?;
    let username = imap_username_for(account);

    let mut session = tokio::time::timeout(IO_TIMEOUT, client.login(username, password))
        .await
        .map_err(|_| "IMAP 登录超时".to_string())?
        .map_err(|(e, _)| {
            let msg = e.to_string();
            if msg.contains("Basic authentication is disabled") {
                "该邮箱服务商已禁用传统密码登录（Basic authentication is disabled），Outlook / Microsoft 365 需通过 OAuth2 授权登录；请先使用 QQ 邮箱、网易 163 或支持应用专用密码的邮箱测试。".to_string()
            } else {
                format!("IMAP 登录失败（请检查用户名或授权码）: {msg}")
            }
        })?;

    if is_netease_host(&account.imap_host) {
        send_netease_id_in_place(&mut session).await;
    }
    Ok(session)
}

/// Connect to an IMAP server using OAuth2 (`AUTHENTICATE XOAUTH2`).
pub async fn connect_imap_xoauth2(
    account: &AccountConfig,
    access_token: &str,
) -> Result<ImapSession, String> {
    if !account
        .imap_host
        .eq_ignore_ascii_case("outlook.office365.com")
    {
        return Err("Microsoft OAuth2 IMAP 仅允许连接 outlook.office365.com".to_string());
    }
    let client = connect_imap_client(account).await?;
    let auth = XOAuth2 {
        user: imap_username_for(account).to_string(),
        token: access_token.to_string(),
        step: 0,
    };
    let mut session = tokio::time::timeout(IO_TIMEOUT, client.authenticate("XOAUTH2", auth))
        .await
        .map_err(|_| "IMAP XOAUTH2 认证超时".to_string())?
        .map_err(|(e, _)| format!("IMAP XOAUTH2 认证失败: {e}"))?;

    if is_netease_host(&account.imap_host) {
        send_netease_id_in_place(&mut session).await;
    }
    Ok(session)
}

fn is_netease_host(host: &str) -> bool {
    let lower = host.trim().to_ascii_lowercase();
    lower.ends_with(".163.com")
        || lower.ends_with(".126.com")
        || lower.ends_with(".yeah.net")
        || lower.ends_with(".188.com")
}

/// NetEase (163/126/yeah.net) requires RFC 2971 `ID` identification on some accounts
/// before `SELECT`/`EXAMINE` is permitted. Best-effort and ignored if unsupported.
async fn send_netease_id_in_place(session: &mut ImapSession) {
    let _ = tokio::time::timeout(
        Duration::from_secs(5),
        session.run_command_and_check_ok(
            "ID (\"name\" \"KiteMail\" \"version\" \"1.0\" \"vendor\" \"KiteMail\")",
        ),
    )
    .await;
}

/// Result of one IMAP IDLE watch cycle on a mailbox.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdleWatchEvent {
    /// The server announced new/expunged mail or the mailbox state changed.
    MailboxChanged,
    /// The connection was lost or authentication failed and needs to be re-established.
    Disconnected,
}

/// Open an examine-only IMAP session on `folder_path` (e.g. `"INBOX"`) and enter
/// `IDLE` push mode. Whenever the server pushes `NewData` (or the unseen/exists count
/// changes after a periodic 60-second heartbeat), calls `on_changed()` so the worker
/// can immediately sync new messages.
///
/// Works with both Password IMAP (163, 126, QQ, Gmail, iCloud, Exmail, Aliyun, etc.)
/// and OAuth2 XOAUTH2 IMAP (`outlook.office365.com:993`, `imap.gmail.com:993`).
pub async fn run_idle_inbox_session<F>(
    mut session: ImapSession,
    folder_path: &str,
    mut on_changed: F,
) -> Result<(), String>
where
    F: FnMut(),
{
    const IDLE_INIT_TIMEOUT: Duration = Duration::from_secs(15);
    const IDLE_DONE_TIMEOUT: Duration = Duration::from_secs(10);
    // Keep each IDLE round to 120 seconds so NAT/firewalls don't drop silent TCP sockets,
    // and so servers that don't push flag changes still get a lightweight UIDNEXT/EXISTS check.
    const IDLE_ROUND_DURATION: Duration = Duration::from_secs(120);

    let initial_mb = tokio::time::timeout(IO_TIMEOUT, session.examine(folder_path))
        .await
        .map_err(|_| format!("IDLE 监听打开 {folder_path} 超时"))?
        .map_err(|e| format!("IDLE 监听打开 {folder_path} 失败: {e}"))?;

    let mut last_exists = initial_mb.exists;
    let mut last_uid_next = initial_mb.uid_next;

    loop {
        let mut handle = session.idle();
        match tokio::time::timeout(IDLE_INIT_TIMEOUT, handle.init()).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(format!("启动 IMAP IDLE 失败: {e}")),
            Err(_) => return Err("启动 IMAP IDLE 超时".to_string()),
        }

        let idle_res = {
            let (idle_fut, _stop) = handle.wait_with_timeout(IDLE_ROUND_DURATION);
            idle_fut.await
        };

        session = match tokio::time::timeout(IDLE_DONE_TIMEOUT, handle.done()).await {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => return Err(format!("结束 IMAP IDLE 握手失败: {e}")),
            Err(_) => return Err("结束 IMAP IDLE (DONE) 超时".to_string()),
        };

        let pushed_new_data = matches!(
            idle_res,
            Ok(async_imap::extensions::idle::IdleResponse::NewData(_))
        );

        // Re-examine to get fresh EXISTS / UIDNEXT and verify connection health.
        let mb = tokio::time::timeout(IO_TIMEOUT, session.examine(folder_path))
            .await
            .map_err(|_| "IDLE 状态校验超时".to_string())?
            .map_err(|e| format!("IDLE 状态校验失败: {e}"))?;

        let state_changed = mb.exists != last_exists || mb.uid_next != last_uid_next;
        last_exists = mb.exists;
        last_uid_next = mb.uid_next;

        if pushed_new_data || state_changed {
            on_changed();
        }
    }
}

/// Open an examine-only IMAP session using password credentials.
pub async fn connect_imap_password(
    account: &AccountConfig,
    password: &str,
) -> Result<ImapSession, String> {
    connect_imap(account, password).await
}

/// Verify IMAP connectivity and login credentials.
pub async fn verify_login(account: &AccountConfig, password: &str) -> Result<(), String> {
    let mut session = connect_imap(account, password).await?;
    let _ = session.logout().await;
    Ok(())
}

/// Fetch mailbox folder list from the IMAP server.
pub async fn fetch_folders(
    account: &AccountConfig,
    password: &str,
) -> Result<Vec<RemoteFolder>, String> {
    let mut session = connect_imap(account, password).await?;
    let mut stream = session
        .list(None, Some("*"))
        .await
        .map_err(|e| format!("拉取文件夹列表失败: {e}"))?;

    let mut raw_entries = Vec::new();
    while let Some(item) = stream.next().await {
        if let Ok(name) = item {
            let (no_select, special_role) = classify_imap_attributes(name.attributes());
            raw_entries.push(RawImapFolderEntry {
                path: name.name().to_string(),
                delimiter: name.delimiter().map(str::to_string),
                no_select,
                special_role,
            });
        }
    }
    drop(stream);
    let _ = session.logout().await;

    let mut folders = resolve_imap_folders(&raw_entries);
    if folders.is_empty() {
        folders.push(RemoteFolder {
            name: "收件箱".to_string(),
            path: "INBOX".to_string(),
            unread_count: 0,
            total_count: 0,
        });
    }
    Ok(folders)
}

async fn fetch_headers_for_uids(
    session: &mut ImapSession,
    target_uids: &[u32],
) -> Result<Vec<CachedHeaderInput>, String> {
    if target_uids.is_empty() {
        return Ok(Vec::new());
    }
    let uid_list = target_uids
        .iter()
        .map(|u| u.to_string())
        .collect::<Vec<_>>()
        .join(",");

    // 完整头部用于解析字段；单独保留有限全文前缀以生成预览。
    // PEEK 不改变服务器端的已读标记。
    let mut fetches = session
        .uid_fetch(
            &uid_list,
            "(UID FLAGS BODY.PEEK[HEADER] BODY.PEEK[]<0.4096>)",
        )
        .await
        .map_err(|e| format!("拉取邮件头失败: {e}"))?;

    let parser = MessageParser::default();
    let mut headers = Vec::with_capacity(target_uids.len());

    while let Some(fetch_res) = fetches.next().await {
        let fetch = fetch_res.map_err(|e| format!("读取邮件头失败: {e}"))?;
        if fetch.header().is_none() && fetch.body().is_none() {
            continue;
        }
        let Some(uid) = fetch.uid else {
            return Err("服务器返回的邮件头缺少 UID".to_string());
        };
        let mut unread = true;
        let mut flagged = false;
        for flag in fetch.flags() {
            match flag {
                async_imap::types::Flag::Seen => unread = false,
                async_imap::types::Flag::Flagged => flagged = true,
                _ => {}
            }
        }
        let raw = fetch
            .header()
            .ok_or_else(|| format!("服务器返回的邮件 {uid} 缺少头部"))?;
        let mut header = parse_header_bytes(&parser, uid, unread, flagged, raw);
        let prefix = fetch
            .body()
            .ok_or_else(|| format!("服务器返回的邮件 {uid} 缺少正文前缀"))?;
        header.preview = parse_header_bytes(&parser, uid, unread, flagged, prefix).preview;
        headers.push(header);
    }
    drop(fetches);
    validate_fetched_uids(target_uids, headers.iter().map(|h| h.uid))?;
    Ok(headers)
}

fn validate_fetched_uids(
    target: &[u32],
    received: impl Iterator<Item = u32>,
) -> Result<(), String> {
    let received: Vec<u32> = received.collect();
    let unique: HashSet<u32> = received.iter().copied().collect();
    if received.len() != target.len()
        || unique.len() != target.len()
        || target.iter().any(|uid| !unique.contains(uid))
    {
        return Err("邮件 UID 已变化，稍后重新读取".to_string());
    }
    Ok(())
}

/// Fetch recent message headers and flags for `folder_path`.
/// Returns `(uid_validity, headers, live_uids)`.
/// Server-side `FLAGGED` and `UNSEEN` UID sets returned alongside a folder sync
/// so older cached messages outside the header fetch window stay in sync.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FolderFlagSnapshot {
    pub flagged_uids: HashSet<u32>,
    pub unseen_uids: HashSet<u32>,
}

/// Fetch folder UIDVALIDITY, newest `MAX_INITIAL_FETCH` headers, and full live UID list.
pub async fn fetch_folder_headers(
    account: &AccountConfig,
    password: &str,
    folder_path: &str,
) -> Result<(u32, Vec<CachedHeaderInput>, Vec<u32>), String> {
    let (uid_validity, headers, live_uids, _) =
        fetch_folder_headers_with_flags(account, password, folder_path).await?;
    Ok((uid_validity, headers, live_uids))
}

/// Fetch folder UIDVALIDITY, newest `MAX_INITIAL_FETCH` headers, full live UID list,
/// and lightweight `UID SEARCH FLAGGED` / `UID SEARCH UNSEEN` snapshots.
pub async fn fetch_folder_headers_with_flags(
    account: &AccountConfig,
    password: &str,
    folder_path: &str,
) -> Result<
    (
        u32,
        Vec<CachedHeaderInput>,
        Vec<u32>,
        Option<FolderFlagSnapshot>,
    ),
    String,
> {
    fetch_folder_headers_incremental(account, password, folder_path, None).await
}

pub(crate) async fn fetch_folder_headers_incremental(
    account: &AccountConfig,
    password: &str,
    folder_path: &str,
    cached: Option<(u32, &HashSet<u32>)>,
) -> Result<
    (
        u32,
        Vec<CachedHeaderInput>,
        Vec<u32>,
        Option<FolderFlagSnapshot>,
    ),
    String,
> {
    let mut session = connect_imap(account, password).await?;
    let mailbox = session
        .select(folder_path)
        .await
        .map_err(|e| format!("打开文件夹 {folder_path} 失败: {e}"))?;

    let uid_validity = mailbox.uid_validity.unwrap_or(1);
    let uids_set = session
        .uid_search("ALL")
        .await
        .map_err(|e| format!("搜索邮件 UID 失败: {e}"))?;

    let mut live_uids: Vec<u32> = uids_set.into_iter().collect();
    live_uids.sort_unstable();

    if live_uids.is_empty() {
        let _ = session.logout().await;
        return Ok((
            uid_validity,
            Vec::new(),
            Vec::new(),
            Some(FolderFlagSnapshot::default()),
        ));
    }

    let flag_snapshot = match (
        tokio::time::timeout(IO_TIMEOUT, session.uid_search("FLAGGED")).await,
        tokio::time::timeout(IO_TIMEOUT, session.uid_search("UNSEEN")).await,
    ) {
        (Ok(Ok(flagged_uids)), Ok(Ok(unseen_uids))) => Some(FolderFlagSnapshot {
            flagged_uids,
            unseen_uids,
        }),
        _ => None,
    };

    let target_uids = select_sync_uids(&live_uids, uid_validity, cached, flag_snapshot.is_some());
    let headers = fetch_headers_for_uids(&mut session, &target_uids).await?;
    let _ = session.logout().await;

    Ok((uid_validity, headers, live_uids, flag_snapshot))
}

fn select_sync_uids(
    live: &[u32],
    validity: u32,
    cached: Option<(u32, &HashSet<u32>)>,
    flags_available: bool,
) -> Vec<u32> {
    let start = live.len().saturating_sub(MAX_INITIAL_FETCH);
    let Some((_, known)) =
        cached.filter(|(v, known)| *v == validity && *v != 0 && !known.is_empty())
    else {
        return live[start..].to_vec();
    };
    let min_known = known.iter().copied().min().unwrap_or(0);
    let mut selected: Vec<u32> = live
        .iter()
        .copied()
        .filter(|uid| *uid >= min_known && !known.contains(uid))
        .take(MAX_INITIAL_FETCH)
        .collect();
    for &uid in &live[start..] {
        if (!known.contains(&uid) || !flags_available) && !selected.contains(&uid) {
            selected.push(uid);
        }
    }
    selected
}

fn select_history_uids(live: &[u32], known: &HashSet<u32>, limit: usize) -> (Vec<u32>, bool) {
    let mut missing = live
        .iter()
        .rev()
        .copied()
        .filter(|uid| !known.contains(uid));
    let mut selected: Vec<u32> = missing.by_ref().take(limit.max(1)).collect();
    let more = missing.next().is_some();
    selected.reverse();
    (selected, more)
}

/// Fetch up to `limit` older message headers with `uid < before_uid` for `folder_path`.
/// Returns `(uid_validity, headers, live_uids, has_more_older)`.
pub async fn fetch_older_folder_headers(
    account: &AccountConfig,
    password: &str,
    folder_path: &str,
    before_uid: u32,
    expected_validity: u32,
    limit: usize,
    uid_snapshot: &mut Option<(u32, Vec<u32>)>,
) -> Result<(u32, Vec<CachedHeaderInput>, bool), String> {
    fetch_older_folder_headers_with_known(
        account,
        password,
        folder_path,
        before_uid,
        expected_validity,
        limit,
        uid_snapshot,
        None,
    )
    .await
}

pub(crate) async fn fetch_older_folder_headers_with_known(
    account: &AccountConfig,
    password: &str,
    folder_path: &str,
    before_uid: u32,
    expected_validity: u32,
    limit: usize,
    uid_snapshot: &mut Option<(u32, Vec<u32>)>,
    known: Option<&HashSet<u32>>,
) -> Result<(u32, Vec<CachedHeaderInput>, bool), String> {
    let mut session = connect_imap(account, password).await?;
    // 任务四：历史批次跑在后台，每一步网络调用都必须有限超时，
    // 避免一次卡住的服务器响应占住单队列。
    let mailbox = tokio::time::timeout(IO_TIMEOUT, session.select(folder_path))
        .await
        .map_err(|_| format!("打开文件夹 {folder_path} 超时"))?
        .map_err(|e| format!("打开文件夹 {folder_path} 失败: {e}"))?;

    let uid_validity = mailbox.uid_validity.unwrap_or(1);
    if expected_validity != 0 && uid_validity != expected_validity {
        *uid_snapshot = None;
        return Err("UIDVALIDITY_CHANGED".to_string());
    }
    if uid_snapshot
        .as_ref()
        .is_none_or(|(validity, _)| *validity != uid_validity)
    {
        let uids_set = tokio::time::timeout(IO_TIMEOUT, session.uid_search("ALL"))
            .await
            .map_err(|_| "搜索邮件 UID 超时".to_string())?
            .map_err(|e| format!("搜索邮件 UID 失败: {e}"))?;
        let mut uids: Vec<u32> = uids_set.into_iter().collect();
        uids.sort_unstable();
        *uid_snapshot = Some((uid_validity, uids));
    }
    let live_uids = &uid_snapshot.as_ref().unwrap().1;

    let (target_uids, has_more) = if let Some(known) = known {
        select_history_uids(live_uids, known, limit)
    } else {
        let end_idx = live_uids.partition_point(|uid| *uid < before_uid);
        let start_idx = end_idx.saturating_sub(limit.max(1));
        (live_uids[start_idx..end_idx].to_vec(), start_idx > 0)
    };
    if target_uids.is_empty() {
        let _ = tokio::time::timeout(IO_TIMEOUT, session.logout()).await;
        return Ok((uid_validity, Vec::new(), false));
    }
    let headers = match tokio::time::timeout(
        IO_TIMEOUT,
        fetch_headers_for_uids(&mut session, &target_uids),
    )
    .await
    {
        Ok(Ok(headers)) => headers,
        result => {
            *uid_snapshot = None;
            return Err(match result {
                Ok(Err(e)) => e,
                Err(_) => format!("拉取历史邮件头超时 ({folder_path})"),
                Ok(Ok(_)) => unreachable!(),
            });
        }
    };
    let _ = tokio::time::timeout(IO_TIMEOUT, session.logout()).await;

    Ok((uid_validity, headers, has_more))
}

/// 批量读取有体积上限的 MIME 内容用于本地搜索：在同一个 IMAP 会话内抓取
/// `uids` 一批（最多 10 封）邮件正文并统一提取文本后登出，避免逐封重建
/// TCP+TLS 连接触发服务器频控。返回 `(uid, 索引文本, 纯文本正文, HTML 正文)`，
/// 超过体积上限的邮件文本为空，供服务器搜索覆盖。
pub async fn fetch_index_texts_batch(
    account: &AccountConfig,
    password: &str,
    folder_path: &str,
    uids: &[u32],
    expected_validity: u32,
) -> Result<Vec<(u32, String, Option<FetchedMessageBody>)>, String> {
    const MAX_BYTES: usize = 262_144;
    if uids.is_empty() {
        return Ok(Vec::new());
    }
    let mut session = connect_imap(account, password).await?;
    let mailbox = tokio::time::timeout(IO_TIMEOUT, session.select(folder_path))
        .await
        .map_err(|_| "打开索引文件夹超时".to_string())?
        .map_err(|e| format!("打开索引文件夹失败: {e}"))?;
    if expected_validity != 0 && mailbox.uid_validity.unwrap_or(1) != expected_validity {
        return Err("UIDVALIDITY_CHANGED".to_string());
    }
    let uid_set = uids
        .iter()
        .map(|uid| uid.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let fetched = tokio::time::timeout(IO_TIMEOUT, async {
        let mut stream = session
            .uid_fetch(uid_set, "(BODY.PEEK[]<0.262145>)")
            .await
            .map_err(|e| format!("拉取索引正文失败: {e}"))?;
        let mut out = Vec::new();
        while let Some(item) = stream.next().await {
            let fetch = item.map_err(|e| format!("读取索引正文失败: {e}"))?;
            let Some(uid) = fetch.uid else { continue };
            let Some(body) = fetch.body() else { continue };
            out.push((uid, body.to_vec()));
        }
        Ok::<Vec<(u32, Vec<u8>)>, String>(out)
    })
    .await
    .map_err(|_| "拉取索引正文超时".to_string())??;
    let _ = tokio::time::timeout(IO_TIMEOUT, session.logout()).await;

    Ok(fetched
        .into_iter()
        .map(|(uid, bytes)| {
            if bytes.len() <= MAX_BYTES {
                if let Ok(full) = parse_rfc822_message(&bytes) {
                    let index_text = if !full.body_text.is_empty()
                        && full.body_text != "(此邮件无可读正文)"
                    {
                        full.body_text.clone()
                    } else if let Some(ref html) = full.body_html {
                        html_to_readable_text(html)
                    } else {
                        String::new()
                    };
                    return (uid, index_text, Some(full));
                }
            }
            (uid, String::new(), None)
        })
        .collect())
}

/// 将单封邮件的 MIME 字节转换为 `(索引文本, 纯文本正文, HTML 正文)`。
/// 超过 `max_bytes` 或无法解析时全部返回空，供服务器搜索覆盖。
#[cfg_attr(not(test), allow(dead_code))]
fn index_parts_from_mime(
    bytes: &[u8],
    max_bytes: usize,
) -> (String, Option<String>, Option<String>) {
    if bytes.len() > max_bytes {
        return (String::new(), None, None);
    }
    let Some(parsed) = MessageParser::default().parse(bytes) else {
        return (String::new(), None, None);
    };
    let text = parsed.body_text(0).map(|t| t.into_owned());
    let html = parsed.body_html(0).map(|h| h.into_owned());
    let index_text = match (&text, &html) {
        (Some(t), _) => t.clone(),
        (None, Some(h)) => html_to_readable_text(h),
        (None, None) => String::new(),
    };
    (index_text, text, html)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PlannedBodyPart {
    section: String,
    mime_type: String,
    charset: Option<String>,
    transfer_encoding: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PlannedInlineCid {
    section: String,
    cid: String,
    mime_type: String,
    transfer_encoding: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PlannedAttachment {
    section: String,
    filename: String,
    mime_type: String,
    estimated_size: usize,
    transfer_encoding: String,
    download_now: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct BodyStructureFetchPlan {
    body_parts: Vec<PlannedBodyPart>,
    inline_cids: Vec<PlannedInlineCid>,
    attachments: Vec<PlannedAttachment>,
}

impl BodyStructureFetchPlan {
    fn has_deferred_attachments(&self) -> bool {
        self.attachments.iter().any(|a| !a.download_now)
    }
}

fn estimated_decoded_size(
    common: &BodyContentCommon<'_>,
    other: &BodyContentSinglePart<'_>,
) -> usize {
    if let Some(size) = common
        .disposition
        .as_ref()
        .and_then(|d| body_param_value(&d.params, "size"))
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|&s| s > 0)
    {
        return size;
    }
    let octets = other.octets as usize;
    match other.transfer_encoding {
        ContentEncoding::Base64 => {
            // Base64 按 RFC 2045 每 76 字符插入一对 `\r\n`（约占 2/78）。
            let effective = octets.saturating_sub((octets / 78) * 2);
            effective / 4 * 3
        }
        _ => octets,
    }
}

fn encoding_str(enc: &ContentEncoding<'_>) -> String {
    match enc {
        ContentEncoding::SevenBit => "7bit".to_string(),
        ContentEncoding::EightBit => "8bit".to_string(),
        ContentEncoding::Binary => "binary".to_string(),
        ContentEncoding::Base64 => "base64".to_string(),
        ContentEncoding::QuotedPrintable => "quoted-printable".to_string(),
        ContentEncoding::Other(s) => s.to_string(),
    }
}

fn decode_mime_param(raw: &str) -> String {
    let cleaned = raw.replace(['\r', '\n'], "").trim().to_string();
    if !cleaned.contains("=?") {
        return cleaned;
    }
    let synthetic = format!("Subject: {cleaned}\r\n\r\n");
    MessageParser::default()
        .parse(synthetic.as_bytes())
        .and_then(|msg| msg.subject().map(str::to_string))
        .unwrap_or(cleaned)
}

fn body_param_value(params: &BodyParams<'_>, key: &str) -> Option<String> {
    params.as_ref()?.iter().find_map(|(k, v)| {
        k.eq_ignore_ascii_case(key)
            .then(|| decode_mime_param(v.as_ref()))
            .filter(|s| !s.is_empty())
    })
}

fn percent_decode_bytes(raw: &str) -> Vec<u8> {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(val) = u8::from_str_radix(hex, 16) {
                    out.push(val);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

fn decode_charset_octets(bytes: &[u8], charset: Option<&str>) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    let use_utf8 = charset.is_none_or(|cs| {
        let c = cs.trim();
        c.is_empty() || c.eq_ignore_ascii_case("utf-8") || c.eq_ignore_ascii_case("us-ascii")
    });
    let text = if use_utf8 {
        String::from_utf8_lossy(bytes).into_owned()
    } else {
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
        decode_text_section(b64.as_bytes(), "text/plain", charset, "base64", false)
            .unwrap_or_else(|| String::from_utf8_lossy(bytes).into_owned())
    };
    text.replace(['\r', '\n'], "").trim().to_string()
}

fn decode_rfc2231_param(params: &BodyParams<'_>, base_key: &str) -> Option<String> {
    let list = params.as_ref()?;
    let prefix = format!("{base_key}*");
    let mut segments: Vec<(usize, bool, &str)> = Vec::new();
    for (k, v) in list {
        let raw_k = k.as_ref();
        if raw_k.len() <= prefix.len() || !raw_k[..prefix.len()].eq_ignore_ascii_case(&prefix) {
            continue;
        }
        let suffix = &raw_k[prefix.len()..];
        let (digits, encoded) = match suffix.strip_suffix('*') {
            Some(d) => (d, true),
            None => (suffix, false),
        };
        if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
            if let Ok(idx) = digits.parse::<usize>() {
                segments.push((idx, encoded, v.as_ref()));
            }
        }
    }
    if !segments.is_empty() {
        segments.sort_by_key(|(idx, _, _)| *idx);
        let mut charset: Option<String> = None;
        let mut octets: Vec<u8> = Vec::new();
        for (pos, (_idx, encoded, val)) in segments.into_iter().enumerate() {
            if encoded {
                let raw_part = if pos == 0 {
                    let mut parts = val.splitn(3, '\'');
                    match (parts.next(), parts.next(), parts.next()) {
                        (Some(cs), Some(_lang), Some(rest)) => {
                            let cs = cs.trim();
                            if !cs.is_empty() {
                                charset = Some(cs.to_string());
                            }
                            rest
                        }
                        _ => val,
                    }
                } else {
                    val
                };
                octets.extend_from_slice(&percent_decode_bytes(raw_part));
            } else {
                octets.extend_from_slice(val.as_bytes());
            }
        }
        let decoded = decode_charset_octets(&octets, charset.as_deref());
        if !decoded.is_empty() {
            return Some(decoded);
        }
    }

    if let Some((_, v)) = list
        .iter()
        .find(|(k, _)| k.as_ref().eq_ignore_ascii_case(&prefix))
    {
        let val = v.as_ref();
        let mut parts = val.splitn(3, '\'');
        let (charset, rest) = match (parts.next(), parts.next(), parts.next()) {
            (Some(cs), Some(_lang), Some(rest)) => {
                let cs = cs.trim();
                ((!cs.is_empty()).then_some(cs), rest)
            }
            _ => (None, val),
        };
        let octets = percent_decode_bytes(rest);
        let decoded = decode_charset_octets(&octets, charset);
        if !decoded.is_empty() {
            return Some(decoded);
        }
    }

    body_param_value(params, base_key)
}

fn extract_bs_filename(
    common: &BodyContentCommon<'_>,
    fallback_idx: usize,
    default_ext: &str,
) -> String {
    common
        .disposition
        .as_ref()
        .and_then(|d| decode_rfc2231_param(&d.params, "filename"))
        .or_else(|| decode_rfc2231_param(&common.ty.params, "name"))
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("attachment-{fallback_idx}{default_ext}"))
}

fn plan_bodystructure_fetch(bs: &BodyStructure<'_>) -> BodyStructureFetchPlan {
    fn walk(bs: &BodyStructure<'_>, section: &str, plan: &mut BodyStructureFetchPlan) {
        match bs {
            BodyStructure::Multipart { bodies, .. } => {
                for (i, child) in bodies.iter().enumerate() {
                    let child_section = if section.is_empty() {
                        format!("{}", i + 1)
                    } else {
                        format!("{section}.{}", i + 1)
                    };
                    walk(child, &child_section, plan);
                }
            }
            BodyStructure::Text { common, other, .. } => {
                let is_att_disp = common
                    .disposition
                    .as_ref()
                    .is_some_and(|d| d.ty.eq_ignore_ascii_case("attachment"));
                let is_plain = common.ty.ty.eq_ignore_ascii_case("text")
                    && common.ty.subtype.eq_ignore_ascii_case("plain");
                let is_html = common.ty.ty.eq_ignore_ascii_case("text")
                    && common.ty.subtype.eq_ignore_ascii_case("html");
                let is_calendar = common.ty.ty.eq_ignore_ascii_case("text")
                    && common.ty.subtype.eq_ignore_ascii_case("calendar");
                let mime_type = format!(
                    "{}/{}",
                    common.ty.ty.to_ascii_lowercase(),
                    common.ty.subtype.to_ascii_lowercase()
                );

                if !is_att_disp && (is_plain || is_html) && !is_calendar {
                    plan.body_parts.push(PlannedBodyPart {
                        section: section.to_string(),
                        mime_type,
                        charset: body_param_value(&common.ty.params, "charset"),
                        transfer_encoding: encoding_str(&other.transfer_encoding),
                    });
                } else if is_att_disp || is_calendar {
                    let size = estimated_decoded_size(common, other);
                    let default_ext = if is_calendar { ".ics" } else { ".txt" };
                    let filename =
                        extract_bs_filename(common, plan.attachments.len() + 1, default_ext);
                    plan.attachments.push(PlannedAttachment {
                        section: section.to_string(),
                        filename,
                        mime_type,
                        estimated_size: size,
                        transfer_encoding: encoding_str(&other.transfer_encoding),
                        download_now: size <= AUTO_CACHE_ATTACHMENT_MAX_BYTES,
                    });
                }
            }
            BodyStructure::Basic { common, other, .. }
            | BodyStructure::Message { common, other, .. } => {
                let mime_type = format!(
                    "{}/{}",
                    common.ty.ty.to_ascii_lowercase(),
                    common.ty.subtype.to_ascii_lowercase()
                );
                if mime_type == "message/disposition-notification" {
                    return;
                }
                let is_att_disp = common
                    .disposition
                    .as_ref()
                    .is_some_and(|d| d.ty.eq_ignore_ascii_case("attachment"));
                let is_image = common.ty.ty.eq_ignore_ascii_case("image");
                let cid = other
                    .id
                    .as_deref()
                    .map(str::trim)
                    .map(|s| {
                        s.trim_matches(|c| c == '<' || c == '>')
                            .to_ascii_lowercase()
                    })
                    .filter(|s| !s.is_empty());
                let size = estimated_decoded_size(common, other);

                if let Some(cid) = cid {
                    if !is_att_disp && (is_image || mime_type == "application/octet-stream") {
                        if size <= MAX_INLINE_CID_BYTES {
                            plan.inline_cids.push(PlannedInlineCid {
                                section: section.to_string(),
                                cid,
                                mime_type,
                                transfer_encoding: encoding_str(&other.transfer_encoding),
                            });
                        }
                        return;
                    }
                }

                let default_ext = match mime_type.as_str() {
                    "message/rfc822" => ".eml",
                    _ => ".bin",
                };
                let filename = extract_bs_filename(common, plan.attachments.len() + 1, default_ext);
                plan.attachments.push(PlannedAttachment {
                    section: section.to_string(),
                    filename,
                    mime_type,
                    estimated_size: size,
                    transfer_encoding: encoding_str(&other.transfer_encoding),
                    download_now: size <= AUTO_CACHE_ATTACHMENT_MAX_BYTES,
                });
            }
        }
    }

    let mut plan = BodyStructureFetchPlan::default();
    let root_section = if matches!(bs, BodyStructure::Multipart { .. }) {
        ""
    } else {
        "1"
    };
    walk(bs, root_section, &mut plan);
    plan
}

fn section_path_for(section: &str) -> Option<SectionPath> {
    let nums: Option<Vec<u32>> = section
        .trim()
        .split('.')
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<u32>().ok())
        .collect();
    let nums = nums?;
    if nums.is_empty() {
        None
    } else {
        Some(SectionPath::Part(nums, None))
    }
}

fn decode_text_section(
    raw: &[u8],
    mime_type: &str,
    charset: Option<&str>,
    transfer_encoding: &str,
    is_html: bool,
) -> Option<String> {
    let mut mime_bytes = Vec::with_capacity(raw.len() + 128);
    let safe_ct = mime_type.replace(['\r', '\n'], "");
    if let Some(cs) = charset.filter(|s| !s.trim().is_empty()) {
        let safe_cs = cs.replace(['\r', '\n', '"'], "");
        mime_bytes.extend_from_slice(
            format!("Content-Type: {safe_ct}; charset=\"{safe_cs}\"\r\n").as_bytes(),
        );
    } else {
        mime_bytes.extend_from_slice(format!("Content-Type: {safe_ct}\r\n").as_bytes());
    }
    if !transfer_encoding.trim().is_empty() {
        let safe_te = transfer_encoding.replace(['\r', '\n'], "");
        mime_bytes
            .extend_from_slice(format!("Content-Transfer-Encoding: {safe_te}\r\n").as_bytes());
    }
    mime_bytes.extend_from_slice(b"\r\n");
    mime_bytes.extend_from_slice(raw);

    let parsed = MessageParser::default().parse(&mime_bytes)?;
    if is_html {
        parsed.body_html(0).map(|h| h.into_owned())
    } else {
        parsed.body_text(0).map(|t| t.into_owned())
    }
}

fn decode_binary_section(raw: &[u8], mime_type: &str, transfer_encoding: &str) -> Vec<u8> {
    let enc = transfer_encoding.trim().to_ascii_lowercase();
    if enc.is_empty() || matches!(enc.as_str(), "7bit" | "8bit" | "binary") {
        return raw.to_vec();
    }
    let mut mime_bytes = Vec::with_capacity(raw.len() + 128);
    let safe_ct = mime_type.replace(['\r', '\n'], "");
    let safe_te = transfer_encoding.replace(['\r', '\n'], "");
    mime_bytes.extend_from_slice(
        format!("Content-Type: {safe_ct}\r\nContent-Transfer-Encoding: {safe_te}\r\n\r\n")
            .as_bytes(),
    );
    mime_bytes.extend_from_slice(raw);
    MessageParser::default()
        .parse(&mime_bytes)
        .and_then(|parsed| parsed.parts.first().map(|p| p.contents().to_vec()))
        .filter(|bytes| !bytes.is_empty() || raw.is_empty())
        .unwrap_or_else(|| raw.to_vec())
}

fn parse_header_metadata(header_bytes: &[u8]) -> (Option<String>, bool, Option<String>) {
    if header_bytes.is_empty() {
        return (None, false, None);
    }
    let mut msg_bytes = Vec::with_capacity(header_bytes.len() + 4);
    msg_bytes.extend_from_slice(header_bytes);
    if !header_bytes.ends_with(b"\r\n\r\n") {
        msg_bytes.extend_from_slice(b"\r\n\r\n");
    }
    let Some(parsed) = MessageParser::default().parse(&msg_bytes) else {
        return (None, false, None);
    };
    let unsubscribe_url =
        raw_header_value(parsed.header_raw("List-Unsubscribe").unwrap_or_default())
            .and_then(crate::cache::extract_unsubscribe_url);
    let unsubscribe_one_click = raw_header_value(
        parsed
            .header_raw("List-Unsubscribe-Post")
            .unwrap_or_default(),
    )
    .map(crate::cache::is_one_click_post)
    .unwrap_or(false);
    let auth_results = raw_header_value(
        parsed
            .header_raw("Authentication-Results")
            .unwrap_or_default(),
    )
    .map(str::to_string)
    .filter(|v| !v.is_empty());
    (unsubscribe_url, unsubscribe_one_click, auth_results)
}

/// Parse complete RFC 822 message bytes into [`FetchedMessageBody`], caching all
/// attachments within the payload.
pub(crate) fn parse_rfc822_message(bytes: &[u8]) -> Result<FetchedMessageBody, String> {
    let parsed = MessageParser::default()
        .parse(bytes)
        .ok_or_else(|| "解析邮件 MIME 内容失败".to_string())?;

    let body_html = parsed
        .body_html(0)
        .map(|h| crate::mime::resolve_cid_inline_images(&parsed, h.into_owned()));
    let body_text = if let Some(text) = parsed.body_text(0) {
        text.into_owned()
    } else if let Some(ref html) = body_html {
        html_to_readable_text(html)
    } else {
        "(此邮件无可读正文)".to_string()
    };

    let mut attachments = Vec::new();
    for att in parsed.attachments() {
        let filename = att
            .attachment_name()
            .unwrap_or("attachment.bin")
            .to_string();
        let mime_type = att
            .content_type()
            .map(|ct| match ct.subtype() {
                Some(sub) => format!("{}/{}", ct.ctype(), sub),
                None => ct.ctype().to_string(),
            })
            .unwrap_or_else(|| "application/octet-stream".to_string());
        attachments.push(CachedAttachmentInput::downloaded(
            filename,
            mime_type,
            att.contents().to_vec(),
        ));
    }

    let unsubscribe_url =
        raw_header_value(parsed.header_raw("List-Unsubscribe").unwrap_or_default())
            .and_then(crate::cache::extract_unsubscribe_url);
    let unsubscribe_one_click = raw_header_value(
        parsed
            .header_raw("List-Unsubscribe-Post")
            .unwrap_or_default(),
    )
    .map(crate::cache::is_one_click_post)
    .unwrap_or(false);
    let auth_results = raw_header_value(
        parsed
            .header_raw("Authentication-Results")
            .unwrap_or_default(),
    )
    .map(str::to_string)
    .filter(|v| !v.is_empty());

    Ok(FetchedMessageBody {
        body_text,
        body_html,
        attachments,
        unsubscribe_url,
        unsubscribe_one_click,
        auth_results,
    })
}

/// Fetch message body and auto-cached attachments for `uid` in `folder_path` using
/// `BODY.PEEK` so server `\Seen` flags are only updated explicitly by `SetSeen`.
pub async fn fetch_message_body(
    account: &AccountConfig,
    password: &str,
    folder_path: &str,
    uid: u32,
) -> Result<FetchedMessageBody, String> {
    let mut session = connect_imap(account, password).await?;
    tokio::time::timeout(BODY_FETCH_TIMEOUT, session.select(folder_path))
        .await
        .map_err(|_| format!("打开文件夹 {folder_path} 超时"))?
        .map_err(|e| format!("打开文件夹 {folder_path} 失败: {e}"))?;

    // 先查 RFC822.SIZE + BODYSTRUCTURE：<= 2 MB 时整封拉取更省往返；
    // > 2 MB 时仅拉正文、内联图与 <= 2 MB 小附件，大附件只存元数据，点击时再按需下载。
    let probe = tokio::time::timeout(IO_TIMEOUT, async {
        let mut fetches = session
            .uid_fetch(uid.to_string(), "(RFC822.SIZE BODYSTRUCTURE)")
            .await
            .map_err(|e| format!("探测邮件结构失败 (UID {uid}): {e}"))?;
        let mut size_opt: Option<u32> = None;
        let mut plan_opt: Option<BodyStructureFetchPlan> = None;
        while let Some(fetch_res) = fetches.next().await {
            if let Ok(fetch) = fetch_res {
                if size_opt.is_none() {
                    size_opt = fetch.size;
                }
                if plan_opt.is_none() {
                    if let Some(bs) = fetch.bodystructure() {
                        plan_opt = Some(plan_bodystructure_fetch(bs));
                    }
                }
            }
        }
        Ok::<(Option<u32>, Option<BodyStructureFetchPlan>), String>((size_opt, plan_opt))
    })
    .await;

    let (size_opt, plan_opt) = match probe {
        Ok(Ok(pair)) => pair,
        _ => (None, None),
    };

    let fetch_full = size_opt.is_some_and(|s| (s as usize) <= AUTO_CACHE_ATTACHMENT_MAX_BYTES)
        || plan_opt
            .as_ref()
            .is_none_or(|p| p.body_parts.is_empty() || !p.has_deferred_attachments());

    if !fetch_full {
        if let Some(plan) = plan_opt {
            let mut body_and_att_items: Vec<String> = Vec::new();
            let mut seen_sections: HashSet<String> = HashSet::new();
            for s in plan
                .body_parts
                .iter()
                .map(|p| p.section.as_str())
                .chain(plan.inline_cids.iter().map(|c| c.section.as_str()))
                .chain(
                    plan.attachments
                        .iter()
                        .filter(|a| a.download_now)
                        .map(|a| a.section.as_str()),
                )
            {
                if seen_sections.insert(s.to_string()) {
                    body_and_att_items.push(format!("BODY.PEEK[{s}]"));
                }
            }
            let mut items_with_headers = vec![
                "BODY.PEEK[HEADER.FIELDS (LIST-UNSUBSCRIBE LIST-UNSUBSCRIBE-POST AUTHENTICATION-RESULTS)]"
                    .to_string(),
            ];
            items_with_headers.extend(body_and_att_items.iter().cloned());

            let fetch_has_any_body = |fetches: &[async_imap::types::Fetch]| -> bool {
                plan.body_parts.iter().any(|bp| {
                    fetches.iter().any(|f| {
                        section_path_for(&bp.section)
                            .and_then(|path| f.section(&path))
                            .or_else(|| (bp.section == "1").then(|| f.text()).flatten())
                            .is_some()
                    })
                })
            };

            let mut fetches = uid_fetch_sections(&mut session, uid, &items_with_headers)
                .await
                .ok()
                .unwrap_or_default();
            if !fetch_has_any_body(&fetches) {
                // 部分 IMAP 服务器不支持在复合 FETCH 中混合 HEADER.FIELDS 与分段路径，
                // 先降级为纯正文/内联图/小附件分段请求，避免直接整封下载数十 MB 邮件。
                if let Ok(plain_fetches) =
                    uid_fetch_sections(&mut session, uid, &body_and_att_items).await
                {
                    fetches = plain_fetches;
                }
            }

            if !fetches.is_empty() {
                let find_section = |section: &str| -> Option<&[u8]> {
                    let path = section_path_for(section);
                    fetches.iter().find_map(|f| {
                        path.as_ref()
                            .and_then(|p| f.section(p))
                            .or_else(|| (section == "1").then(|| f.text()).flatten())
                    })
                };
                let header_bytes = fetches.iter().find_map(|f| f.header()).unwrap_or(&[]);

                let mut plain_text: Option<String> = None;
                let mut html_body: Option<String> = None;
                for part in &plan.body_parts {
                    let Some(raw) = find_section(&part.section) else {
                        continue;
                    };
                    let is_html = part.mime_type == "text/html";
                    let decoded = decode_text_section(
                        raw,
                        &part.mime_type,
                        part.charset.as_deref(),
                        &part.transfer_encoding,
                        is_html,
                    );
                    if let Some(content) = decoded {
                        if is_html && html_body.is_none() {
                            html_body = Some(content);
                        } else if !is_html && plain_text.is_none() {
                            plain_text = Some(content);
                        }
                    }
                }

                if plain_text.is_some() || html_body.is_some() {
                    let mut cid_map: HashMap<String, String> = HashMap::new();
                    for inline in &plan.inline_cids {
                        let Some(raw) = find_section(&inline.section) else {
                            continue;
                        };
                        let bytes = decode_binary_section(
                            raw,
                            &inline.mime_type,
                            &inline.transfer_encoding,
                        );
                        if bytes.is_empty() {
                            continue;
                        }
                        use base64::Engine as _;
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                        cid_map.insert(
                            inline.cid.clone(),
                            format!("data:{};base64,{b64}", inline.mime_type),
                        );
                    }
                    let body_html = html_body.map(|h| crate::mime::replace_cid_urls(h, &cid_map));
                    let body_text = if let Some(text) = plain_text {
                        text
                    } else if let Some(ref html) = body_html {
                        html_to_readable_text(html)
                    } else {
                        "(此邮件无可读正文)".to_string()
                    };

                    let mut attachments = Vec::with_capacity(plan.attachments.len());
                    for att in &plan.attachments {
                        if att.download_now {
                            if let Some(raw) = find_section(&att.section) {
                                let bytes = decode_binary_section(
                                    raw,
                                    &att.mime_type,
                                    &att.transfer_encoding,
                                );
                                attachments.push(CachedAttachmentInput::downloaded(
                                    att.filename.clone(),
                                    att.mime_type.clone(),
                                    bytes,
                                ));
                                continue;
                            }
                        }
                        attachments.push(CachedAttachmentInput {
                            filename: att.filename.clone(),
                            mime_type: att.mime_type.clone(),
                            size_bytes: att.estimated_size,
                            content: None,
                            remote_part: att.section.clone(),
                            transfer_encoding: att.transfer_encoding.clone(),
                        });
                    }

                    let (unsubscribe_url, unsubscribe_one_click, auth_results) =
                        parse_header_metadata(header_bytes);
                    let _ = tokio::time::timeout(IO_TIMEOUT, session.logout()).await;
                    return Ok(FetchedMessageBody {
                        body_text,
                        body_html,
                        attachments,
                        unsubscribe_url,
                        unsubscribe_one_click,
                        auth_results,
                    });
                }
            }
        }
    }

    // 回退路径（<= 2 MB 小邮件、无 BODYSTRUCTURE 或按段拉取未解析出正文）：整封 PEEK 拉取。
    let raw_body = tokio::time::timeout(BODY_FETCH_TIMEOUT, async {
        let mut fetches = session
            .uid_fetch(uid.to_string(), "BODY.PEEK[]")
            .await
            .map_err(|e| format!("拉取邮件正文失败 (UID {uid}): {e}"))?;
        let mut raw_body: Option<Vec<u8>> = None;
        while let Some(fetch_res) = fetches.next().await {
            if let Ok(fetch) = fetch_res {
                if let Some(body) = fetch.body() {
                    raw_body = Some(body.to_vec());
                    break;
                }
            }
        }
        Ok::<Option<Vec<u8>>, String>(raw_body)
    })
    .await
    .map_err(|_| format!("拉取邮件正文超时 (UID {uid})"))??;
    let _ = tokio::time::timeout(IO_TIMEOUT, session.logout()).await;

    let bytes = raw_body.ok_or_else(|| format!("服务器未返回 UID {uid} 的邮件内容"))?;
    parse_rfc822_message(&bytes)
}

/// On-demand download of a single MIME attachment section via `BODY.PEEK[{section}]`,
/// falling back to `BODY.PEEK[]` filename matching when the section fetch fails.
pub async fn fetch_attachment_section(
    account: &AccountConfig,
    password: &str,
    folder_path: &str,
    uid: u32,
    section: &str,
    transfer_encoding: &str,
    fallback_filename: &str,
) -> Result<Vec<u8>, String> {
    let mut session = connect_imap(account, password).await?;
    tokio::time::timeout(IO_TIMEOUT, session.select(folder_path))
        .await
        .map_err(|_| format!("打开文件夹 {folder_path} 超时"))?
        .map_err(|e| format!("打开文件夹 {folder_path} 失败: {e}"))?;

    if !section.trim().is_empty() {
        if let Some(target_path) = section_path_for(section) {
            let query = format!("BODY.PEEK[{section}]");
            let fetched = tokio::time::timeout(ATTACHMENT_FETCH_TIMEOUT, async {
                let mut fetches = session
                    .uid_fetch(uid.to_string(), &query)
                    .await
                    .map_err(|e| format!("按段下载附件失败 (UID {uid}, section {section}): {e}"))?;
                let mut bytes_opt: Option<Vec<u8>> = None;
                while let Some(fetch_res) = fetches.next().await {
                    if let Ok(fetch) = fetch_res {
                        if let Some(raw) = fetch.section(&target_path) {
                            bytes_opt = Some(raw.to_vec());
                            break;
                        }
                    }
                }
                Ok::<Option<Vec<u8>>, String>(bytes_opt)
            })
            .await;

            if let Ok(Ok(Some(raw))) = fetched {
                let decoded =
                    decode_binary_section(&raw, "application/octet-stream", transfer_encoding);
                let _ = tokio::time::timeout(IO_TIMEOUT, session.logout()).await;
                return Ok(decoded);
            }
        }
    }

    let raw_full = tokio::time::timeout(ATTACHMENT_FETCH_TIMEOUT, async {
        let mut fetches = session
            .uid_fetch(uid.to_string(), "BODY.PEEK[]")
            .await
            .map_err(|e| format!("整封下载附件失败 (UID {uid}): {e}"))?;
        let mut raw_body: Option<Vec<u8>> = None;
        while let Some(fetch_res) = fetches.next().await {
            if let Ok(fetch) = fetch_res {
                if let Some(body) = fetch.body() {
                    raw_body = Some(body.to_vec());
                    break;
                }
            }
        }
        Ok::<Option<Vec<u8>>, String>(raw_body)
    })
    .await
    .map_err(|_| format!("下载附件超时 (UID {uid})"))??;
    let _ = tokio::time::timeout(IO_TIMEOUT, session.logout()).await;

    let bytes = raw_full.ok_or_else(|| format!("服务器未返回 UID {uid} 的邮件内容"))?;
    let parsed = MessageParser::default()
        .parse(&bytes)
        .ok_or_else(|| "解析附件所属 MIME 邮件失败".to_string())?;
    select_fallback_attachment(&parsed, fallback_filename)
        .ok_or_else(|| format!("未在邮件 UID {uid} 中找到附件 {fallback_filename}"))
}

async fn uid_fetch_sections(
    session: &mut ImapSession,
    uid: u32,
    items: &[String],
) -> Result<Vec<async_imap::types::Fetch>, String> {
    if items.is_empty() {
        return Ok(Vec::new());
    }
    let query = format!("({})", items.join(" "));
    tokio::time::timeout(BODY_FETCH_TIMEOUT, async {
        let mut fetches = session
            .uid_fetch(uid.to_string(), &query)
            .await
            .map_err(|e| format!("按段拉取邮件正文失败 (UID {uid}): {e}"))?;
        let mut collected = Vec::new();
        while let Some(item) = fetches.next().await {
            if let Ok(fetch) = item {
                collected.push(fetch);
            }
        }
        Ok::<Vec<async_imap::types::Fetch>, String>(collected)
    })
    .await
    .map_err(|_| format!("按段拉取邮件正文超时 (UID {uid})"))?
}

fn select_fallback_attachment(
    parsed: &mail_parser::Message<'_>,
    fallback_filename: &str,
) -> Option<Vec<u8>> {
    let all_atts: Vec<_> = parsed.attachments().collect();
    if let Some(found) = all_atts
        .iter()
        .find(|att| att.attachment_name() == Some(fallback_filename))
    {
        return Some(found.contents().to_vec());
    }
    let target_trim = fallback_filename.trim();
    if let Some(found) = all_atts.iter().find(|att| {
        att.attachment_name()
            .is_some_and(|name| name.trim().eq_ignore_ascii_case(target_trim))
    }) {
        return Some(found.contents().to_vec());
    }
    let target_sanitized = crate::cache::sanitize_filename(fallback_filename);
    if let Some(found) = all_atts.iter().find(|att| {
        att.attachment_name().is_some_and(|name| {
            crate::cache::sanitize_filename(name).eq_ignore_ascii_case(&target_sanitized)
        })
    }) {
        return Some(found.contents().to_vec());
    }
    if all_atts.len() == 1 {
        return Some(all_atts[0].contents().to_vec());
    }
    None
}

/// 从原始头字段文本（含 `字段名:` 前缀）中剥出值部分。
fn raw_header_value(raw: &str) -> Option<&str> {
    let idx = raw.find(':')?;
    Some(raw[idx + 1..].trim())
}

/// Fetch the complete raw RFC 822 bytes for `uid` in `folder_path` via
/// `UID FETCH (BODY.PEEK[])` — PEEK 不改变服务器端 `\Seen` 标记（.eml 导出后端）。
pub async fn fetch_raw_message(
    account: &AccountConfig,
    password: &str,
    folder_path: &str,
    uid: u32,
) -> Result<Vec<u8>, String> {
    let mut session = connect_imap(account, password).await?;
    session
        .select(folder_path)
        .await
        .map_err(|e| format!("打开文件夹 {folder_path} 失败: {e}"))?;

    let mut fetches = session
        .uid_fetch(uid.to_string(), "(BODY.PEEK[])")
        .await
        .map_err(|e| format!("拉取邮件原文失败 (UID {uid}): {e}"))?;

    let mut raw_body: Option<Vec<u8>> = None;
    while let Some(fetch_res) = fetches.next().await {
        if let Ok(fetch) = fetch_res {
            if let Some(body) = fetch.body() {
                raw_body = Some(body.to_vec());
                break;
            }
        }
    }
    drop(fetches);
    let _ = session.logout().await;

    raw_body.ok_or_else(|| format!("服务器未返回 UID {uid} 的原始邮件内容"))
}

/// Add or remove `\Seen` or `\Flagged` on the server for `uid`.
pub async fn set_flag(
    account: &AccountConfig,
    password: &str,
    folder_path: &str,
    uid: u32,
    flag: &str,
    add: bool,
) -> Result<(), String> {
    let mut session = connect_imap(account, password).await?;
    session
        .select(folder_path)
        .await
        .map_err(|e| format!("打开文件夹 {folder_path} 失败: {e}"))?;

    let op = if add {
        "+FLAGS.SILENT"
    } else {
        "-FLAGS.SILENT"
    };
    let query = format!("{op} ({flag})");
    let mut stream = session
        .uid_store(uid.to_string(), query)
        .await
        .map_err(|e| format!("同步邮件标记 {flag} 失败: {e}"))?;
    while stream.next().await.is_some() {}
    drop(stream);
    let _ = session.logout().await;
    Ok(())
}

/// Batch-mark every message in `folder_path` as `\Seen` on the server:
/// `UID SEARCH ALL` gathers the full uid set, then chunked
/// `UID STORE +FLAGS.SILENT (\Seen)` keeps each command line short.
pub async fn mark_folder_all_read(
    account: &AccountConfig,
    password: &str,
    folder_path: &str,
) -> Result<(), String> {
    let mut session = connect_imap(account, password).await?;
    session
        .select(folder_path)
        .await
        .map_err(|e| format!("打开文件夹 {folder_path} 失败: {e}"))?;
    let uids_set = session
        .uid_search("ALL")
        .await
        .map_err(|e| format!("搜索邮件 UID 失败: {e}"))?;
    let mut uids: Vec<u32> = uids_set.into_iter().collect();
    uids.sort_unstable();
    for chunk in uids.chunks(500) {
        let set = chunk
            .iter()
            .map(|u| u.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let mut stores = session
            .uid_store(&set, "+FLAGS.SILENT (\\Seen)")
            .await
            .map_err(|e| format!("批量标记已读失败: {e}"))?;
        while stores.next().await.is_some() {}
    }
    let _ = session.logout().await;
    Ok(())
}

/// Empty `folder_path` on the server (Trash / Junk): every message is marked
/// `\Deleted` via chunked `UID STORE`, then one `EXPUNGE` removes them all.
pub async fn empty_folder(
    account: &AccountConfig,
    password: &str,
    folder_path: &str,
) -> Result<(), String> {
    let mut session = connect_imap(account, password).await?;
    session
        .select(folder_path)
        .await
        .map_err(|e| format!("打开文件夹 {folder_path} 失败: {e}"))?;
    let uids_set = session
        .uid_search("ALL")
        .await
        .map_err(|e| format!("搜索邮件 UID 失败: {e}"))?;
    let mut uids: Vec<u32> = uids_set.into_iter().collect();
    uids.sort_unstable();
    for chunk in uids.chunks(500) {
        let set = chunk
            .iter()
            .map(|u| u.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let mut stores = session
            .uid_store(&set, "+FLAGS.SILENT (\\Deleted)")
            .await
            .map_err(|e| format!("批量标记删除失败: {e}"))?;
        while stores.next().await.is_some() {}
    }
    if !uids.is_empty() {
        {
            let mut expunged = std::pin::pin!(session
                .expunge()
                .await
                .map_err(|e| format!("执行 EXPUNGE 失败: {e}"))?);
            while expunged.as_mut().next().await.is_some() {}
        }
    }
    let _ = session.logout().await;
    Ok(())
}

/// Create a new mailbox `folder_path` on the IMAP server (`CREATE`).
pub async fn create_folder(
    account: &AccountConfig,
    password: &str,
    folder_path: &str,
) -> Result<(), String> {
    let mut session = connect_imap(account, password).await?;
    session
        .create(folder_path)
        .await
        .map_err(|e| format!("创建文件夹 {folder_path} 失败: {e}"))?;
    let _ = session.logout().await;
    Ok(())
}

/// Rename an existing mailbox using a new leaf name and the server hierarchy delimiter.
pub async fn rename_folder(
    account: &AccountConfig,
    password: &str,
    old_path: &str,
    new_name: &str,
) -> Result<Option<(String, Option<String>)>, String> {
    let mut session = connect_imap(account, password).await?;
    let mut stream = session
        .list(None, Some("*"))
        .await
        .map_err(|e| format!("拉取文件夹列表失败: {e}"))?;
    let mut delimiter = None;
    let mut found = false;
    while let Some(item) = stream.next().await {
        let entry = item.map_err(|e| format!("读取文件夹列表失败: {e}"))?;
        if entry.name() == old_path {
            delimiter = entry.delimiter().map(str::to_owned);
            found = true;
        }
    }
    drop(stream);
    if !found {
        return Err(format!("文件夹不存在: {old_path}"));
    }
    let new_path = renamed_folder_path(old_path, new_name, delimiter.as_deref());
    if new_path == old_path {
        let _ = session.logout().await;
        return Ok(None);
    }
    session
        .rename(old_path, &new_path)
        .await
        .map_err(|e| format!("重命名文件夹 {old_path} -> {new_path} 失败: {e}"))?;
    let _ = session.logout().await;
    Ok(Some((new_path, delimiter)))
}

fn renamed_folder_path(old_path: &str, new_name: &str, delimiter: Option<&str>) -> String {
    match delimiter
        .filter(|d| !d.is_empty())
        .and_then(|d| old_path.rfind(d).map(|i| (d, i)))
    {
        Some((d, i)) => format!("{}{}{}", &old_path[..i], d, new_name),
        None => new_name.to_string(),
    }
}

/// Delete a mailbox `folder_path` on the IMAP server (`DELETE`).
pub async fn delete_folder(
    account: &AccountConfig,
    password: &str,
    folder_path: &str,
) -> Result<(), String> {
    let mut session = connect_imap(account, password).await?;
    session
        .delete(folder_path)
        .await
        .map_err(|e| format!("删除文件夹 {folder_path} 失败: {e}"))?;
    let _ = session.logout().await;
    Ok(())
}

/// Move a message to another mailbox via `UID MOVE` (RFC 6851).
pub async fn move_message(
    account: &AccountConfig,
    password: &str,
    src_folder: &str,
    uid: u32,
    dest_folder: &str,
) -> Result<(), String> {
    if dest_folder.trim().is_empty() {
        return Err("目标文件夹路径不能为空".to_string());
    }
    let mut session = connect_imap(account, password).await?;
    session
        .select(src_folder)
        .await
        .map_err(|e| format!("打开文件夹 {src_folder} 失败: {e}"))?;
    session
        .uid_mv(uid.to_string(), dest_folder)
        .await
        .map_err(|e| format!("移动邮件到 {dest_folder} 失败: {e}"))?;
    let _ = session.logout().await;
    Ok(())
}

/// F-3 撤销：在 `current_folder` 中按 RFC Message-ID 头检索邮件并 `UID MOVE`
/// 移回 `original_folder`（移动 / 删除后的邮件已不在本地缓存，需服务端检索定位）。
pub async fn undo_move_by_message_id(
    account: &AccountConfig,
    password: &str,
    current_folder: &str,
    rfc_message_id: &str,
    original_folder: &str,
) -> Result<(), String> {
    let id = rfc_message_id.trim();
    if id.is_empty() {
        return Err("缺少 Message-ID，无法撤销移动".to_string());
    }
    let mut session = connect_imap(account, password).await?;
    session
        .select(current_folder)
        .await
        .map_err(|e| format!("打开文件夹 {current_folder} 失败: {e}"))?;
    let query = format!("HEADER Message-ID \"{}\"", escape_imap_search_arg(id));
    let uids = session
        .uid_search(&query)
        .await
        .map_err(|e| format!("检索邮件 Message-ID 失败: {e}"))?;
    // 多个命中时取最新（最大 UID），避免旧副本干扰。
    let uid = uids
        .into_iter()
        .max()
        .ok_or_else(|| "在当前文件夹中未找到该邮件，无法撤销移动".to_string())?;
    session
        .uid_mv(uid.to_string(), original_folder)
        .await
        .map_err(|e| format!("移动邮件回 {original_folder} 失败: {e}"))?;
    let _ = session.logout().await;
    Ok(())
}

fn supports_targeted_expunge<'a>(atoms: impl Iterator<Item = &'a str>) -> bool {
    atoms
        .into_iter()
        .any(|s| s.eq_ignore_ascii_case("UIDPLUS") || s.eq_ignore_ascii_case("IMAP4rev2"))
}

async fn require_targeted_expunge(session: &mut ImapSession) -> Result<(), String> {
    let caps = session
        .capabilities()
        .await
        .map_err(|e| format!("读取 IMAP 能力失败: {e}"))?;
    let atoms = caps.iter().filter_map(|cap| match cap {
        async_imap::types::Capability::Atom(atom) => Some(atom.as_str()),
        _ => None,
    });
    if supports_targeted_expunge(atoms) {
        Ok(())
    } else {
        Err("服务器不支持按 UID 清除单封邮件，操作已取消".to_string())
    }
}

async fn expunge_selected_uid<T>(
    session: &mut async_imap::Session<T>,
    uid: u32,
) -> Result<(), String>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    session
        .run_command_and_check_ok(format!("UID STORE {uid} +FLAGS.SILENT (\\Deleted)"))
        .await
        .map_err(|e| format!("标记邮件删除失败: {e}"))?;
    session
        .run_command_and_check_ok(format!("UID EXPUNGE {uid}"))
        .await
        .map_err(|e| format!("清除指定邮件失败: {e}"))
}

/// Permanently delete one message when the server has no dedicated trash folder.
pub async fn delete_message(
    account: &AccountConfig,
    password: &str,
    folder_path: &str,
    uid: u32,
) -> Result<(), String> {
    let mut session = connect_imap(account, password).await?;
    session
        .select(folder_path)
        .await
        .map_err(|e| format!("打开文件夹 {folder_path} 失败: {e}"))?;
    require_targeted_expunge(&mut session).await?;
    expunge_selected_uid(&mut session, uid).await?;
    let _ = session.logout().await;
    Ok(())
}

/// Append a sent RFC 822 message copy to `sent_folder_path` as an independent post-send step.
pub async fn append_sent_message(
    account: &AccountConfig,
    password: &str,
    sent_folder_path: &str,
    raw_rfc822: &[u8],
) -> Result<(), String> {
    let mut session = connect_imap(account, password).await?;
    tokio::time::timeout(
        IO_TIMEOUT,
        session.append(sent_folder_path, Some("(\\Seen)"), None, raw_rfc822),
    )
    .await
    .map_err(|_| format!("保存已发副本到 {sent_folder_path} 超时"))?
    .map_err(|e| format!("保存已发副本到 {sent_folder_path} 失败: {e}"))?;
    let _ = session.logout().await;
    Ok(())
}

/// Strip an attachment from message `uid` in `folder_path` on the IMAP server
/// (#289): fetches raw RFC822 + flags/internaldate, rewrites the MIME part to
/// `text/x-moz-deleted`, appends the rewritten copy, finds its new UID, and
/// expunges the original message.
pub async fn strip_imap_attachment(
    account: &AccountConfig,
    password: &str,
    folder_path: &str,
    uid: u32,
    name: &str,
    size: u64,
) -> Result<(Option<u32>, Vec<u8>), String> {
    let mut session = connect_imap(account, password).await?;
    let mailbox = session
        .select(folder_path)
        .await
        .map_err(|e| format!("打开文件夹 {folder_path} 失败: {e}"))?;
    let uid_next = mailbox
        .uid_next
        .ok_or_else(|| "服务器未提供 UIDNEXT，已取消剥离附件".to_string())?;
    require_targeted_expunge(&mut session).await?;

    let mut fetches = session
        .uid_fetch(uid.to_string(), "(UID FLAGS INTERNALDATE BODY.PEEK[])")
        .await
        .map_err(|e| format!("拉取原邮件失败 (UID {uid}): {e}"))?;

    let mut raw_body: Option<Vec<u8>> = None;
    let mut flags_str: Option<String> = None;
    let mut date_str: Option<String> = None;

    while let Some(fetch_res) = fetches.next().await {
        if let Ok(fetch) = fetch_res {
            if fetch.uid == Some(uid) {
                if let Some(body) = fetch.body() {
                    raw_body = Some(body.to_vec());
                }
                let kept: Vec<String> = fetch
                    .flags()
                    .filter_map(|f| match f {
                        async_imap::types::Flag::Seen => Some("\\Seen".to_string()),
                        async_imap::types::Flag::Answered => Some("\\Answered".to_string()),
                        async_imap::types::Flag::Flagged => Some("\\Flagged".to_string()),
                        async_imap::types::Flag::Draft => Some("\\Draft".to_string()),
                        async_imap::types::Flag::Custom(c) => Some(c.to_string()),
                        _ => None,
                    })
                    .collect();
                if !kept.is_empty() {
                    flags_str = Some(format!("({})", kept.join(" ")));
                }
                date_str = fetch
                    .internal_date()
                    .map(|d| format!("\"{}\"", d.format("%d-%b-%Y %H:%M:%S %z")));
            }
        }
    }
    drop(fetches);

    let raw = raw_body.ok_or_else(|| format!("服务器未返回 UID {uid} 的邮件内容"))?;
    let stripped = crate::strip::without_attachment(&raw, name, size, &crate::strip::note_date())
        .map_err(|e| e.text().to_string())?;

    tokio::time::timeout(
        IO_TIMEOUT,
        session.append(
            folder_path,
            flags_str.as_deref(),
            date_str.as_deref(),
            &stripped,
        ),
    )
    .await
    .map_err(|_| "上传剥离附件后的邮件副本超时".to_string())?
    .map_err(|e| format!("上传剥离附件后的邮件副本失败: {e}"))?;

    // APPEND succeeded. Only replace the local UID after identifying the exact
    // rewritten bytes among messages created since SELECT.
    let uncertain = "服务器已接收新副本，但无法确认其 UID。已保留原邮件，请刷新后检查副本。";
    let mut sizes = session
        .uid_fetch(format!("{uid_next}:*"), "(UID RFC822.SIZE)")
        .await
        .map_err(|_| uncertain.to_string())?;
    let mut candidates = Vec::new();
    while let Some(item) = sizes.next().await {
        let fetch = item.map_err(|_| uncertain.to_string())?;
        if let Some(candidate) = fetch.uid.filter(|&u| u >= uid_next && u != uid) {
            if fetch.size.map(u64::from) == Some(stripped.len() as u64) {
                candidates.push(candidate);
            }
        }
    }
    drop(sizes);

    let mut new_uid = None;
    for candidate in candidates {
        let mut bodies = session
            .uid_fetch(candidate.to_string(), "(UID BODY.PEEK[])")
            .await
            .map_err(|_| uncertain.to_string())?;
        while let Some(item) = bodies.next().await {
            let fetch = item.map_err(|_| uncertain.to_string())?;
            if fetch.uid == Some(candidate) && fetch.body() == Some(stripped.as_slice()) {
                if new_uid.replace(candidate).is_some() {
                    return Err(uncertain.to_string());
                }
            }
        }
    }
    if new_uid.is_none() {
        return Err(uncertain.to_string());
    }
    expunge_selected_uid(&mut session, uid).await?;
    let _ = session.logout().await;
    Ok((new_uid, stripped))
}

/// Escape a string for an IMAP quoted search argument.
pub(crate) fn escape_imap_search_arg(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace(['\r', '\n'], " ")
}

/// Build an IMAP `UID SEARCH` query string matching `query` in subject or message text.
pub(crate) fn build_imap_search_query(query: &str) -> String {
    let escaped = escape_imap_search_arg(query.trim());
    if query.is_ascii() {
        format!("OR SUBJECT \"{escaped}\" TEXT \"{escaped}\"")
    } else {
        format!("CHARSET UTF-8 OR SUBJECT \"{escaped}\" TEXT \"{escaped}\"")
    }
}

/// Search messages on the IMAP server in `folder_path` using `UID SEARCH`,
/// returning headers for up to `limit` newest matching UIDs.
pub async fn search_folder_messages(
    account: &AccountConfig,
    password: &str,
    folder_path: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<CachedHeaderInput>, String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let search_timeout = Duration::from_secs(15);
    let mut session = tokio::time::timeout(search_timeout, connect_imap(account, password))
        .await
        .map_err(|_| "IMAP 服务器搜索连接超时".to_string())??;

    tokio::time::timeout(search_timeout, session.select(folder_path))
        .await
        .map_err(|_| format!("打开文件夹 {folder_path} 超时"))?
        .map_err(|e| format!("打开文件夹 {folder_path} 失败: {e}"))?;

    let imap_query = build_imap_search_query(trimmed);
    let uids_res = tokio::time::timeout(search_timeout, session.uid_search(&imap_query))
        .await
        .map_err(|_| "IMAP 服务器搜索超时".to_string())?;

    let uids_set = match uids_res {
        Ok(set) => set,
        Err(_) if !trimmed.is_ascii() => {
            // Fallback for servers that reject CHARSET UTF-8 with OR: try simpler TEXT query.
            let fallback = format!("CHARSET UTF-8 TEXT \"{}\"", escape_imap_search_arg(trimmed));
            tokio::time::timeout(search_timeout, session.uid_search(&fallback))
                .await
                .map_err(|_| "IMAP 服务器搜索超时".to_string())?
                .map_err(|e| format!("IMAP 服务端搜索失败: {e}"))?
        }
        Err(e) => {
            let _ = session.logout().await;
            return Err(format!("IMAP 服务端搜索失败: {e}"));
        }
    };

    let mut uids: Vec<u32> = uids_set.into_iter().collect();
    uids.sort_unstable();
    if uids.is_empty() {
        let _ = session.logout().await;
        return Ok(Vec::new());
    }

    let cap = limit.max(1);
    let start = uids.len().saturating_sub(cap);
    let target_uids = &uids[start..];
    let headers = tokio::time::timeout(
        search_timeout,
        fetch_headers_for_uids(&mut session, target_uids),
    )
    .await
    .map_err(|_| "拉取服务端搜索结果邮件头超时".to_string())??;
    let _ = session.logout().await;
    Ok(headers)
}

/// 解析 FETCH 拉回的邮件前缀字节（含完整邮件头与正文开头）为缓存条目。
/// 任务四：正文开头经 mail_parser 解码后截取为真实预览，历史邮件无需打开
/// 正文即可按预览检索；解码失败或正文不可见时回退占位文案。
fn parse_header_bytes(
    parser: &MessageParser,
    uid: u32,
    unread: bool,
    flagged: bool,
    raw: &[u8],
) -> CachedHeaderInput {
    let Some(msg) = parser.parse(raw) else {
        return CachedHeaderInput {
            uid,
            message_id: String::new(),
            in_reply_to: String::new(),
            references_header: String::new(),
            sender: "(未知发件人)".to_string(),
            recipients: String::new(),
            cc: String::new(),
            subject: "(无法解析标题)".to_string(),
            preview: String::new(),
            date_ts: uid as i64,
            date_display: String::new(),
            unread,
            flagged,
            has_attachment: false,
        };
    };

    let sender = msg
        .from()
        .and_then(|addr| addr.first())
        .map(format_address)
        .unwrap_or_else(|| "(未知发件人)".to_string());

    let recipients = msg
        .to()
        .map(|list| {
            list.iter()
                .map(format_address)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();

    let cc = msg
        .cc()
        .map(|list| {
            list.iter()
                .map(format_address)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();

    let subject = msg
        .subject()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("(无主题)")
        .to_string();

    let message_id = msg.message_id().unwrap_or_default().to_string();
    let in_reply_to = header_value_to_ids(msg.in_reply_to());
    let references_header = header_value_to_ids(msg.references());

    let (date_ts, date_display) = match msg.date() {
        Some(dt) => {
            let ts = dt.to_timestamp();
            let display = chrono::DateTime::from_timestamp(ts, 0)
                .map(|utc| {
                    utc.with_timezone(&chrono::Local)
                        .format("%m-%d %H:%M")
                        .to_string()
                })
                .unwrap_or_default();
            (ts, display)
        }
        None => (uid as i64, String::new()),
    };

    let has_attachment = msg.content_type().is_some_and(|ct| {
        ct.ctype().eq_ignore_ascii_case("multipart") && ct.subtype() == Some("mixed")
    });

    // 任务四：正文开头作为真实预览（preview 级即可，不引入大批正文下载）；
    // 截断导致解码失败或纯 HTML 邮件转文本为空时回退占位文案。
    const PREVIEW_MAX_CHARS: usize = 200;
    let preview_from = |text: String| -> String {
        let compact = text
            .trim()
            .chars()
            .take(PREVIEW_MAX_CHARS)
            .collect::<String>();
        if compact.is_empty() {
            "点击加载邮件正文...".to_string()
        } else {
            compact
        }
    };
    let preview = if let Some(text) = msg.body_text(0) {
        preview_from(text.into_owned())
    } else if let Some(html) = msg.body_html(0) {
        preview_from(html_to_readable_text(&html))
    } else {
        "点击加载邮件正文...".to_string()
    };

    CachedHeaderInput {
        uid,
        message_id,
        in_reply_to,
        references_header,
        sender,
        recipients,
        cc,
        subject,
        preview,
        date_ts,
        date_display,
        unread,
        flagged,
        has_attachment,
    }
}

fn header_value_to_ids(val: &mail_parser::HeaderValue<'_>) -> String {
    match val {
        mail_parser::HeaderValue::Text(s) => s.trim().to_string(),
        mail_parser::HeaderValue::TextList(list) => list
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

fn format_address(addr: &mail_parser::Addr<'_>) -> String {
    match (addr.name(), addr.address()) {
        (Some(name), Some(email)) if !name.trim().is_empty() => {
            format!("{} <{}>", name.trim(), email)
        }
        (_, Some(email)) => email.to_string(),
        (Some(name), None) => name.to_string(),
        _ => "(未知)".to_string(),
    }
}

const IMAP_MUTF7_BASE64: base64::engine::GeneralPurpose = base64::engine::GeneralPurpose::new(
    &base64::alphabet::IMAP_MUTF7,
    base64::engine::general_purpose::NO_PAD
        .with_decode_padding_mode(base64::engine::DecodePaddingMode::RequireNone),
);

fn decode_imap_mutf7_run(run: &str) -> Option<String> {
    use base64::Engine;
    let bytes = IMAP_MUTF7_BASE64
        .decode(run)
        .ok()
        .filter(|b| !b.is_empty() && b.len() % 2 == 0)?;
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|p| u16::from_be_bytes([p[0], p[1]]))
        .collect();
    String::from_utf16(&units).ok()
}

/// Decode IMAP Modified UTF-7 (RFC 3501 §5.1.3) mailbox names for display.
/// Invalid or already-UTF-8 sequences are passed through unchanged.
pub(crate) fn decode_imap_mutf7(name: &str) -> String {
    if !name.contains('&') {
        return name.to_string();
    }

    let bytes = name.as_bytes();
    let mut out = String::with_capacity(name.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'&' {
            let rest = &name[i..];
            let ch = rest.chars().next().unwrap_or('\u{fffd}');
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }
        if bytes.get(i + 1) == Some(&b'-') {
            out.push('&');
            i += 2;
            continue;
        }
        match bytes[i + 1..].iter().position(|&b| b == b'-') {
            Some(end) => {
                let run = &name[i + 1..i + 1 + end];
                match decode_imap_mutf7_run(run) {
                    Some(text) => {
                        out.push_str(&text);
                        i += end + 2;
                    }
                    None => {
                        out.push('&');
                        i += 1;
                    }
                }
            }
            None => {
                out.push('&');
                i += 1;
            }
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ImapSpecialRole {
    Inbox,
    Sent,
    Drafts,
    Trash,
    Junk,
    Archive,
}

impl ImapSpecialRole {
    fn friendly_name(self) -> &'static str {
        match self {
            Self::Inbox => "收件箱",
            Self::Sent => "已发送",
            Self::Drafts => "草稿箱",
            Self::Trash => "已删除",
            Self::Junk => "垃圾邮件",
            Self::Archive => "归档",
        }
    }

    fn from_leaf_name(leaf: &str) -> Option<Self> {
        match leaf.trim().to_ascii_lowercase().as_str() {
            "sent" | "sent messages" | "sent items" | "sent mail" | "已发送" | "已发邮件" => {
                Some(Self::Sent)
            }
            "drafts" | "draft" | "草稿" | "草稿箱" => Some(Self::Drafts),
            "trash" | "deleted" | "deleted messages" | "deleted items" | "bin" | "已删除"
            | "已删除邮件" | "废纸篓" => Some(Self::Trash),
            "junk" | "junk email" | "spam" | "垃圾邮件" => Some(Self::Junk),
            "archive" | "归档" | "存档" => Some(Self::Archive),
            _ => None,
        }
    }
}

struct RawImapFolderEntry {
    path: String,
    delimiter: Option<String>,
    no_select: bool,
    special_role: Option<ImapSpecialRole>,
}

fn classify_imap_attributes(attrs: &[NameAttribute<'_>]) -> (bool, Option<ImapSpecialRole>) {
    let mut no_select = false;
    let mut special_role = None;
    for attr in attrs {
        match attr {
            NameAttribute::NoSelect => no_select = true,
            NameAttribute::Extension(s) => {
                let lower = s.trim().to_ascii_lowercase();
                if lower == "\\noselect" || lower == "noselect" {
                    no_select = true;
                }
            }
            NameAttribute::Sent if special_role.is_none() => {
                special_role = Some(ImapSpecialRole::Sent);
            }
            NameAttribute::Drafts if special_role.is_none() => {
                special_role = Some(ImapSpecialRole::Drafts);
            }
            NameAttribute::Trash if special_role.is_none() => {
                special_role = Some(ImapSpecialRole::Trash);
            }
            NameAttribute::Junk if special_role.is_none() => {
                special_role = Some(ImapSpecialRole::Junk);
            }
            NameAttribute::Archive if special_role.is_none() => {
                special_role = Some(ImapSpecialRole::Archive);
            }
            _ => {}
        }
    }
    (no_select, special_role)
}

fn is_role_placement(path: &str, delimiter: Option<&str>) -> bool {
    let parts: Vec<&str> = match delimiter.filter(|d| !d.is_empty()) {
        Some(d) => path.split(d).collect(),
        None => vec![path],
    };
    match parts.as_slice() {
        [_] => true,
        [parent, _] => parent.eq_ignore_ascii_case("INBOX") || parent.starts_with('['),
        _ => false,
    }
}

fn resolve_imap_folders(entries: &[RawImapFolderEntry]) -> Vec<RemoteFolder> {
    let mut present_special_roles = HashSet::new();
    for entry in entries {
        if entry.no_select {
            continue;
        }
        if entry.path.eq_ignore_ascii_case("INBOX") {
            present_special_roles.insert(ImapSpecialRole::Inbox);
        } else if let Some(role) = entry.special_role {
            present_special_roles.insert(role);
        }
    }

    let mut folders = Vec::with_capacity(entries.len());
    for entry in entries {
        if entry.no_select {
            continue;
        }
        let delim = entry.delimiter.as_deref().filter(|d| !d.is_empty());
        let leaf_raw = match delim {
            Some(d) => entry.path.rsplit(d).next().unwrap_or(&entry.path),
            None => entry.path.as_str(),
        };
        let decoded_leaf = decode_imap_mutf7(leaf_raw);
        let display = if entry.path.eq_ignore_ascii_case("INBOX") {
            ImapSpecialRole::Inbox.friendly_name().to_string()
        } else if let Some(role) = entry.special_role {
            role.friendly_name().to_string()
        } else if is_role_placement(&entry.path, delim) {
            match ImapSpecialRole::from_leaf_name(&decoded_leaf) {
                Some(candidate) if !present_special_roles.contains(&candidate) => {
                    candidate.friendly_name().to_string()
                }
                _ if !decoded_leaf.is_empty() => decoded_leaf,
                _ => entry.path.clone(),
            }
        } else if !decoded_leaf.is_empty() {
            decoded_leaf
        } else {
            entry.path.clone()
        };
        folders.push(RemoteFolder {
            name: display,
            path: entry.path.clone(),
            unread_count: 0,
            total_count: 0,
        });
    }
    folders
}

/// Convert HTML-only messages into readable plain text while stripping scripts/styles.
pub fn html_to_readable_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let mut in_tag = false;
    let mut skip_block = false;
    let mut tag_buf = String::new();

    for ch in html.chars() {
        if ch == '<' {
            in_tag = true;
            tag_buf.clear();
            continue;
        }
        if ch == '>' && in_tag {
            in_tag = false;
            let lower = tag_buf.trim().to_ascii_lowercase();
            // Skip HTML comments (`<!-- ... -->`).
            if lower.starts_with("!--") {
                continue;
            }
            if lower.starts_with("script")
                || lower.starts_with("style")
                || lower.starts_with("head") && !lower.starts_with("header")
            {
                skip_block = true;
            } else if lower.starts_with("/script")
                || lower.starts_with("/style")
                || lower.starts_with("/head") && !lower.starts_with("/header")
            {
                skip_block = false;
            } else if is_block_break_tag(&lower) && !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            continue;
        }
        if in_tag {
            tag_buf.push(ch);
            continue;
        }
        if !skip_block {
            out.push(ch);
        }
    }

    let decoded = out
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'");

    // Trim trailing spaces per line and collapse 3+ consecutive blank lines into 2.
    let mut normalized = String::with_capacity(decoded.len());
    let mut empty_run = 0usize;
    for line in decoded.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            empty_run += 1;
            if empty_run <= 1 && !normalized.is_empty() {
                normalized.push('\n');
            }
        } else {
            if !normalized.is_empty() {
                normalized.push('\n');
            }
            normalized.push_str(trimmed);
            empty_run = 0;
        }
    }
    normalized
}

fn is_block_break_tag(lower: &str) -> bool {
    let name = lower
        .split(|c: char| c.is_whitespace() || c == '/')
        .find(|s| !s.is_empty())
        .unwrap_or("");
    let is_closing = lower.starts_with('/');
    matches!(
        (is_closing, name),
        (_, "br" | "hr")
            | (
                true,
                "p" | "div"
                    | "li"
                    | "tr"
                    | "table"
                    | "blockquote"
                    | "section"
                    | "article"
                    | "h1"
                    | "h2"
                    | "h3"
                    | "h4"
                    | "h5"
                    | "h6"
            )
            | (
                false,
                "p" | "div" | "tr" | "li" | "blockquote" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
            )
    )
}

/// Strip redirectors (up to 4 layers) and common tracking query parameters from
/// `http(s)://` URLs while preserving legitimate parameters and URL fragments.
pub fn strip_tracking_params(raw_url: &str) -> String {
    crate::link_clean::clean_link(raw_url)
}

fn clean_anchor_tag_href(raw_tag: &str) -> String {
    let lower = raw_tag.to_ascii_lowercase();
    let Some(href_pos) = lower.find("href") else {
        return raw_tag.to_string();
    };
    let after_href = &raw_tag[href_pos + 4..];
    let eq_offset = match after_href.find('=') {
        Some(idx) if after_href[..idx].trim().is_empty() => idx,
        _ => return raw_tag.to_string(),
    };
    let rest = &after_href[eq_offset + 1..];
    let ws_len = rest.len() - rest.trim_start().len();
    let val_start_slice = &rest[ws_len..];
    let (quote, inner_start_in_rest, inner_end_in_rest) =
        if let Some(stripped) = val_start_slice.strip_prefix('"') {
            let Some(end_q) = stripped.find('"') else {
                return raw_tag.to_string();
            };
            (Some('"'), ws_len + 1, ws_len + 1 + end_q)
        } else if let Some(stripped) = val_start_slice.strip_prefix('\'') {
            let Some(end_q) = stripped.find('\'') else {
                return raw_tag.to_string();
            };
            (Some('\''), ws_len + 1, ws_len + 1 + end_q)
        } else {
            let end_u = val_start_slice
                .find(|c: char| c.is_whitespace() || c == '>')
                .unwrap_or(val_start_slice.len());
            (None, ws_len, ws_len + end_u)
        };
    let abs_start = href_pos + 4 + eq_offset + 1 + inner_start_in_rest;
    let abs_end = href_pos + 4 + eq_offset + 1 + inner_end_in_rest;
    let _ = quote;
    let orig_url = &raw_tag[abs_start..abs_end];
    let cleaned = strip_tracking_params(orig_url);
    if cleaned == orig_url {
        return raw_tag.to_string();
    }
    let mut out = String::with_capacity(raw_tag.len());
    out.push_str(&raw_tag[..abs_start]);
    out.push_str(&cleaned);
    out.push_str(&raw_tag[abs_end..]);
    out
}

fn extract_style_attr(inner: &str) -> Option<&str> {
    let lower = inner.to_ascii_lowercase();
    let mut search_from = 0usize;
    while let Some(rel) = lower[search_from..].find("style") {
        let pos = search_from + rel;
        let before_ok = pos == 0 || inner.as_bytes()[pos - 1].is_ascii_whitespace();
        let after = pos + 5;
        if before_ok {
            let rest = &inner[after..];
            let trimmed = rest.trim_start();
            if let Some(after_eq) = trimmed.strip_prefix('=') {
                let after_eq = after_eq.trim_start();
                if let Some(q) = after_eq.bytes().next().filter(|&b| b == b'"' || b == b'\'') {
                    let content = &after_eq[1..];
                    if let Some(end) = content.find(q as char) {
                        return Some(&content[..end]);
                    }
                }
            }
        }
        search_from = after;
    }
    None
}

fn style_prop<'a>(style: &'a str, prop: &str) -> Option<&'a str> {
    for decl in style.split(';') {
        let Some((name, val)) = decl.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case(prop) {
            return Some(val.trim());
        }
    }
    None
}

fn is_void_html_tag(tag: &str) -> bool {
    matches!(
        tag,
        "br" | "img" | "hr" | "meta" | "link" | "input" | "wbr" | "col" | "area" | "base"
    )
}

fn has_hidden_attribute(inner: &str) -> bool {
    let bytes = inner.as_bytes();
    let mut i = 0usize;
    // Skip tag name.
    while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] == b'/' {
            break;
        }
        let name_start = i;
        while i < bytes.len()
            && !bytes[i].is_ascii_whitespace()
            && bytes[i] != b'='
            && bytes[i] != b'/'
        {
            i += 1;
        }
        let name = &inner[name_start..i];
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let has_eq = i < bytes.len() && bytes[i] == b'=';
        if has_eq {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
                let quote = bytes[i];
                i += 1;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
            } else {
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
            }
        }
        if name.eq_ignore_ascii_case("hidden") {
            return true;
        }
    }
    false
}

fn css_zero_value(val: &str) -> bool {
    let v = val.trim().trim_end_matches("!important").trim();
    matches!(
        v.to_ascii_lowercase().as_str(),
        "0" | "0px" | "0em" | "0rem" | "0pt" | "0%"
    )
}

fn is_hidden_html_element(inner: &str) -> bool {
    if has_hidden_attribute(inner) {
        return true;
    }
    let Some(style) = extract_style_attr(inner) else {
        return false;
    };
    if let Some(display) = style_prop(style, "display") {
        let v = display.trim().trim_end_matches("!important").trim();
        if v.eq_ignore_ascii_case("none") {
            return true;
        }
    }
    if let Some(vis) = style_prop(style, "visibility") {
        let v = vis.trim().trim_end_matches("!important").trim();
        if v.eq_ignore_ascii_case("hidden") {
            return true;
        }
    }
    if style_prop(style, "font-size").is_some_and(css_zero_value)
        && (style_prop(style, "max-height").is_some_and(css_zero_value)
            || style_prop(style, "opacity").is_some_and(css_zero_value))
    {
        return true;
    }
    false
}

/// Sanitize HTML email bodies for safe native display:
/// strips `<script>`, `<style>`, `<head>`, `<iframe>`, `<object>`, HTML comments,
/// and elements hidden via `hidden` / `display: none` / `visibility: hidden`,
/// optionally removes `<img>` elements referencing remote `http(s)://` URLs,
/// and optionally strips tracking parameters from `<a href="...">` links.
/// Returns `(sanitized_html, blocked_remote_image_count)`.
pub fn sanitize_html_for_display_with_options(
    html: &str,
    block_remote_images: bool,
    clean_links: bool,
) -> (String, usize) {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut skip_block: Option<&'static str> = None;
    let mut hidden_stack: Vec<String> = Vec::new();
    let mut tag_buf = String::new();
    let mut blocked_images = 0usize;

    for ch in html.chars() {
        if ch == '<' {
            in_tag = true;
            tag_buf.clear();
            continue;
        }
        if ch == '>' && in_tag {
            in_tag = false;
            let trimmed = tag_buf.trim();
            let lower = trimmed.to_ascii_lowercase();
            let tag_name = lower
                .trim_start_matches('/')
                .split(|c: char| c.is_whitespace() || c == '/')
                .next()
                .unwrap_or("");
            let is_closing = lower.starts_with('/');
            let self_closing = trimmed.ends_with('/') || is_void_html_tag(tag_name);

            if lower.starts_with("!--") {
                continue;
            }
            if let Some(active) = skip_block {
                if is_closing && tag_name == active {
                    skip_block = None;
                }
                continue;
            }
            if !hidden_stack.is_empty() {
                if is_closing {
                    if let Some(pos) = hidden_stack.iter().rposition(|t| t == tag_name) {
                        hidden_stack.truncate(pos);
                        continue;
                    }
                    if matches!(
                        tag_name,
                        "body"
                            | "html"
                            | "table"
                            | "thead"
                            | "tbody"
                            | "tfoot"
                            | "tr"
                            | "td"
                            | "th"
                    ) {
                        hidden_stack.clear();
                        // Container closed before malformed hidden tag; fall through to emit `</tag>`.
                    } else {
                        continue;
                    }
                } else {
                    let auto_closed = match hidden_stack.last().map(String::as_str) {
                        Some("p") => matches!(
                            tag_name,
                            "p" | "div"
                                | "table"
                                | "h1"
                                | "h2"
                                | "h3"
                                | "h4"
                                | "h5"
                                | "h6"
                                | "ul"
                                | "ol"
                                | "blockquote"
                                | "hr"
                                | "pre"
                                | "section"
                                | "article"
                                | "header"
                                | "footer"
                        ),
                        Some("td" | "th") => matches!(tag_name, "td" | "th" | "tr"),
                        Some("tr") => tag_name == "tr",
                        _ => false,
                    };
                    if auto_closed {
                        hidden_stack.pop();
                        if !hidden_stack.is_empty() {
                            if !self_closing && !tag_name.is_empty() {
                                hidden_stack.push(tag_name.to_string());
                            }
                            continue;
                        }
                        // Unclosed hidden tag was terminated by sibling/block boundary; fall through.
                    } else {
                        if !self_closing && !tag_name.is_empty() {
                            hidden_stack.push(tag_name.to_string());
                        }
                        continue;
                    }
                }
            }
            if !is_closing && matches!(tag_name, "script" | "style" | "head" | "iframe" | "object")
            {
                skip_block = Some(match tag_name {
                    "script" => "script",
                    "style" => "style",
                    "head" => "head",
                    "iframe" => "iframe",
                    _ => "object",
                });
                continue;
            }
            if !is_closing && !tag_name.is_empty() && is_hidden_html_element(trimmed) {
                if !self_closing {
                    hidden_stack.push(tag_name.to_string());
                }
                continue;
            }
            if !is_closing
                && tag_name == "img"
                && block_remote_images
                && (lower.contains("http://") || lower.contains("https://"))
            {
                blocked_images += 1;
                continue;
            }
            out.push('<');
            if clean_links && !is_closing && tag_name == "a" {
                out.push_str(&clean_anchor_tag_href(trimmed));
            } else {
                out.push_str(trimmed);
            }
            out.push('>');
            continue;
        }
        if in_tag {
            tag_buf.push(ch);
            continue;
        }
        if skip_block.is_none() && hidden_stack.is_empty() {
            out.push(ch);
        }
    }

    (normalize_layout_tables_for_display(&out), blocked_images)
}

/// Unwrap email layout `<table>` wrappers into `<div>` blocks so GPUI's
/// `TextViewState::html` preserves line breaks across nested blocks, while
/// keeping multi-column leaf data tables intact (turning inner `<p>`/`<div>`
/// into `<br>` inside their cells).
fn normalize_layout_tables_for_display(html: &str) -> String {
    #[derive(Clone, Copy)]
    enum TokenKind<'a> {
        Text(&'a str),
        Tag {
            raw: &'a str,
            name_end: usize,
            is_closing: bool,
        },
    }

    let mut tokens: Vec<TokenKind<'_>> = Vec::new();
    let mut pos = 0usize;
    let bytes = html.as_bytes();
    while pos < bytes.len() {
        let Some(rel_lt) = html[pos..].find('<') else {
            tokens.push(TokenKind::Text(&html[pos..]));
            break;
        };
        let lt = pos + rel_lt;
        if lt > pos {
            tokens.push(TokenKind::Text(&html[pos..lt]));
        }
        let Some(rel_gt) = html[lt + 1..].find('>') else {
            tokens.push(TokenKind::Text(&html[lt..]));
            break;
        };
        let gt = lt + 1 + rel_gt;
        let inner = html[lt + 1..gt].trim();
        let is_closing = inner.starts_with('/');
        let name_slice = inner.trim_start_matches('/');
        let name_end = name_slice
            .find(|c: char| c.is_whitespace() || c == '/')
            .unwrap_or(name_slice.len());
        tokens.push(TokenKind::Tag {
            raw: &html[lt..=gt],
            name_end,
            is_closing,
        });
        pos = gt + 1;
    }

    struct TableMeta {
        open_idx: usize,
        is_presentation: bool,
        has_nested_table: bool,
        cur_row_cols: usize,
        max_cols: usize,
    }

    let mut is_layout_table = vec![false; tokens.len()];
    let mut table_of_token: Vec<Option<usize>> = vec![None; tokens.len()];
    let mut stack: Vec<TableMeta> = Vec::new();

    for (idx, tok) in tokens.iter().enumerate() {
        let TokenKind::Tag {
            raw,
            name_end,
            is_closing,
        } = *tok
        else {
            continue;
        };
        let inner = raw[1..raw.len() - 1].trim();
        let name_slice = inner.trim_start_matches('/');
        let tag = name_slice[..name_end].to_ascii_lowercase();

        if !is_closing && tag == "table" {
            if let Some(parent) = stack.last_mut() {
                parent.has_nested_table = true;
            }
            let lower = inner.to_ascii_lowercase();
            let is_presentation = lower.contains("role=\"presentation\"")
                || lower.contains("role='presentation'")
                || lower.contains("role=presentation")
                || lower.contains("role=\"none\"")
                || lower.contains("role='none'");
            table_of_token[idx] = Some(idx);
            stack.push(TableMeta {
                open_idx: idx,
                is_presentation,
                has_nested_table: false,
                cur_row_cols: 0,
                max_cols: 0,
            });
        } else if is_closing && tag == "table" {
            if let Some(meta) = stack.pop() {
                let max_cols = meta.max_cols.max(meta.cur_row_cols);
                let layout = meta.is_presentation || meta.has_nested_table || max_cols <= 1;
                is_layout_table[meta.open_idx] = layout;
                is_layout_table[idx] = layout;
                table_of_token[idx] = Some(meta.open_idx);
            }
        } else if let Some(top) = stack.last_mut() {
            table_of_token[idx] = Some(top.open_idx);
            if !is_closing && tag == "tr" {
                top.max_cols = top.max_cols.max(top.cur_row_cols);
                top.cur_row_cols = 0;
            } else if is_closing && tag == "tr" {
                top.max_cols = top.max_cols.max(top.cur_row_cols);
                top.cur_row_cols = 0;
            } else if !is_closing && (tag == "td" || tag == "th") {
                top.cur_row_cols += 1;
            }
        }
    }
    // Unclosed tables default to layout unwrapping.
    for meta in stack {
        is_layout_table[meta.open_idx] = true;
    }

    fn has_bold_font_weight(style: &str) -> bool {
        let Some(val) = style_prop(style, "font-weight") else {
            return false;
        };
        let v = val.trim().trim_end_matches("!important").trim();
        if v.eq_ignore_ascii_case("bold") || v.eq_ignore_ascii_case("bolder") {
            return true;
        }
        v.parse::<u32>().is_ok_and(|n| n >= 600)
    }

    fn large_font_heading(style: &str) -> Option<&'static str> {
        let val = style_prop(style, "font-size")?;
        let v = val.trim().trim_end_matches("!important").trim();
        let lower = v.to_ascii_lowercase();
        let px = if let Some(num) = lower.strip_suffix("px") {
            num.trim().parse::<f32>().ok()?
        } else if let Some(num) = lower.strip_suffix("pt") {
            num.trim().parse::<f32>().ok()? * 1.333
        } else {
            return None;
        };
        if px >= 20.0 {
            Some("h3")
        } else if px >= 17.0 {
            Some("h4")
        } else {
            None
        }
    }

    struct OpenTagState {
        orig_tag: String,
        heading: Option<&'static str>,
        bold: bool,
    }

    let mut out = String::with_capacity(html.len());
    let mut in_data_cell = false;
    let mut data_cell_has_content = false;
    let mut open_tags: Vec<OpenTagState> = Vec::new();

    for (idx, tok) in tokens.iter().enumerate() {
        match *tok {
            TokenKind::Text(t) => {
                if in_data_cell && !t.trim().is_empty() {
                    data_cell_has_content = true;
                }
                out.push_str(t);
            }
            TokenKind::Tag {
                raw,
                name_end,
                is_closing,
            } => {
                let inner = raw[1..raw.len() - 1].trim();
                let name_slice = inner.trim_start_matches('/');
                let tag = name_slice[..name_end].to_ascii_lowercase();
                let self_closing = inner.ends_with('/') || is_void_html_tag(&tag);

                if tag == "center" {
                    out.push_str(if is_closing { "</div>" } else { "<div>" });
                    continue;
                }

                if let Some(open_idx) = table_of_token[idx] {
                    if is_layout_table[open_idx] {
                        match tag.as_str() {
                            "table" | "tr" | "td" | "th" => {
                                if is_closing {
                                    if let Some(pos) =
                                        open_tags.iter().rposition(|s| s.orig_tag == tag)
                                    {
                                        let state = open_tags.remove(pos);
                                        if state.bold {
                                            out.push_str("</b>");
                                        }
                                    }
                                    out.push_str("</div>");
                                } else {
                                    let bold =
                                        extract_style_attr(inner).is_some_and(has_bold_font_weight);
                                    out.push_str("<div>");
                                    if !self_closing {
                                        if bold {
                                            out.push_str("<b>");
                                        }
                                        open_tags.push(OpenTagState {
                                            orig_tag: tag,
                                            heading: None,
                                            bold,
                                        });
                                    }
                                }
                                continue;
                            }
                            "thead" | "tbody" | "tfoot" => continue,
                            _ => {}
                        }
                    } else {
                        // Data table: keep <table>/<tr>/<td>/<th>, but turn block
                        // wrappers inside cells into <br> so cell lines don't merge.
                        match tag.as_str() {
                            "td" | "th" => {
                                in_data_cell = !is_closing;
                                data_cell_has_content = false;
                            }
                            "div" | "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
                                if in_data_cell =>
                            {
                                if is_closing {
                                    if let Some(pos) =
                                        open_tags.iter().rposition(|s| s.orig_tag == tag)
                                    {
                                        let state = open_tags.remove(pos);
                                        if state.bold {
                                            out.push_str("</b>");
                                        }
                                    }
                                    if data_cell_has_content {
                                        out.push_str("<br>");
                                        data_cell_has_content = false;
                                    }
                                } else if !self_closing {
                                    let bold = tag.starts_with('h')
                                        || extract_style_attr(inner)
                                            .is_some_and(has_bold_font_weight);
                                    if bold {
                                        out.push_str("<b>");
                                    }
                                    open_tags.push(OpenTagState {
                                        orig_tag: tag,
                                        heading: None,
                                        bold,
                                    });
                                }
                                continue;
                            }
                            _ => {}
                        }
                    }
                }

                if is_closing {
                    if let Some(pos) = open_tags.iter().rposition(|s| s.orig_tag == tag) {
                        let state = open_tags.remove(pos);
                        if state.bold {
                            out.push_str("</b>");
                        }
                        if let Some(h) = state.heading {
                            out.push_str(&format!("</{h}>"));
                            continue;
                        }
                    }
                    out.push_str(raw);
                } else {
                    let style = extract_style_attr(inner);
                    let heading = if !in_data_cell && matches!(tag.as_str(), "div" | "p") {
                        style.and_then(large_font_heading)
                    } else {
                        None
                    };
                    let already_bold = matches!(
                        tag.as_str(),
                        "b" | "strong" | "th" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
                    );
                    let bold = heading.is_none()
                        && !already_bold
                        && style.is_some_and(has_bold_font_weight);

                    if let Some(h) = heading {
                        out.push_str(&format!("<{h}>"));
                    } else {
                        out.push_str(raw);
                    }
                    if !self_closing {
                        if bold {
                            out.push_str("<b>");
                        }
                        open_tags.push(OpenTagState {
                            orig_tag: tag,
                            heading,
                            bold,
                        });
                    }
                }
            }
        }
    }

    out
}

/// Sanitize HTML email bodies for safe native display (legacy default without link cleaning).
pub fn sanitize_html_for_display(html: &str, block_remote_images: bool) -> (String, usize) {
    sanitize_html_for_display_with_options(html, block_remote_images, false)
}

fn find_tag_end_quote_aware(html: &str, start: usize) -> Option<usize> {
    let bytes = html.as_bytes();
    let mut quote: Option<u8> = None;
    let mut i = start;
    while i < bytes.len() {
        let b = bytes[i];
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None if b == b'"' || b == b'\'' => quote = Some(b),
            None if b == b'>' => return Some(i + 1),
            None => {}
        }
        i += 1;
    }
    None
}

fn find_closing_raw_tag_end(html: &str, from: usize, close_prefix: &[u8]) -> Option<usize> {
    let bytes = html.as_bytes();
    let mut i = from;
    while i + close_prefix.len() <= bytes.len() {
        if bytes[i] == b'<' && bytes[i..i + close_prefix.len()].eq_ignore_ascii_case(close_prefix) {
            let next = bytes.get(i + close_prefix.len()).copied();
            if next.is_some_and(|c| c.is_ascii_whitespace() || c == b'>' || c == b'/') {
                if let Some(gt) = bytes[i + close_prefix.len()..]
                    .iter()
                    .position(|&b| b == b'>')
                {
                    return Some(i + close_prefix.len() + gt + 1);
                }
            }
        }
        i += 1;
    }
    None
}

fn bytes_contain_http_url(bytes: &[u8]) -> bool {
    bytes.windows(7).any(|w| w.eq_ignore_ascii_case(b"http://"))
        || bytes
            .windows(8)
            .any(|w| w.eq_ignore_ascii_case(b"https://"))
}

fn is_remote_url_token(url: &str) -> bool {
    let b = url.trim_start().as_bytes();
    (b.len() >= 7 && b[..7].eq_ignore_ascii_case(b"http://"))
        || (b.len() >= 8 && b[..8].eq_ignore_ascii_case(b"https://"))
        || b.starts_with(b"//")
}

fn img_attrs_have_remote_url(attrs: &str) -> bool {
    let bytes = attrs.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;

    while i < len {
        while i < len && (bytes[i].is_ascii_whitespace() || bytes[i] == b'/') {
            i += 1;
        }
        if i >= len {
            break;
        }
        let name_start = i;
        while i < len && !bytes[i].is_ascii_whitespace() && bytes[i] != b'=' && bytes[i] != b'/' {
            i += 1;
        }
        let name = attrs[name_start..i].to_ascii_lowercase();
        while i < len && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let val = if i < len && bytes[i] == b'=' {
            i += 1;
            while i < len && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < len && (bytes[i] == b'"' || bytes[i] == b'\'') {
                let quote = bytes[i];
                i += 1;
                let v_start = i;
                while i < len && bytes[i] != quote {
                    i += 1;
                }
                let v = &attrs[v_start..i];
                if i < len {
                    i += 1;
                }
                v
            } else {
                let v_start = i;
                while i < len && !bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                &attrs[v_start..i]
            }
        } else {
            ""
        };

        match name.as_str() {
            "src" | "data-src" | "data-original" | "lowsrc" | "dynsrc" => {
                if is_remote_url_token(val) {
                    return true;
                }
            }
            "srcset" => {
                if val.split(',').any(|part| {
                    part.split_whitespace()
                        .next()
                        .is_some_and(is_remote_url_token)
                }) {
                    return true;
                }
            }
            "style" => {
                if bytes_contain_http_url(val.as_bytes()) {
                    return true;
                }
            }
            _ => {}
        }
    }

    false
}

/// Strip remote `<img>` tags and `<script>` blocks from HTML while preserving
/// `<head>`, `<style>`, `<table>`, inline styles, and `data:`/`cid:` images
/// for sandboxed WebView2 thread rendering.
pub fn strip_remote_images_for_webview(html: &str) -> (String, usize) {
    let mut out = String::with_capacity(html.len());
    let mut blocked_count = 0usize;
    let bytes = html.as_bytes();
    let len = bytes.len();
    let mut pos = 0usize;

    while pos < len {
        let Some(rel) = html[pos..].find('<') else {
            out.push_str(&html[pos..]);
            break;
        };
        out.push_str(&html[pos..pos + rel]);
        let tag_start = pos + rel;

        if html[tag_start..].starts_with("<!--") {
            if let Some(end_rel) = html[tag_start + 4..].find("-->") {
                let end = tag_start + 4 + end_rel + 3;
                out.push_str(&html[tag_start..end]);
                pos = end;
            } else {
                out.push_str(&html[tag_start..]);
                break;
            }
            continue;
        }

        if bytes
            .get(tag_start + 1)
            .is_some_and(|b| *b == b'!' || *b == b'?')
        {
            if let Some(tag_end) = find_tag_end_quote_aware(html, tag_start + 1) {
                out.push_str(&html[tag_start..tag_end]);
                pos = tag_end;
            } else {
                out.push_str(&html[tag_start..]);
                break;
            }
            continue;
        }

        let Some(tag_end) = find_tag_end_quote_aware(html, tag_start + 1) else {
            out.push_str(&html[tag_start..]);
            break;
        };

        let inner = &html[tag_start + 1..tag_end - 1];
        let trimmed = inner.trim_start();
        if trimmed.starts_with('/') {
            out.push_str(&html[tag_start..tag_end]);
            pos = tag_end;
            continue;
        }

        let name_end = trimmed
            .find(|c: char| c.is_ascii_whitespace() || c == '/')
            .unwrap_or(trimmed.len());
        let tag_name = trimmed[..name_end].to_ascii_lowercase();
        let attrs = &trimmed[name_end..];

        if tag_name == "script" {
            pos = find_closing_raw_tag_end(html, tag_end, b"</script").unwrap_or(len);
            continue;
        }

        if tag_name == "style" {
            let end = find_closing_raw_tag_end(html, tag_end, b"</style").unwrap_or(len);
            out.push_str(&html[tag_start..end]);
            pos = end;
            continue;
        }

        if tag_name == "img" && img_attrs_have_remote_url(attrs) {
            blocked_count += 1;
            pos = tag_end;
            continue;
        }

        out.push_str(&html[tag_start..tag_end]);
        pos = tag_end;
    }

    (out, blocked_count)
}

/// Convert plain text into escaped HTML paragraphs so it can be rendered in a
/// selectable rich-text view without accidental Markdown interpretation.
pub fn plain_text_to_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 32);
    for para in text.split("\n\n") {
        out.push_str("<p>");
        let mut first_line = true;
        for line in para.lines() {
            if !first_line {
                out.push_str("<br>");
            }
            first_line = false;
            for ch in line.chars() {
                match ch {
                    '&' => out.push_str("&amp;"),
                    '<' => out.push_str("&lt;"),
                    '>' => out.push_str("&gt;"),
                    _ => out.push(ch),
                }
            }
        }
        out.push_str("</p>");
    }
    if out.is_empty() {
        "<p></p>".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn single_delete_checks_tagged_replies_and_sends_only_target_uid() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        for (store_ok, expunge_ok) in [(false, false), (true, false), (true, true)] {
            let (client, server) = tokio::io::duplex(2048);
            let server_task = tokio::spawn(async move {
                let mut io = BufReader::new(server);
                let mut expected = vec![
                    ("LOGIN", true),
                    ("UID STORE 42 +FLAGS.SILENT (\\Deleted)", store_ok),
                ];
                if store_ok {
                    expected.push(("UID EXPUNGE 42", expunge_ok));
                }
                for (command, ok) in expected {
                    let mut line = String::new();
                    io.read_line(&mut line).await.unwrap();
                    let (tag, received) = line.trim_end().split_once(' ').unwrap();
                    if command == "LOGIN" {
                        assert!(received.starts_with("LOGIN "));
                    } else {
                        assert_eq!(received, command);
                    }
                    let status = if ok { "OK" } else { "NO" };
                    io.get_mut()
                        .write_all(format!("{tag} {status} test\r\n").as_bytes())
                        .await
                        .unwrap();
                }
                let mut extra = String::new();
                assert_eq!(io.read_line(&mut extra).await.unwrap(), 0);
            });
            let mut session = async_imap::Client::new(client)
                .login("test", "test")
                .await
                .unwrap();
            let result = super::expunge_selected_uid(&mut session, 42).await;
            assert_eq!(result.is_ok(), store_ok && expunge_ok);
            drop(session);
            server_task.await.unwrap();
        }
    }

    #[test]
    fn targeted_expunge_requires_explicit_capability() {
        use super::supports_targeted_expunge;
        assert!(supports_targeted_expunge(["uidplus"].into_iter()));
        assert!(supports_targeted_expunge(["IMAP4REV2"].into_iter()));
        assert!(!supports_targeted_expunge(
            ["IMAP4rev1", "MOVE"].into_iter()
        ));
    }

    #[test]
    fn incremental_sync_preserves_initial_window_and_flag_fallback() {
        use super::{select_sync_uids, MAX_INITIAL_FETCH};
        use std::collections::HashSet;
        let live: Vec<u32> = (1..=250).collect();
        let known: HashSet<u32> = (1..=100).collect();
        assert_eq!(
            select_sync_uids(&live, 7, Some((7, &known)), true),
            (101..=250).collect::<Vec<_>>()
        );
        let all: HashSet<u32> = live.iter().copied().collect();
        assert!(select_sync_uids(&live, 7, Some((7, &all)), true).is_empty());
        assert_eq!(
            select_sync_uids(&live, 7, Some((7, &all)), false),
            live[250 - MAX_INITIAL_FETCH..]
        );
        assert_eq!(
            select_sync_uids(&live, 8, Some((7, &all)), true),
            live[250 - MAX_INITIAL_FETCH..]
        );
        let mut missing = all.clone();
        missing.remove(&200);
        assert_eq!(
            select_sync_uids(&live, 7, Some((7, &missing)), true),
            vec![200]
        );
    }

    #[test]
    fn fetched_uid_validation_rejects_partial_duplicate_and_wrong_results() {
        use super::validate_fetched_uids;
        assert!(validate_fetched_uids(&[1, 2], [2, 1].into_iter()).is_ok());
        assert!(validate_fetched_uids(&[1, 2], [1].into_iter()).is_err());
        assert!(validate_fetched_uids(&[1, 2], [1, 1].into_iter()).is_err());
        assert!(validate_fetched_uids(&[1, 2], [1, 3].into_iter()).is_err());
    }

    #[test]
    fn large_incremental_gap_is_bounded_and_history_can_fill_it() {
        use super::{select_history_uids, select_sync_uids, MAX_INITIAL_FETCH};
        use std::collections::HashSet;
        let live: Vec<u32> = (1..=50_000).collect();
        let mut known: HashSet<u32> = (1..=100).collect();
        let first = select_sync_uids(&live, 7, Some((7, &known)), true);
        assert_eq!(first.len(), MAX_INITIAL_FETCH * 2);
        assert_eq!(&first[..3], &[101, 102, 103]);
        assert_eq!(*first.last().unwrap(), 50_000);
        known.extend(first);
        let (history, more) = select_history_uids(&live, &known, 100);
        assert!(more);
        assert_eq!(history.len(), 100);
        assert_eq!(*history.last().unwrap(), 49_900);
        for _ in 0..500 {
            let next = select_sync_uids(&live, 7, Some((7, &known)), true);
            assert!(next.len() <= MAX_INITIAL_FETCH * 2);
            if next.is_empty() {
                break;
            }
            known.extend(next);
        }
        assert_eq!(known.len(), live.len());
        known.remove(&25_000);
        assert_eq!(
            select_sync_uids(&live, 7, Some((7, &known)), true),
            vec![25_000]
        );
        assert_eq!(
            select_history_uids(&live, &known, 100),
            (vec![25_000], false)
        );
    }

    #[test]
    fn rename_path_keeps_unchanged_nested_names_and_uses_server_delimiter() {
        assert_eq!(
            renamed_folder_path("Parent/Child", "Child", Some("/")),
            "Parent/Child"
        );
        assert_eq!(
            renamed_folder_path("Parent/Child.name", "New", Some("/")),
            "Parent/New"
        );
        assert_eq!(
            renamed_folder_path("Parent.Child", "New", Some(".")),
            "Parent.New"
        );
        assert_eq!(renamed_folder_path("Parent", "New", None), "New");
    }

    #[test]
    fn index_text_reads_mime_tail_and_limits_size() {
        let raw = b"Subject: hello\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nfirst line\r\nunique tail keyword";
        let (index, text, _) = super::index_parts_from_mime(raw, 1024);
        assert!(index.contains("unique tail keyword"));
        assert!(text
            .as_deref()
            .is_some_and(|t| t.contains("unique tail keyword")));
        let (empty, no_text, no_html) = super::index_parts_from_mime(raw, raw.len() - 1);
        assert_eq!(empty, "");
        assert_eq!(no_text, None);
        assert_eq!(no_html, None);
    }

    use super::*;

    #[test]
    fn html_stripping_preserves_readable_lines() {
        let html = "<html><head><style>body{color:red}</style></head><body><p>你好，世界</p><p>第二行 &amp; 测试</p></body></html>";
        let text = html_to_readable_text(html);
        assert!(text.contains("你好，世界"));
        assert!(text.contains("第二行 & 测试"));
        assert!(!text.contains("color:red"));
    }

    #[test]
    fn html_stripping_separates_adjacent_blocks_and_table_rows() {
        let html = "<div>You authored the thread.</div><p>Message ID: &lt;abc@github.com&gt;</p>";
        let text = html_to_readable_text(html);
        assert_eq!(
            text,
            "You authored the thread.\nMessage ID: <abc@github.com>"
        );
    }

    #[test]
    fn sanitize_html_blocks_remote_images_and_scripts() {
        let raw = "<html><head><style>x{}</style></head><body><script>alert(1)</script><p>正文</p><img src=\"https://tracker.example/p.png\"></body></html>";
        let (clean, blocked) = sanitize_html_for_display(raw, true);
        assert_eq!(blocked, 1);
        assert!(clean.contains("<p>正文</p>"));
        assert!(!clean.contains("alert(1)"));
        assert!(!clean.contains("tracker.example"));

        let (allowed, blocked2) = sanitize_html_for_display(raw, false);
        assert_eq!(blocked2, 0);
        assert!(allowed.contains("tracker.example"));
    }

    #[test]
    fn plain_text_to_html_escapes_special_chars() {
        assert_eq!(
            plain_text_to_html("a < b & c\nline2"),
            "<p>a &lt; b &amp; c<br>line2</p>"
        );
    }

    #[test]
    fn build_imap_search_query_handles_ascii_and_utf8() {
        assert_eq!(
            build_imap_search_query("invoice \"2026\""),
            "OR SUBJECT \"invoice \\\"2026\\\"\" TEXT \"invoice \\\"2026\\\"\""
        );
        assert_eq!(
            build_imap_search_query("发票"),
            "CHARSET UTF-8 OR SUBJECT \"发票\" TEXT \"发票\""
        );
    }

    #[test]
    fn xoauth2_authenticator_sends_bearer_then_empty_on_error() {
        use async_imap::Authenticator;
        let mut auth = XOAuth2 {
            user: "user@outlook.com".to_string(),
            token: "test_token_123".to_string(),
            step: 0,
        };
        assert_eq!(
            auth.process(b""),
            b"user=user@outlook.com\x01auth=Bearer test_token_123\x01\x01".to_vec()
        );
        assert_eq!(auth.process(b"{\"status\":\"400\"}"), Vec::<u8>::new());
    }

    #[test]
    fn netease_host_detection_matches_163_126_yeah() {
        assert!(is_netease_host("imap.163.com"));
        assert!(is_netease_host("imap.126.com"));
        assert!(is_netease_host("imap.yeah.net"));
        assert!(!is_netease_host("imap.gmail.com"));
        assert!(!is_netease_host("outlook.office365.com"));
    }

    #[test]
    fn strip_tracking_params_keeps_legit_params_and_cleans_html_links() {
        assert_eq!(
            strip_tracking_params(
                "https://example.com/post?id=42&utm_source=mail&fbclid=xyz#section"
            ),
            "https://example.com/post?id=42#section"
        );
        assert_eq!(
            strip_tracking_params("https://example.com/post?gclid=123&utm_medium=email"),
            "https://example.com/post"
        );
        assert_eq!(
            strip_tracking_params("mailto:alice@example.com"),
            "mailto:alice@example.com"
        );

        let raw = r#"<p><a href="https://example.com/doc?page=2&utm_campaign=spring&msclkid=9">Link</a></p>"#;
        let (cleaned, _) = sanitize_html_for_display_with_options(raw, true, true);
        assert!(cleaned.contains("https://example.com/doc?page=2\""));
        assert!(!cleaned.contains("utm_campaign"));
        assert!(!cleaned.contains("msclkid"));
    }

    #[test]
    fn unwraps_layout_tables_and_preserves_data_table_cell_breaks() {
        let raw = r#"<table role="presentation"><tbody><tr><td><h3>Title</h3><center>Sub</center><table><tr><th>Status</th><th>Job</th></tr><tr><td>Fail</td><td><div>Build</div><div>1m</div></td></tr></table></td></tr></tbody></table>"#;
        let (cleaned, _) = sanitize_html_for_display_with_options(raw, false, false);
        assert!(cleaned.contains("<h3>Title</h3>"));
        assert!(cleaned.contains("<div>Sub</div>"));
        assert!(cleaned.contains("<table><tr><th>Status</th><th>Job</th></tr>"));
        assert!(cleaned.contains("<td>Build<br>1m<br></td>"));
    }

    #[test]
    fn maps_inline_font_weight_and_large_font_size() {
        let raw = r#"<div style="font-size: 20px; font-weight: 600;">Heading</div><p style="font-size: 18px;">Subhead</p><span style="font-weight: bold;">Bold</span><table><tr><th>A</th><th>B</th></tr><tr><td>1</td><td><div style="font-weight: 700;">Build</div><div>1m</div></td></tr></table>"#;
        let (cleaned, _) = sanitize_html_for_display_with_options(raw, false, false);
        assert!(cleaned.contains("<h3>Heading</h3>"), "got: {cleaned}");
        assert!(cleaned.contains("<h4>Subhead</h4>"), "got: {cleaned}");
        assert!(
            cleaned.contains(r#"<span style="font-weight: bold;"><b>Bold</b></span>"#),
            "got: {cleaned}"
        );
        assert!(
            cleaned.contains("<td><b>Build</b><br>1m<br></td>"),
            "got: {cleaned}"
        );
    }

    #[test]
    fn plan_bodystructure_separates_body_inline_cid_small_and_large_attachments() {
        use async_imap::imap_proto::types::{
            BodyContentCommon, BodyContentSinglePart, BodyStructure as Bs, ContentDisposition,
            ContentEncoding, ContentType,
        };
        use std::borrow::Cow;

        fn common(
            ty: &'static str,
            subtype: &'static str,
            disp: Option<(&'static str, Vec<(&'static str, &'static str)>)>,
            params: Vec<(&'static str, &'static str)>,
        ) -> BodyContentCommon<'static> {
            BodyContentCommon {
                ty: ContentType {
                    ty: Cow::Borrowed(ty),
                    subtype: Cow::Borrowed(subtype),
                    params: (!params.is_empty()).then(|| {
                        params
                            .into_iter()
                            .map(|(k, v)| (Cow::Borrowed(k), Cow::Borrowed(v)))
                            .collect()
                    }),
                },
                disposition: disp.map(|(d, dp)| ContentDisposition {
                    ty: Cow::Borrowed(d),
                    params: (!dp.is_empty()).then(|| {
                        dp.into_iter()
                            .map(|(k, v)| (Cow::Borrowed(k), Cow::Borrowed(v)))
                            .collect()
                    }),
                }),
                language: None,
                location: None,
            }
        }
        fn single(
            id: Option<&'static str>,
            enc: ContentEncoding<'static>,
            octets: u32,
        ) -> BodyContentSinglePart<'static> {
            BodyContentSinglePart {
                id: id.map(Cow::Borrowed),
                md5: None,
                description: None,
                transfer_encoding: enc,
                octets,
            }
        }

        let bs = Bs::Multipart {
            common: common("multipart", "mixed", None, vec![]),
            bodies: vec![
                Bs::Multipart {
                    common: common("multipart", "alternative", None, vec![]),
                    bodies: vec![
                        Bs::Text {
                            common: common("text", "plain", None, vec![("charset", "utf-8")]),
                            other: single(None, ContentEncoding::EightBit, 120),
                            lines: 4,
                            extension: None,
                        },
                        Bs::Text {
                            common: common("text", "html", None, vec![("charset", "utf-8")]),
                            other: single(None, ContentEncoding::QuotedPrintable, 300),
                            lines: 8,
                            extension: None,
                        },
                        Bs::Text {
                            common: common("text", "calendar", None, vec![("name", "invite.ics")]),
                            other: single(None, ContentEncoding::Base64, 800),
                            lines: 20,
                            extension: None,
                        },
                    ],
                    extension: None,
                },
                Bs::Basic {
                    common: common("image", "png", Some(("inline", vec![])), vec![]),
                    other: single(Some("<logo@cid>"), ContentEncoding::Base64, 40_000),
                    extension: None,
                },
                Bs::Basic {
                    common: common(
                        "application",
                        "pdf",
                        Some(("attachment", vec![("filename", "small.pdf")])),
                        vec![],
                    ),
                    other: single(None, ContentEncoding::Base64, 800_000),
                    extension: None,
                },
                Bs::Basic {
                    common: common(
                        "application",
                        "zip",
                        Some(("attachment", vec![("filename", "archive.zip")])),
                        vec![],
                    ),
                    other: single(None, ContentEncoding::Base64, 6_000_000),
                    extension: None,
                },
            ],
            extension: None,
        };

        let plan = super::plan_bodystructure_fetch(&bs);
        assert_eq!(plan.body_parts.len(), 2);
        assert_eq!(plan.body_parts[0].section, "1.1");
        assert_eq!(plan.body_parts[1].section, "1.2");

        assert_eq!(plan.inline_cids.len(), 1);
        assert_eq!(plan.inline_cids[0].section, "2");
        assert_eq!(plan.inline_cids[0].cid, "logo@cid");

        assert_eq!(plan.attachments.len(), 3);
        assert_eq!(plan.attachments[0].filename, "invite.ics");
        assert_eq!(plan.attachments[0].section, "1.3");
        assert!(plan.attachments[0].download_now);

        assert_eq!(plan.attachments[1].filename, "small.pdf");
        assert_eq!(plan.attachments[1].section, "3");
        assert_eq!(plan.attachments[1].estimated_size, 584_616);
        assert!(plan.attachments[1].download_now);
        assert!(plan.has_deferred_attachments());

        assert_eq!(plan.attachments[2].filename, "archive.zip");
        assert_eq!(plan.attachments[2].section, "4");
        assert_eq!(plan.attachments[2].estimated_size, 4_384_614);
        assert!(!plan.attachments[2].download_now);
    }

    #[test]
    fn rfc2231_and_rfc2183_size_are_decoded_in_bodystructure_plan() {
        use async_imap::imap_proto::types::{
            BodyContentCommon, BodyContentSinglePart, BodyStructure as Bs, ContentDisposition,
            ContentEncoding, ContentType,
        };
        use std::borrow::Cow;

        let bs_single = Bs::Basic {
            common: BodyContentCommon {
                ty: ContentType {
                    ty: Cow::Borrowed("application"),
                    subtype: Cow::Borrowed("pdf"),
                    params: None,
                },
                disposition: Some(ContentDisposition {
                    ty: Cow::Borrowed("attachment"),
                    params: Some(vec![
                        (
                            Cow::Borrowed("filename*"),
                            Cow::Borrowed("utf-8''%E6%B5%8B%E8%AF%95%E6%96%87%E6%A1%A3.pdf"),
                        ),
                        (Cow::Borrowed("size"), Cow::Borrowed("3145728")),
                    ]),
                }),
                language: None,
                location: None,
            },
            other: BodyContentSinglePart {
                id: None,
                md5: None,
                description: None,
                transfer_encoding: ContentEncoding::Base64,
                octets: 4_300_000,
            },
            extension: None,
        };
        let plan_single = super::plan_bodystructure_fetch(&bs_single);
        assert_eq!(plan_single.attachments.len(), 1);
        assert_eq!(plan_single.attachments[0].filename, "测试文档.pdf");
        assert_eq!(plan_single.attachments[0].estimated_size, 3_145_728);
        assert!(!plan_single.attachments[0].download_now);

        let bs_cont = Bs::Basic {
            common: BodyContentCommon {
                ty: ContentType {
                    ty: Cow::Borrowed("application"),
                    subtype: Cow::Borrowed("octet-stream"),
                    params: Some(vec![
                        (
                            Cow::Borrowed("name*0*"),
                            Cow::Borrowed("utf-8''%E4%B8%AD%E6%96%87"),
                        ),
                        (
                            Cow::Borrowed("name*1*"),
                            Cow::Borrowed("%E9%99%84%E4%BB%B6"),
                        ),
                        (Cow::Borrowed("name*2"), Cow::Borrowed(".bin")),
                    ]),
                },
                disposition: None,
                language: None,
                location: None,
            },
            other: BodyContentSinglePart {
                id: None,
                md5: None,
                description: None,
                transfer_encoding: ContentEncoding::Binary,
                octets: 512,
            },
            extension: None,
        };
        let plan_cont = super::plan_bodystructure_fetch(&bs_cont);
        assert_eq!(plan_cont.attachments[0].filename, "中文附件.bin");
        assert!(plan_cont.attachments[0].download_now);
        assert!(!plan_cont.has_deferred_attachments());
    }

    #[test]
    fn select_fallback_attachment_matches_case_and_sanitized_names() {
        let raw = concat!(
            "Subject: Att Test\r\n",
            "MIME-Version: 1.0\r\n",
            "Content-Type: multipart/mixed; boundary=\"B1\"\r\n\r\n",
            "--B1\r\n",
            "Content-Type: text/plain\r\n\r\nhello\r\n",
            "--B1\r\n",
            "Content-Type: application/octet-stream\r\n",
            "Content-Disposition: attachment; filename=\"Report:2026?.bin\"\r\n\r\nfirst-bytes\r\n",
            "--B1\r\n",
            "Content-Type: application/pdf\r\n",
            "Content-Disposition: attachment; filename=\"Summary.PDF\"\r\n\r\nsecond-bytes\r\n",
            "--B1--\r\n"
        );
        let parsed = MessageParser::default().parse(raw.as_bytes()).unwrap();
        assert_eq!(
            super::select_fallback_attachment(&parsed, "summary.pdf").as_deref(),
            Some(b"second-bytes".as_slice())
        );
        assert_eq!(
            super::select_fallback_attachment(&parsed, "Report_2026_.bin").as_deref(),
            Some(b"first-bytes".as_slice())
        );
    }

    #[test]
    fn decode_binary_and_text_sections_via_mail_parser() {
        use base64::Engine as _;
        let payload = b"PDF-binary-\x00\xFF-data";
        let b64 = base64::engine::general_purpose::STANDARD.encode(payload);
        let decoded = super::decode_binary_section(b64.as_bytes(), "application/pdf", "base64");
        assert_eq!(decoded, payload);

        let text_b64 =
            base64::engine::general_purpose::STANDARD.encode("你好 MIME 正文".as_bytes());
        let decoded_text = super::decode_text_section(
            text_b64.as_bytes(),
            "text/plain",
            Some("utf-8"),
            "base64",
            false,
        );
        assert_eq!(decoded_text.as_deref(), Some("你好 MIME 正文"));
    }

    #[test]
    fn decode_imap_mutf7_decodes_gmail_chinese_labels_and_passes_invalid() {
        assert_eq!(decode_imap_mutf7("&g0l6Pw-"), "草稿");
        assert_eq!(decode_imap_mutf7("&kc2JgQ-"), "重要");
        assert_eq!(decode_imap_mutf7("&XfJSoGYfaAc-"), "已加星标");
        assert_eq!(decode_imap_mutf7("&V4NXPpCuTvY-"), "垃圾邮件");
        assert_eq!(decode_imap_mutf7("&XfJSIJZkkK5O9g-"), "已删除邮件");
        assert_eq!(decode_imap_mutf7("&YkBnCZCuTvY-"), "所有邮件");
        assert_eq!(decode_imap_mutf7("R&-D"), "R&D");
        assert_eq!(decode_imap_mutf7("&2D3eQg-"), "🙂");
        assert_eq!(decode_imap_mutf7("Tom & Jerry"), "Tom & Jerry");
        assert_eq!(decode_imap_mutf7("&!!!-"), "&!!!-");
        assert_eq!(decode_imap_mutf7("&"), "&");
        assert_eq!(decode_imap_mutf7("已加星标"), "已加星标");
    }

    #[test]
    fn resolve_imap_folders_filters_noselect_maps_special_use_and_decodes_leaves() {
        let (no_select, role) = classify_imap_attributes(&[
            NameAttribute::Extension("\\HasChildren".into()),
            NameAttribute::Extension("\\Noselect".into()),
        ]);
        assert!(no_select);
        assert_eq!(role, None);

        let entries = vec![
            RawImapFolderEntry {
                path: "INBOX".to_string(),
                delimiter: Some("/".to_string()),
                no_select: false,
                special_role: None,
            },
            RawImapFolderEntry {
                path: "[Gmail]".to_string(),
                delimiter: Some("/".to_string()),
                no_select: classify_imap_attributes(&[
                    NameAttribute::NoSelect,
                    NameAttribute::Extension("\\HasChildren".into()),
                ])
                .0,
                special_role: None,
            },
            RawImapFolderEntry {
                path: "[Gmail]/&g0l6Pw-".to_string(),
                delimiter: Some("/".to_string()),
                no_select: false,
                special_role: classify_imap_attributes(&[NameAttribute::Drafts]).1,
            },
            RawImapFolderEntry {
                path: "[Gmail]/&XfJT0ZABkK5O9g-".to_string(),
                delimiter: Some("/".to_string()),
                no_select: false,
                special_role: classify_imap_attributes(&[NameAttribute::Sent]).1,
            },
            RawImapFolderEntry {
                path: "[Gmail]/&XfJSIJZkkK5O9g-".to_string(),
                delimiter: Some("/".to_string()),
                no_select: false,
                special_role: classify_imap_attributes(&[NameAttribute::Trash]).1,
            },
            RawImapFolderEntry {
                path: "[Gmail]/&V4NXPpCuTvY-".to_string(),
                delimiter: Some("/".to_string()),
                no_select: false,
                special_role: classify_imap_attributes(&[NameAttribute::Junk]).1,
            },
            RawImapFolderEntry {
                path: "[Gmail]/&XfJSoGYfaAc-".to_string(),
                delimiter: Some("/".to_string()),
                no_select: false,
                special_role: classify_imap_attributes(&[NameAttribute::Flagged]).1,
            },
            RawImapFolderEntry {
                path: "[Gmail]/&kc2JgQ-".to_string(),
                delimiter: Some("/".to_string()),
                no_select: false,
                special_role: classify_imap_attributes(&[NameAttribute::Extension(
                    "\\Important".into(),
                )])
                .1,
            },
            RawImapFolderEntry {
                path: "[Gmail]/&YkBnCZCuTvY-".to_string(),
                delimiter: Some("/".to_string()),
                no_select: false,
                special_role: classify_imap_attributes(&[NameAttribute::All]).1,
            },
            // 同名顶层自定义标签不应在已有真实 \Trash 时冒充 "已删除"。
            RawImapFolderEntry {
                path: "Trash".to_string(),
                delimiter: Some("/".to_string()),
                no_select: false,
                special_role: None,
            },
            // 多级中文自定义目录保留完整 MUTF-7 路径，显示名取解码后叶子段。
            RawImapFolderEntry {
                path: "&XeVPXA-".to_string(),
                delimiter: Some("/".to_string()),
                no_select: false,
                special_role: None,
            },
            RawImapFolderEntry {
                path: "&XeVPXA-/&mHl27g-A".to_string(),
                delimiter: Some("/".to_string()),
                no_select: false,
                special_role: None,
            },
        ];

        let got = resolve_imap_folders(&entries);
        let pairs: Vec<(&str, &str)> = got
            .iter()
            .map(|f| (f.path.as_str(), f.name.as_str()))
            .collect();
        assert_eq!(
            pairs,
            vec![
                ("INBOX", "收件箱"),
                ("[Gmail]/&g0l6Pw-", "草稿箱"),
                ("[Gmail]/&XfJT0ZABkK5O9g-", "已发送"),
                ("[Gmail]/&XfJSIJZkkK5O9g-", "已删除"),
                ("[Gmail]/&V4NXPpCuTvY-", "垃圾邮件"),
                ("[Gmail]/&XfJSoGYfaAc-", "已加星标"),
                ("[Gmail]/&kc2JgQ-", "重要"),
                ("[Gmail]/&YkBnCZCuTvY-", "所有邮件"),
                ("Trash", "Trash"),
                ("&XeVPXA-", "工作"),
                ("&XeVPXA-/&mHl27g-A", "项目A"),
            ]
        );
    }

    #[test]
    fn resolve_imap_folders_falls_back_by_leaf_name_without_special_use() {
        let entries = vec![
            RawImapFolderEntry {
                path: "INBOX".to_string(),
                delimiter: Some(".".to_string()),
                no_select: false,
                special_role: None,
            },
            RawImapFolderEntry {
                path: "INBOX.Sent".to_string(),
                delimiter: Some(".".to_string()),
                no_select: false,
                special_role: None,
            },
            RawImapFolderEntry {
                path: "Archive".to_string(),
                delimiter: Some("/".to_string()),
                no_select: false,
                special_role: None,
            },
            RawImapFolderEntry {
                path: "Archive/2020/Sent".to_string(),
                delimiter: Some("/".to_string()),
                no_select: false,
                special_role: None,
            },
            RawImapFolderEntry {
                path: "Sent Messages".to_string(),
                delimiter: None,
                no_select: false,
                special_role: None,
            },
        ];

        let got = resolve_imap_folders(&entries);
        let pairs: Vec<(&str, &str)> = got
            .iter()
            .map(|f| (f.path.as_str(), f.name.as_str()))
            .collect();
        assert_eq!(
            pairs,
            vec![
                ("INBOX", "收件箱"),
                ("INBOX.Sent", "已发送"),
                ("Archive", "归档"),
                ("Archive/2020/Sent", "Sent"),
                ("Sent Messages", "已发送"),
            ]
        );
    }

    #[test]
    fn sanitize_html_strips_hidden_preheader_elements() {
        let raw = r#"<div style="display:none;max-height:0px;overflow:hidden;"><span>Hidden preheader</span></div><div style="visibility: hidden !important;">Also hidden</div><p hidden>Attr hidden</p><div style="font-size:0;opacity:0;">Zero hidden</div><p style="display:block;">Visible body</p>"#;
        let (clean, _) = sanitize_html_for_display_with_options(raw, false, false);
        assert!(!clean.contains("Hidden preheader"), "got: {clean}");
        assert!(!clean.contains("Also hidden"), "got: {clean}");
        assert!(!clean.contains("Attr hidden"), "got: {clean}");
        assert!(!clean.contains("Zero hidden"), "got: {clean}");
        assert!(clean.contains("Visible body"), "got: {clean}");
    }

    #[test]
    fn sanitize_html_recovers_from_unclosed_hidden_tags_at_boundaries() {
        let raw = concat!(
            r#"<table><tr><td><div style="display:none">Preheader<span>unclosed</td><td><p>Cell body</p></td></tr></table>"#,
            r#"<p style="display:none">Hidden p<p>Next p</p>"#,
            r#"<div style="display:none"><span>unclosed inner</div><p>After outer div</p>"#,
        );
        let (clean, _) = sanitize_html_for_display_with_options(raw, false, false);
        assert!(!clean.contains("Preheader"), "got: {clean}");
        assert!(!clean.contains("unclosed"), "got: {clean}");
        assert!(!clean.contains("Hidden p"), "got: {clean}");
        assert!(clean.contains("Cell body"), "got: {clean}");
        assert!(clean.contains("<p>Next p</p>"), "got: {clean}");
        assert!(clean.contains("<p>After outer div</p>"), "got: {clean}");
    }

    #[test]
    fn strip_remote_images_for_webview_preserves_styles_tables_and_inline_images() {
        let raw = concat!(
            "<html><head><style>.card { color: red; } /* <img src=\"https://fake\"> */</style></head>",
            "<body><!-- <img src=\"https://comment.example/a.png\"> -->",
            "<script>alert(1)</script>",
            "<table width=\"600\"><tr><td>",
            "<img alt=\"1 > 0\" SRC=\"HTTPS://tracker.example/pixel.png\" />",
            "<img srcset=\"data:image/png;base64,AAA 1x, https://cdn.example/b.png 2x\">",
            "<img alt=\"https://safe-alt.example\" src=\"data:image/png;base64,BBB\">",
            "<img src=\"cid:logo@example.com\">",
            "<p style=\"color: blue\">正文</p>",
            "</td></tr></table></body></html>"
        );
        let (stripped, blocked) = strip_remote_images_for_webview(raw);
        assert_eq!(blocked, 2, "stripped: {stripped}");
        assert!(stripped.contains("<style>.card { color: red; }"));
        assert!(stripped.contains("<table width=\"600\">"));
        assert!(stripped.contains("<p style=\"color: blue\">正文</p>"));
        assert!(stripped.contains("src=\"data:image/png;base64,BBB\""));
        assert!(stripped.contains("src=\"cid:logo@example.com\""));
        assert!(!stripped.contains("tracker.example"));
        assert!(!stripped.contains("cdn.example/b.png"));
        assert!(!stripped.contains("<script>"));
    }
}
