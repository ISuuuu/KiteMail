//! Navigation, resource, external-link, and trusted script-bridge security policies.

/// Synthetic origin used for in-memory reader documents.
pub const DOCUMENT_URL: &str = "https://kitemail.invalid/reader";

/// Return the versioned synthetic URL for a loaded document revision.
pub fn document_url(version: u64) -> String {
    format!("{DOCUMENT_URL}/{version}")
}

/// Check whether `url` is the exact main-document URL for `version`.
pub fn allow_main_navigation(url: &str, version: u64) -> bool {
    url == document_url(version)
}

/// Only inertial `srcdoc` / `about:blank` subframes are permitted to navigate.
pub fn allow_frame_navigation(url: &str) -> bool {
    matches!(url, "about:srcdoc" | "about:blank")
}

/// Whether `url` targets the synthetic internal reader origin.
pub fn is_internal_reader_url(url: &str) -> bool {
    url::Url::parse(url).ok().is_some_and(|parsed| {
        parsed.scheme() == "https" && parsed.host_str() == Some("kitemail.invalid")
    })
}

/// Validate an external URL for user-initiated handoff (`http`, `https`, `mailto`).
pub fn parse_external_url(value: &str) -> Option<url::Url> {
    let url = url::Url::parse(value).ok()?;
    let allowed = match url.scheme() {
        "http" | "https" => {
            url.host_str().is_some() && url.username().is_empty() && url.password().is_none()
        }
        "mailto" => !url.path().is_empty(),
        _ => false,
    };
    allowed.then_some(url)
}

/// Whether `value` is an allowed external URL.
pub fn allowed_external_url(value: &str) -> bool {
    parse_external_url(value).is_some()
}

/// Validate and optionally strip tracking parameters from an external link.
pub fn prepare_external_url(value: &str, clean: bool) -> Option<String> {
    let url = parse_external_url(value)?;
    let url = if clean {
        parse_external_url(&kitemail_mail_core::link_clean::clean_link(url.as_str()))?
    } else {
        url
    };
    Some(url.into())
}

/// Whether `value` is an allowed remote image URL (`http`/`https` without embedded credentials).
pub fn allowed_mail_image_url(value: &str, allow_remote: bool) -> bool {
    allow_remote
        && url::Url::parse(value).is_ok_and(|url| {
            matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
        })
}

/// Supported inline `data:image/*` formats decoded for native image rendering.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InlineImageFormat {
    Png,
    Jpeg,
    Gif,
    Webp,
    Bmp,
}

/// Decode a `data:image/*` URL into its image format and raw bytes.
pub fn decode_inline_mail_image(value: &str) -> Option<(InlineImageFormat, Vec<u8>)> {
    let data = data_url::DataUrl::process(value).ok()?;
    let mime = data.mime_type();
    if mime.type_ != "image" {
        return None;
    }
    let format = match mime.subtype.as_str() {
        "png" => InlineImageFormat::Png,
        "jpeg" => InlineImageFormat::Jpeg,
        "gif" => InlineImageFormat::Gif,
        "webp" => InlineImageFormat::Webp,
        "bmp" => InlineImageFormat::Bmp,
        _ => return None,
    };
    let (bytes, _) = data.decode_to_vec().ok()?;
    Some((format, bytes))
}

/// Build strict HTTP response headers for the synthetic reader document.
pub fn document_headers(allow_remote_images: bool) -> String {
    let image_sources = if allow_remote_images {
        "data: http: https:"
    } else {
        "data:"
    };
    format!(
        "Content-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; img-src {image_sources}; style-src 'unsafe-inline'; frame-src 'self'; base-uri 'none'; form-action 'none'\r\n"
    )
}

/// Decide whether a WebView2 subresource request for a remote image may proceed.
pub fn allow_remote_image_request(
    url: &str,
    is_image_context: bool,
    allowed: bool,
    referer: &str,
    version: u64,
) -> bool {
    is_image_context
        && (referer.is_empty() || referer == document_url(version))
        && allowed_mail_image_url(url, allowed)
}

/// Whether changing remote-image permission requires retiring the existing WebView2 host
/// so cached images/cookies from the previous mode cannot bleed through.
pub fn needs_new_host(has_document: bool, old_allowed: bool, new_allowed: bool) -> bool {
    has_document && (old_allowed || new_allowed)
}

/// Extract and validate a `mailto:` URL from an `open-link:` script bridge message.
pub fn open_link_message(message: &str) -> Option<&str> {
    let url = message.strip_prefix("open-link:")?;
    let parsed = parse_external_url(url)?;
    (parsed.scheme() == "mailto").then_some(url)
}

/// Document container mode loaded in the WebView2 host.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DocumentMode {
    #[default]
    SingleBody,
    ThreadContainer,
}

/// Explicit reader load status exposed to native/Tauri hosts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReaderLoadStatus {
    Loading,
    Ready,
    Unavailable,
    ProcessFailed,
}

/// Typed action emitted by the trusted thread-container wrapper script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WebViewAction {
    CardFold { msg_id: u32, collapsed: bool },
    CardReply(u32),
    CardReplyAll(u32),
    CardForward(u32),
    CardMarkUnread(u32),
    CardToggleFlag(u32),
    CardArchive(u32),
    CardDelete(u32),
    CardExportEml(u32),
    CardCopyBody(u32),
    CardToggleRaw(u32),
    CardUnsubscribe(u32),
    CardAllowRemote(u32),
    CardLoadBody(u32),
    CardOpenAttachment(u32),
    CardSaveAttachment(u32),
    CardSaveAllAttachments(u32),
    CardIcsReply { msg_id: u32, status: &'static str },
    CardMdn { msg_id: u32, send: bool },
    ScrollAnchor { msg_id: u32, offset_px: i32 },
    FindCount { current: u32, total: u32 },
}

/// Parse and strictly validate a thread-card bridge message from `reader_thread.js`.
pub fn parse_thread_action(message: &str) -> Option<WebViewAction> {
    let mut parts = message.split(':');
    let verb = parts.next()?;
    let id: u32 = parts.next()?.parse().ok()?;
    let third = parts.next();
    if parts.next().is_some() {
        return None;
    }
    match (verb, third) {
        ("card-fold", Some("1")) => Some(WebViewAction::CardFold {
            msg_id: id,
            collapsed: true,
        }),
        ("card-fold", Some("0")) => Some(WebViewAction::CardFold {
            msg_id: id,
            collapsed: false,
        }),
        ("card-reply", None) => Some(WebViewAction::CardReply(id)),
        ("card-reply-all", None) => Some(WebViewAction::CardReplyAll(id)),
        ("card-forward", None) => Some(WebViewAction::CardForward(id)),
        ("card-mark-unread", None) => Some(WebViewAction::CardMarkUnread(id)),
        ("card-toggle-flag", None) => Some(WebViewAction::CardToggleFlag(id)),
        ("card-archive", None) => Some(WebViewAction::CardArchive(id)),
        ("card-delete", None) => Some(WebViewAction::CardDelete(id)),
        ("card-export-eml", None) => Some(WebViewAction::CardExportEml(id)),
        ("card-copy-body", None) => Some(WebViewAction::CardCopyBody(id)),
        ("card-toggle-raw", None) => Some(WebViewAction::CardToggleRaw(id)),
        ("card-unsub", None) => Some(WebViewAction::CardUnsubscribe(id)),
        ("card-allow-remote", None) => Some(WebViewAction::CardAllowRemote(id)),
        ("card-load-body", None) => Some(WebViewAction::CardLoadBody(id)),
        ("card-open-att", None) => Some(WebViewAction::CardOpenAttachment(id)),
        ("card-save-att", None) => Some(WebViewAction::CardSaveAttachment(id)),
        ("card-save-all-atts", None) => Some(WebViewAction::CardSaveAllAttachments(id)),
        ("find-count", Some(raw_total)) => {
            let total: u32 = raw_total.parse().ok()?;
            Some(WebViewAction::FindCount { current: id, total })
        }
        ("card-ics", Some("ACCEPTED")) => Some(WebViewAction::CardIcsReply {
            msg_id: id,
            status: "ACCEPTED",
        }),
        ("card-ics", Some("TENTATIVE")) => Some(WebViewAction::CardIcsReply {
            msg_id: id,
            status: "TENTATIVE",
        }),
        ("card-ics", Some("DECLINED")) => Some(WebViewAction::CardIcsReply {
            msg_id: id,
            status: "DECLINED",
        }),
        ("card-mdn", Some("send")) => Some(WebViewAction::CardMdn {
            msg_id: id,
            send: true,
        }),
        ("card-mdn", Some("ignore")) => Some(WebViewAction::CardMdn {
            msg_id: id,
            send: false,
        }),
        ("scroll-anchor", Some(raw_offset)) => {
            let offset: i32 = raw_offset.parse().ok()?;
            (offset >= 0).then(|| WebViewAction::ScrollAnchor {
                msg_id: id,
                offset_px: offset.min(200_000),
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepared_links_are_checked_and_cleaned_for_dispatch() {
        let tracked = "https://example.com/path?id=42&utm_source=mail";
        assert_eq!(
            prepare_external_url(tracked, true).as_deref(),
            Some("https://example.com/path?id=42")
        );
        assert_eq!(
            prepare_external_url(tracked, false).as_deref(),
            Some(tracked)
        );
        assert_eq!(
            prepare_external_url("MAILTO:user@example.com", false).as_deref(),
            Some("mailto:user@example.com")
        );
        assert!(prepare_external_url("file:///C:/test", true).is_none());
    }

    #[test]
    fn navigation_is_limited_to_the_document_and_inert_frames() {
        assert!(allow_main_navigation(&document_url(2), 2));
        assert!(!allow_main_navigation(&document_url(1), 2));
        assert!(!allow_main_navigation(
            "https://kitemail.invalid/reader?other",
            2
        ));
        assert!(!allow_main_navigation("https://example.com/", 2));
        assert!(allow_frame_navigation("about:srcdoc"));
        assert!(allow_frame_navigation("about:blank"));
        assert!(!allow_frame_navigation(DOCUMENT_URL));
        assert!(!allow_frame_navigation("file:///C:/test"));
        assert!(!allow_frame_navigation("https://example.com/"));
        assert!(allowed_external_url("https://example.com/path"));
        assert!(allowed_external_url("mailto:user@example.com"));
        assert!(!allowed_external_url("file:///C:/test"));
        assert!(!allowed_external_url("https://user:pass@example.com"));
        assert!(!allowed_external_url("javascript:alert(1)"));
        assert!(document_headers(false).contains("Referrer-Policy: no-referrer"));
        assert!(document_headers(true).contains("Referrer-Policy: no-referrer"));
        assert!(document_headers(false).contains("img-src data:;"));
        assert!(document_headers(true).contains("img-src data: http: https:;"));
        assert!(document_headers(true).contains("default-src 'none'"));
        assert!(allow_remote_image_request(
            "https://example.com/a.png",
            true,
            true,
            &document_url(4),
            4
        ));
        assert!(!allow_remote_image_request(
            "https://example.com/a.png",
            true,
            false,
            &document_url(4),
            4
        ));
        assert!(!allow_remote_image_request(
            "https://example.com/a.png",
            true,
            true,
            &document_url(3),
            4
        ));
        assert!(allow_remote_image_request(
            "https://example.com/a.png",
            true,
            true,
            "",
            4
        ));
        assert!(!allow_remote_image_request(
            "https://example.com/a.png",
            true,
            false,
            "",
            4
        ));
        assert!(!allow_remote_image_request(
            "https://example.com/a.png",
            false,
            true,
            &document_url(4),
            4
        ));
    }

    #[test]
    fn thread_actions_parse_valid_and_reject_invalid_messages() {
        assert_eq!(
            parse_thread_action("card-fold:42:1"),
            Some(WebViewAction::CardFold {
                msg_id: 42,
                collapsed: true
            })
        );
        assert_eq!(
            parse_thread_action("card-fold:42:0"),
            Some(WebViewAction::CardFold {
                msg_id: 42,
                collapsed: false
            })
        );
        assert_eq!(
            parse_thread_action("card-reply:7"),
            Some(WebViewAction::CardReply(7))
        );
        assert_eq!(
            parse_thread_action("card-reply-all:7"),
            Some(WebViewAction::CardReplyAll(7))
        );
        assert_eq!(
            parse_thread_action("card-forward:7"),
            Some(WebViewAction::CardForward(7))
        );
        assert_eq!(
            parse_thread_action("card-mark-unread:7"),
            Some(WebViewAction::CardMarkUnread(7))
        );
        assert_eq!(
            parse_thread_action("card-toggle-flag:7"),
            Some(WebViewAction::CardToggleFlag(7))
        );
        assert_eq!(
            parse_thread_action("card-archive:7"),
            Some(WebViewAction::CardArchive(7))
        );
        assert_eq!(
            parse_thread_action("card-delete:7"),
            Some(WebViewAction::CardDelete(7))
        );
        assert_eq!(
            parse_thread_action("card-export-eml:7"),
            Some(WebViewAction::CardExportEml(7))
        );
        assert_eq!(
            parse_thread_action("card-copy-body:7"),
            Some(WebViewAction::CardCopyBody(7))
        );
        assert_eq!(
            parse_thread_action("card-toggle-raw:7"),
            Some(WebViewAction::CardToggleRaw(7))
        );
        assert_eq!(
            parse_thread_action("card-unsub:7"),
            Some(WebViewAction::CardUnsubscribe(7))
        );
        assert_eq!(
            parse_thread_action("card-allow-remote:7"),
            Some(WebViewAction::CardAllowRemote(7))
        );
        assert_eq!(parse_thread_action("card-trust-sender:7"), None);
        assert_eq!(
            parse_thread_action("card-load-body:7"),
            Some(WebViewAction::CardLoadBody(7))
        );
        assert_eq!(
            parse_thread_action("card-open-att:99"),
            Some(WebViewAction::CardOpenAttachment(99))
        );
        assert_eq!(
            parse_thread_action("card-save-att:99"),
            Some(WebViewAction::CardSaveAttachment(99))
        );
        assert_eq!(
            parse_thread_action("find-count:2:5"),
            Some(WebViewAction::FindCount {
                current: 2,
                total: 5
            })
        );
        assert_eq!(
            parse_thread_action("find-count:0:0"),
            Some(WebViewAction::FindCount {
                current: 0,
                total: 0
            })
        );
        assert_eq!(
            parse_thread_action("card-save-all-atts:7"),
            Some(WebViewAction::CardSaveAllAttachments(7))
        );
        assert_eq!(
            parse_thread_action("card-ics:7:ACCEPTED"),
            Some(WebViewAction::CardIcsReply {
                msg_id: 7,
                status: "ACCEPTED"
            })
        );
        assert_eq!(
            parse_thread_action("card-ics:7:TENTATIVE"),
            Some(WebViewAction::CardIcsReply {
                msg_id: 7,
                status: "TENTATIVE"
            })
        );
        assert_eq!(
            parse_thread_action("card-ics:7:DECLINED"),
            Some(WebViewAction::CardIcsReply {
                msg_id: 7,
                status: "DECLINED"
            })
        );
        assert_eq!(
            parse_thread_action("card-mdn:7:send"),
            Some(WebViewAction::CardMdn {
                msg_id: 7,
                send: true
            })
        );
        assert_eq!(
            parse_thread_action("card-mdn:7:ignore"),
            Some(WebViewAction::CardMdn {
                msg_id: 7,
                send: false
            })
        );
        assert_eq!(
            parse_thread_action("scroll-anchor:7:120"),
            Some(WebViewAction::ScrollAnchor {
                msg_id: 7,
                offset_px: 120
            })
        );

        for invalid in [
            "",
            "card-unknown:1",
            "card-reply:-1",
            "card-reply:abc",
            "card-reply:1:extra",
            "card-fold:1:2",
            "card-fold:1",
            "card-ics:1:MAYBE",
            "card-mdn:1:maybe",
            "scroll-anchor:1:-5",
            "scroll-anchor:1:NaN",
        ] {
            assert_eq!(
                parse_thread_action(invalid),
                None,
                "should reject {invalid}"
            );
        }
    }

    #[test]
    fn open_link_message_only_accepts_mailto() {
        assert_eq!(
            open_link_message("open-link:mailto:team@makenotion.com"),
            Some("mailto:team@makenotion.com")
        );
        assert_eq!(
            open_link_message("open-link:MAILTO:user@example.com?subject=hi"),
            Some("MAILTO:user@example.com?subject=hi")
        );
        for invalid in [
            "thread-wheel:10",
            "open-link:",
            "open-link:mailto:",
            "open-link:mailto:?subject=only",
            "open-link:https://example.com",
            "open-link:file:///C:/test",
            "open-link:javascript:alert(1)",
        ] {
            assert_eq!(open_link_message(invalid), None);
        }
    }

    #[test]
    fn needs_new_host_tracks_remote_image_permission_transitions() {
        assert!(!needs_new_host(false, false, true));
        assert!(!needs_new_host(true, false, false));
        assert!(needs_new_host(true, false, true));
        assert!(needs_new_host(true, true, false));
        assert!(needs_new_host(true, true, true));
    }

    #[test]
    fn inline_image_decoding_handles_supported_data_urls() {
        assert!(matches!(
            decode_inline_mail_image("data:image/png;base64,iVBORw0KGgo="),
            Some((InlineImageFormat::Png, _))
        ));
        assert!(decode_inline_mail_image("data:text/plain,hello").is_none());
        assert!(decode_inline_mail_image("https://example.com/a.png").is_none());
    }
}
