//! 写信 / 回复面板（方案文档第三节页面 3）：
//! - 顶部操作栏：标题（写新邮件 / 回复邮件）+ 发件账号胶囊切换（多账号时）
//!   + 右侧「存草稿 / 添加附件 / 取消 / 发送」按钮组。
//! - 信头表单区（细底边框分隔、紧凑对齐）：收件人行右侧「抄送 / 密送」小按钮，
//!   默认折叠，展开后显示 Cc / Bcc 输入行；主题输入行。
//! - 附件芯片条：圆角胶囊标签（📎 文件名 (大小 KB) ×，点击移除）。
//! - 正文编辑区：`TextareaState` 多行编辑器，`flex-1` 撑满剩余高度。

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState, Textarea};
use gpui_kit::component::{h_flex, v_flex, Selectable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{div, px, Context, Entity, FontWeight, IntoElement, ParentElement, Styled};

use crate::app::HylkiApp;
use crate::theme;

/// 信头行标签宽度。
const LABEL_WIDTH: f32 = 56.0;

/// 单行信头：左侧标签 + 输入框（细底边框分隔）。
fn header_row(
    label: &'static str,
    input: &Entity<InputState>,
    p: &theme::HylkiPalette,
) -> gpui_kit::Div {
    h_flex()
        .items_center()
        .gap_2()
        .border_b_1()
        .border_color(p.border)
        .py_1()
        .child(
            div()
                .w(px(LABEL_WIDTH))
                .flex_shrink_0()
                .text_sm()
                .text_color(p.text_muted)
                .child(label),
        )
        .child(div().flex_1().min_w_0().child(Input::new(input)))
}

/// 附件芯片（圆角胶囊：📎 文件名 (大小) ×，点击移除）。
fn attachment_chip(idx: usize, path: &str, cx: &mut Context<HylkiApp>) -> Button {
    let name = std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path)
        .to_string();
    let size_label = std::fs::metadata(path)
        .map(|m| m.len())
        .map(|bytes| format!("{} KB", (bytes + 1023) / 1024))
        .unwrap_or_default();
    let label = if size_label.is_empty() {
        format!("📎 {name} ×")
    } else {
        format!("📎 {name} ({size_label}) ×")
    };
    Button::new(("remove-att", idx))
        .outline()
        .label(label)
        .rounded(px(9999.0))
        .text_size(px(12.0))
        .on_click(cx.listener(move |this, _, _, cx| {
            if idx < this.compose_attachments.len() {
                this.compose_attachments.remove(idx);
                cx.notify();
            }
        }))
}

pub(crate) fn render(app: &HylkiApp, cx: &mut Context<HylkiApp>) -> gpui_kit::AnyElement {
    // 避坑清单第 4 条：render 开头先把语义色 Copy 成局部变量。
    let p = theme::current(cx);
    let muted_fg = p.text_muted;

    // 发件账号胶囊（多账号时切换 compose_account_id）。
    let account_pills: Vec<gpui_kit::AnyElement> = app
        .accounts
        .iter()
        .map(|acc| {
            let acc_id = acc.id;
            let active = acc_id == app.compose_account_id;
            Button::new(("compose-from", acc_id))
                .ghost()
                .compact()
                .selected(active)
                .label(acc.email.clone())
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.compose_account_id = acc_id;
                    cx.notify();
                }))
                .into_any_element()
        })
        .collect();

    let attachment_chips: Vec<gpui_kit::AnyElement> = app
        .compose_attachments
        .iter()
        .enumerate()
        .map(|(idx, path)| attachment_chip(idx, path, cx).into_any_element())
        .collect();

    v_flex()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(p.canvas)
        .p_4()
        .gap_3()
        // 顶部操作栏：标题 + 发件账号 / 右侧按钮组。
        .child(
            h_flex()
                .justify_between()
                .items_start()
                .gap_2()
                .child(
                    v_flex()
                        .gap_1()
                        .min_w_0()
                        .child(div().text_base().font_weight(FontWeight::BOLD).child(
                            if app.compose_in_reply_to.is_empty() {
                                "写新邮件"
                            } else {
                                "回复邮件"
                            },
                        ))
                        .when(!app.accounts.is_empty(), |el| {
                            el.child(
                                h_flex()
                                    .gap_1()
                                    .flex_wrap()
                                    .items_center()
                                    .child(div().text_xs().text_color(muted_fg).child("发件账号："))
                                    .children(account_pills),
                            )
                        }),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .flex_shrink_0()
                        .child(
                            Button::new("compose-save-draft")
                                .outline()
                                .label("存草稿")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.save_local_draft(cx);
                                })),
                        )
                        .child(
                            Button::new("compose-attach")
                                .outline()
                                .label("添加附件")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.pick_attachments(cx);
                                })),
                        )
                        .child(
                            Button::new("compose-cancel")
                                .ghost()
                                .label("取消")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.right_mode = crate::app::RightPaneMode::Reader;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("compose-send")
                                .primary()
                                .label("发送")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.send_compose(cx);
                                })),
                        ),
                ),
        )
        // 信头表单区：收件人（右侧「抄送 / 密送」小按钮）+ 可选 Cc / Bcc + 主题。
        .child(
            v_flex()
                .rounded(px(8.0))
                .border_1()
                .border_color(p.border)
                .bg(p.surface)
                .px_3()
                .gap_0p5()
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .border_b_1()
                        .border_color(p.border)
                        .py_1()
                        .child(
                            div()
                                .w(px(LABEL_WIDTH))
                                .flex_shrink_0()
                                .text_sm()
                                .text_color(muted_fg)
                                .child("收件人"),
                        )
                        .child(div().flex_1().min_w_0().child(Input::new(&app.compose_to)))
                        .child(
                            Button::new("toggle-cc-bcc")
                                .ghost()
                                .compact()
                                .text_size(px(12.0))
                                .label(if app.compose_show_cc_bcc {
                                    "收起抄送/密送"
                                } else {
                                    "抄送 / 密送"
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.toggle_compose_cc_bcc(cx);
                                })),
                        ),
                )
                .when(app.compose_show_cc_bcc, |el| {
                    el.child(header_row("抄送", &app.compose_cc, &p))
                        .child(header_row("密送", &app.compose_bcc, &p))
                })
                .child(header_row("主题", &app.compose_subject, &p)),
        )
        // 附件芯片条。
        .when(!app.compose_attachments.is_empty(), |el| {
            el.child(h_flex().gap_2().flex_wrap().children(attachment_chips))
        })
        // 正文编辑区（flex-1 撑满剩余高度）。
        .child(
            div()
                .flex_1()
                .min_h_0()
                .rounded(px(8.0))
                .border_1()
                .border_color(p.border)
                .bg(p.surface)
                .p_2()
                .child(Textarea::new(&app.compose_body)),
        )
        .into_any_element()
}
