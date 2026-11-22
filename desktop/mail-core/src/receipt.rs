//! Read receipts (RFC 8098): Message Disposition Notifications (MDN).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiptRequest {
    pub to: String,
    #[serde(default)]
    pub matches_return_path: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub original_recipient: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub message_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadReport {
    pub message_id: String,
    pub by: String,
    pub at: i64,
}

fn unfold_header(value: &str) -> String {
    value
        .lines()
        .map(|l| l.trim())
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

fn raw_header(parsed: &mail_parser::Message, name: &str) -> Option<String> {
    let raw = parsed.raw_message();
    parsed
        .headers()
        .iter()
        .find(|h| h.name().eq_ignore_ascii_case(name))
        .and_then(|h| raw.get(h.offset_start()..h.offset_end()))
        .map(|b| unfold_header(&String::from_utf8_lossy(b)))
        .filter(|v| !v.is_empty())
}

fn addresses(value: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for item in value.split(',') {
        let addr = match (item.find('<'), item.rfind('>')) {
            (Some(a), Some(b)) if a < b => item[a + 1..b].trim().to_string(),
            _ => item
                .split_whitespace()
                .find(|w| w.contains('@'))
                .unwrap_or_default()
                .trim_matches(|c| c == '"' || c == '(' || c == ')')
                .to_string(),
        };
        if addr.contains('@') && !out.iter().any(|o| o.eq_ignore_ascii_case(&addr)) {
            out.push(addr);
        }
    }
    out
}

pub fn detect(parsed: &mail_parser::Message) -> Option<ReceiptRequest> {
    let to = addresses(&raw_header(parsed, "Disposition-Notification-To")?);
    if to.is_empty() {
        return None;
    }
    let return_path = raw_header(parsed, "Return-Path")
        .map(|v| addresses(&v))
        .unwrap_or_default();
    let matches_return_path = to.len() == 1
        && return_path
            .first()
            .is_some_and(|rp| rp.eq_ignore_ascii_case(&to[0]));
    Some(ReceiptRequest {
        to: to.join(", "),
        matches_return_path,
        original_recipient: raw_header(parsed, "Original-Recipient").unwrap_or_default(),
        message_id: parsed.message_id().unwrap_or_default().to_string(),
    })
}

pub fn detect_raw(raw: &[u8]) -> Option<ReceiptRequest> {
    let parsed = mail_parser::MessageParser::default().parse(raw)?;
    detect(&parsed)
}

pub fn read_report(parsed: &mail_parser::Message) -> Option<ReadReport> {
    let part = parsed.parts.iter().find(|p| is_report_part(p))?;
    let fields = report_lines(&String::from_utf8_lossy(part.contents()));
    let field = |name: &str| {
        fields.iter().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.trim()
                .eq_ignore_ascii_case(name)
                .then(|| value.trim().to_string())
        })
    };
    let shown = field("Disposition")
        .and_then(|d| d.split_once(';').map(|(_, kind)| kind.to_string()))
        .is_some_and(|kind| {
            kind.split('/')
                .next()
                .is_some_and(|k| k.trim().eq_ignore_ascii_case("displayed"))
        });
    if !shown {
        return None;
    }
    let message_id = field("Original-Message-ID")
        .or_else(|| raw_header(parsed, "In-Reply-To"))
        .map(|v| {
            v.trim()
                .trim_start_matches('<')
                .trim_end_matches('>')
                .trim()
                .to_string()
        })
        .filter(|v| !v.is_empty())?;
    let by = field("Final-Recipient")
        .map(|v| match v.split_once(';') {
            Some((_, addr)) => addr.trim().to_string(),
            None => v,
        })
        .or_else(|| {
            parsed
                .from()
                .and_then(|a| a.first())
                .and_then(|a| a.address())
                .map(str::to_string)
        })
        .unwrap_or_default();
    Some(ReadReport {
        message_id,
        by,
        at: parsed.date().map_or(0, |d| d.to_timestamp()),
    })
}

fn report_lines(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        match out.last_mut() {
            Some(last) if line.starts_with([' ', '\t']) => {
                last.push(' ');
                last.push_str(line.trim());
            }
            _ if !line.trim().is_empty() => out.push(line.to_string()),
            _ => {}
        }
    }
    out
}

fn address_field(addr: &str) -> String {
    let kind = if addr.is_ascii() { "rfc822" } else { "utf-8" };
    format!("{kind};{addr}")
}

pub fn report_fields(
    recipient: &str,
    request: &ReceiptRequest,
    message_id: &str,
    automatic: bool,
) -> String {
    let mut out = String::from("Reporting-UA: Hylki\r\n");
    if !request.original_recipient.is_empty() {
        out.push_str(&format!(
            "Original-Recipient: {}\r\n",
            request.original_recipient
        ));
    }
    out.push_str(&format!(
        "Final-Recipient: {}\r\n",
        address_field(recipient)
    ));
    let clean_mid = message_id
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>');
    if !clean_mid.is_empty() {
        out.push_str(&format!("Original-Message-ID: <{clean_mid}>\r\n"));
    }
    let mode = if automatic {
        "MDN-sent-automatically"
    } else {
        "MDN-sent-manually"
    };
    out.push_str(&format!("Disposition: manual-action/{mode}; displayed\r\n"));
    out
}

pub fn build_mdn_raw(
    from_email: &str,
    subject: &str,
    request: &ReceiptRequest,
    automatic: bool,
) -> String {
    let boundary = "----=_Hylki_MDN_Boundary_001";
    let fields = report_fields(from_email, request, &request.message_id, automatic);
    let human = format!(
        "This is a return receipt for the mail that you sent to {from_email}.\r\n\r\nNote: This return receipt only acknowledges that the message was displayed on the recipient's computer."
    );
    format!(
        "From: <{from_email}>\r\n\
         To: {}\r\n\
         Subject: Read: {subject}\r\n\
         MIME-Version: 1.0\r\n\
         Content-Type: multipart/report; report-type=disposition-notification; boundary=\"{boundary}\"\r\n\
         \r\n\
         --{boundary}\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\
         \r\n\
         {human}\r\n\
         --{boundary}\r\n\
         Content-Type: message/disposition-notification\r\n\
         \r\n\
         {fields}\r\n\
         --{boundary}--\r\n",
        request.to
    )
}

pub fn is_report_part(part: &mail_parser::MessagePart) -> bool {
    use mail_parser::MimeHeaders;
    part.content_type().is_some_and(|c| {
        c.ctype().eq_ignore_ascii_case("message")
            && c.subtype()
                .is_some_and(|s| s.eq_ignore_ascii_case("disposition-notification"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_parser::MessageParser;

    #[test]
    fn detects_request_and_builds_mdn() {
        let raw = concat!(
            "Return-Path: <ann@example.org>\r\n",
            "Disposition-Notification-To: Ann <ann@example.org>\r\n",
            "Message-ID: <msg-1@example.org>\r\n",
            "From: Ann <ann@example.org>\r\n",
            "Subject: Quarterly Report\r\n\r\nBody\r\n"
        );
        let parsed = MessageParser::default().parse(raw.as_bytes()).unwrap();
        let req = detect(&parsed).unwrap();
        assert_eq!(req.to, "ann@example.org");
        assert!(req.matches_return_path);
        assert_eq!(req.message_id, "msg-1@example.org");

        let mdn = build_mdn_raw("bob@example.com", "Quarterly Report", &req, false);
        assert!(mdn.contains("message/disposition-notification"));
        assert!(mdn.contains("Original-Message-ID: <msg-1@example.org>"));
        assert!(mdn.contains("MDN-sent-manually; displayed"));
    }
}
