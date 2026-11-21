//! Microsoft Graph REST mail client (folders, messages, attachments, flags, sendMail).

use std::path::Path;

use base64::Engine;

use crate::cache::{CachedAttachmentInput, CachedHeaderInput, RemoteFolder};
use crate::imap_client::html_to_readable_text;
use crate::smtp_client::SendOutcome;

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
pub fn fetch_folder_headers(
    access_token: &str,
    folder_graph_id: &str,
) -> Result<Vec<(CachedHeaderInput, String)>, String> {
    let encoded_folder = urlencoding_segment(folder_graph_id);
    let url = format!(
        "{GRAPH_BASE}/mailFolders/{encoded_folder}/messages?\
         $top=100&$orderby=receivedDateTime%20desc&\
         $select=id,subject,from,toRecipients,receivedDateTime,isRead,flag,hasAttachments,bodyPreview,internetMessageId"
    );

    let resp = ureq::get(&url)
        .set("Authorization", &format!("Bearer {access_token}"))
        .call()
        .map_err(format_graph_error)?;

    let json: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("解析 Graph 邮件列表失败: {e}"))?;

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
                    sender,
                    recipients,
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

    Ok(items)
}

/// Fetch message body and file attachments via Microsoft Graph.
pub fn fetch_message_body(
    access_token: &str,
    remote_id: &str,
) -> Result<(String, Vec<CachedAttachmentInput>), String> {
    let encoded_id = urlencoding_segment(remote_id);
    let url = format!("{GRAPH_BASE}/messages/{encoded_id}?$select=body,hasAttachments");
    let resp = ureq::get(&url)
        .set("Authorization", &format!("Bearer {access_token}"))
        .call()
        .map_err(format_graph_error)?;

    let json: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("解析 Graph 邮件正文失败: {e}"))?;

    let content_type = json["body"]["contentType"].as_str().unwrap_or("text");
    let raw_content = json["body"]["content"].as_str().unwrap_or("");
    let body_text = if content_type.eq_ignore_ascii_case("html") {
        html_to_readable_text(raw_content)
    } else {
        raw_content.to_string()
    };

    let mut attachments = Vec::new();
    if json["hasAttachments"].as_bool().unwrap_or(false) {
        let att_url = format!("{GRAPH_BASE}/messages/{encoded_id}/attachments");
        let att_resp = ureq::get(&att_url)
            .set("Authorization", &format!("Bearer {access_token}"))
            .call()
            .map_err(format_graph_error)?;
        let att_json: serde_json::Value = att_resp
            .into_json()
            .map_err(|e| format!("解析 Graph 附件失败: {e}"))?;
        if let Some(arr) = att_json["value"].as_array() {
            for att in arr {
                if let Some(b64) = att["contentBytes"].as_str() {
                    if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64) {
                        attachments.push(CachedAttachmentInput {
                            filename: att["name"].as_str().unwrap_or("attachment.bin").to_string(),
                            mime_type: att["contentType"]
                                .as_str()
                                .unwrap_or("application/octet-stream")
                                .to_string(),
                            content: bytes,
                        });
                    }
                }
            }
        }
    }

    Ok((body_text, attachments))
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

/// Send an email (with optional attachments) via Microsoft Graph `sendMail`.
pub fn send_message(
    access_token: &str,
    to_addr: &str,
    subject: &str,
    body_text: &str,
    attachment_paths: &[String],
) -> SendOutcome {
    let address = extract_email_address(to_addr);
    if address.is_empty() {
        return SendOutcome::Failed("收件人邮箱地址不能为空".to_string());
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

    let mut message_obj = serde_json::json!({
        "subject": subject,
        "body": {
            "contentType": "Text",
            "content": body_text
        },
        "toRecipients": [
            {
                "emailAddress": {
                    "address": address
                }
            }
        ]
    });

    if !graph_attachments.is_empty() {
        message_obj["attachments"] = serde_json::Value::Array(graph_attachments);
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

fn urlencoding_segment(input: &str) -> String {
    url::form_urlencoded::byte_serialize(input.as_bytes()).collect()
}
