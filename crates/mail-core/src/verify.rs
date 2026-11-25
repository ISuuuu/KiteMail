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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthResult {
    Pass,
    Fail,
    Other,
    Missing,
}

impl AuthResult {
    fn parse(value: &str) -> AuthResult {
        match value {
            "pass" => AuthResult::Pass,
            "fail" | "softfail" | "permerror" => AuthResult::Fail,
            _ => AuthResult::Other,
        }
    }
}

#[derive(Debug, Default)]
struct Evidence {
    spf: Option<AuthResult>,
    dkim: Option<AuthResult>,
    dmarc: Option<AuthResult>,
    dkim_domain: Option<String>,
    from_domain: String,
    from_name: String,
    reply_to_domain: Option<String>,
    return_path_domain: Option<String>,
    checked: bool,
    authority: Option<String>,
}

impl Evidence {
    fn get(&self, which: Method) -> AuthResult {
        match which {
            Method::Spf => self.spf,
            Method::Dkim => self.dkim,
            Method::Dmarc => self.dmarc,
        }
        .unwrap_or(AuthResult::Missing)
    }
}

#[derive(Clone, Copy)]
enum Method {
    Spf,
    Dkim,
    Dmarc,
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

    for header in parsed
        .headers()
        .iter()
        .filter(|h| h.name().eq_ignore_ascii_case("Authentication-Results"))
    {
        if let Some(line) = header_text(&header.value) {
            let who = authserv_id(&line);
            let top = ev.authority.get_or_insert_with(|| who.clone());
            if registrable(top) != registrable(&who) {
                continue;
            }
            ev.checked = true;
            parse_auth_results(&line, &mut ev);
        }
    }
    if ev.spf.is_none() {
        if let Some(line) = parsed.header("Received-SPF").and_then(header_text) {
            let word = line.split_whitespace().next().unwrap_or("").to_string();
            ev.spf = Some(AuthResult::parse(&word.to_ascii_lowercase()));
            ev.checked = true;
        }
    }
    judge(&ev)
}

/// Lightweight check when only the `From` header string (and optional
/// `Authentication-Results` header) is available without full raw MIME.
pub fn check_sender_summary(from: &str, auth_results: Option<&str>) -> SenderCheck {
    let mut ev = Evidence::default();
    let (name, email) = split_display_and_email(from);
    ev.from_name = name;
    ev.from_domain = domain_of(&email).unwrap_or_default();
    if let Some(line) = auth_results {
        if !line.trim().is_empty() {
            ev.authority = Some(authserv_id(line));
            ev.checked = true;
            parse_auth_results(line, &mut ev);
        }
    }
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

fn parse_auth_results(line: &str, ev: &mut Evidence) {
    let lower = line.to_ascii_lowercase();
    let mut this_spf = None;
    let mut this_dkim = None;
    let mut this_dmarc = None;
    let mut this_domain = None;
    for token in lower.split([';', ' ', '\t', '\r', '\n']) {
        let Some((key, value)) = token.split_once('=') else {
            continue;
        };
        let value =
            value.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '.' && c != '-');
        match key {
            "spf" => this_spf = Some(AuthResult::parse(value)),
            "dkim" => this_dkim = Some(AuthResult::parse(value)),
            "dmarc" => this_dmarc = Some(AuthResult::parse(value)),
            "header.d" if !value.is_empty() => this_domain = Some(value.to_string()),
            _ => {}
        }
    }
    if ev.spf.is_none() {
        ev.spf = this_spf;
    }
    if ev.dkim.is_none() {
        if let Some(r) = this_dkim {
            ev.dkim = Some(r);
            ev.dkim_domain = this_domain;
        }
    }
    if ev.dmarc.is_none() {
        ev.dmarc = this_dmarc;
    }
}

fn authserv_id(line: &str) -> String {
    let first = line.split(';').next().unwrap_or("").trim();
    if first.contains('=') {
        return String::new();
    }
    first
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn failed_and_nothing_passed(dkim: AuthResult, spf: AuthResult) -> bool {
    let any_failed = dkim == AuthResult::Fail || spf == AuthResult::Fail;
    let any_passed = dkim == AuthResult::Pass || spf == AuthResult::Pass;
    any_failed && !any_passed
}

fn judge(ev: &Evidence) -> SenderCheck {
    let from = if ev.from_domain.is_empty() {
        "(未知域名)"
    } else {
        &ev.from_domain
    };
    let dmarc = ev.get(Method::Dmarc);
    let dkim = ev.get(Method::Dkim);
    let spf = ev.get(Method::Spf);
    let dkim_aligned = dkim == AuthResult::Pass
        && ev
            .dkim_domain
            .as_deref()
            .is_some_and(|d| same_site(d, &ev.from_domain));

    let (mut trust, mut summary) = match (dmarc, dkim, spf) {
        (AuthResult::Pass, _, _) => (
            SenderTrust::Pass,
            format!("邮件服务商已验证该邮件确实来自 {from}（DMARC 通过）。"),
        ),
        (AuthResult::Fail, _, _) => (
            SenderTrust::Fail,
            format!("该邮件未通过 {from} 的真实性校验（DMARC 失败），请警惕伪造发件人。"),
        ),
        _ if dkim_aligned => (
            SenderTrust::Pass,
            format!("该邮件携带来自 {from} 的有效 DKIM 签名。"),
        ),
        _ if failed_and_nothing_passed(dkim, spf) => (
            SenderTrust::Suspicious,
            format!("无法确认该邮件来自 {from}，且已执行的发件人校验失败。"),
        ),
        _ if !ev.checked => (
            SenderTrust::Unverified,
            "邮件服务商未提供 SPF/DKIM/DMARC 校验头。".to_string(),
        ),
        _ => (
            SenderTrust::Unverified,
            format!("现有校验信息不足以完全确认发件域名 {from}。"),
        ),
    };

    let mut findings = Vec::new();
    for (label, res) in [("DMARC", dmarc), ("DKIM", dkim), ("SPF", spf)] {
        match res {
            AuthResult::Pass => findings.push(format!("{label}: 通过")),
            AuthResult::Fail => findings.push(format!("{label}: 失败")),
            AuthResult::Other => findings.push(format!("{label}: 未决")),
            AuthResult::Missing => {}
        }
    }
    if let Some(d) = &ev.dkim_domain {
        findings.push(format!("DKIM 签名域: {d}"));
    }

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
        dkim_domain: ev.dkim_domain.clone(),
        authority: ev.authority.clone(),
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
    fn dmarc_pass_and_spoof_detection() {
        let ok_raw = concat!(
            "Authentication-Results: mx.example.com; spf=pass; dkim=pass header.d=amazon.in; dmarc=pass\r\n",
            "From: Amazon <update@amazon.in>\r\n\r\nhi\r\n"
        );
        let res = check_sender(ok_raw.as_bytes());
        assert_eq!(res.trust, SenderTrust::Pass);
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
