//! 系统代理探测与 TLS 配置：更新检查与下载按实时代理配置路由并信任系统根证书。
//!
//! 顺序：环境变量（`NO_PROXY` 绕过、`HTTPS_PROXY` / `HTTP_PROXY` / `ALL_PROXY`）
//! → Windows WinHTTP（PAC 自动配置脚本、手动代理与 `ProxyOverride`、WPAD 自动检测）。
//! 每次构造请求都实时读取，不静态缓存，确保用户开关或切换代理端口后立即生效。

use std::net::IpAddr;
use std::sync::Arc;

const HTTPS_ENV_KEYS: [&str; 6] = [
    "HTTPS_PROXY",
    "https_proxy",
    "ALL_PROXY",
    "all_proxy",
    "HTTP_PROXY",
    "http_proxy",
];

const HTTP_ENV_KEYS: [&str; 4] = ["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"];

/// 已解析且可直接挂载到 `ureq` 的代理配置；`display` 仅含脱敏后的 `scheme://host:port`。
pub(crate) struct ResolvedProxy {
    pub proxy: ureq::Proxy,
    pub display: String,
}

/// 实时探测目标 URL 应走的代理（环境变量优先，其次系统代理）。
pub(crate) fn resolve(target_url: &str) -> Option<ResolvedProxy> {
    resolve_with(target_url, |key| std::env::var(key).ok(), platform)
}

/// 构造更新请求用的 `AgentBuilder`，同时返回脱敏代理地址（直连时为 `None`）。
pub(crate) fn agent_builder(target_url: &str) -> (ureq::AgentBuilder, Option<String>) {
    build_agent(resolve(target_url))
}

fn build_agent(resolved: Option<ResolvedProxy>) -> (ureq::AgentBuilder, Option<String>) {
    let mut builder = ureq::AgentBuilder::new();
    // 使用系统原生 TLS（Windows 上走 SChannel），自动信任系统根证书与受信任的企业/代理根证书。
    if let Ok(tls) = native_tls::TlsConnector::new() {
        builder = builder.tls_connector(Arc::new(tls));
    }
    match resolved {
        Some(resolved) => (builder.proxy(resolved.proxy), Some(resolved.display)),
        None => (builder, None),
    }
}

fn resolve_with(
    target_url: &str,
    env: impl Fn(&str) -> Option<String>,
    platform_fn: impl Fn(&str) -> Option<ResolvedProxy>,
) -> Option<ResolvedProxy> {
    let (target_scheme, target_host, target_port) = parse_target(target_url);

    let no_proxy = ["NO_PROXY", "no_proxy"]
        .into_iter()
        .filter_map(&env)
        .map(|value| value.trim().to_string())
        .find(|value| !value.is_empty());
    if let (Some(list), Some(host)) = (no_proxy.as_deref(), target_host.as_deref()) {
        if matches_bypass(list, ',', host, target_port, false) {
            return None;
        }
    }

    let keys: &[&str] = if target_scheme == "https" {
        &HTTPS_ENV_KEYS
    } else {
        &HTTP_ENV_KEYS
    };

    if let Some(proxy) = keys
        .iter()
        .filter_map(|key| env(key))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .find_map(|value| parse(&value))
    {
        return Some(proxy);
    }

    platform_fn(target_url)
}

fn parse_target(target_url: &str) -> (String, Option<String>, Option<u16>) {
    let parsed = url::Url::parse(target_url).ok();
    let scheme = parsed
        .as_ref()
        .map(|url| url.scheme().to_ascii_lowercase())
        .filter(|scheme| !scheme.is_empty())
        .unwrap_or_else(|| "https".to_string());
    let host = parsed
        .as_ref()
        .and_then(|url| url.host_str().map(str::to_string));
    let port = parsed.as_ref().and_then(|url| url.port_or_known_default());
    (scheme, host, port)
}

fn parse(value: &str) -> Option<ResolvedProxy> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    let candidate = strip_pac_directive(trimmed)?;
    let (normalized_scheme, was_https, url_str) = match candidate.split_once("://") {
        Some((raw_scheme, rest)) => {
            let lower = raw_scheme.to_ascii_lowercase();
            let (scheme, is_https) = match lower.as_str() {
                "http" => ("http", false),
                "https" => ("http", true),
                "socks" | "socks5" | "socks5h" => ("socks5", false),
                "socks4" => ("socks4", false),
                "socks4a" => ("socks4a", false),
                _ => return None,
            };
            (scheme, is_https, format!("{scheme}://{rest}"))
        }
        None => ("http", false, format!("http://{candidate}")),
    };

    let parsed = url::Url::parse(&url_str).ok()?;
    if !matches!(parsed.path(), "" | "/") || parsed.query().is_some() || parsed.fragment().is_some()
    {
        return None;
    }

    let host_str = match parsed.host()? {
        url::Host::Domain(domain) if !domain.is_empty() => domain.to_string(),
        url::Host::Ipv4(ip) => ip.to_string(),
        url::Host::Ipv6(_) => return None,
        _ => return None,
    };

    let username = parsed.username();
    let password = parsed.password();
    let has_creds = !username.is_empty() || password.is_some();

    if was_https {
        let is_loopback = host_str == "127.0.0.1" || host_str.eq_ignore_ascii_case("localhost");
        // 仅对无凭据的本地回环代理宽容回落为 HTTP；远程 HTTPS 代理或带凭据配置绝不降级明文。
        if !is_loopback || has_creds {
            return None;
        }
    }

    if !username.is_empty() && password.is_none() {
        return None;
    }
    if username.is_empty() && password == Some("") {
        return None;
    }

    let default_port = if normalized_scheme == "http" {
        80
    } else {
        1080
    };
    let port = match parsed.port() {
        Some(0) => return None,
        Some(port) => port,
        None => default_port,
    };

    let display = format!("{normalized_scheme}://{host_str}:{port}");
    let ureq_spec = match password {
        Some(pass) => format!("{normalized_scheme}://{username}:{pass}@{host_str}:{port}"),
        None => display.clone(),
    };
    let proxy = ureq::Proxy::new(&ureq_spec).ok()?;
    Some(ResolvedProxy { proxy, display })
}

fn strip_pac_directive(value: &str) -> Option<String> {
    let mut parts = value.split_whitespace();
    let first = parts.next()?;
    let Some(second) = parts.next() else {
        if first.eq_ignore_ascii_case("DIRECT") {
            return None;
        }
        return Some(value.to_string());
    };
    if parts.next().is_some() {
        return None;
    }
    let prefix = if first.eq_ignore_ascii_case("DIRECT") {
        return None;
    } else if first.eq_ignore_ascii_case("PROXY") || first.eq_ignore_ascii_case("HTTP") {
        "http://"
    } else if first.eq_ignore_ascii_case("SOCKS")
        || first.eq_ignore_ascii_case("SOCKS5")
        || first.eq_ignore_ascii_case("SOCKS5H")
    {
        "socks5://"
    } else if first.eq_ignore_ascii_case("SOCKS4") {
        "socks4://"
    } else if first.eq_ignore_ascii_case("SOCKS4A") {
        "socks4a://"
    } else {
        return None;
    };

    if second.contains("://") {
        Some(second.to_string())
    } else {
        Some(format!("{prefix}{second}"))
    }
}

/// 从分号分隔的代理列表（如 `http=host:port;https=host:port` 或 PAC 返回串）中按目标协议挑选可用代理，
/// 前一条目无效时自动回退到后续候选。
fn select_proxy(server_list: &str, target_scheme: &str) -> Option<ResolvedProxy> {
    let mut https_bucket = Vec::new();
    let mut http_bucket = Vec::new();
    let mut plain_bucket = Vec::new();
    let mut socks_bucket = Vec::new();

    for entry in server_list
        .split(';')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
    {
        match entry.split_once('=') {
            Some((proto, addr)) => {
                let proto = proto.trim();
                let addr = addr.trim();
                if addr.is_empty() {
                    continue;
                }
                if proto.eq_ignore_ascii_case("https") {
                    https_bucket.push(addr.to_string());
                } else if proto.eq_ignore_ascii_case("http") {
                    http_bucket.push(addr.to_string());
                } else if proto.eq_ignore_ascii_case("socks")
                    || proto.eq_ignore_ascii_case("socks5")
                    || proto.eq_ignore_ascii_case("socks5h")
                {
                    socks_bucket.push(with_default_scheme(addr, "socks5"));
                } else if proto.eq_ignore_ascii_case("socks4") {
                    socks_bucket.push(with_default_scheme(addr, "socks4"));
                } else if proto.eq_ignore_ascii_case("socks4a") {
                    socks_bucket.push(with_default_scheme(addr, "socks4a"));
                }
            }
            None => plain_bucket.push(entry.to_string()),
        }
    }

    let buckets = if target_scheme.eq_ignore_ascii_case("https") {
        [https_bucket, plain_bucket, http_bucket, socks_bucket]
    } else {
        [http_bucket, plain_bucket, https_bucket, socks_bucket]
    };

    buckets
        .into_iter()
        .flatten()
        .find_map(|candidate| parse(&candidate))
}

fn with_default_scheme(addr: &str, scheme: &str) -> String {
    if addr.contains("://") || addr.split_whitespace().nth(1).is_some() {
        addr.to_string()
    } else {
        format!("{scheme}://{addr}")
    }
}

fn matches_bypass(
    list: &str,
    sep: char,
    host: &str,
    port: Option<u16>,
    allow_local_token: bool,
) -> bool {
    let normalized_host = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if normalized_host.is_empty() {
        return false;
    }

    for raw in list
        .split(sep)
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
    {
        if raw == "*" {
            return true;
        }
        if allow_local_token && raw.eq_ignore_ascii_case("<local>") {
            if !normalized_host.contains('.') {
                return true;
            }
            continue;
        }

        let (host_pat, pat_port) = split_bypass_host_port(raw);
        if let Some(expected_port) = pat_port {
            if port != Some(expected_port) {
                continue;
            }
        }

        let host_pat = host_pat.trim_end_matches('.').to_ascii_lowercase();
        if host_pat.is_empty() {
            continue;
        }
        if host_pat == "*" {
            return true;
        }
        if host_pat.contains('*') {
            if wildcard_match(&host_pat, &normalized_host) {
                return true;
            }
            continue;
        }
        if let Some(suffix) = host_pat.strip_prefix('.') {
            if !suffix.is_empty()
                && (normalized_host == suffix || normalized_host.ends_with(&host_pat))
            {
                return true;
            }
            continue;
        }
        if host_pat.parse::<IpAddr>().is_ok() {
            if normalized_host == host_pat {
                return true;
            }
            continue;
        }
        if normalized_host == host_pat || normalized_host.ends_with(&format!(".{host_pat}")) {
            return true;
        }
    }

    false
}

fn split_bypass_host_port(raw: &str) -> (&str, Option<u16>) {
    if let Some(rest) = raw.strip_prefix('[') {
        if let Some((host, after)) = rest.split_once(']') {
            if let Some(port_str) = after.strip_prefix(':') {
                if let Ok(port) = port_str.parse::<u16>() {
                    return (host, Some(port));
                }
            }
            return (host, None);
        }
    }
    if let Some((left, right)) = raw.rsplit_once(':') {
        if !left.contains(':') {
            if let Ok(port) = right.parse::<u16>() {
                return (left, Some(port));
            }
        }
    }
    (raw, None)
}

fn wildcard_match(pattern: &str, text: &str) -> bool {
    let p = pattern.as_bytes();
    let t = text.as_bytes();
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star_pi, mut match_ti) = (None, 0usize);

    while ti < t.len() {
        if pi < p.len() && p[pi] == t[ti] {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star_pi = Some(pi);
            match_ti = ti;
            pi += 1;
        } else if let Some(sp) = star_pi {
            pi = sp + 1;
            match_ti += 1;
            ti = match_ti;
        } else {
            return false;
        }
    }

    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }
    pi == p.len()
}

#[cfg(windows)]
fn platform(target_url: &str) -> Option<ResolvedProxy> {
    winhttp::resolve(target_url)
}

#[cfg(not(windows))]
fn platform(_target_url: &str) -> Option<ResolvedProxy> {
    None
}

#[cfg(windows)]
mod winhttp {
    use windows_sys::Win32::Foundation::{GlobalFree, FALSE, TRUE};
    use windows_sys::Win32::Networking::WinHttp::{
        WinHttpCloseHandle, WinHttpGetIEProxyConfigForCurrentUser, WinHttpGetProxyForUrl,
        WinHttpOpen, WinHttpSetTimeouts, WINHTTP_ACCESS_TYPE_NAMED_PROXY,
        WINHTTP_ACCESS_TYPE_NO_PROXY, WINHTTP_AUTOPROXY_AUTO_DETECT, WINHTTP_AUTOPROXY_CONFIG_URL,
        WINHTTP_AUTOPROXY_OPTIONS, WINHTTP_AUTO_DETECT_TYPE_DHCP, WINHTTP_AUTO_DETECT_TYPE_DNS_A,
        WINHTTP_CURRENT_USER_IE_PROXY_CONFIG, WINHTTP_PROXY_INFO,
    };

    use super::{matches_bypass, parse_target, select_proxy, ResolvedProxy};

    enum AutoProxyOutcome {
        Direct,
        Proxy(ResolvedProxy),
        Unavailable,
    }

    struct WinHttpSession(*mut core::ffi::c_void);

    impl WinHttpSession {
        fn open() -> Option<Self> {
            let agent = wide("Hylki");
            let handle = unsafe {
                WinHttpOpen(
                    agent.as_ptr(),
                    WINHTTP_ACCESS_TYPE_NO_PROXY,
                    std::ptr::null(),
                    std::ptr::null(),
                    0,
                )
            };
            (!handle.is_null()).then_some(Self(handle))
        }
    }

    impl Drop for WinHttpSession {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    WinHttpCloseHandle(self.0);
                }
            }
        }
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// 读取 WinHTTP 分配的 UTF-16 缓冲区并无条件调用 `GlobalFree` 释放。
    unsafe fn take_global_wide(ptr: windows_sys::core::PWSTR) -> Option<String> {
        if ptr.is_null() {
            return None;
        }
        let mut len = 0usize;
        while *ptr.add(len) != 0 {
            len += 1;
        }
        let value = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len));
        GlobalFree(ptr.cast());
        let trimmed = value.trim().to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    }

    pub(super) fn resolve(target_url: &str) -> Option<ResolvedProxy> {
        let (target_scheme, target_host, target_port) = parse_target(target_url);

        let mut ie_config = WINHTTP_CURRENT_USER_IE_PROXY_CONFIG {
            fAutoDetect: FALSE,
            lpszAutoConfigUrl: std::ptr::null_mut(),
            lpszProxy: std::ptr::null_mut(),
            lpszProxyBypass: std::ptr::null_mut(),
        };
        if unsafe { WinHttpGetIEProxyConfigForCurrentUser(&mut ie_config) } == FALSE {
            return None;
        }
        let auto_config_url = unsafe { take_global_wide(ie_config.lpszAutoConfigUrl) };
        let manual_proxy = unsafe { take_global_wide(ie_config.lpszProxy) };
        let manual_bypass = unsafe { take_global_wide(ie_config.lpszProxyBypass) };

        if let Some(pac_url) = auto_config_url.as_deref() {
            match resolve_auto_proxy(
                target_url,
                &target_scheme,
                target_host.as_deref(),
                target_port,
                5000,
                Some(pac_url),
            ) {
                AutoProxyOutcome::Direct => return None,
                AutoProxyOutcome::Proxy(resolved) => return Some(resolved),
                AutoProxyOutcome::Unavailable => {}
            }
        }

        if let Some(proxy_list) = manual_proxy.as_deref() {
            if let (Some(bypass), Some(host)) = (manual_bypass.as_deref(), target_host.as_deref()) {
                if matches_bypass(bypass, ';', host, target_port, true) {
                    return None;
                }
            }
            return select_proxy(proxy_list, &target_scheme);
        }

        if ie_config.fAutoDetect != FALSE {
            if let AutoProxyOutcome::Proxy(resolved) = resolve_auto_proxy(
                target_url,
                &target_scheme,
                target_host.as_deref(),
                target_port,
                2000,
                None,
            ) {
                return Some(resolved);
            }
        }

        None
    }

    fn resolve_auto_proxy(
        target_url: &str,
        target_scheme: &str,
        target_host: Option<&str>,
        target_port: Option<u16>,
        timeout_ms: i32,
        pac_url: Option<&str>,
    ) -> AutoProxyOutcome {
        let Some(session) = WinHttpSession::open() else {
            return AutoProxyOutcome::Unavailable;
        };
        unsafe {
            WinHttpSetTimeouts(session.0, timeout_ms, timeout_ms, timeout_ms, timeout_ms);
        }

        let pac_wide = pac_url.map(wide);
        let mut options = match pac_wide.as_ref() {
            Some(url_w) => WINHTTP_AUTOPROXY_OPTIONS {
                dwFlags: WINHTTP_AUTOPROXY_CONFIG_URL,
                dwAutoDetectFlags: 0,
                lpszAutoConfigUrl: url_w.as_ptr(),
                lpvReserved: std::ptr::null_mut(),
                dwReserved: 0,
                fAutoLogonIfChallenged: TRUE,
            },
            None => WINHTTP_AUTOPROXY_OPTIONS {
                dwFlags: WINHTTP_AUTOPROXY_AUTO_DETECT,
                dwAutoDetectFlags: WINHTTP_AUTO_DETECT_TYPE_DHCP | WINHTTP_AUTO_DETECT_TYPE_DNS_A,
                lpszAutoConfigUrl: std::ptr::null(),
                lpvReserved: std::ptr::null_mut(),
                dwReserved: 0,
                fAutoLogonIfChallenged: TRUE,
            },
        };

        let target_wide = wide(target_url);
        let mut proxy_info = WINHTTP_PROXY_INFO {
            dwAccessType: 0,
            lpszProxy: std::ptr::null_mut(),
            lpszProxyBypass: std::ptr::null_mut(),
        };

        let ok = unsafe {
            WinHttpGetProxyForUrl(
                session.0,
                target_wide.as_ptr(),
                &mut options,
                &mut proxy_info,
            )
        };
        if ok == FALSE {
            return AutoProxyOutcome::Unavailable;
        }

        let proxy_list = unsafe { take_global_wide(proxy_info.lpszProxy) };
        let bypass = unsafe { take_global_wide(proxy_info.lpszProxyBypass) };

        match proxy_info.dwAccessType {
            WINHTTP_ACCESS_TYPE_NO_PROXY => AutoProxyOutcome::Direct,
            WINHTTP_ACCESS_TYPE_NAMED_PROXY => {
                if let (Some(bypass), Some(host)) = (bypass.as_deref(), target_host) {
                    if matches_bypass(bypass, ';', host, target_port, true) {
                        return AutoProxyOutcome::Direct;
                    }
                }
                let Some(list) = proxy_list.as_deref() else {
                    return AutoProxyOutcome::Unavailable;
                };
                match select_proxy(list, target_scheme) {
                    Some(resolved) => AutoProxyOutcome::Proxy(resolved),
                    None => AutoProxyOutcome::Unavailable,
                }
            }
            _ => AutoProxyOutcome::Unavailable,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use super::*;

    #[test]
    fn selects_proxies_by_protocol_with_fallback() {
        assert_eq!(
            select_proxy("http=127.0.0.1:7890;https=127.0.0.1:7890", "https")
                .map(|p| p.display)
                .as_deref(),
            Some("http://127.0.0.1:7890")
        );
        assert_eq!(
            select_proxy("socks=127.0.0.1:7891;http=10.0.0.2:8080", "https")
                .map(|p| p.display)
                .as_deref(),
            Some("http://10.0.0.2:8080")
        );
        assert_eq!(
            select_proxy("http=10.0.0.2:8080;https=10.0.0.3:8443", "https")
                .map(|p| p.display)
                .as_deref(),
            Some("http://10.0.0.3:8443")
        );
        assert_eq!(
            select_proxy("https=;http=127.0.0.1:7897", "https")
                .map(|p| p.display)
                .as_deref(),
            Some("http://127.0.0.1:7897")
        );
        assert_eq!(
            select_proxy("https=https://remote:443;http=127.0.0.1:7897", "https")
                .map(|p| p.display)
                .as_deref(),
            Some("http://127.0.0.1:7897")
        );
        assert_eq!(
            select_proxy("socks=127.0.0.1:7891", "https")
                .map(|p| p.display)
                .as_deref(),
            Some("socks5://127.0.0.1:7891")
        );
        assert_eq!(
            select_proxy("PROXY 127.0.0.1:7897; DIRECT", "https")
                .map(|p| p.display)
                .as_deref(),
            Some("http://127.0.0.1:7897")
        );
        assert_eq!(
            select_proxy("SOCKS5 127.0.0.1:7898", "https")
                .map(|p| p.display)
                .as_deref(),
            Some("socks5://127.0.0.1:7898")
        );
        assert!(select_proxy("DIRECT", "https").is_none());
    }

    #[test]
    fn parses_valid_proxies_and_rejects_malformed_or_unsafe_ones() {
        assert!(parse("").is_none());
        assert!(parse("   ").is_none());
        assert_eq!(
            parse("HTTP://127.0.0.1:7897").map(|p| p.display).as_deref(),
            Some("http://127.0.0.1:7897")
        );
        assert_eq!(
            parse("socks5h://127.0.0.1:7898")
                .map(|p| p.display)
                .as_deref(),
            Some("socks5://127.0.0.1:7898")
        );
        assert_eq!(
            parse("socks4://127.0.0.1:1080")
                .map(|p| p.display)
                .as_deref(),
            Some("socks4://127.0.0.1:1080")
        );
        assert_eq!(
            parse("https://127.0.0.1:7897")
                .map(|p| p.display)
                .as_deref(),
            Some("http://127.0.0.1:7897")
        );
        assert!(parse("https://proxy.example.com:443").is_none());
        assert!(parse("https://user:secret@127.0.0.1:7897").is_none());
        assert!(parse("[::1]:7897").is_none());
        assert!(parse("http://127.0.0.1:7897/foo").is_none());

        let authed = parse("http://user:secret@127.0.0.1:7897").unwrap();
        assert_eq!(authed.display, "http://127.0.0.1:7897");
        assert!(!authed.display.contains("user"));
        assert!(!authed.display.contains("secret"));
    }

    #[test]
    fn matches_no_proxy_and_windows_bypass_rules() {
        assert!(matches_bypass("*", ',', "github.com", Some(443), false));
        assert!(matches_bypass(
            ".github.com",
            ',',
            "api.github.com",
            Some(443),
            false
        ));
        assert!(matches_bypass(
            "github.com",
            ',',
            "github.com",
            Some(443),
            false
        ));
        assert!(matches_bypass(
            "github.com",
            ',',
            "objects.github.com",
            Some(443),
            false
        ));
        assert!(!matches_bypass(
            "github.com",
            ',',
            "notgithub.com",
            Some(443),
            false
        ));
        assert!(matches_bypass(
            "github.com:443",
            ',',
            "github.com",
            Some(443),
            false
        ));
        assert!(!matches_bypass(
            "github.com:8443",
            ',',
            "github.com",
            Some(443),
            false
        ));
        assert!(matches_bypass(
            "*.example.com",
            ',',
            "cdn.example.com",
            Some(443),
            false
        ));

        let win_override = "localhost;127.*;*.corp.local;<local>";
        assert!(matches_bypass(
            win_override,
            ';',
            "127.0.0.1",
            Some(80),
            true
        ));
        assert!(matches_bypass(
            win_override,
            ';',
            "git.corp.local",
            Some(443),
            true
        ));
        assert!(matches_bypass(
            win_override,
            ';',
            "intranet",
            Some(80),
            true
        ));
        assert!(!matches_bypass(
            win_override,
            ';',
            "github.com",
            Some(443),
            true
        ));
    }

    #[test]
    fn resolve_with_honours_env_no_proxy_and_live_changes() {
        let env_map = RefCell::new(HashMap::<String, String>::new());
        let platform_proxy = RefCell::new(Some("127.0.0.1:7897".to_string()));

        let lookup_env = |key: &str| env_map.borrow().get(key).cloned();
        let lookup_platform = |url: &str| {
            let (scheme, _, _) = parse_target(url);
            platform_proxy
                .borrow()
                .as_deref()
                .and_then(|s| select_proxy(s, &scheme))
        };

        // 1. 仅配置平台代理时使用平台代理；平台代理切换/关闭后立刻生效，无静态缓存。
        assert_eq!(
            resolve_with("https://github.com/a", lookup_env, lookup_platform)
                .map(|p| p.display)
                .as_deref(),
            Some("http://127.0.0.1:7897")
        );
        *platform_proxy.borrow_mut() = None;
        assert!(resolve_with("https://github.com/a", lookup_env, lookup_platform).is_none());

        // 2. 环境变量优先于平台代理。
        *platform_proxy.borrow_mut() = Some("127.0.0.1:7897".to_string());
        env_map
            .borrow_mut()
            .insert("HTTPS_PROXY".into(), "socks5://127.0.0.1:1080".into());
        assert_eq!(
            resolve_with("https://github.com/a", lookup_env, lookup_platform)
                .map(|p| p.display)
                .as_deref(),
            Some("socks5://127.0.0.1:1080")
        );

        // 3. NO_PROXY 命中时绕过代理直接连。
        env_map
            .borrow_mut()
            .insert("NO_PROXY".into(), ".github.com".into());
        assert!(resolve_with("https://api.github.com/a", lookup_env, lookup_platform).is_none());
    }

    #[test]
    fn http_proxy_agent_sends_connect_handshake() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let n = stream.read(&mut buf).unwrap();
            let req = String::from_utf8_lossy(&buf[..n]).into_owned();
            let _ = stream.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n");
            req
        });

        let (builder, label) = build_agent(parse(&format!("http://{addr}")));
        assert_eq!(label.as_deref(), Some(format!("http://{addr}")).as_deref());
        let agent = builder.build();
        let err = agent
            .get("https://example.com/latest.json")
            .call()
            .unwrap_err();
        assert!(matches!(err, ureq::Error::Transport(_)));

        let captured = server.join().unwrap();
        assert!(captured.starts_with("CONNECT example.com:443 HTTP/1.1\r\n"));
    }
}
