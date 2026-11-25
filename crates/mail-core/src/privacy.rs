//! Privacy helpers: HTML link tracking/redirector cleanup.

pub use crate::link_clean::{clean_link, strip_tracking, unwrap_redirect};

pub fn clean_tracking_urls_in_html(html: &str) -> String {
    let (sanitized, _) =
        crate::imap_client::sanitize_html_for_display_with_options(html, false, true);
    sanitized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_safelinks_in_html() {
        let html = r#"<a href="https://nam12.safelinks.protection.outlook.com/?url=https%3A%2F%2Fexample.com%2F%3Futm_source%3Dtest">Go</a>"#;
        let out = clean_tracking_urls_in_html(html);
        assert!(out.contains(r#"href="https://example.com/""#), "got {out}");
    }
}
