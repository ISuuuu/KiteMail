//! 单 WebView2 会话线程容器文档构建器（对齐 Linux 版 `src/ui/message_view/document.rs`）：
//!
//! 将多封聚合会话的全部卡片（头、徽章、横幅、正文、附件芯片）拼装成同一份 HTML 文档，
//! 每封邮件正文放进独立的 `<iframe sandbox="allow-same-origin allow-popups">` 内：
//! - **零脚本邮件隔离**：外层与子 iframe 均禁用邮件内 JS；外层仅在加载完成后由宿主
//!   `ExecuteScript` 注入受信桥接脚本 `reader_thread.js`。
//! - **逐卡远程图双层防线**：整份文档仅在存在至少一张允许远程图的卡片时才在 HTTP CSP
//!   放开 `http: https:`；所有禁止远程图的卡片既先剥离外链 `<img>`，又在其 `srcdoc`
//!   内部注入收紧版 `<meta http-equiv="Content-Security-Policy" content="... img-src data: ...">`。
//! - **折叠卡片延迟水合**：初始处于 `km-folded` 的 iframe 放在模板中，正文写进 `data-km-srcdoc`，
//!   不写入 `srcdoc`，只有用户或工具栏展开时才由 JS 水合，避免长会话内存暴涨。

use std::fmt::Write as _;

use kitemail_mail_core::config;
use kitemail_mail_core::imap_client::{plain_text_to_html, strip_remote_images_for_webview};
use kitemail_mail_core::verify::{check_sender_summary, SenderTrust};
use kitemail_mail_core::{MessageDetail, ThreadMessage};

use crate::app::split_sender;
use crate::theme::{self, css_hex_color, HylkiPalette};
use crate::ui::avatar::avatar_badge_spec;
use crate::ui::reader::{first_snippet_line, format_raw_source, recipient_list};
use crate::webview_dark::{adapt_colors_for_dark, pin_color_scheme};

pub(crate) struct ThreadCardRenderInput<'a> {
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

pub(crate) struct ThreadDocContext<'a> {
    pub detail: &'a MessageDetail,
    pub cards: &'a [ThreadCardRenderInput<'a>],
    pub palette: &'a HylkiPalette,
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

fn escape_html(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

fn hsla_rgba_css(color: gpui_kit::Hsla, alpha_override: Option<f32>) -> String {
    let rgb = gpui_kit::Rgba::from(color);
    let r = (rgb.r * 255.0).round().clamp(0.0, 255.0) as u8;
    let g = (rgb.g * 255.0).round().clamp(0.0, 255.0) as u8;
    let b = (rgb.b * 255.0).round().clamp(0.0, 255.0) as u8;
    let a = alpha_override.unwrap_or(rgb.a).clamp(0.0, 1.0);
    format!("rgba({r},{g},{b},{a:.2})")
}

fn find_html_tag(lower: &str, tag: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(at) = lower[from..].find(tag).map(|i| from + i) {
        let next = lower
            .as_bytes()
            .get(at + tag.len())
            .copied()
            .unwrap_or(b'>');
        if next == b'>' || next == b'/' || next.is_ascii_whitespace() {
            return Some(at);
        }
        from = at + tag.len();
    }
    None
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
    let (body_bg, body_fg) = theme::reader_body_colors(card_dark);
    let bg = css_hex_color(body_bg);
    let fg = css_hex_color(body_fg);
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
    let (card_bg_hsla, _) = theme::reader_body_colors(ctx.app_dark_mode);
    let canvas = css_hex_color(p.canvas);
    let card_bg = css_hex_color(card_bg_hsla);
    let raised = css_hex_color(p.surface_raised);
    let border = css_hex_color(p.border);
    let text = css_hex_color(p.text);
    let muted = css_hex_color(p.text_muted);
    let accent = css_hex_color(p.accent);
    let accent_border = hsla_rgba_css(p.accent, Some(0.45));
    let accent_soft = hsla_rgba_css(p.accent, Some(0.12));
    let danger = css_hex_color(p.danger);
    let danger_border = hsla_rgba_css(p.danger, Some(0.45));
    let danger_bg = hsla_rgba_css(p.danger, Some(0.10));
    let ui_scale = config::clamp_ui_text_scale(ctx.ui_text_scale_percent) as f32 / 100.0;
    let base_px = 14.0 * ui_scale;
    let small_px = 12.0 * ui_scale;
    let zoom_scale = ctx.zoom_percent.clamp(50, 250) as f32 / 100.0;
    let raw_font_px = (12.0 * zoom_scale).max(10.0);
    let star_hex = format!("#{:06x}", theme::STAR_GOLD);
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

#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn build_thread_document(ctx: &ThreadDocContext<'_>) -> (Vec<u8>, bool) {
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
            let (picked_bg, _) = theme::reader_body_colors(card.card_dark);
            let picked_class = if card.card_dark != ctx.app_dark_mode {
                " km-picked"
            } else {
                ""
            };
            let pan_style = format!(" style=\"background:{}\"", css_hex_color(picked_bg));
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

        // Header
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

        // Details container (hidden when folded)
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
        let palette = crate::theme::palette_for("system", false);
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
        let palette = crate::theme::palette_for("system", false);
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
        let palette = crate::theme::palette_for("system", false);
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
        let palette = crate::theme::palette_for("system", false);
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
