//! Microsoft Graph REST mail client (folders, messages, attachments, flags, sendMail).

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::Path;

use base64::Engine;

use crate::cache::{CachedAttachmentInput, CachedHeaderInput, RemoteFolder};
use crate::imap_client::html_to_readable_text;
use crate::smtp_client::SendOutcome;
use crate::{FetchedMessageBody, AUTO_CACHE_ATTACHMENT_MAX_BYTES};

const MAX_INLINE_CID_BYTES: usize = 5 * 1024 * 1024;

const GRAPH_BASE: &str = "https://graph.microsoft.com/v1.0/me";

fn validate_graph_url(value: &str) -> Result<(), String> {
    let url = url::Url::parse(value).map_err(|_| "无效的 Graph 分页地址".to_string())?;
    if url.scheme() != "https"
        || url.host_str() != Some("graph.microsoft.com")
        || url.port().is_some_and(|p| p != 443)
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("不可信的 Graph 分页地址".to_string());
    }
    Ok(())
}

fn graph_agent() -> ureq::Agent {
    ureq::AgentBuilder::new().redirects(0).build()
}

fn require_graph_success(response: ureq::Response) -> Result<ureq::Response, String> {
    let status = response.status();
    if !(200..300).contains(&status) {
        return Err(format!("Graph API 返回非成功状态 (HTTP {status})"));
    }
    Ok(response)
}

fn graph_send_success(response: ureq::Response) -> SendOutcome {
    let status = response.status();
    if (200..300).contains(&status) {
        SendOutcome::Accepted
    } else {
        SendOutcome::Uncertain(format!("Graph 发送结果待确认 (HTTP {status})"))
    }
}

fn format_graph_error(err: ureq::Error) -> String {
    match err {
        ureq::Error::Status(code, _) => format!("Graph API 错误 (HTTP {code})"),
        ureq::Error::Transport(t) => format!("Graph API 网络错误: {:?}", t.kind()),
    }
}

/// Stable positive 32-bit local UID derived from a Graph message ID string.
fn uid_from_remote_id(remote_id: &str) -> u32 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in remote_id.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let val = (hash as u32) ^ ((hash >> 32) as u32);
    if val == 0 {
        1
    } else {
        val
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum GraphWellKnownRole {
    Inbox,
    Sent,
    Drafts,
    Trash,
    Junk,
    Archive,
}

impl GraphWellKnownRole {
    const ALL: [(&'static str, Self); 6] = [
        ("inbox", Self::Inbox),
        ("sentitems", Self::Sent),
        ("drafts", Self::Drafts),
        ("deleteditems", Self::Trash),
        ("junkemail", Self::Junk),
        ("archive", Self::Archive),
    ];

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

    fn from_display_name(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "inbox" | "收件箱" => Some(Self::Inbox),
            "sent items" | "sent" | "已发送邮件" | "已发送" => Some(Self::Sent),
            "drafts" | "草稿" => Some(Self::Drafts),
            "deleted items" | "trash" | "已删除邮件" => Some(Self::Trash),
            "junk email" | "spam" | "垃圾邮件" => Some(Self::Junk),
            "archive" | "存档" => Some(Self::Archive),
            _ => None,
        }
    }
}

/// 查询 Graph 系统保留角色文件夹 ID（`inbox` / `sentitems` / `drafts` / `deleteditems` /
/// `junkemail` / `archive`），用于在列表存在同名自定义文件夹（如第三方客户端建的 `Trash`）时
/// 准确识别系统文件夹，避免冒充标准角色。单项 404 或网络失败静默跳过。
fn fetch_well_known_roles(access_token: &str) -> HashMap<String, GraphWellKnownRole> {
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout(std::time::Duration::from_secs(10))
        .build();
    let auth = format!("Bearer {access_token}");
    let mut roles = HashMap::new();
    // 分两批每批最多 3 并发，低于 Exchange Online 单邮箱 4 并发上限，避免触发 429 限流。
    for chunk in GraphWellKnownRole::ALL.chunks(3) {
        let batch: Vec<(String, GraphWellKnownRole)> = std::thread::scope(|s| {
            let handles: Vec<_> = chunk
                .iter()
                .map(|&(wk, role)| {
                    let agent = &agent;
                    let auth = &auth;
                    s.spawn(move || {
                        let url = format!("{GRAPH_BASE}/mailFolders/{wk}?$select=id");
                        let resp = agent
                            .get(&url)
                            .set("Authorization", auth)
                            .call()
                            .ok()
                            .and_then(|r| require_graph_success(r).ok())?;
                        let json: serde_json::Value = resp.into_json().ok()?;
                        let id = json["id"].as_str()?.trim();
                        if id.is_empty() {
                            return None;
                        }
                        Some((id.to_string(), role))
                    })
                })
                .collect();
            handles
                .into_iter()
                .filter_map(|h| h.join().ok().flatten())
                .collect()
        });
        for (id, role) in batch {
            roles.insert(id, role);
        }
    }
    roles
}

fn resolve_graph_folder_name(
    id: &str,
    raw_name: &str,
    well_known_by_id: &HashMap<String, GraphWellKnownRole>,
    present_roles: &HashSet<GraphWellKnownRole>,
) -> (String, bool) {
    if let Some(&role) = well_known_by_id.get(id) {
        return (
            role.friendly_name().to_string(),
            role == GraphWellKnownRole::Inbox,
        );
    }
    if let Some(candidate) = GraphWellKnownRole::from_display_name(raw_name) {
        // 该角色没有实际存在的系统保留文件夹时，回退按名称映射以保持旧行为兼容；
        // 已有真正系统文件夹时保留原始名称，防止非系统文件夹冒充标准角色。
        if !present_roles.contains(&candidate) {
            return (
                candidate.friendly_name().to_string(),
                candidate == GraphWellKnownRole::Inbox,
            );
        }
    }
    (raw_name.to_string(), false)
}

const MAX_GRAPH_FOLDER_PAGES: usize = 20;

fn parse_graph_folder_items(
    items: &[serde_json::Value],
    well_known_by_id: &HashMap<String, GraphWellKnownRole>,
) -> Vec<(RemoteFolder, bool)> {
    let present_roles: HashSet<GraphWellKnownRole> = items
        .iter()
        .filter_map(|item| item["id"].as_str())
        .filter_map(|id| well_known_by_id.get(id).copied())
        .collect();

    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let Some(id) = item["id"].as_str() else {
            continue;
        };
        let raw_name = item["displayName"].as_str().unwrap_or("Folder");
        let (friendly, is_inbox) =
            resolve_graph_folder_name(id, raw_name, well_known_by_id, &present_roles);
        let unread = item["unreadItemCount"].as_u64().unwrap_or(0) as u32;
        let total = item["totalItemCount"].as_u64().unwrap_or(0) as usize;
        out.push((
            RemoteFolder {
                name: friendly,
                path: id.to_string(),
                unread_count: unread,
                total_count: total,
            },
            is_inbox,
        ));
    }
    out
}

#[cfg(test)]
fn parse_graph_folders_json(
    json: &serde_json::Value,
    well_known_by_id: &HashMap<String, GraphWellKnownRole>,
) -> Vec<(RemoteFolder, bool)> {
    let Some(arr) = json["value"].as_array() else {
        return Vec::new();
    };
    parse_graph_folder_items(arr, well_known_by_id)
}

/// Fetch mail folders via Microsoft Graph (following `@odata.nextLink` up to
/// [`MAX_GRAPH_FOLDER_PAGES`] pages so large mailboxes are never truncated).
pub fn fetch_folders(access_token: &str) -> Result<Vec<(RemoteFolder, bool)>, String> {
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout(std::time::Duration::from_secs(15))
        .build();
    let auth = format!("Bearer {access_token}");
    let mut next_url = Some(format!(
        "{GRAPH_BASE}/mailFolders?$top=100&$select=id,displayName,unreadItemCount,totalItemCount"
    ));
    let mut seen_urls = HashSet::new();
    let mut all_items = Vec::new();

    for _ in 0..MAX_GRAPH_FOLDER_PAGES {
        let Some(url) = next_url.take() else {
            break;
        };
        if !seen_urls.insert(url.clone()) {
            break;
        }
        validate_graph_url(&url)?;
        let resp = agent
            .get(&url)
            .set("Authorization", &auth)
            .call()
            .map_err(format_graph_error)
            .and_then(require_graph_success)?;
        let json: serde_json::Value = resp
            .into_json()
            .map_err(|e| format!("解析 Graph 文件夹响应失败: {e}"))?;
        if let Some(arr) = json["value"].as_array() {
            all_items.extend(arr.iter().cloned());
        }
        next_url = json["@odata.nextLink"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string);
    }

    let well_known = fetch_well_known_roles(access_token);
    Ok(parse_graph_folder_items(&all_items, &well_known))
}

/// Fetch inbox unread/total counts via a single lightweight Graph well-known query.
pub(crate) fn fetch_inbox_counts(access_token: &str) -> Result<(u32, usize), String> {
    let url = format!("{GRAPH_BASE}/mailFolders/inbox?$select=unreadItemCount,totalItemCount");
    let resp = graph_agent()
        .get(&url)
        .timeout(std::time::Duration::from_secs(15))
        .set("Authorization", &format!("Bearer {access_token}"))
        .call()
        .map_err(format_graph_error)
        .and_then(require_graph_success)?;
    let json: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("解析 Graph 收件箱计数失败: {e}"))?;
    let unread = json["unreadItemCount"].as_u64().unwrap_or(0) as u32;
    let total = json["totalItemCount"].as_u64().unwrap_or(0) as usize;
    Ok((unread, total))
}

/// Fetch recent message headers from a Graph folder.
/// Returns `(items, next_link)` where `next_link` is Graph's `@odata.nextLink` when more pages exist.
pub fn fetch_folder_headers(
    access_token: &str,
    folder_graph_id: &str,
) -> Result<(Vec<(CachedHeaderInput, String)>, Option<String>), String> {
    let encoded_folder = urlencoding_segment(folder_graph_id);
    let url = format!(
        "{GRAPH_BASE}/mailFolders/{encoded_folder}/messages?\
         $top=100&$orderby=receivedDateTime%20desc&\
         $select=id,subject,from,toRecipients,ccRecipients,receivedDateTime,isRead,flag,hasAttachments,bodyPreview,internetMessageId,conversationId"
    );
    fetch_headers_from_url(access_token, &url)
}

/// Fetch the next page of message headers from a Graph `@odata.nextLink` URL.
pub fn fetch_folder_headers_next(
    access_token: &str,
    next_link_url: &str,
) -> Result<(Vec<(CachedHeaderInput, String)>, Option<String>), String> {
    fetch_headers_from_url(access_token, next_link_url)
}

/// Search messages across the mailbox using Microsoft Graph `$search` with timeout protection.
pub fn search_messages(
    access_token: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<(CachedHeaderInput, String)>, String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let sanitized = trimmed.replace('"', " ");
    let quoted = format!("\"{sanitized}\"");
    let encoded_search = urlencoding_segment(&quoted);
    let top = limit.clamp(1, 100);
    let url = format!(
        "{GRAPH_BASE}/messages?$search={encoded_search}&$top={top}&\
         $select=id,subject,from,toRecipients,ccRecipients,receivedDateTime,isRead,flag,hasAttachments,bodyPreview,internetMessageId"
    );
    let resp = graph_agent()
        .get(&url)
        .timeout(std::time::Duration::from_secs(15))
        .set("Authorization", &format!("Bearer {access_token}"))
        .set("ConsistencyLevel", "eventual")
        .call()
        .map_err(format_graph_error)
        .and_then(require_graph_success)?;

    let json: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("解析 Graph 搜索结果失败: {e}"))?;

    let (items, _) = parse_graph_messages_json(&json);
    Ok(items)
}

fn fetch_headers_from_url(
    access_token: &str,
    url: &str,
) -> Result<(Vec<(CachedHeaderInput, String)>, Option<String>), String> {
    // 任务四：历史批次跑在后台，请求必须有限超时（对齐 search_messages 的 15 秒），
    // 避免一次卡住的响应占住单队列。
    validate_graph_url(url)?;
    let resp = graph_agent()
        .get(url)
        .timeout(std::time::Duration::from_secs(15))
        .set("Authorization", &format!("Bearer {access_token}"))
        .call()
        .map_err(format_graph_error)
        .and_then(require_graph_success)?;

    let json: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("解析 Graph 邮件列表失败: {e}"))?;

    Ok(parse_graph_messages_json(&json))
}

fn parse_graph_messages_json(
    json: &serde_json::Value,
) -> (Vec<(CachedHeaderInput, String)>, Option<String>) {
    let next_link = json["@odata.nextLink"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);

    let mut items = Vec::new();
    if let Some(arr) = json["value"].as_array() {
        for msg in arr {
            let Some(remote_id) = msg["id"].as_str() else {
                continue;
            };
            let uid = uid_from_remote_id(remote_id);
            let message_id = msg["internetMessageId"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            let conversation_id = msg["conversationId"]
                .as_str()
                .unwrap_or_default()
                .trim()
                .to_string();
            let sender = format_graph_address(&msg["from"]["emailAddress"]);
            let recipients = msg["toRecipients"]
                .as_array()
                .map(|list| {
                    list.iter()
                        .map(|r| format_graph_address(&r["emailAddress"]))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let cc = msg["ccRecipients"]
                .as_array()
                .map(|list| {
                    list.iter()
                        .map(|r| format_graph_address(&r["emailAddress"]))
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let subject = msg["subject"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("(无主题)")
                .to_string();
            let preview = msg["bodyPreview"]
                .as_str()
                .unwrap_or_default()
                .replace('\r', " ")
                .replace('\n', " ");

            let received = msg["receivedDateTime"].as_str().unwrap_or_default();
            let (date_ts, date_display) = chrono::DateTime::parse_from_rfc3339(received)
                .map(|dt| {
                    let ts = dt.timestamp();
                    let disp = dt
                        .with_timezone(&chrono::Local)
                        .format("%m-%d %H:%M")
                        .to_string();
                    (ts, disp)
                })
                .unwrap_or((uid as i64, String::new()));

            let unread = !msg["isRead"].as_bool().unwrap_or(false);
            let flagged = msg["flag"]["flagStatus"].as_str() == Some("flagged");
            let has_attachment = msg["hasAttachments"].as_bool().unwrap_or(false);

            items.push((
                CachedHeaderInput {
                    uid,
                    message_id,
                    in_reply_to: String::new(),
                    references_header: conversation_id,
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
                },
                remote_id.to_string(),
            ));
        }
    }

    (items, next_link)
}

/// Fetch message body, inline `cid:` images, and auto-cached small attachments via Microsoft Graph.
/// Attachments larger than `AUTO_CACHE_ATTACHMENT_MAX_BYTES` store metadata only (`content: None`).
pub fn fetch_message_body(
    access_token: &str,
    remote_id: &str,
) -> Result<FetchedMessageBody, String> {
    let encoded_id = urlencoding_segment(remote_id);
    let url = format!(
        "{GRAPH_BASE}/messages/{encoded_id}?$select=body,hasAttachments,internetMessageHeaders"
    );
    let resp = graph_agent()
        .get(&url)
        .set("Authorization", &format!("Bearer {access_token}"))
        .call()
        .map_err(format_graph_error)
        .and_then(require_graph_success)?;

    let json: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("解析 Graph 邮件正文失败: {e}"))?;

    let content_type = json["body"]["contentType"].as_str().unwrap_or("text");
    let raw_content = json["body"]["content"].as_str().unwrap_or("");
    let is_html = content_type.eq_ignore_ascii_case("html");
    let mut body_html = if is_html {
        Some(raw_content.to_string())
    } else {
        None
    };

    let mut cid_map: HashMap<String, String> = HashMap::new();
    let mut attachments = Vec::new();
    if json["hasAttachments"].as_bool().unwrap_or(false) {
        let att_url = format!(
            "{GRAPH_BASE}/messages/{encoded_id}/attachments?$select=id,name,contentType,size,isInline"
        );
        let att_resp = graph_agent()
            .get(&att_url)
            .set("Authorization", &format!("Bearer {access_token}"))
            .call()
            .map_err(format_graph_error)
            .and_then(require_graph_success)?;
        let att_json: serde_json::Value = att_resp
            .into_json()
            .map_err(|e| format!("解析 Graph 附件元数据失败: {e}"))?;
        if let Some(arr) = att_json["value"].as_array() {
            for att in arr {
                let att_id = att["id"].as_str().unwrap_or("").trim();
                let filename = att["name"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or("attachment.bin")
                    .to_string();
                let mime_type = att["contentType"]
                    .as_str()
                    .filter(|s| !s.trim().is_empty())
                    .unwrap_or("application/octet-stream")
                    .to_string();
                let size_bytes = att["size"].as_u64().unwrap_or(0) as usize;
                let is_inline = att["isInline"].as_bool().unwrap_or(false);

                if is_inline {
                    if size_bytes <= MAX_INLINE_CID_BYTES && !att_id.is_empty() {
                        let encoded_att_id = urlencoding_segment(att_id);
                        let item_url = format!(
                            "{GRAPH_BASE}/messages/{encoded_id}/attachments/{encoded_att_id}"
                        );
                        if let Ok(item_resp) = graph_agent()
                            .get(&item_url)
                            .set("Authorization", &format!("Bearer {access_token}"))
                            .call()
                            .map_err(format_graph_error)
                            .and_then(require_graph_success)
                        {
                            if let Ok(item_json) = item_resp.into_json::<serde_json::Value>() {
                                if let (Some(cid_raw), Some(b64)) = (
                                    item_json["contentId"].as_str(),
                                    item_json["contentBytes"].as_str(),
                                ) {
                                    let cid = cid_raw
                                        .trim()
                                        .trim_matches(|c| c == '<' || c == '>')
                                        .to_ascii_lowercase();
                                    if !cid.is_empty()
                                        && mime_type.to_ascii_lowercase().starts_with("image/")
                                    {
                                        cid_map
                                            .insert(cid, format!("data:{mime_type};base64,{b64}"));
                                        continue;
                                    }
                                }
                                if let Some(b64) = item_json["contentBytes"].as_str() {
                                    if let Ok(bytes) =
                                        base64::engine::general_purpose::STANDARD.decode(b64)
                                    {
                                        attachments.push(CachedAttachmentInput::downloaded(
                                            filename, mime_type, bytes,
                                        ));
                                    }
                                }
                            }
                        }
                    }
                    continue;
                }

                if size_bytes <= AUTO_CACHE_ATTACHMENT_MAX_BYTES && !att_id.is_empty() {
                    if let Ok(bytes) = fetch_attachment_content(access_token, remote_id, att_id) {
                        attachments.push(CachedAttachmentInput::downloaded(
                            filename, mime_type, bytes,
                        ));
                        continue;
                    }
                }

                if !att_id.is_empty() {
                    attachments.push(CachedAttachmentInput {
                        filename,
                        mime_type,
                        size_bytes,
                        content: None,
                        remote_part: att_id.to_string(),
                        transfer_encoding: "graph".to_string(),
                    });
                }
            }
        }
    }

    if !cid_map.is_empty() {
        body_html = body_html.map(|html| crate::mime::replace_cid_urls(html, &cid_map));
    }
    let body_text = if let Some(ref html) = body_html {
        html_to_readable_text(html)
    } else {
        raw_content.to_string()
    };

    // F-7 一键退订：internetMessageHeaders 中查找退订相关头（名称大小写不敏感）。
    let (unsubscribe_url, unsubscribe_one_click, auth_results) =
        parse_graph_headers(&json["internetMessageHeaders"]);

    Ok(FetchedMessageBody {
        body_text,
        body_html,
        attachments,
        unsubscribe_url,
        unsubscribe_one_click,
        auth_results,
    })
}

fn parse_graph_headers(headers: &serde_json::Value) -> (Option<String>, bool, Option<String>) {
    let header_value = |name: &str| -> Option<String> {
        headers
            .as_array()?
            .iter()
            .find(|h| {
                h["name"]
                    .as_str()
                    .is_some_and(|n| n.eq_ignore_ascii_case(name))
            })
            .and_then(|h| h["value"].as_str().map(str::trim).map(ToString::to_string))
    };
    let unsubscribe_url =
        header_value("List-Unsubscribe").and_then(|v| crate::cache::extract_unsubscribe_url(&v));
    let unsubscribe_one_click = header_value("List-Unsubscribe-Post")
        .map(|v| crate::cache::is_one_click_post(&v))
        .unwrap_or(false);
    let auth_results = header_value("Authentication-Results").filter(|v| !v.is_empty());
    (unsubscribe_url, unsubscribe_one_click, auth_results)
}

/// On-demand download of a single Graph file attachment's raw bytes.
pub fn fetch_attachment_content(
    access_token: &str,
    remote_msg_id: &str,
    remote_att_id: &str,
) -> Result<Vec<u8>, String> {
    let encoded_msg = urlencoding_segment(remote_msg_id);
    let encoded_att = urlencoding_segment(remote_att_id);
    let url = format!("{GRAPH_BASE}/messages/{encoded_msg}/attachments/{encoded_att}");
    let resp = graph_agent()
        .get(&url)
        .set("Authorization", &format!("Bearer {access_token}"))
        .timeout(std::time::Duration::from_secs(180))
        .call()
        .map_err(format_graph_error)
        .and_then(require_graph_success)?;
    let json: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("解析 Graph 附件内容失败: {e}"))?;
    let b64 = json["contentBytes"]
        .as_str()
        .ok_or_else(|| "Graph 附件未包含可下载的字节内容".to_string())?;
    base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("解码 Graph 附件 Base64 失败: {e}"))
}

/// 只取正文供搜索索引；无附件且响应未超限时顺便返回完整 [`FetchedMessageBody`] 供缓存。
pub fn fetch_index_text(
    access_token: &str,
    remote_id: &str,
) -> Result<(String, Option<FetchedMessageBody>), String> {
    const MAX_BYTES: usize = 262_144;
    let encoded_id = urlencoding_segment(remote_id);
    let url = format!(
        "{GRAPH_BASE}/messages/{encoded_id}?$select=body,hasAttachments,internetMessageHeaders"
    );
    let response = graph_agent()
        .get(&url)
        .set("Authorization", &format!("Bearer {access_token}"))
        .timeout(std::time::Duration::from_secs(20))
        .call()
        .map_err(format_graph_error)
        .and_then(require_graph_success)?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("读取 Graph 索引正文失败: {e}"))?;
    if bytes.len() > MAX_BYTES {
        return Ok((String::new(), None));
    }
    let json: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("解析 Graph 索引正文失败: {e}"))?;
    let content = json["body"]["content"].as_str().unwrap_or("");
    let is_html = json["body"]["contentType"]
        .as_str()
        .unwrap_or("text")
        .eq_ignore_ascii_case("html");
    let text = if is_html {
        html_to_readable_text(content)
    } else {
        content.to_string()
    };
    let has_attachments = json["hasAttachments"].as_bool().unwrap_or(false);
    let full = if !has_attachments {
        let (unsubscribe_url, unsubscribe_one_click, auth_results) =
            parse_graph_headers(&json["internetMessageHeaders"]);
        Some(FetchedMessageBody {
            body_text: text.clone(),
            body_html: is_html.then(|| content.to_string()),
            attachments: Vec::new(),
            unsubscribe_url,
            unsubscribe_one_click,
            auth_results,
        })
    } else {
        None
    };
    Ok((text, full))
}

/// Fetch the complete raw MIME message via Graph `/$value` (used for .eml export).
pub fn fetch_raw_message(access_token: &str, remote_id: &str) -> Result<Vec<u8>, String> {
    let encoded_id = urlencoding_segment(remote_id);
    let url = format!("{GRAPH_BASE}/messages/{encoded_id}/$value");
    let resp = graph_agent()
        .get(&url)
        .set("Authorization", &format!("Bearer {access_token}"))
        .call()
        .map_err(format_graph_error)
        .and_then(require_graph_success)?;
    let mut reader = resp.into_reader();
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(|e| format!("读取 Graph 原始邮件失败: {e}"))?;
    Ok(bytes)
}

/// F-3 撤销：按 `internetMessageId` 在 `current_folder_id` 中检索邮件并移回
/// `original_folder_id`（移动 / 删除后的邮件已不在本地缓存，需服务端检索定位）。
pub fn undo_move_by_message_id(
    access_token: &str,
    current_folder_id: &str,
    rfc_message_id: &str,
    original_folder_id: &str,
) -> Result<(), String> {
    let id = rfc_message_id.trim();
    if id.is_empty() {
        return Err("缺少 Message-ID，无法撤销移动".to_string());
    }
    let encoded_folder = urlencoding_segment(current_folder_id);
    let filter = format!("internetMessageId eq '{id}'");
    let encoded_filter = urlencoding_segment(&filter);
    let url = format!(
        "{GRAPH_BASE}/mailFolders/{encoded_folder}/messages?$filter={encoded_filter}&$select=id&$top=1"
    );
    let resp = graph_agent()
        .get(&url)
        .set("Authorization", &format!("Bearer {access_token}"))
        .call()
        .map_err(format_graph_error)
        .and_then(require_graph_success)?;
    let json: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("解析 Graph 撤销检索结果失败: {e}"))?;
    let Some(remote_id) = json["value"]
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|msg| msg["id"].as_str())
        .map(str::to_string)
    else {
        return Err("在当前文件夹中未找到该邮件，无法撤销移动".to_string());
    };
    move_message(access_token, &remote_id, original_folder_id)
}

/// Update read (`isRead`) state on Microsoft Graph.
pub fn set_seen(access_token: &str, remote_id: &str, seen: bool) -> Result<(), String> {
    let encoded_id = urlencoding_segment(remote_id);
    let url = format!("{GRAPH_BASE}/messages/{encoded_id}");
    graph_agent()
        .patch(&url)
        .set("Authorization", &format!("Bearer {access_token}"))
        .send_json(serde_json::json!({ "isRead": seen }))
        .map_err(format_graph_error)
        .and_then(require_graph_success)?;
    Ok(())
}

/// Update flagged (`flag.flagStatus`) state on Microsoft Graph.
pub fn set_flagged(access_token: &str, remote_id: &str, flagged: bool) -> Result<(), String> {
    let encoded_id = urlencoding_segment(remote_id);
    let url = format!("{GRAPH_BASE}/messages/{encoded_id}");
    let status = if flagged { "flagged" } else { "notFlagged" };
    graph_agent()
        .patch(&url)
        .set("Authorization", &format!("Bearer {access_token}"))
        .send_json(serde_json::json!({ "flag": { "flagStatus": status } }))
        .map_err(format_graph_error)
        .and_then(require_graph_success)?;
    Ok(())
}

/// Move a message to another folder via Microsoft Graph.
/// `dest_folder_id` accepts a Graph folder ID or a well-known folder name
/// such as `deleteditems`.
pub fn move_message(
    access_token: &str,
    remote_id: &str,
    dest_folder_id: &str,
) -> Result<(), String> {
    if dest_folder_id.trim().is_empty() {
        return Err("目标文件夹 ID 不能为空".to_string());
    }
    let encoded_id = urlencoding_segment(remote_id);
    let url = format!("{GRAPH_BASE}/messages/{encoded_id}/move");
    graph_agent()
        .post(&url)
        .set("Authorization", &format!("Bearer {access_token}"))
        .send_json(serde_json::json!({ "destinationId": dest_folder_id }))
        .map_err(format_graph_error)
        .and_then(require_graph_success)?;
    Ok(())
}

/// Send an email (with optional attachments) via Microsoft Graph `sendMail`.
pub fn send_message(
    access_token: &str,
    to_addr: &str,
    cc_addrs: &str,
    bcc_addrs: &str,
    reply_to: Option<&str>,
    subject: &str,
    body_text: &str,
    // WP-10-3：按正文格式预先算好的 HTML；`Some` 时以 `contentType: Html` 发送。
    body_html: Option<&str>,
    attachment_paths: &[String],
) -> SendOutcome {
    let to_list = graph_recipient_array(to_addr);
    if to_list.is_empty() {
        return SendOutcome::Failed("Recipient address is empty".to_string());
    }

    let mut graph_attachments = Vec::new();
    for path_str in attachment_paths {
        let path = Path::new(path_str);
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("attachment.bin")
            .to_string();
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                return SendOutcome::Failed(format!("读取附件失败 ({}): {e}", path.display()))
            }
        };
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        graph_attachments.push(serde_json::json!({
            "@odata.type": "#microsoft.graph.fileAttachment",
            "name": name,
            "contentBytes": b64
        }));
    }

    // 正文里的 data: 图片提升成 cid: 内联附件；Graph 用 contentId + isInline
    // 把附件标成内联，否则收件端只会看到红叉。
    let (body_html, inline_images) = match body_html {
        Some(html) => {
            let (html, images) = crate::inline::extract_data_images(html);
            (Some(html), images)
        }
        None => (None, Vec::new()),
    };
    for image in inline_images {
        graph_attachments.push(serde_json::json!({
            "@odata.type": "#microsoft.graph.fileAttachment",
            "name": image.filename,
            "contentId": image.content_id,
            "isInline": true,
            "contentBytes": base64::engine::general_purpose::STANDARD.encode(&image.data)
        }));
    }

    // 有 HTML 正文时按 HTML 发送（Graph 的 body.contentType 只有 Text / Html 两种）。
    let (content_type, content) = match body_html.as_deref() {
        Some(html) => ("Html", html),
        None => ("Text", body_text),
    };
    let mut message_obj = serde_json::json!({
        "subject": subject,
        "body": {
            "contentType": content_type,
            "content": content
        },
        "toRecipients": serde_json::Value::Array(to_list)
    });

    if !graph_attachments.is_empty() {
        message_obj["attachments"] = serde_json::Value::Array(graph_attachments);
    }

    let cc_recipients = graph_recipient_array(cc_addrs);
    if !cc_recipients.is_empty() {
        message_obj["ccRecipients"] = serde_json::Value::Array(cc_recipients);
    }
    let bcc_recipients = graph_recipient_array(bcc_addrs);
    if !bcc_recipients.is_empty() {
        message_obj["bccRecipients"] = serde_json::Value::Array(bcc_recipients);
    }
    if let Some(rt) = reply_to {
        let reply_to_recipients = graph_recipient_array(rt);
        if !reply_to_recipients.is_empty() {
            message_obj["replyTo"] = serde_json::Value::Array(reply_to_recipients);
        }
    }

    let payload = serde_json::json!({
        "message": message_obj,
        "saveToSentItems": true
    });

    let url = format!("{GRAPH_BASE}/sendMail");
    match graph_agent()
        .post(&url)
        .set("Authorization", &format!("Bearer {access_token}"))
        .send_json(payload)
    {
        Ok(response) => graph_send_success(response),
        Err(ureq::Error::Status(code, _)) => {
            if code >= 500 {
                SendOutcome::Uncertain(format!("Graph 发送结果待确认 (HTTP {code})"))
            } else {
                SendOutcome::Failed(format!("Graph 发信被拒绝 (HTTP {code})"))
            }
        }
        Err(ureq::Error::Transport(t)) => {
            SendOutcome::Uncertain(format!("Graph 网络中断，发送结果待确认: {:?}", t.kind()))
        }
    }
}

fn format_graph_address(addr: &serde_json::Value) -> String {
    let name = addr["name"].as_str().unwrap_or("").trim();
    let email = addr["address"].as_str().unwrap_or("").trim();
    if !name.is_empty() && !email.is_empty() {
        format!("{name} <{email}>")
    } else if !email.is_empty() {
        email.to_string()
    } else {
        "(未知)".to_string()
    }
}

fn extract_email_address(input: &str) -> String {
    let trimmed = input.trim();
    if let (Some(start), Some(end)) = (trimmed.rfind('<'), trimmed.rfind('>')) {
        if start < end {
            return trimmed[start + 1..end].trim().to_string();
        }
    }
    trimmed.to_string()
}

/// Build a Graph `ccRecipients` / `bccRecipients` array from a
/// comma-separated address list, skipping parts without a usable address.
fn graph_recipient_array(raw: &str) -> Vec<serde_json::Value> {
    raw.split(',')
        .filter_map(|part| {
            let address = extract_email_address(part);
            if address.is_empty() || !address.contains('@') {
                None
            } else {
                Some(serde_json::json!({
                    "emailAddress": { "address": address }
                }))
            }
        })
        .collect()
}

fn urlencoding_segment(input: &str) -> String {
    url::form_urlencoded::byte_serialize(input.as_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_recipient_array_keeps_every_address_in_comma_list() {
        // To 栏逗号分隔的多个收件人必须全部保留（修复多收件人漏发）。
        let addrs = graph_recipient_array("Alice <alice@example.com>, bob@example.com ,  ");
        let emails: Vec<&str> = addrs
            .iter()
            .map(|v| v["emailAddress"]["address"].as_str().unwrap())
            .collect();
        assert_eq!(emails, vec!["alice@example.com", "bob@example.com"]);

        // 无 @ 的片段跳过；全无效时为空（send_message 据此报收件人为空）。
        assert!(graph_recipient_array("notaaddress").is_empty());
        assert!(graph_recipient_array("   ").is_empty());
    }

    #[test]
    fn custom_trash_or_role_namesakes_keep_original_name_when_well_known_exists() {
        let json = serde_json::json!({
            "value": [
                { "id": "custom_trash", "displayName": "Trash", "unreadItemCount": 0, "totalItemCount": 5 },
                { "id": "custom_deleted", "displayName": "Deleted Items", "unreadItemCount": 0, "totalItemCount": 1 },
                { "id": "custom_sent", "displayName": "Sent", "unreadItemCount": 0, "totalItemCount": 2 },
                { "id": "custom_drafts", "displayName": "Drafts", "unreadItemCount": 0, "totalItemCount": 3 },
                { "id": "custom_spam", "displayName": "Spam", "unreadItemCount": 0, "totalItemCount": 4 },
                { "id": "custom_archive", "displayName": "Archive", "unreadItemCount": 0, "totalItemCount": 6 },
                { "id": "sys_inbox", "displayName": "收件箱", "unreadItemCount": 10, "totalItemCount": 2444 },
                { "id": "sys_deleted", "displayName": "已删除邮件", "unreadItemCount": 0, "totalItemCount": 4 },
                { "id": "sys_sent", "displayName": "已发送邮件", "unreadItemCount": 0, "totalItemCount": 55 },
                { "id": "sys_drafts", "displayName": "草稿", "unreadItemCount": 0, "totalItemCount": 2 },
                { "id": "sys_junk", "displayName": "垃圾邮件", "unreadItemCount": 0, "totalItemCount": 19 },
                { "id": "sys_archive", "displayName": "存档", "unreadItemCount": 0, "totalItemCount": 0 }
            ]
        });
        let well_known = HashMap::from([
            ("sys_inbox".to_string(), GraphWellKnownRole::Inbox),
            ("sys_deleted".to_string(), GraphWellKnownRole::Trash),
            ("sys_sent".to_string(), GraphWellKnownRole::Sent),
            ("sys_drafts".to_string(), GraphWellKnownRole::Drafts),
            ("sys_junk".to_string(), GraphWellKnownRole::Junk),
            ("sys_archive".to_string(), GraphWellKnownRole::Archive),
        ]);

        let parsed = parse_graph_folders_json(&json, &well_known);
        let by_id: HashMap<&str, (&str, bool, usize)> = parsed
            .iter()
            .map(|(f, is_inbox)| (f.path.as_str(), (f.name.as_str(), *is_inbox, f.total_count)))
            .collect();

        assert_eq!(by_id["sys_inbox"], ("收件箱", true, 2444));
        assert_eq!(by_id["sys_deleted"], ("已删除", false, 4));
        assert_eq!(by_id["sys_sent"], ("已发送", false, 55));
        assert_eq!(by_id["sys_drafts"], ("草稿箱", false, 2));
        assert_eq!(by_id["sys_junk"], ("垃圾邮件", false, 19));
        assert_eq!(by_id["sys_archive"], ("归档", false, 0));

        assert_eq!(by_id["custom_trash"], ("Trash", false, 5));
        assert_eq!(by_id["custom_deleted"], ("Deleted Items", false, 1));
        assert_eq!(by_id["custom_sent"], ("Sent", false, 2));
        assert_eq!(by_id["custom_drafts"], ("Drafts", false, 3));
        assert_eq!(by_id["custom_spam"], ("Spam", false, 4));
        assert_eq!(by_id["custom_archive"], ("Archive", false, 6));
    }

    #[test]
    fn well_known_ids_map_non_english_and_non_chinese_system_folders() {
        let json = serde_json::json!({
            "value": [
                { "id": "de_inbox", "displayName": "Posteingang", "unreadItemCount": 3, "totalItemCount": 10 },
                { "id": "de_trash", "displayName": "Gelöschte Elemente", "unreadItemCount": 1, "totalItemCount": 2 }
            ]
        });
        let well_known = HashMap::from([
            ("de_inbox".to_string(), GraphWellKnownRole::Inbox),
            ("de_trash".to_string(), GraphWellKnownRole::Trash),
        ]);

        let parsed = parse_graph_folders_json(&json, &well_known);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].0.name, "收件箱");
        assert!(parsed[0].1);
        assert_eq!(parsed[1].0.name, "已删除");
        assert!(!parsed[1].1);
    }

    #[test]
    fn missing_well_known_role_falls_back_to_display_name_mapping() {
        let json = serde_json::json!({
            "value": [
                { "id": "f1", "displayName": "Inbox", "unreadItemCount": 1, "totalItemCount": 5 },
                { "id": "f2", "displayName": "已删除邮件", "unreadItemCount": 0, "totalItemCount": 2 },
                { "id": "f3", "displayName": "Archive", "unreadItemCount": 0, "totalItemCount": 3 }
            ]
        });

        let parsed = parse_graph_folders_json(&json, &HashMap::new());
        assert_eq!(parsed[0].0.name, "收件箱");
        assert!(parsed[0].1);
        assert_eq!(parsed[1].0.name, "已删除");
        assert!(!parsed[1].1);
        assert_eq!(parsed[2].0.name, "归档");
        assert!(!parsed[2].1);

        // well_known_by_id 中即使有 id 但不在返回的 value 列表中，也不抑制回退映射。
        let partial_wk =
            HashMap::from([("absent_archive".to_string(), GraphWellKnownRole::Archive)]);
        let parsed_partial = parse_graph_folders_json(&json, &partial_wk);
        assert_eq!(parsed_partial[2].0.name, "归档");
    }

    #[test]
    fn parse_graph_folders_handles_missing_fields_and_empty_value() {
        let empty = serde_json::json!({});
        assert!(parse_graph_folders_json(&empty, &HashMap::new()).is_empty());

        let edge = serde_json::json!({
            "value": [
                { "displayName": "NoIdFolder", "unreadItemCount": 1, "totalItemCount": 1 },
                { "id": "only_id", "unreadItemCount": 2, "totalItemCount": 7 }
            ]
        });
        let parsed = parse_graph_folders_json(&edge, &HashMap::new());
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].0.path, "only_id");
        assert_eq!(parsed[0].0.name, "Folder");
        assert_eq!(parsed[0].0.unread_count, 2);
        assert_eq!(parsed[0].0.total_count, 7);
        assert!(!parsed[0].1);
    }
}

#[cfg(test)]
mod graph_url_security_tests {
    use super::*;
    #[test]
    fn graph_error_does_not_copy_reply_body() {
        let response = ureq::Response::new(404, "Not Found", "synthetic private text").unwrap();
        assert_eq!(
            format_graph_error(ureq::Error::Status(404, response)),
            "Graph API 错误 (HTTP 404)"
        );
    }

    #[test]
    fn redirect_is_never_success_but_accepted_status_is() {
        assert!(require_graph_success(ureq::Response::new(302, "Found", "").unwrap()).is_err());
        assert!(require_graph_success(ureq::Response::new(202, "Accepted", "").unwrap()).is_ok());
        assert!(matches!(
            graph_send_success(ureq::Response::new(302, "Found", "").unwrap()),
            SendOutcome::Uncertain(_)
        ));
        assert!(matches!(
            graph_send_success(ureq::Response::new(202, "Accepted", "").unwrap()),
            SendOutcome::Accepted
        ));
    }

    #[test]
    fn pagination_rejects_untrusted_origins() {
        assert!(validate_graph_url("https://graph.microsoft.com/v1.0/me/messages?$skip=1").is_ok());
        for url in [
            "http://graph.microsoft.com/v1.0/me",
            "https://graph.microsoft.com.evil.test/",
            "https://graph.microsoft.com@evil.test/",
            "https://graph.microsoft.com:444/",
            "https://user@graph.microsoft.com/",
        ] {
            assert!(validate_graph_url(url).is_err(), "{url}");
        }
        assert!(fetch_folder_headers_next("secret", "https://evil.test/").is_err());
    }
}
