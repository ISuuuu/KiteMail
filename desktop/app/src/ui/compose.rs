//! 写信 / 回复面板（方案文档第三节页面 3）：
//! - 顶部操作按钮（取消 / 存草稿 / 附件 / 发送）合并进 app.rs 的 48px 顶栏（B-8），
//!   本面板不再画第二层标题。
//! - 信头表单区（细底边框分隔、紧凑对齐）：发件人账号胶囊行 + 收件人行右侧
//!   「抄送 / 密送」小按钮，默认折叠，展开后显示 Cc / Bcc 输入行；主题输入行。
//! - 附件芯片条：圆角胶囊标签（📎 文件名 (大小 KB) ×，点击移除）。
//! - 正文编辑区：`TextareaState` 多行编辑器，`flex-1` 撑满剩余高度。

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState, Textarea};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{h_flex, v_flex, Icon, Selectable, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    div, px, Context, Entity, FontWeight, InteractiveElement, IntoElement, MouseButton,
    ParentElement, SharedString, Styled,
};

use crate::app::{filter_contact_suggestions, ComposeAddressField, HylkiApp, RecipientPill};
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

/// 收件人胶囊（对齐 Linux `src/ui/recipient_row.rs` 的 `.recipient-pill`）：
/// 点击主体把地址放回输入框继续编辑，`⇄` 移到下一个收件人字段，`×` 删除；
/// 非法地址渲染红色 `.invalid` 边框。
fn recipient_pill_view(
    field: ComposeAddressField,
    idx: usize,
    pill: &RecipientPill,
    p: &theme::HylkiPalette,
    cx: &mut Context<HylkiApp>,
) -> gpui_kit::AnyElement {
    let (bg, fg, border) = if pill.valid {
        (p.accent_soft(), p.text, p.border)
    } else {
        (p.danger.opacity(0.12), p.danger, p.danger)
    };
    let pill_id = field.index() * 1000 + idx;
    let label = pill.label.clone();
    h_flex()
        .id(("recipient-pill", pill_id))
        .items_center()
        .gap_1()
        .pl_2()
        .pr_1()
        .py_0p5()
        .rounded(px(9999.0))
        .border_1()
        .border_color(border)
        .bg(bg)
        .cursor_pointer()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, window, cx| {
                this.edit_recipient_pill(field, idx, window, cx);
            }),
        )
        .child(
            div()
                .text_xs()
                .text_color(fg)
                .max_w(px(220.0))
                .truncate()
                .child(SharedString::from(label)),
        )
        .child(
            div()
                .id(("recipient-pill-move", pill_id))
                .px_1()
                .rounded(px(4.0))
                .text_xs()
                .text_color(fg)
                .cursor_pointer()
                .hover(move |s| s.bg(bg))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.move_recipient_pill(field, idx, cx);
                        cx.stop_propagation();
                    }),
                )
                .child(SharedString::from("⇄")),
        )
        .child(
            div()
                .id(("recipient-pill-remove", pill_id))
                .px_1()
                .rounded(px(4.0))
                .text_xs()
                .text_color(fg)
                .cursor_pointer()
                .hover(move |s| s.bg(bg))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| {
                        this.remove_recipient_pill(field, idx, cx);
                        cx.stop_propagation();
                    }),
                )
                .child(SharedString::from("×")),
        )
        .into_any_element()
}

/// 收件人信头行：左侧标签 + 胶囊流 + 输入框（输入框承载「正在输入的这一段」）。
fn recipient_row(
    field: ComposeAddressField,
    app: &HylkiApp,
    p: &theme::HylkiPalette,
    cx: &mut Context<HylkiApp>,
) -> gpui_kit::Div {
    let pills: Vec<gpui_kit::AnyElement> = app
        .compose_pills(field)
        .iter()
        .enumerate()
        .map(|(idx, pill)| recipient_pill_view(field, idx, pill, p, cx))
        .collect();
    let input = app.compose_field_input(field);
    let border = p.border;
    let muted_fg = p.text_muted;
    h_flex()
        .items_center()
        .gap_2()
        .border_b_1()
        .border_color(border)
        .py_1()
        .child(
            div()
                .w(px(LABEL_WIDTH))
                .flex_shrink_0()
                .text_sm()
                .text_color(muted_fg)
                .child(field.label()),
        )
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .flex_wrap()
                .items_center()
                .gap_1()
                .children(pills)
                .child(div().flex_1().min_w(px(120.0)).child(Input::new(&input))),
        )
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
                // F-4：附件变化属于写信内容变更，重置自动保存定时器。
                this.schedule_draft_autosave(cx);
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
    // 已收进胶囊的地址不再出现在建议里（输入框现在只承载「正在输入的这一段」）。
    let taken: std::collections::HashSet<String> = app
        .compose_pills(field)
        .iter()
        .map(|p| p.address.to_lowercase())
        .collect();
    let suggestions: Vec<_> = filter_contact_suggestions(input_value, &app.known_contacts, 8)
        .into_iter()
        .filter(|c| !taken.contains(&c.email.to_lowercase()))
        .take(5)
        .collect();
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

/// WP-10-2 发件身份控件（对齐 Linux `adw::ComboRow` + `custom_from_row`）：
/// 默认渲染「账号默认身份 + 别名」下拉，选中后写进 `compose_from_addr`；
/// 点菜单里的「自定义发件人…」切换为手输框，手输框旁的按钮退回下拉。
fn compose_from_control(
    app: &HylkiApp,
    entity: Entity<HylkiApp>,
    p: &theme::HylkiPalette,
    _cx: &mut Context<HylkiApp>,
) -> gpui_kit::AnyElement {
    if app.compose_from_custom {
        let ent_back = entity.clone();
        return h_flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .items_center()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&app.compose_from)),
            )
            .child(
                Button::new("compose-from-back")
                    .ghost()
                    .compact()
                    .text_size(px(12.0))
                    .label("用账号身份")
                    .on_click(move |_, _, cx| {
                        ent_back.update(cx, |this, cx| this.end_custom_compose_from(cx));
                    }),
            )
            .into_any_element();
    }

    let choices = app.compose_from_choices();
    let current_from = app.compose_from_addr.clone();
    let accent = p.accent;
    let ent_menu = entity.clone();
    Button::new("compose-from-identity")
        .ghost()
        .compact()
        .label(app.compose_from_display())
        .icon(IconName::ChevronDown)
        .dropdown_menu(move |mut menu, _, _| {
            for choice in choices.iter() {
                let selected = choice.from_addr == current_from;
                let from_addr = choice.from_addr.clone();
                let reply_to = choice.reply_to.clone();
                let ent_item = ent_menu.clone();
                menu = menu.item(
                    PopupMenuItem::new(choice.label.clone())
                        .when(selected, |item| {
                            item.icon(Icon::new(IconName::Check).text_color(accent))
                        })
                        .on_click(move |_, window, cx| {
                            ent_item.update(cx, |this, cx| {
                                this.select_compose_from(
                                    from_addr.clone(),
                                    reply_to.clone(),
                                    window,
                                    cx,
                                )
                            });
                        }),
                );
            }
            let ent_custom = ent_menu.clone();
            menu.item(
                PopupMenuItem::new("自定义发件人…")
                    .icon(Icon::new(IconName::Pencil))
                    .on_click(move |_, window, cx| {
                        ent_custom
                            .update(cx, |this, cx| this.begin_custom_compose_from(window, cx));
                    }),
            )
        })
        .into_any_element()
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

    // 发件账号胶囊（B-8：From 行入信头表单第一行，多账号时行内切换 compose_account_id）。
    let account_pills: Vec<gpui_kit::AnyElement> = app
        .accounts
        .iter()
        .filter(|acc| acc.enabled)
        .map(|acc| {
            let acc_id = acc.id;
            let active = acc_id == app.compose_account_id;
            Button::new(("compose-from", acc_id))
                .ghost()
                .compact()
                .selected(active)
                .label(acc.display_label())
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

    // 任务一：当前所选发件账号已停用（或不存在）时的提示条；发送入口同时阻断，
    // 要求用户明确选择可用账号后才能发送。
    let sender_unavailable = app.compose_sender_unavailable();
    let has_enabled_account = app.accounts.iter().any(|acc| acc.enabled);

    v_flex()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(p.canvas)
        .p_4()
        .gap_3()
        // 发件账号停用提示条。
        .when(sender_unavailable, |el| {
            el.child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .justify_between()
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(p.danger.opacity(0.5))
                    .bg(p.danger.opacity(0.10))
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(p.danger)
                            .child(SharedString::from(if has_enabled_account {
                                "当前所选发件账号已停用，请点击上方发件人选择可用账号后再发送"
                            } else {
                                "所有邮箱账号均已停用，请先在偏好设置中启用一个账号后再发送"
                            })),
                    ),
            )
        })
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
        // 遗漏附件发送前提醒横幅（主题或正文出现附件关键词但未添加附件时触发）。
        .when_some(app.compose_attachment_reminder.as_ref(), |el, kw| {
            let kw_label = kw.clone();
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
                            .child(SharedString::from(format!(
                                "正文中提到了「{kw_label}」，但尚未添加任何附件，是否仍要发送？"
                            ))),
                    )
                    .child(
                        h_flex()
                            .gap_1p5()
                            .items_center()
                            .child(
                                Button::new("compose-rem-attach")
                                    .primary()
                                    .compact()
                                    .icon(IconName::Paperclip)
                                    .label("添加附件")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.compose_attachment_reminder = None;
                                        this.pick_attachments(cx);
                                    })),
                            )
                            .child(
                                Button::new("compose-rem-send")
                                    .outline()
                                    .compact()
                                    .label("仍要发送")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.send_compose_forced(cx);
                                    })),
                            )
                            .child(
                                Button::new("compose-rem-dismiss")
                                    .ghost()
                                    .compact()
                                    .label("取消")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.compose_attachment_reminder = None;
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
        })
        // 信头表单区：发件人（多账号行内切换）+ 收件人（右侧「抄送 / 密送」小按钮）+ 可选 Cc / Bcc + 主题。
        .child(
            v_flex()
                .rounded(px(8.0))
                .border_1()
                .border_color(p.border)
                .bg(p.surface)
                .px_3()
                .gap_0p5()
                .when(!app.accounts.is_empty(), |form| {
                    // 发件人行（B-8：From 入信头表单第一行，标签与其它行左对齐）。
                    form.child(
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
                                    .child("发件人"),
                            )
                            .child(
                                h_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .flex_wrap()
                                    .gap_1()
                                    .children(account_pills),
                            )
                            // WP-10-2：发件身份下拉（账号默认身份 + 别名 + 自定义）。
                            .child(compose_from_control(app, cx.entity(), &p, cx)),
                    )
                })
                .child(
                    recipient_row(ComposeAddressField::To, app, &p, cx).child(
                        Button::new("toggle-cc-bcc")
                            .ghost()
                            .compact()
                            .text_size(px(12.0))
                            .label(if app.compose_show_cc_bcc {
                                "收起抄送/密送/Reply-To"
                            } else {
                                "抄送 / 密送 / Reply-To"
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_compose_cc_bcc(cx);
                            })),
                    ),
                )
                .children(to_suggestions)
                .when(app.compose_show_cc_bcc, |el| {
                    el.child(recipient_row(ComposeAddressField::Cc, app, &p, cx))
                        .children(cc_suggestions)
                        .child(recipient_row(ComposeAddressField::Bcc, app, &p, cx))
                        .children(bcc_suggestions)
                        // WP-10-2：Reply-To 行（对应原版 `reply_to_row`，默认折叠）。
                        .child(header_row("Reply-To", &app.compose_reply_to, &p))
                })
                .child(header_row("主题", &app.compose_subject, &p)),
        )
        // F-4：草稿自动保存常驻小字提示（右对齐，自动保存成功后出现）。
        .when_some(app.compose_autosaved_at.clone(), |el, saved_at| {
            el.child(
                h_flex()
                    .justify_end()
                    .items_center()
                    .gap_1()
                    .child(
                        Icon::new(IconName::Check)
                            .with_size(px(12.0))
                            .text_color(muted_fg),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted_fg)
                            .child(SharedString::from(format!("草稿已自动保存 {saved_at}"))),
                    ),
            )
        })
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
