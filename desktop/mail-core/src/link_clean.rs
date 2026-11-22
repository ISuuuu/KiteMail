//! Link trackers and redirectors: unwrap up to 4 layers of marketing/security
//! redirectors (Outlook SafeLinks, Proofpoint, Mimecast, SendGrid, Google,
//! Facebook, YouTube) and strip tracking query parameters (`utm_*`, `fbclid`,
//! `gclid`, `mc_eid`, etc.).

const TRACKING: &[&str] = &[
    "utm_*",
    "fbclid",
    "gclid",
    "gclsrc",
    "dclid",
    "gbraid",
    "wbraid",
    "msclkid",
    "yclid",
    "twclid",
    "ttclid",
    "igshid",
    "li_fat_id",
    "mc_cid",
    "mc_eid",
    "_hsenc",
    "_hsmi",
    "__hssc",
    "__hstc",
    "__hsfp",
    "hsctatracking",
    "mkt_tok",
    "oly_anon_id",
    "oly_enc_id",
    "vero_id",
    "vero_conv",
    "_openstat",
    "ck_subscriber_id",
    "ml_subscriber",
    "ml_subscriber_hash",
    "rb_clickid",
    "s_cid",
    "wickedid",
    "_kx",
    "sc_cid",
    "spm",
    "ref_src",
];

pub const MAX_UNWRAP: usize = 4;

pub fn clean_link(uri: &str) -> String {
    let mut current = uri.trim().to_string();
    for _ in 0..MAX_UNWRAP {
        match unwrap_redirect(&current) {
            Some(target) if target != current => current = target,
            _ => break,
        }
    }
    strip_tracking(&current)
}

struct Parts<'a> {
    host: String,
    path: &'a str,
    head: &'a str,
    query: Option<&'a str>,
    fragment: &'a str,
}

fn parts(uri: &str) -> Option<Parts<'_>> {
    let lower = uri.get(..8)?.to_ascii_lowercase();
    let rest_at = if lower.starts_with("https://") {
        8
    } else if lower.starts_with("http://") {
        7
    } else {
        return None;
    };
    let (before_fragment, fragment) = match uri.find('#') {
        Some(i) => uri.split_at(i),
        None => (uri, ""),
    };
    let (head, query) = match before_fragment.split_once('?') {
        Some((h, q)) => (h, Some(q)),
        None => (before_fragment, None),
    };
    let after_scheme = head.get(rest_at..)?;
    let (authority, path) = match after_scheme.find('/') {
        Some(i) => after_scheme.split_at(i),
        None => (after_scheme, ""),
    };
    let host = authority.rsplit('@').next().unwrap_or("");
    let host = if host.starts_with('[') {
        host.split(']')
            .next()
            .map(|h| format!("{h}]"))
            .unwrap_or_default()
    } else {
        host.split(':').next().unwrap_or("").to_string()
    };
    Some(Parts {
        host: host.trim_end_matches('.').to_ascii_lowercase(),
        path,
        head,
        query,
        fragment,
    })
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (
                (bytes[i + 1] as char).to_digit(16),
                (bytes[i + 2] as char).to_digit(16),
            ) {
                out.push(((h << 4) | l) as u8);
                i += 3;
                continue;
            }
        } else if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn query_param<'a>(query: &'a str, names: &[&str]) -> Option<&'a str> {
    query.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        names
            .iter()
            .any(|want| name.eq_ignore_ascii_case(want))
            .then_some(value)
    })
}

fn validate_web_target(raw: &str) -> Option<String> {
    let target = raw.trim();
    let lower = target.to_ascii_lowercase();
    let is_web = lower.starts_with("https://") || lower.starts_with("http://");
    if !is_web || target.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    parts(target)?;
    Some(target.to_string())
}

pub fn unwrap_redirect(uri: &str) -> Option<String> {
    let p = parts(uri)?;
    let path_trimmed = p.path.trim_end_matches('/');

    if (p.host == "urldefense.com" || p.host.ends_with(".urldefense.com"))
        && p.path.starts_with("/v3/__")
    {
        if let Some(idx) = uri.find("/v3/__") {
            let after = &uri[idx + 6..];
            if let Some(end) = after.find("__") {
                let candidate = &after[..end];
                if let Some(valid) = validate_web_target(candidate) {
                    return Some(valid);
                }
                let decoded = percent_decode(candidate);
                if let Some(valid) = validate_web_target(&decoded) {
                    return Some(valid);
                }
            }
        }
    }

    if (p.host == "urldefense.proofpoint.com" || p.host.ends_with(".proofpoint.com"))
        && path_trimmed.eq_ignore_ascii_case("/v2/url")
    {
        let raw = query_param(p.query?, &["u"])?;
        let replaced: String = raw
            .chars()
            .map(|c| match c {
                '-' => '%',
                '_' => '/',
                _ => c,
            })
            .collect();
        let decoded = percent_decode(&replaced);
        if let Some(valid) = validate_web_target(&decoded) {
            return Some(valid);
        }
    }

    let param_names: &[&str] = if p.host.ends_with(".safelinks.protection.outlook.com")
        || p.host == "safelinks.protection.outlook.com"
    {
        &["url"]
    } else if p.host.ends_with(".mimecast.com") || p.host == "mimecast.com" {
        &["url", "u", "domain"]
    } else if p.host.ends_with(".sendgrid.net") || p.host == "sendgrid.net" {
        &["url", "u", "target"]
    } else {
        let bare = p.host.strip_prefix("www.").unwrap_or(&p.host);
        if is_google(bare) && path_trimmed.eq_ignore_ascii_case("/url") {
            &["q", "url"]
        } else if matches!(
            p.host.as_str(),
            "l.facebook.com" | "lm.facebook.com" | "l.messenger.com"
        ) && path_trimmed.eq_ignore_ascii_case("/l.php")
        {
            &["u"]
        } else if matches!(bare, "youtube.com" | "m.youtube.com")
            && path_trimmed.eq_ignore_ascii_case("/redirect")
        {
            &["q"]
        } else {
            return None;
        }
    };

    let raw = query_param(p.query?, param_names)?;
    let decoded = percent_decode(raw);
    validate_web_target(&decoded)
}

fn is_google(host: &str) -> bool {
    let Some(tld) = host.strip_prefix("google.") else {
        return false;
    };
    let country = |s: &str| s.len() == 2 && s.bytes().all(|b| b.is_ascii_lowercase());
    tld == "com"
        || country(tld)
        || tld
            .split_once('.')
            .is_some_and(|(second, cc)| matches!(second, "co" | "com") && country(cc))
}

fn is_tracking(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    TRACKING.iter().any(|t| match t.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => name == *t,
    })
}

pub fn strip_tracking(uri: &str) -> String {
    let Some(p) = parts(uri) else {
        return uri.to_string();
    };
    let Some(query) = p.query else {
        return uri.to_string();
    };
    let kept: Vec<&str> = query
        .split('&')
        .filter(|pair| {
            let name = pair.split_once('=').map_or(*pair, |(n, _)| n);
            !is_tracking(name)
        })
        .collect();
    if kept.len() == query.split('&').count() {
        return uri.to_string();
    }
    let mut out = p.head.to_string();
    if kept.iter().any(|k| !k.is_empty()) {
        out.push('?');
        out.push_str(&kept.join("&"));
    }
    out.push_str(p.fragment);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unwraps_safelinks_proofpoint_mimecast_sendgrid_and_strips_trackers() {
        let safelink = "https://nam12.safelinks.protection.outlook.com/?url=https%3A%2F%2Fexample.org%2Fdoc%3Futm_source%3Dmail%26id%3D42&data=05";
        assert_eq!(clean_link(safelink), "https://example.org/doc?id=42");

        let proofpoint_v2 = "https://urldefense.proofpoint.com/v2/url?u=https-3A__example.com_invoice-3Futm-5Fmedium-3Demail&d=DwMFaQ";
        assert_eq!(clean_link(proofpoint_v2), "https://example.com/invoice");

        let proofpoint_v3 =
            "https://urldefense.com/v3/__https://example.net/login?fbclid=123&ok=1__;!!AbCd!";
        assert_eq!(clean_link(proofpoint_v3), "https://example.net/login?ok=1");

        let mimecast =
            "https://protect-us.mimecast.com/s/abc?url=https%3A%2F%2Fexample.org%2Fgo%3Fgclid%3D99";
        assert_eq!(clean_link(mimecast), "https://example.org/go");

        let sendgrid = "https://u1234.ct.sendgrid.net/ls/click?url=https%3A%2F%2Fexample.org%2Fhome%3Fmc_eid%3D1";
        assert_eq!(clean_link(sendgrid), "https://example.org/home");
    }

    #[test]
    fn unwraps_up_to_four_layers() {
        let l1 = "https://example.org/final?utm_source=x&keep=1";
        let l2 = format!(
            "https://www.google.com/url?q={}",
            url::form_urlencoded::byte_serialize(l1.as_bytes()).collect::<String>()
        );
        let l3 = format!(
            "https://l.facebook.com/l.php?u={}",
            url::form_urlencoded::byte_serialize(l2.as_bytes()).collect::<String>()
        );
        let l4 = format!(
            "https://nam01.safelinks.protection.outlook.com/?url={}",
            url::form_urlencoded::byte_serialize(l3.as_bytes()).collect::<String>()
        );
        let l5 = format!(
            "https://protect-eu.mimecast.com/s/1?url={}",
            url::form_urlencoded::byte_serialize(l4.as_bytes()).collect::<String>()
        );
        assert_eq!(clean_link(&l5), "https://example.org/final?keep=1");
    }

    #[test]
    fn preserves_non_http_and_legit_params() {
        assert_eq!(
            clean_link("mailto:alice@example.com"),
            "mailto:alice@example.com"
        );
        assert_eq!(
            clean_link("https://example.org/?utmost=1&gclidx=2#sec"),
            "https://example.org/?utmost=1&gclidx=2#sec"
        );
    }
}
