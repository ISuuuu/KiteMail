//! 右栏阅读态（方案文档第三节 1.3）：
//! - 顶部操作栏 48px：回复 / 回复全部 / 转发 / 星标切换（沿用现有逻辑）；
//!   归档、删除本轮渲染但禁用置灰（TODO phase2）；右侧标记已读/未读。
//! - 标题区（内边距 16px 24px 8px）：所属账号邮箱胶囊（浅蓝底蓝字圆角）
//!   + 20px 粗体主题。
//! - 卡片式正文（外层滚动、内边距 12px 24px 24px）：`surface_raised`
//!   圆角 12px、1px border、内边距 20px；卡片头部：34x34 头像 +
//!   发件人姓名与 <邮箱> + 「收件人」胶囊 + 右侧时间；正文行高 1.6；
//!   附件条：横向圆角附件小卡片（文件图标 + 文件名 + 大小 + 「保存」，
//!   接现有 SaveAttachment + rfd 流程）。
//! - 空状态：居中淡色邮箱图标 + 「选择一封邮件查看正文」。

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::Icon;
use gpui_kit::component::IconName;
use gpui_kit::component::{h_flex, v_flex, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    div, px, relative, Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled,
};

use crate::app::{split_sender, HylkiApp};
use crate::theme;
use crate::ui::avatar;

/// 顶部操作栏高度。
const HEADER_HEIGHT: f32 = 48.0;
/// 卡片头像尺寸。
const CARD_AVATAR: f32 = 34.0;

/// 48px 操作栏上的紧凑图标按钮。
fn action_button(id: &'static str, icon: IconName, tooltip: &'static str) -> Button {
    Button::new(id)
        .ghost()
        .compact()
        .icon(icon)
        .tooltip(tooltip)
}

pub(crate) fn render(app: &HylkiApp, cx: &mut Context<HylkiApp>) -> gpui_kit::AnyElement {
    // 避坑清单第 4 条：render 开头先把语义色 Copy 出来。
    let p = theme::current(cx);
    let (canvas, border, muted_fg, accent) = (p.canvas, p.border, p.text_muted, p.accent);

    let Some(detail) = app.selected_message.clone() else {
        return v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(canvas)
            .items_center()
            .justify_center()
            .gap_2()
            .child(
                Icon::new(IconName::Inbox)
                    .with_size(px(36.0))
                    .text_color(muted_fg),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(muted_fg)
                    .child("选择一封邮件查看正文"),
            )
            .into_any_element();
    };

    let (sender_name, sender_email) = split_sender(&detail.sender);
    let display_name = if sender_name.is_empty() {
        detail.sender.clone()
    } else {
        sender_name
    };
    let owner_email = app.account_email_for(detail.account_id).to_string();
    let flagged = detail.flagged;

    // 附件条。
    let attachment_rows: Vec<gpui_kit::AnyElement> = detail
        .attachments
        .iter()
        .map(|att| {
            let att_id = att.id;
            let filename = att.filename.clone();
            let size_kb = (att.size_bytes + 1023) / 1024;
            h_flex()
                .items_center()
                .gap_2()
                .px_2()
                .py_1()
                .rounded(px(8.0))
                .border_1()
                .border_color(border)
                .bg(canvas)
                .child(
                    Icon::new(IconName::FileText)
                        .with_size(px(14.0))
                        .text_color(muted_fg),
                )
                .child(
                    div()
                        .text_xs()
                        .max_w(px(240.0))
                        .truncate()
                        .child(SharedString::from(filename.clone())),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted_fg)
                        .child(SharedString::from(format!("{size_kb} KB"))),
                )
                .child(
                    Button::new(("save-att", att_id))
                        .ghost()
                        .compact()
                        .icon(IconName::ArrowDown)
                        .tooltip("保存附件")
                        .on_click(cx.listener(move |this, _, _, _| {
                            this.save_attachment(att_id, &filename);
                        })),
                )
        })
        .map(|row| row.into_any_element())
        .collect();

    v_flex()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(canvas)
        // 顶部操作栏 48px。
        .child(
            h_flex()
                .h(px(HEADER_HEIGHT))
                .flex_shrink_0()
                .px_3()
                .gap_1()
                .items_center()
                .justify_between()
                .border_b_1()
                .border_color(border)
                .child({
                    let reply_detail = detail.clone();
                    let reply_all_detail = detail.clone();
                    let forward_detail = detail.clone();
                    let star_detail = detail.clone();
                    h_flex()
                        .gap_1()
                        .items_center()
                        .child(
                            action_button("reader-reply", IconName::Undo, "回复").on_click(
                                cx.listener(move |this, _, window, cx| {
                                    this.start_reply(&reply_detail, window, cx);
                                }),
                            ),
                        )
                        .child(
                            action_button("reader-reply-all", IconName::Undo2, "回复全部")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.start_reply_all(&reply_all_detail, window, cx);
                                })),
                        )
                        .child(
                            action_button("reader-forward", IconName::Redo, "转发").on_click(
                                cx.listener(move |this, _, window, cx| {
                                    this.start_forward(&forward_detail, window, cx);
                                }),
                            ),
                        )
                        .child(
                            Button::new("reader-star")
                                .ghost()
                                .compact()
                                .icon(if flagged {
                                    IconName::StarFill
                                } else {
                                    IconName::Star
                                })
                                .tooltip(if flagged { "取消星标" } else { "加星标" })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.toggle_flagged(&star_detail, cx);
                                })),
                        )
                        // 迭代 2：归档 / 删除接线（IMAP MOVE / Graph move）；
                        // 演示模式不支持服务端移动，隐藏这两个入口。
                        .when(!app.demo_mode, |el| {
                            el.child(
                                action_button("reader-archive", IconName::FolderClosed, "归档")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.archive_selected(cx);
                                    })),
                            )
                            .child(
                                action_button("reader-delete", IconName::Delete, "删除").on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.delete_selected(cx);
                                    }),
                                ),
                            )
                        })
                }),
        )
        .child(h_flex().gap_1().items_center().when(!app.demo_mode, |el| {
            el.child(
                Button::new("reader-toggle-seen")
                    .ghost()
                    .compact()
                    .icon(IconName::EyeOff)
                    .tooltip("标记为未读（沿用现有 SetSeen）")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_message_seen(cx);
                    })),
            )
        }))
        // 标题区（16px 24px 8px）。
        .child(
            v_flex()
                .flex_shrink_0()
                .items_start()
                .pt(px(16.0))
                .px(px(24.0))
                .pb(px(8.0))
                .gap_1()
                .when(!owner_email.is_empty(), |el| {
                    el.child(
                        h_flex().child(
                            div()
                                .px_2()
                                .py_0p5()
                                .rounded_full()
                                .bg(p.accent_soft())
                                .text_color(accent)
                                .text_xs()
                                .font_weight(gpui_kit::FontWeight::MEDIUM)
                                .child(SharedString::from(owner_email)),
                        ),
                    )
                })
                .child(
                    div()
                        .text_size(px(20.0))
                        .font_weight(gpui_kit::FontWeight::BOLD)
                        .text_color(p.text)
                        .child(SharedString::from(detail.subject.clone())),
                ),
        )
        // 卡片式正文（外层滚动，内边距 12px 24px 24px）。
        .child(
            v_flex()
                .id("reader-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .pt(px(12.0))
                .px(px(24.0))
                .pb(px(24.0))
                .child(
                    v_flex()
                        .bg(p.surface_raised)
                        .rounded(px(12.0))
                        .border_1()
                        .border_color(border)
                        .p(px(20.0))
                        .gap_3()
                        // 卡片头部：34x34 头像 + 姓名 <邮箱> + 「收件人」胶囊 + 右侧时间。
                        .child(
                            h_flex()
                                .items_center()
                                .gap_2()
                                .child(avatar::avatar(&display_name, &sender_email, CARD_AVATAR))
                                .child(
                                    v_flex()
                                        .min_w_0()
                                        .child(
                                            div()
                                                .text_sm()
                                                .font_weight(gpui_kit::FontWeight::BOLD)
                                                .text_color(p.text)
                                                .child(SharedString::from(display_name)),
                                        )
                                        .when(!sender_email.is_empty(), |el| {
                                            el.child(div().text_xs().text_color(muted_fg).child(
                                                SharedString::from(format!("<{sender_email}>")),
                                            ))
                                        }),
                                )
                                .child(
                                    div()
                                        .px_2()
                                        .py_0p5()
                                        .rounded_full()
                                        .bg(p.accent_soft())
                                        .text_color(p.text)
                                        .text_xs()
                                        .child("收件人"),
                                )
                                .child(div().flex_1())
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(muted_fg)
                                        .child(SharedString::from(detail.date_display.clone())),
                                ),
                        )
                        // 正文：行高 1.6。
                        .child(
                            div()
                                .text_sm()
                                .line_height(relative(1.6))
                                .text_color(p.text)
                                .child(SharedString::from(detail.body_text.clone())),
                        )
                        // 卡片底部附件条。
                        .when(!detail.attachments.is_empty(), |card| {
                            card.child(
                                h_flex()
                                    .gap_2()
                                    .flex_wrap()
                                    .pt_2()
                                    .border_t_1()
                                    .border_color(border)
                                    .children(attachment_rows),
                            )
                        }),
                ),
        )
        .into_any_element()
}
