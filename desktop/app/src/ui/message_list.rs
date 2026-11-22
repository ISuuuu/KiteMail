//! 中栏（方案文档第三节 1.2 与五.3）：
//! - 顶部栏 48px（与左栏、右栏严格齐平）：侧栏折叠开关 + 搜索框（即时过滤，Esc 清空）
//!   + 列表计数 + 筛选下拉按钮（全部 / 仅未读 / 仅星标）。
//! - 邮件列表行：行高约 82px（偏好 compact_list 紧凑密度为 64px 且隐藏摘要行）、
//!   外边距 2px 6px、内边距 13px 8px 13px 10px、圆角 12px（对齐 Linux 原版样式）；
//!   左侧 36x36 圆形头像 + 固定 10px 宽未读蓝点占位（保证已读/未读文字严格左对齐）；
//!   右侧三行（发件人 / 主题 / 摘要 + 多账号胶囊），未读加粗；
//!   第 1 行时间旁为会话计数胶囊（B-4，仅会话邮件数 > 1 时显示，对齐原版 .thread-chip）；
//!   已发送 / 草稿视图第 1 行显示「致：收件人」（B-6）；
//!   选中态整行半透明 accent 底（悬停加深），复刻 Linux 原版列表行样式。
//! - 底部轻量分页条：仅当存在上一页 / 下一页时显示。
//! - 邮件行右键菜单（B-2）：回复 / 回复全部 / 转发、标记已读/未读、加星/取消星标、
//!   归档、标记为垃圾邮件、移动到子菜单、删除、复制邮件正文、查看邮件原文、导出为 .eml
//!   （演示模式保留本地的已读 / 星标 / 回复 / 复制 / 原文 / 导出项，服务端移动类隐藏）；
//!   回收站 / 垃圾邮件文件夹中「标记为垃圾邮件」替换为「非垃圾邮件（移回收件箱）」+
//!   「移回收件箱」（对齐原版 message_row / card_menu 语义）；
//!   草稿视图行右键为「打开草稿 / 删除草稿」。

use std::collections::HashSet;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{ContextMenu, ContextMenuExt, DropdownMenu, PopupMenuItem};
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::Icon;
use gpui_kit::component::{h_flex, v_flex, Disableable, Selectable, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    div, px, Context, Div, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    ParentElement, SharedString, Stateful, StatefulInteractiveElement, Styled, Window,
    WindowControlArea,
};
use hylki_mail_core::{date_group_label, LocalDraft, MessageSort, MessageSummary, SearchScope};

use crate::app::{CurrentView, HylkiApp, PendingRowAction, ReaderPosition, RightPaneMode};
use crate::theme;
use crate::ui::{avatar, window_controls};

/// 中栏默认宽度（方案文档：340 ~ 380px；L-2 起可拖拽调整并持久化到偏好）。
pub(crate) const LIST_WIDTH: f32 = 360.0;
/// 顶部栏高度（与左栏 / 右栏统一为 48px）。
const HEADER_HEIGHT: f32 = 48.0;
/// 列表行最小高度：行内容随字体度量自适应增高（三行文本按 gpui 默认
/// 1.618 倍行高需要约 95px，固定 82px 会把带 truncate 的主题行压扁、
/// 中文被竖向裁切），这里只兜底。
const ROW_HEIGHT: f32 = 82.0;
/// 紧凑列表行最小高度（偏好 compact_list，隐藏摘要行，方案文档五.2）。
const COMPACT_ROW_HEIGHT: f32 = 64.0;

/// 邮件行右键菜单（方案文档五.3 + B-2）。
///
/// `draft` 为 `Some` 时该行是本地草稿行，菜单切换为「打开草稿 / 删除草稿」。
/// `in_trash_or_junk` 时「标记为垃圾邮件」替换为恢复到收件箱的两项（对齐原版）。
#[allow(clippy::too_many_arguments)]
fn row_context_menu(
    row: Stateful<Div>,
    msg: &MessageSummary,
    draft: Option<LocalDraft>,
    account_folders: Vec<(String, String)>,
    demo_mode: bool,
    exporting_eml: bool,
    in_trash_or_junk: bool,
    inbox_path: Option<String>,
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
        let entity_for_reply = entity.clone();
        let entity_for_reply_all = entity.clone();
        let entity_for_forward = entity.clone();
        let entity_for_copy = entity.clone();
        let entity_for_source = entity.clone();
        let entity_for_eml = entity.clone();
        // 内层 on_click 为 move 闭包，需逐项克隆（外层 builder 是 Fn，可多次调用）。
        let folder_path_for_seen = folder_path.clone();
        let folder_path_for_flag = folder_path.clone();
        let folder_path_for_reply = folder_path.clone();
        let folder_path_for_reply_all = folder_path.clone();
        let folder_path_for_forward = folder_path.clone();
        let folder_path_for_copy = folder_path.clone();
        let folder_path_for_source = folder_path.clone();
        let folder_path_for_eml = folder_path.clone();
        let current_folder_path = folder_path.clone();
        let folders_for_move = account_folders.clone();
        // B-2 顶部：回复 / 回复全部 / 转发（正文未就绪时挂起动作，加载后执行）。
        let mut menu = menu
            .item(PopupMenuItem::new("回复").on_click(move |_, window, cx| {
                entity_for_reply.update(cx, |this, cx| {
                    this.begin_row_action(
                        PendingRowAction::Reply,
                        account_id,
                        message_id,
                        uid,
                        unread,
                        &folder_path_for_reply,
                        window,
                        cx,
                    );
                });
            }))
            .item(
                PopupMenuItem::new("回复全部").on_click(move |_, window, cx| {
                    entity_for_reply_all.update(cx, |this, cx| {
                        this.begin_row_action(
                            PendingRowAction::ReplyAll,
                            account_id,
                            message_id,
                            uid,
                            unread,
                            &folder_path_for_reply_all,
                            window,
                            cx,
                        );
                    });
                }),
            )
            .item(PopupMenuItem::new("转发").on_click(move |_, window, cx| {
                entity_for_forward.update(cx, |this, cx| {
                    this.begin_row_action(
                        PendingRowAction::Forward,
                        account_id,
                        message_id,
                        uid,
                        unread,
                        &folder_path_for_forward,
                        window,
                        cx,
                    );
                });
            }))
            .separator()
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
        // 演示模式仅支持本地 seen / flagged / 回复 / 复制 / 原文 / 导出项；
        // 归档 / 标垃圾 / 移动 / 删除仅真实账号可用。
        if !demo_mode {
            menu = menu
                .separator()
                .item(PopupMenuItem::new("归档").on_click(move |_, _, cx| {
                    entity_for_archive.update(cx, |this, cx| {
                        this.archive_message(account_id, message_id, cx);
                    });
                }));
            // B-2：回收站 / 垃圾邮件文件夹中把「标记为垃圾邮件」替换为恢复到收件箱两项。
            if in_trash_or_junk {
                let inbox_target = inbox_path.clone().unwrap_or_else(|| "INBOX".to_string());
                let inbox_for_not_spam = inbox_target.clone();
                menu = menu
                    .item(PopupMenuItem::new("非垃圾邮件（移回收件箱）").on_click(
                        move |_, _, cx| {
                            let p = inbox_for_not_spam.clone();
                            entity_for_spam.update(cx, |this, cx| {
                                this.move_message(account_id, message_id, p, cx);
                            });
                        },
                    ))
                    .item(PopupMenuItem::new("移回收件箱").on_click(move |_, _, cx| {
                        let p = inbox_target.clone();
                        entity_for_move.update(cx, |this, cx| {
                            this.move_message(account_id, message_id, p, cx);
                        });
                    }));
            } else {
                menu = menu.item(
                    PopupMenuItem::new("标记为垃圾邮件").on_click(move |_, _, cx| {
                        entity_for_spam.update(cx, |this, cx| {
                            this.mark_spam_message(account_id, message_id, cx);
                        });
                    }),
                );
                menu = menu.submenu("移动到文件夹...", window, cx, move |mut sub, _, _| {
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
                });
            }
            menu = menu.item(PopupMenuItem::new("删除").on_click(move |_, _, cx| {
                entity_for_delete.update(cx, |this, cx| {
                    this.delete_message(account_id, message_id, cx);
                });
            }));
        }
        // B-2 底部：复制正文 / 查看原文 / 导出 .eml（导出进行中禁用重复点击）。
        menu = menu
            .separator()
            .item(
                PopupMenuItem::new("复制邮件正文").on_click(move |_, window, cx| {
                    entity_for_copy.update(cx, |this, cx| {
                        this.begin_row_action(
                            PendingRowAction::CopyBody,
                            account_id,
                            message_id,
                            uid,
                            unread,
                            &folder_path_for_copy,
                            window,
                            cx,
                        );
                    });
                }),
            )
            .item(
                PopupMenuItem::new("查看邮件原文").on_click(move |_, window, cx| {
                    entity_for_source.update(cx, |this, cx| {
                        this.begin_row_action(
                            PendingRowAction::ViewSource,
                            account_id,
                            message_id,
                            uid,
                            unread,
                            &folder_path_for_source,
                            window,
                            cx,
                        );
                    });
                }),
            )
            .item(
                PopupMenuItem::new(if exporting_eml {
                    "导出中..."
                } else {
                    "导出为 .eml"
                })
                .disabled(exporting_eml)
                .on_click(move |_, _, cx| {
                    let p = folder_path_for_eml.clone();
                    entity_for_eml.update(cx, |this, cx| {
                        this.request_export_eml(account_id, message_id, uid, p, cx);
                    });
                }),
            );
        menu
    })
}

/// 列表行（圆角卡片）。
///
/// P3 状态参数：`list_focused` 为「列表持焦点」近似态（见 render 中的计算），
/// `kbd_cursor` 为键盘游标行（对齐 Linux .kbd-cursor）。
fn message_row(
    app: &HylkiApp,
    msg: &MessageSummary,
    draft: Option<LocalDraft>,
    is_selected: bool,
    show_account_tag: bool,
    list_focused: bool,
    kbd_cursor: bool,
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
    // B-6：已发送 / 草稿视图第 1 行显示「致：收件人」（多个收件人由 truncate 截断）。
    let show_recipients = app.list_view_shows_recipients();
    let recipients_text = msg.recipients.trim();
    let first_line = if show_recipients && !recipients_text.is_empty() {
        format!("致：{recipients_text}")
    } else {
        display_name.clone()
    };
    let acc_email = if show_account_tag || sender_email.is_empty() {
        app.account_email_for(account_id).to_string()
    } else {
        String::new()
    };
    let (acc_label, acc_color) = if show_account_tag {
        (
            app.account_label_for(account_id),
            app.account_color_for(account_id).unwrap_or(p.accent),
        )
    } else {
        (String::new(), p.accent)
    };
    // 头像取色键优先用发件人自身邮箱。
    let avatar_key = if sender_email.is_empty() {
        acc_email.clone()
    } else {
        sender_email
    };
    // 偏好「列表密度」：紧凑模式行高约 64px 且隐藏摘要行（方案文档五.2）；
    // L-2：列表拖宽 ≥600px 时自动切换单行表格模式（对齐原版 LIST_WIDE_ON，迟滞在 app.rs）。
    let compact = app.prefs.compact_list || app.list_wide;

    let row = h_flex()
        .id(("msg", msg_id))
        .flex_shrink_0()
        .min_h(px(if compact {
            COMPACT_ROW_HEIGHT
        } else {
            ROW_HEIGHT
        }))
        // 外边距 / 内边距 / 圆角对齐 Linux 原版（margin 2px 6px、padding 13 8 13 10、radius 12px）。
        .mx(px(6.0))
        .my(px(2.0))
        .pt(px(13.0))
        .pr(px(8.0))
        .pb(px(13.0))
        .pl(px(10.0))
        .gap_2()
        // 头像在整行垂直居中（对齐原版 avatar valign center）；文本列 h_full 撑满不受影响。
        .items_center()
        .rounded(px(12.0))
        .cursor_pointer()
        .overflow_hidden()
        // P3 状态底色矩阵（对齐 Linux styles.css .message-row 区块）：
        // - 悬停未选中：中性微灰（原版 alpha(currentColor, 0.05)）；
        // - 选中且列表持焦点（或悬停经过）：accent 0.5 药丸（原版 focus-within
        //   与 :selected:hover 同为 alpha(accent_bg_color, 0.5)）；
        // - 选中但焦点离开列表（阅读器）：变暗为 0.3，不清除（dimming, never
        //   vanishing）。
        .hover(move |s| {
            if is_selected {
                s.bg(p.accent.opacity(0.50))
            } else {
                s.bg(p.text.opacity(0.05))
            }
        })
        .when(is_selected, |row| {
            row.bg(if list_focused {
                p.accent.opacity(0.50)
            } else {
                p.accent.opacity(0.30)
            })
        })
        // P3：键盘游标行（原版 .kbd-cursor，列表持焦点时才画）：非选中行加
        // accent 0.1 底 + accent 0.85 内描边；选中行药丸上改用 正文色 0.8
        // 内描边（accent-on-accent 会消失）。gpui 无 inset box-shadow，用 2px
        // 圆角边框近似（行盒内绘制，不改布局）。
        .when(kbd_cursor && list_focused, |row| {
            let ring = if is_selected {
                p.text.opacity(0.8)
            } else {
                p.accent.opacity(0.85)
            };
            row.border_2()
                .border_color(ring)
                .when(!is_selected, |row| row.bg(p.accent.opacity(0.10)))
        })
        .on_click({
            let draft = draft.clone();
            cx.listener(move |this, event: &gpui_kit::ClickEvent, window, cx| {
                // P3：点击行视为列表持焦点，并清除键盘游标（原版 kbd-cursor 在
                // 点击时移除，a click never draws it）。
                this.list_focused = true;
                this.kbd_cursor_message_id = None;
                match draft.as_ref() {
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
                }
            })
        })
        // 左侧：可选 36x36 圆形头像 + 固定 10px 宽未读圆点占位（确保已读/未读文字严格左对齐）。
        .child(
            h_flex()
                .flex_shrink_0()
                .items_center()
                .gap_1()
                .when(app.prefs.show_avatars, |el| {
                    el.child(avatar::avatar(&display_name, &avatar_key, 36.0))
                })
                .child(
                    div()
                        .w(px(10.0))
                        .h(px(10.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(unread, |dot| {
                            // 未读圆点 10x10（原版 .unread-dot）：选中行在 accent 药丸上
                            // 使用正文色以保证浅色选中背景上的对比度。
                            dot.child(
                                div()
                                    .w(px(10.0))
                                    .h(px(10.0))
                                    .rounded_full()
                                    .bg(if is_selected { p.text } else { p.accent }),
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
                                // 发件人字重层次（原版 .message-sender：已读 / 未读加粗）；
                                // 已发送 / 草稿视图显示「致：收件人」（B-6）。
                                .font_weight(if unread {
                                    gpui_kit::FontWeight::BOLD
                                } else {
                                    gpui_kit::FontWeight::MEDIUM
                                })
                                // 半透明选中背景使用正文色，兼顾亮色与暗色主题。
                                .text_color(p.text)
                                .child(SharedString::from(first_line)),
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
                                            .text_color(theme::star_gold()),
                                    )
                                })
                                .when(msg.has_attachment, |icons| {
                                    icons.child(
                                        Icon::new(IconName::Paperclip)
                                            .with_size(px(13.0))
                                            // P3：选中行附件图标使用正文色。
                                            .text_color(if is_selected {
                                                p.text
                                            } else {
                                                p.text_muted
                                            }),
                                    )
                                })
                                .child({
                                    let now_ts = std::time::SystemTime::now()
                                        .duration_since(std::time::UNIX_EPOCH)
                                        .map(|d| d.as_secs() as i64)
                                        .unwrap_or(0);
                                    let date_text = hylki_mail_core::format_message_date(
                                        msg.date_ts,
                                        &msg.date_display,
                                        &app.prefs.date_style,
                                        &app.prefs.clock_style,
                                        now_ts,
                                    );
                                    div()
                                        .text_xs()
                                        // P3：选中行时间使用正文色。
                                        .text_color(if is_selected { p.text } else { p.text_muted })
                                        .child(SharedString::from(date_text))
                                })
                                // B-4 会话计数胶囊（对齐原版 .thread-chip：仅 >1 封往来时显示，
                                // 紧凑圆角、10px 粗体、主题色弱底）；选中时使用卡片底色和正文色。
                                .when(msg.thread_count > 1, |icons| {
                                    icons.child(
                                        div()
                                            .flex_shrink_0()
                                            .px(px(7.0))
                                            .py(px(2.0))
                                            .rounded_full()
                                            .bg(if is_selected {
                                                p.surface_raised
                                            } else {
                                                p.accent_soft()
                                            })
                                            .text_size(px(10.0))
                                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                            .text_color(if is_selected { p.text } else { p.accent })
                                            .child(SharedString::from(format!(
                                                "{} ▸",
                                                msg.thread_count
                                            ))),
                                    )
                                }),
                        ),
                )
                // 第 2 行：主题（未读加粗；已读 85% 透明度，复刻原版 .message-subject 层次；
                // P3：选中行使用正文色，覆盖已读的 85% 透明度）。
                .child(
                    div()
                        .text_sm()
                        .truncate()
                        .font_weight(if unread {
                            gpui_kit::FontWeight::SEMIBOLD
                        } else {
                            gpui_kit::FontWeight::NORMAL
                        })
                        .text_color(if is_selected {
                            p.text
                        } else if unread {
                            p.text
                        } else {
                            p.text.opacity(0.85)
                        })
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
                                    // 摘要 50% 透明度（原版 .message-preview）；
                                    // P3：选中行使用正文色。
                                    .text_color(if is_selected {
                                        p.text
                                    } else {
                                        p.text.opacity(0.5)
                                    })
                                    .child(SharedString::from(msg.preview.clone())),
                            )
                            .when(!acc_label.is_empty(), |el| {
                                el.child(
                                    div()
                                        .flex_shrink_0()
                                        .max_w(px(110.0))
                                        .truncate()
                                        .px_1p5()
                                        .py_0p5()
                                        .rounded_full()
                                        .text_size(px(10.0))
                                        // P3：选中行账号标签反转为中性胶囊（Linux 无
                                        // 对应规则；accent 药丸上沿用账号彩色会沉底，
                                        // 改为正文色弱底和正文色文字保层级）。
                                        .bg(if is_selected {
                                            p.text.opacity(0.12)
                                        } else {
                                            acc_color.opacity(0.14)
                                        })
                                        .text_color(if is_selected { p.text } else { acc_color })
                                        .child(SharedString::from(acc_label)),
                                )
                            }),
                    )
                }),
        );

    let account_folders: Vec<(String, String)> = app
        .folders_by_account
        .get(&account_id)
        .map(|list| {
            list.iter()
                .map(|f| (f.name.clone(), f.path.clone()))
                .collect()
        })
        .unwrap_or_default();
    // B-2：导出进行中状态与回收站 / 垃圾邮件文件夹语义。
    let exporting_eml = app.pending_eml_export == Some(msg.id);
    let in_trash_or_junk = app.folder_is_trash_or_junk(account_id, msg.folder_id);
    let inbox_path = app.inbox_folder_path(account_id);
    row_context_menu(
        row,
        msg,
        draft,
        account_folders,
        app.demo_mode,
        exporting_eml,
        in_trash_or_junk,
        inbox_path,
        cx,
    )
}

/// 渲染中栏。
pub(crate) fn render(
    app: &HylkiApp,
    window: &Window,
    cx: &mut Context<HylkiApp>,
) -> gpui_kit::AnyElement {
    // 避坑清单第 4 条：render 开头先把语义色 Copy 出来。
    let p = theme::current(cx);
    let (surface, border, muted_fg) = (p.surface, p.border, p.text_muted);

    let selected_id = app.selected_message.as_ref().map(|m| m.id);
    let has_search_query = !app.search_query.trim().is_empty();
    let cross_folder_search = has_search_query && app.search_scope == SearchScope::AllFolders;
    // 本地草稿视图（真实账号）：从 self.drafts 合成列表行（跨文件夹搜索时不拦截）。
    let drafts_view =
        app.current_view == CurrentView::Drafts && !app.demo_mode && !cross_folder_search;
    let show_account_tag = (app.current_view.is_unified() || cross_folder_search)
        && app.accounts.iter().filter(|a| a.enabled).count() > 1;
    let current_sort = app.list_sort;
    let quick_filters = app.quick_filters;
    let is_unified_list = !drafts_view;
    // P3：「列表持焦点」近似态（gpui 没有列表级 focus-within）：最近一次列表
    // 交互（点击行 / 键盘导航）且当前无文本输入持焦点时视为聚焦，驱动选中行
    // 药丸 0.5 / 0.3 两档与键盘游标轮廓；点击阅读器由 reader 的 mouse_down 清除。
    let list_focused = app.list_focused && !app.is_text_input_focused(window, cx);

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
                    list_focused,
                    // 草稿行不参与键盘游标（无选择语义）。
                    false,
                    &p,
                    cx,
                )
                .into_any_element()
            })
            .collect()
    } else {
        Vec::new()
    };

    // 合并本地结果与服务端在线搜索命中（按 id 与 (account_id, uid) 去重）。
    let mut merged_items: Vec<&MessageSummary> = Vec::new();
    let mut extra_remote_count = 0usize;
    if !drafts_view {
        let mut seen_ids: HashSet<u32> = HashSet::new();
        let mut seen_uid_keys: HashSet<(u32, u32)> = HashSet::new();
        for msg in &app.current_page.items {
            seen_ids.insert(msg.id);
            seen_uid_keys.insert((msg.account_id, msg.uid));
            merged_items.push(msg);
        }
        if has_search_query && app.server_search_query.trim() == app.search_query.trim() {
            for msg in &app.server_search_results {
                if !seen_ids.contains(&msg.id)
                    && !seen_uid_keys.contains(&(msg.account_id, msg.uid))
                {
                    seen_ids.insert(msg.id);
                    seen_uid_keys.insert((msg.account_id, msg.uid));
                    merged_items.push(msg);
                    extra_remote_count += 1;
                }
            }
        }
    }

    // 常规邮件行：按时间排序且未关闭分组偏好时在列表行之间插入日期分组头（今天 / 昨天 / 本周 / 更早）。
    let show_date_groups =
        !drafts_view && app.prefs.list_group_by_date && current_sort.uses_date_groups();
    let now_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let mut filtered_rows: Vec<gpui_kit::AnyElement> = Vec::with_capacity(merged_items.len() + 4);
    if !drafts_view {
        let mut last_group: Option<&'static str> = None;
        for msg in &merged_items {
            if show_date_groups {
                let group = date_group_label(msg.date_ts, now_ts);
                if last_group != Some(group) {
                    last_group = Some(group);
                    filtered_rows.push(
                        div()
                            .flex_shrink_0()
                            .px_4()
                            .pt_2()
                            .pb_1()
                            .text_xs()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .text_color(muted_fg)
                            .child(group)
                            .into_any_element(),
                    );
                }
            }
            let is_selected =
                selected_id == Some(msg.id) || app.selected_message_ids.contains(&msg.id);
            // P3：键盘游标行（j/k、方向键移动，点击行后清除）。
            let kbd_cursor = app.kbd_cursor_message_id == Some(msg.id);
            filtered_rows.push(
                message_row(
                    app,
                    msg,
                    None,
                    is_selected,
                    show_account_tag,
                    list_focused,
                    kbd_cursor,
                    &p,
                    cx,
                )
                .into_any_element(),
            );
        }
    }

    let loaded_count = if drafts_view {
        draft_rows.len()
    } else {
        app.current_page.items.len()
    };
    let total = if drafts_view {
        draft_rows.len()
    } else {
        app.current_page.total + extra_remote_count
    };
    let has_more_local = !drafts_view && loaded_count < app.current_page.total;
    let has_more_remote =
        !drafts_view && !cross_folder_search && app.current_view_has_more_remote();
    let has_more = has_more_local || has_more_remote;
    let is_loading_more = !drafts_view && (app.loading_more_local || app.loading_more_remote);

    let has_visible_rows = if drafts_view {
        !draft_rows.is_empty()
    } else {
        !merged_items.is_empty()
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
    } else if has_search_query {
        (
            IconName::Search,
            "未找到匹配的邮件",
            "可尝试切换「全部文件夹」或按回车「搜服务器」",
        )
    } else if !quick_filters.is_empty() {
        (IconName::ListFilter, "当前筛选条件下暂无邮件", "")
    } else {
        (IconName::Inbox, "暂无邮件", "")
    };

    // 顶部排序与多条件筛选下拉菜单。
    let filter_menu_button = {
        let entity = cx.entity();
        let active_labels = quick_filters.active_labels();
        let active_tooltip = if active_labels.is_empty() {
            format!("排序与筛选（{}）", current_sort.label())
        } else {
            format!(
                "排序：{}｜已筛选：{}",
                current_sort.label(),
                active_labels.join("、")
            )
        };
        let has_active_filter = !quick_filters.is_empty();
        Button::new("list-filter-menu")
            .ghost()
            .compact()
            .icon(IconName::ListFilter)
            .when(has_active_filter, |btn| btn.text_color(p.accent))
            .tooltip(active_tooltip)
            .dropdown_menu(move |menu, _, _| {
                let mut menu = menu;
                for sort in MessageSort::ALL {
                    let entity_for_item = entity.clone();
                    let is_current = current_sort == sort;
                    menu = menu.item(
                        PopupMenuItem::new(sort.label())
                            .when(is_current, |item| item.icon(Icon::new(IconName::Check)))
                            .on_click(move |_, _, cx| {
                                entity_for_item.update(cx, |this, cx| {
                                    this.set_list_sort(sort, cx);
                                });
                            }),
                    );
                }
                menu = menu.separator();

                let e_unread = entity.clone();
                menu = menu.item(
                    PopupMenuItem::new("仅未读")
                        .when(quick_filters.unread, |item| {
                            item.icon(Icon::new(IconName::Check))
                        })
                        .on_click(move |_, _, cx| {
                            e_unread.update(cx, |this, cx| this.toggle_quick_filter_unread(cx));
                        }),
                );

                let e_starred = entity.clone();
                menu = menu.item(
                    PopupMenuItem::new("仅星标")
                        .when(quick_filters.starred, |item| {
                            item.icon(Icon::new(IconName::Check))
                        })
                        .on_click(move |_, _, cx| {
                            e_starred.update(cx, |this, cx| this.toggle_quick_filter_starred(cx));
                        }),
                );

                let e_att = entity.clone();
                menu = menu.item(
                    PopupMenuItem::new("含附件")
                        .when(quick_filters.has_attachment, |item| {
                            item.icon(Icon::new(IconName::Check))
                        })
                        .on_click(move |_, _, cx| {
                            e_att.update(cx, |this, cx| this.toggle_quick_filter_attachment(cx));
                        }),
                );

                let e_today = entity.clone();
                menu = menu.item(
                    PopupMenuItem::new("今天")
                        .when(quick_filters.today, |item| {
                            item.icon(Icon::new(IconName::Check))
                        })
                        .on_click(move |_, _, cx| {
                            e_today.update(cx, |this, cx| this.toggle_quick_filter_today(cx));
                        }),
                );

                let e_clear = entity.clone();
                menu = menu.separator().item(
                    PopupMenuItem::new("清除筛选")
                        .disabled(quick_filters.is_empty())
                        .on_click(move |_, _, cx| {
                            e_clear.update(cx, |this, cx| this.clear_quick_filters(cx));
                        }),
                );
                menu
            })
    };

    let can_search_server = !app.demo_mode && app.search_query.trim().chars().count() >= 2;
    let search_scope = app.search_scope;
    let server_loading = app.server_search_loading;
    let search_open = app.search_open;
    // 搜索栏行显示条件：点 🔍 / Ctrl+F 展开时，已有搜索词时，或「始终显示搜索栏」偏好开启时。
    let show_search_bar = search_open || has_search_query || app.prefs.list_search_always_open;
    let search_bottom = app.prefs.list_search_bottom;
    let bulk_bottom = app.prefs.list_bulk_bar_bottom;
    let show_bulk_bar = !drafts_view && app.selected_message_ids.len() > 1;
    let bulk_count = app.selected_message_ids.len();
    let bulk_demo_mode = app.demo_mode;
    // L-6：阅读器在下方 / 关闭时列表顶栏即窗口顶栏（对齐原版 controls_in_list），
    // 需在此承载 Windows 窗口控制按钮（阅读器右侧时由阅读器顶栏承载）。
    let controls_in_list = !app.focus_mode
        && app.right_mode != RightPaneMode::Welcome
        && app.reader_position != ReaderPosition::Right;

    // 宽度由外层容器决定（L-2：可拖拽 / Hidden 模式占满），列表本体填满容器。
    v_flex()
        .w_full()
        .h_full()
        .flex_shrink_0()
        .bg(surface)
        .border_r_1()
        .border_color(border)
        // 顶部栏 48px（与左栏、右栏对齐）：折叠开关 + 🔍 图标 + 计数 + 筛选下拉。
        // 搜索框收纳进顶栏下方按需展开的搜索栏行（对齐原版 L-3）。
        .child(
            h_flex()
                .h(px(HEADER_HEIGHT))
                .flex_shrink_0()
                .px_2()
                .gap_1p5()
                .items_center()
                .border_b_1()
                .border_color(border)
                .child(
                    Button::new("list-toggle-sidebar")
                        .ghost()
                        .compact()
                        // P3：窄轨（手动折叠或断点自动折叠）且浮层未开时显示展开
                        // 图标；断点窄轨下点击 toggle 打开 / 关闭浮层侧栏而非原地展开。
                        .icon(if app.sidebar_narrow() && !app.sidebar_peek_open {
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
                    Button::new("list-search-toggle")
                        .ghost()
                        .compact()
                        .icon(IconName::Search)
                        .tooltip("搜索邮件（Ctrl+F）")
                        .selected(search_open)
                        .on_click(cx.listener(|this, _, _, cx| {
                            // 再点 🔍 收起（有搜索词时范围胶囊行保持显示）。
                            this.search_open = !this.search_open;
                            cx.notify();
                        })),
                )
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .items_center()
                        .window_control_area(WindowControlArea::Drag)
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted_fg)
                                .child(SharedString::from(count_text)),
                        ),
                )
                .when(is_unified_list, |el| el.child(filter_menu_button))
                // L-6：阅读器不在右侧（下方 / 关闭）时本顶栏即窗口顶栏，承载
                // Windows 窗口控制按钮；拖拽热区由中间计数区的 WindowControlArea 补足。
                .when(controls_in_list, |el| {
                    el.child(window_controls::render_window_controls(
                        "list-wc", window, cx,
                    ))
                }),
        )
        // 搜索栏行（根据偏好置于顶部栏下方或列表底部）。
        .when(show_search_bar && !search_bottom, |el| {
            el.child(render_search_bar(
                app,
                search_scope,
                can_search_server,
                server_loading,
                false,
                &p,
                cx,
            ))
        })
        // 多选批量操作条（Ctrl+点击选中 > 1 封邮件时出现，可配置置顶或置底）。
        .when(show_bulk_bar && !bulk_bottom, |el| {
            el.child(render_bulk_bar(bulk_count, bulk_demo_mode, false, &p, cx))
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
                                                f.child(div().text_xs().text_color(muted_fg).child(
                                                    SharedString::from(format!(
                                                        "已加载全部 {total} 封邮件"
                                                    )),
                                                ))
                                            }),
                                    )
                                }),
                        ),
                )
                .child(Scrollbar::vertical(&app.list_scroll_handle)),
        )
        .when(show_bulk_bar && bulk_bottom, |el| {
            el.child(render_bulk_bar(bulk_count, bulk_demo_mode, true, &p, cx))
        })
        .when(show_search_bar && search_bottom, |el| {
            el.child(render_search_bar(
                app,
                search_scope,
                can_search_server,
                server_loading,
                true,
                &p,
                cx,
            ))
        })
        .into_any_element()
}

fn render_search_bar(
    app: &HylkiApp,
    search_scope: SearchScope,
    can_search_server: bool,
    server_loading: bool,
    bottom: bool,
    p: &theme::HylkiPalette,
    cx: &mut Context<HylkiApp>,
) -> gpui_kit::AnyElement {
    h_flex()
        .h(px(34.0))
        .flex_shrink_0()
        .px_2()
        .gap_1()
        .items_center()
        .when(bottom, |el| el.border_t_1())
        .when(!bottom, |el| el.border_b_1())
        .border_color(p.border)
        .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
            // Esc 清空搜索并收起；Enter 在搜索词长度 >= 2 时触发服务端在线搜索。
            if event.keystroke.key == "escape" {
                this.search_open = false;
                this.search_input.update(cx, |s, cx| {
                    s.set_value("", window, cx);
                });
                this.search_query = String::new();
                this.clear_server_search();
                this.reset_list_pagination();
                this.reload_current_page(cx);
            } else if event.keystroke.key == "enter" {
                this.trigger_server_search(cx);
            }
        }))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(Input::new(&app.search_input)),
        )
        .child(
            div()
                .id("search-scope-this")
                .flex_shrink_0()
                .px_2()
                .py_0p5()
                .rounded_full()
                .text_xs()
                .cursor_pointer()
                .bg(if search_scope == SearchScope::ThisFolder {
                    p.accent
                } else {
                    p.accent_soft()
                })
                .text_color(if search_scope == SearchScope::ThisFolder {
                    p.accent_fg
                } else {
                    p.text
                })
                .on_click(cx.listener(|this, _, _, cx| {
                    this.set_search_scope(SearchScope::ThisFolder, cx);
                }))
                .child("当前视图"),
        )
        .child(
            div()
                .id("search-scope-all")
                .flex_shrink_0()
                .px_2()
                .py_0p5()
                .rounded_full()
                .text_xs()
                .cursor_pointer()
                .bg(if search_scope == SearchScope::AllFolders {
                    p.accent
                } else {
                    p.accent_soft()
                })
                .text_color(if search_scope == SearchScope::AllFolders {
                    p.accent_fg
                } else {
                    p.text
                })
                .on_click(cx.listener(|this, _, _, cx| {
                    this.set_search_scope(SearchScope::AllFolders, cx);
                }))
                .child("全部文件夹"),
        )
        .when(can_search_server, |bar| {
            bar.child(
                Button::new("search-server-btn")
                    .ghost()
                    .compact()
                    .label(if server_loading {
                        "搜索中..."
                    } else {
                        "搜服务器"
                    })
                    .disabled(server_loading)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.trigger_server_search(cx);
                    })),
            )
        })
        .into_any_element()
}

fn render_bulk_bar(
    count: usize,
    demo_mode: bool,
    bottom: bool,
    p: &theme::HylkiPalette,
    cx: &mut Context<HylkiApp>,
) -> gpui_kit::AnyElement {
    h_flex()
        .flex_shrink_0()
        .px_2()
        .py_1()
        .gap_1()
        .flex_wrap()
        .items_center()
        .justify_between()
        .when(bottom, |el| el.border_t_1())
        .when(!bottom, |el| el.border_b_1())
        .border_color(p.border)
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
        )
        .into_any_element()
}
