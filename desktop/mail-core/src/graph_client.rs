//! Microsoft Graph REST mail client (folders, messages, attachments, flags, sendMail).

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use base64::Engine;

use crate::cache::{CachedAttachmentInput, CachedHeaderInput, RemoteFolder};
use crate::imap_client::html_to_readable_text;
use crate::smtp_client::SendOutcome;
use crate::{FetchedMessageBody, AUTO_CACHE_ATTACHMENT_MAX_BYTES};

const MAX_INLINE_CID_BYTES: usize = 5 * 1024 * 1024;

const GRAPH_BASE: &str = "https://graph.microsoft.com/v1.0/me";

fn format_graph_error(err: ureq::Error) -> String {
    match err {
        ureq::Error::Status(code, resp) => {
            let body = resp.into_string().unwrap_or_default();
            format!("Graph API 错误 (HTTP {code}): {body}")
        }
        ureq::Error::Transport(t) => format!("Graph API 网络错误: {t}"),
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

/// Fetch mail folders via Microsoft Graph.
pub fn fetch_folders(access_token: &str) -> Result<Vec<(RemoteFolder, bool)>, String> {
    let url = format!(
        "{GRAPH_BASE}/mailFolders?$top=50&$select=id,displayName,unreadItemCount,totalItemCount"
    );
    let resp = ureq::get(&url)
        .set("Authorization", &format!("Bearer {access_token}"))
        .call()
        .map_err(format_graph_error)?;

    let json: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("解析 Graph 文件夹响应失败: {e}"))?;

    let mut out = Vec::new();
    if let Some(arr) = json["value"].as_array() {
        for item in arr {
            let Some(id) = item["id"].as_str() else {
                continue;
            };
            let raw_name = item["displayName"].as_str().unwrap_or("Folder");
            let (friendly, is_inbox) = map_graph_folder_name(raw_name);
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
    }
    Ok(out)
}

fn map_graph_folder_name(raw: &str) -> (String, bool) {
    match raw.trim().to_ascii_lowercase().as_str() {
        "inbox" | "收件箱" => ("收件箱".to_string(), true),
        "sent items" | "sent" | "已发送邮件" | "已发送" => ("已发送".to_string(), false),
        "drafts" | "草稿" => ("草稿箱".to_string(), false),
        "deleted items" | "trash" | "已删除邮件" => ("已删除".to_string(), false),
        "junk email" | "spam" | "垃圾邮件" => ("垃圾邮件".to_string(), false),
        "archive" | "存档" => ("归档".to_string(), false),
        _ => (raw.to_string(), false),
    }
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
    let resp = ureq::get(&url)
        .timeout(std::time::Duration::from_secs(15))
        .set("Authorization", &format!("Bearer {access_token}"))
        .set("ConsistencyLevel", "eventual")
        .call()
        .map_err(format_graph_error)?;

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
    let resp = ureq::get(url)
        .timeout(std::time::Duration::from_secs(15))
        .set("Authorization", &format!("Bearer {access_token}"))
        .call()
        .map_err(format_graph_error)?;

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
    let resp = ureq::get(&url)
        .set("Authorization", &format!("Bearer {access_token}"))
        .call()
        .map_err(format_graph_error)?;

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
        let att_resp = ureq::get(&att_url)
            .set("Authorization", &format!("Bearer {access_token}"))
            .call()
            .map_err(format_graph_error)?;
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
                        if let Ok(item_resp) = ureq::get(&item_url)
                            .set("Authorization", &format!("Bearer {access_token}"))
                            .call()
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
    let resp = ureq::get(&url)
        .set("Authorization", &format!("Bearer {access_token}"))
        .timeout(std::time::Duration::from_secs(180))
        .call()
        .map_err(format_graph_error)?;
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
    let response = ureq::get(&url)
        .set("Authorization", &format!("Bearer {access_token}"))
        .timeout(std::time::Duration::from_secs(20))
        .call()
        .map_err(format_graph_error)?;
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
    let resp = ureq::get(&url)
        .set("Authorization", &format!("Bearer {access_token}"))
        .call()
        .map_err(format_graph_error)?;
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
    let resp = ureq::get(&url)
        .set("Authorization", &format!("Bearer {access_token}"))
        .call()
        .map_err(format_graph_error)?;
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
    ureq::patch(&url)
        .set("Authorization", &format!("Bearer {access_token}"))
        .send_json(serde_json::json!({ "isRead": seen }))
        .map_err(format_graph_error)?;
    Ok(())
}

/// Update flagged (`flag.flagStatus`) state on Microsoft Graph.
pub fn set_flagged(access_token: &str, remote_id: &str, flagged: bool) -> Result<(), String> {
    let encoded_id = urlencoding_segment(remote_id);
    let url = format!("{GRAPH_BASE}/messages/{encoded_id}");
    let status = if flagged { "flagged" } else { "notFlagged" };
    ureq::patch(&url)
        .set("Authorization", &format!("Bearer {access_token}"))
        .send_json(serde_json::json!({ "flag": { "flagStatus": status } }))
        .map_err(format_graph_error)?;
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
    ureq::post(&url)
        .set("Authorization", &format!("Bearer {access_token}"))
        .send_json(serde_json::json!({ "destinationId": dest_folder_id }))
        .map_err(format_graph_error)?;
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

    // 有 HTML 正文时按 HTML 发送（Graph 的 body.contentType 只有 Text / Html 两种）。
    let (content_type, content) = match body_html {
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
    match ureq::post(&url)
        .set("Authorization", &format!("Bearer {access_token}"))
        .send_json(payload)
    {
        Ok(_) => SendOutcome::Accepted,
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            if code >= 500 {
                SendOutcome::Uncertain(format!("Graph 发送结果待确认 (HTTP {code}): {body}"))
            } else {
                SendOutcome::Failed(format!("Graph 发信被拒绝 (HTTP {code}): {body}"))
            }
        }
        Err(ureq::Error::Transport(t)) => {
            SendOutcome::Uncertain(format!("Graph 网络中断，发送结果待确认: {t}"))
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
}
