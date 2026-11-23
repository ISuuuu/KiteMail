//! Remote HTTP image fetcher for `hylki-windows`'s GPUI `HttpClient`.

use std::future::Future;
use std::io::Read;
use std::pin::Pin;
use std::sync::OnceLock;
use std::time::Duration;

const MAX_BYTES: u64 = 50 * 1024 * 1024;

fn build_agent() -> ureq::Agent {
    let mut builder = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout(Duration::from_secs(30));

    let proxy_env = std::env::var("HTTPS_PROXY")
        .or_else(|_| std::env::var("https_proxy"))
        .or_else(|_| std::env::var("HTTP_PROXY"))
        .or_else(|_| std::env::var("http_proxy"));
    if let Ok(raw) = proxy_env {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            if let Ok(proxy) = ureq::Proxy::new(trimmed) {
                builder = builder.proxy(proxy);
            }
        }
    }

    builder.build()
}

fn shared_agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(build_agent)
}

fn is_http_url(url: &str) -> bool {
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

/// Fetch `url` over HTTP/HTTPS and return `(status, content_type, body_bytes)`.
pub fn fetch_remote_image(url: &str) -> Result<(u16, String, Vec<u8>), String> {
    if !is_http_url(url) {
        return Err(format!("unsupported remote image scheme: {url}"));
    }

    let response = match shared_agent()
        .get(url)
        .set("Accept", "image/*,*/*;q=0.8")
        .call()
    {
        Ok(resp) => resp,
        Err(ureq::Error::Status(code, resp)) => {
            let ct = resp.header("Content-Type").unwrap_or("").to_string();
            return Ok((code, ct, Vec::new()));
        }
        Err(ureq::Error::Transport(err)) => return Err(err.to_string()),
    };

    let status = response.status();
    let content_type = response.header("Content-Type").unwrap_or("").to_string();
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("remote image exceeds 50 MiB limit".to_string());
    }

    Ok((status, content_type, bytes))
}

/// Async wrapper around [`fetch_remote_image`] running on a background thread
/// so GPUI's foreground event loop is never blocked.
pub fn fetch_remote_image_async(
    url: String,
) -> Pin<Box<dyn Future<Output = Result<(u16, String, Vec<u8>), String>> + Send + 'static>> {
    Box::pin(async move {
        let (tx, rx) = futures::channel::oneshot::channel();
        std::thread::Builder::new()
            .name("hylki-remote-img".to_string())
            .spawn(move || {
                let _ = tx.send(fetch_remote_image(&url));
            })
            .map_err(|e| e.to_string())?;
        rx.await.map_err(|e| e.to_string())?
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_http_schemes() {
        assert!(fetch_remote_image("file:///C:/secret.png").is_err());
        assert!(fetch_remote_image("javascript:alert(1)").is_err());
        assert!(is_http_url("https://example.com/a.png"));
        assert!(is_http_url("HTTP://example.com/a.png"));
    }
}
