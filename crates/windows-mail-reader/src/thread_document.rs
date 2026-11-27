//! Single WebView2 thread-container HTML document builder.
//!
//! Renders all cards of a conversation thread into one HTML document where each
//! message body sits in its own `<iframe sandbox="allow-same-origin allow-popups">`:
//! - **Scriptless mail frames**: inner mail iframes disallow JS; only the outer
//!   container receives the trusted `reader_thread.js` bridge after load.
//! - **Two-layer remote-image isolation**: the outer HTTP CSP allows `http: https:`
//!   only when at least one card permits remote images; every blocked card strips
//!   remote `<img>` tags and injects a tightened `img-src data:` meta CSP inside
//!   its `srcdoc`.
//! - **Deferred hydration for collapsed cards**: initially collapsed cards keep
//!   their frame inside `<template class="km-frame-template">` with `data-km-srcdoc`
//!   until expanded.

use std::fmt::Write as _;

use kitemail_mail_core::config;
use kitemail_mail_core::imap_client::{plain_text_to_html, strip_remote_images_for_webview};
use kitemail_mail_core::verify::{check_sender_summary, SenderTrust};
use kitemail_mail_core::{MessageDetail, ThreadMessage};

use crate::dark::{adapt_colors_for_dark, pin_color_scheme};
use crate::document::{escape_html, find_html_tag, reader_body_colors, DocumentAppearance};

/// Starred amber gold color (`#e5a50a`).
pub const STAR_GOLD: u32 = 0xe5_a5_0a;

const AVATAR_COLORS: [&str; 10] = [
    "#337fdc", "#0f9ac8", "#29ae74", "#6ab85b", "#d29d09", "#d68400", "#ed5b00", "#e62d42",
    "#e33b6a", "#9945b5",
];

fn fnv1a(text: &str) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in text.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

fn is_cjk(c: char) -> bool {
    matches!(
        c as u32,
        0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0xF900..=0xFAFF | 0x30000..=0x3134F
    )
}

fn word_initials(text: &str) -> Vec<char> {
    let outside = {
        let mut out = String::new();
        let mut depth = 0usize;
        for c in text.chars() {
            match c {
                '(' | '<' | '[' => depth += 1,
                ')' | '>' | ']' => {
                    if depth > 0 {
                        depth -= 1;
                    }
                }
                _ if depth > 0 => {}
                _ => out.push(c),
            }
        }
        if out.chars().any(|c| c.is_alphanumeric()) {
            out
        } else {
            text.to_string()
        }
    };
    outside
        .split_whitespace()
        .filter_map(|w| w.chars().find(|c| c.is_alphanumeric()))
        .collect()
}

fn sender_initials(name: &str) -> String {
    let trimmed = name.trim();
    if let Some(first) = trimmed.chars().find(|c| c.is_alphanumeric()) {
        if is_cjk(first) {
            return first.to_string();
        }
    }
    let letters = word_initials(&trimmed.to_uppercase());
    let mut out = String::new();
    if let Some(&first) = letters.first() {
        out.push(first);
        if first.is_ascii_alphanumeric() {
            if let Some(&last_ascii) = letters[1..]
                .iter()
                .rev()
                .find(|c| c.is_ascii_alphanumeric())
            {
                out.push(last_ascii);
            }
        } else if letters.len() > 1 {
            out.extend(letters.last());
        }
    }
    if out.is_empty() {
        "?".to_string()
    } else {
        out
    }
}

/// Return avatar initials and preset hex background color for a sender.
pub fn avatar_badge_spec(name: &str, email: &str) -> (String, &'static str) {
    let trimmed_email = email.trim();
    if !trimmed_email.is_empty() {
        if let Some(brand) = kitemail_mail_core::avatar::lookup_builtin_brand(trimmed_email) {
            return (brand.short.to_string(), brand.color);
        }
    }
    let initials = sender_initials(name);
    let color_key = if trimmed_email.is_empty() {
        name
    } else {
        trimmed_email
    };
    let bg = AVATAR_COLORS[(fnv1a(color_key) % AVATAR_COLORS.len() as u32) as usize];
    (initials, bg)
}

/// Split `"Display Name <email@domain>"` into `(display_name, email)`.
pub fn split_sender(sender: &str) -> (String, String) {
    if let (Some(open), Some(close)) = (sender.find('<'), sender.rfind('>')) {
        if close > open {
            let name = sender[..open].trim().to_string();
            let email = sender[open + 1..close].trim().to_string();
            return (name, email);
        }
    }
    (sender.trim().to_string(), String::new())
}

/// Normalize comma-separated recipients into a clean display string.
pub fn recipient_list(raw: &str) -> String {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Extract a single-line snippet for folded thread cards.
pub fn first_snippet_line(body_text: &str) -> String {
    body_text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
        .chars()
        .take(120)
        .collect()
}

/// Format raw message headers, plain text, and optional HTML source for raw-view mode.
#[allow(clippy::too_many_arguments)]
pub fn format_raw_source(
    rfc_message_id: &str,
    sender: &str,
    recipients: &str,
    cc: &str,
    date_display: &str,
    subject: &str,
    body_text: &str,
    body_html: Option<&str>,
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Message-ID: {}\nFrom: {}\nTo: {}\nCc: {}\nDate: {}\nSubject: {}\n",
        if rfc_message_id.trim().is_empty() {
            "(无)"
        } else {
            rfc_message_id
        },
        sender,
        if recipients.trim().is_empty() {
            "(无)"
        } else {
            recipients
        },
        if cc.trim().is_empty() { "(无)" } else { cc },
        date_display,
        subject,
    ));
    out.push_str("\n--- 纯文本正文 ---\n");
    out.push_str(body_text);
    if let Some(html) = body_html.filter(|s| !s.trim().is_empty()) {
        out.push_str("\n\n--- 原始 HTML 源码 ---\n");
        out.push_str(html);
    }
    out
}

/// Plain CSS color palette for thread container rendering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadPalette {
    pub canvas: String,
    pub surface_raised: String,
    pub border: String,
    pub text: String,
    pub text_muted: String,
    pub accent: String,
    pub accent_border: String,
    pub accent_soft: String,
    pub danger: String,
    pub danger_border: String,
    pub danger_bg: String,
}

impl ThreadPalette {
    /// Build a default thread palette for light or dark mode.
    pub fn for_dark(dark: bool) -> Self {
        let appearance = DocumentAppearance::for_dark(dark, 100, true);
        Self::from_appearance(&appearance)
    }

    /// Derive a thread palette from a [`DocumentAppearance`].
    pub fn from_appearance(appearance: &DocumentAppearance) -> Self {
        let (accent_border, accent_soft) = css_alpha_pair(&appearance.accent_color, 0.45, 0.12)
            .unwrap_or_else(|| {
                (
                    "rgba(53,132,228,0.45)".to_string(),
                    "rgba(53,132,228,0.12)".to_string(),
                )
            });
        let danger = if appearance.dark {
            "#f66151"
        } else {
            "#e01b24"
        };
        let (danger_border, danger_bg) = css_alpha_pair(danger, 0.45, 0.10).unwrap_or_else(|| {
            (
                "rgba(224,27,36,0.45)".to_string(),
                "rgba(224,27,36,0.10)".to_string(),
            )
        });
        Self {
            canvas: appearance.canvas_bg.clone(),
            surface_raised: if appearance.dark {
                "#363636".to_string()
            } else {
                "#ffffff".to_string()
            },
            border: appearance.border_color.clone(),
            text: appearance.text_color.clone(),
            text_muted: appearance.muted_color.clone(),
            accent: appearance.accent_color.clone(),
            accent_border,
            accent_soft,
            danger: danger.to_string(),
            danger_border,
            danger_bg,
        }
    }
}

fn css_alpha_pair(color: &str, alpha_a: f32, alpha_b: f32) -> Option<(String, String)> {
    let [r, g, b, _] = crate::dark::parse_css_color(color, false)?;
    let to8 = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    let (r, g, b) = (to8(r), to8(g), to8(b));
    Some((
        format!("rgba({r},{g},{b},{alpha_a:.2})"),
        format!("rgba({r},{g},{b},{alpha_b:.2})"),
    ))
}

/// Per-card render flags for a thread document.
pub struct ThreadCardRenderInput<'a> {
    pub message: &'a ThreadMessage,
    pub is_current: bool,
    pub is_collapsed: bool,
    pub show_raw_source: bool,
    pub block_remote: bool,
    pub card_dark: bool,
    pub card_pending: bool,
    pub eml_exporting: bool,
    pub auth_results: Option<&'a str>,
    pub unsubscribe_url: Option<&'a str>,
}

/// Input context for building a multi-card thread container HTML document.
pub struct ThreadDocContext<'a> {
    pub detail: &'a MessageDetail,
    pub cards: &'a [ThreadCardRenderInput<'a>],
    pub palette: &'a ThreadPalette,
    pub app_dark_mode: bool,
    pub zoom_percent: u32,
    pub ui_text_scale_percent: u32,
    pub date_style: &'a str,
    pub clock_style: &'a str,
    pub show_remote_banner: bool,
    pub always_show_recipients: bool,
    pub card_attachments: bool,
    pub demo_mode: bool,
    pub read_receipt_policy: &'a str,
    pub initial_anchor: Option<(u32, i32)>,
}

fn build_card_frame_html(
    body_text: &str,
    body_html: Option<&str>,
    block_remote: bool,
    card_dark: bool,
    zoom_percent: u32,
) -> (String, usize) {
    let (source_html, blocked_images) =
        if let Some(raw_html) = body_html.filter(|s| !s.trim().is_empty()) {
            if block_remote {
                strip_remote_images_for_webview(raw_html)
            } else {
                (raw_html.to_string(), 0)
            }
        } else {
            (plain_text_to_html(body_text), 0)
        };

    let html = if card_dark {
        adapt_colors_for_dark(&source_html)
    } else {
        source_html
    };
    let html = pin_color_scheme(&html, card_dark);
    let (bg, fg) = reader_body_colors(card_dark);
    let scheme = if card_dark { "dark" } else { "light" };
    let img_src = if block_remote {
        "data:"
    } else {
        "data: http: https:"
    };
    let meta_csp = format!(
        "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src {img_src}; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'\">"
    );
    let style = format!(
        "<style>:root{{color-scheme:{scheme};zoom:{}%}}html{{display:flow-root!important;height:auto!important;min-height:0!important;overflow-y:hidden!important}}body{{height:auto!important;min-height:0!important}}:where(body){{margin:0;background-color:{bg};color:{fg};font:14px/1.6 system-ui,\"Segoe UI\",sans-serif}}img{{max-width:100%;height:auto}}</style>",
        zoom_percent.clamp(50, 250)
    );
    let lower = html.to_ascii_lowercase();
    let insertion = find_html_tag(&lower, "<head")
        .or_else(|| lower.find("<!doctype"))
        .and_then(|start| lower[start..].find('>').map(|end| start + end + 1))
        .unwrap_or(0);
    let doc = format!(
        "{}{}{}{}",
        &html[..insertion],
        meta_csp,
        style,
        &html[insertion..]
    );
    (doc, blocked_images)
}

fn container_css(ctx: &ThreadDocContext<'_>) -> String {
    let p = ctx.palette;
    let scheme = if ctx.app_dark_mode { "dark" } else { "light" };
    let (card_bg, _) = reader_body_colors(ctx.app_dark_mode);
    let canvas = &p.canvas;
    let raised = &p.surface_raised;
    let border = &p.border;
    let text = &p.text;
    let muted = &p.text_muted;
    let accent = &p.accent;
    let accent_border = &p.accent_border;
    let accent_soft = &p.accent_soft;
    let danger = &p.danger;
    let danger_border = &p.danger_border;
    let danger_bg = &p.danger_bg;
    let ui_scale = config::clamp_ui_text_scale(ctx.ui_text_scale_percent) as f32 / 100.0;
    let base_px = 14.0 * ui_scale;
    let small_px = 12.0 * ui_scale;
    let zoom_scale = ctx.zoom_percent.clamp(50, 250) as f32 / 100.0;
    let raw_font_px = (12.0 * zoom_scale).max(10.0);
    let star_hex = format!("#{:06x}", STAR_GOLD);
    let scroll_thumb = if ctx.app_dark_mode {
        "rgba(255,255,255,0.20)"
    } else {
        "rgba(0,0,0,0.18)"
    };
    let menu_shadow = if ctx.app_dark_mode {
        "0 8px 24px rgba(0,0,0,0.55)"
    } else {
        "0 8px 24px rgba(0,0,0,0.14)"
    };

    format!(
        "\
:root{{color-scheme:{scheme};--km-canvas:{canvas};--km-card:{card_bg};--km-raised:{raised};\
--km-border:{border};--km-text:{text};--km-muted:{muted};--km-accent:{accent};\
--km-accent-border:{accent_border};--km-accent-soft:{accent_soft};--km-danger:{danger};\
--km-danger-border:{danger_border};--km-danger-bg:{danger_bg};--km-star:{star_hex};}}\
*{{box-sizing:border-box;}}\
html,body{{margin:0;padding:0;background:var(--km-canvas);color:var(--km-text);\
font:{base_px:.2}px/1.5 system-ui,\"Segoe UI\",sans-serif;}}\
body{{padding:12px 24px 24px;}}\
::-webkit-scrollbar{{width:8px;height:8px;}}\
::-webkit-scrollbar-thumb{{background:{scroll_thumb};border-radius:4px;}}\
::-webkit-scrollbar-track{{background:transparent;}}\
.km-card{{background:var(--km-card);border:1px solid var(--km-border);border-radius:12px;\
padding:20px;margin-bottom:12px;box-shadow:0 1px 2px rgba(0,0,0,0.04);}}\
.km-card:last-child{{margin-bottom:0;}}\
.km-card.km-current{{border-color:var(--km-accent-border);}}\
.km-card.km-folded{{background:var(--km-raised);padding:12px 20px;}}\
.km-hdr{{display:flex;align-items:center;gap:8px;cursor:pointer;user-select:none;min-width:0;}}\
.km-ava{{width:34px;height:34px;border-radius:50%;flex:none;display:flex;align-items:center;\
justify-content:center;color:#fff;font-weight:700;font-size:14.28px;}}\
.km-card.km-folded .km-ava{{width:28px;height:28px;font-size:11.76px;}}\
.km-sender-col{{min-width:0;display:flex;flex-direction:column;flex:0 1 auto;}}\
.km-card.km-folded .km-sender-col{{flex-direction:row;align-items:center;gap:8px;flex:1;}}\
.km-name{{font-size:{base_px:.2}px;font-weight:700;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;}}\
.km-card.km-folded .km-name{{font-weight:600;flex:none;max-width:220px;}}\
.km-email{{font-size:{small_px:.2}px;color:var(--km-muted);white-space:nowrap;overflow:hidden;text-overflow:ellipsis;}}\
.km-card.km-folded .km-email{{display:none;}}\
.km-snippet{{display:none;font-size:{small_px:.2}px;color:var(--km-muted);white-space:nowrap;\
overflow:hidden;text-overflow:ellipsis;flex:1;min-width:0;}}\
.km-card.km-folded .km-snippet{{display:block;}}\
.km-spacer{{flex:1;min-width:8px;}}\
.km-card.km-folded .km-spacer{{display:none;}}\
.km-badge{{flex:none;padding:2px 8px;border-radius:999px;font-size:{small_px:.2}px;\
background:var(--km-canvas);border:1px solid currentColor;line-height:1.3;}}\
.km-card.km-folded .km-badge{{display:none;}}\
.km-star-dot{{display:none;color:var(--km-star);font-size:{small_px:.2}px;flex:none;}}\
.km-star-dot.on{{display:inline-flex;}}\
.km-date{{flex:none;font-size:{small_px:.2}px;color:var(--km-muted);white-space:nowrap;}}\
.km-more-btn{{flex:none;width:28px;height:28px;border:0;border-radius:6px;background:transparent;\
color:var(--km-muted);cursor:pointer;display:inline-flex;align-items:center;justify-content:center;\
font-size:16px;line-height:1;}}\
.km-more-btn:hover{{background:var(--km-accent-soft);color:var(--km-text);}}\
.km-card.km-folded .km-details{{display:none;}}\
.km-details{{display:flex;flex-direction:column;gap:12px;margin-top:8px;}}\
.km-rcpts{{padding-left:42px;display:flex;flex-direction:column;gap:2px;font-size:{small_px:.2}px;color:var(--km-muted);}}\
.km-rcpts div{{white-space:nowrap;overflow:hidden;text-overflow:ellipsis;}}\
.km-banner{{display:flex;align-items:center;justify-content:space-between;gap:8px;padding:6px 12px;\
border-radius:8px;font-size:{small_px:.2}px;background:var(--km-canvas);border:1px solid var(--km-border);}}\
.km-banner.km-banner-col{{flex-direction:column;align-items:stretch;gap:6px;padding:8px 12px;}}\
.km-banner-row{{display:flex;align-items:center;justify-content:space-between;gap:8px;flex-wrap:wrap;}}\
.km-banner.km-banner-danger{{background:var(--km-danger-bg);border-color:var(--km-danger-border);color:var(--km-danger);}}\
.km-banner.km-banner-accent{{background:var(--km-accent-soft);border-color:transparent;}}\
.km-btn-group{{display:flex;align-items:center;gap:6px;flex:none;flex-wrap:wrap;}}\
.km-btn{{appearance:none;border:1px solid var(--km-border);background:var(--km-card);color:var(--km-text);\
border-radius:6px;padding:4px 10px;font-size:{small_px:.2}px;cursor:pointer;line-height:1.35;}}\
.km-btn:hover{{background:var(--km-accent-soft);border-color:var(--km-accent-border);}}\
.km-btn.ghost{{border-color:transparent;background:transparent;color:var(--km-muted);}}\
.km-btn.ghost:hover{{background:var(--km-accent-soft);color:var(--km-text);}}\
.km-pan{{overflow-x:auto;border-radius:6px;}}\
.km-pan.km-picked{{border:1px solid var(--km-border);border-radius:8px;padding:8px;}}\
iframe.km-frame{{width:100%;border:0;display:block;min-height:24px;visibility:hidden;}}\
iframe.km-frame.km-live{{visibility:visible;}}\
.km-raw{{margin:0;padding:12px;border-radius:8px;background:var(--km-canvas);border:1px solid var(--km-border);\
font-family:ui-monospace,Consolas,monospace;font-size:{raw_font_px:.2}px;white-space:pre-wrap;overflow-wrap:anywhere;}}\
.km-loading{{padding:20px 0;text-align:center;font-size:{base_px:.2}px;color:var(--km-muted);}}\
.km-atts{{display:flex;flex-wrap:wrap;align-items:center;gap:8px;padding-top:10px;border-top:1px solid var(--km-border);}}\
.km-att-chip{{display:inline-flex;align-items:center;gap:8px;padding:4px 8px;border-radius:8px;\
border:1px solid var(--km-border);background:var(--km-canvas);font-size:{small_px:.2}px;}}\
.km-att-name{{max-width:240px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;}}\
.km-att-size{{color:var(--km-muted);}}\
.km-menu{{position:fixed;z-index:9999;min-width:176px;padding:4px;border-radius:8px;\
background:var(--km-raised);border:1px solid var(--km-border);box-shadow:{menu_shadow};\
display:flex;flex-direction:column;gap:2px;}}\
.km-menu-item{{appearance:none;border:0;background:transparent;color:var(--km-text);text-align:left;\
padding:6px 10px;border-radius:6px;font-size:{small_px:.2}px;cursor:pointer;line-height:1.35;}}\
.km-menu-item:hover:not(:disabled){{background:var(--km-accent-soft);}}\
.km-menu-item.danger{{color:var(--km-danger);}}\
.km-menu-item:disabled{{opacity:0.5;cursor:default;}}\
.km-menu-sep{{height:1px;background:var(--km-border);margin:3px 6px;}}\
.km-link-preview{{position:fixed;left:12px;bottom:10px;z-index:9998;max-width:75vw;\
padding:4px 10px;border-radius:6px;background:var(--km-raised);border:1px solid var(--km-border);\
color:var(--km-muted);font-size:11px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis;\
pointer-events:none;opacity:0;transition:opacity 100ms ease;}}\
.km-link-preview.on{{opacity:1;}}\
"
    )
}

/// Build the full thread container HTML document and report whether any card allows remote images.
pub fn build_thread_document(ctx: &ThreadDocContext<'_>) -> (Vec<u8>, bool) {
    let mut any_remote_allowed = false;
    let mut cards_html = String::new();

    for card in ctx.cards {
        let tm = card.message;
        let card_subject = if tm.subject.trim().is_empty() {
            ctx.detail.subject.as_str()
        } else {
            tm.subject.as_str()
        };
        let formatted_date = kitemail_mail_core::format_message_datetime(
            tm.date_ts,
            &tm.date_display,
            ctx.date_style,
            ctx.clock_style,
        );
        let (sender_name, sender_email) = split_sender(&tm.sender);
        let display_name = if sender_name.is_empty() {
            tm.sender.clone()
        } else {
            sender_name
        };
        let (avatar_text, avatar_bg) = avatar_badge_spec(&display_name, &sender_email);
        let snippet = first_snippet_line(&tm.body_text);
        let sender_check = check_sender_summary(&tm.sender, card.auth_results);
        let trust_badge = match sender_check.trust {
            SenderTrust::Pass => Some(("已验证", "#219653")),
            SenderTrust::Suspicious => Some(("疑似仿冒", "#d97706")),
            SenderTrust::Fail => Some(("校验失败", "var(--km-danger)")),
            SenderTrust::Unverified => None,
        };
        let spoof_banner = if matches!(
            sender_check.trust,
            SenderTrust::Suspicious | SenderTrust::Fail
        ) {
            Some(
                sender_check
                    .lookalike_warning
                    .clone()
                    .unwrap_or_else(|| sender_check.summary.clone()),
            )
        } else {
            None
        };
        let detected_invite = if tm.body_text.contains("BEGIN:VCALENDAR") {
            kitemail_mail_core::invite::parse(&tm.body_text)
        } else {
            None
        };
        let has_ics_attachment = tm
            .attachments
            .iter()
            .any(|a| a.filename.to_ascii_lowercase().ends_with(".ics"));
        let has_mdn_header = ctx.read_receipt_policy != "never"
            && kitemail_mail_core::receipt::detect_raw(tm.body_text.as_bytes()).is_some();

        let (body_html_block, blocked_images) = if card.show_raw_source {
            let raw = format_raw_source(
                &tm.rfc_message_id,
                &tm.sender,
                &tm.recipients,
                &tm.cc,
                &formatted_date,
                card_subject,
                &tm.body_text,
                tm.body_html.as_deref(),
            );
            (
                format!("<pre class=\"km-raw\">{}</pre>", escape_html(&raw)),
                0,
            )
        } else if card.card_pending {
            (
                "<div class=\"km-loading\">正在加载邮件正文…</div>".to_string(),
                0,
            )
        } else if !tm.body_loaded {
            (
                format!(
                    "<div><button type=\"button\" class=\"km-btn\" data-action=\"load-body\" data-msg-id=\"{}\">加载完整正文</button></div>",
                    tm.id
                ),
                0,
            )
        } else {
            if !card.block_remote
                && tm
                    .body_html
                    .as_deref()
                    .is_some_and(|h| !h.trim().is_empty())
            {
                any_remote_allowed = true;
            }
            let (frame_doc, blocked) = build_card_frame_html(
                &tm.body_text,
                tm.body_html.as_deref(),
                card.block_remote,
                card.card_dark,
                ctx.zoom_percent,
            );
            let src_attr = if card.is_collapsed {
                "data-km-srcdoc"
            } else {
                "srcdoc"
            };
            let (picked_bg, _) = reader_body_colors(card.card_dark);
            let picked_class = if card.card_dark != ctx.app_dark_mode {
                " km-picked"
            } else {
                ""
            };
            let pan_style = format!(" style=\"background:{picked_bg}\"");
            let (template_start, template_end) = if card.is_collapsed {
                ("<template class=\"km-frame-template\">", "</template>")
            } else {
                ("", "")
            };
            (
                format!(
                    "<div class=\"km-pan{picked_class}\"{pan_style}>{template_start}<iframe class=\"km-frame\" data-msg-id=\"{}\" sandbox=\"allow-same-origin allow-popups\" style=\"height:160px\" {src_attr}=\"{}\"></iframe>{template_end}</div>",
                    tm.id,
                    escape_html(&frame_doc)
                ),
                blocked,
            )
        };

        let mut classes = String::from("km-card");
        if card.is_current {
            classes.push_str(" km-current");
        }
        if card.is_collapsed {
            classes.push_str(" km-folded");
        }

        let _ = write!(
            cards_html,
            "<section class=\"{classes}\" data-msg-id=\"{}\" data-flagged=\"{}\" data-exporting=\"{}\" data-raw=\"{}\" data-demo=\"{}\" data-unsub=\"{}\">",
            tm.id,
            u8::from(tm.flagged),
            u8::from(card.eml_exporting),
            u8::from(card.show_raw_source),
            u8::from(ctx.demo_mode),
            u8::from(card.unsubscribe_url.is_some()),
        );

        let _ = write!(
            cards_html,
            "<div class=\"km-hdr\"><div class=\"km-ava\" style=\"background:{avatar_bg}\">{}</div><div class=\"km-sender-col\"><div class=\"km-name\">{}</div>",
            escape_html(&avatar_text),
            escape_html(&display_name),
        );
        if !sender_email.is_empty() {
            let _ = write!(
                cards_html,
                "<div class=\"km-email\">&lt;{}&gt;</div>",
                escape_html(&sender_email)
            );
        }
        let _ = write!(
            cards_html,
            "</div><div class=\"km-snippet\">{}</div><div class=\"km-spacer\"></div>",
            escape_html(&snippet)
        );
        if let Some((label, color)) = trust_badge {
            let _ = write!(
                cards_html,
                "<span class=\"km-badge\" style=\"color:{color}\">{}</span>",
                escape_html(label)
            );
        }
        let _ = write!(
            cards_html,
            "<span class=\"km-star-dot{}\" title=\"已加星标\">★</span><div class=\"km-date\">{}</div><button type=\"button\" class=\"km-more-btn\" title=\"更多操作\">⋯</button></div>",
            if tm.flagged { " on" } else { "" },
            escape_html(&formatted_date)
        );

        cards_html.push_str("<div class=\"km-details\">");

        if ctx.always_show_recipients {
            let rcpts = recipient_list(&tm.recipients);
            let ccs = recipient_list(&tm.cc);
            if !rcpts.is_empty() || !ccs.is_empty() {
                cards_html.push_str("<div class=\"km-rcpts\">");
                if !rcpts.is_empty() {
                    let _ = write!(cards_html, "<div>收件人：{}</div>", escape_html(&rcpts));
                }
                if !ccs.is_empty() {
                    let _ = write!(cards_html, "<div>抄送：{}</div>", escape_html(&ccs));
                }
                cards_html.push_str("</div>");
            }
        }

        if card.show_raw_source {
            let _ = write!(
                cards_html,
                "<div class=\"km-banner\"><span>邮件原文视图（原始信头与正文源码）</span><button type=\"button\" class=\"km-btn\" data-action=\"toggle-raw\" data-msg-id=\"{}\">返回正文</button></div>",
                tm.id
            );
        }

        if let Some(warning) = spoof_banner {
            let _ = write!(
                cards_html,
                "<div class=\"km-banner km-banner-danger\"><span>⚠ {}</span></div>",
                escape_html(&warning)
            );
        }

        if has_mdn_header {
            let _ = write!(
                cards_html,
                "<div class=\"km-banner\"><span>发件人请求已读回执（RFC 8098）</span><div class=\"km-btn-group\"><button type=\"button\" class=\"km-btn\" data-action=\"mdn\" data-decision=\"send\" data-msg-id=\"{}\">发送回执</button><button type=\"button\" class=\"km-btn ghost\" data-action=\"mdn\" data-decision=\"ignore\" data-msg-id=\"{}\">忽略</button></div></div>",
                tm.id, tm.id
            );
        }

        if detected_invite.is_some() || has_ics_attachment {
            let title = detected_invite
                .as_ref()
                .map(|i| i.summary.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("日历会议邀请 (.ics)");
            let loc = detected_invite
                .as_ref()
                .map(|i| i.location.as_str())
                .unwrap_or("");
            let org = detected_invite
                .as_ref()
                .and_then(|i| i.organizer.as_ref())
                .map(|o| {
                    if o.name.is_empty() {
                        o.email.clone()
                    } else {
                        format!("{} <{}>", o.name, o.email)
                    }
                })
                .unwrap_or_default();
            let _ = write!(
                cards_html,
                "<div class=\"km-banner km-banner-col\"><div class=\"km-banner-row\"><strong>{}</strong><div class=\"km-btn-group\">\
                <button type=\"button\" class=\"km-btn\" data-action=\"ics\" data-status=\"ACCEPTED\" data-msg-id=\"{}\">接受</button>\
                <button type=\"button\" class=\"km-btn\" data-action=\"ics\" data-status=\"TENTATIVE\" data-msg-id=\"{}\">暂定</button>\
                <button type=\"button\" class=\"km-btn\" data-action=\"ics\" data-status=\"DECLINED\" data-msg-id=\"{}\">拒绝</button>\
                </div></div>",
                escape_html(title),
                tm.id,
                tm.id,
                tm.id
            );
            if !org.is_empty() || !loc.is_empty() {
                let mut meta = Vec::new();
                if !org.is_empty() {
                    meta.push(format!("组织者：{org}"));
                }
                if !loc.is_empty() {
                    meta.push(format!("地点：{loc}"));
                }
                let _ = write!(
                    cards_html,
                    "<div style=\"color:var(--km-muted)\">{}</div>",
                    escape_html(&meta.join(" · "))
                );
            }
            cards_html.push_str("</div>");
        }

        if !card.show_raw_source && blocked_images > 0 && ctx.show_remote_banner {
            let _ = write!(
                cards_html,
                "<div class=\"km-banner km-banner-accent\"><span>已拦截 {blocked_images} 张远程图片以保护隐私</span><div class=\"km-btn-group\">\
                <button type=\"button\" class=\"km-btn\" data-action=\"allow-remote\" data-msg-id=\"{}\">显示本次图片</button>\
                </div></div>",
                tm.id
            );
        }

        cards_html.push_str(&body_html_block);

        if ctx.card_attachments && !tm.attachments.is_empty() {
            cards_html.push_str("<div class=\"km-atts\">");
            for att in &tm.attachments {
                let size_text = config::format_bytes(att.size_bytes as u64);
                let size_label = if att.downloaded {
                    size_text
                } else {
                    format!("约 {size_text} · 未下载")
                };
                let tip = if att.downloaded {
                    "保存附件"
                } else {
                    "下载并保存附件"
                };
                let _ = write!(
                    cards_html,
                    "<div class=\"km-att-chip\"><span class=\"km-att-name\" title=\"{}\">{}</span><span class=\"km-att-size\">{}</span><button type=\"button\" class=\"km-btn ghost\" title=\"{tip}\" data-action=\"save-att\" data-att-id=\"{}\">保存</button></div>",
                    escape_html(&att.filename),
                    escape_html(&att.filename),
                    escape_html(&size_label),
                    att.id
                );
            }
            if tm.attachments.len() >= 2 {
                let _ = write!(
                    cards_html,
                    "<button type=\"button\" class=\"km-btn\" data-action=\"save-all-atts\" data-msg-id=\"{}\">保存全部附件</button>",
                    tm.id
                );
            }
            cards_html.push_str("</div>");
        }

        cards_html.push_str("</div></section>");
    }

    let (anchor_id, anchor_offset) = ctx.initial_anchor.unwrap_or((ctx.detail.id, 0));
    let css = container_css(ctx);
    let doc = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><style>{css}</style></head><body data-initial-anchor-id=\"{anchor_id}\" data-initial-anchor-offset=\"{anchor_offset}\">{cards_html}</body></html>"
    );
    (doc.into_bytes(), any_remote_allowed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kitemail_mail_core::AttachmentMeta;

    fn sample_thread_msg(id: u32, html: Option<&str>, text: &str) -> ThreadMessage {
        ThreadMessage {
            id,
            rfc_message_id: format!("<m{id}@example.com>"),
            subject: "Re: Test Thread".to_string(),
            sender: "Alice <alice@example.com>".to_string(),
            recipients: "bob@example.com".to_string(),
            cc: String::new(),
            date_ts: 1_700_000_000,
            date_display: "10:00".to_string(),
            body_text: text.to_string(),
            body_html: html.map(str::to_string),
            body_loaded: true,
            unread: false,
            flagged: id == 1,
            attachments: Vec::new(),
            uid: id,
            folder_path: "INBOX".to_string(),
        }
    }

    fn sample_detail(msgs: Vec<ThreadMessage>) -> MessageDetail {
        MessageDetail {
            id: 2,
            account_id: 1,
            folder_id: 1,
            folder_path: "INBOX".to_string(),
            uid: 2,
            rfc_message_id: "<m2@example.com>".to_string(),
            sender: "Bob <bob@example.com>".to_string(),
            recipients: "alice@example.com".to_string(),
            cc: String::new(),
            subject: "Re: Test Thread".to_string(),
            date_ts: 1_700_000_100,
            date_display: "10:05".to_string(),
            body_text: "Latest".to_string(),
            body_html: Some("<p>Latest</p>".to_string()),
            flagged: false,
            attachments: Vec::new(),
            thread_messages: msgs,
            unsubscribe_url: None,
            unsubscribe_one_click: false,
            auth_results: None,
        }
    }

    #[test]
    fn collapsed_cards_defer_srcdoc_and_expanded_cards_use_srcdoc() {
        let msgs = vec![
            sample_thread_msg(1, Some("<p>Older hidden body</p>"), "Older"),
            sample_thread_msg(2, Some("<p>Newest body</p>"), "Newest"),
        ];
        let detail = sample_detail(msgs.clone());
        let palette = ThreadPalette::for_dark(false);
        let inputs = vec![
            ThreadCardRenderInput {
                message: &msgs[0],
                is_current: false,
                is_collapsed: true,
                show_raw_source: false,
                block_remote: true,
                card_dark: false,
                card_pending: false,
                eml_exporting: false,
                auth_results: None,
                unsubscribe_url: None,
            },
            ThreadCardRenderInput {
                message: &msgs[1],
                is_current: true,
                is_collapsed: false,
                show_raw_source: false,
                block_remote: true,
                card_dark: false,
                card_pending: false,
                eml_exporting: false,
                auth_results: None,
                unsubscribe_url: None,
            },
        ];
        let ctx = ThreadDocContext {
            detail: &detail,
            cards: &inputs,
            palette: &palette,
            app_dark_mode: false,
            zoom_percent: 100,
            ui_text_scale_percent: 100,
            date_style: "relative",
            clock_style: "24h",
            show_remote_banner: true,
            always_show_recipients: false,
            card_attachments: true,
            demo_mode: false,
            read_receipt_policy: "ask",
            initial_anchor: None,
        };
        let (bytes, any_remote) = build_thread_document(&ctx);
        let html = String::from_utf8(bytes).unwrap();
        assert!(!any_remote);
        assert!(html.contains(
            "<template class=\"km-frame-template\"><iframe class=\"km-frame\" data-msg-id=\"1\""
        ));
        assert!(html.contains("</iframe></template></div>"));
        assert!(!html.contains(
            "<template class=\"km-frame-template\"><iframe class=\"km-frame\" data-msg-id=\"2\""
        ));
        assert!(html.contains("data-msg-id=\"1\" sandbox=\"allow-same-origin allow-popups\" style=\"height:160px\" data-km-srcdoc=\""));
        assert!(html.contains("data-msg-id=\"2\" sandbox=\"allow-same-origin allow-popups\" style=\"height:160px\" srcdoc=\""));
    }

    #[test]
    fn mixed_remote_image_permissions_isolate_blocked_card_with_meta_csp_and_stripping() {
        let msgs = vec![
            sample_thread_msg(
                1,
                Some("<p>Allowed <img src=\"https://trusted.example/a.png\"></p>"),
                "Allowed",
            ),
            sample_thread_msg(
                2,
                Some("<p>Blocked <img src=\"https://tracker.example/pixel.png\"></p>"),
                "Blocked",
            ),
        ];
        let detail = sample_detail(msgs.clone());
        let palette = ThreadPalette::for_dark(false);
        let inputs = vec![
            ThreadCardRenderInput {
                message: &msgs[0],
                is_current: false,
                is_collapsed: false,
                show_raw_source: false,
                block_remote: false,
                card_dark: false,
                card_pending: false,
                eml_exporting: false,
                auth_results: None,
                unsubscribe_url: None,
            },
            ThreadCardRenderInput {
                message: &msgs[1],
                is_current: true,
                is_collapsed: false,
                show_raw_source: false,
                block_remote: true,
                card_dark: false,
                card_pending: false,
                eml_exporting: false,
                auth_results: None,
                unsubscribe_url: None,
            },
        ];
        let ctx = ThreadDocContext {
            detail: &detail,
            cards: &inputs,
            palette: &palette,
            app_dark_mode: false,
            zoom_percent: 100,
            ui_text_scale_percent: 100,
            date_style: "relative",
            clock_style: "24h",
            show_remote_banner: true,
            always_show_recipients: false,
            card_attachments: true,
            demo_mode: false,
            read_receipt_policy: "ask",
            initial_anchor: Some((2, 40)),
        };
        let (bytes, any_remote) = build_thread_document(&ctx);
        let html = String::from_utf8(bytes).unwrap();
        assert!(any_remote);
        assert!(html.contains("https://trusted.example/a.png"));
        assert!(!html.contains("https://tracker.example/pixel.png"));
        assert!(html.contains("img-src data: http: https:;"));
        assert!(html.contains("img-src data:;"));
        assert!(html.contains("已拦截 1 张远程图片以保护隐私"));
        assert!(html.contains("data-initial-anchor-id=\"2\" data-initial-anchor-offset=\"40\""));
    }

    #[test]
    fn raw_source_unloaded_body_and_xss_escaping_are_handled_safely() {
        let mut msg1 = sample_thread_msg(1, Some("<script>alert(1)</script><p>Hi</p>"), "Plain");
        msg1.sender = "<img src=x onerror=alert(1)> <evil@example.com>".to_string();
        msg1.attachments.push(AttachmentMeta {
            id: 77,
            filename: "\"><script>x</script>.pdf".to_string(),
            mime_type: "application/pdf".to_string(),
            size_bytes: 2048,
            downloaded: false,
        });
        let mut msg2 = sample_thread_msg(2, Some("<p>Unread body</p>"), "");
        msg2.body_loaded = false;
        let msgs = vec![msg1, msg2];
        let mut detail = sample_detail(msgs.clone());
        detail.subject = "<script>subj</script>".to_string();
        let palette = ThreadPalette::for_dark(false);
        let inputs = vec![
            ThreadCardRenderInput {
                message: &msgs[0],
                is_current: false,
                is_collapsed: false,
                show_raw_source: true,
                block_remote: false,
                card_dark: false,
                card_pending: false,
                eml_exporting: false,
                auth_results: None,
                unsubscribe_url: None,
            },
            ThreadCardRenderInput {
                message: &msgs[1],
                is_current: true,
                is_collapsed: false,
                show_raw_source: false,
                block_remote: false,
                card_dark: false,
                card_pending: false,
                eml_exporting: false,
                auth_results: None,
                unsubscribe_url: None,
            },
        ];
        let ctx = ThreadDocContext {
            detail: &detail,
            cards: &inputs,
            palette: &palette,
            app_dark_mode: false,
            zoom_percent: 100,
            ui_text_scale_percent: 100,
            date_style: "relative",
            clock_style: "24h",
            show_remote_banner: true,
            always_show_recipients: false,
            card_attachments: true,
            demo_mode: false,
            read_receipt_policy: "ask",
            initial_anchor: None,
        };
        let (bytes, any_remote) = build_thread_document(&ctx);
        let html = String::from_utf8(bytes).unwrap();
        assert!(!any_remote);
        assert!(!html.contains("<script>"));
        assert!(html.contains("<pre class=\"km-raw\">"));
        assert!(html.contains("data-action=\"load-body\" data-msg-id=\"2\""));
        assert!(html.contains("data-action=\"save-att\" data-att-id=\"77\""));
    }

    #[test]
    fn build_card_frame_html_preserves_styles_and_tables_when_blocking_remote_images() {
        let raw_html = concat!(
            "<html><head><style>.mail-wrap { background: #fafafa; } table { width: 100%; }</style></head>",
            "<body><table class=\"mail-wrap\"><tr><td>",
            "<img src=\"https://tracker.example/open.png\">",
            "<img src=\"data:image/png;base64,AAA\">",
            "<p>排版正文</p>",
            "</td></tr></table></body></html>"
        );
        let msg = sample_thread_msg(1, Some(raw_html), "排版正文");
        let detail = sample_detail(vec![msg.clone()]);
        let inputs = vec![ThreadCardRenderInput {
            message: &msg,
            is_current: true,
            is_collapsed: false,
            show_raw_source: false,
            block_remote: true,
            card_dark: false,
            card_pending: false,
            eml_exporting: false,
            auth_results: None,
            unsubscribe_url: None,
        }];
        let palette = ThreadPalette::for_dark(false);
        let ctx = ThreadDocContext {
            detail: &detail,
            cards: &inputs,
            palette: &palette,
            app_dark_mode: false,
            zoom_percent: 100,
            ui_text_scale_percent: 100,
            date_style: "relative",
            clock_style: "24h",
            show_remote_banner: true,
            always_show_recipients: false,
            card_attachments: true,
            demo_mode: false,
            read_receipt_policy: "ask",
            initial_anchor: None,
        };
        let (bytes, any_remote) = build_thread_document(&ctx);
        let doc = String::from_utf8(bytes).unwrap();
        assert!(!any_remote);
        assert!(doc.contains("img-src data:;"));
        assert!(doc.contains("已拦截 1 张远程图片以保护隐私"));
        assert!(doc.contains(".mail-wrap { background: #fafafa; }"));
        assert!(doc.contains("&lt;table class=&quot;mail-wrap&quot;&gt;"));
        assert!(doc.contains("data:image/png;base64,AAA"));
        assert!(!doc.contains("tracker.example/open.png"));
    }
}
