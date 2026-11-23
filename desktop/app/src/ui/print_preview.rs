//! 邮件打印预览与 PDF 导出（WP-09，对齐 `src/ui/print_preview.rs`）：
//! - 生成带 `@page` 边距、信头摘要、净化 HTML 正文（含 `cid:` 内联数据图）与附件清单的打印文档；
//! - 在阅读器内提供纸张式预览浮层（支持 A4 / Letter、含信头、含附件、加宽左边距切换）；
//! - 支持「打印」（生成打印页并在浏览器/系统打印对话框打开）与「导出 PDF」（优先调用 Windows
//!   Edge 无头打印生成真实 PDF，不可用时回退保存打印 HTML）。

use std::path::Path;
use std::process::Command;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::text::TextView;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    black, div, hsla, px, Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled,
};
use html5ever::{parse_document, tendril::TendrilSink};
use hylki_mail_core::imap_client::plain_text_to_html;
use hylki_mail_core::{format_message_datetime, MessageDetail};
use markup5ever_rcdom::{Handle, NodeData, RcDom};

use crate::app::HylkiApp;
use crate::theme;

/// 打印预览状态（保存在 `HylkiApp` 中，关闭时清空）。
#[derive(Clone, Debug)]
pub(crate) struct PrintPreviewState {
    pub(crate) detail: MessageDetail,
    pub(crate) include_headers: bool,
    pub(crate) include_attachments: bool,
    pub(crate) wider_left_margin: bool,
    pub(crate) paper_a4: bool,
    pub(crate) status_note: Option<SharedString>,
}

impl PrintPreviewState {
    pub(crate) fn new(detail: MessageDetail) -> Self {
        Self {
            detail,
            include_headers: true,
            include_attachments: true,
            wider_left_margin: false,
            paper_a4: true,
            status_note: None,
        }
    }
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn safe_print_html(html: &str) -> String {
    fn emit(node: &Handle, output: &mut String, remaining_depth: usize) {
        if remaining_depth == 0 {
            return;
        }
        match &node.data {
            NodeData::Text { contents } => output.push_str(&escape_html(&contents.borrow())),
            NodeData::Element { name, attrs, .. } => {
                let tag = name.local.as_ref();
                if name.ns.as_ref() != "http://www.w3.org/1999/xhtml"
                    || matches!(
                        tag,
                        "head" | "script" | "style" | "iframe" | "object" | "template"
                    )
                {
                    return;
                }
                if tag == "img" {
                    if let Some(src) = attrs
                        .borrow()
                        .iter()
                        .find(|attr| attr.name.local.as_ref() == "src")
                        .map(|attr| attr.value.as_ref())
                        .filter(|src| crate::webview_host::decode_inline_mail_image(src).is_some())
                    {
                        output.push_str(&format!("<img src=\"{}\">", escape_html(src)));
                    }
                    return;
                }
                if tag == "a" {
                    let href = attrs
                        .borrow()
                        .iter()
                        .find(|attr| attr.name.local.as_ref() == "href")
                        .and_then(|attr| {
                            crate::webview_host::prepared_mail_link(&attr.value, false)
                        });
                    if let Some(ref href) = href {
                        output.push_str(&format!("<a href=\"{}\">", escape_html(href)));
                    }
                    for child in node.children.borrow().iter() {
                        emit(child, output, remaining_depth - 1);
                    }
                    if href.is_some() {
                        output.push_str("</a>");
                    }
                    return;
                }
                let keep = matches!(
                    tag,
                    "p" | "div"
                        | "span"
                        | "br"
                        | "hr"
                        | "blockquote"
                        | "pre"
                        | "code"
                        | "ul"
                        | "ol"
                        | "li"
                        | "table"
                        | "thead"
                        | "tbody"
                        | "tfoot"
                        | "tr"
                        | "th"
                        | "td"
                        | "strong"
                        | "b"
                        | "em"
                        | "i"
                        | "u"
                        | "s"
                        | "h1"
                        | "h2"
                        | "h3"
                        | "h4"
                        | "h5"
                        | "h6"
                );
                if keep {
                    output.push_str(&format!("<{tag}>"));
                }
                for child in node.children.borrow().iter() {
                    emit(child, output, remaining_depth - 1);
                }
                if keep && !matches!(tag, "br" | "hr") {
                    output.push_str(&format!("</{tag}>"));
                }
            }
            _ => {
                for child in node.children.borrow().iter() {
                    emit(child, output, remaining_depth - 1);
                }
            }
        }
    }
    let dom = parse_document(RcDom::default(), Default::default()).one(html);
    let mut result = String::new();
    emit(&dom.document, &mut result, 128);
    result
}

fn safe_print_body(detail: &MessageDetail) -> String {
    match detail
        .body_html
        .as_deref()
        .filter(|html| !html.trim().is_empty())
    {
        Some(html) => safe_print_html(html),
        None => plain_text_to_html(&detail.body_text),
    }
}

/// 生成独立可打印 HTML 文档（含 `@page` 纸张与边距样式）。
pub(crate) fn build_print_html(
    state: &PrintPreviewState,
    date_style: &str,
    clock_style: &str,
    auto_print_script: bool,
) -> String {
    let d = &state.detail;
    let formatted_date =
        format_message_datetime(d.date_ts, &d.date_display, date_style, clock_style);
    let page_size = if state.paper_a4 { "A4" } else { "Letter" };
    let left_mm = if state.wider_left_margin { 25 } else { 15 };

    let body_fragment = safe_print_body(d);

    let headers_html = if state.include_headers {
        format!(
            r#"<div class="headers">
  <h1>{subj}</h1>
  <div><strong>发件人：</strong>{from}</div>
  <div><strong>收件人：</strong>{to}</div>
  <div><strong>日期：</strong>{date}</div>
</div>"#,
            subj = escape_html(&d.subject),
            from = escape_html(&d.sender),
            to = escape_html(&d.recipients),
            date = escape_html(&formatted_date),
        )
    } else {
        String::new()
    };

    let attachments_html = if state.include_attachments && !d.attachments.is_empty() {
        let items: String = d
            .attachments
            .iter()
            .map(|a| {
                format!(
                    "<li>{} ({})</li>",
                    escape_html(&a.filename),
                    crate::ui::attachment_drawer::format_bytes(a.size_bytes)
                )
            })
            .collect();
        format!(
            r#"<div class="attachments"><strong>附件 ({})：</strong><ul>{items}</ul></div>"#,
            d.attachments.len()
        )
    } else {
        String::new()
    };

    let (script, script_policy) = if auto_print_script {
        (
            "<script>window.addEventListener('load', () => setTimeout(() => window.print(), 150));</script>",
            "'sha256-03cAtnQrar3eNxsFUHdrM0gX366FLCI7x6FnkYpr4bg='",
        )
    } else {
        ("", "'none'")
    };

    format!(
        r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data:; style-src 'unsafe-inline'; script-src {script_policy}; base-uri 'none'; form-action 'none'">
<title>{title}</title>
<style>
  @page {{ size: {page_size}; margin: 15mm 15mm 18mm {left_mm}mm; }}
  body {{ font-family: "Segoe UI", "Microsoft YaHei UI", sans-serif; color: #1f2328; line-height: 1.6; max-width: 780px; margin: 0 auto; padding: 16px; }}
  .headers {{ border-bottom: 1px solid #d0d7de; padding-bottom: 12px; margin-bottom: 16px; font-size: 13px; }}
  .headers h1 {{ font-size: 20px; margin: 0 0 8px 0; }}
  .body img {{ max-width: 100%; height: auto; }}
  .attachments {{ margin-top: 20px; padding-top: 12px; border-top: 1px solid #d0d7de; font-size: 12px; color: #57606a; }}
</style>
{script}
</head>
<body>
{headers_html}
<div class="body">{body_fragment}</div>
{attachments_html}
</body>
</html>"#,
        title = escape_html(&d.subject),
    )
}

/// 查找系统中的 Microsoft Edge 可执行文件，用于 `--print-to-pdf`。
fn find_msedge() -> Option<&'static str> {
    const CANDIDATES: [&str; 3] = [
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
        "msedge.exe",
    ];
    for c in CANDIDATES {
        if c == "msedge.exe" || Path::new(c).exists() {
            return Some(c);
        }
    }
    None
}

/// 将打印 HTML 导出为 PDF（通过 Edge 无头打印），返回状态提示。
pub(crate) fn export_print_pdf(
    state: &PrintPreviewState,
    dest_pdf: &Path,
    date_style: &str,
    clock_style: &str,
) -> Result<String, String> {
    let html = build_print_html(state, date_style, clock_style, false);
    let tmp_html = create_print_temp_file(&html)?;

    #[cfg(windows)]
    if let (Some(edge), Ok(file_url)) = (find_msedge(), url::Url::from_file_path(&tmp_html)) {
        let out_arg = format!("--print-to-pdf={}", dest_pdf.display());
        let status = Command::new(edge)
            .args([
                "--headless",
                "--disable-gpu",
                "--no-pdf-header-footer",
                &out_arg,
                file_url.as_str(),
            ])
            .status();
        if let Ok(s) = status {
            if s.success() && dest_pdf.exists() {
                let _ = std::fs::remove_file(&tmp_html);
                return Ok(format!("已导出 PDF：{}", dest_pdf.display()));
            }
        }
    }
    let _ = std::fs::remove_file(&tmp_html);

    // 回退：保存为可打印 HTML 并提示用户。
    let fallback_html = dest_pdf.with_extension("html");
    write_fallback_html(&fallback_html, &html)?;
    Ok(format!(
        "未检测到 Edge 无头打印，已导出为可打印网页：{}",
        fallback_html.display()
    ))
}

fn create_print_temp_file(html: &str) -> Result<std::path::PathBuf, String> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    loop {
        let path = std::env::temp_dir().join(format!(
            "hylki-print-{}-{}.html",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                if let Err(e) = file.write_all(html.as_bytes()) {
                    drop(file);
                    let _ = std::fs::remove_file(&path);
                    return Err(format!("写入临时打印文件失败: {e}"));
                }
                return Ok(path);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("创建临时打印文件失败: {e}")),
        }
    }
}

fn write_fallback_html(path: &Path, html: &str) -> Result<(), String> {
    use std::io::Write;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .and_then(|mut file| file.write_all(html.as_bytes()))
        .map_err(|e| format!("导出打印文件失败（{}）: {e}", path.display()))
}

/// 在后台线程打开自动触发 `window.print()` 的打印页面。
pub(crate) fn launch_system_print(
    state: &PrintPreviewState,
    date_style: &str,
    clock_style: &str,
) -> Result<(), String> {
    let html = build_print_html(state, date_style, clock_style, true);
    let tmp_html = create_print_temp_file(&html)?;
    #[cfg(target_os = "windows")]
    {
        use windows::core::{w, HSTRING};
        use windows::Win32::System::Com::{
            CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
        };
        use windows::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWDEFAULT};

        let path = HSTRING::from(tmp_html.as_os_str());
        let result = unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE)
                .ok()
                .map(|()| {
                    let result = ShellExecuteW(None, w!("open"), &path, None, None, SW_SHOWDEFAULT);
                    CoUninitialize();
                    result.0 as isize
                })
        };
        let error = match result {
            Ok(code) if code > 32 => return Ok(()),
            Ok(code) => format!("ShellExecuteW 返回 {code}"),
            Err(error) => error.to_string(),
        };
        let _ = std::fs::remove_file(&tmp_html);
        return Err(format!("打开系统打印页失败: {error}"));
    }
    #[cfg(not(windows))]
    Ok(())
}

/// 渲染打印预览模态浮层。
pub(crate) fn render_modal(
    state: &PrintPreviewState,
    date_style: &str,
    clock_style: &str,
    cx: &mut Context<HylkiApp>,
) -> gpui_kit::AnyElement {
    let p = theme::current(cx);
    let d = &state.detail;
    let formatted_date =
        format_message_datetime(d.date_ts, &d.date_display, date_style, clock_style);
    let body_html = safe_print_body(d);

    let app_for_link = cx.entity().downgrade();
    let paper_bg = hsla(0.0, 0.0, 1.0, 1.0);
    let paper_fg = hsla(0.0, 0.0, 0.13, 1.0);
    let paper_muted = hsla(0.0, 0.0, 0.42, 1.0);
    let paper_border = hsla(0.0, 0.0, 0.85, 1.0);

    div()
        .id("print-preview-backdrop")
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom_0()
        .bg(black().opacity(0.45))
        .flex()
        .items_center()
        .justify_center()
        .p_6()
        .on_click(cx.listener(|this, _, _, cx| {
            this.close_print_preview(cx);
        }))
        .child(
            v_flex()
                .id("print-preview-dialog")
                .w(px(820.0))
                .max_w_full()
                .h(px(680.0))
                .max_h_full()
                .rounded(px(12.0))
                .bg(p.surface_raised)
                .border_1()
                .border_color(p.border)
                .shadow_lg()
                .overflow_hidden()
                .on_click(|_, _, cx| cx.stop_propagation())
                // 顶部控制条。
                .child(
                    h_flex()
                        .px_4()
                        .py_2p5()
                        .border_b_1()
                        .border_color(p.border)
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(gpui_kit::FontWeight::BOLD)
                                        .text_color(p.text)
                                        .child("打印预览 / 导出 PDF"),
                                )
                                .child(
                                    Button::new("print-opt-headers")
                                        .ghost()
                                        .compact()
                                        .label(if state.include_headers {
                                            "✓ 含信头"
                                        } else {
                                            "含信头"
                                        })
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            if let Some(ref mut pp) = this.print_preview {
                                                pp.include_headers = !pp.include_headers;
                                                cx.notify();
                                            }
                                        })),
                                )
                                .child(
                                    Button::new("print-opt-att")
                                        .ghost()
                                        .compact()
                                        .label(if state.include_attachments {
                                            "✓ 含附件"
                                        } else {
                                            "含附件"
                                        })
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            if let Some(ref mut pp) = this.print_preview {
                                                pp.include_attachments = !pp.include_attachments;
                                                cx.notify();
                                            }
                                        })),
                                )
                                .child(
                                    Button::new("print-opt-paper")
                                        .ghost()
                                        .compact()
                                        .label(if state.paper_a4 { "A4" } else { "Letter" })
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            if let Some(ref mut pp) = this.print_preview {
                                                pp.paper_a4 = !pp.paper_a4;
                                                cx.notify();
                                            }
                                        })),
                                )
                                .child(
                                    Button::new("print-opt-margin")
                                        .ghost()
                                        .compact()
                                        .label(if state.wider_left_margin {
                                            "✓ 宽左边距"
                                        } else {
                                            "标准边距"
                                        })
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            if let Some(ref mut pp) = this.print_preview {
                                                pp.wider_left_margin = !pp.wider_left_margin;
                                                cx.notify();
                                            }
                                        })),
                                ),
                        )
                        .child(
                            h_flex()
                                .gap_1p5()
                                .items_center()
                                .child(
                                    Button::new("print-export-pdf")
                                        .outline()
                                        .compact()
                                        .icon(IconName::Download)
                                        .label("导出 PDF")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.export_print_preview_pdf(cx);
                                        })),
                                )
                                .child(
                                    Button::new("print-run-system")
                                        .primary()
                                        .compact()
                                        .icon(IconName::FileText)
                                        .label("打印")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.run_system_print(cx);
                                        })),
                                )
                                .child(
                                    Button::new("print-close")
                                        .ghost()
                                        .compact()
                                        .icon(IconName::X)
                                        .tooltip("关闭")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.close_print_preview(cx);
                                        })),
                                ),
                        ),
                )
                .when_some(state.status_note.clone(), |dlg, note| {
                    dlg.child(
                        div()
                            .px_4()
                            .py_1p5()
                            .bg(p.accent_soft())
                            .text_xs()
                            .text_color(p.text)
                            .child(note),
                    )
                })
                // 纸张预览画布。
                .child(
                    div()
                        .id("print-preview-canvas")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scrollbar()
                        .bg(p.canvas)
                        .p_6()
                        .flex()
                        .justify_center()
                        .child(
                            v_flex()
                                .w(px(if state.paper_a4 { 660.0 } else { 690.0 }))
                                .bg(paper_bg)
                                .text_color(paper_fg)
                                .rounded(px(4.0))
                                .border_1()
                                .border_color(paper_border)
                                .shadow_md()
                                .pt(px(32.0))
                                .pr(px(32.0))
                                .pb(px(36.0))
                                .pl(px(if state.wider_left_margin { 56.0 } else { 32.0 }))
                                .gap_3()
                                .when(state.include_headers, |paper| {
                                    paper.child(
                                        v_flex()
                                            .pb_3()
                                            .border_b_1()
                                            .border_color(paper_border)
                                            .gap_1()
                                            .child(
                                                div()
                                                    .text_size(px(18.0))
                                                    .font_weight(gpui_kit::FontWeight::BOLD)
                                                    .child(SharedString::from(d.subject.clone())),
                                            )
                                            .child(div().text_xs().text_color(paper_muted).child(
                                                SharedString::from(format!("发件人：{}", d.sender)),
                                            ))
                                            .child(div().text_xs().text_color(paper_muted).child(
                                                SharedString::from(format!(
                                                    "收件人：{}",
                                                    d.recipients
                                                )),
                                            ))
                                            .child(div().text_xs().text_color(paper_muted).child(
                                                SharedString::from(format!(
                                                    "日期：{formatted_date}"
                                                )),
                                            )),
                                    )
                                })
                                .child(
                                    div().text_size(px(14.0)).text_color(paper_fg).child(
                                        TextView::html(
                                            SharedString::from(format!(
                                                "print-preview-body-{}",
                                                d.id
                                            )),
                                            body_html,
                                        )
                                        .selectable(true)
                                        .on_link_click(
                                            move |url, event, _, cx| {
                                                HylkiApp::open_clicked_mail_link(
                                                    &app_for_link,
                                                    url,
                                                    event,
                                                    cx,
                                                );
                                            },
                                        ),
                                    ),
                                )
                                .when(
                                    state.include_attachments && !d.attachments.is_empty(),
                                    |paper| {
                                        let att_lines: Vec<String> = d
                                            .attachments
                                            .iter()
                                            .map(|a| {
                                                format!(
                                                    "• {} ({})",
                                                    a.filename,
                                                    crate::ui::attachment_drawer::format_bytes(
                                                        a.size_bytes
                                                    )
                                                )
                                            })
                                            .collect();
                                        paper.child(
                                            v_flex()
                                                .pt_3()
                                                .border_t_1()
                                                .border_color(paper_border)
                                                .gap_1()
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .font_weight(gpui_kit::FontWeight::BOLD)
                                                        .text_color(paper_muted)
                                                        .child(SharedString::from(format!(
                                                            "附件 ({})",
                                                            d.attachments.len()
                                                        ))),
                                                )
                                                .children(att_lines.into_iter().map(|line| {
                                                    div()
                                                        .text_xs()
                                                        .text_color(paper_muted)
                                                        .child(SharedString::from(line))
                                                })),
                                        )
                                    },
                                ),
                        ),
                ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_print_jobs_use_distinct_temp_files() {
        let first = create_print_temp_file("first").unwrap();
        let second = create_print_temp_file("second").unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "first");
        assert_eq!(std::fs::read_to_string(&second).unwrap(), "second");
        std::fs::remove_file(first).unwrap();
        std::fs::remove_file(second).unwrap();
    }

    #[test]
    fn fallback_html_does_not_overwrite_existing_file() {
        let path = std::env::temp_dir().join(format!(
            "hylki-fallback-test-{}-{}.html",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        write_fallback_html(&path, "first").unwrap();
        assert!(write_fallback_html(&path, "second").is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn print_html_preserves_safe_layout_and_inline_images() {
        let html = "<p>First <strong>important</strong></p><ul><li>Item</li></ul><table><tr><td>Cell</td></tr></table><img src=\"data:image/png;base64,iVBORw0KGgo=\">";
        let clean = safe_print_html(html);
        assert!(clean.contains("<p>First <strong>important</strong></p>"));
        assert!(clean.contains("<li>Item</li>"));
        assert!(clean.contains("<td>Cell</td>"));
        assert!(safe_print_html("<a href=\"https://example.com\">Link</a>")
            .contains("<a href=\"https://example.com/\">Link</a>"));
        assert!(!safe_print_html("<a href=\"file:///test\">Link</a>").contains("href"));
        assert!(clean.contains("data:image/png;base64,iVBORw0KGgo="));
    }

    #[test]
    fn print_html_drops_remote_resources_and_active_content() {
        let html = "<p>Visible</p><img src=\"https://example.com/image.png\"><script>ignored</script><div style=\"background:url(https://example.com/a)\">Text</div>";
        let clean = safe_print_html(html);
        assert!(clean.contains("<p>Visible</p>"));
        assert!(clean.contains("<div>Text</div>"));
        assert!(!clean.contains("example.com"));
        assert!(!clean.contains("ignored"));
    }

    #[test]
    fn print_html_accepts_html_only_body() {
        let clean = safe_print_html("<html><body><p>HTML only &amp; readable</p></body></html>");
        assert!(clean.contains("<p>HTML only &amp; readable</p>"));
    }
}
