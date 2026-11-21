//! Async IMAP client supporting TLS and STARTTLS with MIME header/body parsing.

use std::time::Duration;

use async_native_tls::TlsConnector;
use futures::StreamExt;
use mail_parser::{MessageParser, MimeHeaders};
use tokio::net::TcpStream;

use crate::cache::{CachedAttachmentInput, CachedHeaderInput, RemoteFolder};
use crate::config::{AccountConfig, SecurityMode};

const IO_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_INITIAL_FETCH: usize = 100;

type TlsStream = async_native_tls::TlsStream<TcpStream>;
type ImapSession = async_imap::Session<TlsStream>;

async fn connect_imap(account: &AccountConfig, password: &str) -> Result<ImapSession, String> {
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

    let username = if account.imap_username.trim().is_empty() {
        account.email.as_str()
    } else {
        account.imap_username.as_str()
    };

    let session = client.login(username, password).await.map_err(|(e, _)| {
        let msg = e.to_string();
        if msg.contains("Basic authentication is disabled") {
            "该邮箱服务商已禁用传统密码登录（Basic authentication is disabled），Outlook / Microsoft 365 需通过 OAuth2 授权登录；请先使用 QQ 邮箱、网易 163 或支持应用专用密码的邮箱测试。".to_string()
        } else {
            format!("IMAP 登录失败（请检查用户名或授权码）: {msg}")
        }
    })?;

    Ok(session)
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

    let mut fetches = session
        .uid_fetch(&uid_list, "(UID FLAGS RFC822.HEADER)")
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
        let raw_header = fetch.header().unwrap_or_default();
        headers.push(parse_header_bytes(
            &parser, uid, unread, flagged, raw_header,
        ));
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

    let older_uids: Vec<u32> = live_uids
        .iter()
        .copied()
        .filter(|&u| u < before_uid)
        .collect();
    if older_uids.is_empty() {
        let _ = session.logout().await;
        return Ok((uid_validity, Vec::new(), live_uids, false));
    }

    let batch = limit.max(1);
    let start_idx = older_uids.len().saturating_sub(batch);
    let target_uids = &older_uids[start_idx..];
    let has_more = start_idx > 0;
    let headers = fetch_headers_for_uids(&mut session, target_uids).await?;
    let _ = session.logout().await;

    Ok((uid_validity, headers, live_uids, has_more))
}

/// Fetch full RFC 822 message body and attachments for `uid` in `folder_path`.
pub async fn fetch_message_body(
    account: &AccountConfig,
    password: &str,
    folder_path: &str,
    uid: u32,
) -> Result<(String, Option<String>, Vec<CachedAttachmentInput>), String> {
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

    Ok((body_text, body_html, attachments))
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

/// Move a message to another mailbox via `UID MOVE` (RFC 6851).
pub async fn move_message(
    account: &AccountConfig,
    password: &str,
    src_folder: &str,
    uid: u32,
    dest_folder: &str,
) -> Result<(), String> {
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
    let headers = tokio::time::timeout(search_timeout, fetch_headers_for_uids(&mut session, target_uids))
        .await
        .map_err(|_| "拉取服务端搜索结果邮件头超时".to_string())??;
    let _ = session.logout().await;
    Ok(headers)
}

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

    CachedHeaderInput {
        uid,
        message_id,
        in_reply_to,
        references_header,
        sender,
        recipients,
        subject,
        preview: "点击加载邮件正文...".to_string(),
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

/// Sanitize HTML email bodies for safe native display:
/// strips `<script>`, `<style>`, `<head>`, `<iframe>`, and HTML comments,
/// and optionally removes `<img>` elements referencing remote `http(s)://` URLs.
/// Returns `(sanitized_html, blocked_remote_image_count)`.
pub fn sanitize_html_for_display(html: &str, block_remote_images: bool) -> (String, usize) {
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
            out.push_str(trimmed);
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
}
