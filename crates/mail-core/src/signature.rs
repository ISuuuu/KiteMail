//! 邮件签名：外部 HTML / 纯文本的清洗，以及按写信正文格式给出可插入的文本。
//!
//! 对齐 Linux 原版 `src/ui/rich_editor/signature.rs` 与 `src/ui/compose/signature.rs`：
//! - 纯文本签名转义后换行变 `<br>`；
//! - 外部 HTML（导入的文件、粘贴的源码）过白名单清洗：脚本、样式块、事件属性、
//!   危险协议与内联样式一律丢掉，表格保留；
//! - 导入目录内的本地图片内联成 `data:` URI，远程与目录外图片不自动加载；
//! - 插入正文时 HTML 正文给 `<div class="hylki-sig">` 块，Markdown / 纯文本正文
//!   先转换到对应表示，避免把标签当文字发出去。

use std::path::Path;

use base64::Engine as _;

use crate::markup::{self, ComposeFormat};

/// 清洗后保留的标签（对齐原版 ammonia 白名单，外加常见行内/块级标签）。
const ALLOWED_TAGS: &[&str] = &[
    "a",
    "b",
    "big",
    "blockquote",
    "br",
    "caption",
    "center",
    "code",
    "col",
    "colgroup",
    "dd",
    "del",
    "div",
    "dl",
    "dt",
    "em",
    "font",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "i",
    "img",
    "ins",
    "li",
    "ol",
    "p",
    "pre",
    "s",
    "small",
    "span",
    "strike",
    "strong",
    "sub",
    "sup",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "tr",
    "tt",
    "u",
    "ul",
];

/// 整块丢弃（连同内容）的标签：能执行、能重排版式的东西。
const DROP_WITH_CONTENT: &[&str] = &[
    "script", "style", "head", "iframe", "object", "embed", "noscript", "template", "svg", "math",
];

/// 任何标签上都可以保留的属性（原版 `.add_generic_attributes`）。
const GENERIC_ATTRS: &[&str] = &[
    "style",
    "class",
    "title",
    "dir",
    "lang",
    "width",
    "height",
    "align",
    "valign",
    "bgcolor",
    "border",
    "cellpadding",
    "cellspacing",
    "color",
    "face",
    "size",
];

const A_ATTRS: &[&str] = &["href", "title", "target"];
const IMG_ATTRS: &[&str] = &["src", "alt", "width", "height", "border"];

/// 本地图片的大小上限，超过就保持原样不内联。
const MAX_IMAGE_BYTES: u64 = 8 * 1024 * 1024;

/// 一段签名的 HTML 表示：纯文本转义并把换行变成 `<br>`，HTML 原样返回。
pub fn signature_to_html(sig: &str, is_html: bool) -> String {
    if is_html {
        sig.to_string()
    } else {
        escape_text(sig)
    }
}

/// 外部来源（导入的 HTML / 文本文件，或粘贴进源码框的内容）转成签名 HTML：
/// 看起来像 HTML 就清洗，否则按纯文本转义。
pub fn signature_from_source(source: &str, base: Option<&Path>) -> String {
    if !markup::looks_like_html(source) {
        return signature_to_html(source.trim_end(), false);
    }
    sanitize_with_base(source, base)
}

/// 清洗外部 HTML 签名；本地图片内联成 `data:` URI。
pub fn sanitize_signature_html(html: &str) -> String {
    sanitize_with_base(html, None)
}

/// 签名块（HTML），带 `-- ` 分隔符与 `.hylki-sig` 包裹，与原版一致。
pub fn signature_block_html(sig: &str, is_html: bool, dashes: bool) -> String {
    let body = signature_to_html(sig, is_html);
    if body.trim().is_empty() {
        return String::new();
    }
    let dashes = if dashes { "-- <br>" } else { "" };
    format!("<div class=\"kitemail-sig\"><br>{dashes}{body}</div>")
}

/// 按正文格式给出可直接放进编辑区的签名文本；空签名返回空串。
///
/// - HTML 正文：原版同款 `<div class="hylki-sig">` 块（含真实标签与样式）；
/// - Markdown / 富文本 / 纯文本正文：HTML 签名先转换到该表示（桌面端编辑区是
///   多行文本框，原样塞标签只会被当成文字），纯文本签名保持原样。
pub fn signature_for(format: ComposeFormat, sig: &str, is_html: bool, dashes: bool) -> String {
    if sig.trim().is_empty() {
        return String::new();
    }
    if format == ComposeFormat::Html {
        return signature_block_html(sig, is_html, dashes);
    }
    if !is_html {
        let separator = if dashes { "-- \n" } else { "" };
        return format!("{separator}{}", sig.trim_end());
    }
    let html = signature_to_html(sig, true);
    let text = match format {
        ComposeFormat::Plain => markup::html_to_plain(&html),
        _ => markup::html_to_markdown(&html),
    };
    let text = text.trim_end();
    if text.trim().is_empty() {
        return String::new();
    }
    let separator = if dashes { "-- \n" } else { "" };
    format!("{separator}{text}")
}

/// 纯文本 → 转义后的 HTML，换行变 `<br>`。
fn escape_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("<br>"),
            _ => out.push(ch),
        }
    }
    out
}

/// 清洗 HTML；`base` 用来解析相对路径的本地图片。
fn sanitize_with_base(html: &str, base: Option<&Path>) -> String {
    let mut out = String::with_capacity(html.len());
    let mut tag = String::new();
    let mut in_tag = false;
    let mut skip: Option<String> = None;

    for ch in html.chars() {
        if !in_tag {
            if ch == '<' {
                in_tag = true;
                tag.clear();
            } else if skip.is_none() {
                out.push(ch);
            }
            continue;
        }
        if ch != '>' {
            tag.push(ch);
            continue;
        }
        in_tag = false;
        let raw = tag.trim();
        if raw.is_empty() || raw.starts_with('!') || raw.starts_with('?') {
            // 注释、DOCTYPE、处理指令：丢掉。
            continue;
        }
        let parsed = parse_tag(raw);
        let name = parsed.name.as_str();

        if let Some(block) = skip.as_deref() {
            if parsed.closing && name == block {
                skip = None;
            }
            continue;
        }
        if !parsed.closing && DROP_WITH_CONTENT.contains(&name) {
            skip = Some(name.to_string());
            continue;
        }
        if !ALLOWED_TAGS.contains(&name) {
            // 不认识的标签只丢标签本身，内容留着。
            continue;
        }
        if parsed.closing {
            out.push_str("</");
            out.push_str(name);
            out.push('>');
            continue;
        }
        out.push('<');
        out.push_str(name);
        for (key, value) in &parsed.attrs {
            let key = key.to_ascii_lowercase();
            if key.starts_with("on") || key == "style" {
                continue;
            }
            let allowed = GENERIC_ATTRS.contains(&key.as_str())
                || (name == "a" && A_ATTRS.contains(&key.as_str()))
                || (name == "img" && IMG_ATTRS.contains(&key.as_str()));
            if !allowed {
                continue;
            }
            let value = if key == "src" {
                let Some(value) = local_image_data_uri(value, base).or_else(|| {
                    value
                        .trim()
                        .to_ascii_lowercase()
                        .starts_with("data:image/")
                        .then(|| value.clone())
                }) else {
                    continue;
                };
                value
            } else {
                value.clone()
            };
            if (key == "href" || key == "src" || key == "style") && !value_allowed(&key, &value) {
                continue;
            }
            out.push(' ');
            out.push_str(&key);
            out.push_str("=\"");
            push_attr_value(&mut out, &value);
            out.push('"');
        }
        out.push('>');
    }
    out.trim().to_string()
}

/// 属性值是否可以留着：URL 类属性的协议白名单，`style` 里不许出现脚本协议。
fn value_allowed(key: &str, value: &str) -> bool {
    match key {
        "href" | "src" => url_is_safe(value),
        "style" => {
            let lower = value.to_ascii_lowercase();
            !lower.contains("url(")
                && !lower.contains("@import")
                && !lower.contains("javascript:")
                && !lower.contains("vbscript:")
                && !lower.contains("expression(")
        }
        _ => true,
    }
}

/// 只放行原版允许的协议；相对地址（没有协议）照常放行。
fn url_is_safe(value: &str) -> bool {
    let compact: String = value
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_control())
        .collect();
    let lower = compact.to_ascii_lowercase();
    let Some(colon) = lower.find(':') else {
        return true;
    };
    let scheme = &lower[..colon];
    if scheme.contains('/') || scheme.contains('?') || scheme.contains('#') {
        return true;
    }
    match scheme {
        "http" | "https" | "mailto" | "tel" | "cid" => true,
        "data" => {
            let rest = &lower[colon + 1..];
            rest.starts_with("image/png")
                || rest.starts_with("image/jpeg")
                || rest.starts_with("image/gif")
                || rest.starts_with("image/webp")
                || rest.starts_with("image/bmp")
                || rest.starts_with("image/avif")
        }
        _ => false,
    }
}

/// `src` 指向本地图片文件时读成 `data:` URI；URL 与读不到的路径返回 `None`。
fn local_image_data_uri(value: &str, base: Option<&Path>) -> Option<String> {
    let v = value.trim();
    if v.is_empty() || has_url_scheme(v) {
        return None;
    }
    if v.starts_with('/') || v.starts_with('\\') || v.contains(':') {
        return None;
    }
    let base = base?;
    if base.to_string_lossy().starts_with("\\\\") {
        return None;
    }
    let base = base.canonicalize().ok()?;
    let mut candidate = base.clone();
    for component in Path::new(v).components() {
        match component {
            std::path::Component::Normal(part) => candidate.push(part),
            _ => return None,
        }
        if std::fs::symlink_metadata(&candidate)
            .ok()?
            .file_type()
            .is_symlink()
        {
            return None;
        }
    }
    let path = candidate.canonicalize().ok()?;
    if !path.starts_with(&base) {
        return None;
    }
    let mime = match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "avif" => "image/avif",
        _ => return None,
    };
    let meta = std::fs::metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > MAX_IMAGE_BYTES {
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    Some(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    ))
}

/// 是否带协议头（`http://`、`data:`、`cid:` …）；Windows 盘符 `C:\` 不算。
fn has_url_scheme(value: &str) -> bool {
    let Some(colon) = value.find(':') else {
        return false;
    };
    let scheme = &value[..colon];
    if scheme.is_empty() || scheme.contains('/') || scheme.contains('\\') {
        return false;
    }
    if scheme.len() == 1 && scheme.chars().all(|c| c.is_ascii_alphabetic()) {
        let rest = &value[colon + 1..];
        if rest.starts_with('/') || rest.starts_with('\\') {
            return false;
        }
    }
    true
}

/// 把属性值写进输出：只断开引号与尖括号，已存在的实体保持原样。
fn push_attr_value(out: &mut String, value: &str) {
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
}

struct ParsedTag {
    closing: bool,
    name: String,
    attrs: Vec<(String, String)>,
}

/// 把 `<...>` 内部拆成标签名与属性；解析不了的片段按原样丢弃。
fn parse_tag(raw: &str) -> ParsedTag {
    let mut rest = raw.trim();
    let closing = rest.starts_with('/');
    if closing {
        rest = rest[1..].trim_start();
    }
    let name: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    let mut attrs = Vec::new();
    let chars: Vec<char> = rest[name.len()..].chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        while i < chars.len() && (chars[i].is_whitespace() || chars[i] == '/') {
            i += 1;
        }
        if i >= chars.len() {
            break;
        }
        let start = i;
        while i < chars.len() && !chars[i].is_whitespace() && chars[i] != '=' && chars[i] != '/' {
            i += 1;
        }
        let key: String = chars[start..i].iter().collect();
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if i < chars.len() && chars[i] == '=' {
            i += 1;
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            if i < chars.len() && (chars[i] == '"' || chars[i] == '\'') {
                let quote = chars[i];
                i += 1;
                while i < chars.len() && chars[i] != quote {
                    value.push(chars[i]);
                    i += 1;
                }
                if i < chars.len() {
                    i += 1;
                }
            } else {
                while i < chars.len() && !chars[i].is_whitespace() && chars[i] != '/' {
                    value.push(chars[i]);
                    i += 1;
                }
            }
        }
        if !key.is_empty() {
            attrs.push((key, value));
        }
    }
    ParsedTag {
        closing,
        name: name.to_ascii_lowercase(),
        attrs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_escaped_like_a_typed_signature() {
        assert_eq!(
            signature_to_html("Ada & Co\nCEO", false),
            "Ada &amp; Co<br>CEO"
        );
        assert_eq!(
            signature_from_source("Ada & Co\nCEO\n", None),
            "Ada &amp; Co<br>CEO"
        );
    }

    #[test]
    fn html_keeps_structure_and_styles_but_not_scripts() {
        let src = "<html><head><style>body{color:red}</style></head><body>\
                   <table style=\"border:0\"><tr><td style=\"color:#333\" onclick=\"x()\">\
                   <b>Ada</b> <a href=\"https://example.com\">site</a></td></tr></table>\
                   <script>alert(1)</script></body></html>";
        let out = sanitize_signature_html(src);
        assert!(out.contains("<table>"), "{out}");
        assert!(out.contains("<td>"), "{out}");
        assert!(
            out.contains("<a href=\"https://example.com\">site</a>"),
            "{out}"
        );
        assert!(!out.contains("onclick"), "{out}");
        assert!(!out.contains("script"), "{out}");
        assert!(!out.contains("color:red"), "{out}");
    }

    #[test]
    fn dangerous_urls_and_styles_are_dropped() {
        let out = sanitize_signature_html(
            "<a href=\"javascript:alert(1)\">x</a>\
             <a href=\"https://example.com\">y</a>\
             <img src=\"data:text/html;base64,AAA\">\
             <p style=\"background:url(javascript:alert(1))\">z</p>",
        );
        assert!(!out.contains("javascript"), "{out}");
        assert!(out.contains("href=\"https://example.com\""), "{out}");
        assert!(!out.contains("text/html"), "{out}");
        assert!(!out.contains("background"), "{out}");
    }

    #[test]
    fn local_images_are_embedded_and_remote_ones_blocked() {
        let dir = std::env::temp_dir().join(format!("hylki-sig-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("logo.png"), [0x89, b'P', b'N', b'G']).unwrap();
        let src = "<p><img src=\"logo.png\" width=\"80\"> \
                   <img src=\"https://x.example/a.png\"> <img src=\"missing.png\"></p>";
        let out = signature_from_source(src, Some(&dir));
        std::fs::remove_dir_all(&dir).ok();
        assert!(out.contains("src=\"data:image/png;base64,"), "{out}");
        assert!(out.contains("width=\"80\""), "{out}");
        assert!(!out.contains("src=\"https://x.example/a.png\""), "{out}");
        assert!(!out.contains("src=\"missing.png\""), "{out}");
        let escaped = signature_from_source(
            "<img src=\"../outside.png\"><p style=\"background:url(https://evil.test/x)\">x</p>",
            Some(&dir),
        );
        assert!(!escaped.contains("src="));
        assert!(!escaped.contains("style="));
    }

    #[test]
    fn html_signature_lands_in_html_body_under_dashes() {
        let out = signature_for(ComposeFormat::Html, "<p><b>Ada</b></p>", true, true);
        assert_eq!(
            out,
            "<div class=\"kitemail-sig\"><br>-- <br><p><b>Ada</b></p></div>"
        );
        assert_eq!(
            signature_for(ComposeFormat::Html, "<p>Ada</p>", true, false),
            "<div class=\"kitemail-sig\"><br><p>Ada</p></div>"
        );
    }

    #[test]
    fn html_signature_is_converted_for_text_bodies() {
        let sig = "<p><strong>Ada</strong></p>";
        let md = signature_for(ComposeFormat::Markdown, sig, true, true);
        assert!(md.starts_with("-- \n"), "{md}");
        assert!(md.contains("**Ada**"), "{md}");

        let plain = signature_for(ComposeFormat::Plain, sig, true, false);
        assert_eq!(plain.trim(), "Ada");

        // 纯文本签名保持原样，不会因为换行被当成 HTML。
        let text = signature_for(ComposeFormat::Plain, "张三\n研发部", false, true);
        assert_eq!(text, "-- \n张三\n研发部");
    }
}
