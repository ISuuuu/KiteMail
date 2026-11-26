//! Sender authentication verification (SPF / DKIM / DMARC) and lookalike /
//! display-name spoofing detection.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SenderTrust {
    Pass,
    #[default]
    Unverified,
    Suspicious,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SenderCheck {
    pub trust: SenderTrust,
    pub summary: String,
    pub findings: Vec<String>,
    pub from_domain: String,
    pub dkim_domain: Option<String>,
    pub authority: Option<String>,
    pub lookalike_warning: Option<String>,
}

#[derive(Debug, Default)]
struct Evidence {
    from_domain: String,
    from_name: String,
    reply_to_domain: Option<String>,
    return_path_domain: Option<String>,
}

/// Verify sender authenticity from a raw RFC 822 message.
pub fn check_sender(raw: &[u8]) -> SenderCheck {
    let mut ev = Evidence::default();
    let Some(parsed) = mail_parser::MessageParser::default().parse(raw) else {
        return judge(&ev);
    };

    let first_addr = |a: Option<&mail_parser::Address>| -> Option<String> {
        a.and_then(|a| a.first())
            .and_then(|x| x.address())
            .map(|s| s.to_ascii_lowercase())
    };
    if let Some(addr) = first_addr(parsed.from()) {
        ev.from_domain = domain_of(&addr).unwrap_or_default();
    }
    ev.from_name = parsed
        .from()
        .and_then(|a| a.first())
        .and_then(|x| x.name())
        .unwrap_or_default()
        .to_string();
    ev.reply_to_domain = first_addr(parsed.reply_to()).and_then(|a| domain_of(&a));
    ev.return_path_domain = parsed
        .header("Return-Path")
        .and_then(header_text)
        .and_then(|v| domain_of(v.trim().trim_start_matches('<').trim_end_matches('>')));

    // Message headers are sender-controlled; there is no configured trusted
    // receiving server whose authentication results can be distinguished.
    judge(&ev)
}

/// Lightweight check when only the `From` header string (and optional
/// `Authentication-Results` header) is available without full raw MIME.
pub fn check_sender_summary(from: &str, auth_results: Option<&str>) -> SenderCheck {
    let mut ev = Evidence::default();
    let (name, email) = split_display_and_email(from);
    ev.from_name = name;
    ev.from_domain = domain_of(&email).unwrap_or_default();
    let _ = auth_results; // No trusted receiving authority is configured.
    judge(&ev)
}

fn split_display_and_email(raw: &str) -> (String, String) {
    let trimmed = raw.trim();
    if let (Some(a), Some(b)) = (trimmed.find('<'), trimmed.rfind('>')) {
        if a < b {
            return (
                trimmed[..a].trim().trim_matches('"').to_string(),
                trimmed[a + 1..b].trim().to_ascii_lowercase(),
            );
        }
    }
    if trimmed.contains('@') {
        (String::new(), trimmed.to_ascii_lowercase())
    } else {
        (trimmed.to_string(), String::new())
    }
}

fn header_text(value: &mail_parser::HeaderValue) -> Option<String> {
    match value {
        mail_parser::HeaderValue::Text(t) => Some(t.to_string()),
        mail_parser::HeaderValue::TextList(list) => list.first().map(|t| t.to_string()),
        _ => None,
    }
}

fn judge(ev: &Evidence) -> SenderCheck {
    let from = if ev.from_domain.is_empty() {
        "(未知域名)"
    } else {
        &ev.from_domain
    };
    let mut trust = SenderTrust::Unverified;
    let mut summary = "无法确认认证结果来自可信的邮件服务商。".to_string();
    let mut findings = Vec::new();

    let mut lookalike_warning: Option<String> = None;

    if let Some(claimed) = domain_in_display_name(&ev.from_name) {
        if !ev.from_domain.is_empty() && !same_site(&claimed, &ev.from_domain) {
            let msg =
                format!("仿冒风险：发件人显示名称声称来自 \"{claimed}\"，但实际发件域名为 {from}");
            findings.push(msg.clone());
            lookalike_warning = Some(msg);
            if trust == SenderTrust::Pass || trust == SenderTrust::Unverified {
                trust = SenderTrust::Suspicious;
                summary = format!("{from} 与显示名称中的 {claimed} 不匹配，疑似仿冒。");
            }
        }
    }

    if ev.from_domain.contains("xn--") || is_homograph_suspect(&ev.from_domain) {
        let msg = format!("仿冒域名警告：发件域名 {from} 包含国际化编码或易混淆字符");
        findings.push(msg.clone());
        lookalike_warning.get_or_insert(msg);
        if trust == SenderTrust::Pass || trust == SenderTrust::Unverified {
            trust = SenderTrust::Suspicious;
        }
    }

    if let Some(reply) = &ev.reply_to_domain {
        if !ev.from_domain.is_empty() && !same_site(reply, &ev.from_domain) {
            findings.push(format!("回复地址域名 ({reply}) 与发件域名 ({from}) 不同"));
        }
    }
    if let Some(rp) = &ev.return_path_domain {
        if !ev.from_domain.is_empty() && !same_site(rp, &ev.from_domain) {
            findings.push(format!("退信地址域名: {rp}"));
        }
    }

    SenderCheck {
        trust,
        summary,
        findings,
        from_domain: ev.from_domain.clone(),
        dkim_domain: None,
        authority: None,
        lookalike_warning,
    }
}

fn is_homograph_suspect(domain: &str) -> bool {
    !domain.is_ascii()
}

pub fn domain_of(addr: &str) -> Option<String> {
    let (_, domain) = addr.rsplit_once('@')?;
    let domain = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    (!domain.is_empty()).then_some(domain)
}

fn same_site(a: &str, b: &str) -> bool {
    !a.is_empty() && !b.is_empty() && registrable(a) == registrable(b)
}

pub fn registrable(domain: &str) -> String {
    const TWO_PART: &[&str] = &[
        "co", "com", "net", "org", "gov", "edu", "ac", "or", "ne", "gouv",
    ];
    let labels: Vec<&str> = domain.split('.').filter(|l| !l.is_empty()).collect();
    let n = labels.len();
    if n < 3 {
        return labels.join(".");
    }
    let take = if labels[n - 1].len() <= 3 && TWO_PART.contains(&labels[n - 2]) {
        3
    } else {
        2
    };
    labels[n - take..].join(".")
}

fn domain_in_display_name(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    if let Some(at) = lower.find('@') {
        let rest = &lower[at + 1..];
        let end = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '-'))
            .unwrap_or(rest.len());
        let domain = rest[..end].trim_end_matches('.');
        if domain.contains('.') {
            return Some(domain.to_string());
        }
    }
    lower
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '-'))
        .find(|word| {
            let word = word.trim_end_matches('.');
            match word.rsplit_once('.') {
                Some((host, tld)) => {
                    !host.is_empty()
                        && (2..=24).contains(&tld.len())
                        && tld.chars().all(|c| c.is_ascii_alphabetic())
                }
                None => false,
            }
        })
        .map(|w| w.trim_end_matches('.').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untrusted_authentication_still_checks_reply_and_return_path() {
        let raw = concat!(
            "Authentication-Results: fake.example; dmarc=pass\r\n",
            "From: User <user@example.com>\r\n",
            "Reply-To: reply@other.example\r\n",
            "Return-Path: <bounce@third.example>\r\n\r\nbody"
        );
        let result = check_sender(raw.as_bytes());
        assert_eq!(result.trust, SenderTrust::Unverified);
        assert!(result.findings.iter().any(|s| s.contains("other.example")));
        assert!(result.findings.iter().any(|s| s.contains("third.example")));
    }

    #[test]
    fn dmarc_pass_and_spoof_detection() {
        let ok_raw = concat!(
            "Authentication-Results: mx.example.com; spf=pass; dkim=pass header.d=amazon.in; dmarc=pass\r\n",
            "From: Amazon <update@amazon.in>\r\n\r\nhi\r\n"
        );
        let res = check_sender(ok_raw.as_bytes());
        assert_eq!(res.trust, SenderTrust::Unverified);
        assert!(res.lookalike_warning.is_none());

        let spoof_raw = concat!(
            "Authentication-Results: mx.example.com; spf=pass; dkim=pass header.d=evil.com; dmarc=pass\r\n",
            "From: \"support@apple.com\" <phish@evil.com>\r\n\r\nhi\r\n"
        );
        let spoof = check_sender(spoof_raw.as_bytes());
        assert_eq!(spoof.trust, SenderTrust::Suspicious);
        assert!(spoof.lookalike_warning.is_some());
    }
}
