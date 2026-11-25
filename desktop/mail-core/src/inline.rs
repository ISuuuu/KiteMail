//! 正文里的内联图片：把 `data:` URI 提升成随信发送的 `cid:` 附件。
//!
//! 签名里的本地 logo、正文里粘贴的图片，在写信链路里都是
//! `data:image/…;base64,…`。原样发出去会被 Gmail / Outlook 拦掉（它们只认
//! `cid:` 引用 + `multipart/related` 里的内联段），所以发送前把字节抽出来，
//! HTML 里换成 `cid:`，由 SMTP / Graph 各自作为 inline 附件带上。
//!
//! 处理两处：`<img src="data:…">` 与 `style="…url(data:…)"` 里的背景图。
//!
//! 对齐 Linux 原版：签名导入时把本地图片读成 `data:`，发送时再提升为
//! `cid:` part 的做法。

use base64::Engine as _;
use sha2::{Digest, Sha256};

/// 单张内联图的上限；超过就留在原文里不动，至少不会把整封信卡死。
const MAX_INLINE_BYTES: usize = 8 * 1024 * 1024;

/// 一张待随信发送的内联图片。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineImage {
    /// 不带尖括号的 Content-ID；HTML 里写作 `cid:{content_id}`。
    pub content_id: String,
    pub filename: String,
    pub mime: String,
    pub data: Vec<u8>,
}

/// 把 HTML 里的 `data:image/…;base64,…` 换成 `cid:…`。
///
/// 返回改写后的 HTML 和要去重发送的图片；相同字节只保留一张。非图片
/// `data:`、解不出来的 base64、过大的图片一律保持原样。
pub fn extract_data_images(html: &str) -> (String, Vec<InlineImage>) {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut images: Vec<InlineImage> = Vec::new();
    let mut pos = 0usize;

    while let Some((at, kind)) = next_attr(&lower, pos) {
        let Some((value_start, value_end, value, quote)) = attr_value(html, at, kind.len()) else {
            out.push_str(&html[pos..at + kind.len()]);
            pos = at + kind.len();
            continue;
        };
        out.push_str(&html[pos..value_start]);
        let rewritten = match kind {
            Attr::Src => image_from_data_uri(value)
                .map(|(mime, data)| format!("cid:{}", add_image(mime, data, &mut images))),
            Attr::Style => rewrite_css_urls(value, &mut images),
        };
        match rewritten {
            Some(value) => out.push_str(&value),
            None => out.push_str(value),
        }
        // 值的收尾引号原样保留。
        out.push(quote);
        pos = value_end;
    }
    out.push_str(&html[pos..]);
    (out, images)
}

/// 会被改写的属性。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Attr {
    /// `<img src="data:…">`
    Src,
    /// `style="…url(data:…)…"`
    Style,
}

impl Attr {
    fn len(self) -> usize {
        match self {
            Attr::Src => 4,
            Attr::Style => 6,
        }
    }
}

/// 下一个 ` src=` / ` style=` 的位置（前面必须是空白，避开 `data-src=`）。
fn next_attr(lower: &str, from: usize) -> Option<(usize, Attr)> {
    let src = find_attr(lower, from, "src=");
    let style = find_attr(lower, from, "style=");
    match (src, style) {
        (Some(src), Some(style)) if src <= style => Some((src, Attr::Src)),
        (Some(src), None) => Some((src, Attr::Src)),
        (_, Some(style)) => Some((style, Attr::Style)),
        (None, None) => None,
    }
}

fn find_attr(lower: &str, from: usize, needle: &str) -> Option<usize> {
    let mut from = from;
    while let Some(i) = lower.get(from..)?.find(needle) {
        let at = from + i;
        if at > 0 && lower.as_bytes()[at - 1].is_ascii_whitespace() {
            return Some(at);
        }
        from = at + needle.len();
    }
    None
}

/// 解析 `attr=` 后面带引号的值：返回（值起点，值后的字节，值，引号）。
fn attr_value(html: &str, at: usize, attr_len: usize) -> Option<(usize, usize, &str, char)> {
    let after = html.get(at + attr_len..)?;
    let trimmed = after.trim_start();
    let spaces = after.len() - trimmed.len();
    let quote = trimmed.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let value_start = at + attr_len + spaces + quote.len_utf8();
    let rest = html.get(value_start..)?;
    let len = rest.find(quote)?;
    Some((
        value_start,
        value_start + len + quote.len_utf8(),
        &rest[..len],
        quote,
    ))
}

/// style 里每个 `url(data:image/…)` 换成 `url(cid:…)`；没有可换的返回 `None`。
///
/// 解析到一半失败时整段放弃（调用方写回原值），不会留下只换了一半的 style。
fn rewrite_css_urls(style: &str, images: &mut Vec<InlineImage>) -> Option<String> {
    let lower = style.to_ascii_lowercase();
    let mut found: Vec<InlineImage> = Vec::new();
    let mut out = String::with_capacity(style.len());
    let mut pos = 0usize;
    let mut changed = false;

    while let Some(i) = lower.get(pos..).and_then(|s| s.find("url(")) {
        let open = pos + i + 4;
        let after = style.get(open..)?;
        let trimmed = after.trim_start();
        let spaces = after.len() - trimmed.len();
        let (quote, body_start) = match trimmed.chars().next() {
            Some(quote @ ('"' | '\'')) => (Some(quote), open + spaces + quote.len_utf8()),
            _ => (None, open + spaces),
        };
        let rest = style.get(body_start..)?;
        let end = match quote {
            Some(quote) => rest.find(quote)?,
            None => rest.find(')')?,
        };
        match image_from_data_uri(&rest[..end]) {
            Some((mime, data)) => {
                out.push_str(&style[pos..body_start]);
                out.push_str("cid:");
                out.push_str(&add_image(mime, data, &mut found));
                changed = true;
            }
            None => out.push_str(&style[pos..body_start + end]),
        }
        pos = body_start + end;
    }
    out.push_str(&style[pos..]);

    if !changed {
        return None;
    }
    for image in found {
        if !images.iter().any(|img| img.content_id == image.content_id) {
            images.push(image);
        }
    }
    Some(out)
}

/// 记录一张图片并返回它的 Content-ID；相同字节复用同一个 id。
fn add_image(mime: String, data: Vec<u8>, images: &mut Vec<InlineImage>) -> String {
    let content_id = content_id(&data);
    if !images.iter().any(|img| img.content_id == content_id) {
        images.push(InlineImage {
            filename: filename_for(&content_id, &mime),
            content_id: content_id.clone(),
            mime,
            data,
        });
    }
    content_id
}

/// `data:image/…;base64,…` 解出 MIME 与字节；不是图片 data URI 时返回 `None`。
fn image_from_data_uri(value: &str) -> Option<(String, Vec<u8>)> {
    let v = value.trim();
    let Some(scheme) = v.get(..5) else {
        return None;
    };
    if !scheme.eq_ignore_ascii_case("data:") {
        return None;
    }
    let comma = v.find(',')?;
    let meta = v[5..comma].to_ascii_lowercase();
    let mime = meta.strip_suffix(";base64")?.trim();
    let mime = normalized_image_mime(mime)?;
    let payload: String = v[comma + 1..]
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let data = base64::engine::general_purpose::STANDARD
        .decode(payload)
        .ok()?;
    if data.is_empty() || data.len() > MAX_INLINE_BYTES {
        return None;
    }
    Some((mime, data))
}

/// 只认邮件里常见的位图类型；`image/jpg` 归一成 `image/jpeg`。
fn normalized_image_mime(mime: &str) -> Option<String> {
    let mime = match mime {
        "image/jpg" => "image/jpeg",
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/bmp" | "image/svg+xml"
        | "image/avif" => mime,
        _ => return None,
    };
    Some(mime.to_string())
}

/// 由图片字节算出的稳定 Content-ID：同图重复出现时自动去重。
fn content_id(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    let mut hex = String::with_capacity(16);
    for byte in digest.iter().take(8) {
        hex.push_str(&format!("{byte:02x}"));
    }
    format!("hylki-{hex}@inline")
}

fn filename_for(content_id: &str, mime: &str) -> String {
    let ext = match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        "image/svg+xml" => "svg",
        _ => "avif",
    };
    let stem = content_id
        .strip_prefix("hylki-")
        .and_then(|s| s.split('@').next())
        .unwrap_or("image");
    format!("image-{stem}.{ext}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1×1 PNG 的 base64，足够当夹具。
    const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

    #[test]
    fn data_image_becomes_cid_reference() {
        let html = format!(r#"<p><img src="data:image/png;base64,{PNG}" width="10"></p>"#);
        let (out, images) = extract_data_images(&html);
        assert!(!out.contains("data:"), "{out}");
        assert_eq!(images.len(), 1);
        assert!(out.contains(&format!("src=\"cid:{}\"", images[0].content_id)));
        assert_eq!(images[0].mime, "image/png");
        assert!(out.contains("width=\"10\""), "{out}");
        assert!(!images[0].data.is_empty());
    }

    #[test]
    fn css_background_image_is_lifted() {
        let html = format!(
            r#"<div style="background-image:url(data:image/png;base64,{PNG});color:#333">x</div>"#
        );
        let (out, images) = extract_data_images(&html);
        assert_eq!(images.len(), 1);
        assert!(out.contains("url(cid:"), "{out}");
        assert!(!out.contains("data:"), "{out}");
        assert!(out.contains("color:#333"), "其余样式应保留: {out}");

        // 带引号的 url('…') 同样处理。
        let quoted =
            format!(r#"<div style="background:url('data:image/png;base64,{PNG}')"></div>"#);
        let (out, images) = extract_data_images(&quoted);
        assert_eq!(images.len(), 1);
        assert!(out.contains("url('cid:"), "{out}");
    }

    #[test]
    fn identical_images_are_sent_once() {
        let html = format!(
            r#"<img src="data:image/png;base64,{PNG}"><img src='data:image/png;base64,{PNG}'>"#
        );
        let (out, images) = extract_data_images(&html);
        assert_eq!(images.len(), 1);
        assert_eq!(out.matches("cid:").count(), 2, "{out}");
    }

    #[test]
    fn non_image_and_broken_data_uris_are_left_alone() {
        let html = r#"<a href="data:text/html;base64,PGI+eA==">x</a>
            <img src="data:image/png;base64,!!!!">
            <img src="https://x.example/a.png">
            <img src="logo.png">
            <p style="color:red">y</p>"#;
        let (out, images) = extract_data_images(html);
        assert!(images.is_empty());
        assert_eq!(out, html);
    }

    #[test]
    fn unquoted_src_is_skipped_without_corrupting_html() {
        let html = "<img src=data:image/png;base64,AAAA><p>after</p>";
        let (out, images) = extract_data_images(html);
        assert!(images.is_empty());
        assert_eq!(out, html);
    }
}
