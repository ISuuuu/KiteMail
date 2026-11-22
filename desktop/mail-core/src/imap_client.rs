//! Async IMAP client supporting TLS and STARTTLS with MIME header/body parsing.

use std::time::Duration;

use async_native_tls::TlsConnector;
use futures::StreamExt;
use mail_parser::{MessageParser, MimeHeaders};
use tokio::net::TcpStream;

use crate::cache::{CachedAttachmentInput, CachedHeaderInput, RemoteFolder};
use crate::config::{AccountConfig, SecurityMode};
use crate::FetchedMessageBody;

const IO_TIMEOUT: Duration = Duration::from_secs(20);
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

    let mut session = client.login(username, password).await.map_err(|(e, _)| {
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
            "ID (\"name\" \"Hylki\" \"version\" \"1.0\" \"vendor\" \"Hylki\")",
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

    let mut folders = Vec::new();
    while let Some(item) = stream.next().await {
        if let Ok(name) = item {
            let path = name.name().to_string();
            let display = friendly_folder_name(&path);
            folders.push(RemoteFolder {
                name: display,
                path,
                unread_count: 0,
                total_count: 0,
            });
        }
    }
    drop(stream);
    let _ = session.logout().await;

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

    // 任务四：改拉全文前缀而非纯邮件头——前 4KB 覆盖头 + 正文开头，
    // mail_parser 提取出真实预览文本后历史邮件无需打开正文即可被搜索
    //（PEEK 不改变服务器端 \Seen 标记）。
    let mut fetches = session
        .uid_fetch(&uid_list, "(UID FLAGS BODY.PEEK[]<0.4096>)")
        .await
        .map_err(|e| format!("拉取邮件头失败: {e}"))?;

    let parser = MessageParser::default();
    let mut headers = Vec::with_capacity(target_uids.len());

    while let Some(fetch_res) = fetches.next().await {
        let Ok(fetch) = fetch_res else {
            continue;
        };
        let Some(uid) = fetch.uid else {
            continue;
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
        let raw = fetch.body().unwrap_or_default();
        headers.push(parse_header_bytes(&parser, uid, unread, flagged, raw));
    }
    drop(fetches);
    Ok(headers)
}

/// Fetch recent message headers and flags for `folder_path`.
/// Returns `(uid_validity, headers, live_uids)`.
pub async fn fetch_folder_headers(
    account: &AccountConfig,
    password: &str,
    folder_path: &str,
) -> Result<(u32, Vec<CachedHeaderInput>, Vec<u32>), String> {
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
        return Ok((uid_validity, Vec::new(), Vec::new()));
    }

    let start_idx = live_uids.len().saturating_sub(MAX_INITIAL_FETCH);
    let target_uids = &live_uids[start_idx..];
    let headers = fetch_headers_for_uids(&mut session, target_uids).await?;
    let _ = session.logout().await;

    Ok((uid_validity, headers, live_uids))
}

/// Fetch up to `limit` older message headers with `uid < before_uid` for `folder_path`.
/// Returns `(uid_validity, headers, live_uids, has_more_older)`.
pub async fn fetch_older_folder_headers(
    account: &AccountConfig,
    password: &str,
    folder_path: &str,
    before_uid: u32,
    limit: usize,
) -> Result<(u32, Vec<CachedHeaderInput>, Vec<u32>, bool), String> {
    let mut session = connect_imap(account, password).await?;
    // 任务四：历史批次跑在后台，每一步网络调用都必须有限超时，
    // 避免一次卡住的服务器响应占住单队列。
    let mailbox = tokio::time::timeout(IO_TIMEOUT, session.select(folder_path))
        .await
        .map_err(|_| format!("打开文件夹 {folder_path} 超时"))?
        .map_err(|e| format!("打开文件夹 {folder_path} 失败: {e}"))?;

    let uid_validity = mailbox.uid_validity.unwrap_or(1);
    let uids_set = tokio::time::timeout(IO_TIMEOUT, session.uid_search("ALL"))
        .await
        .map_err(|_| "搜索邮件 UID 超时".to_string())?
        .map_err(|e| format!("搜索邮件 UID 失败: {e}"))?;

    let mut live_uids: Vec<u32> = uids_set.into_iter().collect();
    live_uids.sort_unstable();

    let older_uids: Vec<u32> = live_uids
        .iter()
        .copied()
        .filter(|&u| u < before_uid)
        .collect();
    if older_uids.is_empty() {
        let _ = tokio::time::timeout(IO_TIMEOUT, session.logout()).await;
        return Ok((uid_validity, Vec::new(), live_uids, false));
    }

    let batch = limit.max(1);
    let start_idx = older_uids.len().saturating_sub(batch);
    let target_uids = &older_uids[start_idx..];
    let has_more = start_idx > 0;
    let headers = tokio::time::timeout(
        IO_TIMEOUT,
        fetch_headers_for_uids(&mut session, target_uids),
    )
    .await
    .map_err(|_| format!("拉取历史邮件头超时 ({folder_path})"))??;
    let _ = tokio::time::timeout(IO_TIMEOUT, session.logout()).await;

    Ok((uid_validity, headers, live_uids, has_more))
}

/// Fetch full RFC 822 message body and attachments for `uid` in `folder_path`.
pub async fn fetch_message_body(
    account: &AccountConfig,
    password: &str,
    folder_path: &str,
    uid: u32,
) -> Result<FetchedMessageBody, String> {
    let mut session = connect_imap(account, password).await?;
    session
        .select(folder_path)
        .await
        .map_err(|e| format!("打开文件夹 {folder_path} 失败: {e}"))?;

    let mut fetches = session
        .uid_fetch(uid.to_string(), "RFC822")
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
    drop(fetches);
    let _ = session.logout().await;

    let bytes = raw_body.ok_or_else(|| format!("服务器未返回 UID {uid} 的邮件内容"))?;
    let parsed = MessageParser::default()
        .parse(&bytes)
        .ok_or_else(|| "解析邮件 MIME 内容失败".to_string())?;

    let body_html = parsed.body_html(0).map(|h| h.into_owned());
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
        attachments.push(CachedAttachmentInput {
            filename,
            mime_type,
            content: att.contents().to_vec(),
        });
    }

    // F-7 一键退订：`List-Unsubscribe` / `List-Unsubscribe-Post` 头随正文抓取
    // 一并读出（header_raw 含字段名，剥出值后再提取 URL）。
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

    Ok(FetchedMessageBody {
        body_text,
        body_html,
        attachments,
        unsubscribe_url,
        unsubscribe_one_click,
    })
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

/// Mark a message `\Deleted` and expunge the mailbox. Fallback for deleting
/// when the server has no dedicated trash folder.
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
    let mut stores = session
        .uid_store(uid.to_string(), "+FLAGS.SILENT (\\Deleted)")
        .await
        .map_err(|e| format!("标记邮件删除失败: {e}"))?;
    while stores.next().await.is_some() {}
    drop(stores);
    {
        let mut expunged = std::pin::pin!(session
            .expunge()
            .await
            .map_err(|e| format!("执行 EXPUNGE 失败: {e}"))?);
        while expunged.as_mut().next().await.is_some() {}
    }
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

fn friendly_folder_name(path: &str) -> String {
    match path.to_ascii_uppercase().as_str() {
        "INBOX" => "收件箱".to_string(),
        "SENT" | "SENT MESSAGES" | "SENT ITEMS" => "已发送".to_string(),
        "DRAFTS" => "草稿箱".to_string(),
        "TRASH" | "DELETED" | "DELETED MESSAGES" => "已删除".to_string(),
        "JUNK" | "SPAM" => "垃圾邮件".to_string(),
        "ARCHIVE" => "归档".to_string(),
        _ => path.to_string(),
    }
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

/// Strip common tracking query parameters from `http(s)://` URLs while preserving
/// legitimate parameters and URL fragments.
pub fn strip_tracking_params(raw_url: &str) -> String {
    let Ok(mut parsed) = url::Url::parse(raw_url.trim()) else {
        return raw_url.to_string();
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return raw_url.to_string();
    }
    let kept: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(k, _)| !is_tracking_param(k))
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    if parsed.query().is_none() {
        return raw_url.to_string();
    }
    if kept.is_empty() {
        parsed.set_query(None);
    } else {
        let mut ser = parsed.query_pairs_mut();
        ser.clear();
        for (k, v) in kept {
            ser.append_pair(&k, &v);
        }
        drop(ser);
    }
    parsed.to_string()
}

fn is_tracking_param(key: &str) -> bool {
    let k = key.to_ascii_lowercase();
    k.starts_with("utm_")
        || matches!(
            k.as_str(),
            "fbclid"
                | "gclid"
                | "dclid"
                | "msclkid"
                | "mc_cid"
                | "mc_eid"
                | "igshid"
                | "spm"
                | "yclid"
                | "ref_src"
                | "_hsenc"
                | "_hsmi"
                | "mkt_tok"
        )
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

/// Sanitize HTML email bodies for safe native display:
/// strips `<script>`, `<style>`, `<head>`, `<iframe>`, and HTML comments,
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

            if lower.starts_with("!--") {
                continue;
            }
            if let Some(active) = skip_block {
                if is_closing && tag_name == active {
                    skip_block = None;
                }
                continue;
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
        if skip_block.is_none() {
            out.push(ch);
        }
    }

    (out, blocked_images)
}

/// Sanitize HTML email bodies for safe native display (legacy default without link cleaning).
pub fn sanitize_html_for_display(html: &str, block_remote_images: bool) -> (String, usize) {
    sanitize_html_for_display_with_options(html, block_remote_images, false)
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
}
