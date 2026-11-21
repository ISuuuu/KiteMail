//! 左栏（方案文档第三节 1.1）：
//! - 顶部栏 48px：刷新按钮 + 居中 `Hylki` 粗体标题 + 菜单按钮
//!   （账号管理 / 切换亮暗主题 / 偏好设置 / 命令面板 Ctrl+K）。
//! - 全宽蓝色「写邮件」按钮（38px 高、圆角 8px、accent 色）。
//! - 统一导航区（34px 高、圆角 8、左图标 + 名称 + 右侧未读数胶囊）：
//!   收件箱 / 已加星标 / 已发送 / 草稿箱 / 归档 / 发件箱待处理。
//! - 账号折叠分组：彩色头像（角上未读小蓝圈）+ 账号名 + 邮箱小字 +
//!   右侧单账号刷新按钮 + 折叠箭头；展开列出各文件夹。
//! - 底部固定区：联系人、附件入口。

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::Icon;
use gpui_kit::component::{h_flex, v_flex, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    div, px, Context, Div, InteractiveElement, IntoElement, ParentElement, SharedString, Stateful,
    StatefulInteractiveElement, Styled, WindowControlArea,
};
use hylki_mail_core::{MailRequest, OutboxItem, OutboxStatus};

use crate::app::{CurrentView, HylkiApp, MainContentView};
use crate::theme;
use crate::ui::avatar;

/// 左栏宽度（方案文档：230 ~ 250px）。
const SIDEBAR_WIDTH: f32 = 240.0;
/// 顶部栏高度。
const HEADER_HEIGHT: f32 = 48.0;
/// 导航行高度。
const NAV_ROW_HEIGHT: f32 = 34.0;

/// 未读 / 计数胶囊（柔和区分普通与告警状态）。
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
            pill.bg(palette.accent).text_color(palette.accent_fg)
        })
        .when(!danger && !selected, |pill| {
            pill.bg(palette.accent).text_color(palette.accent_fg)
        })
}

/// 根据文件夹路径或中文名挑选语义图标。
fn folder_icon(path: &str, name: &str) -> IconName {
    let p = path.to_ascii_uppercase();
    if p == "INBOX" || name.contains("收件箱") {
        IconName::Inbox
    } else if p.contains("SENT") || name.contains("已发送") {
        IconName::Send
    } else if p.contains("DRAFT") || name.contains("草稿") {
        IconName::FilePen
    } else if p.contains("ARCHIVE") || name.contains("归档") {
        IconName::Archive
    } else if p.contains("TRASH") || p.contains("DELETED") || name.contains("已删除") {
        IconName::Trash
    } else if p.contains("JUNK") || p.contains("SPAM") || name.contains("垃圾") {
        IconName::TriangleAlert
    } else {
        IconName::Folder
    }
}

/// 统一导航行：图标 + 名称 + 右侧胶囊（34px 高、圆角 8px，柔和选中态避免与中栏撞色）。
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
        .when(selected, |el| el.bg(palette.accent_soft()));

    row = row.child(Icon::new(icon).with_size(px(16.0)).text_color(if selected {
        palette.accent
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
                palette.accent
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
    let (sidebar_bg, text, muted_fg, accent) = (p.sidebar, p.text, p.text_muted, p.accent);
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
                            .tooltip("刷新此账号")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(ref w) = this.worker {
                                    w.send(MailRequest::SyncAccount { account_id: acc_id });
                                    cx.notify();
                                }
                            })),
                    )
                    .child(
                        Button::new(("cfg-acc", acc_id))
                            .ghost()
                            .compact()
                            .icon(IconName::Settings)
                            .tooltip("账号设置")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let cfg = this.accounts.iter().find(|a| a.id == acc_id).cloned();
                                this.load_account_into_form(cfg.as_ref(), window, cx);
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
                // 仅在「账号文件夹」视图下高亮具体文件夹行。
                let is_selected = folder_id == selected_folder_id
                    && app.current_view == CurrentView::AccountFolder;
                let unread = folder.unread_count;
                let total = folder.total_count as u32;
                let folder_name = folder.name.clone();
                let icon = folder_icon(&folder_path, &folder_name);

                h_flex()
                    .id(("folder", folder_id))
                    .h(px(NAV_ROW_HEIGHT))
                    .pl(px(28.0))
                    .pr_2()
                    .gap_2()
                    .items_center()
                    .rounded(px(8.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(p.accent_soft()))
                    .when(is_selected, |row| row.bg(p.accent_soft()))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_folder(acc_id, folder_id, folder_path.clone(), cx);
                    }))
                    .child(
                        Icon::new(icon)
                            .with_size(px(15.0))
                            .text_color(if is_selected { accent } else { muted_fg }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .truncate()
                            .font_weight(if is_selected {
                                gpui_kit::FontWeight::SEMIBOLD
                            } else {
                                gpui_kit::FontWeight::NORMAL
                            })
                            .text_color(if is_selected { accent } else { text })
                            .child(SharedString::from(folder_name)),
                    )
                    .when(unread > 0, |row| {
                        row.child(count_pill(unread, is_selected, false, p))
                    })
                    .when(unread == 0 && total > 0, |row| {
                        row.child(
                            div()
                                .text_xs()
                                .text_color(if is_selected { accent } else { muted_fg })
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
    let current_view = app.current_view;
    // 统一视图行选中态由 `current_view` 决定（演示模式同样映射）。
    let is_unified_selected = current_view == CurrentView::UnifiedInbox;
    let is_starred_selected = current_view == CurrentView::Starred;
    let is_sent_selected = current_view == CurrentView::Sent;
    let is_drafts_selected = current_view == CurrentView::Drafts;
    let is_archive_selected = current_view == CurrentView::Archive;
    // 草稿计数：真实账号取本地草稿数，演示模式取演示草稿箱邮件总数。
    let drafts_count = if demo_mode {
        app.demo_store
            .folders
            .iter()
            .find(|f| f.id == hylki_mail_core::demo::DEMO_DRAFTS_ID)
            .map(|f| f.total_count as u32)
            .unwrap_or(0)
    } else {
        app.drafts.len() as u32
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

    // 菜单按钮的弹出菜单（账号管理 / 切换亮暗主题 / 偏好设置 / 命令面板 / 退出）。
    let menu_button = {
        let entity = cx.entity();
        let entity_for_theme = entity.clone();
        let entity_for_prefs = entity.clone();
        let entity_for_palette = entity.clone();
        let entity_for_quit = entity.clone();
        Button::new("sb-menu")
            .ghost()
            .compact()
            .icon(IconName::Menu)
            .dropdown_menu(move |menu, _, _| {
                let entity = entity.clone();
                let entity_for_theme = entity_for_theme.clone();
                let entity_for_prefs = entity_for_prefs.clone();
                let entity_for_palette = entity_for_palette.clone();
                let entity_for_quit = entity_for_quit.clone();
                menu.item(
                    // 演示模式下账号管理不可用（无 worker 账号可编辑）。
                    PopupMenuItem::new("账号管理")
                        .icon(Icon::new(IconName::Settings))
                        .disabled(demo_mode)
                        .on_click(move |_, _, cx| {
                            entity.update(cx, |app, cx| {
                                app.open_account_manager(cx);
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
                .item(
                    PopupMenuItem::new("偏好设置")
                        .icon(Icon::new(IconName::Settings2))
                        .on_click(move |_, window, cx| {
                            entity_for_prefs.update(cx, |app, cx| {
                                app.open_preferences(window, cx);
                            });
                        }),
                )
                .item(
                    PopupMenuItem::new("命令面板  Ctrl+K")
                        .icon(Icon::new(IconName::Command))
                        .on_click(move |_, window, cx| {
                            entity_for_palette.update(cx, |app, cx| {
                                app.open_command_palette(window, cx);
                            });
                        }),
                )
                .separator()
                .item(
                    PopupMenuItem::new("退出 Hylki")
                        .icon(Icon::new(IconName::LogOut))
                        .on_click(move |_, _, cx| {
                            entity_for_quit.update(cx, |app, cx| {
                                app.quit_app(cx);
                            });
                        }),
                )
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
                            // 演示模式刷新演示数据；真实账号同步所有账号（阶段 3 起
                            // 与命令面板 / Ctrl+R 共用 refresh_all）。
                            this.refresh_all(cx);
                        })),
                )
                .child(
                    h_flex()
                        .flex_1()
                        .h_full()
                        .items_center()
                        .justify_center()
                        .window_control_area(WindowControlArea::Drag)
                        .child(
                            div()
                                .text_base()
                                .font_weight(gpui_kit::FontWeight::BOLD)
                                .text_color(text)
                                .child("Hylki"),
                        ),
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
                        this.select_unified_view(CurrentView::UnifiedInbox, cx);
                    })),
                )
                // 迭代 2：已加星标 / 已发送 / 归档接入 mail-core 统一视图查询。
                .child(
                    nav_row(
                        "nav-starred",
                        IconName::Star,
                        "已加星标",
                        None,
                        is_starred_selected,
                        &p,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.select_unified_view(CurrentView::Starred, cx);
                    })),
                )
                .child(
                    nav_row(
                        "nav-sent",
                        IconName::Send,
                        "已发送",
                        None,
                        is_sent_selected,
                        &p,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.select_unified_view(CurrentView::Sent, cx);
                    })),
                )
                // 草稿箱：真实账号展示本地草稿（中栏列表 + 侧栏条目），
                // 演示模式映射到演示草稿箱文件夹。
                .child(
                    nav_row(
                        "nav-drafts",
                        IconName::FilePen,
                        "草稿箱",
                        Some((drafts_count, false)),
                        is_drafts_selected,
                        &p,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.select_unified_view(CurrentView::Drafts, cx);
                    })),
                )
                .child(
                    nav_row(
                        "nav-archive",
                        IconName::Archive,
                        "归档",
                        None,
                        is_archive_selected,
                        &p,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.select_unified_view(CurrentView::Archive, cx);
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
                .children(account_sections),
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
                // 联系人 / 附件总览主内容视图入口（阶段 3，保留左栏切换）。
                .child(
                    nav_row(
                        "nav-contacts",
                        IconName::CircleUser,
                        "联系人",
                        None,
                        app.main_view == MainContentView::Contacts,
                        &p,
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_contacts(window, cx);
                    })),
                )
                .child(
                    nav_row(
                        "nav-attachments",
                        IconName::Paperclip,
                        "附件",
                        None,
                        app.main_view == MainContentView::Gallery,
                        &p,
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_gallery(window, cx);
                    })),
                ),
        )
        .into_any_element()
}
