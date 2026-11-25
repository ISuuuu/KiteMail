//! Brand Logo / BIMI / Favicon / Gravatar avatar service (pure Rust, no GTK).

use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

const MAILBOX_DOMAINS: &[&str] = &[
    "gmail.com",
    "googlemail.com",
    "outlook.com",
    "hotmail.com",
    "live.com",
    "msn.com",
    "icloud.com",
    "me.com",
    "mac.com",
    "yahoo.com",
    "ymail.com",
    "proton.me",
    "protonmail.com",
    "pm.me",
    "fastmail.com",
    "qq.com",
    "163.com",
    "126.com",
    "yeah.net",
    "foxmail.com",
    "sina.com",
    "sohu.com",
    "aol.com",
    "gmx.com",
    "gmx.net",
    "web.de",
    "zoho.com",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrandMeta {
    pub name: &'static str,
    pub short: &'static str,
    pub color: &'static str,
}

const KNOWN_BRANDS: &[(&str, BrandMeta)] = &[
    (
        "github.com",
        BrandMeta {
            name: "GitHub",
            short: "GH",
            color: "#24292f",
        },
    ),
    (
        "apple.com",
        BrandMeta {
            name: "Apple",
            short: "AP",
            color: "#1d1d1f",
        },
    ),
    (
        "microsoft.com",
        BrandMeta {
            name: "Microsoft",
            short: "MS",
            color: "#0078d4",
        },
    ),
    (
        "google.com",
        BrandMeta {
            name: "Google",
            short: "GO",
            color: "#4285f4",
        },
    ),
    (
        "amazon.com",
        BrandMeta {
            name: "Amazon",
            short: "AZ",
            color: "#ff9900",
        },
    ),
    (
        "paypal.com",
        BrandMeta {
            name: "PayPal",
            short: "PP",
            color: "#003087",
        },
    ),
    (
        "stripe.com",
        BrandMeta {
            name: "Stripe",
            short: "ST",
            color: "#635bff",
        },
    ),
    (
        "linear.app",
        BrandMeta {
            name: "Linear",
            short: "LN",
            color: "#5e6ad2",
        },
    ),
    (
        "notion.so",
        BrandMeta {
            name: "Notion",
            short: "NO",
            color: "#2f3437",
        },
    ),
    (
        "cloudflare.com",
        BrandMeta {
            name: "Cloudflare",
            short: "CF",
            color: "#f38020",
        },
    ),
    (
        "vercel.com",
        BrandMeta {
            name: "Vercel",
            short: "VC",
            color: "#111111",
        },
    ),
    (
        "openai.com",
        BrandMeta {
            name: "OpenAI",
            short: "OA",
            color: "#10a37f",
        },
    ),
    (
        "gitlab.com",
        BrandMeta {
            name: "GitLab",
            short: "GL",
            color: "#fc6d26",
        },
    ),
];

pub fn is_mailbox_domain(domain: &str) -> bool {
    let d = domain.trim().trim_end_matches('.').to_ascii_lowercase();
    MAILBOX_DOMAINS.iter().any(|&m| m == d)
}

pub fn domain_of(email: &str) -> Option<String> {
    let host = email.rsplit('@').next()?.trim().trim_end_matches('.');
    let host = host.to_ascii_lowercase();
    if host.is_empty() || !host.contains('.') || host.parse::<std::net::IpAddr>().is_ok() {
        return None;
    }
    let labels: Vec<&str> = host.split('.').filter(|l| !l.is_empty()).collect();
    if labels.len() < 2 {
        return None;
    }
    let tail = &labels[labels.len().saturating_sub(2)..];
    let take = if labels.len() > 2 && tail[0].len() <= 3 && tail[1].len() == 2 {
        3
    } else {
        2
    };
    Some(labels[labels.len() - take.min(labels.len())..].join("."))
}

pub fn lookup_builtin_brand(email: &str) -> Option<&'static BrandMeta> {
    let domain = domain_of(email)?;
    if is_mailbox_domain(&domain) {
        return None;
    }
    KNOWN_BRANDS
        .iter()
        .find(|(d, _)| *d == domain)
        .map(|(_, meta)| meta)
}

/// SHA-256 hex digest of the trimmed, lowercased email (supported by Gravatar v3).
pub fn gravatar_hash(email: &str) -> String {
    let normalized = email.trim().to_ascii_lowercase();
    let digest = Sha256::digest(normalized.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn gravatar_url(email: &str, size: u32) -> String {
    let hash = gravatar_hash(email);
    format!("https://gravatar.com/avatar/{hash}?s={size}&d=404")
}

pub fn bimi_dns_name(domain: &str) -> String {
    format!("default._bimi.{}", domain.trim().trim_end_matches('.'))
}

pub fn favicon_candidates(domain: &str) -> Vec<String> {
    let d = domain.trim().trim_end_matches('.');
    vec![
        format!("https://{d}/apple-touch-icon.png"),
        format!("https://{d}/favicon.ico"),
    ]
}

/// Resolve a cached brand logo or Gravatar image file from `cache_dir` if present.
pub fn cached_avatar_path(cache_dir: &Path, email: &str) -> Option<PathBuf> {
    if let Some(domain) = domain_of(email) {
        if !is_mailbox_domain(&domain) {
            let bimi = cache_dir.join("logos").join(format!("{domain}.img"));
            if bimi.is_file() {
                return Some(bimi);
            }
        }
    }
    let hash = gravatar_hash(email);
    let grav = cache_dir.join("avatars").join(format!("{hash}.img"));
    if grav.is_file() {
        return Some(grav);
    }
    None
}

/// Fetch Gravatar image bytes (when `use_gravatar` is enabled) and cache to disk.
pub fn fetch_and_cache_gravatar(
    cache_dir: &Path,
    email: &str,
    size: u32,
) -> Result<Option<PathBuf>, String> {
    let avatars_dir = cache_dir.join("avatars");
    std::fs::create_dir_all(&avatars_dir).map_err(|e| e.to_string())?;
    let hash = gravatar_hash(email);
    let dest = avatars_dir.join(format!("{hash}.img"));
    let miss = avatars_dir.join(format!("{hash}.miss"));
    if dest.is_file() {
        return Ok(Some(dest));
    }
    if miss.is_file() {
        return Ok(None);
    }
    let url = gravatar_url(email, size);
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(5))
        .build();
    match agent.get(&url).call() {
        Ok(resp) if resp.status() == 200 => {
            use std::io::Read;
            let mut bytes = Vec::new();
            resp.into_reader()
                .take(512 * 1024)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            if !bytes.is_empty() {
                std::fs::write(&dest, &bytes).map_err(|e| e.to_string())?;
                return Ok(Some(dest));
            }
            let _ = std::fs::write(&miss, b"");
            Ok(None)
        }
        Ok(_) => {
            let _ = std::fs::write(&miss, b"");
            Ok(None)
        }
        Err(ureq::Error::Status(404, _)) => {
            let _ = std::fs::write(&miss, b"");
            Ok(None)
        }
        Err(e) => Err(format!("Gravatar 请求失败: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_brand_and_skips_mailbox_domains() {
        assert!(is_mailbox_domain("gmail.com"));
        assert!(!is_mailbox_domain("github.com"));
        assert!(lookup_builtin_brand("alice@gmail.com").is_none());
        let gh = lookup_builtin_brand("notifications@mail.github.com").unwrap();
        assert_eq!(gh.name, "GitHub");
        assert_eq!(gh.short, "GH");
    }

    #[test]
    fn builds_gravatar_and_bimi_names() {
        assert_eq!(bimi_dns_name("example.com"), "default._bimi.example.com");
        let url = gravatar_url("Alice@Example.com ", 96);
        assert!(url.starts_with("https://gravatar.com/avatar/"));
        assert!(url.ends_with("?s=96&d=404"));
    }
}
