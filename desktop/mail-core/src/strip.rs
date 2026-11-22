//! Removing an attachment from a message on the server (#289).
//!
//! IMAP servers cannot edit a message in place: the only way to take a file out
//! is to store a copy without it (`APPEND`) and delete the original. The copy
//! is the original byte for byte except for the one MIME part, which becomes a
//! short note in Thunderbird's format (`text/x-moz-deleted`), so either client
//! recognises what the other removed.

use base64::Engine as _;
use mail_parser::{MessageParser, MessagePart, MimeHeaders};

/// Whether a MIME part is the placeholder note left where an attachment was removed.
pub fn is_placeholder(part: &MessagePart<'_>) -> bool {
    part.content_type().is_some_and(|c| {
        c.ctype().eq_ignore_ascii_case("text")
            && c.subtype()
                .is_some_and(|s| s.eq_ignore_ascii_case("x-moz-deleted"))
    })
}

/// Why an attachment could not be removed.
#[derive(Debug, PartialEq, Eq)]
pub enum StripError {
    /// The message could not be parsed, or no attachment by that name is in it.
    NotFound,
    /// The part sits under a signature or inside encryption: changing it would
    /// break the signature, and cannot be done to encrypted content.
    Protected,
    /// The attachment is the whole message (a single-part message).
    WholeMessage,
}

impl StripError {
    pub fn text(&self) -> &'static str {
        match self {
            StripError::NotFound => "服务器上已找不到该附件",
            StripError::Protected => "该邮件已签名或加密，无法剥离附件",
            StripError::WholeMessage => "该附件即整封邮件，请直接删除邮件",
        }
    }
}

/// Timestamp for `X-Mozilla-Altered` in UTC (`YYYY-MM-DDTHH:MM:SSZ`).
pub fn note_date() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// Encode `bytes` as MIME base64 wrapped at 76 characters per line.
fn base64_lines(bytes: &[u8], eol: &str) -> String {
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    let mut out = String::with_capacity(encoded.len() + (encoded.len() / 76 + 1) * eol.len());
    for chunk in encoded.as_bytes().chunks(76) {
        if let Ok(s) = std::str::from_utf8(chunk) {
            out.push_str(s);
            out.push_str(eol);
        }
    }
    out
}

/// A quoted header parameter, or RFC 2047 encoded-word when non-ASCII.
fn header_param(value: &str) -> String {
    if value.is_ascii() {
        let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{escaped}\"")
    } else {
        let b64 = base64::engine::general_purpose::STANDARD.encode(value.as_bytes());
        format!("\"=?UTF-8?B?{b64}?=\"")
    }
}

/// Build the `text/x-moz-deleted` placeholder MIME part.
fn placeholder(name: &str, original_headers: &[u8], date: &str, eol: &str) -> Vec<u8> {
    let label = header_param(&format!("Deleted: {name}"));
    let note =
        "You deleted an attachment from this message. The original MIME headers for the attachment were:";
    let mut body = note.as_bytes().to_vec();
    body.extend_from_slice(eol.as_bytes());

    let text = String::from_utf8_lossy(original_headers);
    for line in text.trim_end_matches(['\r', '\n']).lines() {
        body.extend_from_slice(line.trim_end_matches('\r').as_bytes());
        body.extend_from_slice(eol.as_bytes());
    }

    let ascii = body.is_ascii();
    let mut out = Vec::new();
    let mut push_line = |s: &str| {
        out.extend_from_slice(s.as_bytes());
        out.extend_from_slice(eol.as_bytes());
    };
    push_line(&format!(
        "Content-Type: text/x-moz-deleted; charset=UTF-8; name={label}"
    ));
    push_line(if ascii {
        "Content-Transfer-Encoding: 7bit"
    } else {
        "Content-Transfer-Encoding: base64"
    });
    push_line(&format!("Content-Disposition: inline; filename={label}"));
    push_line(&format!(
        "X-Mozilla-Altered: AttachmentDeleted; date=\"{date}\""
    ));
    push_line("");
    if ascii {
        out.extend_from_slice(&body);
    } else {
        out.extend_from_slice(base64_lines(&body, eol).as_bytes());
    }
    if out.ends_with(eol.as_bytes()) {
        out.truncate(out.len() - eol.len());
    }
    out
}

/// Rewrite `raw` RFC 822 bytes without the attachment named `name` (picking the
/// closest decoded size to `size` when multiple share the name).
pub fn without_attachment(
    raw: &[u8],
    name: &str,
    size: u64,
    date: &str,
) -> Result<Vec<u8>, StripError> {
    let parsed = MessageParser::default()
        .parse(raw)
        .ok_or(StripError::NotFound)?;

    // Locate the matching attachment part index in `parsed.parts`.
    let mut best: Option<(usize, u64)> = None;
    for (idx, part) in parsed.parts.iter().enumerate() {
        if is_placeholder(part) {
            continue;
        }
        let Some(part_name) = part.attachment_name() else {
            continue;
        };
        if part_name == name {
            let diff = (part.contents().len() as u64).abs_diff(size);
            if best.is_none_or(|(_, best_diff)| diff < best_diff) {
                best = Some((idx, diff));
            }
        }
    }

    let (id, _) = best.ok_or(StripError::NotFound)?;
    if id == 0 {
        return Err(StripError::WholeMessage);
    }

    let part = &parsed.parts[id];
    let (start, end) = (part.offset_header, part.offset_end);
    let protected = parsed.parts.iter().any(|p| {
        p.offset_header <= start
            && end <= p.offset_end
            && p.content_type().is_some_and(|c| {
                let sub = c.subtype().unwrap_or_default();
                (c.ctype().eq_ignore_ascii_case("multipart")
                    && (sub.eq_ignore_ascii_case("signed")
                        || sub.eq_ignore_ascii_case("encrypted")))
                    || (c.ctype().eq_ignore_ascii_case("application")
                        && sub.eq_ignore_ascii_case("pkcs7-mime"))
            })
    });
    if protected {
        return Err(StripError::Protected);
    }

    let headers = raw
        .get(start..part.offset_body)
        .ok_or(StripError::NotFound)?;
    let eol = if raw.windows(2).any(|w| w == b"\r\n") {
        "\r\n"
    } else {
        "\n"
    };

    let mut out = Vec::with_capacity(raw.len().saturating_sub(end - start) + 512);
    out.extend_from_slice(&raw[..start]);
    out.extend_from_slice(&placeholder(name, headers, date, eol));
    out.extend_from_slice(&raw[end..]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_attachment_replaces_part_with_moz_deleted_placeholder() {
        let raw = concat!(
            "From: a@example.com\r\n",
            "To: b@example.com\r\n",
            "Subject: Strip test\r\n",
            "MIME-Version: 1.0\r\n",
            "Content-Type: multipart/mixed; boundary=\"bnd1\"\r\n\r\n",
            "--bnd1\r\n",
            "Content-Type: text/plain; charset=utf-8\r\n\r\n",
            "Hello world\r\n",
            "--bnd1\r\n",
            "Content-Type: application/pdf; name=\"report.pdf\"\r\n",
            "Content-Disposition: attachment; filename=\"report.pdf\"\r\n",
            "Content-Transfer-Encoding: base64\r\n\r\n",
            "JVBERi0xLjQK\r\n",
            "--bnd1--\r\n",
        );
        let stripped =
            without_attachment(raw.as_bytes(), "report.pdf", 9, "2026-11-22T00:00:00Z").unwrap();
        let text = String::from_utf8_lossy(&stripped);
        assert!(text.contains("text/x-moz-deleted"));
        assert!(text.contains("Deleted: report.pdf"));
        assert!(!text.contains("JVBERi0xLjQK"));
    }

    #[test]
    fn without_attachment_rejects_signed_multipart() {
        let raw = concat!(
            "From: a@example.com\r\n",
            "To: b@example.com\r\n",
            "Subject: Signed\r\n",
            "MIME-Version: 1.0\r\n",
            "Content-Type: multipart/signed; boundary=\"sig1\"\r\n\r\n",
            "--sig1\r\n",
            "Content-Type: application/pdf; name=\"doc.pdf\"\r\n",
            "Content-Disposition: attachment; filename=\"doc.pdf\"\r\n\r\n",
            "data\r\n",
            "--sig1--\r\n",
        );
        let err =
            without_attachment(raw.as_bytes(), "doc.pdf", 4, "2026-11-22T00:00:00Z").unwrap_err();
        assert_eq!(err, StripError::Protected);
    }
}
