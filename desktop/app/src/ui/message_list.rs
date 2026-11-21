//! 中栏（方案文档第三节 1.2 与五.3）：
//! - 顶部栏 48px（与左栏、右栏严格齐平）：侧栏折叠开关 + 搜索框（即时过滤，Esc 清空）
//!   + 列表计数 + 筛选下拉按钮（全部 / 仅未读 / 仅星标）。
//! - 邮件列表行：行高约 82px（偏好 compact_list 紧凑密度为 64px 且隐藏摘要行）、
//!   水平内边距 12px、外边距 2px 8px、圆角 10px；
//!   左侧 36x36 圆形头像 + 固定 8px 宽未读蓝点占位（保证已读/未读文字严格左对齐）；
//!   右侧三行（发件人 / 主题 / 摘要 + 多账号胶囊），未读加粗；选中态整行 accent 蓝底白字。
//! - 底部轻量分页条：仅当存在上一页 / 下一页时显示。
//! - 邮件行右键菜单：标记已读/未读、加星/取消星标、归档、删除
//!   （演示模式仅保留本地的已读 / 星标两项，归档 / 删除隐藏）；
//!   草稿视图行右键为「打开草稿 / 删除草稿」。

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{ContextMenu, ContextMenuExt, DropdownMenu, PopupMenuItem};
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::Icon;
use gpui_kit::component::{h_flex, v_flex, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    div, px, Context, Div, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    ParentElement, SharedString, Stateful, StatefulInteractiveElement, Styled,
};
use hylki_mail_core::{LocalDraft, MessageSummary};

use crate::app::{CurrentView, HylkiApp, ListFilter};
use crate::theme;
use crate::ui::avatar;

/// 中栏宽度（方案文档：340 ~ 380px）。
const LIST_WIDTH: f32 = 360.0;
/// 顶部栏高度（与左栏 / 右栏统一为 48px）。
const HEADER_HEIGHT: f32 = 48.0;
/// 列表行高度。
const ROW_HEIGHT: f32 = 82.0;
/// 紧凑列表行高（偏好 compact_list，隐藏摘要行，方案文档五.2）。
const COMPACT_ROW_HEIGHT: f32 = 64.0;

/// 邮件行右键菜单（方案文档五.3）。
///
/// `draft` 为 `Some` 时该行是本地草稿行，菜单切换为「打开草稿 / 删除草稿」。
fn row_context_menu(
    row: Stateful<Div>,
    msg: &MessageSummary,
    draft: Option<LocalDraft>,
    account_folders: Vec<(String, String)>,
    demo_mode: bool,
    cx: &mut Context<HylkiApp>,
) -> ContextMenu<Stateful<Div>> {
    let entity = cx.entity();
    let unread = msg.unread;
    let flagged = msg.flagged;
    let account_id = msg.account_id;
    let message_id = msg.id;
    let uid = msg.uid;
    let folder_path = msg.folder_path.clone();
    row.context_menu(move |menu, window, cx| {
        let entity = entity.clone();
        if let Some(draft) = draft.clone() {
            // 本地草稿行：打开编辑 / 删除草稿（走现有 DeleteLocalDraft）。
            let draft_id = draft.id;
            let entity_for_open = entity.clone();
            return menu
                .item(
                    PopupMenuItem::new("打开草稿").on_click(move |_, window, cx| {
                        entity_for_open.update(cx, |this, cx| {
                            this.open_local_draft(&draft, window, cx);
                        });
                    }),
                )
                .item(PopupMenuItem::new("删除草稿").on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| this.delete_row_draft(draft_id, cx));
                }));
        }
        let entity_for_flag = entity.clone();
        let entity_for_archive = entity.clone();
        let entity_for_spam = entity.clone();
        let entity_for_move = entity.clone();
        let entity_for_delete = entity.clone();
        // 内层 on_click 为 move 闭包，需逐项克隆（外层 builder 是 Fn，可多次调用）。
        let folder_path_for_seen = folder_path.clone();
        let folder_path_for_flag = folder_path.clone();
        let current_folder_path = folder_path.clone();
        let folders_for_move = account_folders.clone();
        let mut menu = menu
            .item(
                PopupMenuItem::new(if unread {
                    "标记为已读"
                } else {
                    "标记为未读"
                })
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.set_row_seen(
                            account_id,
                            message_id,
                            uid,
                            folder_path_for_seen.clone(),
                            !unread,
                            cx,
                        );
                    });
                }),
            )
            .item(
                PopupMenuItem::new(if flagged { "取消星标" } else { "加星标" }).on_click(
                    move |_, _, cx| {
                        entity_for_flag.update(cx, |this, cx| {
                            this.set_row_flagged(
                                account_id,
                                message_id,
                                uid,
                                folder_path_for_flag.clone(),
                                !flagged,
                                cx,
                            );
                        });
                    },
                ),
            );
        // 演示模式仅支持本地 seen / flagged 两项；归档 / 标垃圾 / 移动 / 删除仅真实账号可用。
        if !demo_mode {
            menu = menu
                .separator()
                .item(PopupMenuItem::new("归档").on_click(move |_, _, cx| {
                    entity_for_archive.update(cx, |this, cx| {
                        this.archive_message(account_id, message_id, cx);
                    });
                }))
                .item(PopupMenuItem::new("标记为垃圾邮件").on_click(move |_, _, cx| {
                    entity_for_spam.update(cx, |this, cx| {
                        this.mark_spam_message(account_id, message_id, cx);
                    });
                }))
                .submenu("移动到文件夹...", window, cx, move |mut sub, _, _| {
                    if folders_for_move.is_empty() {
                        sub = sub.item(PopupMenuItem::new("暂无可用文件夹").disabled(true));
                        return sub;
                    }
                    for (fname, fpath) in &folders_for_move {
                        let is_current = *fpath == current_folder_path;
                        let target_path = fpath.clone();
                        let e_move = entity_for_move.clone();
                        sub = sub.item(
                            PopupMenuItem::new(fname.clone())
                                .disabled(is_current)
                                .when(is_current, |item| item.icon(Icon::new(IconName::Check)))
                                .on_click(move |_, _, cx| {
                                    let p = target_path.clone();
                                    e_move.update(cx, |this, cx| {
                                        this.move_message(account_id, message_id, p, cx);
                                    });
                                }),
                        );
                    }
                    sub
                })
                .item(PopupMenuItem::new("删除").on_click(move |_, _, cx| {
                    entity_for_delete.update(cx, |this, cx| {
                        this.delete_message(account_id, message_id, cx);
                    });
                }));
        }
        menu
    })
}

/// 列表行（圆角卡片）。
fn message_row(
    app: &HylkiApp,
    msg: &MessageSummary,
    draft: Option<LocalDraft>,
    is_selected: bool,
    show_account_tag: bool,
    p: &theme::HylkiPalette,
    cx: &mut Context<HylkiApp>,
) -> ContextMenu<Stateful<Div>> {
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
    // 偏好「列表密度」：紧凑模式行高约 64px 且隐藏摘要行（方案文档五.2）。
    let compact = app.prefs.compact_list;

    let row = h_flex()
        .id(("msg", msg_id))
        .flex_shrink_0()
        .h(px(if compact {
            COMPACT_ROW_HEIGHT
        } else {
            ROW_HEIGHT
        }))
        .mx_2()
        .my_0p5()
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
        .on_click({
            let draft = draft.clone();
            cx.listener(move |this, event: &gpui_kit::ClickEvent, window, cx| match draft.as_ref() {
                // 草稿行：点击进入写信编辑。
                Some(d) => this.open_local_draft(d, window, cx),
                None => {
                    if event.modifiers().control {
                        this.toggle_message_selection(msg_id, cx);
                    } else {
                        this.selected_message_ids.clear();
                        this.select_message(account_id, msg_id, uid, unread, &folder_path, cx);
                    }
                }
            })
        })
        // 左侧：36x36 圆形头像 + 固定 8px 宽未读圆点占位（确保已读/未读文字严格左对齐）。
        .child(
            h_flex()
                .flex_shrink_0()
                .items_center()
                .gap_1()
                .pt_0p5()
                .child(avatar::avatar(&display_name, &avatar_key, 36.0))
                .child(
                    div()
                        .w(px(8.0))
                        .h(px(8.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(unread, |dot| {
                            dot.child(
                                div()
                                    .w(px(7.0))
                                    .h(px(7.0))
                                    .rounded_full()
                                    .bg(if is_selected { p.accent_fg } else { p.accent }),
                            )
                        }),
                ),
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
                                        Icon::new(IconName::Paperclip)
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
                // 第 3 行：摘要 + 多账号小胶囊标签；紧凑密度下隐藏。
                .when(!compact, |row| {
                    row.child(
                        h_flex()
                            .items_center()
                            .gap_1p5()
                            .min_w_0()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_xs()
                                    .truncate()
                                    .text_color(if is_selected {
                                        p.accent_fg.opacity(0.88)
                                    } else {
                                        p.text_muted
                                    })
                                    .child(SharedString::from(msg.preview.clone())),
                            )
                            .when(!acc_email.is_empty(), |el| {
                                el.child(
                                    div()
                                        .flex_shrink_0()
                                        .max_w(px(110.0))
                                        .truncate()
                                        .px_1p5()
                                        .py_0p5()
                                        .rounded_full()
                                        .text_size(px(10.0))
                                        .bg(if is_selected {
                                            p.accent_fg.opacity(0.22)
                                        } else {
                                            p.accent_soft()
                                        })
                                        .text_color(if is_selected {
                                            p.accent_fg
                                        } else {
                                            p.accent
                                        })
                                        .child(SharedString::from(acc_email)),
                                )
                            }),
                    )
                }),
        );

    let account_folders: Vec<(String, String)> = app
        .folders_by_account
        .get(&account_id)
        .map(|list| list.iter().map(|f| (f.name.clone(), f.path.clone())).collect())
        .unwrap_or_default();
    row_context_menu(row, msg, draft, account_folders, app.demo_mode, cx)
}

/// 渲染中栏。
pub(crate) fn render(app: &HylkiApp, cx: &mut Context<HylkiApp>) -> gpui_kit::AnyElement {
    // 避坑清单第 4 条：render 开头先把语义色 Copy 出来。
    let p = theme::current(cx);
    let (surface, border, muted_fg) = (p.surface, p.border, p.text_muted);

    let selected_id = app.selected_message.as_ref().map(|m| m.id);
    // 本地草稿视图（真实账号）：从 self.drafts 合成列表行。
    let drafts_view = app.current_view == CurrentView::Drafts && !app.demo_mode;
    let show_account_tag = app.current_view.is_unified() && app.accounts.len() > 1;
    let current_filter = app.list_filter;
    let is_unified_list = !drafts_view;

    let search_key = app.search_query.trim().to_lowercase();
    let matches_query = |subject: &str, body: &str| {
        search_key.is_empty()
            || subject.to_lowercase().contains(&search_key)
            || body.to_lowercase().contains(&search_key)
    };

    // 草稿视图行（随搜索过滤）。
    let draft_rows: Vec<gpui_kit::AnyElement> = if drafts_view {
        app.drafts
            .iter()
            .filter(|d| matches_query(&d.subject, &d.body_text))
            .map(|d| {
                let summary = app.draft_row_summary(d);
                message_row(
                    app,
                    &summary,
                    Some(d.clone()),
                    false,
                    show_account_tag,
                    &p,
                    cx,
                )
                .into_any_element()
            })
            .collect()
    } else {
        Vec::new()
    };

    // 常规邮件行（过滤已下沉到 SQLite / DemoStore，current_page.items 即精准结果）。
    let filtered_rows: Vec<gpui_kit::AnyElement> = if drafts_view {
        Vec::new()
    } else {
        app.current_page
            .items
            .iter()
            .map(|msg| {
                let is_selected =
                    selected_id == Some(msg.id) || app.selected_message_ids.contains(&msg.id);
                message_row(app, msg, None, is_selected, show_account_tag, &p, cx)
                    .into_any_element()
            })
            .collect()
    };

    let loaded_count = if drafts_view {
        draft_rows.len()
    } else {
        app.current_page.items.len()
    };
    let total = if drafts_view {
        draft_rows.len()
    } else {
        app.current_page.total
    };
    let has_more_local = !drafts_view && loaded_count < total;
    let has_more_remote = !drafts_view && app.current_view_has_more_remote();
    let has_more = has_more_local || has_more_remote;
    let is_loading_more = !drafts_view && (app.loading_more_local || app.loading_more_remote);

    let has_visible_rows = if drafts_view {
        !draft_rows.is_empty()
    } else {
        !app.current_page.items.is_empty()
    };

    let count_text = if drafts_view {
        if total == 0 {
            "暂无草稿".to_string()
        } else {
            format!("共 {total} 条草稿")
        }
    } else if total == 0 {
        "暂无邮件".to_string()
    } else if has_more_remote {
        format!("{total}+")
    } else {
        total.to_string()
    };

    // 空态文案：草稿 / 常规筛选无结果。
    let (empty_icon, empty_title, empty_hint) = if drafts_view {
        (
            IconName::FilePen,
            "暂无本地草稿",
            "在写信面板点击「存草稿」后显示在这里",
        )
    } else if current_filter == ListFilter::Unread {
        (IconName::Inbox, "暂无未读邮件", "")
    } else if current_filter == ListFilter::Starred {
        (IconName::Star, "暂无星标邮件", "")
    } else {
        (IconName::Inbox, "暂无邮件", "")
    };

    // 顶部筛选下拉菜单（与左栏 / 右栏同为 48px 顶栏，去除突兀的第二层工具条）。
    let filter_menu_button = {
        let entity = cx.entity();
        let active_tooltip = match current_filter {
            ListFilter::All => "筛选邮件（当前：全部）",
            ListFilter::Unread => "筛选邮件（当前：仅未读）",
            ListFilter::Starred => "筛选邮件（当前：仅星标）",
        };
        Button::new("list-filter-menu")
            .ghost()
            .compact()
            .icon(IconName::ListFilter)
            .tooltip(active_tooltip)
            .dropdown_menu(move |menu, _, _| {
                let mut menu = menu;
                for filter in [ListFilter::All, ListFilter::Unread, ListFilter::Starred] {
                    let entity_for_item = entity.clone();
                    let is_current = current_filter == filter;
                    menu = menu.item(
                        PopupMenuItem::new(filter.label())
                            .when(is_current, |item| item.icon(Icon::new(IconName::Check)))
                            .on_click(move |_, _, cx| {
                                entity_for_item.update(cx, |this, cx| {
                                    this.set_list_filter(filter, cx);
                                });
                            }),
                    );
                }
                menu
            })
    };

    v_flex()
        .w(px(LIST_WIDTH))
        .h_full()
        .flex_shrink_0()
        .bg(surface)
        .border_r_1()
        .border_color(border)
        // 顶部栏 48px（与左栏、右栏对齐）：折叠开关 + 搜索框 + 计数 + 筛选下拉。
        .child(
            h_flex()
                .h(px(HEADER_HEIGHT))
                .flex_shrink_0()
                .px_2()
                .gap_1p5()
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
                        this.reset_list_pagination();
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
                .child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(muted_fg)
                        .child(SharedString::from(count_text)),
                )
                .when(is_unified_list, |el| el.child(filter_menu_button)),
        )
        // 多选批量操作条（Ctrl+点击选中 > 1 封邮件时出现）。
        .when(!drafts_view && app.selected_message_ids.len() > 1, |el| {
            let count = app.selected_message_ids.len();
            let demo_mode = app.demo_mode;
            el.child(
                h_flex()
                    .flex_shrink_0()
                    .px_2()
                    .py_1()
                    .gap_1()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(border)
                    .bg(p.accent_soft())
                    .child(
                        div()
                            .text_xs()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .text_color(p.accent)
                            .child(SharedString::from(format!("已选 {count} 封"))),
                    )
                    .child(
                        h_flex()
                            .gap_0p5()
                            .items_center()
                            .child(
                                Button::new("batch-mark-read")
                                    .ghost()
                                    .compact()
                                    .text_size(px(11.0))
                                    .label("标为已读")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.batch_mark_read(cx);
                                    })),
                            )
                            .child(
                                Button::new("batch-star")
                                    .ghost()
                                    .compact()
                                    .text_size(px(11.0))
                                    .label("加星标")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.batch_toggle_flagged(cx);
                                    })),
                            )
                            .when(!demo_mode, |row| {
                                row.child(
                                    Button::new("batch-archive")
                                        .ghost()
                                        .compact()
                                        .text_size(px(11.0))
                                        .label("归档")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.batch_archive(cx);
                                        })),
                                )
                                .child(
                                    Button::new("batch-delete")
                                        .ghost()
                                        .compact()
                                        .text_size(px(11.0))
                                        .label("删除")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.batch_delete(cx);
                                        })),
                                )
                            })
                            .child(
                                Button::new("batch-clear")
                                    .ghost()
                                    .compact()
                                    .text_size(px(11.0))
                                    .label("取消选择")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.clear_message_selection(cx);
                                    })),
                            ),
                    ),
            )
        })
        // 列表（滚到底部自动预加载下一批 100 条）。
        .child(
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.maybe_preload_more(cx);
                    }),
                )
                .child(
                    div()
                        .id("message-list-scroll")
                        .size_full()
                        .flex()
                        .flex_col()
                        .overflow_y_scroll()
                        .track_scroll(&app.list_scroll_handle)
                        .on_scroll(cx.listener(|this, _, _, cx| {
                            this.maybe_preload_more(cx);
                        }))
                        .child(
                            v_flex()
                                .flex_none()
                                .w_full()
                                .h_auto()
                                .min_h_full()
                                .py_1()
                                .when(drafts_view, |el| el.children(draft_rows))
                                .when(!drafts_view, |el| el.children(filtered_rows))
                                .when(!has_visible_rows, |el| {
                                    el.child(
                                        v_flex()
                                            .flex_1()
                                            .items_center()
                                            .justify_center()
                                            .gap_1()
                                            .pt_8()
                                            .child(
                                                Icon::new(empty_icon)
                                                    .with_size(px(28.0))
                                                    .text_color(muted_fg),
                                            )
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(muted_fg)
                                                    .child(empty_title),
                                            )
                                            .when(!empty_hint.is_empty(), |el| {
                                                el.child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(muted_fg)
                                                        .child(empty_hint),
                                                )
                                            }),
                                    )
                                })
                                .when(!drafts_view && has_visible_rows, |el| {
                                    el.child(
                                        h_flex()
                                            .h(px(40.0))
                                            .flex_shrink_0()
                                            .items_center()
                                            .justify_center()
                                            .when(is_loading_more, |f| {
                                                f.child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(muted_fg)
                                                        .child("正在加载更多邮件..."),
                                                )
                                            })
                                            .when(!is_loading_more && has_more, |f| {
                                                f.child(
                                                    Button::new("list-load-more")
                                                        .ghost()
                                                        .compact()
                                                        .label("加载更多邮件")
                                                        .on_click(cx.listener(|this, _, _, cx| {
                                                            this.trigger_load_more(cx);
                                                        })),
                                                )
                                            })
                                            .when(!is_loading_more && !has_more, |f| {
                                                f.child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(muted_fg)
                                                        .child(SharedString::from(format!(
                                                            "已加载全部 {total} 封邮件"
                                                        ))),
                                                )
                                            }),
                                    )
                                }),
                        ),
                )
                .child(Scrollbar::vertical(&app.list_scroll_handle)),
        )
        .into_any_element()
}
