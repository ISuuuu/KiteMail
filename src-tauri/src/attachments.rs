//! Wave 3 Group A IPC commands: attachment gallery, preview, safe saving,
//! sanitized print HTML generation, and standalone reader window loading.

use std::path::{Component, Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use kitemail_mail_core::cache::{unique_attachment_path, AttachmentRecord, CacheStore};
use kitemail_mail_core::config;
use kitemail_mail_core::imap_client::{plain_text_to_html, sanitize_html_for_display_with_options};
use kitemail_mail_core::{MailRequest, MessageDetail};
use kitemail_windows_mail_reader::escape_html;
use tauri::{AppHandle, Manager, State};

use crate::commands::AppState;
use crate::dto::{
    AppMode, BodyStatusDto, CachedAttachmentDto, FetchAttachmentPreviewInput,
    FetchAttachmentPreviewOutput, GetMessagePrintHtmlInput, GetMessagePrintHtmlOutput, IpcError,
    ListCachedAttachmentsInput, ListCachedAttachmentsOutput, OpenAttachmentInput,
    OpenAttachmentOutput, OpenMessageWindowInput, OpenMessageWindowOutput, PickDirectoryOutput,
    SaveAllAttachmentsInput, SaveAllAttachmentsOutput, SaveAttachmentInput, SaveAttachmentOutput,
    MAIN_WINDOW_LABEL,
};
use crate::mail_session::{
    build_open_message_output, is_message_remote_images_blocked, MailSession, MAX_PAGE_LIMIT,
    MAX_QUERY_LEN,
};

pub const MAX_ATTACHMENT_PREVIEW_BYTES: usize = 10 * 1024 * 1024;
const ATTACHMENT_WORKER_TIMEOUT: Duration = Duration::from_secs(20);

const DEMO_SAFE_SVG: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"640\" height=\"360\" viewBox=\"0 0 640 360\"><rect width=\"640\" height=\"360\" rx=\"16\" fill=\"#f4f6fb\"/><rect x=\"24\" y=\"24\" width=\"156\" height=\"312\" rx=\"10\" fill=\"#e8edf7\"/><rect x=\"196\" y=\"24\" width=\"192\" height=\"312\" rx=\"10\" fill=\"#ffffff\" stroke=\"#d6dfef\"/><rect x=\"404\" y=\"24\" width=\"212\" height=\"312\" rx=\"10\" fill=\"#ffffff\" stroke=\"#d6dfef\"/><text x=\"320\" y=\"186\" text-anchor=\"middle\" font-family=\"Segoe UI, Microsoft YaHei, sans-serif\" font-size=\"18\" fill=\"#2a4b8d\">KiteMail Demo Preview</text></svg>";
const DEMO_SAMPLE_TEXT: &str = "KiteMail Windows Baseline Notes\n\n1. MSVC x86_64-pc-windows-msvc workspace build verified.\n2. Three-pane layout and WebView2 reader isolation enabled.\n";

pub fn encode_base64(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((triple >> 18) & 0x3F) as usize] as char);
        out.push(TABLE[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(TABLE[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(TABLE[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn file_ext_lower(filename: &str) -> String {
    Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default()
}

pub fn is_image_attachment(mime_type: &str, filename: &str) -> bool {
    let lower_mime = mime_type.trim().to_ascii_lowercase();
    if lower_mime.starts_with("image/") {
        return true;
    }
    if lower_mime.is_empty() || lower_mime == "application/octet-stream" {
        return matches!(
            file_ext_lower(filename).as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "bmp" | "ico"
        );
    }
    false
}

fn is_text_attachment(mime_type: &str, filename: &str) -> bool {
    let lower_mime = mime_type.trim().to_ascii_lowercase();
    if lower_mime.starts_with("text/")
        || lower_mime == "application/json"
        || lower_mime == "application/xml"
    {
        return true;
    }
    if lower_mime.is_empty() || lower_mime == "application/octet-stream" {
        return matches!(
            file_ext_lower(filename).as_str(),
            "txt" | "md" | "csv" | "log" | "json" | "xml" | "toml" | "yaml" | "yml"
        );
    }
    false
}

pub fn is_pdf_attachment(mime_type: &str, filename: &str) -> bool {
    let lower_mime = mime_type.trim().to_ascii_lowercase();
    lower_mime == "application/pdf"
        || ((lower_mime.is_empty() || lower_mime == "application/octet-stream")
            && file_ext_lower(filename) == "pdf")
}

fn sanitize_mime_for_data_url(mime_type: &str, filename: &str) -> String {
    let base = mime_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let valid_token = !base.is_empty()
        && base.contains('/')
        && base
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'+' | b'.'));
    if valid_token {
        return base;
    }
    match file_ext_lower(filename).as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "txt" | "log" | "md" => "text/plain",
        "csv" => "text/csv",
        "json" => "application/json",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
    .to_string()
}

fn is_windows_reserved_filename(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name).to_ascii_lowercase();
    matches!(
        stem.as_str(),
        "con"
            | "prn"
            | "aux"
            | "nul"
            | "com1"
            | "com2"
            | "com3"
            | "com4"
            | "com5"
            | "com6"
            | "com7"
            | "com8"
            | "com9"
            | "lpt1"
            | "lpt2"
            | "lpt3"
            | "lpt4"
            | "lpt5"
            | "lpt6"
            | "lpt7"
            | "lpt8"
            | "lpt9"
    )
}

fn validate_path_common(raw: &str, label: &str) -> Result<PathBuf, IpcError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(IpcError::invalid_input(format!("{label} 不能为空"), None));
    }
    if trimmed.chars().any(|c| c == '\0' || (c as u32) < 0x20) {
        return Err(IpcError::invalid_input(
            format!("{label} 包含非法控制字符或空字符"),
            None,
        ));
    }
    if trimmed.starts_with(r"\\?\") || trimmed.starts_with(r"\\.\") {
        return Err(IpcError::invalid_input(
            format!("{label} 不允许使用设备命名空间路径"),
            None,
        ));
    }
    let path = PathBuf::from(trimmed);
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(IpcError::invalid_input(
            format!("{label} 不允许包含路径穿越片段 (..)"),
            None,
        ));
    }
    Ok(path)
}

pub fn validate_custom_dest_path(raw: &str) -> Result<PathBuf, IpcError> {
    let path = validate_path_common(raw, "destPath")?;
    let trimmed = raw.trim();
    if trimmed.ends_with('/') || trimmed.ends_with('\\') || path.is_dir() {
        return Err(IpcError::invalid_input(
            "destPath 必须包含有效文件名，不能指向目录",
            None,
        ));
    }
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return Err(IpcError::invalid_input("destPath 缺少文件名", None));
    };
    if is_windows_reserved_filename(name) {
        return Err(IpcError::invalid_input("destPath 包含系统保留文件名", None));
    }
    Ok(path)
}

pub fn validate_custom_dest_dir(raw: &str) -> Result<PathBuf, IpcError> {
    let path = validate_path_common(raw, "destDir")?;
    if path.is_file() {
        return Err(IpcError::invalid_input(
            "destDir 必须指向目录而非已有文件",
            None,
        ));
    }
    Ok(path)
}

fn default_downloads_dir(data_dir: &Path) -> PathBuf {
    #[cfg(test)]
    {
        data_dir.join("downloads")
    }
    #[cfg(not(test))]
    {
        dirs::download_dir().unwrap_or_else(|| data_dir.join("downloads"))
    }
}

fn write_bytes_safely(dest_path: &Path, bytes: &[u8]) -> Result<(), IpcError> {
    if let Some(parent) = dest_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|e| IpcError::internal(format!("创建保存目录失败: {e}"), None))?;
    }
    std::fs::write(dest_path, bytes)
        .map_err(|e| IpcError::internal(format!("写入附件文件失败: {e}"), None))
}

fn is_dangerous_print_tag(tag_name: &str) -> bool {
    matches!(
        tag_name,
        "script"
            | "iframe"
            | "object"
            | "embed"
            | "applet"
            | "base"
            | "meta"
            | "link"
            | "form"
            | "noscript"
            | "template"
            | "foreignobject"
    )
}

fn strip_unsafe_attrs_from_tag(tag_inner: &str) -> String {
    let mut parts = tag_inner.splitn(2, char::is_whitespace);
    let tag_name = parts.next().unwrap_or("");
    let Some(rest) = parts.next() else {
        return tag_inner.to_string();
    };
    let trailing_slash = rest.trim_end().ends_with('/');
    let attr_src = if trailing_slash {
        rest.trim_end().strip_suffix('/').unwrap_or(rest).trim()
    } else {
        rest.trim()
    };

    let bytes = attr_src.as_bytes();
    let mut i = 0usize;
    let mut kept: Vec<String> = Vec::new();
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let name_start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'=' {
            i += 1;
        }
        let attr_name = &attr_src[name_start..i];
        let attr_lower = attr_name.to_ascii_lowercase();

        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut attr_val: Option<&str> = None;
        let mut full_end = i;
        if i < bytes.len() && bytes[i] == b'=' {
            i += 1;
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
                let quote = bytes[i];
                i += 1;
                let val_start = i;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
                attr_val = Some(&attr_src[val_start..i]);
                if i < bytes.len() {
                    i += 1;
                }
                full_end = i;
            } else {
                let val_start = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                attr_val = Some(&attr_src[val_start..i]);
                full_end = i;
            }
        }

        if attr_lower.starts_with("on") || attr_lower == "srcdoc" {
            continue;
        }
        if matches!(
            attr_lower.as_str(),
            "href" | "src" | "action" | "formaction" | "xlink:href" | "poster"
        ) {
            if let Some(val) = attr_val {
                let compact: String = val
                    .chars()
                    .filter(|c| !c.is_whitespace() && !c.is_control())
                    .collect::<String>()
                    .to_ascii_lowercase();
                if compact.starts_with("javascript:")
                    || compact.starts_with("vbscript:")
                    || compact.starts_with("data:text/html")
                {
                    continue;
                }
            }
        }
        kept.push(attr_src[name_start..full_end].to_string());
    }

    if kept.is_empty() {
        if trailing_slash {
            format!("{tag_name} /")
        } else {
            tag_name.to_string()
        }
    } else if trailing_slash {
        format!("{tag_name} {} /", kept.join(" "))
    } else {
        format!("{tag_name} {}", kept.join(" "))
    }
}

/// Sanitize HTML body for printing: removes scripts, embedded active content,
/// `on*` event attributes, and `javascript:`/`vbscript:`/`data:text/html` URIs.
pub fn sanitize_print_html(raw_html: &str, block_remote_images: bool, clean_links: bool) -> String {
    let (pre_cleaned, _) =
        sanitize_html_for_display_with_options(raw_html, block_remote_images, clean_links);
    let mut out = String::with_capacity(pre_cleaned.len());
    let mut in_tag = false;
    let mut tag_buf = String::new();
    let mut skip_block: Option<String> = None;

    for ch in pre_cleaned.chars() {
        if ch == '<' {
            in_tag = true;
            tag_buf.clear();
            continue;
        }
        if ch == '>' && in_tag {
            in_tag = false;
            let trimmed = tag_buf.trim();
            let lower = trimmed.to_ascii_lowercase();
            let is_closing = lower.starts_with('/');
            let tag_name = lower
                .trim_start_matches('/')
                .split(|c: char| c.is_whitespace() || c == '/')
                .next()
                .unwrap_or("");
            if lower.starts_with("!--") || lower.starts_with("!doctype") {
                continue;
            }
            if let Some(ref active) = skip_block {
                if is_closing && tag_name == active {
                    skip_block = None;
                }
                continue;
            }
            if is_dangerous_print_tag(tag_name) {
                if !is_closing
                    && !trimmed.ends_with('/')
                    && !matches!(tag_name, "embed" | "base" | "meta" | "link")
                {
                    skip_block = Some(tag_name.to_string());
                }
                continue;
            }
            out.push('<');
            if is_closing {
                out.push('/');
                out.push_str(tag_name);
            } else {
                out.push_str(&strip_unsafe_attrs_from_tag(trimmed));
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
    out
}

fn render_message_print_document(
    detail: &MessageDetail,
    include_header: bool,
    block_remote_images: bool,
    clean_links: bool,
) -> String {
    let safe_subject = escape_html(&detail.subject);
    let header_html = if include_header {
        let safe_sender = escape_html(&detail.sender);
        let safe_recipients = escape_html(&detail.recipients);
        let safe_date = escape_html(&detail.date_display);
        let cc_row = if detail.cc.trim().is_empty() {
            String::new()
        } else {
            format!(
                "<div><strong>抄送：</strong>{}</div>",
                escape_html(&detail.cc)
            )
        };
        format!(
            "<header class=\"print-header\">\
                <h1>{safe_subject}</h1>\
                <div class=\"meta\">\
                    <div><strong>发件人：</strong>{safe_sender}</div>\
                    <div><strong>收件人：</strong>{safe_recipients}</div>\
                    {cc_row}\
                    <div><strong>日期：</strong>{safe_date}</div>\
                </div>\
            </header>"
        )
    } else {
        String::new()
    };

    let body_html = if let Some(ref html) = detail.body_html {
        sanitize_print_html(html, block_remote_images, clean_links)
    } else {
        plain_text_to_html(&detail.body_text)
    };

    format!(
        "<!doctype html>\
<html lang=\"zh-CN\">\
<head>\
<meta charset=\"utf-8\">\
<title>{safe_subject}</title>\
<style>\
@page {{ margin: 16mm; }}\
html, body {{ margin: 0; padding: 0; background: #ffffff; color: #1a1d21; font: 14px/1.6 'Segoe UI', 'Microsoft YaHei', sans-serif; }}\
.print-container {{ max-width: 820px; margin: 0 auto; padding: 24px; }}\
.print-header {{ border-bottom: 1px solid #d9dee7; padding-bottom: 12px; margin-bottom: 18px; page-break-inside: avoid; }}\
.print-header h1 {{ font-size: 20px; margin: 0 0 10px; line-height: 1.35; }}\
.print-header .meta {{ font-size: 13px; color: #4a5260; line-height: 1.55; }}\
.print-body {{ word-break: break-word; overflow-wrap: anywhere; }}\
.print-body img {{ max-width: 100%; height: auto; }}\
.print-body pre, .print-body blockquote {{ page-break-inside: avoid; }}\
@media print {{ .print-container {{ padding: 0; max-width: none; }} body {{ background: #ffffff; color: #000000; }} }}\
</style>\
</head>\
<body>\
<main class=\"print-container\">{header_html}<article class=\"print-body\">{body_html}</article></main>\
</body>\
</html>"
    )
}

fn demo_attachment_payload(attachment_id: u32, filename: &str, mime_type: &str) -> Vec<u8> {
    if is_image_attachment(mime_type, filename) {
        DEMO_SAFE_SVG.as_bytes().to_vec()
    } else {
        format!("{DEMO_SAMPLE_TEXT}\nAttachment ID: {attachment_id}\nFilename: {filename}\n")
            .into_bytes()
    }
}

impl MailSession {
    pub fn list_cached_attachments(
        &self,
        input: &ListCachedAttachmentsInput,
    ) -> Result<ListCachedAttachmentsOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        let query = input.query.as_deref().unwrap_or("").trim();
        if query.chars().count() > MAX_QUERY_LEN {
            return Err(IpcError::invalid_input(
                format!("搜索关键词长度不能超过 {MAX_QUERY_LEN} 个字符"),
                None,
            ));
        }
        let offset = input.offset.unwrap_or(0);
        let raw_limit = input.limit.unwrap_or(50);
        if raw_limit == 0 {
            return Err(IpcError::invalid_input("limit 必须大于 0", None));
        }
        let limit = raw_limit.min(MAX_PAGE_LIMIT);

        let all_items = match self.mode {
            AppMode::Demo => {
                let demo = self.demo.lock().expect("demo mutex poisoned");
                demo.store.list_cached_attachments(query, 0, 10_000)
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
                            IpcError::unavailable(format!("本地邮件缓存暂不可用: {e}"), None)
                        })?;
                    cache
                        .list_cached_attachments(query, 0, 100_000)
                        .map_err(|e| IpcError::internal(format!("读取附件列表失败: {e}"), None))?
                }
            }
        };

        let total = all_items.len();
        let start = offset.min(total);
        let end = start.saturating_add(limit).min(total);
        let items = all_items[start..end]
            .iter()
            .map(|att| CachedAttachmentDto {
                id: att.id,
                message_id: att.message_id,
                account_id: att.account_id,
                filename: att.filename.clone(),
                mime_type: att.mime_type.clone(),
                size_bytes: att.size_bytes,
                sender: att.sender.clone(),
                subject: att.subject.clone(),
                date_ts: att.date_ts,
                date_display: att.date_display.clone(),
                downloaded: att.downloaded,
                is_image: is_image_attachment(&att.mime_type, &att.filename),
            })
            .collect();

        Ok(ListCachedAttachmentsOutput {
            items,
            total,
            has_more: end < total,
            state_version: self.state_version(),
        })
    }

    pub fn fetch_attachment_preview(
        &self,
        input: &FetchAttachmentPreviewInput,
    ) -> Result<FetchAttachmentPreviewOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        if input.attachment_id == 0 {
            return Err(IpcError::invalid_input("attachmentId 必须为正整数", None));
        }

        match self.mode {
            AppMode::Demo => {
                let att = {
                    let demo = self.demo.lock().expect("demo mutex poisoned");
                    demo.store
                        .list_cached_attachments("", 0, 10_000)
                        .into_iter()
                        .find(|a| a.id == input.attachment_id)
                }
                .ok_or_else(|| {
                    IpcError::invalid_input(format!("未找到附件 #{}", input.attachment_id), None)
                })?;

                if att.size_bytes > MAX_ATTACHMENT_PREVIEW_BYTES {
                    return Err(IpcError::invalid_input("附件体积超过 10 MB 预览上限", None));
                }
                let is_img = is_image_attachment(&att.mime_type, &att.filename);
                let (preview_mime, bytes) = if is_img {
                    (
                        "image/svg+xml".to_string(),
                        DEMO_SAFE_SVG.as_bytes().to_vec(),
                    )
                } else {
                    (
                        sanitize_mime_for_data_url(&att.mime_type, &att.filename),
                        demo_attachment_payload(att.id, &att.filename, &att.mime_type),
                    )
                };
                let b64 = encode_base64(&bytes);
                let data_url = format!("data:{preview_mime};base64,{b64}");
                Ok(FetchAttachmentPreviewOutput {
                    attachment_id: att.id,
                    filename: att.filename,
                    mime_type: preview_mime,
                    data_url,
                    size_bytes: bytes.len(),
                })
            }
            AppMode::Live => {
                let (data_dir, worker) = {
                    let live = self.live.lock().expect("live mutex poisoned");
                    (live.data_dir.clone(), live.worker.clone())
                };
                let cache = CacheStore::open_for_data_dir_read_only(&data_dir).map_err(|e| {
                    IpcError::unavailable(format!("本地邮件缓存暂不可用: {e}"), None)
                })?;
                let record = cache
                    .get_attachment_record(input.attachment_id)
                    .map_err(|e| IpcError::internal(format!("读取附件记录失败: {e}"), None))?
                    .ok_or_else(|| {
                        IpcError::invalid_input(
                            format!("未找到附件 #{}", input.attachment_id),
                            None,
                        )
                    })?;

                let (filename, mime_type, bytes) = match record {
                    AttachmentRecord::Local {
                        filename,
                        mime_type,
                        bytes,
                        ..
                    } => {
                        if bytes.len() > MAX_ATTACHMENT_PREVIEW_BYTES {
                            return Err(IpcError::invalid_input(
                                "附件体积超过 10 MB 预览上限",
                                None,
                            ));
                        }
                        (filename, mime_type, bytes)
                    }
                    AttachmentRecord::PendingRemote { size_bytes, .. } => {
                        if size_bytes > MAX_ATTACHMENT_PREVIEW_BYTES {
                            return Err(IpcError::invalid_input(
                                "附件体积超过 10 MB 预览上限",
                                None,
                            ));
                        }
                        let Some(w) = worker else {
                            return Err(IpcError::unavailable("邮件后台服务尚未就绪", None));
                        };
                        if !w.is_async_worker() {
                            w.send(MailRequest::FetchAttachmentBytes {
                                attachment_id: input.attachment_id,
                            });
                            return Err(IpcError::unavailable(
                                "附件尚未下载到本地缓存，请稍后重试",
                                None,
                            ));
                        }
                        let (tx, rx) = mpsc::channel();
                        {
                            let mut live = self.live.lock().expect("live mutex poisoned");
                            live.waiters
                                .attachment_bytes
                                .insert(input.attachment_id, tx);
                        }
                        w.send(MailRequest::FetchAttachmentBytes {
                            attachment_id: input.attachment_id,
                        });
                        let res = rx.recv_timeout(ATTACHMENT_WORKER_TIMEOUT).map_err(|_| {
                            let mut live = self.live.lock().expect("live mutex poisoned");
                            live.waiters.attachment_bytes.remove(&input.attachment_id);
                            IpcError::unavailable("下载预览附件超时，请稍后重试", None)
                        })?;
                        res.map_err(|msg| IpcError::unavailable(msg, None))?
                    }
                };

                if bytes.len() > MAX_ATTACHMENT_PREVIEW_BYTES {
                    return Err(IpcError::invalid_input("附件体积超过 10 MB 预览上限", None));
                }
                if !is_image_attachment(&mime_type, &filename)
                    && !is_text_attachment(&mime_type, &filename)
                    && !is_pdf_attachment(&mime_type, &filename)
                {
                    return Err(IpcError::invalid_input(
                        "仅支持预览图片、PDF 或文本类附件",
                        None,
                    ));
                }
                let safe_mime = sanitize_mime_for_data_url(&mime_type, &filename);
                let data_url = format!("data:{safe_mime};base64,{}", encode_base64(&bytes));
                let size_bytes = bytes.len();
                Ok(FetchAttachmentPreviewOutput {
                    attachment_id: input.attachment_id,
                    filename,
                    mime_type: safe_mime,
                    data_url,
                    size_bytes,
                })
            }
        }
    }

    pub fn save_attachment(
        &self,
        input: &SaveAttachmentInput,
    ) -> Result<SaveAttachmentOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        if input.attachment_id == 0 {
            return Err(IpcError::invalid_input("attachmentId 必须为正整数", None));
        }
        let explicit_dest = input
            .dest_path
            .as_deref()
            .map(validate_custom_dest_path)
            .transpose()?;

        match self.mode {
            AppMode::Demo => {
                let att = {
                    let demo = self.demo.lock().expect("demo mutex poisoned");
                    demo.store
                        .list_cached_attachments("", 0, 10_000)
                        .into_iter()
                        .find(|a| a.id == input.attachment_id)
                }
                .ok_or_else(|| {
                    IpcError::invalid_input(format!("未找到附件 #{}", input.attachment_id), None)
                })?;

                let target_path = match explicit_dest {
                    Some(p) => p,
                    None => unique_attachment_path(
                        &default_downloads_dir(self.prefs.data_dir()),
                        &att.filename,
                    ),
                };
                let bytes = demo_attachment_payload(att.id, &att.filename, &att.mime_type);
                write_bytes_safely(&target_path, &bytes)?;
                let saved_path = target_path.to_string_lossy().into_owned();
                Ok(SaveAttachmentOutput {
                    saved: true,
                    saved_path: saved_path.clone(),
                    message: format!("附件已保存至 {saved_path}"),
                })
            }
            AppMode::Live => {
                let (data_dir, worker) = {
                    let live = self.live.lock().expect("live mutex poisoned");
                    (live.data_dir.clone(), live.worker.clone())
                };
                let cache = CacheStore::open_for_data_dir_read_only(&data_dir).map_err(|e| {
                    IpcError::unavailable(format!("本地邮件缓存暂不可用: {e}"), None)
                })?;
                let record = cache
                    .get_attachment_record(input.attachment_id)
                    .map_err(|e| IpcError::internal(format!("读取附件记录失败: {e}"), None))?
                    .ok_or_else(|| {
                        IpcError::invalid_input(
                            format!("未找到附件 #{}", input.attachment_id),
                            None,
                        )
                    })?;

                match record {
                    AttachmentRecord::Local {
                        filename, bytes, ..
                    } => {
                        let target_path = match explicit_dest {
                            Some(p) => p,
                            None => {
                                unique_attachment_path(&default_downloads_dir(&data_dir), &filename)
                            }
                        };
                        write_bytes_safely(&target_path, &bytes)?;
                        let saved_path = target_path.to_string_lossy().into_owned();
                        Ok(SaveAttachmentOutput {
                            saved: true,
                            saved_path: saved_path.clone(),
                            message: format!("附件已保存至 {saved_path}"),
                        })
                    }
                    AttachmentRecord::PendingRemote { filename, .. } => {
                        let target_path = match explicit_dest {
                            Some(p) => p,
                            None => {
                                unique_attachment_path(&default_downloads_dir(&data_dir), &filename)
                            }
                        };
                        if let Some(parent) =
                            target_path.parent().filter(|p| !p.as_os_str().is_empty())
                        {
                            std::fs::create_dir_all(parent).map_err(|e| {
                                IpcError::internal(format!("创建保存目录失败: {e}"), None)
                            })?;
                        }
                        let Some(w) = worker else {
                            return Err(IpcError::unavailable("邮件后台服务尚未就绪", None));
                        };
                        if !w.is_async_worker() {
                            w.send(MailRequest::SaveAttachment {
                                attachment_id: input.attachment_id,
                                dest_path: target_path.clone(),
                            });
                            let saved_path = target_path.to_string_lossy().into_owned();
                            return Ok(SaveAttachmentOutput {
                                saved: false,
                                saved_path,
                                message: "已提交后台下载并保存附件请求".to_string(),
                            });
                        }
                        let (tx, rx) = mpsc::channel();
                        {
                            let mut live = self.live.lock().expect("live mutex poisoned");
                            live.waiters.attachment_saved.push_back(tx);
                        }
                        w.send(MailRequest::SaveAttachment {
                            attachment_id: input.attachment_id,
                            dest_path: target_path,
                        });
                        let res = rx.recv_timeout(ATTACHMENT_WORKER_TIMEOUT).map_err(|_| {
                            IpcError::unavailable("下载并保存附件超时，请稍后重试", None)
                        })?;
                        let path = res.map_err(|msg| IpcError::internal(msg, None))?;
                        let saved_path = path.to_string_lossy().into_owned();
                        Ok(SaveAttachmentOutput {
                            saved: true,
                            saved_path: saved_path.clone(),
                            message: format!("附件已保存至 {saved_path}"),
                        })
                    }
                }
            }
        }
    }

    pub fn save_all_attachments(
        &self,
        input: &SaveAllAttachmentsInput,
    ) -> Result<SaveAllAttachmentsOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        if input.attachment_ids.is_empty() {
            return Err(IpcError::invalid_input("attachmentIds 不能为空", None));
        }
        if input.attachment_ids.iter().any(|&id| id == 0) {
            return Err(IpcError::invalid_input(
                "attachmentIds 中的 ID 必须为正整数",
                None,
            ));
        }
        let explicit_dir = input
            .dest_dir
            .as_deref()
            .map(validate_custom_dest_dir)
            .transpose()?;

        let data_dir = match self.mode {
            AppMode::Demo => self.prefs.data_dir().to_path_buf(),
            AppMode::Live => self
                .live
                .lock()
                .expect("live mutex poisoned")
                .data_dir
                .clone(),
        };
        let target_dir = explicit_dir.unwrap_or_else(|| default_downloads_dir(&data_dir));
        std::fs::create_dir_all(&target_dir)
            .map_err(|e| IpcError::internal(format!("创建保存目录失败: {e}"), None))?;

        let mut saved_paths = Vec::new();
        match self.mode {
            AppMode::Demo => {
                let all_atts = {
                    let demo = self.demo.lock().expect("demo mutex poisoned");
                    demo.store.list_cached_attachments("", 0, 10_000)
                };
                for &att_id in &input.attachment_ids {
                    let att = all_atts.iter().find(|a| a.id == att_id).ok_or_else(|| {
                        IpcError::invalid_input(format!("未找到附件 #{att_id}"), None)
                    })?;
                    let dest = unique_attachment_path(&target_dir, &att.filename);
                    let bytes = demo_attachment_payload(att.id, &att.filename, &att.mime_type);
                    write_bytes_safely(&dest, &bytes)?;
                    saved_paths.push(dest.to_string_lossy().into_owned());
                }
            }
            AppMode::Live => {
                let worker = self
                    .live
                    .lock()
                    .expect("live mutex poisoned")
                    .worker
                    .clone();
                let cache = CacheStore::open_for_data_dir_read_only(&data_dir).map_err(|e| {
                    IpcError::unavailable(format!("本地邮件缓存暂不可用: {e}"), None)
                })?;

                for &att_id in &input.attachment_ids {
                    let rec = cache
                        .get_attachment_record(att_id)
                        .map_err(|e| IpcError::internal(format!("读取附件记录失败: {e}"), None))?
                        .ok_or_else(|| {
                            IpcError::invalid_input(format!("未找到附件 #{att_id}"), None)
                        })?;
                    match rec {
                        AttachmentRecord::Local {
                            filename, bytes, ..
                        } => {
                            let dest = unique_attachment_path(&target_dir, &filename);
                            write_bytes_safely(&dest, &bytes)?;
                            saved_paths.push(dest.to_string_lossy().into_owned());
                        }
                        AttachmentRecord::PendingRemote { filename, .. } => {
                            let Some(ref w) = worker else {
                                return Err(IpcError::unavailable("邮件后台服务尚未就绪", None));
                            };
                            if !w.is_async_worker() {
                                w.send(MailRequest::SaveAllAttachments {
                                    attachment_ids: input.attachment_ids.clone(),
                                    dest_dir: target_dir.clone(),
                                });
                                let dest_dir_str = target_dir.to_string_lossy().into_owned();
                                return Ok(SaveAllAttachmentsOutput {
                                    saved_count: saved_paths.len(),
                                    dest_dir: dest_dir_str,
                                    saved_paths,
                                    message: "已提交批量保存远端附件请求".to_string(),
                                });
                            }
                            let dest = unique_attachment_path(&target_dir, &filename);
                            let (tx, rx) = mpsc::channel();
                            {
                                let mut live = self.live.lock().expect("live mutex poisoned");
                                live.waiters.attachment_saved.push_back(tx);
                            }
                            w.send(MailRequest::SaveAttachment {
                                attachment_id: att_id,
                                dest_path: dest,
                            });
                            let res = rx.recv_timeout(ATTACHMENT_WORKER_TIMEOUT).map_err(|_| {
                                IpcError::unavailable("下载并保存附件超时，请稍后重试", None)
                            })?;
                            let path = res.map_err(|msg| IpcError::internal(msg, None))?;
                            saved_paths.push(path.to_string_lossy().into_owned());
                        }
                    }
                }
            }
        }

        let dest_dir_str = target_dir.to_string_lossy().into_owned();
        let saved_count = saved_paths.len();
        Ok(SaveAllAttachmentsOutput {
            saved_count,
            dest_dir: dest_dir_str.clone(),
            saved_paths,
            message: format!("已保存 {saved_count} 个附件到 {dest_dir_str}"),
        })
    }

    pub fn get_message_print_html(
        &self,
        input: &GetMessagePrintHtmlInput,
    ) -> Result<GetMessagePrintHtmlOutput, IpcError> {
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
        let raw_prefs = self.prefs.snapshot_raw();
        let allowed_msgs = self
            .allowed_remote_messages
            .lock()
            .expect("allowed_remote_messages mutex poisoned")
            .clone();
        let block_remote = is_message_remote_images_blocked(&detail, &raw_prefs, &allowed_msgs);
        let include_header = input.include_header.unwrap_or(true);
        let html_document = render_message_print_document(
            &detail,
            include_header,
            block_remote,
            raw_prefs.clean_links,
        );

        Ok(GetMessagePrintHtmlOutput {
            subject: detail.subject,
            html_document,
            plain_text: detail.body_text,
        })
    }

    pub fn open_message_window(
        &self,
        input: &OpenMessageWindowInput,
    ) -> Result<OpenMessageWindowOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        if input.account_id == 0 || input.message_id == 0 {
            return Err(IpcError::invalid_input(
                "accountId 与 messageId 必须为正整数",
                None,
            ));
        }

        let raw_prefs = self.prefs.snapshot_raw();
        let allowed_msgs = self
            .allowed_remote_messages
            .lock()
            .expect("allowed_remote_messages mutex poisoned")
            .clone();

        let (detail, body_loaded) = match self.mode {
            AppMode::Demo => {
                let d = self.load_detail_for_action(input.account_id, input.message_id)?;
                (d, true)
            }
            AppMode::Live => {
                let (data_dir, cached) = {
                    let live = self.live.lock().expect("live mutex poisoned");
                    (
                        live.data_dir.clone(),
                        live.body_cache
                            .get(&(input.account_id, input.message_id))
                            .cloned(),
                    )
                };
                if let Some(d) = cached {
                    (d, true)
                } else {
                    let cache =
                        CacheStore::open_for_data_dir_read_only(&data_dir).map_err(|e| {
                            IpcError::unavailable(format!("本地邮件缓存暂不可用: {e}"), None)
                        })?;
                    let Some((d, _, loaded)) = cache
                        .get_message_detail_ordered(input.message_id, raw_prefs.thread_newest_first)
                        .map_err(|e| IpcError::internal(format!("读取邮件详情失败: {e}"), None))?
                    else {
                        return Err(IpcError::message_not_found(input.message_id, None));
                    };
                    if d.account_id != input.account_id {
                        return Err(IpcError::message_not_found(input.message_id, None));
                    }
                    (d, loaded)
                }
            }
        };

        let body_status = if body_loaded {
            BodyStatusDto::Ready
        } else {
            BodyStatusDto::Loading
        };
        let remote_blocked = body_loaded
            .then(|| is_message_remote_images_blocked(&detail, &raw_prefs, &allowed_msgs));
        let window_label = format!("reader-{}-{}", input.account_id, input.message_id);
        let reader_session_id = format!("window-reader-{}-{}", input.account_id, input.message_id);
        let message = build_open_message_output(
            "open-message-window".to_string(),
            reader_session_id,
            &detail,
            body_status,
            body_loaded,
            remote_blocked,
        );

        Ok(OpenMessageWindowOutput {
            window_label,
            opened: false,
            message,
        })
    }

    pub fn open_attachment(
        &self,
        input: &OpenAttachmentInput,
    ) -> Result<OpenAttachmentOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        if input.attachment_id == 0 {
            return Err(IpcError::invalid_input("attachmentId 必须为正整数", None));
        }

        let (data_dir, raw_filename) = match self.mode {
            AppMode::Demo => {
                let att = {
                    let demo = self.demo.lock().expect("demo mutex poisoned");
                    demo.store
                        .list_cached_attachments("", 0, 10_000)
                        .into_iter()
                        .find(|a| a.id == input.attachment_id)
                }
                .ok_or_else(|| {
                    IpcError::invalid_input(format!("未找到附件 #{}", input.attachment_id), None)
                })?;
                (self.prefs.data_dir().to_path_buf(), att.filename)
            }
            AppMode::Live => {
                let data_dir = self
                    .live
                    .lock()
                    .expect("live mutex poisoned")
                    .data_dir
                    .clone();
                let cache = CacheStore::open_for_data_dir_read_only(&data_dir).map_err(|e| {
                    IpcError::unavailable(format!("本地邮件缓存暂不可用: {e}"), None)
                })?;
                let record = cache
                    .get_attachment_record(input.attachment_id)
                    .map_err(|e| IpcError::internal(format!("读取附件记录失败: {e}"), None))?
                    .ok_or_else(|| {
                        IpcError::invalid_input(
                            format!("未找到附件 #{}", input.attachment_id),
                            None,
                        )
                    })?;
                let filename = match record {
                    AttachmentRecord::Local { filename, .. }
                    | AttachmentRecord::PendingRemote { filename, .. } => filename,
                };
                (data_dir, filename)
            }
        };

        let safe_name = Path::new(&raw_filename)
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::trim)
            .filter(|n| !n.is_empty() && !is_windows_reserved_filename(n))
            .unwrap_or("attachment.bin")
            .to_string();
        let stage_dir = data_dir
            .join("opened-attachments")
            .join(input.attachment_id.to_string());
        let target_path = stage_dir.join(&safe_name);
        let saved = self.save_attachment(&SaveAttachmentInput {
            attachment_id: input.attachment_id,
            dest_path: Some(target_path.to_string_lossy().into_owned()),
        })?;
        let opened = launch_path_with_default_app(&saved.saved_path)?;
        Ok(OpenAttachmentOutput {
            opened,
            path: saved.saved_path.clone(),
            message: format!("已打开附件 {}", safe_name),
        })
    }
}

fn launch_path_with_default_app(path: &str) -> Result<bool, IpcError> {
    #[cfg(all(windows, not(test)))]
    {
        use windows::core::{w, PCWSTR};
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

        let wide: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
        let res = unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                PCWSTR(wide.as_ptr()),
                None,
                None,
                SW_SHOWNORMAL,
            )
        };
        if (res.0 as isize) <= 32 {
            return Err(IpcError::internal("调用系统默认应用打开附件失败", None));
        }
        return Ok(true);
    }
    #[cfg(not(all(windows, not(test))))]
    {
        let _ = path;
        Ok(true)
    }
}

pub fn pick_directory_blocking(parent_hwnd: Option<isize>) -> Result<PickDirectoryOutput, IpcError> {
    #[cfg(all(windows, not(test)))]
    {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::System::Com::{
            CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
            COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
        };
        use windows::Win32::UI::Shell::{
            FileOpenDialog, IFileOpenDialog, FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST,
            FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
        };

        const ERROR_CANCELLED_HRESULT: i32 = 0x8007_04C7u32 as i32;
        let co_hr =
            unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        let needs_uninit = co_hr.is_ok();
        let result = (|| -> Result<PickDirectoryOutput, IpcError> {
            let dialog: IFileOpenDialog =
                unsafe { CoCreateInstance(&FileOpenDialog, None, CLSCTX_ALL) }.map_err(|e| {
                    IpcError::internal(format!("初始化文件夹选择器失败: {e}"), None)
                })?;
            let opts = unsafe { dialog.GetOptions() }.map_err(|e| {
                IpcError::internal(format!("读取文件夹选择器选项失败: {e}"), None)
            })?;
            unsafe {
                dialog
                    .SetOptions(opts | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST)
                    .map_err(|e| {
                        IpcError::internal(format!("配置文件夹选择器失败: {e}"), None)
                    })?;
            }
            let owner = parent_hwnd.map(|h| HWND(h as *mut core::ffi::c_void));
            if let Err(err) = unsafe { dialog.Show(owner) } {
                if err.code().0 == ERROR_CANCELLED_HRESULT {
                    return Ok(PickDirectoryOutput {
                        cancelled: true,
                        path: None,
                    });
                }
                return Err(IpcError::internal(
                    format!("打开文件夹选择器失败: {err}"),
                    None,
                ));
            }
            let item = unsafe { dialog.GetResult() }
                .map_err(|e| IpcError::internal(format!("获取所选目录失败: {e}"), None))?;
            let pwstr = unsafe { item.GetDisplayName(SIGDN_FILESYSPATH) }
                .map_err(|e| IpcError::internal(format!("读取目录路径失败: {e}"), None))?;
            let path = unsafe { pwstr.to_string() }.unwrap_or_default();
            unsafe {
                CoTaskMemFree(Some(pwstr.0 as *const core::ffi::c_void));
            }
            if path.trim().is_empty() {
                Ok(PickDirectoryOutput {
                    cancelled: true,
                    path: None,
                })
            } else {
                Ok(PickDirectoryOutput {
                    cancelled: false,
                    path: Some(path),
                })
            }
        })();
        if needs_uninit {
            unsafe { CoUninitialize() };
        }
        return result;
    }
    #[cfg(not(all(windows, not(test))))]
    {
        let _ = parent_hwnd;
        Ok(PickDirectoryOutput {
            cancelled: true,
            path: None,
        })
    }
}

#[tauri::command]
pub fn open_attachment(
    state: State<'_, AppState>,
    input: OpenAttachmentInput,
) -> Result<OpenAttachmentOutput, IpcError> {
    state.session.open_attachment(&input)
}

#[tauri::command]
pub async fn pick_directory(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<PickDirectoryOutput, IpcError> {
    if state.session.is_locked() {
        return Err(IpcError::locked(None));
    }
    #[cfg(windows)]
    let parent_hwnd: Option<isize> = {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        app.get_webview_window(MAIN_WINDOW_LABEL).and_then(|win| {
            let handle = win.window_handle().ok()?;
            match handle.as_raw() {
                RawWindowHandle::Win32(win32) => Some(win32.hwnd.get()),
                _ => None,
            }
        })
    };
    #[cfg(not(windows))]
    let parent_hwnd: Option<isize> = {
        let _ = app;
        None
    };

    tauri::async_runtime::spawn_blocking(move || pick_directory_blocking(parent_hwnd))
        .await
        .map_err(|e| IpcError::internal(format!("目录选择任务异常: {e}"), None))?
}

#[tauri::command]
pub fn list_cached_attachments(
    state: State<'_, AppState>,
    input: ListCachedAttachmentsInput,
) -> Result<ListCachedAttachmentsOutput, IpcError> {
    state.session.list_cached_attachments(&input)
}

#[tauri::command]
pub fn fetch_attachment_preview(
    state: State<'_, AppState>,
    input: FetchAttachmentPreviewInput,
) -> Result<FetchAttachmentPreviewOutput, IpcError> {
    state.session.fetch_attachment_preview(&input)
}

#[tauri::command]
pub fn save_attachment(
    state: State<'_, AppState>,
    input: SaveAttachmentInput,
) -> Result<SaveAttachmentOutput, IpcError> {
    state.session.save_attachment(&input)
}

#[tauri::command]
pub fn save_all_attachments(
    state: State<'_, AppState>,
    input: SaveAllAttachmentsInput,
) -> Result<SaveAllAttachmentsOutput, IpcError> {
    state.session.save_all_attachments(&input)
}

#[tauri::command]
pub fn get_message_print_html(
    state: State<'_, AppState>,
    input: GetMessagePrintHtmlInput,
) -> Result<GetMessagePrintHtmlOutput, IpcError> {
    state.session.get_message_print_html(&input)
}

#[tauri::command]
pub fn open_message_window(
    state: State<'_, AppState>,
    input: OpenMessageWindowInput,
) -> Result<OpenMessageWindowOutput, IpcError> {
    state.session.open_message_window(&input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::{OpenMessageInput, UnlockAppInput};
    use crate::mail_session::tests::{make_live_test_session, make_test_session};
    use kitemail_mail_core::cache::{CachedAttachmentInput, CachedHeaderInput, RemoteFolder};

    #[test]
    fn base64_encoder_matches_rfc4648_vectors() {
        assert_eq!(encode_base64(b""), "");
        assert_eq!(encode_base64(b"f"), "Zg==");
        assert_eq!(encode_base64(b"fo"), "Zm8=");
        assert_eq!(encode_base64(b"foo"), "Zm9v");
        assert_eq!(encode_base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn path_validation_rejects_traversal_null_and_reserved_names() {
        assert_eq!(
            validate_custom_dest_path("../secret.txt").unwrap_err().code,
            "INVALID_INPUT"
        );
        assert_eq!(
            validate_custom_dest_path(r"C:\tmp\..\Windows\evil.txt")
                .unwrap_err()
                .code,
            "INVALID_INPUT"
        );
        assert_eq!(
            validate_custom_dest_path("C:\\tmp\\evil\0.txt")
                .unwrap_err()
                .code,
            "INVALID_INPUT"
        );
        assert_eq!(
            validate_custom_dest_path("C:\\tmp\\CON.txt")
                .unwrap_err()
                .code,
            "INVALID_INPUT"
        );
        assert_eq!(
            validate_custom_dest_dir("C:\\tmp\\..\\dir")
                .unwrap_err()
                .code,
            "INVALID_INPUT"
        );
        assert!(validate_custom_dest_path(r"C:\tmp\report..final.pdf").is_ok());
    }

    #[test]
    fn sanitize_print_html_strips_scripts_event_handlers_and_js_urls() {
        let dirty = concat!(
            "<p onclick=\"alert(1)\">Hello</p>",
            "<script>evil()</script>",
            "<iframe src=\"https://evil.example\"></iframe>",
            "<img src=\"x\" onerror=alert(2)>",
            "<a href=\"javascript:alert(3)\" onmouseover='steal()'>click</a>",
            "<svg onload=\"alert(4)\"><circle cx=\"1\" cy=\"1\" r=\"1\"/></svg>"
        );
        let clean = sanitize_print_html(dirty, false, false);
        assert!(!clean.contains("<script"));
        assert!(!clean.contains("evil()"));
        assert!(!clean.contains("<iframe"));
        assert!(!clean.contains("onclick"));
        assert!(!clean.contains("onerror"));
        assert!(!clean.contains("onmouseover"));
        assert!(!clean.contains("onload"));
        assert!(!clean.contains("javascript:"));
        assert!(clean.contains("Hello"));
    }

    #[test]
    fn demo_attachments_gallery_preview_save_print_and_standalone_window_work() {
        let (session, _, dir) = make_test_session();

        let page = session
            .list_cached_attachments(&ListCachedAttachmentsInput {
                query: Some("截图".to_string()),
                offset: Some(0),
                limit: Some(10),
            })
            .expect("list demo attachments");
        assert!(!page.items.is_empty());
        assert!(page.items.iter().all(|a| a.is_image));
        let first_img_id = page.items[0].id;

        let preview = session
            .fetch_attachment_preview(&FetchAttachmentPreviewInput {
                attachment_id: first_img_id,
            })
            .expect("preview demo image");
        assert!(preview.data_url.starts_with("data:image/svg+xml;base64,"));

        // Save to default isolated directory with deduplication.
        let saved1 = session
            .save_attachment(&SaveAttachmentInput {
                attachment_id: first_img_id,
                dest_path: None,
            })
            .expect("save #1");
        let saved2 = session
            .save_attachment(&SaveAttachmentInput {
                attachment_id: first_img_id,
                dest_path: None,
            })
            .expect("save #2");
        assert!(saved1.saved && saved2.saved);
        assert_ne!(saved1.saved_path, saved2.saved_path);
        assert!(Path::new(&saved1.saved_path).is_file());
        assert!(Path::new(&saved2.saved_path).is_file());

        // Print HTML includes header and @media print, or omits header when requested.
        let print_with_hdr = session
            .get_message_print_html(&GetMessagePrintHtmlInput {
                account_id: 1,
                message_id: 1,
                include_header: Some(true),
            })
            .expect("print with header");
        assert!(print_with_hdr.html_document.contains("@media print"));
        assert!(print_with_hdr.html_document.contains("print-header"));

        let print_no_hdr = session
            .get_message_print_html(&GetMessagePrintHtmlInput {
                account_id: 1,
                message_id: 1,
                include_header: Some(false),
            })
            .expect("print without header");
        assert!(!print_no_hdr
            .html_document
            .contains("<header class=\"print-header\">"));

        // Open main reader on #1, then open standalone window on #2 -> main reader stays on #1.
        let main_open = session
            .open_message(
                &OpenMessageInput {
                    request_id: "main-1".to_string(),
                    account_id: 1,
                    message_id: 1,
                },
                "rs-main-1".to_string(),
            )
            .expect("open main #1");
        assert_eq!(main_open.message_id, 1);

        let win_open = session
            .open_message_window(&OpenMessageWindowInput {
                account_id: 1,
                message_id: 2,
            })
            .expect("open standalone #2");
        assert_eq!(win_open.window_label, "reader-1-2");
        assert!(!win_open.opened);
        assert_eq!(win_open.message.message_id, 2);

        let active_selected = *session.active_selected_msg.lock().unwrap();
        assert_eq!(active_selected, Some((1, 1)));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn live_attachments_reject_when_locked_and_enforce_10mb_preview_cap() {
        let (session, _, _, _, dir) = make_live_test_session(false, Some("1234"));

        // Seed cache with one normal attachment and one >10MB pending attachment.
        let (small_id, huge_id) = {
            let mut store = CacheStore::open_for_data_dir(&dir).unwrap();
            let folders = store
                .sync_folders(
                    1,
                    &[RemoteFolder {
                        name: "收件箱".to_string(),
                        path: "INBOX".to_string(),
                        unread_count: 1,
                        total_count: 1,
                    }],
                )
                .unwrap();
            let inbox_id = folders[0].id;
            store
                .commit_folder_headers(
                    1,
                    inbox_id,
                    1,
                    &[CachedHeaderInput {
                        uid: 1,
                        message_id: "<att@example.com>".to_string(),
                        in_reply_to: String::new(),
                        references_header: String::new(),
                        sender: "Sender <s@example.com>".to_string(),
                        recipients: "tester@example.com".to_string(),
                        cc: String::new(),
                        subject: "附件测试".to_string(),
                        preview: "附件".to_string(),
                        date_ts: 1_700_000_000,
                        date_display: "10:00".to_string(),
                        unread: true,
                        flagged: false,
                        has_attachment: true,
                    }],
                    None,
                )
                .unwrap();
            let page = store.list_messages_page(inbox_id, "", 0, 10).unwrap();
            let msg_id = page.items[0].id;
            store
                .store_message_body_with_html(
                    msg_id,
                    "正文",
                    Some("<p>正文</p><script>alert(1)</script>"),
                    &[
                        CachedAttachmentInput::downloaded(
                            "note.txt",
                            "text/plain",
                            b"hello".to_vec(),
                        ),
                        CachedAttachmentInput {
                            filename: "huge.png".to_string(),
                            mime_type: "image/png".to_string(),
                            size_bytes: MAX_ATTACHMENT_PREVIEW_BYTES + 1,
                            content: None,
                            remote_part: "2".to_string(),
                            transfer_encoding: "base64".to_string(),
                        },
                    ],
                    None,
                    false,
                    None,
                )
                .unwrap();
            let atts = store.list_cached_attachments("", 0, 10).unwrap();
            let small = atts.iter().find(|a| a.filename == "note.txt").unwrap().id;
            let huge = atts.iter().find(|a| a.filename == "huge.png").unwrap().id;
            store.checkpoint_and_close_all();
            (small, huge)
        };

        session.lock_app(Some("manual"));
        assert_eq!(
            session
                .list_cached_attachments(&ListCachedAttachmentsInput::default())
                .unwrap_err()
                .code,
            "LOCKED"
        );
        assert_eq!(
            session
                .fetch_attachment_preview(&FetchAttachmentPreviewInput {
                    attachment_id: small_id
                })
                .unwrap_err()
                .code,
            "LOCKED"
        );
        assert_eq!(
            session
                .save_attachment(&SaveAttachmentInput {
                    attachment_id: small_id,
                    dest_path: None
                })
                .unwrap_err()
                .code,
            "LOCKED"
        );
        assert_eq!(
            session
                .save_all_attachments(&SaveAllAttachmentsInput {
                    attachment_ids: vec![small_id],
                    dest_dir: None
                })
                .unwrap_err()
                .code,
            "LOCKED"
        );
        assert_eq!(
            session
                .get_message_print_html(&GetMessagePrintHtmlInput {
                    account_id: 1,
                    message_id: 1,
                    include_header: None
                })
                .unwrap_err()
                .code,
            "LOCKED"
        );
        assert_eq!(
            session
                .open_message_window(&OpenMessageWindowInput {
                    account_id: 1,
                    message_id: 1
                })
                .unwrap_err()
                .code,
            "LOCKED"
        );

        session.unlock_app(&UnlockAppInput {
            passcode: "1234".to_string(),
        });

        let preview_ok = session
            .fetch_attachment_preview(&FetchAttachmentPreviewInput {
                attachment_id: small_id,
            })
            .expect("small text preview");
        assert_eq!(preview_ok.data_url, "data:text/plain;base64,aGVsbG8=");

        let preview_huge = session
            .fetch_attachment_preview(&FetchAttachmentPreviewInput {
                attachment_id: huge_id,
            })
            .unwrap_err();
        assert_eq!(preview_huge.code, "INVALID_INPUT");

        let dest_dir = dir.join("batch");
        let batch = session
            .save_all_attachments(&SaveAllAttachmentsInput {
                attachment_ids: vec![small_id],
                dest_dir: Some(dest_dir.to_string_lossy().into_owned()),
            })
            .expect("save all local");
        assert_eq!(batch.saved_count, 1);
        assert!(Path::new(&batch.saved_paths[0]).is_file());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
