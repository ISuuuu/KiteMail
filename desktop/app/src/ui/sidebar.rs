//! 左栏（方案文档第三节 1.1）：
//! - 顶部栏 48px：刷新按钮 + 居中 `Hylki` 粗体标题 + 菜单按钮
//!   （账号管理 / 切换亮暗主题 / 偏好设置与命令面板——后者本轮渲染但禁用）。
//! - 全宽蓝色「写邮件」按钮（38px 高、圆角 8px、accent 色）。
//! - 统一导航区（34px 高、圆角 8、左图标 + 名称 + 右侧未读数胶囊）：
//!   收件箱 / 已加星标 / 已发送 / 草稿箱 / 归档 / 发件箱待处理。
//! - 账号折叠分组：彩色头像（角上未读小蓝圈）+ 账号名 + 邮箱小字 +
//!   右侧单账号刷新按钮 + 折叠箭头；展开列出各文件夹。
//! - 底部固定区：联系人、附件入口。

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::Icon;
use gpui_kit::component::IconName;
use gpui_kit::component::{h_flex, v_flex, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    div, px, Context, Div, InteractiveElement, IntoElement, ParentElement, SharedString, Stateful,
    StatefulInteractiveElement, Styled,
};
use hylki_mail_core::{MailRequest, OutboxItem, OutboxStatus};

use crate::app::{HylkiApp, UNIFIED_INBOX_FOLDER_ID};
use crate::theme;
use crate::ui::avatar;

/// 左栏宽度（方案文档：230 ~ 250px）。
const SIDEBAR_WIDTH: f32 = 240.0;
/// 顶部栏高度。
const HEADER_HEIGHT: f32 = 48.0;
/// 导航行高度。
const NAV_ROW_HEIGHT: f32 = 34.0;

/// 未读 / 计数胶囊（选中行反白，告警行标红）。
fn count_pill(
    n: u32,
    selected: bool,
    danger: bool,
    palette: &theme::HylkiPalette,
) -> gpui_kit::Div {
    div()
        .rounded_full()
        .px_1p5()
        .min_w(px(20.0))
        .h(px(18.0))
        .flex()
        .items_center()
        .justify_center()
        .text_xs()
        .font_weight(gpui_kit::FontWeight::BOLD)
        .child(SharedString::from(if n > 999 {
            "999+".to_string()
        } else {
            n.to_string()
        }))
        .when(danger, |pill| {
            pill.bg(palette.danger).text_color(palette.accent_fg)
        })
        .when(!danger && selected, |pill| {
            pill.bg(palette.accent_fg).text_color(palette.accent)
        })
        .when(!danger && !selected, |pill| {
            pill.bg(palette.accent).text_color(palette.accent_fg)
        })
}

/// 统一导航行：图标 + 名称 + 右侧胶囊（34px 高、圆角 8px）。
fn nav_row(
    id: &'static str,
    icon: IconName,
    label: &'static str,
    count: Option<(u32, bool)>,
    selected: bool,
    palette: &theme::HylkiPalette,
) -> Stateful<Div> {
    let mut row = h_flex()
        .id(id)
        .h(px(NAV_ROW_HEIGHT))
        .px_2()
        .gap_2()
        .items_center()
        .rounded(px(8.0))
        .cursor_pointer()
        .hover(|s| s.bg(palette.accent_soft()))
        .when(selected, |el| el.bg(palette.accent));

    row = row.child(Icon::new(icon).with_size(px(16.0)).text_color(if selected {
        palette.accent_fg
    } else {
        palette.text_muted
    }));
    row = row.child(
        div()
            .flex_1()
            .text_sm()
            .truncate()
            .font_weight(if selected {
                gpui_kit::FontWeight::SEMIBOLD
            } else {
                gpui_kit::FontWeight::NORMAL
            })
            .text_color(if selected {
                palette.accent_fg
            } else {
                palette.text
            })
            .child(SharedString::from(label)),
    );
    if let Some((n, danger)) = count {
        if n > 0 {
            row = row.child(count_pill(n, selected, danger, palette));
        }
    }
    row
}

/// 账号折叠分组（头像 + 名称 + 邮箱 + 单账号刷新 + 折叠箭头 + 文件夹列表）。
fn account_section(
    app: &HylkiApp,
    acc_id: u32,
    display_name: String,
    email: String,
    folders: &[hylki_mail_core::FolderSummary],
    p: &theme::HylkiPalette,
    cx: &mut Context<HylkiApp>,
) -> gpui_kit::AnyElement {
    let (sidebar_bg, text, muted_fg, accent, accent_fg) =
        (p.sidebar, p.text, p.text_muted, p.accent, p.accent_fg);
    let selected_folder_id = app.selected_folder_id;
    let expanded = app.account_expanded(acc_id);
    let account_unread: u32 = folders
        .iter()
        .filter(|f| f.path.eq_ignore_ascii_case("INBOX") || f.name.contains("收件箱"))
        .map(|f| f.unread_count)
        .sum();
    let demo_mode = app.demo_mode;

    v_flex()
        .gap_0p5()
        .child(
            h_flex()
                .id(("acc-header", acc_id))
                .h(px(40.0))
                .px_1()
                .gap_2()
                .items_center()
                .rounded(px(8.0))
                .cursor_pointer()
                .hover(|s| s.bg(p.accent_soft()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.toggle_account_expanded(acc_id, cx);
                }))
                .child(avatar::account_avatar(
                    &display_name,
                    &email,
                    28.0,
                    account_unread,
                    accent,
                    sidebar_bg,
                ))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui_kit::FontWeight::MEDIUM)
                                .text_color(text)
                                .truncate()
                                .child(SharedString::from(display_name)),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted_fg)
                                .truncate()
                                .child(SharedString::from(email)),
                        ),
                )
                .when(!demo_mode, |row| {
                    row.child(
                        Button::new(("sync-acc", acc_id))
                            .ghost()
                            .compact()
                            .icon(IconName::RefreshCw)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(ref w) = this.worker {
                                    w.send(MailRequest::SyncAccount { account_id: acc_id });
                                    cx.notify();
                                }
                            })),
                    )
                })
                .child(
                    Icon::new(if expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .with_size(px(14.0))
                    .text_color(muted_fg),
                ),
        )
        .when(expanded, |section| {
            section.children(folders.iter().map(|folder| {
                let folder_id = folder.id;
                let folder_path = folder.path.clone();
                let is_selected = folder_id == selected_folder_id;
                let unread = folder.unread_count;
                let total = folder.total_count as u32;
                let folder_name = folder.name.clone();

                h_flex()
                    .id(("folder", folder_id))
                    .h(px(NAV_ROW_HEIGHT))
                    .pl(px(38.0))
                    .pr_2()
                    .gap_2()
                    .items_center()
                    .rounded(px(8.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(p.accent_soft()))
                    .when(is_selected, |row| row.bg(p.accent))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_folder(acc_id, folder_id, folder_path.clone(), cx);
                    }))
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .truncate()
                            .text_color(if is_selected { accent_fg } else { text })
                            .child(SharedString::from(folder_name)),
                    )
                    .when(unread > 0, |row| {
                        row.child(count_pill(unread, is_selected, false, p))
                    })
                    .when(unread == 0 && total > 0, |row| {
                        row.child(
                            div()
                                .text_xs()
                                .text_color(if is_selected { accent_fg } else { muted_fg })
                                .child(SharedString::from(total.to_string())),
                        )
                    })
            }))
        })
        .into_any_element()
}

pub(crate) fn render(app: &HylkiApp, cx: &mut Context<HylkiApp>) -> gpui_kit::AnyElement {
    // 避坑清单第 4 条：render 开头先把语义色 Copy 出来。
    let p = theme::current(cx);
    let (sidebar_bg, border, text, muted_fg, danger) =
        (p.sidebar, p.border, p.text, p.text_muted, p.danger);
    let unified_unread = app.unified_unread_count();
    let drafts_count = app.drafts.len() as u32;
    let outbox_items: Vec<OutboxItem> = app
        .outbox_by_account
        .values()
        .flat_map(|v| v.iter().cloned())
        .collect();
    let outbox_pending = outbox_items
        .iter()
        .filter(|i| matches!(i.status, OutboxStatus::Uncertain | OutboxStatus::Failed))
        .count() as u32;
    let dark = app.dark_mode;
    let demo_mode = app.demo_mode;
    // 统一收件箱行选中态：真实账号看 UNIFIED_INBOX_FOLDER_ID(0)，演示模式看演示 INBOX。
    let is_unified_selected = if demo_mode {
        app.selected_folder_id == hylki_mail_core::demo::DEMO_INBOX_ID
    } else {
        app.selected_folder_id == UNIFIED_INBOX_FOLDER_ID
    };

    let account_sections: Vec<gpui_kit::AnyElement> = app
        .accounts
        .iter()
        .map(|acc| {
            let display_name = if acc.display_name.is_empty() {
                acc.email.clone()
            } else {
                acc.display_name.clone()
            };
            let folders = app
                .folders_by_account
                .get(&acc.id)
                .cloned()
                .unwrap_or_default();
            account_section(
                app,
                acc.id,
                display_name,
                acc.email.clone(),
                &folders,
                &p,
                cx,
            )
        })
        .collect();

    // 菜单按钮的弹出菜单（账号管理 / 切换亮暗主题 / 禁用的偏好设置与命令面板）。
    let menu_button = {
        let entity = cx.entity();
        let entity_for_theme = entity.clone();
        Button::new("sb-menu")
            .ghost()
            .compact()
            .icon(IconName::Menu)
            .dropdown_menu(move |menu, _, _| {
                let entity = entity.clone();
                let entity_for_theme = entity_for_theme.clone();
                menu.item(
                    PopupMenuItem::new("账号管理")
                        .icon(Icon::new(IconName::Settings))
                        .on_click(move |_, window, cx| {
                            entity.update(cx, |app, cx| {
                                let first = app.accounts.first().cloned();
                                app.load_account_into_form(first.as_ref(), window, cx);
                            });
                        }),
                )
                .item(
                    PopupMenuItem::new(if dark {
                        "切换亮色主题"
                    } else {
                        "切换暗色主题"
                    })
                    .icon(Icon::new(if dark { IconName::Sun } else { IconName::Moon }))
                    .on_click(move |_, _, cx| {
                        entity_for_theme.update(cx, |app, cx| {
                            app.toggle_theme(cx);
                        });
                    }),
                )
                .separator()
                // TODO(phase3): 偏好设置页（preferences.rs）落地后接线。
                .item(PopupMenuItem::new("偏好设置").disabled(true))
                // TODO(phase3): Ctrl+K 命令面板（command_palette.rs）落地后接线。
                .item(PopupMenuItem::new("命令面板  Ctrl+K").disabled(true))
            })
    };

    v_flex()
        .w(px(SIDEBAR_WIDTH))
        .h_full()
        .flex_shrink_0()
        .bg(sidebar_bg)
        .border_r_1()
        .border_color(border)
        // 顶部栏 48px：刷新 + Hylki + 菜单。
        .child(
            h_flex()
                .h(px(HEADER_HEIGHT))
                .flex_shrink_0()
                .px_2()
                .gap_1()
                .items_center()
                .justify_between()
                .child(
                    Button::new("sb-refresh")
                        .ghost()
                        .compact()
                        .icon(IconName::RefreshCw)
                        .tooltip(if demo_mode {
                            "刷新演示数据"
                        } else {
                            "同步所有账号"
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            if this.demo_mode {
                                this.reload_current_page(cx);
                                return;
                            }
                            if let Some(ref w) = this.worker {
                                this.error_banner = None;
                                this.status_line = SharedString::from("正在独立刷新所有账号...");
                                w.send(MailRequest::SyncAllAccounts);
                                cx.notify();
                            }
                        })),
                )
                .child(
                    div()
                        .text_base()
                        .font_weight(gpui_kit::FontWeight::BOLD)
                        .text_color(text)
                        .child("Hylki"),
                )
                .child(menu_button),
        )
        // 全宽蓝色「写邮件」按钮（38px、圆角 8px）。
        .child(
            div().px_3().pb_2().flex_shrink_0().child(
                Button::new("sb-compose")
                    .primary()
                    .label("写邮件")
                    .icon(IconName::Plus)
                    .rounded(px(8.0))
                    .h(px(38.0))
                    .w_full()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.start_new_compose(window, cx);
                    })),
            ),
        )
        // 可滚动主体：统一导航区 + 账号分组 + 草稿 / 发件箱。
        .child(
            v_flex()
                .id("sidebar-scroll")
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .px_2()
                .gap_0p5()
                .child(
                    nav_row(
                        "nav-inbox",
                        IconName::Inbox,
                        "收件箱",
                        Some((unified_unread, false)),
                        is_unified_selected,
                        &p,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.select_unified_inbox(cx);
                    })),
                )
                // TODO(phase2): 已加星标——后端统一视图查询（另一并行分支提供）就绪后接线。
                .child(
                    nav_row("nav-starred", IconName::Star, "已加星标", None, false, &p).on_click(
                        cx.listener(|this, _, _, cx| {
                            // 暂回落到统一收件箱。
                            this.select_unified_inbox(cx);
                        }),
                    ),
                )
                // TODO(phase2): 已发送——同上。
                .child(
                    nav_row("nav-sent", IconName::Redo2, "已发送", None, false, &p).on_click(
                        cx.listener(|this, _, _, cx| {
                            // 暂回落到统一收件箱。
                            this.select_unified_inbox(cx);
                        }),
                    ),
                )
                // 草稿箱行沿用现有本地草稿展示逻辑（草稿条目列于账号分组后）。
                .child(
                    nav_row(
                        "nav-drafts",
                        IconName::FileText,
                        "草稿箱",
                        Some((drafts_count, false)),
                        false,
                        &p,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.select_unified_inbox(cx);
                    })),
                )
                // TODO(phase2): 归档——同上。
                .child(
                    nav_row(
                        "nav-archive",
                        IconName::FolderClosed,
                        "归档",
                        None,
                        false,
                        &p,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        // 暂回落到统一收件箱。
                        this.select_unified_inbox(cx);
                    })),
                )
                .when(!outbox_items.is_empty(), |el| {
                    el.child(
                        nav_row(
                            "nav-outbox",
                            IconName::TriangleAlert,
                            "发件箱待处理",
                            Some((outbox_pending.max(outbox_items.len() as u32), true)),
                            false,
                            &p,
                        )
                        .on_click(cx.listener(|_, _, _, _| {
                            // 发件箱条目直接列在本行下方，行点击无操作。
                        })),
                    )
                    .children(outbox_items.iter().map(|item| {
                        let outbox_id = item.id;
                        let account_id = item.account_id;
                        let label = match item.status {
                            OutboxStatus::Uncertain => "发送结果待确认",
                            OutboxStatus::Failed => "发送失败",
                            OutboxStatus::Sending => "正在发送",
                            OutboxStatus::Queued => "排队中",
                        };
                        v_flex()
                            .pl(px(38.0))
                            .pr_2()
                            .py_1()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(gpui_kit::FontWeight::BOLD)
                                    .text_color(if item.status == OutboxStatus::Sending {
                                        muted_fg
                                    } else {
                                        danger
                                    })
                                    .child(SharedString::from(label)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted_fg)
                                    .truncate()
                                    .child(SharedString::from(item.subject.clone())),
                            )
                            .when(!item.last_error.is_empty(), |e| {
                                e.child(
                                    div()
                                        .text_xs()
                                        .text_color(muted_fg)
                                        .truncate()
                                        .child(SharedString::from(item.last_error.clone())),
                                )
                            })
                            .when(item.status != OutboxStatus::Sending, |e| {
                                e.child(
                                    Button::new(("retry-outbox", outbox_id))
                                        .ghost()
                                        .compact()
                                        .label("手动重发")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if let Some(ref w) = this.worker {
                                                w.send(MailRequest::RetryOutboxItem {
                                                    account_id,
                                                    outbox_id,
                                                });
                                                cx.notify();
                                            }
                                        })),
                                )
                            })
                    }))
                })
                .when(app.accounts.is_empty(), |el| {
                    el.child(
                        div()
                            .pt_1()
                            .text_xs()
                            .text_color(muted_fg)
                            .child("尚未配置邮箱，请在右侧添加账号。"),
                    )
                })
                .children(account_sections)
                // 草稿条目（沿用现有本地草稿展示逻辑）。
                .when(!app.drafts.is_empty(), |el| {
                    el.child(
                        div()
                            .pt_1()
                            .text_xs()
                            .text_color(muted_fg)
                            .child(SharedString::from(format!(
                                "本地草稿（{}）",
                                app.drafts.len()
                            ))),
                    )
                    .children(app.drafts.iter().map(|draft| {
                        let d = draft.clone();
                        let title = if d.subject.trim().is_empty() {
                            "(无主题草稿)".to_string()
                        } else {
                            d.subject.clone()
                        };
                        h_flex()
                            .id(("draft", d.id))
                            .pl(px(38.0))
                            .pr_2()
                            .h(px(NAV_ROW_HEIGHT))
                            .gap_2()
                            .items_center()
                            .rounded(px(8.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(p.accent_soft()))
                            .child(
                                Icon::new(IconName::FileText)
                                    .with_size(px(14.0))
                                    .text_color(muted_fg),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .text_xs()
                                    .truncate()
                                    .text_color(text)
                                    .child(SharedString::from(title)),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_local_draft(&d, window, cx);
                            }))
                    }))
                }),
        )
        // 底部固定区：联系人 / 附件入口。
        .child(
            v_flex()
                .flex_shrink_0()
                .px_2()
                .py_2()
                .gap_0p5()
                .border_t_1()
                .border_color(border)
                // TODO(phase3): 联系人页（contacts.rs）落地后切换视图。
                .child(
                    nav_row(
                        "nav-contacts",
                        IconName::CircleUser,
                        "联系人",
                        None,
                        false,
                        &p,
                    )
                    .on_click(cx.listener(|_, _, _, _| {})),
                )
                // TODO(phase3): 附件总览页（gallery.rs）落地后切换视图。
                .child(
                    nav_row(
                        "nav-attachments",
                        IconName::FileText,
                        "附件",
                        None,
                        false,
                        &p,
                    )
                    .on_click(cx.listener(|_, _, _, _| {})),
                ),
        )
        .into_any_element()
}
