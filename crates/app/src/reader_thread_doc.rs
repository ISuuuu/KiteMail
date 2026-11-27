//! GPUI 适配器：将 [`HylkiPalette`] 转为纯 CSS 色板并委托给
//! `kitemail-windows-mail-reader::build_thread_document`。

use kitemail_mail_core::MessageDetail;
pub(crate) use kitemail_windows_mail_reader::ThreadCardRenderInput;
use kitemail_windows_mail_reader::ThreadPalette;

use crate::theme::{css_hex_color, HylkiPalette};

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

fn hsla_rgba_css(color: gpui_kit::Hsla, alpha_override: Option<f32>) -> String {
    let rgb = gpui_kit::Rgba::from(color);
    let r = (rgb.r * 255.0).round().clamp(0.0, 255.0) as u8;
    let g = (rgb.g * 255.0).round().clamp(0.0, 255.0) as u8;
    let b = (rgb.b * 255.0).round().clamp(0.0, 255.0) as u8;
    let a = alpha_override.unwrap_or(rgb.a).clamp(0.0, 1.0);
    format!("rgba({r},{g},{b},{a:.2})")
}

#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn build_thread_document(ctx: &ThreadDocContext<'_>) -> (Vec<u8>, bool) {
    let p = ctx.palette;
    let palette = ThreadPalette {
        canvas: css_hex_color(p.canvas),
        surface_raised: css_hex_color(p.surface_raised),
        border: css_hex_color(p.border),
        text: css_hex_color(p.text),
        text_muted: css_hex_color(p.text_muted),
        accent: css_hex_color(p.accent),
        accent_border: hsla_rgba_css(p.accent, Some(0.45)),
        accent_soft: hsla_rgba_css(p.accent, Some(0.12)),
        danger: css_hex_color(p.danger),
        danger_border: hsla_rgba_css(p.danger, Some(0.45)),
        danger_bg: hsla_rgba_css(p.danger, Some(0.10)),
    };
    let shared_ctx = kitemail_windows_mail_reader::ThreadDocContext {
        detail: ctx.detail,
        cards: ctx.cards,
        palette: &palette,
        app_dark_mode: ctx.app_dark_mode,
        zoom_percent: ctx.zoom_percent,
        ui_text_scale_percent: ctx.ui_text_scale_percent,
        date_style: ctx.date_style,
        clock_style: ctx.clock_style,
        show_remote_banner: ctx.show_remote_banner,
        always_show_recipients: ctx.always_show_recipients,
        card_attachments: ctx.card_attachments,
        demo_mode: ctx.demo_mode,
        read_receipt_policy: ctx.read_receipt_policy,
        initial_anchor: ctx.initial_anchor,
    };
    kitemail_windows_mail_reader::build_thread_document(&shared_ctx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kitemail_mail_core::ThreadMessage;

    #[test]
    fn gpui_adapter_converts_palette_and_builds_thread_document() {
        let msg = ThreadMessage {
            id: 1,
            rfc_message_id: "<m1@example.com>".to_string(),
            subject: "Re: Test".to_string(),
            sender: "Alice <alice@example.com>".to_string(),
            recipients: "bob@example.com".to_string(),
            cc: String::new(),
            date_ts: 1_700_000_000,
            date_display: "10:00".to_string(),
            body_text: "Hello".to_string(),
            body_html: Some("<p>Hello</p>".to_string()),
            body_loaded: true,
            unread: false,
            flagged: false,
            attachments: Vec::new(),
            uid: 1,
            folder_path: "INBOX".to_string(),
        };
        let detail = MessageDetail {
            id: 1,
            account_id: 1,
            folder_id: 1,
            folder_path: "INBOX".to_string(),
            uid: 1,
            rfc_message_id: "<m1@example.com>".to_string(),
            sender: "Alice <alice@example.com>".to_string(),
            recipients: "bob@example.com".to_string(),
            cc: String::new(),
            subject: "Re: Test".to_string(),
            date_ts: 1_700_000_000,
            date_display: "10:00".to_string(),
            body_text: "Hello".to_string(),
            body_html: Some("<p>Hello</p>".to_string()),
            flagged: false,
            attachments: Vec::new(),
            thread_messages: vec![msg.clone()],
            unsubscribe_url: None,
            unsubscribe_one_click: false,
            auth_results: None,
        };
        let palette = crate::theme::palette_for("system", false);
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
        assert!(html.contains("--km-canvas:#fafafa"));
        assert!(html.contains("--km-accent:#3584e4"));
    }
}
