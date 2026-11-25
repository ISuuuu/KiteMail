//! 右栏阅读态（方案文档第三节 1.3）：
//! - 顶部操作栏 48px：左侧回复 / 回复全部 / 转发 / 星标 / 归档 / 删除；
//!   右侧标记已读/未读（双向动态切换）+ 复制正文；窗口过窄（<880px，B-5）时
//!   上述右侧低频动作收进「⋯」下拉菜单。
//! - 标题区（内边距 16px 24px 8px）：所属账号邮箱胶囊（浅蓝底蓝字圆角）
//!   + 阅读视图开关与单封邮件亮 / 暗背景切换（B-5）+ 20px 粗体主题。
//! - 卡片式正文（外层滚动、内边距 12px 24px 24px）：阅读器正文底色
//!   圆角 12px、1px border、微阴影、内边距 20px；卡片头部：34x34 头像 +
//!   发件人姓名与 <邮箱> + 真实收件人胶囊 + 右侧时间 + `...` 更多操作菜单
//!   （B-2：标为未读 / 星标切换 / 归档 / 删除 / 导出为 .eml / 复制正文 / 查看原文）；
//!   正文行高 1.6；附件条：横向圆角附件小卡片（接 SaveAttachment + rfd）。
//! - 空状态：居中淡色邮箱图标 + 「选择一封邮件查看正文」。

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::text::{TextView, TextViewState};
use gpui_kit::component::Icon;
use gpui_kit::component::{h_flex, v_flex, Disableable, Selectable, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    div, px, AppContext, Context, DragMoveEvent, Entity, InteractiveElement, IntoElement,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, Window,
};
use hylki_mail_core::imap_client::{plain_text_to_html, sanitize_html_for_display_with_options};
use hylki_mail_core::mime::{split_quoted_html, split_quoted_plain};
use hylki_mail_core::{AttachmentMeta, MailRequest, MessageDetail, ThreadMessage};

use crate::app::{split_sender, HylkiApp, READER_OVERFLOW_BREAKPOINT};
use crate::theme;
use crate::ui::attachment_drawer::{self, AttachmentDrawerDrag, DrawerAttachment};
use crate::ui::message_window::LinkPreviewTooltip;
use crate::ui::{avatar, window_controls};
use crate::webview_host::BodyView;

/// 顶部操作栏高度。
const HEADER_HEIGHT: f32 = 48.0;
/// 卡片头像尺寸。
const CARD_AVATAR: f32 = 34.0;

pub(super) fn body_loading(
    background: gpui_kit::Hsla,
    foreground: gpui_kit::Hsla,
    show_message: bool,
) -> gpui_kit::Div {
    let base = v_flex()
        .w_full()
        .items_center()
        .justify_center()
        .bg(background);
    if show_message {
        base.text_sm()
            .text_color(foreground)
            .child("正在加载邮件正文…")
    } else {
        base
    }
}

/// 48px 操作栏上的紧凑图标按钮。
fn action_button(id: &'static str, icon: IconName, tooltip: &'static str) -> Button {
    Button::new(id)
        .ghost()
        .compact()
        .icon(icon)
        .tooltip(tooltip)
}

/// 收件人预览文本（去空白项后以逗号重排）。
fn recipient_list(raw: &str) -> String {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

/// 提取折叠卡片用的单行正文摘要。
fn first_snippet_line(body_text: &str) -> String {
    body_text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
        .chars()
        .take(120)
        .collect()
}

/// 组装「查看邮件原文」展示内容（信头 + 原始纯文本 + 可选 HTML 源码）。
fn format_raw_source(
    rfc_message_id: &str,
    sender: &str,
    recipients: &str,
    cc: &str,
    date_display: &str,
    subject: &str,
    body_text: &str,
    body_html: Option<&str>,
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Message-ID: {}\nFrom: {}\nTo: {}\nCc: {}\nDate: {}\nSubject: {}\n",
        if rfc_message_id.trim().is_empty() {
            "(无)"
        } else {
            rfc_message_id
        },
        sender,
        if recipients.trim().is_empty() {
            "(无)"
        } else {
            recipients
        },
        if cc.trim().is_empty() { "(无)" } else { cc },
        date_display,
        subject,
    ));
    out.push_str("\n--- 纯文本正文 ---\n");
    out.push_str(body_text);
    if let Some(html) = body_html.filter(|s| !s.trim().is_empty()) {
        out.push_str("\n\n--- 原始 HTML 源码 ---\n");
        out.push_str(html);
    }
    out
}

/// 为单张会话卡片构造独立的 `...` 更多操作菜单（B-2：回复 / 回复全部 / 转发 /
/// 标为未读 / 星标切换 / 归档 / 删除 / 导出 .eml / 复制正文 / 查看原文）。
fn build_card_more_menu(
    card_detail: MessageDetail,
    show_raw_source: bool,
    demo_mode: bool,
    eml_exporting: bool,
    cx: &mut Context<HylkiApp>,
) -> gpui_kit::AnyElement {
    let entity = cx.entity();
    let popup_entity = entity.clone();
    let msg_id = card_detail.id;
    let account_id = card_detail.account_id;
    let uid = card_detail.uid;
    let folder_path = card_detail.folder_path.clone();
    let card_flagged = card_detail.flagged;
    let body_for_copy = card_detail.body_text.clone();
    let reply_detail = card_detail.clone();
    let reply_all_detail = card_detail.clone();
    let unsubscribe_url = card_detail.unsubscribe_url.clone();
    let unsubscribe_one_click = card_detail.unsubscribe_one_click;
    let forward_detail = card_detail;

    Button::new(("reader-card-more", msg_id))
        .ghost()
        .compact()
        .icon(IconName::Ellipsis)
        .tooltip("更多操作")
        .dropdown_menu(move |menu, _, _| {
            let e_reply = entity.clone();
            let e_reply_all = entity.clone();
            let e_forward = entity.clone();
            let e_seen = entity.clone();
            let e_flag = entity.clone();
            let e_archive = entity.clone();
            let e_delete = entity.clone();
            let e_eml = entity.clone();
            let e_copy = entity.clone();
            let e_raw = entity.clone();
            let e_unsub = entity.clone();
            let d_reply = reply_detail.clone();
            let d_reply_all = reply_all_detail.clone();
            let d_forward = forward_detail.clone();
            let copy_text = body_for_copy.clone();
            let folder_seen = folder_path.clone();
            let folder_flag = folder_path.clone();
            let folder_eml = folder_path.clone();
            let unsub_url = unsubscribe_url.clone();
            let mut menu = menu
                .item(
                    PopupMenuItem::new("回复")
                        .icon(Icon::new(IconName::Reply))
                        .on_click(move |_, window, cx| {
                            let d = d_reply.clone();
                            e_reply.update(cx, |this, cx| {
                                this.start_reply(&d, window, cx);
                            });
                        }),
                )
                .item(
                    PopupMenuItem::new("回复全部")
                        .icon(Icon::new(IconName::ReplyAll))
                        .on_click(move |_, window, cx| {
                            let d = d_reply_all.clone();
                            e_reply_all.update(cx, |this, cx| {
                                this.start_reply_all(&d, window, cx);
                            });
                        }),
                )
                .item(
                    PopupMenuItem::new("转发")
                        .icon(Icon::new(IconName::Forward))
                        .on_click(move |_, window, cx| {
                            let d = d_forward.clone();
                            e_forward.update(cx, |this, cx| {
                                this.start_forward(&d, window, cx);
                            });
                        }),
                )
                .separator()
                .item(
                    PopupMenuItem::new("标为未读")
                        .icon(Icon::new(IconName::Mail))
                        .on_click(move |_, _, cx| {
                            let p = folder_seen.clone();
                            e_seen.update(cx, |this, cx| {
                                this.set_row_seen(account_id, msg_id, uid, p, false, cx);
                            });
                        }),
                )
                .item(
                    PopupMenuItem::new(if card_flagged {
                        "取消星标"
                    } else {
                        "加星标"
                    })
                    .icon(if card_flagged {
                        Icon::new(IconName::Star)
                    } else {
                        Icon::new(IconName::StarFill)
                    })
                    .on_click(move |_, _, cx| {
                        let p = folder_flag.clone();
                        e_flag.update(cx, |this, cx| {
                            this.set_row_flagged(account_id, msg_id, uid, p, !card_flagged, cx);
                        });
                    }),
                );
            // 归档 / 删除为服务端移动，演示模式隐藏（与列表行菜单惯例一致）。
            if !demo_mode {
                menu = menu
                    .separator()
                    .item(
                        PopupMenuItem::new("归档")
                            .icon(Icon::new(IconName::Archive))
                            .on_click(move |_, _, cx| {
                                e_archive.update(cx, |this, cx| {
                                    this.archive_message(account_id, msg_id, cx);
                                });
                            }),
                    )
                    .item(
                        PopupMenuItem::new("删除")
                            .icon(Icon::new(IconName::Trash))
                            .on_click(move |_, _, cx| {
                                e_delete.update(cx, |this, cx| {
                                    this.delete_message(account_id, msg_id, cx);
                                });
                            }),
                    );
            }
            menu = menu
                .separator()
                .item(
                    PopupMenuItem::new(if eml_exporting {
                        "导出中..."
                    } else {
                        "导出为 .eml"
                    })
                    .disabled(eml_exporting)
                    .icon(Icon::new(IconName::Download))
                    .on_click(move |_, _, cx| {
                        let p = folder_eml.clone();
                        e_eml.update(cx, |this, cx| {
                            this.request_export_eml(account_id, msg_id, uid, p, cx);
                        });
                    }),
                )
                .item(
                    PopupMenuItem::new("复制邮件正文")
                        .icon(Icon::new(IconName::Copy))
                        .on_click(move |_, _, cx| {
                            let text = copy_text.clone();
                            e_copy.update(cx, |this, cx| this.copy_text_to_clipboard(text, cx));
                        }),
                )
                .item(
                    PopupMenuItem::new(if show_raw_source {
                        "返回正文"
                    } else {
                        "查看邮件原文"
                    })
                    .icon(Icon::new(IconName::FileCode))
                    .on_click(move |_, _, cx| {
                        e_raw.update(cx, |this, cx| this.toggle_raw_source(msg_id, cx));
                    }),
                );
            if let Some(url) = unsub_url {
                menu = menu.separator().item(
                    PopupMenuItem::new("退订")
                        .icon(Icon::new(IconName::MailX))
                        .on_click(move |_, _, cx| {
                            let u = url.clone();
                            e_unsub.update(cx, |this, cx| {
                                this.trigger_unsubscribe(&u, unsubscribe_one_click, cx);
                            });
                        }),
                );
            }
            menu
        })
        .on_open_change(move |open, _, cx| {
            popup_entity.update(cx, |app, cx| {
                app.set_reader_popup_open(*open);
                cx.notify();
            });
        })
        .into_any_element()
}

/// 渲染单张邮件正文卡片（既用于单封邮件，也用于会话线程中的多张叠放卡片）。
///
/// `bg_pick` 为 B-5 的单封邮件亮 / 暗背景覆盖（`Some(false)` 强制亮色、
/// `Some(true)` 强制暗色、`None` 跟随主题；会话级状态，不持久化）。
#[allow(clippy::too_many_arguments)]
fn render_message_card(
    account_id: u32,
    msg_id: u32,
    rfc_message_id: &str,
    subject: &str,
    sender: &str,
    recipients: &str,
    cc: &str,
    // 收件方网关写入的 `Authentication-Results` 头（无则不做真实性判定）。
    auth_results: Option<&str>,
    date_display: &str,
    date_ts: i64,
    date_style: &str,
    clock_style: &str,
    body_text: &str,
    body_html: Option<&str>,
    body_loaded: bool,
    body_pending: bool,
    prefer_plain: bool,
    block_remote_images: bool,
    clean_links: bool,
    show_remote_banner: bool,
    is_collapsed: bool,
    show_raw_source: bool,
    quote_expanded: bool,
    zoom_percent: u32,
    bg_pick: Option<bool>,
    forced_theme_bg: Option<bool>,
    card_chrome: bool,
    always_show_recipients: bool,
    card_attachments: bool,
    attachments: &[AttachmentMeta],
    is_current: bool,
    more_menu: gpui_kit::AnyElement,
    body_view: BodyView,
    webview_in_thread: bool,
    scale_factor: f32,
    p: &theme::HylkiPalette,
    app: &mut HylkiApp,
    cx: &mut Context<HylkiApp>,
) -> gpui_kit::AnyElement {
    let app_for_link = cx.entity().downgrade();
    let app_for_quote_link = app_for_link.clone();
    let (canvas, border, muted_fg, accent) = (p.canvas, p.border, p.text_muted, p.accent);
    let formatted_date =
        hylki_mail_core::format_message_datetime(date_ts, date_display, date_style, clock_style);
    let (sender_name, sender_email) = split_sender(sender);
    let display_name = if sender_name.is_empty() {
        sender.to_string()
    } else {
        sender_name
    };
    let sender_check = hylki_mail_core::verify::check_sender_summary(sender, auth_results);
    // 「未验证」对用户不是可行动的结论（很多服务商干脆不写校验头），因此只在
    // 有明确结论时显示徽标：已验证 / 疑似仿冒 / 校验失败。
    let trust_badge = match sender_check.trust {
        hylki_mail_core::verify::SenderTrust::Pass => {
            Some(("已验证", gpui_kit::hsla(0.38, 0.65, 0.36, 1.0)))
        }
        hylki_mail_core::verify::SenderTrust::Suspicious => {
            Some(("疑似仿冒", gpui_kit::hsla(0.10, 0.85, 0.45, 1.0)))
        }
        hylki_mail_core::verify::SenderTrust::Fail => Some(("校验失败", p.danger)),
        hylki_mail_core::verify::SenderTrust::Unverified => None,
    };
    let spoof_banner = if matches!(
        sender_check.trust,
        hylki_mail_core::verify::SenderTrust::Suspicious
            | hylki_mail_core::verify::SenderTrust::Fail
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
    let detected_invite = if body_text.contains("BEGIN:VCALENDAR") {
        hylki_mail_core::invite::parse(body_text)
    } else {
        None
    };
    let has_ics_attachment = attachments
        .iter()
        .any(|a| a.filename.to_ascii_lowercase().ends_with(".ics"));
    let read_receipt_policy = app.prefs.read_receipt_policy.clone();
    let has_mdn_header = read_receipt_policy != "never"
        && hylki_mail_core::receipt::detect_raw(body_text.as_bytes()).is_some();

    // 折叠态：紧凑显示展开箭头 + 头像 + 发件人 + 单行摘要 + 时间 + 更多菜单。
    if is_collapsed {
        let snippet = first_snippet_line(body_text);
        let has_snippet = !snippet.is_empty();
        return h_flex()
            .bg(p.surface_raised)
            .rounded(px(12.0))
            .border_1()
            .border_color(if is_current {
                accent.opacity(0.45)
            } else {
                border
            })
            .shadow_xs()
            .px(px(20.0))
            .py(px(12.0))
            .gap_2()
            .items_center()
            .child(
                h_flex()
                    .id(("reader-card-hdr", msg_id))
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_thread_card_collapsed(msg_id, cx);
                    }))
                    .child(avatar::avatar(&display_name, &sender_email, 28.0))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_sm()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .text_color(p.text)
                            .child(SharedString::from(display_name)),
                    )
                    .when(has_snippet, |el| {
                        el.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_xs()
                                .text_color(muted_fg)
                                .truncate()
                                .child(SharedString::from(snippet)),
                        )
                    })
                    .when(!has_snippet, |el| el.child(div().flex_1()))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(muted_fg)
                            .child(SharedString::from(formatted_date.clone())),
                    ),
            )
            .child(more_menu)
            .into_any_element();
    }

    let cc_list = recipient_list(cc);
    let recipient_list = recipient_list(recipients);
    let zoom_scale = (zoom_percent.clamp(50, 250) as f32) / 100.0;
    let body_font_px = 14.0 * zoom_scale;
    let forced_dark = bg_pick.or(forced_theme_bg);
    let (body_bg, body_fg) = theme::reader_body_colors(forced_dark.unwrap_or(app.dark_mode));
    let body_muted = body_fg.opacity(0.75);
    let (card_bg, _) = theme::reader_body_colors(app.dark_mode);

    // 决定原生富文本渲染内容：有 HTML 且未切纯文本模式时渲染净化后的 HTML，
    // 否则将纯文本安全转义后通过 TextView::new(&state) 渲染，
    // 并拆出历史引用块以支持默认折叠（`⋯ 显示引用内容 (N 行)`）。
    let (main_html, quoted_part, blocked_images): (String, Option<(String, usize)>, usize) =
        if show_raw_source {
            let raw = format_raw_source(
                rfc_message_id,
                sender,
                recipients,
                cc,
                &formatted_date,
                subject,
                body_text,
                body_html,
            );
            (plain_text_to_html(&raw), None, 0)
        } else if !prefer_plain {
            if let Some(raw_html) = body_html.filter(|s| !s.trim().is_empty()) {
                let (sanitized, blocked) = sanitize_html_for_display_with_options(
                    raw_html,
                    block_remote_images,
                    clean_links,
                );
                if body_view == BodyView::Native {
                    let split = split_quoted_html(&sanitized);
                    (split.main, split.quoted, blocked)
                } else {
                    (String::new(), None, blocked)
                }
            } else {
                let split = split_quoted_plain(body_text);
                (
                    plain_text_to_html(&split.main),
                    split
                        .quoted
                        .map(|(q, lines)| (plain_text_to_html(&q), lines)),
                    0,
                )
            }
        } else {
            let split = split_quoted_plain(body_text);
            (
                plain_text_to_html(&split.main),
                split
                    .quoted
                    .map(|(q, lines)| (plain_text_to_html(&q), lines)),
                0,
            )
        };
    let text_state: Option<Entity<TextViewState>> = if body_view != BodyView::Native {
        app.reader_text_states.remove(&msg_id);
        None
    } else {
        Some(app.ensure_reader_text_state(msg_id, &main_html, cx))
    };

    let card_attachment_ids: Vec<u32> = attachments.iter().map(|a| a.id).collect();
    let attachment_rows: Vec<gpui_kit::AnyElement> = attachments
        .iter()
        .map(|att| {
            let att_id = att.id;
            let filename = att.filename.clone();
            let size_text = hylki_mail_core::config::format_bytes(att.size_bytes as u64);
            let size_label = if att.downloaded {
                size_text
            } else {
                format!("约 {size_text} · 未下载")
            };
            let save_tooltip = if att.downloaded {
                "保存附件"
            } else {
                "下载并保存附件"
            };
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
                        .child(SharedString::from(size_label)),
                )
                .child(
                    Button::new(("save-att", att_id))
                        .ghost()
                        .compact()
                        .icon(IconName::Download)
                        .tooltip(save_tooltip)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.save_attachment(att_id, &filename, cx);
                        })),
                )
                .into_any_element()
        })
        .collect();

    v_flex()
        .when(
            body_view != BodyView::Native && !webview_in_thread,
            |card| card.flex_1().min_h_0(),
        )
        .when(card_chrome, |card| {
            card.bg(card_bg)
                .rounded(px(12.0))
                .border_1()
                .border_color(if is_current {
                    accent.opacity(0.45)
                } else {
                    border
                })
                .shadow_xs()
                .p(px(20.0))
        })
        .when(!card_chrome, |card| card.bg(card_bg).py_2())
        .gap_3()
        .child(
            v_flex()
                .gap_1()
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(
                            h_flex()
                                .id(("reader-card-hdr", msg_id))
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .items_center()
                                .gap_2()
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.toggle_thread_card_collapsed(msg_id, cx);
                                }))
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
                                                    .child(SharedString::from(format!(
                                                        "<{sender_email}>"
                                                    ))),
                                            )
                                        }),
                                )
                                .when_some(trust_badge, |el, (label, color)| {
                                    el.child(
                                        div()
                                            .flex_shrink_0()
                                            .px_2()
                                            .py_0p5()
                                            .rounded_full()
                                            .bg(canvas)
                                            .border_1()
                                            .border_color(color.opacity(0.45))
                                            .text_color(color)
                                            .text_xs()
                                            .child(SharedString::from(label)),
                                    )
                                })
                                .child(div().flex_1())
                                .child(
                                    div()
                                        .flex_shrink_0()
                                        .text_xs()
                                        .text_color(muted_fg)
                                        .child(SharedString::from(formatted_date)),
                                ),
                        )
                        .child(more_menu),
                )
                .when(
                    always_show_recipients && (!recipient_list.is_empty() || !cc_list.is_empty()),
                    |el| {
                        el.child(
                            v_flex()
                                // 与发件人姓名块左对齐：头像宽度 + 头部 gap_2。
                                .pl(px(CARD_AVATAR + 8.0))
                                .gap_0p5()
                                .text_xs()
                                .text_color(muted_fg)
                                .when(!recipient_list.is_empty(), |row| {
                                    row.child(div().truncate().child(SharedString::from(format!(
                                        "收件人：{recipient_list}"
                                    ))))
                                })
                                .when(!cc_list.is_empty(), |row| {
                                    row.child(
                                        div()
                                            .truncate()
                                            .child(SharedString::from(format!("抄送：{cc_list}"))),
                                    )
                                }),
                        )
                    },
                ),
        )
        .when(show_raw_source, |card| {
            card.child(
                h_flex()
                    .px_3()
                    .py_1()
                    .items_center()
                    .justify_between()
                    .rounded(px(8.0))
                    .bg(canvas)
                    .border_1()
                    .border_color(border)
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui_kit::FontWeight::MEDIUM)
                            .text_color(muted_fg)
                            .child("邮件原文视图（原始信头与正文源码）"),
                    )
                    .child(
                        Button::new(("exit-raw-source", msg_id))
                            .outline()
                            .compact()
                            .label("返回正文")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_raw_source(msg_id, cx);
                            })),
                    ),
            )
        })
        .when_some(spoof_banner, |card, warning| {
            card.child(
                h_flex()
                    .px_3()
                    .py_1p5()
                    .gap_2()
                    .items_center()
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(p.danger.opacity(0.45))
                    .bg(p.danger.opacity(0.10))
                    .child(
                        Icon::new(IconName::TriangleAlert)
                            .with_size(px(14.0))
                            .text_color(p.danger),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.danger)
                            .child(SharedString::from(warning)),
                    ),
            )
        })
        .when(has_mdn_header, |card| {
            card.child(
                h_flex()
                    .px_3()
                    .py_1p5()
                    .gap_2()
                    .items_center()
                    .justify_between()
                    .rounded(px(8.0))
                    .bg(canvas)
                    .border_1()
                    .border_color(border)
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.text)
                            .child("发件人请求已读回执（RFC 8098）"),
                    )
                    .child(
                        h_flex()
                            .gap_1p5()
                            .child(
                                Button::new(("send-mdn", msg_id))
                                    .outline()
                                    .compact()
                                    .label("发送回执")
                                    .on_click(cx.listener(|_this, _, _, cx| {
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new(("ignore-mdn", msg_id))
                                    .ghost()
                                    .compact()
                                    .label("忽略")
                                    .on_click(cx.listener(|_this, _, _, cx| {
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
        })
        .when(detected_invite.is_some() || has_ics_attachment, |card| {
            let title = detected_invite
                .as_ref()
                .map(|i| i.summary.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "日历会议邀请 (.ics)".to_string());
            let loc = detected_invite
                .as_ref()
                .map(|i| i.location.clone())
                .unwrap_or_default();
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
            let inv_for_accept = detected_invite.clone();
            let inv_for_tentative = detected_invite.clone();
            let inv_for_decline = detected_invite.clone();
            card.child(
                v_flex()
                    .px_3()
                    .py_2()
                    .gap_1p5()
                    .rounded(px(8.0))
                    .bg(canvas)
                    .border_1()
                    .border_color(border)
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                    .text_color(p.text)
                                    .child(SharedString::from(title)),
                            )
                            .child(
                                h_flex()
                                    .gap_1p5()
                                    .child(
                                        Button::new(("ics-accept", msg_id))
                                            .outline()
                                            .compact()
                                            .label("接受")
                                            .on_click(cx.listener(move |_this, _, _, cx| {
                                                if let Some(ref inv) = inv_for_accept {
                                                    let _ = hylki_mail_core::invite::reply_ics(
                                                        inv, "", "", "ACCEPTED",
                                                    );
                                                }
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        Button::new(("ics-tentative", msg_id))
                                            .outline()
                                            .compact()
                                            .label("暂定")
                                            .on_click(cx.listener(move |_this, _, _, cx| {
                                                if let Some(ref inv) = inv_for_tentative {
                                                    let _ = hylki_mail_core::invite::reply_ics(
                                                        inv,
                                                        "",
                                                        "",
                                                        "TENTATIVE",
                                                    );
                                                }
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        Button::new(("ics-decline", msg_id))
                                            .outline()
                                            .compact()
                                            .label("拒绝")
                                            .on_click(cx.listener(move |_this, _, _, cx| {
                                                if let Some(ref inv) = inv_for_decline {
                                                    let _ = hylki_mail_core::invite::reply_ics(
                                                        inv, "", "", "DECLINED",
                                                    );
                                                }
                                                cx.notify();
                                            })),
                                    ),
                            ),
                    )
                    .when(!loc.is_empty() || !org.is_empty(), |el| {
                        let meta = [
                            (!org.is_empty()).then(|| format!("组织者：{org}")),
                            (!loc.is_empty()).then(|| format!("地点：{loc}")),
                        ]
                        .into_iter()
                        .flatten()
                        .collect::<Vec<_>>()
                        .join(" · ");
                        el.child(
                            div()
                                .text_xs()
                                .text_color(muted_fg)
                                .child(SharedString::from(meta)),
                        )
                    }),
            )
        })
        .when(
            !show_raw_source && blocked_images > 0 && show_remote_banner,
            |card| {
                let sender_str = sender.to_string();
                card.child(
                    h_flex()
                        .px_3()
                        .py_1p5()
                        .gap_2()
                        .items_center()
                        .justify_between()
                        .rounded(px(8.0))
                        .bg(p.accent_soft())
                        .child(div().text_xs().text_color(p.text).child(SharedString::from(
                            format!("已拦截 {blocked_images} 张远程图片以保护隐私"),
                        )))
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    Button::new(("allow-remote-img", msg_id))
                                        .outline()
                                        .compact()
                                        .label("显示本次图片")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.allow_remote_images_for(msg_id, cx);
                                        })),
                                )
                                .child(
                                    Button::new(("trust-remote-sender", msg_id))
                                        .outline()
                                        .compact()
                                        .label("总是信任此发件人")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.allow_remote_images_for(msg_id, cx);
                                            this.trust_remote_sender(&sender_str, cx);
                                        })),
                                ),
                        ),
                )
            },
        )
        .when_some(text_state, |card, text_state| {
            card.child({
                div()
                    .when(show_raw_source, |el| {
                        el.p_3()
                            .rounded(px(8.0))
                            .bg(canvas)
                            .border_1()
                            .border_color(border)
                            .text_size(px((12.0 * zoom_scale).max(10.0)))
                    })
                    .when(!show_raw_source, |el| el.text_size(px(body_font_px)))
                    .when(forced_dark.is_some(), |el| {
                        el.bg(body_bg)
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(border)
                            .p_2()
                    })
                    .text_color(if body_loaded { body_fg } else { body_muted })
                    .child(
                        TextView::new(&text_state)
                            .image_source(move |uri| {
                                crate::webview_host::mail_image_source(uri, !block_remote_images)
                            })
                            .on_link_click(move |url, event, _, cx| {
                                HylkiApp::open_clicked_mail_link(&app_for_link, url, event, cx);
                            })
                            .link_tooltip(|url, _, cx| {
                                let url = url.clone();
                                cx.new(move |_| LinkPreviewTooltip { url: url.clone() })
                                    .into()
                            }),
                    )
            })
        })
        .when(body_view == BodyView::Loading, |card| {
            card.child(
                body_loading(body_bg, body_muted, body_pending)
                    .when(webview_in_thread, |el| {
                        el.h(px(500.0)).justify_start().pt_4()
                    })
                    .when(!webview_in_thread, |el| el.flex_1().min_h_0()),
            )
        })
        .when(body_view == BodyView::Html, |card| {
            #[cfg(windows)]
            {
                let host = app.reader_webview.as_ref().unwrap().clone();
                let (version, _) = host.load_status();
                let visible = !app.reader_webview_yields_to_popup();
                if let Some(snapshot) = host.popup_snapshot() {
                    // 浮层打开期间：同一布局位置换成正文快照，尺寸与子窗口一致。
                    use gpui_kit::StyledImage as _;

                    let img_el = if let Some((ox, oy, w, h)) = snapshot.clipped_rect_px {
                        gpui_kit::img(snapshot.image)
                            .absolute()
                            .left(px(ox as f32 / scale_factor))
                            .top(px(oy as f32 / scale_factor))
                            .w(px(w as f32 / scale_factor))
                            .h(px(h as f32 / scale_factor))
                            .object_fit(gpui_kit::ObjectFit::Fill)
                    } else {
                        gpui_kit::img(snapshot.image)
                            .absolute()
                            .inset_0()
                            .size_full()
                            .object_fit(gpui_kit::ObjectFit::Fill)
                    };
                    return card.child(
                        div()
                            .relative()
                            .overflow_hidden()
                            .w_full()
                            .when(webview_in_thread, |el| el.h(px(500.0)))
                            .when(!webview_in_thread, |el| el.flex_1().min_h_0())
                            .child(img_el),
                    );
                }
                let app_reserved_strip_px = app.reader_webview_bottom_clearance_px();
                return card.child(
                    gpui_kit::canvas(
                        move |bounds, window, cx| {
                            let scale = window.scale_factor();
                            let rect = windows::Win32::Foundation::RECT {
                                left: (bounds.origin.x.as_f32() * scale).round() as i32,
                                top: (bounds.origin.y.as_f32() * scale).round() as i32,
                                right: ((bounds.origin.x + bounds.size.width).as_f32() * scale)
                                    .round() as i32,
                                bottom: ((bounds.origin.y + bounds.size.height).as_f32() * scale)
                                    .round() as i32,
                            };
                            let mask = window.content_mask().bounds;
                            let mut clip = windows::Win32::Foundation::RECT {
                                left: (mask.origin.x.as_f32() * scale).round() as i32,
                                top: (mask.origin.y.as_f32() * scale).round() as i32,
                                right: ((mask.origin.x + mask.size.width).as_f32() * scale).round()
                                    as i32,
                                bottom: ((mask.origin.y + mask.size.height).as_f32() * scale)
                                    .round() as i32,
                            };
                            let reserved_bottom = app_reserved_strip_px;
                            if reserved_bottom > 0.0 {
                                let max_bottom =
                                    ((window.viewport_size().height.as_f32() - reserved_bottom)
                                        * scale)
                                        .round() as i32;
                                clip.bottom = clip.bottom.min(max_bottom);
                            }
                            cx.defer(move |_| {
                                if host.load_status() == (version, BodyView::Html) {
                                    host.set_clipped_bounds(rect, clip);
                                    host.set_visible(visible);
                                }
                            });
                        },
                        |_, _, _, _| {},
                    )
                    .when(webview_in_thread, |el| el.h(px(500.0)))
                    .when(!webview_in_thread, |el| el.flex_1().min_h_0())
                    .w_full(),
                );
            }
            #[cfg(not(windows))]
            card
        })
        .when_some(quoted_part, |card, (q_html, lines)| {
            card.child(
                v_flex()
                    .gap_2()
                    .child(
                        Button::new(("quote-fold-btn", msg_id))
                            .outline()
                            .compact()
                            .label(SharedString::from(if quote_expanded {
                                format!("▾ 隐藏引用内容 ({lines} 行)")
                            } else {
                                format!("⋯ 显示引用内容 ({lines} 行)")
                            }))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_quote_expanded(msg_id, cx);
                            })),
                    )
                    .when(quote_expanded, |c| {
                        c.child(
                            div()
                                .pl_3()
                                .bg(body_bg)
                                .border_l_2()
                                .border_color(accent.opacity(0.45))
                                .text_color(body_muted)
                                .text_size(px((body_font_px - 1.0).max(11.0)))
                                .child(
                                    TextView::html(
                                        SharedString::from(format!("reader-quote-{msg_id}")),
                                        q_html,
                                    )
                                    .selectable(true)
                                    .image_source(move |uri| {
                                        crate::webview_host::mail_image_source(
                                            uri,
                                            !block_remote_images,
                                        )
                                    })
                                    .on_link_click(move |url, event, _, cx| {
                                        HylkiApp::open_clicked_mail_link(
                                            &app_for_quote_link,
                                            url,
                                            event,
                                            cx,
                                        );
                                    })
                                    .link_tooltip(
                                        |url, _, cx| {
                                            let url = url.clone();
                                            cx.new(move |_| LinkPreviewTooltip { url: url.clone() })
                                                .into()
                                        },
                                    ),
                                ),
                        )
                    }),
            )
        })
        .when(!body_loaded && !body_pending, |card| {
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
        .when(card_attachments && !attachment_rows.is_empty(), |card| {
            let show_save_all = card_attachment_ids.len() >= 2;
            let save_all_ids = card_attachment_ids;
            card.child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .items_center()
                    .pt_2()
                    .border_t_1()
                    .border_color(border)
                    .children(attachment_rows)
                    .when(show_save_all, |row| {
                        row.child(
                            Button::new(("save-all-att-card", msg_id))
                                .outline()
                                .compact()
                                .icon(IconName::Download)
                                .label("保存全部附件")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.save_all_attachments(save_all_ids.clone(), cx);
                                })),
                        )
                    }),
            )
        })
        .into_any_element()
}

#[cfg(windows)]
fn webview_eligible(
    html: Option<&str>,
    body_pending: bool,
    prefer_plain: bool,
    raw_source: bool,
    collapsed: bool,
) -> bool {
    html.is_some_and(crate::webview_host::supports_document)
        && !body_pending
        && !prefer_plain
        && !raw_source
        && !collapsed
}

pub(crate) fn render(
    app: &mut HylkiApp,
    window: &Window,
    cx: &mut Context<HylkiApp>,
) -> gpui_kit::AnyElement {
    // L-6：阅读器在下方时列表顶栏即窗口顶栏，此处不再重复画窗口控制按钮。
    let pane_top = app.pane_at_window_top();
    render_with_pane_top(app, window, cx, pane_top)
}

/// 同 [`render`]，但由调用方决定是否在阅读器顶栏画窗口控制按钮。
///
/// WP-10-4：内联分屏回复时阅读器在窗格下半部分，窗口控制按钮由上方写信面板承载。
pub(crate) fn render_with_pane_top(
    app: &mut HylkiApp,
    window: &Window,
    cx: &mut Context<HylkiApp>,
    pane_top: bool,
) -> gpui_kit::AnyElement {
    // 避坑清单第 4 条：render 开头先把语义色 Copy 出来。
    let p = theme::current(cx);
    let (canvas, border, muted_fg, accent) = (p.canvas, p.border, p.text_muted, p.accent);
    // B-5：窗口宽度低于阈值、偏好「始终折叠」或专注模式偏好触发折叠时收进「⋯」菜单。
    let narrow_topbar = app.prefs.reader_toolbar_fold_right
        || (app.focus_mode && app.prefs.focus_fold_toolbar)
        || window.viewport_size().width.as_f32() < READER_OVERFLOW_BREAKPOINT;
    let entity_for_overflow = cx.entity();
    let win_controls = window_controls::render_window_controls("reader-wc", window, cx);

    let Some(detail) = app.selected_message.clone() else {
        return v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(canvas)
            .child(
                h_flex()
                    .h(px(HEADER_HEIGHT))
                    .flex_shrink_0()
                    .items_center()
                    .border_b_1()
                    .border_color(border)
                    .child(window_controls::drag_spacer())
                    .when(pane_top, |bar| bar.child(win_controls)),
            )
            .child(
                v_flex()
                    .flex_1()
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
                    ),
            )
            .into_any_element();
    };

    let owner_label = app.account_label_for(detail.account_id);
    let owner_color = app.account_color_for(detail.account_id).unwrap_or(accent);
    let flagged = detail.flagged;
    let is_unread = app.selected_message_unread();
    let prefer_plain = app.reader_prefer_plain;
    let zoom_percent = app.prefs.reader_zoom_percent.clamp(50, 250);
    let forced_theme_bg = match app.prefs.message_theme.as_str() {
        "light" => Some(false),
        "dark" => Some(true),
        _ => None,
    };
    let single_message_card = app.prefs.single_message_card;
    let thread_count = detail.thread_messages.len();
    // 占位详情：正文仍在远端加载中，显示加载提示而不是“加载完整正文”按钮。
    let body_pending = app.pending_body_message_id == Some(detail.id);

    #[cfg(windows)]
    let body_view = {
        use crate::webview_host::{build_body_document, DocumentAppearance};
        use std::hash::{Hash, Hasher};

        let in_thread = thread_count > 1;
        let anchor = detail
            .thread_messages
            .iter()
            .find(|message| message.id == detail.id);
        let html = if thread_count > 1 {
            anchor.and_then(|message| message.body_html.as_deref())
        } else {
            detail.body_html.as_deref()
        };
        let body_pending = body_pending
            || (thread_count > 1 && !anchor.is_some_and(|message| message.body_loaded));
        let block_remote = !app.remote_images_allowed(detail.id, &detail.sender);
        let eligible = webview_eligible(
            html,
            body_pending,
            prefer_plain,
            app.raw_source_message_ids.contains(&detail.id),
            app.collapsed_thread_cards.contains(&detail.id),
        );
        // 浮层打开期间正文由 WebView2 快照顶替（见 WebViewHost::set_popup_open），
        // 这里只管其它会盖住正文或被正文盖住的界面状态。
        let unobscured = app.active_modal == crate::app::ActiveModal::None
            && app.print_preview.is_none()
            && !app.reader_find_open
            && !app.compose_split
            && !app.sidebar_peek_open
            && app.drawer_image_preview.is_none()
            && !app.mail_dir_switching;
        let forced_dark = app
            .message_bg_picks
            .get(&detail.id)
            .copied()
            .or(forced_theme_bg);
        let dark = forced_dark.unwrap_or(app.dark_mode);
        let (background, foreground) = theme::reader_body_colors(dark);
        let appearance = DocumentAppearance {
            dark,
            zoom_percent,
            background,
            foreground,
        };
        let key = html.filter(|_| eligible).map(|html| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            detail.id.hash(&mut hasher);
            html.hash(&mut hasher);
            detail.subject.hash(&mut hasher);
            appearance.hash(&mut hasher);
            block_remote.hash(&mut hasher);
            in_thread.hash(&mut hasher);
            hasher.finish()
        });
        if key.is_none() && app.reader_webview_key.take().is_some() {
            if let Some(host) = app.reader_webview.clone() {
                cx.defer_in(window, move |this, _, _| {
                    if this.reader_webview_key.is_none() {
                        let _ = host.load(b"<!doctype html>".to_vec());
                    }
                });
            }
        }
        let new_key = key.filter(|key| app.reader_webview_key != Some(*key));
        if let Some(key) = new_key {
            let document = build_body_document(html.unwrap(), appearance);
            app.reader_webview_key = Some(key);
            let message_id = detail.id;
            cx.defer_in(window, move |this, window, cx| {
                if this.reader_webview_key != Some(key)
                    || this.selected_message.as_ref().map(|message| message.id) != Some(message_id)
                {
                    return;
                }
                if let Some(host) = this.ensure_reader_webview(window, cx) {
                    let _ = host.load_document(document, !block_remote, in_thread);
                }
                cx.notify();
            });
        }
        let view = BodyView::for_document(
            eligible && unobscured,
            new_key.is_some(),
            app.reader_webview.as_deref(),
        );
        if let Some(host) = app.reader_webview.clone() {
            if view != BodyView::Html || app.reader_webview_yields_to_popup() {
                cx.defer_in(window, move |_, _, _| host.set_visible(false));
            }
        }
        view
    };
    #[cfg(not(windows))]
    let body_view = BodyView::Native;
    let body_view = if body_pending {
        BodyView::Loading
    } else {
        body_view
    };

    // 汇总当前会话全部附件 ID（用于顶部「保存全部附件」入口）。
    let all_thread_attachment_ids: Vec<u32> = if thread_count > 1 {
        detail
            .thread_messages
            .iter()
            .flat_map(|tm| tm.attachments.iter().map(|a| a.id))
            .collect()
    } else {
        detail.attachments.iter().map(|a| a.id).collect()
    };
    let any_card_collapsed = thread_count > 1
        && detail
            .thread_messages
            .iter()
            .any(|tm| app.collapsed_thread_cards.contains(&tm.id));

    // B-5：标题区亮 / 暗按钮的当前态与目标邮件（会话线程下作用于锚点邮件本身）。
    let bg_msg_id = detail.id;
    let bg_msg_picked_dark = app
        .message_bg_picks
        .get(&bg_msg_id)
        .copied()
        .unwrap_or(app.dark_mode);

    let date_style = app.prefs.date_style.clone();
    let clock_style = app.prefs.clock_style.clone();
    let clean_links = app.prefs.clean_links;
    let show_remote_banner = app.prefs.show_remote_banner;
    let always_show_recipients = app.prefs.always_show_recipients;
    let card_attachments = app.prefs.card_attachments;
    let show_account_chip = app.prefs.reader_show_account_chip;
    let show_view_switch = app.prefs.reader_show_view_switch;
    let show_theme_switch = app.prefs.reader_show_theme_switch;

    // 构造单卡或多线程叠放卡片列表：每张卡片均带独立的 `...` 操作菜单与折叠/原文态。
    let cards: Vec<gpui_kit::AnyElement> = if thread_count > 1 {
        detail
            .thread_messages
            .iter()
            .map(|tm: &ThreadMessage| {
                let is_current = tm.id == detail.id;
                let is_collapsed = app.collapsed_thread_cards.contains(&tm.id);
                let show_raw_source = app.raw_source_message_ids.contains(&tm.id);
                let block_remote = !app.remote_images_allowed(tm.id, &tm.sender);
                let card_subject = if tm.subject.trim().is_empty() {
                    detail.subject.clone()
                } else {
                    tm.subject.clone()
                };
                // 退订信息只在当前打开（锚点）邮件的 detail 中；会话其余卡片
                // 无详情数据，维持 None（横幅仅出现在锚点卡片上）。
                let is_anchor = tm.id == detail.id;
                let card_unsubscribe_url = if is_anchor {
                    detail.unsubscribe_url.as_deref()
                } else {
                    None
                };
                let card_unsubscribe_one_click = is_anchor && detail.unsubscribe_one_click;
                let card_detail = MessageDetail {
                    id: tm.id,
                    account_id: detail.account_id,
                    folder_id: detail.folder_id,
                    // 卡片自身的 UID / 文件夹（旧缓存数据缺省时回落锚点值）。
                    folder_path: if tm.folder_path.is_empty() {
                        detail.folder_path.clone()
                    } else {
                        tm.folder_path.clone()
                    },
                    uid: if tm.uid > 0 { tm.uid } else { detail.uid },
                    rfc_message_id: tm.rfc_message_id.clone(),
                    sender: tm.sender.clone(),
                    recipients: tm.recipients.clone(),
                    cc: tm.cc.clone(),
                    subject: card_subject.clone(),
                    date_display: tm.date_display.clone(),
                    date_ts: tm.date_ts,
                    body_text: tm.body_text.clone(),
                    body_html: tm.body_html.clone(),
                    flagged: tm.flagged,
                    attachments: tm.attachments.clone(),
                    thread_messages: Vec::new(),
                    unsubscribe_url: card_unsubscribe_url.map(str::to_string),
                    unsubscribe_one_click: card_unsubscribe_one_click,
                    auth_results: if is_anchor {
                        detail.auth_results.clone()
                    } else {
                        None
                    },
                };
                let eml_exporting = app.pending_eml_export == Some(tm.id);
                let menu = build_card_more_menu(
                    card_detail,
                    show_raw_source,
                    app.demo_mode,
                    eml_exporting,
                    cx,
                );
                render_message_card(
                    detail.account_id,
                    tm.id,
                    &tm.rfc_message_id,
                    &card_subject,
                    &tm.sender,
                    &tm.recipients,
                    &tm.cc,
                    if is_anchor {
                        detail.auth_results.as_deref()
                    } else {
                        None
                    },
                    &tm.date_display,
                    tm.date_ts,
                    &date_style,
                    &clock_style,
                    &tm.body_text,
                    tm.body_html.as_deref(),
                    tm.body_loaded,
                    is_current && body_pending,
                    prefer_plain,
                    block_remote,
                    clean_links,
                    show_remote_banner,
                    is_collapsed,
                    show_raw_source,
                    app.expanded_quote_message_ids.contains(&tm.id),
                    zoom_percent,
                    app.message_bg_picks.get(&tm.id).copied(),
                    forced_theme_bg,
                    true,
                    always_show_recipients,
                    card_attachments,
                    &tm.attachments,
                    is_current,
                    menu,
                    if is_current {
                        body_view
                    } else {
                        BodyView::Native
                    },
                    true,
                    window.scale_factor(),
                    &p,
                    app,
                    cx,
                )
            })
            .collect()
    } else {
        let is_collapsed = app.collapsed_thread_cards.contains(&detail.id);
        let show_raw_source = app.raw_source_message_ids.contains(&detail.id);
        let block_remote = !app.remote_images_allowed(detail.id, &detail.sender);
        let eml_exporting = app.pending_eml_export == Some(detail.id);
        let menu = build_card_more_menu(
            detail.clone(),
            show_raw_source,
            app.demo_mode,
            eml_exporting,
            cx,
        );
        vec![render_message_card(
            detail.account_id,
            detail.id,
            &detail.rfc_message_id,
            &detail.subject,
            &detail.sender,
            &detail.recipients,
            &detail.cc,
            detail.auth_results.as_deref(),
            &detail.date_display,
            detail.date_ts,
            &date_style,
            &clock_style,
            &detail.body_text,
            detail.body_html.as_deref(),
            !body_pending,
            body_pending,
            prefer_plain,
            block_remote,
            clean_links,
            show_remote_banner,
            is_collapsed,
            show_raw_source,
            app.expanded_quote_message_ids.contains(&detail.id),
            zoom_percent,
            app.message_bg_picks.get(&detail.id).copied(),
            forced_theme_bg,
            single_message_card,
            always_show_recipients,
            card_attachments,
            &detail.attachments,
            false,
            menu,
            body_view,
            false,
            window.scale_factor(),
            &p,
            app,
            cx,
        )]
    };

    // WP-09：收集底部附件抽屉条目并触发图片附件缩略图懒加载。
    let drawer_items: Vec<DrawerAttachment> = if thread_count > 1 {
        detail
            .thread_messages
            .iter()
            .flat_map(|tm| {
                let mid = tm.id;
                let aid = detail.account_id;
                tm.attachments.iter().map(move |a| DrawerAttachment {
                    account_id: aid,
                    message_id: mid,
                    meta: a.clone(),
                })
            })
            .collect()
    } else {
        detail
            .attachments
            .iter()
            .map(|a| DrawerAttachment {
                account_id: detail.account_id,
                message_id: detail.id,
                meta: a.clone(),
            })
            .collect()
    };
    if !app.attachment_drawer_collapsed {
        for item in &drawer_items {
            app.ensure_attachment_thumbnail(
                item.meta.id,
                &item.meta.mime_type,
                item.meta.downloaded,
            );
        }
    }
    let drawer_element = attachment_drawer::render_drawer_with_feedback(
        &drawer_items,
        app.attachment_drawer_collapsed,
        app.attachment_drawer_height,
        app.attachment_drawer_list_view,
        app.attachment_drawer_sort_desc,
        app.demo_mode,
        &app.attachment_thumb_cache,
        &app.stripping_attachment_ids,
        app.pending_strip_attachment_id,
        attachment_drawer::DrawerFeedback {
            downloading_ids: Some(&app.downloading_attachment_ids),
            saving_all: app.saving_all_attachments,
            item_errors: Some(&app.attachment_item_errors),
            drawer_error: app.attachment_drawer_error.as_ref(),
        },
        cx,
    );

    let find_open = app.reader_find_open;
    let find_input = app.reader_find_input.clone();
    let find_query = app.reader_find_query.clone();
    let find_active = app.reader_find_active_match;
    let find_total = app.reader_find_total_matches;
    let print_detail = detail.clone();
    let popout_detail = detail.clone();
    let overflow_print_detail = detail.clone();
    let overflow_popout_detail = detail.clone();

    v_flex()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(canvas)
        .on_drag_move(
            cx.listener(|this, event: &DragMoveEvent<AttachmentDrawerDrag>, _, cx| {
                this.on_attachment_drawer_drag_move(event, cx);
            }),
        )
        // WP-01：点击阅读器不再使列表失焦（对齐 Linux theme.rs：选中行保持
        // accent 纯色高亮，仅写信编辑器 / 搜索输入持焦点时才转中性灰）。
        // 顶部操作栏 48px（空白区可拖拽窗口，最右侧为 Windows 原生窗口控制按钮）。
        .child(
            h_flex()
                .h(px(HEADER_HEIGHT))
                .flex_shrink_0()
                .pl_3()
                .gap_1()
                .items_center()
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
                                // 激活（已加星标）时统一用琥珀金星标色。
                                .when(flagged, |btn| btn.text_color(theme::star_gold()))
                                .tooltip(if flagged { "取消星标" } else { "加星标" })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.toggle_flagged(&star_detail, cx);
                                })),
                        )
                        // 迭代 2：归档 / 删除 / 标垃圾 / 移动到文件夹接线（IMAP MOVE / Graph move）；
                        // 演示模式不支持服务端移动，隐藏这些入口。
                        .when(!app.demo_mode, |el| {
                            let move_account_id = detail.account_id;
                            let move_message_id = detail.id;
                            let current_folder_path = detail.folder_path.clone();
                            let account_folders: Vec<(String, String)> = app
                                .folders_by_account
                                .get(&move_account_id)
                                .map(|list| {
                                    list.iter()
                                        .map(|f| (f.name.clone(), f.path.clone()))
                                        .collect()
                                })
                                .unwrap_or_default();
                            let move_entity = cx.entity();
                            let popup_entity = cx.entity();
                            el.child(
                                action_button("reader-archive", IconName::Archive, "归档")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.archive_selected(cx);
                                    })),
                            )
                            .child(
                                action_button("reader-spam", IconName::Ban, "标记为垃圾邮件")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.mark_spam_selected(cx);
                                    })),
                            )
                            .child(
                                Button::new("reader-move-to")
                                    .ghost()
                                    .compact()
                                    .icon(IconName::FolderClosed)
                                    .tooltip("移动到文件夹...")
                                    .dropdown_menu(move |mut menu, _, _| {
                                        if account_folders.is_empty() {
                                            menu = menu.item(
                                                PopupMenuItem::new("暂无可用文件夹").disabled(true),
                                            );
                                            return menu;
                                        }
                                        for (folder_name, folder_path) in &account_folders {
                                            let is_current = *folder_path == current_folder_path;
                                            let target_path = folder_path.clone();
                                            let entity_for_item = move_entity.clone();
                                            menu = menu.item(
                                                PopupMenuItem::new(folder_name.clone())
                                                    .disabled(is_current)
                                                    .when(is_current, |item| {
                                                        item.icon(Icon::new(IconName::Check))
                                                    })
                                                    .on_click(move |_, _, cx| {
                                                        let path = target_path.clone();
                                                        entity_for_item.update(cx, |this, cx| {
                                                            this.move_message(
                                                                move_account_id,
                                                                move_message_id,
                                                                path,
                                                                cx,
                                                            );
                                                        });
                                                    }),
                                            );
                                        }
                                        menu
                                    })
                                    .on_open_change(move |open, _, cx| {
                                        popup_entity.update(cx, |app, cx| {
                                            app.set_reader_popup_open(*open);
                                            cx.notify();
                                        });
                                    }),
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
                .child(window_controls::drag_spacer())
                // 右侧操作组：动态标记已读/未读 + 复制正文（严格放在 48px 顶栏内）。
                // 缩放改为 Ctrl+= / Ctrl+- / Ctrl+0 快捷键 + 标题区缩放胶囊（S-8），不再常驻顶栏。
                // B-5：HTML/纯文本切换升级为标题区「阅读视图」按钮（低频且与阅读视图语义重合）；
                // 窗口宽度低于 READER_OVERFLOW_BREAKPOINT（880px，≈三栏最小宽度之和的下限）
                // 时剩余低频动作收进「⋯」下拉，宽窗口仍平铺（对齐原版 reader 顶栏折叠 #475）。
                .child(
                    h_flex()
                        .gap_1()
                        .pr_1()
                        .items_center()
                        .when(!narrow_topbar, |el| {
                            let d_print = print_detail.clone();
                            let d_pop = popout_detail.clone();
                            el.child(
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
                                action_button("reader-copy-body", IconName::Copy, "复制正文")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.copy_selected_body(cx);
                                    })),
                            )
                            .child(
                                Button::new("reader-find-btn")
                                    .ghost()
                                    .compact()
                                    .icon(IconName::Search)
                                    .selected(find_open)
                                    .tooltip("页内查找 (Ctrl+F)")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.toggle_reader_find(window, cx);
                                    })),
                            )
                            .child(
                                action_button(
                                    "reader-print-btn",
                                    IconName::FileText,
                                    "打印 / 导出 PDF",
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.open_print_preview(d_print.clone(), cx);
                                    },
                                )),
                            )
                            .child(
                                action_button(
                                    "reader-popout-btn",
                                    IconName::ExternalLink,
                                    "在新窗口打开邮件",
                                )
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.pop_out_message_window(d_pop.clone(), cx);
                                    },
                                )),
                            )
                        })
                        .when(narrow_topbar, |el| {
                            let popup_entity = cx.entity();
                            let d_print = overflow_print_detail.clone();
                            let d_pop = overflow_popout_detail.clone();
                            el.child(
                                Button::new("reader-overflow")
                                    .ghost()
                                    .compact()
                                    .icon(IconName::Ellipsis)
                                    .tooltip("更多邮件操作")
                                    .dropdown_menu(move |menu, _, _| {
                                        let e_seen = entity_for_overflow.clone();
                                        let e_copy = entity_for_overflow.clone();
                                        let e_find = entity_for_overflow.clone();
                                        let e_print = entity_for_overflow.clone();
                                        let e_pop = entity_for_overflow.clone();
                                        let dp = d_print.clone();
                                        let dpop = d_pop.clone();
                                        menu.item(
                                            PopupMenuItem::new(if is_unread {
                                                "标记为已读"
                                            } else {
                                                "标记为未读"
                                            })
                                            .icon(Icon::new(if is_unread {
                                                IconName::MailOpen
                                            } else {
                                                IconName::Mail
                                            }))
                                            .on_click(move |_, _, cx| {
                                                e_seen.update(cx, |app, cx| {
                                                    app.toggle_message_seen(cx);
                                                });
                                            }),
                                        )
                                        .item(
                                            PopupMenuItem::new("复制正文")
                                                .icon(Icon::new(IconName::Copy))
                                                .on_click(move |_, _, cx| {
                                                    e_copy.update(cx, |app, cx| {
                                                        app.copy_selected_body(cx);
                                                    });
                                                }),
                                        )
                                        .separator()
                                        .item(
                                            PopupMenuItem::new("页内查找 (Ctrl+F)")
                                                .icon(Icon::new(IconName::Search))
                                                .on_click(move |_, window, cx| {
                                                    e_find.update(cx, |app, cx| {
                                                        app.toggle_reader_find(window, cx);
                                                    });
                                                }),
                                        )
                                        .item(
                                            PopupMenuItem::new("打印 / 导出 PDF")
                                                .icon(Icon::new(IconName::FileText))
                                                .on_click(move |_, _, cx| {
                                                    let d = dp.clone();
                                                    e_print.update(cx, |app, cx| {
                                                        app.open_print_preview(d, cx);
                                                    });
                                                }),
                                        )
                                        .item(
                                            PopupMenuItem::new("在新窗口打开邮件")
                                                .icon(Icon::new(IconName::ExternalLink))
                                                .on_click(move |_, _, cx| {
                                                    let d = dpop.clone();
                                                    e_pop.update(cx, |app, cx| {
                                                        app.pop_out_message_window(d, cx);
                                                    });
                                                }),
                                        )
                                    })
                                    .on_open_change(move |open, _, cx| {
                                        popup_entity.update(cx, |app, cx| {
                                            app.set_reader_popup_open(*open);
                                            cx.notify();
                                        });
                                    }),
                            )
                        }),
                )
                // L-6：阅读器在下方时窗口控制按钮由列表顶栏承载，此处不再重复画。
                .when(pane_top, |bar| bar.child(win_controls)),
        )
        // WP-09：阅读器页内查找栏（Ctrl+F）。
        .when(find_open, |root| {
            let match_text = if find_query.trim().is_empty() {
                String::new()
            } else if find_total == 0 {
                "无匹配".to_string()
            } else {
                format!("{} / {}", find_active + 1, find_total)
            };
            root.child(
                h_flex()
                    .px_4()
                    .py_1p5()
                    .gap_2()
                    .items_center()
                    .bg(p.surface_raised)
                    .border_b_1()
                    .border_color(border)
                    .child(
                        div()
                            .w(px(260.0))
                            .child(Input::new(&find_input).small().cleanable(true)),
                    )
                    .when(!match_text.is_empty(), |bar| {
                        bar.child(
                            div()
                                .text_xs()
                                .text_color(muted_fg)
                                .child(SharedString::from(match_text)),
                        )
                    })
                    .child(
                        Button::new("reader-find-prev")
                            .ghost()
                            .compact()
                            .icon(IconName::ChevronUp)
                            .disabled(find_total == 0)
                            .tooltip("上一个匹配 (Shift+Enter)")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.step_reader_find(-1, cx);
                            })),
                    )
                    .child(
                        Button::new("reader-find-next")
                            .ghost()
                            .compact()
                            .icon(IconName::ChevronDown)
                            .disabled(find_total == 0)
                            .tooltip("下一个匹配 (Enter)")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.step_reader_find(1, cx);
                            })),
                    )
                    .child(
                        Button::new("reader-find-close")
                            .ghost()
                            .compact()
                            .icon(IconName::X)
                            .tooltip("关闭查找 (Esc)")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_reader_find(window, cx);
                            })),
                    ),
            )
        })
        // 标题区（8px 24px 8px，L-5 实测顶边距）：账号胶囊 + 缩放胶囊 + 可选会话数胶囊
        // + 阅读视图 & 亮暗切换（B-5）+ 展开/折叠全部 & 保存全部附件 + 20px 粗体主题。
        .child(
            v_flex()
                .flex_shrink_0()
                .w_full()
                .pt(px(8.0))
                .px(px(24.0))
                .pb(px(8.0))
                .gap_1()
                // B-5：标题区改为常驻行——右侧新增「阅读视图」与亮 / 暗背景切换按钮。
                .child({
                    let save_all_ids = all_thread_attachment_ids.clone();
                    let save_all_count = save_all_ids.len();
                    h_flex()
                        .w_full()
                        .gap_2()
                        .items_center()
                        .justify_between()
                        .child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .when(show_account_chip && !owner_label.is_empty(), |row| {
                                    row.child(
                                        div()
                                            .px_2()
                                            .py_0p5()
                                            .rounded_full()
                                            .bg(owner_color.opacity(0.14))
                                            .text_color(owner_color)
                                            .text_xs()
                                            .font_weight(gpui_kit::FontWeight::MEDIUM)
                                            .child(SharedString::from(owner_label)),
                                    )
                                })
                                // 缩放胶囊（S-8）：非 100% 时显示，点击重置（Ctrl+0 同效）。
                                .when(zoom_percent != 100, |row| {
                                    row.child(
                                        div()
                                            .id("reader-zoom-pill")
                                            .px_2()
                                            .py_0p5()
                                            .rounded_full()
                                            .border_1()
                                            .border_color(border)
                                            .text_color(accent)
                                            .text_xs()
                                            .cursor_pointer()
                                            .hover(|s| s.bg(p.accent_soft()))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.set_reader_zoom(100, cx);
                                            }))
                                            .child(SharedString::from(format!("{zoom_percent}%"))),
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
                        .child(
                            h_flex()
                                .gap_1p5()
                                .items_center()
                                // B-5：阅读视图（复用现有 HTML/纯文本切换状态，
                                // 以纯文本渲染剥离 HTML 花哨排版）。
                                .when(show_view_switch, |row| {
                                    row.child(
                                        Button::new("reader-reading-view")
                                            .ghost()
                                            .compact()
                                            .icon(IconName::BookText)
                                            .selected(prefer_plain)
                                            .tooltip(if prefer_plain {
                                                "阅读视图已开启（点击恢复原始排版）"
                                            } else {
                                                "阅读视图（剥离花哨排版，以纯文本渲染）"
                                            })
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.toggle_reader_plain_mode(cx);
                                            })),
                                    )
                                })
                                // B-5：单封邮件亮 / 暗背景切换（仅本会话，不持久化）。
                                .when(show_theme_switch, |row| {
                                    row.child(
                                        Button::new("reader-bg-toggle")
                                            .ghost()
                                            .compact()
                                            .icon(if bg_msg_picked_dark {
                                                IconName::Sun
                                            } else {
                                                IconName::Moon
                                            })
                                            .tooltip("切换此邮件亮 / 暗背景（仅本会话）")
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.toggle_message_bg_pick(bg_msg_id, cx);
                                            })),
                                    )
                                })
                                .when(save_all_count >= 2, |row| {
                                    row.child(
                                        Button::new("reader-save-all-attachments")
                                            .outline()
                                            .compact()
                                            .icon(IconName::Download)
                                            .label(SharedString::from(format!(
                                                "保存全部附件 ({save_all_count})"
                                            )))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.save_all_attachments(save_all_ids.clone(), cx);
                                            })),
                                    )
                                })
                                .when(thread_count > 1, |row| {
                                    row.child(
                                        Button::new("reader-toggle-fold-all")
                                            .ghost()
                                            .compact()
                                            .label(if any_card_collapsed {
                                                "展开全部"
                                            } else {
                                                "折叠全部"
                                            })
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if any_card_collapsed {
                                                    this.expand_all_thread_cards(cx);
                                                } else {
                                                    this.collapse_older_thread_cards(cx);
                                                }
                                            })),
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
        .child({
            let content = v_flex()
                .id("reader-scroll")
                .flex_1()
                .min_h_0()
                .pt(px(12.0))
                .px(px(24.0))
                .pb(px(24.0))
                .gap_3()
                .children(cards);
            if body_view != BodyView::Native && thread_count <= 1 {
                content.into_any_element()
            } else {
                content.overflow_y_scrollbar().into_any_element()
            }
        })
        .when_some(drawer_element, |root, drawer| root.child(drawer))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn webview_requires_loaded_html_without_native_only_features() {
        let html = Some("<p>Hello</p>");
        assert!(webview_eligible(html, false, false, false, false));
        assert!(webview_eligible(
            Some(r#"<p><a href="mailto:team@makenotion.com">Email</a></p>"#),
            false,
            false,
            false,
            false
        ));
        assert!(webview_eligible(
            Some("<blockquote>quote</blockquote>"),
            false,
            false,
            false,
            false
        ));
        assert!(webview_eligible(
            Some(r#"<div class="gmail_quote">quote</div>"#),
            false,
            false,
            false,
            false
        ));
        assert!(webview_eligible(
            Some("<img src=cid:example>"),
            false,
            false,
            false,
            false
        ));
        assert!(!webview_eligible(None, false, false, false, false));
        assert!(!webview_eligible(Some(""), false, false, false, false));
        assert!(!webview_eligible(Some("   "), false, false, false, false));

        assert!(!webview_eligible(html, true, false, false, false));
        assert!(!webview_eligible(html, false, true, false, false));
        assert!(!webview_eligible(html, false, false, true, false));
        assert!(!webview_eligible(html, false, false, false, true));
    }

    #[test]
    fn first_snippet_line_skips_blank_lines() {
        assert_eq!(first_snippet_line("\n  \n  你好世界  \n第二行"), "你好世界");
        assert_eq!(first_snippet_line("   \n\t "), "");
    }

    #[test]
    fn format_raw_source_includes_headers_and_optional_html() {
        let raw = format_raw_source(
            "<id@example.com>",
            "Alice <a@example.com>",
            "b@example.com",
            "c@example.com",
            "10:00",
            "主题",
            "纯文本正文",
            Some("<p>HTML正文</p>"),
        );
        assert!(raw.contains("Message-ID: <id@example.com>"));
        assert!(raw.contains("From: Alice <a@example.com>"));
        assert!(raw.contains("To: b@example.com"));
        assert!(raw.contains("Cc: c@example.com"));
        assert!(raw.contains("Subject: 主题"));
        assert!(raw.contains("纯文本正文"));
        assert!(raw.contains("<p>HTML正文</p>"));
    }
}
