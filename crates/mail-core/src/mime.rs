//! MIME inline `cid:` image resolution and reply quote-block splitting.
//!
//! - [`resolve_cid_inline_images`]: extracts `Content-ID` image parts from a
//!   parsed RFC 822 message and rewrites `cid:...` references in the HTML body
//!   to `data:image/...;base64,...` URIs so native `TextView::html` can render
//!   inline pictures offline.
//! - [`split_quoted_html`] / [`split_quoted_plain`]: detects historical reply
//!   quotes (`<blockquote`, `gmail_quote`, `On ... wrote:`, `>` blocks) so the
//!   reader can fold them behind a collapsible `⋯ 显示引用内容 (N 行)` pill.

use std::collections::HashMap;

use base64::Engine as _;
use mail_parser::{Message, MimeHeaders};

/// Result of splitting a message body into its primary content and an optional
/// trailing quoted reply block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuoteSplit {
    /// Primary (top-level) body content.
    pub main: String,
    /// Optional `(quoted_content, estimated_line_count)`.
    pub quoted: Option<(String, usize)>,
}

/// Normalize a `Content-ID` header value or `cid:` target by trimming
/// whitespace and surrounding `<...>` angle brackets, lowercasing for lookup.
fn normalize_cid(raw: &str) -> String {
    let trimmed = raw.trim();
    let stripped = trimmed
        .strip_prefix('<')
        .and_then(|s| s.strip_suffix('>'))
        .unwrap_or(trimmed);
    stripped.trim().to_ascii_lowercase()
}

/// Infer an `image/...` MIME type for an inline part when the declared type is
/// missing or generic `application/octet-stream`.
fn detect_image_mime(part_mime: Option<(&str, Option<&str>)>, bytes: &[u8]) -> Option<String> {
    if let Some((ctype, subtype)) = part_mime {
        if ctype.eq_ignore_ascii_case("image") {
            let sub = subtype.unwrap_or("png").to_ascii_lowercase();
            return Some(format!("image/{sub}"));
        }
        if !ctype.eq_ignore_ascii_case("application") {
            return None;
        }
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png".to_string())
    } else if bytes.starts_with(b"\xFF\xD8\xFF") {
        Some("image/jpeg".to_string())
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif".to_string())
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp".to_string())
    } else if bytes.starts_with(b"BM") {
        Some("image/bmp".to_string())
    } else {
        None
    }
}

/// Build a map from normalized `Content-ID` to `data:image/...;base64,...` URI
/// from all inline image parts in `parsed`.
pub fn extract_cid_data_uris(parsed: &Message<'_>) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for part in &parsed.parts {
        let Some(cid) = part.content_id() else {
            continue;
        };
        let key = normalize_cid(cid);
        if key.is_empty() {
            continue;
        }
        let bytes = part.contents();
        if bytes.is_empty() {
            continue;
        }
        let ct = part.content_type().map(|c| (c.ctype(), c.subtype()));
        let Some(mime) = detect_image_mime(ct, bytes) else {
            continue;
        };
        let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
        map.insert(key, format!("data:{mime};base64,{b64}"));
    }
    map
}

/// Replace all `cid:<id>` references in `html` using `cid_map` (keys must be
/// normalized via lowercase without angle brackets).
pub fn replace_cid_urls(html: String, cid_map: &HashMap<String, String>) -> String {
    if cid_map.is_empty() || html.is_empty() {
        return html;
    }
    let lower = html.to_ascii_lowercase();
    if !lower.contains("cid:") {
        return html;
    }

    let mut out = String::with_capacity(html.len());
    let mut cursor = 0usize;
    let bytes = html.as_bytes();

    while let Some(rel) = lower[cursor..].find("cid:") {
        let cid_start = cursor + rel;
        let id_start = cid_start + 4;
        // Stop at quote, whitespace, closing paren, angle bracket, or &quot;.
        let mut id_end = id_start;
        while id_end < bytes.len() {
            let b = bytes[id_end];
            if matches!(
                b,
                b'"' | b'\'' | b')' | b'>' | b'<' | b' ' | b'\t' | b'\r' | b'\n'
            ) {
                break;
            }
            id_end += 1;
        }
        let raw_id = &html[id_start..id_end];
        let norm = normalize_cid(raw_id);
        let decoded_norm = percent_decode_cid(&norm);
        let replacement = cid_map.get(&norm).or_else(|| cid_map.get(&decoded_norm));

        if let Some(data_uri) = replacement {
            out.push_str(&html[cursor..cid_start]);
            out.push_str(data_uri);
            cursor = id_end;
        } else {
            out.push_str(&html[cursor..id_end]);
            cursor = id_end;
        }
    }
    out.push_str(&html[cursor..]);
    out
}

/// Decode simple `%XX` sequences in a CID string (e.g. `%40` -> `@`).
fn percent_decode_cid(input: &str) -> String {
    if !input.contains('%') {
        return input.to_string();
    }
    let mut out = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
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
    String::from_utf8_lossy(&out).to_ascii_lowercase()
}

/// Extract `Content-ID` inline images from `parsed` and rewrite `cid:` URIs in
/// `html` into `data:image/...;base64,...` URIs.
pub fn resolve_cid_inline_images(parsed: &Message<'_>, html: String) -> String {
    let map = extract_cid_data_uris(parsed);
    replace_cid_urls(html, &map)
}

/// Estimate visible line count from an HTML fragment (for the fold pill label).
fn count_html_lines(html: &str) -> usize {
    let text = crate::imap_client::html_to_readable_text(html);
    text.lines().filter(|l| !l.trim().is_empty()).count().max(1)
}

/// Find the earliest quote marker in sanitized/raw HTML and split the document
/// into `(main_html, Some((quoted_html, line_count)))`.
///
/// Recognizes:
/// - `<div class="gmail_quote"...`
/// - `<div id="divRplyFwdMsg"...` / `<div id="appendonsend"...`
/// - `<blockquote...` (when preceded by non-empty main content)
pub fn split_quoted_html(html: &str) -> QuoteSplit {
    let lower = html.to_ascii_lowercase();
    let markers = [
        "<div class=\"gmail_quote\"",
        "<div class='gmail_quote'",
        "<div id=\"divrplyfwdmsg\"",
        "<div id=\"appendonsend\"",
        "<blockquote",
    ];

    let split_at = markers.iter().filter_map(|m| lower.find(m)).min();

    let Some(idx) = split_at else {
        return QuoteSplit {
            main: html.to_string(),
            quoted: None,
        };
    };

    let main_part = html[..idx].trim_end();
    let quoted_part = html[idx..].trim();

    // Only fold if there is actual content before AND inside the quote.
    let main_readable = crate::imap_client::html_to_readable_text(main_part);
    if main_readable.trim().is_empty() || quoted_part.is_empty() {
        return QuoteSplit {
            main: html.to_string(),
            quoted: None,
        };
    }

    let lines = count_html_lines(quoted_part);
    QuoteSplit {
        main: main_part.to_string(),
        quoted: Some((quoted_part.to_string(), lines)),
    }
}

/// Returns `true` if `line` looks like a reply attribution header
/// (e.g., `On Mon, Jan 1, 2026, Alice wrote:` or `在 2026年... 写道：` or `-----Original Message-----`).
fn is_attribution_line(line: &str) -> bool {
    let t = line.trim();
    if t.is_empty() {
        return false;
    }
    let lower = t.to_ascii_lowercase();
    (lower.starts_with("on ") && (lower.ends_with("wrote:") || lower.ends_with("wrote：")))
        || (t.starts_with("在 ") && (t.ends_with("写道：") || t.ends_with("写道:")))
        || lower.contains("-----original message-----")
        || lower.contains("--- original message ---")
        || lower.contains("原始邮件")
}

/// Split a plain-text message body into primary content and trailing quoted
/// reply block (`On ... wrote:` / `>` lines).
pub fn split_quoted_plain(text: &str) -> QuoteSplit {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() < 3 {
        return QuoteSplit {
            main: text.to_string(),
            quoted: None,
        };
    }

    // Look for an attribution line followed by `>` lines or quote content,
    // or a contiguous trailing block of >= 2 `>`-prefixed lines.
    let mut split_idx: Option<usize> = None;

    for (i, line) in lines.iter().enumerate() {
        if i == 0 {
            continue;
        }
        if is_attribution_line(line) {
            let remaining = lines[i + 1..]
                .iter()
                .filter(|l| !l.trim().is_empty())
                .count();
            if remaining >= 1 {
                split_idx = Some(i);
                break;
            }
        }
    }

    if split_idx.is_none() {
        // Check for trailing `>` block (allow blank lines inside the block).
        let mut first_quote = None;
        let mut quote_count = 0usize;
        for (i, line) in lines.iter().enumerate().rev() {
            let trimmed = line.trim_start();
            if trimmed.starts_with('>') {
                first_quote = Some(i);
                quote_count += 1;
            } else if trimmed.is_empty() {
                continue;
            } else {
                break;
            }
        }
        if quote_count >= 2 {
            if let Some(fq) = first_quote {
                if fq > 0 {
                    // Include an immediately preceding attribution line if present.
                    let prev_non_empty = lines[..fq].iter().rposition(|l| !l.trim().is_empty());
                    if let Some(p) = prev_non_empty {
                        if p > 0 && is_attribution_line(lines[p]) {
                            split_idx = Some(p);
                        } else {
                            split_idx = Some(fq);
                        }
                    } else {
                        split_idx = Some(fq);
                    }
                }
            }
        }
    }

    let Some(idx) = split_idx else {
        return QuoteSplit {
            main: text.to_string(),
            quoted: None,
        };
    };

    let main_lines = &lines[..idx];
    let quoted_lines = &lines[idx..];
    if main_lines.iter().all(|l| l.trim().is_empty()) {
        return QuoteSplit {
            main: text.to_string(),
            quoted: None,
        };
    }

    let main_str = main_lines.join("\n").trim_end().to_string();
    let quoted_str = quoted_lines.join("\n").trim().to_string();
    let count = quoted_lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .count()
        .max(1);

    QuoteSplit {
        main: main_str,
        quoted: Some((quoted_str, count)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_cid_urls_rewrites_matching_inline_images() {
        let mut map = HashMap::new();
        map.insert(
            "logo123@example.com".to_string(),
            "data:image/png;base64,AAAA".to_string(),
        );
        let html = r#"<p>Hello</p><img src="cid:logo123@example.com" alt="logo"><img src="cid:missing@x">"#
            .to_string();
        let out = replace_cid_urls(html, &map);
        assert!(out.contains(r#"src="data:image/png;base64,AAAA""#));
        assert!(out.contains(r#"src="cid:missing@x""#));
    }

    #[test]
    fn resolve_cid_inline_images_parses_rfc822_multipart_related() {
        let raw = concat!(
            "From: a@example.com\r\n",
            "To: b@example.com\r\n",
            "Subject: CID test\r\n",
            "MIME-Version: 1.0\r\n",
            "Content-Type: multipart/related; boundary=\"rel1\"\r\n\r\n",
            "--rel1\r\n",
            "Content-Type: text/html; charset=utf-8\r\n\r\n",
            "<p>Hi <img src=\"cid:img1@test\"></p>\r\n",
            "--rel1\r\n",
            "Content-Type: image/png\r\n",
            "Content-ID: <img1@test>\r\n",
            "Content-Transfer-Encoding: base64\r\n\r\n",
            "iVBORw0KGgo=\r\n",
            "--rel1--\r\n",
        );
        let parsed = mail_parser::MessageParser::default()
            .parse(raw.as_bytes())
            .unwrap();
        let html = parsed.body_html(0).unwrap().into_owned();
        let resolved = resolve_cid_inline_images(&parsed, html);
        assert!(
            resolved.contains("data:image/png;base64,"),
            "expected data URI in {resolved}"
        );
    }

    #[test]
    fn split_quoted_html_folds_gmail_quote() {
        let html = r#"<div>Thanks!</div><div class="gmail_quote"><div>On Mon, Alice wrote:</div><blockquote><p>Line 1</p><p>Line 2</p></blockquote></div>"#;
        let split = split_quoted_html(html);
        assert_eq!(split.main, "<div>Thanks!</div>");
        let (q, lines) = split.quoted.expect("should split gmail_quote");
        assert!(q.contains("Line 1"));
        assert!(lines >= 2);
    }

    #[test]
    fn split_quoted_plain_folds_attribution_and_gt_lines() {
        let text = "Sounds good, see you tomorrow.\n\nOn Mon, Jan 1, Alice wrote:\n> Hello\n> Are we meeting?";
        let split = split_quoted_plain(text);
        assert_eq!(split.main, "Sounds good, see you tomorrow.");
        let (q, lines) = split.quoted.expect("should split plain quote");
        assert!(q.starts_with("On Mon, Jan 1, Alice wrote:"));
        assert_eq!(lines, 3);
    }
}
