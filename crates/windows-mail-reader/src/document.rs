//! Single-message HTML/plain-text document builders and plain CSS appearance types.

use kitemail_mail_core::imap_client::{plain_text_to_html, strip_remote_images_for_webview};

use crate::dark;

/// Default reader body background and foreground CSS hex colors.
pub fn reader_body_colors(dark: bool) -> (&'static str, &'static str) {
    if dark {
        ("#202124", "#e8eaed")
    } else {
        ("#ffffff", "#202124")
    }
}

/// Pure CSS-color appearance configuration for single-body and thread documents.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DocumentAppearance {
    pub canvas_bg: String,
    pub text_color: String,
    pub muted_color: String,
    pub border_color: String,
    pub accent_color: String,
    pub dark: bool,
    pub zoom_percent: u32,
    pub block_remote_images: bool,
}

impl DocumentAppearance {
    /// Build default appearance values for light or dark reader grounds.
    pub fn for_dark(dark: bool, zoom_percent: u32, block_remote_images: bool) -> Self {
        let (bg, fg) = reader_body_colors(dark);
        Self {
            canvas_bg: bg.to_string(),
            text_color: fg.to_string(),
            muted_color: if dark { "#a7a7a9" } else { "#707074" }.to_string(),
            border_color: if dark { "#454545" } else { "#d9d9d9" }.to_string(),
            accent_color: "#3584e4".to_string(),
            dark,
            zoom_percent,
            block_remote_images,
        }
    }
}

impl Default for DocumentAppearance {
    fn default() -> Self {
        Self::for_dark(false, 100, true)
    }
}

/// Whether raw HTML contains non-whitespace content suitable for WebView2 rendering.
pub fn supports_document(html: &str) -> bool {
    !html.trim().is_empty()
}

/// Find the byte offset of an opening HTML tag (`<head`, `<body`, etc.) in lowercase HTML.
pub fn find_html_tag(lower: &str, tag: &str) -> Option<usize> {
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

/// Escape text/HTML for safe attribute or text-node embedding.
pub fn escape_html(value: &str) -> String {
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

fn escape_srcdoc_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Build a sandboxed single-message HTML body document without an outer header.
pub fn build_body_document(html: &str, appearance: &DocumentAppearance) -> Vec<u8> {
    make_document(html, None, appearance)
}

/// Build a sandboxed single-message HTML document with an outer subject/sender header.
pub fn build_document(
    html: &str,
    subject: &str,
    sender: &str,
    appearance: &DocumentAppearance,
) -> Vec<u8> {
    make_document(html, Some((subject, sender)), appearance)
}

/// Build a sandboxed plain-text body document.
pub fn build_plain_text_document(text: &str, appearance: &DocumentAppearance) -> Vec<u8> {
    let html = plain_text_to_html(text);
    make_document(&html, None, appearance)
}

fn make_document(
    html: &str,
    header: Option<(&str, &str)>,
    appearance: &DocumentAppearance,
) -> Vec<u8> {
    let source_html = if appearance.block_remote_images {
        strip_remote_images_for_webview(html).0
    } else {
        html.to_string()
    };
    let html = if appearance.dark {
        dark::adapt_colors_for_dark(&source_html)
    } else {
        source_html
    };
    let html = dark::pin_color_scheme(&html, appearance.dark);
    let bg = appearance.canvas_bg.as_str();
    let fg = appearance.text_color.as_str();
    let scheme = if appearance.dark { "dark" } else { "light" };
    let style = format!(
        "<style>:root{{color-scheme:{scheme};zoom:{}%}}:where(body){{background-color:{bg};color:{fg}}}</style>",
        appearance.zoom_percent.clamp(50, 250)
    );
    let lower = html.to_ascii_lowercase();
    let insertion = find_html_tag(&lower, "<head")
        .or_else(|| lower.find("<!doctype"))
        .and_then(|start| lower[start..].find('>').map(|end| start + end + 1))
        .unwrap_or(0);
    let html = format!("{}{}{}", &html[..insertion], style, &html[insertion..]);
    let escaped = escape_srcdoc_attr(&html);
    let header = header
        .map(|(subject, sender)| {
            format!(
                "<header><h1>{}</h1><div>{}</div></header>",
                escape_srcdoc_attr(subject),
                escape_srcdoc_attr(sender)
            )
        })
        .unwrap_or_default();
    format!("<!doctype html><meta charset=utf-8><style>:root{{color-scheme:{scheme}}}html,body{{height:100%;margin:0}}body{{display:flex;flex-direction:column;font:14px system-ui;background:{bg};color:{fg}}}header{{padding:24px 24px 12px}}h1{{font-size:20px;margin:0 0 12px}}iframe{{flex:1;width:100%;border:0;min-height:0}}</style>{header}<iframe sandbox=\"allow-same-origin allow-popups\" srcdoc=\"{escaped}\"></iframe>").into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_preserves_markup_inside_an_escaped_sandboxed_frame() {
        let light = DocumentAppearance::for_dark(false, 100, false);
        let document = String::from_utf8(build_document(
            "<style>p{color:red}</style><p title=\"x\">A & B</p>",
            "<subject>",
            "sender & co",
            &light,
        ))
        .unwrap();
        assert!(document.contains("<iframe sandbox=\"allow-same-origin allow-popups\" srcdoc=\""));
        assert!(document.contains("&lt;style&gt;p{color:red}&lt;/style&gt;"));
        assert!(document.contains("title=&quot;x&quot;"));
        assert!(document.contains("&lt;subject&gt;"));
        assert!(document.contains("sender &amp; co"));
        assert!(document.contains("background:#ffffff;color:#202124"));
        assert!(document.contains(":where(body){background-color:#ffffff;color:#202124}"));
        assert!(supports_document(
            "<p><a href='https://example.com'>link</a></p>"
        ));
        assert!(supports_document(
            "<a target='_blank' href='https://example.com'>link</a>"
        ));
        assert!(supports_document("<blockquote>quote</blockquote>"));
        assert!(supports_document("<div class='gmail_quote'>quote</div>"));
        assert!(supports_document("<a href='mailto:a@example.com'>mail</a>"));
        assert!(!supports_document(""));
        assert!(!supports_document("   \n\t "));

        let dark_appearance = DocumentAppearance::for_dark(true, 150, false);
        let dark = String::from_utf8(build_body_document(
            "<!doctype html><html><head><style>p{color:#000}</style></head><body><p>x</p></body></html>",
            &dark_appearance,
        ))
        .unwrap();
        assert!(dark.contains("color-scheme:dark;zoom:150%"));
        assert!(dark.contains(":where(body){background-color:#202124;color:#e8eaed}"));
        assert!(dark.contains("&lt;!doctype html&gt;&lt;html&gt;&lt;head&gt;&lt;style&gt;"));

        let clamped_appearance = DocumentAppearance {
            zoom_percent: 999,
            ..light
        };
        let clamped =
            String::from_utf8(build_body_document("<p>x</p>", &clamped_appearance)).unwrap();
        assert!(clamped.contains("zoom:250%"));
    }

    #[test]
    fn make_document_does_not_insert_style_inside_header_element() {
        let light = DocumentAppearance::for_dark(false, 100, false);
        let document = String::from_utf8(build_body_document(
            "<header><h1>Title</h1></header><p>Body</p>",
            &light,
        ))
        .unwrap();
        assert!(document.contains("srcdoc=\"&lt;style&gt;:root{color-scheme:light;zoom:100%}"));
        assert!(document
            .contains("&lt;/style&gt;&lt;header&gt;&lt;h1&gt;Title&lt;/h1&gt;&lt;/header&gt;"));
    }

    #[test]
    fn build_body_document_omits_header() {
        let light = DocumentAppearance::for_dark(false, 100, true);
        let document = String::from_utf8(build_body_document("<p>正文内容</p>", &light)).unwrap();
        assert!(!document.contains("<header>"));
        assert!(document.contains("<iframe"));
    }

    #[test]
    fn build_body_document_strips_remote_images_when_blocked() {
        let blocked = DocumentAppearance::for_dark(false, 100, true);
        let doc = String::from_utf8(build_body_document(
            "<p>Hello <img src=\"https://tracker.example/pixel.png\"><img src=\"data:image/png;base64,AAA\"></p>",
            &blocked,
        ))
        .unwrap();
        assert!(!doc.contains("tracker.example/pixel.png"));
        assert!(doc.contains("data:image/png;base64,AAA"));
    }
}
