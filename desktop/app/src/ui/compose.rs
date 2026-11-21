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
use gpui_kit::{
    div, px, Context, Entity, FontWeight, InteractiveElement, IntoElement, MouseButton,
    ParentElement, SharedString, Styled,
};

use crate::app::{filter_contact_suggestions, ComposeAddressField, HylkiApp};
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

/// 渲染某个收件人字段（To / Cc / Bcc）的联系人自动补全下拉列表（最多 5 条）。
fn render_autocomplete_dropdown(
    app: &HylkiApp,
    field: ComposeAddressField,
    input_value: &str,
    p: &theme::HylkiPalette,
    cx: &mut Context<HylkiApp>,
) -> Option<gpui_kit::AnyElement> {
    if app.compose_autocomplete_field != Some(field) {
        return None;
    }
    let suggestions = filter_contact_suggestions(input_value, &app.known_contacts, 5);
    if suggestions.is_empty() {
        return None;
    }
    let (border, surface_raised, accent_soft, text_fg, muted_fg) = (
        p.border,
        p.surface_raised,
        p.accent_soft(),
        p.text,
        p.text_muted,
    );
    let rows: Vec<gpui_kit::AnyElement> = suggestions
        .into_iter()
        .enumerate()
        .map(|(idx, contact)| {
            let contact_owned = contact.clone();
            let display_name = if contact.name.trim().is_empty() {
                contact.email.clone()
            } else {
                contact.name.clone()
            };
            let email = contact.email.clone();
            h_flex()
                .id(("compose-ac", idx))
                .px_2p5()
                .py_1p5()
                .gap_2()
                .items_center()
                .justify_between()
                .rounded(px(6.0))
                .cursor_pointer()
                .hover(move |s| s.bg(accent_soft))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        this.apply_contact_suggestion(field, &contact_owned, window, cx);
                        cx.stop_propagation();
                    }),
                )
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(text_fg)
                        .truncate()
                        .child(SharedString::from(display_name)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted_fg)
                        .truncate()
                        .child(SharedString::from(email)),
                )
                .into_any_element()
        })
        .collect();

    Some(
        v_flex()
            .ml(px(LABEL_WIDTH + 8.0))
            .my_1()
            .p_1()
            .gap_0p5()
            .rounded(px(8.0))
            .border_1()
            .border_color(border)
            .bg(surface_raised)
            .shadow_md()
            .children(rows)
            .into_any_element(),
    )
}

pub(crate) fn render(app: &HylkiApp, cx: &mut Context<HylkiApp>) -> gpui_kit::AnyElement {
    // 避坑清单第 4 条：render 开头先把语义色 Copy 成局部变量。
    let p = theme::current(cx);
    let muted_fg = p.text_muted;
    let to_value = app.compose_to.read(cx).value().to_string();
    let cc_value = app.compose_cc.read(cx).value().to_string();
    let bcc_value = app.compose_bcc.read(cx).value().to_string();
    let to_suggestions =
        render_autocomplete_dropdown(app, ComposeAddressField::To, &to_value, &p, cx);
    let cc_suggestions =
        render_autocomplete_dropdown(app, ComposeAddressField::Cc, &cc_value, &p, cx);
    let bcc_suggestions =
        render_autocomplete_dropdown(app, ComposeAddressField::Bcc, &bcc_value, &p, cx);

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
                                    this.request_close_compose(cx);
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
        // 未保存退出确认条。
        .when(app.compose_confirm_discard, |el| {
            el.child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .justify_between()
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.accent_soft())
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(p.text)
                            .child("有未保存的邮件内容，确定要离开吗？"),
                    )
                    .child(
                        h_flex()
                            .gap_1p5()
                            .items_center()
                            .child(
                                Button::new("compose-discard-save")
                                    .primary()
                                    .compact()
                                    .label("保存为草稿")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.save_draft_and_close_compose(cx);
                                    })),
                            )
                            .child(
                                Button::new("compose-discard-drop")
                                    .outline()
                                    .compact()
                                    .label("放弃")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.discard_compose(cx);
                                    })),
                            )
                            .child(
                                Button::new("compose-discard-keep")
                                    .ghost()
                                    .compact()
                                    .label("继续编辑")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.compose_confirm_discard = false;
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
        })
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
                .children(to_suggestions)
                .when(app.compose_show_cc_bcc, |el| {
                    el.child(header_row("抄送", &app.compose_cc, &p))
                        .children(cc_suggestions)
                        .child(header_row("密送", &app.compose_bcc, &p))
                        .children(bcc_suggestions)
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
