//! 中栏（方案文档第三节 1.2）：
//! - 顶部栏 48px：侧栏折叠开关 + 搜索框（即时过滤，Esc 清空）+ 列表计数。
//! - 次级工具条 34px：筛选切换（全部 / 仅未读 / 仅星标，对当前页客户端过滤）
//!   + 上一页 / 下一页分页按钮（沿用现有分页逻辑）。
//! - 邮件列表行：行高约 82px、水平内边距 12px、外边距 4px 8px、圆角 10px；
//!   左侧 36x36 圆形头像 + 7px 蓝色未读圆点；右侧三行（发件人 / 主题 /
//!   摘要），未读加粗；选中态整行 accent 蓝底白字。

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::Icon;
use gpui_kit::component::IconName;
use gpui_kit::component::{h_flex, v_flex, Disableable, Selectable, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    div, px, Context, Div, InteractiveElement, IntoElement, KeyDownEvent, ParentElement,
    SharedString, Stateful, StatefulInteractiveElement, Styled,
};
use hylki_mail_core::MessageSummary;

use crate::app::{HylkiApp, ListFilter, PAGE_SIZE, UNIFIED_INBOX_FOLDER_ID};
use crate::theme;
use crate::ui::avatar;

/// 中栏宽度（方案文档：340 ~ 380px）。
const LIST_WIDTH: f32 = 360.0;
/// 顶部栏高度。
const HEADER_HEIGHT: f32 = 48.0;
/// 次级工具条高度。
const TOOLBAR_HEIGHT: f32 = 34.0;
/// 列表行高度。
const ROW_HEIGHT: f32 = 82.0;

/// 列表行（圆角卡片）。
fn message_row(
    app: &HylkiApp,
    msg: &MessageSummary,
    is_selected: bool,
    show_account_tag: bool,
    p: &theme::HylkiPalette,
    cx: &mut Context<HylkiApp>,
) -> Stateful<Div> {
    let account_id = msg.account_id;
    let msg_id = msg.id;
    let uid = msg.uid;
    let unread = msg.unread;
    let folder_path = msg.folder_path.clone();
    let (sender_name, sender_email) = crate::app::split_sender(&msg.sender);
    let display_name = if sender_name.is_empty() {
        msg.sender.clone()
    } else {
        sender_name
    };
    let acc_email = if show_account_tag {
        app.account_email_for(account_id).to_string()
    } else {
        String::new()
    };
    // 头像取色键优先用发件人自身邮箱。
    let avatar_key = if sender_email.is_empty() {
        acc_email.clone()
    } else {
        sender_email
    };

    h_flex()
        .id(("msg", msg_id))
        .h(px(ROW_HEIGHT))
        .mx_2()
        .my_1()
        .px_3()
        .py_2()
        .gap_2()
        .items_start()
        .rounded(px(10.0))
        .cursor_pointer()
        .overflow_hidden()
        .hover(move |s| {
            if is_selected {
                s.bg(p.accent)
            } else {
                s.bg(p.accent_soft())
            }
        })
        .when(is_selected, |row| row.bg(p.accent))
        .when(!is_selected && unread, |row| {
            row.bg(p.surface_raised).border_1().border_color(p.border)
        })
        .on_click(cx.listener(move |this, _, _, cx| {
            this.select_message(account_id, msg_id, uid, unread, &folder_path, cx);
        }))
        // 左侧：36x36 圆形头像 + 头像右侧 7px 蓝色未读圆点。
        .child(
            h_flex()
                .items_center()
                .gap_1()
                .child(avatar::avatar(&display_name, &avatar_key, 36.0))
                .when(unread, |dot| {
                    dot.child(div().w(px(7.0)).h(px(7.0)).rounded_full().bg(p.accent))
                }),
        )
        // 右侧三行信息。
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .h_full()
                .gap_0p5()
                // 第 1 行：发件人 + 附件 / 星标 / 时间。
                .child(
                    h_flex()
                        .justify_between()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_sm()
                                .truncate()
                                .font_weight(if unread {
                                    gpui_kit::FontWeight::BOLD
                                } else {
                                    gpui_kit::FontWeight::NORMAL
                                })
                                .text_color(if is_selected { p.accent_fg } else { p.text })
                                .child(SharedString::from(display_name)),
                        )
                        .child(
                            h_flex()
                                .flex_shrink_0()
                                .gap_1()
                                .items_center()
                                .when(msg.flagged, |icons| {
                                    icons.child(
                                        Icon::new(IconName::StarFill)
                                            .with_size(px(13.0))
                                            .text_color(if is_selected {
                                                p.accent_fg
                                            } else {
                                                gpui_kit::yellow()
                                            }),
                                    )
                                })
                                .when(msg.has_attachment, |icons| {
                                    icons.child(
                                        Icon::new(IconName::FileText)
                                            .with_size(px(13.0))
                                            .text_color(if is_selected {
                                                p.accent_fg
                                            } else {
                                                p.text_muted
                                            }),
                                    )
                                })
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(if is_selected {
                                            p.accent_fg
                                        } else {
                                            p.text_muted
                                        })
                                        .child(SharedString::from(msg.date_display.clone())),
                                ),
                        ),
                )
                // 第 2 行：主题（未读加粗）。
                .child(
                    div()
                        .text_sm()
                        .truncate()
                        .font_weight(if unread {
                            gpui_kit::FontWeight::SEMIBOLD
                        } else {
                            gpui_kit::FontWeight::NORMAL
                        })
                        .text_color(if is_selected { p.accent_fg } else { p.text })
                        .child(SharedString::from(msg.subject.clone())),
                )
                // 第 3 行：两行以内摘要（行高裁剪由固定行高 + overflow_hidden 完成）。
                .child(
                    div()
                        .text_xs()
                        .line_height(px(16.0))
                        .text_color(if is_selected {
                            p.accent_fg
                        } else {
                            p.text_muted
                        })
                        .child(SharedString::from(if acc_email.is_empty() {
                            msg.preview.clone()
                        } else {
                            format!("[{acc_email}] {}", msg.preview)
                        })),
                ),
        )
}

/// 渲染中栏。
pub(crate) fn render(app: &HylkiApp, cx: &mut Context<HylkiApp>) -> gpui_kit::AnyElement {
    // 避坑清单第 4 条：render 开头先把语义色 Copy 出来。
    let p = theme::current(cx);
    let (surface, border, muted_fg) = (p.surface, p.border, p.text_muted);

    let selected_id = app.selected_message.as_ref().map(|m| m.id);
    let start = if app.current_page.items.is_empty() {
        0
    } else {
        app.current_page.offset + 1
    };
    let end = app.current_page.offset + app.current_page.items.len();
    let total = app.current_page.total;
    let show_account_tag =
        app.selected_folder_id == UNIFIED_INBOX_FOLDER_ID && app.accounts.len() > 1;
    let current_filter = app.list_filter;
    let has_prev = app.page_offset > 0;
    let has_next = app.page_offset + PAGE_SIZE < total;

    let has_visible_rows = app
        .current_page
        .items
        .iter()
        .any(|msg| match current_filter {
            ListFilter::All => true,
            ListFilter::Unread => msg.unread,
            ListFilter::Starred => msg.flagged,
        });

    let filtered_rows: Vec<gpui_kit::AnyElement> = app
        .current_page
        .items
        .iter()
        .filter(|msg| match current_filter {
            ListFilter::All => true,
            ListFilter::Unread => msg.unread,
            ListFilter::Starred => msg.flagged,
        })
        .map(|msg| {
            let is_selected = selected_id == Some(msg.id);
            message_row(app, msg, is_selected, show_account_tag, &p, cx).into_any_element()
        })
        .collect();

    v_flex()
        .w(px(LIST_WIDTH))
        .h_full()
        .flex_shrink_0()
        .bg(surface)
        .border_r_1()
        .border_color(border)
        // 顶部栏 48px：侧栏折叠开关 + 搜索框 + 列表计数。
        .child(
            h_flex()
                .h(px(HEADER_HEIGHT))
                .flex_shrink_0()
                .px_2()
                .gap_2()
                .items_center()
                .border_b_1()
                .border_color(border)
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    // Esc 清空搜索（搜索框获得焦点时按键事件冒泡到此处）。
                    if event.keystroke.key == "escape" {
                        this.search_input.update(cx, |s, cx| {
                            s.set_value("", window, cx);
                        });
                        this.search_query = String::new();
                        this.page_offset = 0;
                        this.reload_current_page(cx);
                    }
                }))
                .child(
                    Button::new("list-toggle-sidebar")
                        .ghost()
                        .compact()
                        .icon(if app.sidebar_hidden {
                            IconName::PanelLeftOpen
                        } else {
                            IconName::PanelLeftClose
                        })
                        .tooltip("折叠 / 展开左栏")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.toggle_sidebar(cx);
                        })),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(Input::new(&app.search_input)),
                )
                .child(div().flex_shrink_0().text_xs().text_color(muted_fg).child(
                    SharedString::from(if total == 0 {
                        "暂无邮件".to_string()
                    } else {
                        format!("{start}-{end} / {total}")
                    }),
                )),
        )
        // 次级工具条：筛选切换 + 分页。
        .child(
            h_flex()
                .h(px(TOOLBAR_HEIGHT))
                .flex_shrink_0()
                .px_2()
                .gap_1()
                .items_center()
                .border_b_1()
                .border_color(border)
                .children(
                    [ListFilter::All, ListFilter::Unread, ListFilter::Starred].map(|filter| {
                        Button::new(filter.id())
                            .ghost()
                            .compact()
                            .selected(current_filter == filter)
                            .label(filter.label())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.set_list_filter(filter, cx);
                            }))
                    }),
                )
                .child(div().flex_1())
                .child(
                    Button::new("list-prev-page")
                        .ghost()
                        .compact()
                        .icon(IconName::ChevronLeft)
                        .tooltip("上一页")
                        .disabled(!has_prev)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.prev_page(cx);
                        })),
                )
                .child(
                    Button::new("list-next-page")
                        .ghost()
                        .compact()
                        .icon(IconName::ChevronRight)
                        .tooltip("下一页")
                        .disabled(!has_next)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.next_page(cx);
                        })),
                ),
        )
        // 列表（滚动）。
        .child(
            div()
                .id("message-list-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .py_1()
                .children(filtered_rows)
                .when(!has_visible_rows, |el| {
                    el.child(
                        v_flex()
                            .flex_1()
                            .items_center()
                            .justify_center()
                            .gap_1()
                            .pt_8()
                            .child(
                                Icon::new(IconName::Inbox)
                                    .with_size(px(28.0))
                                    .text_color(muted_fg),
                            )
                            .child(div().text_sm().text_color(muted_fg).child("暂无邮件")),
                    )
                }),
        )
        .into_any_element()
}
