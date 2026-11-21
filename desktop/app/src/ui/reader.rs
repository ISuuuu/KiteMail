//! 右栏阅读态（方案文档第三节 1.3）：
//! - 顶部操作栏 48px：左侧回复 / 回复全部 / 转发 / 星标 / 归档 / 删除；
//!   右侧标记已读/未读（双向动态切换）+ 复制正文。
//! - 标题区（内边距 16px 24px 8px）：所属账号邮箱胶囊（浅蓝底蓝字圆角）
//!   + 20px 粗体主题。
//! - 卡片式正文（外层滚动、内边距 12px 24px 24px）：`surface_raised`
//!   圆角 12px、1px border、微阴影、内边距 20px；卡片头部：34x34 头像 +
//!   发件人姓名与 <邮箱> + 真实收件人胶囊 + 右侧时间 + `...` 更多操作菜单；
//!   正文行高 1.6；附件条：横向圆角附件小卡片（接 SaveAttachment + rfd）。
//! - 空状态：居中淡色邮箱图标 + 「选择一封邮件查看正文」。

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::text::TextView;
use gpui_kit::component::Icon;
use gpui_kit::component::{h_flex, v_flex, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    div, px, Context, InteractiveElement, IntoElement, ParentElement, SharedString, Styled,
};
use hylki_mail_core::imap_client::{plain_text_to_html, sanitize_html_for_display};
use hylki_mail_core::{AttachmentMeta, MailRequest, ThreadMessage};

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

/// 解析收件人列表，返回 `(计数胶囊文案, 收件人预览文本)`。
fn recipient_summary(raw: &str) -> (String, String) {
    let list: Vec<&str> = raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if list.is_empty() {
        ("无收件人".to_string(), String::new())
    } else {
        (format!("{} 位收件人", list.len()), list.join(", "))
    }
}

/// 渲染单张邮件正文卡片（既用于单封邮件，也用于会话线程中的多张叠放卡片）。
#[allow(clippy::too_many_arguments)]
fn render_message_card(
    account_id: u32,
    msg_id: u32,
    sender: &str,
    recipients: &str,
    date_display: &str,
    body_text: &str,
    body_html: Option<&str>,
    body_loaded: bool,
    prefer_plain: bool,
    block_remote_images: bool,
    attachments: &[AttachmentMeta],
    is_current: bool,
    more_menu: Option<gpui_kit::AnyElement>,
    p: &theme::HylkiPalette,
    cx: &mut Context<HylkiApp>,
) -> gpui_kit::AnyElement {
    let (canvas, border, muted_fg, accent) = (p.canvas, p.border, p.text_muted, p.accent);
    let (sender_name, sender_email) = split_sender(sender);
    let display_name = if sender_name.is_empty() {
        sender.to_string()
    } else {
        sender_name
    };
    let (recipient_pill, recipient_list) = recipient_summary(recipients);

    // 决定原生富文本渲染内容：有 HTML 且未切纯文本模式时渲染净化后的 HTML，
    // 否则将纯文本安全转义后通过 TextView::html(...).selectable(true) 渲染，
    // 保证所有邮件正文均支持鼠标框选与 Ctrl+C 复制。
    let (rendered_html, blocked_images) = if !prefer_plain {
        if let Some(raw_html) = body_html.filter(|s| !s.trim().is_empty()) {
            sanitize_html_for_display(raw_html, block_remote_images)
        } else {
            (plain_text_to_html(body_text), 0)
        }
    } else {
        (plain_text_to_html(body_text), 0)
    };

    let attachment_rows: Vec<gpui_kit::AnyElement> = attachments
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
                    Icon::new(IconName::Paperclip)
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
                        .icon(IconName::Download)
                        .tooltip("保存附件")
                        .on_click(cx.listener(move |this, _, _, _| {
                            this.save_attachment(att_id, &filename);
                        })),
                )
                .into_any_element()
        })
        .collect();

    v_flex()
        .bg(p.surface_raised)
        .rounded(px(12.0))
        .border_1()
        .border_color(if is_current {
            accent.opacity(0.45)
        } else {
            border
        })
        .shadow_xs()
        .p(px(20.0))
        .gap_3()
        .child(
            v_flex()
                .gap_1()
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
                                        .truncate()
                                        .child(SharedString::from(display_name)),
                                )
                                .when(!sender_email.is_empty(), |el| {
                                    el.child(
                                        div()
                                            .text_xs()
                                            .text_color(muted_fg)
                                            .truncate()
                                            .child(SharedString::from(format!("<{sender_email}>"))),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .px_2()
                                .py_0p5()
                                .rounded_full()
                                .bg(canvas)
                                .border_1()
                                .border_color(border)
                                .text_color(muted_fg)
                                .text_xs()
                                .child(SharedString::from(recipient_pill)),
                        )
                        .child(div().flex_1())
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_xs()
                                .text_color(muted_fg)
                                .child(SharedString::from(date_display.to_string())),
                        )
                        .children(more_menu),
                )
                .when(!recipient_list.is_empty(), |el| {
                    el.child(
                        div()
                            .pl(px(CARD_AVATAR + 8.0))
                            .text_xs()
                            .text_color(muted_fg)
                            .truncate()
                            .child(SharedString::from(format!("收件人：{recipient_list}"))),
                    )
                }),
        )
        .when(blocked_images > 0, |card| {
            card.child(
                h_flex()
                    .px_3()
                    .py_1p5()
                    .gap_2()
                    .items_center()
                    .justify_between()
                    .rounded(px(8.0))
                    .bg(p.accent_soft())
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.text)
                            .child(SharedString::from(format!(
                                "已拦截 {blocked_images} 张远程图片以保护隐私"
                            ))),
                    )
                    .child(
                        Button::new(("allow-remote-img", msg_id))
                            .outline()
                            .compact()
                            .label("显示远程图片")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.allow_remote_images_for(msg_id, cx);
                            })),
                    ),
            )
        })
        .child(
            div()
                .text_sm()
                .text_color(if body_loaded { p.text } else { muted_fg })
                .child(
                    TextView::html(
                        SharedString::from(format!(
                            "reader-body-{msg_id}-{}-{}",
                            prefer_plain as u8, block_remote_images as u8
                        )),
                        rendered_html,
                    )
                    .selectable(true),
                ),
        )
        .when(!body_loaded, |card| {
            card.child(
                h_flex().child(
                    Button::new(("load-thread-body", msg_id))
                        .outline()
                        .compact()
                        .label("加载完整正文")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(ref w) = this.worker {
                                w.send(MailRequest::LoadMessageBody {
                                    account_id,
                                    message_id: msg_id,
                                });
                                cx.notify();
                            }
                        })),
                ),
            )
        })
        .when(!attachment_rows.is_empty(), |card| {
            card.child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .pt_2()
                    .border_t_1()
                    .border_color(border)
                    .children(attachment_rows),
            )
        })
        .into_any_element()
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
                Icon::new(IconName::Mail)
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

    let owner_email = app.account_email_for(detail.account_id).to_string();
    let flagged = detail.flagged;
    let is_unread = app.selected_message_unread();
    let prefer_plain = app.reader_prefer_plain;
    let thread_count = detail.thread_messages.len();
    let has_any_html = detail
        .body_html
        .as_deref()
        .is_some_and(|s| !s.trim().is_empty())
        || detail
            .thread_messages
            .iter()
            .any(|m| m.body_html.as_deref().is_some_and(|s| !s.trim().is_empty()));

    // 卡片头部 `...` 更多操作菜单。
    let card_more_menu = {
        let entity = cx.entity();
        let reply_detail = detail.clone();
        let reply_all_detail = detail.clone();
        let forward_detail = detail.clone();
        let star_detail = detail.clone();
        Button::new("reader-card-more")
            .ghost()
            .compact()
            .icon(IconName::Ellipsis)
            .tooltip("更多操作")
            .dropdown_menu(move |menu, _, _| {
                let e_reply = entity.clone();
                let e_reply_all = entity.clone();
                let e_forward = entity.clone();
                let e_star = entity.clone();
                let e_copy = entity.clone();
                let d_reply = reply_detail.clone();
                let d_reply_all = reply_all_detail.clone();
                let d_forward = forward_detail.clone();
                let d_star = star_detail.clone();
                menu.item(
                    PopupMenuItem::new("回复")
                        .icon(Icon::new(IconName::Reply))
                        .on_click(move |_, window, cx| {
                            let d = d_reply.clone();
                            e_reply.update(cx, |this, cx| this.start_reply(&d, window, cx));
                        }),
                )
                .item(
                    PopupMenuItem::new("回复全部")
                        .icon(Icon::new(IconName::ReplyAll))
                        .on_click(move |_, window, cx| {
                            let d = d_reply_all.clone();
                            e_reply_all.update(cx, |this, cx| this.start_reply_all(&d, window, cx));
                        }),
                )
                .item(
                    PopupMenuItem::new("转发")
                        .icon(Icon::new(IconName::Forward))
                        .on_click(move |_, window, cx| {
                            let d = d_forward.clone();
                            e_forward.update(cx, |this, cx| this.start_forward(&d, window, cx));
                        }),
                )
                .separator()
                .item(
                    PopupMenuItem::new(if flagged { "取消星标" } else { "加星标" })
                        .icon(Icon::new(if flagged {
                            IconName::StarFill
                        } else {
                            IconName::Star
                        }))
                        .on_click(move |_, _, cx| {
                            let d = d_star.clone();
                            e_star.update(cx, |this, cx| this.toggle_flagged(&d, cx));
                        }),
                )
                .item(
                    PopupMenuItem::new("复制邮件正文")
                        .icon(Icon::new(IconName::Copy))
                        .on_click(move |_, _, cx| {
                            e_copy.update(cx, |this, cx| this.copy_selected_body(cx));
                        }),
                )
            })
            .into_any_element()
    };

    // 构造单卡或多线程叠放卡片列表。
    let cards: Vec<gpui_kit::AnyElement> = if thread_count > 1 {
        let mut more_menu = Some(card_more_menu);
        detail
            .thread_messages
            .iter()
            .map(|tm: &ThreadMessage| {
                let is_current = tm.id == detail.id;
                let menu = if is_current { more_menu.take() } else { None };
                let block_remote =
                    app.prefs.block_remote_images && !app.allowed_remote_image_ids.contains(&tm.id);
                render_message_card(
                    detail.account_id,
                    tm.id,
                    &tm.sender,
                    &tm.recipients,
                    &tm.date_display,
                    &tm.body_text,
                    tm.body_html.as_deref(),
                    tm.body_loaded,
                    prefer_plain,
                    block_remote,
                    &tm.attachments,
                    is_current,
                    menu,
                    &p,
                    cx,
                )
            })
            .collect()
    } else {
        let block_remote =
            app.prefs.block_remote_images && !app.allowed_remote_image_ids.contains(&detail.id);
        vec![render_message_card(
            detail.account_id,
            detail.id,
            &detail.sender,
            &detail.recipients,
            &detail.date_display,
            &detail.body_text,
            detail.body_html.as_deref(),
            true,
            prefer_plain,
            block_remote,
            &detail.attachments,
            false,
            Some(card_more_menu),
            &p,
            cx,
        )]
    };

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
                            action_button("reader-reply", IconName::Reply, "回复").on_click(
                                cx.listener(move |this, _, window, cx| {
                                    this.start_reply(&reply_detail, window, cx);
                                }),
                            ),
                        )
                        .child(
                            action_button("reader-reply-all", IconName::ReplyAll, "回复全部")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.start_reply_all(&reply_all_detail, window, cx);
                                })),
                        )
                        .child(
                            action_button("reader-forward", IconName::Forward, "转发").on_click(
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
                                action_button("reader-archive", IconName::Archive, "归档")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.archive_selected(cx);
                                    })),
                            )
                            .child(
                                action_button("reader-delete", IconName::Trash, "删除").on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.delete_selected(cx);
                                    }),
                                ),
                            )
                        })
                })
                // 右侧操作组：动态标记已读/未读 + HTML/纯文本切换 + 复制正文（严格放在 48px 顶栏内）。
                .child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .when(has_any_html, |el| {
                            el.child(
                                Button::new("reader-toggle-html")
                                    .ghost()
                                    .compact()
                                    .icon(if prefer_plain {
                                        IconName::FileCode
                                    } else {
                                        IconName::FileText
                                    })
                                    .tooltip(if prefer_plain {
                                        "切换为富文本 HTML 视图"
                                    } else {
                                        "切换为纯文本视图"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.toggle_reader_plain_mode(cx);
                                    })),
                            )
                        })
                        .child(
                            Button::new("reader-toggle-seen")
                                .ghost()
                                .compact()
                                .icon(if is_unread {
                                    IconName::MailOpen
                                } else {
                                    IconName::Mail
                                })
                                .tooltip(if is_unread {
                                    "标记为已读"
                                } else {
                                    "标记为未读"
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.toggle_message_seen(cx);
                                })),
                        )
                        .child(
                            action_button("reader-copy-body", IconName::Copy, "复制正文").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.copy_selected_body(cx);
                                }),
                            ),
                        ),
                ),
        )
        // 标题区（16px 24px 8px）：账号胶囊 + 可选会话数胶囊 + 20px 粗体主题。
        .child(
            v_flex()
                .flex_shrink_0()
                .items_start()
                .pt(px(16.0))
                .px(px(24.0))
                .pb(px(8.0))
                .gap_1()
                .when(!owner_email.is_empty() || thread_count > 1, |el| {
                    el.child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .when(!owner_email.is_empty(), |row| {
                                row.child(
                                    div()
                                        .px_2()
                                        .py_0p5()
                                        .rounded_full()
                                        .bg(p.accent_soft())
                                        .text_color(accent)
                                        .text_xs()
                                        .font_weight(gpui_kit::FontWeight::MEDIUM)
                                        .child(SharedString::from(owner_email)),
                                )
                            })
                            .when(thread_count > 1, |row| {
                                row.child(
                                    div()
                                        .px_2()
                                        .py_0p5()
                                        .rounded_full()
                                        .bg(p.surface_raised)
                                        .border_1()
                                        .border_color(border)
                                        .text_color(muted_fg)
                                        .text_xs()
                                        .child(SharedString::from(format!(
                                            "{thread_count} 封往来会话"
                                        ))),
                                )
                            }),
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
        // 卡片式正文（外层滚动，内边距 12px 24px 24px；多封会话纵向叠放）。
        .child(
            v_flex()
                .id("reader-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .pt(px(12.0))
                .px(px(24.0))
                .pb(px(24.0))
                .gap_3()
                .children(cards),
        )
        .into_any_element()
}
