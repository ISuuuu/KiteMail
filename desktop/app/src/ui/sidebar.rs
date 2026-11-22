//! 左栏（方案文档第三节 1.1）：
//! - 顶部栏 48px：刷新按钮 + 居中 `Hylki` 粗体标题 + 菜单按钮
//!   （账号管理 / 显示隐藏账号列表 / 全屏 / 切换亮暗主题 / 偏好设置 /
//!   命令面板 Ctrl+K / 键盘快捷键 F1 / 关于，分组顺序对齐原版主菜单）。
//! - 全宽蓝色「写邮件」按钮（38px 高、圆角 8px、accent 色）。
//! - 统一导航区（34px 高、圆角 8、左图标 + 名称 + 右侧未读数胶囊）：
//!   收件箱 / 已加星标 / 已发送 / 草稿箱 / 归档 / 发件箱待处理。
//! - 账号折叠分组：彩色头像（角上未读小蓝圈）+ 账号名 + 邮箱小字 +
//!   右侧单账号刷新按钮 + 折叠箭头；展开列出各文件夹。
//! - 底部固定区：联系人、附件入口。

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{ContextMenuExt, DropdownMenu, PopupMenuItem};
use gpui_kit::component::resizable::{resize_handle, HandleEdge};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::Icon;
use gpui_kit::component::{h_flex, v_flex, Selectable, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    div, px, AppContext, Axis, Context, Div, InteractiveElement, IntoElement, ParentElement,
    SharedString, Stateful, StatefulInteractiveElement, Styled, WindowControlArea,
};
use hylki_mail_core::config::{self, AccountConfig, AuthBackend};
use hylki_mail_core::{MailRequest, OutboxItem, OutboxStatus};

use crate::app::{CurrentView, HylkiApp, MainContentView, ReaderPosition, SidebarDrag};
use crate::theme;
use crate::ui::avatar;

/// 左栏默认宽度（方案文档：230 ~ 250px；L-2 起可拖拽调整为 180~280，持久化到偏好）。
pub(crate) const SIDEBAR_WIDTH: f32 = 240.0;
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
    acc: &AccountConfig,
    folders: &[hylki_mail_core::FolderSummary],
    data_dir: &std::path::Path,
    p: &theme::HylkiPalette,
    cx: &mut Context<HylkiApp>,
) -> gpui_kit::AnyElement {
    let acc_id = acc.id;
    let primary_label = acc.display_label();
    let secondary_label = if primary_label == acc.email {
        acc.display_name.trim().to_string()
    } else {
        acc.email.clone()
    };
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
                .child(avatar::account_circle_with_badge(
                    acc,
                    data_dir,
                    28.0,
                    // 展开时「收件箱」行已有未读胶囊，头像角标去重；窄轨模式始终显示角标。
                    if expanded { 0 } else { account_unread },
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
                                .child(SharedString::from(primary_label)),
                        )
                        .when(!secondary_label.is_empty(), |col| {
                            col.child(
                                div()
                                    .text_xs()
                                    .text_color(muted_fg)
                                    .truncate()
                                    .child(SharedString::from(secondary_label)),
                            )
                        }),
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
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let cfg = this.accounts.iter().find(|a| a.id == acc_id).cloned();
                                this.open_settings_edit_account(cfg, cx);
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
                // 右键菜单（对齐原版 folder_menu_items 的可用子集）。
                let is_trash_or_junk = {
                    let up = folder_path.to_ascii_uppercase();
                    up.contains("TRASH")
                        || up.contains("DELETED")
                        || folder_name.contains("已删除")
                        || up.contains("JUNK")
                        || up.contains("SPAM")
                        || folder_name.contains("垃圾")
                };
                // Graph（Outlook OAuth）账号暂无批量清空能力，禁用「清空文件夹」。
                let is_graph = app
                    .accounts
                    .iter()
                    .find(|a| a.id == acc_id)
                    .map(|a| a.auth_backend == AuthBackend::MicrosoftGraphOAuth2)
                    .unwrap_or(false);
                let menu_entity = cx.entity();
                // 右键菜单独立副本：folder_path / folder_name 会在下方行构建中被 move。
                let menu_path = folder_path.clone();
                let menu_name = folder_name.clone();

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
                    .context_menu(move |menu, _, _| {
                        let menu_entity = menu_entity.clone();
                        if demo_mode {
                            // 演示模式：均为真实账号动作，全部禁用（与行内按钮惯例一致）。
                            return menu
                                .item(PopupMenuItem::new("全部标为已读").disabled(true))
                                .item(PopupMenuItem::new("刷新此文件夹").disabled(true));
                        }
                        let mut menu = menu
                            .item(PopupMenuItem::new("全部标为已读").on_click({
                                let e = menu_entity.clone();
                                let path = menu_path.clone();
                                move |_, _, cx| {
                                    e.update(cx, |this, cx| {
                                        this.mark_folder_all_read(
                                            acc_id,
                                            folder_id,
                                            path.clone(),
                                            cx,
                                        );
                                    });
                                }
                            }))
                            .item(PopupMenuItem::new("刷新此文件夹").on_click({
                                let e = menu_entity.clone();
                                let path = menu_path.clone();
                                move |_, _, cx| {
                                    e.update(cx, |this, cx| {
                                        this.refresh_folder(acc_id, folder_id, path.clone(), cx);
                                    });
                                }
                            }));
                        if is_trash_or_junk {
                            let e = menu_entity.clone();
                            let path = menu_path.clone();
                            let name = menu_name.clone();
                            menu = menu.separator().item(
                                PopupMenuItem::new("清空文件夹…")
                                    .disabled(is_graph)
                                    .on_click(move |_, _, cx| {
                                        e.update(cx, |this, cx| {
                                            this.request_empty_folder(
                                                acc_id,
                                                folder_id,
                                                path.clone(),
                                                name.clone(),
                                                cx,
                                            );
                                        });
                                    }),
                            );
                        }
                        menu
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

    let data_dir = config::data_dir();
    let account_sections: Vec<gpui_kit::AnyElement> = app
        .accounts
        .iter()
        .filter(|acc| acc.enabled)
        .map(|acc| {
            let folders = app
                .folders_by_account
                .get(&acc.id)
                .cloned()
                .unwrap_or_default();
            account_section(app, acc, &folders, &data_dir, &p, cx)
        })
        .collect();

    // 菜单按钮的弹出菜单（账号管理 / 切换亮暗主题 / 偏好设置 / 命令面板 / 退出）。
    let menu_button = main_menu_button(app, cx);

    v_flex()
        .w(px(app.sidebar_width))
        .min_w(px(crate::app::SIDEBAR_MIN_WIDTH))
        .max_w(px(crate::app::SIDEBAR_MAX_WIDTH))
        .h_full()
        .flex_shrink_0()
        .relative()
        .bg(sidebar_bg)
        .border_r_1()
        .border_color(border)
        // L-2：右缘拖拽把手（180~280 调宽，移动监听与持久化在 app.rs 根节点）；
        // 窄轨模式（render_collapsed）不挂把手。WP-01：断点自动窄轨下完整侧栏
        // 只会出现在半屏抽屉浮层里（对齐 Linux OverlaySplitView 抽屉模式宽度
        // 固定不可拖拽），此时也不挂把手。
        .when(!app.auto_rail_active, |sidebar| {
            sidebar.child(
                resize_handle::<SidebarDrag, SidebarDrag>(
                    "sidebar-resize-handle",
                    Axis::Horizontal,
                )
                .inside(HandleEdge::Trailing)
                .on_drag(SidebarDrag, |_, _, _, cx| cx.new(|_| SidebarDrag)),
            )
        })
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
        // 统一导航固定区（对齐原版 pinned_box 三段式）：置于滚动区上方，
        // 账号多时统一收件箱等入口不会被滚走。
        .child(
            v_flex()
                .flex_shrink_0()
                .px_2()
                .gap_0p5()
                .when(app.prefs.unified_show_inbox, |el| {
                    el.child(
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
                        }))
                        // 统一视图行右键：仅「全部标为已读」（对所有账号收件箱批量）。
                        .context_menu({
                            let menu_entity = cx.entity();
                            move |menu, _, _| {
                                let menu_entity = menu_entity.clone();
                                menu.item(
                                    PopupMenuItem::new("全部标为已读（所有账号收件箱）")
                                        .disabled(demo_mode)
                                        .on_click(move |_, _, cx| {
                                            menu_entity.update(cx, |this, cx| {
                                                this.mark_all_inboxes_read(cx);
                                            });
                                        }),
                                )
                            }
                        }),
                    )
                })
                // 迭代 2：已加星标 / 已发送 / 归档接入 mail-core 统一视图查询。
                .when(app.prefs.unified_show_starred, |el| {
                    el.child(
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
                })
                .when(app.prefs.unified_show_sent, |el| {
                    el.child(
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
                })
                // 草稿箱：真实账号展示本地草稿（中栏列表 + 侧栏条目），
                // 演示模式映射到演示草稿箱文件夹。
                .when(app.prefs.unified_show_drafts, |el| {
                    el.child(
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
                })
                .when(app.prefs.unified_show_archive, |el| {
                    el.child(
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
                })
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
                }),
        )
        // 账号区段：细分隔线 + 可滚动账号分组树。Ctrl+Shift+A / 主菜单
        // 「显示 / 隐藏账号列表」（show_accounts，对齐原版 Show Accounts）
        // 关闭时整体隐藏，统一导航固定区保持不动。
        .when(app.show_accounts, |el| {
            el.child(div().mx_2().my_1().h(px(1.0)).bg(border)).child(
                v_flex()
                    .id("sidebar-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .px_2()
                    .py_1()
                    .gap_0p5()
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
        })
        // 底部固定区：联系人 / 附件入口。
        .when(
            app.prefs.sidebar_show_contacts || app.prefs.sidebar_show_attachments,
            |el| {
                el.child(
                    v_flex()
                        .flex_shrink_0()
                        .px_2()
                        .py_2()
                        .gap_0p5()
                        .border_t_1()
                        .border_color(border)
                        // 联系人 / 附件总览主内容视图入口（阶段 3，保留左栏切换）。
                        .when(app.prefs.sidebar_show_contacts, |el| {
                            el.child(
                                nav_row(
                                    "nav-contacts",
                                    IconName::CircleUser,
                                    "联系人",
                                    None,
                                    app.main_view == MainContentView::Contacts,
                                    &p,
                                )
                                .on_click(cx.listener(
                                    |this, _, window, cx| {
                                        this.open_contacts(window, cx);
                                    },
                                )),
                            )
                        })
                        .when(app.prefs.sidebar_show_attachments, |el| {
                            el.child(
                                nav_row(
                                    "nav-attachments",
                                    IconName::Paperclip,
                                    "附件",
                                    None,
                                    app.main_view == MainContentView::Gallery,
                                    &p,
                                )
                                .on_click(cx.listener(
                                    |this, _, window, cx| {
                                        this.open_gallery(window, cx);
                                    },
                                )),
                            )
                        }),
                )
            },
        )
        .into_any_element()
}

/// 主菜单按钮（宽栏顶栏与窄轨共用）。分组与顺序对齐原版 boot.rs
/// `build_main_menu`：设置类（账号管理 / 显示隐藏账号列表）→ 窗口类
/// （全屏 / 主题 / 偏好设置）→ 帮助类（命令面板 / 快捷键 / 关于）→ 退出。
fn main_menu_button(app: &HylkiApp, cx: &mut Context<HylkiApp>) -> impl IntoElement {
    let dark = app.dark_mode;
    let demo_mode = app.demo_mode;
    let show_accounts = app.show_accounts;
    let reader_position = app.reader_position;
    let focus_mode = app.focus_mode;
    let entity = cx.entity();
    let entity_for_accounts = entity.clone();
    let entity_for_fullscreen = entity.clone();
    let entity_for_theme = entity.clone();
    let entity_for_prefs = entity.clone();
    let entity_for_palette = entity.clone();
    let entity_for_shortcuts = entity.clone();
    let entity_for_about = entity.clone();
    let entity_for_quit = entity.clone();
    let entity_for_reader_pos = entity.clone();
    let entity_for_focus = entity.clone();
    Button::new("sb-menu")
        .ghost()
        .compact()
        .icon(IconName::Menu)
        .dropdown_menu(move |menu, window, cx| {
            let entity = entity.clone();
            let entity_for_accounts = entity_for_accounts.clone();
            let entity_for_fullscreen = entity_for_fullscreen.clone();
            let entity_for_theme = entity_for_theme.clone();
            let entity_for_prefs = entity_for_prefs.clone();
            let entity_for_palette = entity_for_palette.clone();
            let entity_for_shortcuts = entity_for_shortcuts.clone();
            let entity_for_about = entity_for_about.clone();
            let entity_for_quit = entity_for_quit.clone();
            let entity_for_reader_pos = entity_for_reader_pos.clone();
            let entity_for_focus = entity_for_focus.clone();
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
                // 对齐原版「Show Accounts」（Ctrl+Shift+A）：隐藏 / 恢复侧栏账号区段。
                PopupMenuItem::new(if show_accounts {
                    "隐藏账号列表  Ctrl+Shift+A"
                } else {
                    "显示账号列表  Ctrl+Shift+A"
                })
                .icon(Icon::new(IconName::PanelLeft))
                .on_click(move |_, _, cx| {
                    entity_for_accounts.update(cx, |app, cx| {
                        app.toggle_accounts_list(cx);
                    });
                }),
            )
            .separator()
            // L-6：阅读器位置子菜单（右上 / 下方 / 关闭，当前项打勾，Ctrl+Alt+1/2/3），
            // 对齐原版主菜单「Reading Pane」单选组（#475）。
            .submenu("阅读器位置", window, cx, move |mut sub, _, _| {
                for (label, position) in [
                    ("右侧  Ctrl+Alt+1", ReaderPosition::Right),
                    ("下方  Ctrl+Alt+2", ReaderPosition::Below),
                    ("关闭  Ctrl+Alt+3", ReaderPosition::Hidden),
                ] {
                    let e_pos = entity_for_reader_pos.clone();
                    let is_current = reader_position == position;
                    sub = sub.item(PopupMenuItem::new(label).checked(is_current).on_click(
                        move |_, _, cx| {
                            e_pos.update(cx, |app, cx| {
                                app.set_reader_position(position, cx);
                            });
                        },
                    ));
                }
                sub
            })
            .item(
                // L-6：专注模式开关（会话级，Ctrl+Shift+F，对齐原版 focus-mode）。
                PopupMenuItem::new(if focus_mode {
                    "关闭专注模式  Ctrl+Shift+F"
                } else {
                    "专注模式  Ctrl+Shift+F"
                })
                .checked(focus_mode)
                .icon(Icon::new(IconName::Eye))
                .on_click(move |_, _, cx| {
                    entity_for_focus.update(cx, |app, cx| {
                        app.toggle_focus_mode(cx);
                    });
                }),
            )
            .item(
                PopupMenuItem::new("全屏切换  F11")
                    .icon(Icon::new(IconName::Maximize))
                    .on_click(move |_, window, cx| {
                        entity_for_fullscreen.update(cx, |app, cx| {
                            app.toggle_fullscreen(window, cx);
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
            .item(
                PopupMenuItem::new("设置  Ctrl+,")
                    .icon(Icon::new(IconName::Settings2))
                    .on_click(move |_, window, cx| {
                        entity_for_prefs.update(cx, |app, cx| {
                            app.open_preferences(window, cx);
                        });
                    }),
            )
            .separator()
            .item(
                PopupMenuItem::new("命令面板  Ctrl+K")
                    .icon(Icon::new(IconName::Command))
                    .on_click(move |_, window, cx| {
                        entity_for_palette.update(cx, |app, cx| {
                            app.open_command_palette(window, cx, false);
                        });
                    }),
            )
            .item(
                PopupMenuItem::new("键盘快捷键  F1")
                    .icon(Icon::new(IconName::Keyboard))
                    .on_click(move |_, _, cx| {
                        entity_for_shortcuts.update(cx, |app, cx| {
                            app.open_shortcuts(cx);
                        });
                    }),
            )
            .item(
                PopupMenuItem::new("关于 Hylki")
                    .icon(Icon::new(IconName::Info))
                    .on_click(move |_, _, cx| {
                        entity_for_about.update(cx, |app, cx| {
                            app.open_about(cx);
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
}

/// 折叠窄轨宽度（对齐原版 `SIDEBAR_RAIL_WIDTH` = 80px）。
pub(crate) const RAIL_WIDTH: f32 = 80.0;

/// 左栏折叠窄轨（`sidebar_hidden` 时替代宽栏，对齐原版图标窄轨）：
/// - 顶部固定：主菜单 + 刷新；下方写邮件图标按钮（触发现有写信动作）。
/// - 固定统一收件箱图标按钮，右上角迷你角标显示全账号未读总数。
/// - 中部滚动区：各账号圆形头像（28px，右上角该账号未读数角标始终显示），
///   点击恢复宽栏并展开该账号分组。
/// - 底部固定：联系人 / 附件入口。
/// - 无账号（欢迎向导阶段）时仅显示固定按钮（账号区为空即降级）。
pub(crate) fn render_collapsed(app: &HylkiApp, cx: &mut Context<HylkiApp>) -> gpui_kit::AnyElement {
    // 避坑清单第 4 条：render 开头先把语义色 Copy 出来。
    let p = theme::current(cx);
    let (sidebar_bg, border, accent, accent_fg) = (p.sidebar, p.border, p.accent, p.accent_fg);
    let demo_mode = app.demo_mode;
    let data_dir = config::data_dir();
    let unified_unread = app.unified_unread_count();
    let is_unified_selected = app.current_view == CurrentView::UnifiedInbox;

    v_flex()
        .w(px(RAIL_WIDTH))
        .h_full()
        .flex_shrink_0()
        .bg(sidebar_bg)
        .border_r_1()
        .border_color(border)
        // 顶部固定：主菜单 + 刷新（与宽栏 48px 顶栏同高）。
        .child(
            h_flex()
                .h(px(HEADER_HEIGHT))
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .gap_0p5()
                .child(main_menu_button(app, cx))
                .child(
                    Button::new("rail-refresh")
                        .ghost()
                        .compact()
                        .icon(IconName::RefreshCw)
                        .tooltip(if demo_mode {
                            "刷新演示数据"
                        } else {
                            "同步所有账号"
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.refresh_all(cx);
                        })),
                ),
        )
        // 写邮件图标按钮（触发现有 NewCompose 写信动作）。
        .child(
            div().px_3().pb_2().flex_shrink_0().child(
                Button::new("rail-compose")
                    .primary()
                    .icon(IconName::Plus)
                    .tooltip("写邮件")
                    .rounded(px(8.0))
                    .h(px(38.0))
                    .w_full()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.start_new_compose(window, cx);
                    })),
            ),
        )
        // 统一收件箱图标按钮 + 右上角全账号未读迷你角标（固定不随账号区滚动）。
        .child(
            h_flex().flex_shrink_0().justify_center().pb_2().child(
                div()
                    .relative()
                    .child(
                        Button::new("rail-unified")
                            .ghost()
                            .compact()
                            .icon(IconName::Inbox)
                            .tooltip("统一收件箱")
                            .selected(is_unified_selected)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.select_unified_view(CurrentView::UnifiedInbox, cx);
                            })),
                    )
                    .when(unified_unread > 0 && app.prefs.rail_unread_dots, |el| {
                        el.child(
                            div()
                                .absolute()
                                .top(px(-2.0))
                                .right(px(-2.0))
                                .rounded_full()
                                .w(px(8.0))
                                .h(px(8.0))
                                .bg(accent)
                                .border_1()
                                .border_color(sidebar_bg),
                        )
                    })
                    .when(unified_unread > 0 && !app.prefs.rail_unread_dots, |el| {
                        el.child(
                            div()
                                .absolute()
                                .top(px(-4.0))
                                .right(px(-6.0))
                                .rounded_full()
                                .min_w(px(16.0))
                                .h(px(16.0))
                                .bg(accent)
                                .border_1()
                                .border_color(sidebar_bg)
                                .flex()
                                .items_center()
                                .justify_center()
                                .px_0p5()
                                .text_size(px(10.0))
                                .text_color(accent_fg)
                                .font_weight(gpui_kit::FontWeight::BOLD)
                                .child(SharedString::from(if unified_unread > 99 {
                                    "99+".to_string()
                                } else {
                                    unified_unread.to_string()
                                })),
                        )
                    }),
            ),
        )
        // 账号头像滚动区（窄轨模式头像右上角未读角标始终显示）；
        // show_accounts 关闭时随宽栏一并隐藏（对齐原版 Show Accounts）。
        .when(app.show_accounts, |el| {
            let rail_dots = app.prefs.rail_unread_dots;
            el.child(
                v_flex()
                    .id("rail-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .items_center()
                    .py_1()
                    .gap_1()
                    .children(app.accounts.iter().filter(|acc| acc.enabled).map(|acc| {
                        let acc_id = acc.id;
                        let account_unread: u32 = app
                            .folders_by_account
                            .get(&acc.id)
                            .map(|folders| {
                                folders
                                    .iter()
                                    .filter(|f| {
                                        f.path.eq_ignore_ascii_case("INBOX")
                                            || f.name.contains("收件箱")
                                    })
                                    .map(|f| f.unread_count)
                                    .sum()
                            })
                            .unwrap_or(0);
                        div()
                            .id(("rail-acc", acc_id))
                            .relative()
                            .cursor_pointer()
                            .p(px(2.0))
                            .rounded(px(8.0))
                            .hover(|s| s.bg(p.accent_soft()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                // P3：断点窄轨下点击头像改为打开浮层侧栏（窄轨本身
                                // 不原地展开，对齐 Linux rail 交互）；宽窗口 / 手动
                                // 折叠时维持原行为：恢复宽栏并展开该账号分组。
                                if this.auto_rail_active {
                                    this.sidebar_peek_open = true;
                                } else {
                                    this.sidebar_hidden = false;
                                }
                                this.collapsed_accounts.remove(&acc_id);
                                cx.notify();
                            }))
                            .child(avatar::account_circle_with_badge(
                                acc,
                                &data_dir,
                                28.0,
                                if rail_dots { 0 } else { account_unread },
                                accent,
                                sidebar_bg,
                            ))
                            .when(rail_dots && account_unread > 0, |el| {
                                el.child(
                                    div()
                                        .absolute()
                                        .top(px(0.0))
                                        .right(px(0.0))
                                        .rounded_full()
                                        .w(px(8.0))
                                        .h(px(8.0))
                                        .bg(accent)
                                        .border_1()
                                        .border_color(sidebar_bg),
                                )
                            })
                    })),
            )
        })
        // 底部固定：联系人 / 附件图标入口（现有入口的窄轨形态）。
        .when(
            app.prefs.sidebar_show_contacts || app.prefs.sidebar_show_attachments,
            |el| {
                el.child(
                    v_flex()
                        .flex_shrink_0()
                        .py_2()
                        .items_center()
                        .gap_1()
                        .border_t_1()
                        .border_color(border)
                        .when(app.prefs.sidebar_show_contacts, |el| {
                            el.child(
                                Button::new("rail-contacts")
                                    .ghost()
                                    .compact()
                                    .icon(IconName::CircleUser)
                                    .tooltip("联系人")
                                    .selected(app.main_view == MainContentView::Contacts)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_contacts(window, cx);
                                    })),
                            )
                        })
                        .when(app.prefs.sidebar_show_attachments, |el| {
                            el.child(
                                Button::new("rail-attachments")
                                    .ghost()
                                    .compact()
                                    .icon(IconName::Paperclip)
                                    .tooltip("附件")
                                    .selected(app.main_view == MainContentView::Gallery)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_gallery(window, cx);
                                    })),
                            )
                        }),
                )
            },
        )
        .into_any_element()
}
