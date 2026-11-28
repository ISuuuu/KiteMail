//! Microsoft OAuth2 (PKCE) sign-in via system browser + local loopback listener.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

use base64::Engine;
use sha2::{Digest, Sha256};

/// KiteMail's public Entra ID client registration (PKCE, no secret required).
pub const DEFAULT_MICROSOFT_CLIENT_ID: &str = "01cdc012-c8d8-4d03-822c-76696a01c14e";
const MICROSOFT_AUTH_URL: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/authorize";
const MICROSOFT_TOKEN_URL: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/token";
const MICROSOFT_SCOPES: &str = "https://graph.microsoft.com/Mail.ReadWrite \
                                https://graph.microsoft.com/Mail.Send \
                                https://graph.microsoft.com/User.Read \
                                offline_access";
/// Authorize scope requesting both Microsoft Graph and Exchange Online IMAP IDLE permissions.
const MICROSOFT_AUTH_SCOPES: &str = "https://graph.microsoft.com/Mail.ReadWrite \
                                     https://graph.microsoft.com/Mail.Send \
                                     https://graph.microsoft.com/User.Read \
                                     https://outlook.office.com/IMAP.AccessAsUser.All \
                                     offline_access";
/// Token refresh scope for Exchange Online IMAP (`outlook.office365.com:993` XOAUTH2 + IDLE).
const MICROSOFT_IMAP_SCOPES: &str =
    "https://outlook.office.com/IMAP.AccessAsUser.All offline_access";

fn oauth_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .redirects(0)
        .timeout(Duration::from_secs(20))
        .build()
}

fn require_oauth_success(response: ureq::Response) -> Result<ureq::Response, String> {
    let status = response.status();
    if !(200..300).contains(&status) {
        return Err(format!("OAuth 服务返回非成功状态 (HTTP {status})"));
    }
    Ok(response)
}

const SUCCESS_HTML: &str = r##"<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<title>KiteMail — 登录成功</title>
<style>
  body { font-family: "Segoe UI", system-ui, sans-serif; display: grid; place-items: center;
         height: 100vh; margin: 0; background: #f4f7fc; color: #182338; }
  .card { background: #fff; padding: 36px 40px; border-radius: 16px; text-align: center;
          box-shadow: 0 12px 36px rgba(24,38,64,.12); max-width: 420px; }
  h1 { font-size: 22px; margin: 0 0 10px; }
  p { color: #546482; font-size: 14px; line-height: 1.6; margin: 0; }
</style>
</head>
<body>
  <div class="card">
    <h1>Outlook 授权成功</h1>
    <p>你的账号已连接到 KiteMail，可以关闭此浏览器标签页返回客户端。</p>
  </div>
</body>
</html>"##;

/// Result of an interactive Microsoft OAuth2 sign-in.
#[derive(Clone, Debug)]
pub struct OAuthSignInResult {
    pub client_id: String,
    pub email: String,
    pub display_name: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in_secs: u64,
}

/// Explicit outcome for interactive OAuth2 login (distinguishes success, user cancellation, and failure).
#[derive(Clone, Debug)]
pub enum OAuthLoginOutcome {
    Success(OAuthSignInResult),
    Cancelled(String),
    Failed(String),
}

/// Result of refreshing an access token.
#[derive(Clone, Debug)]
pub struct RefreshedTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in_secs: u64,
}

fn random_base64url(num_bytes: usize) -> Result<String, String> {
    let mut bytes = vec![0u8; num_bytes];
    getrandom::getrandom(&mut bytes).map_err(|e| format!("生成随机数失败: {e}"))?;
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&bytes))
}

fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

fn open_system_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", url])
            .spawn()
            .map_err(|e| format!("打开系统浏览器失败: {e}"))?;
        Ok(())
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = url;
        Err("当前非 Windows 环境".to_string())
    }
}

/// Run interactive Microsoft OAuth2 PKCE login in the user's system browser.
/// `login_hint`（可选）为表单中已填写的邮箱，非空时预填微软登录页的账号框。
pub fn run_microsoft_oauth_login(
    custom_client_id: &str,
    login_hint: Option<&str>,
) -> Result<OAuthSignInResult, String> {
    match run_microsoft_oauth_login_cancellable(custom_client_id, login_hint, None) {
        OAuthLoginOutcome::Success(res) => Ok(res),
        OAuthLoginOutcome::Cancelled(msg) | OAuthLoginOutcome::Failed(msg) => Err(msg),
    }
}

/// Run interactive Microsoft OAuth2 PKCE login with cancellation support and explicit status.
pub fn run_microsoft_oauth_login_cancellable(
    custom_client_id: &str,
    login_hint: Option<&str>,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> OAuthLoginOutcome {
    use std::sync::atomic::Ordering;

    if cancel.is_some_and(|c| c.load(Ordering::SeqCst)) {
        return OAuthLoginOutcome::Cancelled("已取消 Outlook 授权".to_string());
    }

    let client_id = if custom_client_id.trim().is_empty() {
        DEFAULT_MICROSOFT_CLIENT_ID.to_string()
    } else {
        custom_client_id.trim().to_string()
    };

    let verifier = match random_base64url(32) {
        Ok(v) => v,
        Err(e) => return OAuthLoginOutcome::Failed(e),
    };
    let challenge = pkce_challenge(&verifier);
    let state = match random_base64url(16) {
        Ok(s) => s,
        Err(e) => return OAuthLoginOutcome::Failed(e),
    };

    let listener = match TcpListener::bind("127.0.0.1:0") {
        Ok(l) => l,
        Err(e) => return OAuthLoginOutcome::Failed(format!("启动本地 OAuth 回调端口失败: {e}")),
    };
    let port = match listener.local_addr() {
        Ok(addr) => addr.port(),
        Err(e) => return OAuthLoginOutcome::Failed(format!("读取回调端口失败: {e}")),
    };
    let redirect_uri = format!("http://localhost:{port}/");

    let mut auth_url = match url::Url::parse(MICROSOFT_AUTH_URL) {
        Ok(u) => u,
        Err(e) => return OAuthLoginOutcome::Failed(format!("构造授权地址失败: {e}")),
    };
    {
        let mut pairs = auth_url.query_pairs_mut();
        pairs
            .append_pair("client_id", &client_id)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("response_mode", "query")
            .append_pair("scope", MICROSOFT_AUTH_SCOPES)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &state)
            .append_pair("prompt", "select_account");
        if let Some(hint) = login_hint.map(str::trim).filter(|s| !s.is_empty()) {
            pairs.append_pair("login_hint", hint);
        }
    }

    if let Err(e) = open_system_browser(auth_url.as_str()) {
        return OAuthLoginOutcome::Failed(e);
    }

    if let Err(e) = listener.set_nonblocking(true) {
        return OAuthLoginOutcome::Failed(format!("设置回调监听器失败: {e}"));
    }

    let deadline = Instant::now() + Duration::from_secs(180);
    let code = loop {
        if cancel.is_some_and(|c| c.load(Ordering::SeqCst)) {
            return OAuthLoginOutcome::Cancelled("已取消 Outlook 授权".to_string());
        }
        if Instant::now() > deadline {
            return OAuthLoginOutcome::Failed(
                "等待浏览器 OAuth2 授权超时（180 秒），请重试".to_string(),
            );
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                let mut buf = [0u8; 8192];
                let n = stream.read(&mut buf).unwrap_or(0);
                let req_str = String::from_utf8_lossy(&buf[..n]);
                let first_line = req_str.lines().next().unwrap_or("");
                let target = first_line.split_whitespace().nth(1).unwrap_or("/");
                if !target.starts_with('/') {
                    let _ =
                        stream.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n");
                    return OAuthLoginOutcome::Failed("无效的 OAuth 回调请求路径".to_string());
                }
                if target.starts_with("/favicon.ico") {
                    let _ =
                        stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
                    continue;
                }

                let full_url = format!("http://localhost{target}");
                let parsed = match url::Url::parse(&full_url) {
                    Ok(p) => p,
                    Err(e) => {
                        return OAuthLoginOutcome::Failed(format!("解析回调参数失败: {e}"));
                    }
                };

                let mut got_code = None;
                let mut got_state = None;
                let mut got_err = None;
                let mut got_err_desc = None;
                for (k, v) in parsed.query_pairs() {
                    match k.as_ref() {
                        "code" => got_code = Some(v.into_owned()),
                        "state" => got_state = Some(v.into_owned()),
                        "error" => got_err = Some(v.into_owned()),
                        "error_description" => got_err_desc = Some(v.into_owned()),
                        _ => {}
                    }
                }

                if let Some(err) = got_err {
                    let body = format!("授权失败: {err}");
                    let resp = format!(
                        "HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(resp.as_bytes());
                    if err == "access_denied" || err == "user_cancelled" {
                        return OAuthLoginOutcome::Cancelled(format!(
                            "用户取消了 Outlook 授权: {}",
                            got_err_desc.unwrap_or(err)
                        ));
                    }
                    return OAuthLoginOutcome::Failed(format!(
                        "Outlook 授权被拒绝或失败: {}",
                        got_err_desc.unwrap_or(err)
                    ));
                }

                if got_state.as_deref() != Some(state.as_str()) {
                    let _ =
                        stream.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n");
                    return OAuthLoginOutcome::Failed("OAuth2 state 校验失败，请重试".to_string());
                }

                if let Some(c) = got_code {
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        SUCCESS_HTML.len(),
                        SUCCESS_HTML
                    );
                    let _ = stream.write_all(resp.as_bytes());
                    break c;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(120));
            }
            Err(e) => return OAuthLoginOutcome::Failed(format!("接收 OAuth 回调失败: {e}")),
        }
    };

    if cancel.is_some_and(|c| c.load(Ordering::SeqCst)) {
        return OAuthLoginOutcome::Cancelled("已取消 Outlook 授权".to_string());
    }

    let token_resp = match oauth_agent()
        .post(MICROSOFT_TOKEN_URL)
        .send_form(&[
            ("client_id", client_id.as_str()),
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("code_verifier", verifier.as_str()),
            ("scope", MICROSOFT_SCOPES),
        ])
        .map_err(format_ureq_error)
        .and_then(require_oauth_success)
    {
        Ok(resp) => resp,
        Err(e) => return OAuthLoginOutcome::Failed(e),
    };

    let json: serde_json::Value = match token_resp.into_json() {
        Ok(v) => v,
        Err(e) => return OAuthLoginOutcome::Failed(format!("解析 Token 响应失败: {e}")),
    };

    let Some(access_token) = json["access_token"].as_str().map(str::to_string) else {
        return OAuthLoginOutcome::Failed("微软未返回 access_token".to_string());
    };
    let Some(refresh_token) = json["refresh_token"].as_str().map(str::to_string) else {
        return OAuthLoginOutcome::Failed(
            "微软未返回 refresh_token（请确认包含 offline_access 权限）".to_string(),
        );
    };
    let expires_in_secs = json["expires_in"].as_u64().unwrap_or(3600);

    let (email, display_name) = match fetch_me(&access_token) {
        Ok(info) => info,
        Err(e) => return OAuthLoginOutcome::Failed(e),
    };

    OAuthLoginOutcome::Success(OAuthSignInResult {
        client_id,
        email,
        display_name,
        access_token,
        refresh_token,
        expires_in_secs,
    })
}

/// Exchange a saved `refresh_token` for a fresh Microsoft Graph access token.
pub fn refresh_microsoft_token(
    custom_client_id: &str,
    refresh_token: &str,
) -> Result<RefreshedTokens, String> {
    refresh_microsoft_token_with_scope(custom_client_id, refresh_token, MICROSOFT_SCOPES)
}

/// Exchange a saved `refresh_token` for a fresh Exchange Online IMAP access token (`XOAUTH2` + `IDLE`).
pub fn refresh_microsoft_imap_token(
    custom_client_id: &str,
    refresh_token: &str,
) -> Result<RefreshedTokens, String> {
    refresh_microsoft_token_with_scope(custom_client_id, refresh_token, MICROSOFT_IMAP_SCOPES)
}

fn refresh_microsoft_token_with_scope(
    custom_client_id: &str,
    refresh_token: &str,
    scope: &str,
) -> Result<RefreshedTokens, String> {
    let client_id = if custom_client_id.trim().is_empty() {
        DEFAULT_MICROSOFT_CLIENT_ID
    } else {
        custom_client_id.trim()
    };

    let resp = oauth_agent()
        .post(MICROSOFT_TOKEN_URL)
        .send_form(&[
            ("client_id", client_id),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("scope", scope),
        ])
        .map_err(format_ureq_error)
        .and_then(require_oauth_success)?;

    let json: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("解析刷新令牌响应失败: {e}"))?;

    let access_token = json["access_token"]
        .as_str()
        .ok_or_else(|| "刷新令牌未返回 access_token".to_string())?
        .to_string();
    let new_refresh = json["refresh_token"].as_str().map(|s| s.to_string());
    let expires_in_secs = json["expires_in"].as_u64().unwrap_or(3600);

    Ok(RefreshedTokens {
        access_token,
        refresh_token: new_refresh,
        expires_in_secs,
    })
}

fn fetch_me(access_token: &str) -> Result<(String, String), String> {
    let resp = oauth_agent()
        .get("https://graph.microsoft.com/v1.0/me?$select=displayName,mail,userPrincipalName")
        .set("Authorization", &format!("Bearer {access_token}"))
        .call()
        .map_err(format_ureq_error)
        .and_then(require_oauth_success)?;

    let json: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("解析 Microsoft 个人信息失败: {e}"))?;

    let email = json["mail"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| json["userPrincipalName"].as_str())
        .unwrap_or("outlook-user@outlook.com")
        .to_string();
    let display_name = json["displayName"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&email)
        .to_string();
    Ok((email, display_name))
}

fn format_ureq_error(err: ureq::Error) -> String {
    match err {
        ureq::Error::Status(code, _) => format!("OAuth HTTP 错误 (HTTP {code})"),
        ureq::Error::Transport(t) => format!("网络请求失败: {:?}", t.kind()),
    }
}

pub const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
pub const GOOGLE_MAIL_SCOPES: &str = "https://mail.google.com/ email profile";

/// Generic OAuth2 PKCE provider configuration (supports Google & Custom OAuth2).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OAuthProviderConfig {
    pub auth_url: String,
    pub token_url: String,
    pub client_id: String,
    #[serde(default)]
    pub client_secret: Option<String>,
    pub scopes: String,
    #[serde(default)]
    pub userinfo_url: Option<String>,
}

impl OAuthProviderConfig {
    pub fn google(client_id: impl Into<String>, client_secret: Option<String>) -> Self {
        Self {
            auth_url: GOOGLE_AUTH_URL.to_string(),
            token_url: GOOGLE_TOKEN_URL.to_string(),
            client_id: client_id.into(),
            client_secret,
            scopes: GOOGLE_MAIL_SCOPES.to_string(),
            userinfo_url: Some("https://www.googleapis.com/oauth2/v3/userinfo".to_string()),
        }
    }
}

/// Build the authorization URL for a PKCE flow (shared by Google & Custom OAuth2).
pub fn build_pkce_auth_url(
    cfg: &OAuthProviderConfig,
    redirect_uri: &str,
    challenge: &str,
    state: &str,
    login_hint: Option<&str>,
) -> Result<String, String> {
    let mut url =
        url::Url::parse(&cfg.auth_url).map_err(|e| format!("无效的 OAuth2 auth_url: {e}"))?;
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("client_id", cfg.client_id.trim());
        q.append_pair("response_type", "code");
        q.append_pair("redirect_uri", redirect_uri);
        q.append_pair("scope", cfg.scopes.trim());
        q.append_pair("code_challenge", challenge);
        q.append_pair("code_challenge_method", "S256");
        q.append_pair("state", state);
        q.append_pair("access_type", "offline");
        if let Some(hint) = login_hint.map(str::trim).filter(|s| !s.is_empty()) {
            q.append_pair("login_hint", hint);
        }
    }
    Ok(url.into())
}

/// Refresh an OAuth2 access token for Google or any custom OAuth2 provider.
pub fn refresh_custom_token(
    cfg: &OAuthProviderConfig,
    refresh_token: &str,
) -> Result<RefreshedTokens, String> {
    let mut form: Vec<(&str, &str)> = vec![
        ("client_id", cfg.client_id.trim()),
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
    ];
    if let Some(secret) = cfg
        .client_secret
        .as_deref()
        .filter(|s| !s.trim().is_empty())
    {
        form.push(("client_secret", secret.trim()));
    }
    let resp = oauth_agent()
        .post(&cfg.token_url)
        .send_form(&form)
        .map_err(format_ureq_error)
        .and_then(require_oauth_success)?;
    let json: serde_json::Value = resp
        .into_json()
        .map_err(|e| format!("解析 OAuth2 刷新响应失败: {e}"))?;
    let access_token = json["access_token"]
        .as_str()
        .ok_or_else(|| "OAuth2 刷新响应缺少 access_token".to_string())?
        .to_string();
    let new_refresh = json["refresh_token"].as_str().map(str::to_string);
    let expires_in_secs = json["expires_in"].as_u64().unwrap_or(3600);
    Ok(RefreshedTokens {
        access_token,
        refresh_token: new_refresh,
        expires_in_secs,
    })
}

/// Refresh a Google OAuth2 token.
pub fn refresh_google_token(
    client_id: &str,
    client_secret: Option<&str>,
    refresh_token: &str,
) -> Result<RefreshedTokens, String> {
    let cfg = OAuthProviderConfig::google(client_id, client_secret.map(str::to_string));
    refresh_custom_token(&cfg, refresh_token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_google_and_custom_pkce_urls() {
        let cfg = OAuthProviderConfig::google("google-client-123", None);
        let url = build_pkce_auth_url(
            &cfg,
            "http://127.0.0.1:54321/",
            "challenge_abc",
            "state_xyz",
            Some("user@gmail.com"),
        )
        .unwrap();
        assert!(url.starts_with(GOOGLE_AUTH_URL));
        assert!(url.contains("client_id=google-client-123"));
        assert!(url.contains("code_challenge=challenge_abc"));
        assert!(url.contains("login_hint=user%40gmail.com"));
    }

    #[test]
    fn oauth_error_redacts_body_and_rejects_redirects() {
        let resp = ureq::Response::new(400, "Bad Request", "secret token leak").unwrap();
        let formatted = format_ureq_error(ureq::Error::Status(400, resp));
        assert_eq!(formatted, "OAuth HTTP 错误 (HTTP 400)");
        assert!(!formatted.contains("secret"));

        assert!(require_oauth_success(ureq::Response::new(302, "Found", "").unwrap()).is_err());
        assert!(require_oauth_success(ureq::Response::new(200, "OK", "").unwrap()).is_ok());
    }

    #[test]
    fn cancellable_oauth_returns_cancelled_immediately_when_signalled() {
        let cancel = std::sync::atomic::AtomicBool::new(true);
        let outcome = run_microsoft_oauth_login_cancellable("", None, Some(&cancel));
        assert!(matches!(outcome, OAuthLoginOutcome::Cancelled(_)));
    }
}
