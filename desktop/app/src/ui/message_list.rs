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

use std::collections::{HashMap, HashSet};

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{ContextMenu, ContextMenuExt, DropdownMenu, PopupMenuItem};
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::Icon;
use gpui_kit::component::{h_flex, v_flex, Disableable, Selectable, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    div, px, AppContext, Context, Div, DragMoveEvent, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, ParentElement, SharedString, Stateful, StatefulInteractiveElement,
    Styled, Window, WindowControlArea,
};
use hylki_mail_core::config::ListColumn;
use hylki_mail_core::{date_group_label, LocalDraft, MessageSort, MessageSummary, SearchScope};

use crate::app::{
    ColumnResizeDrag, CurrentView, HylkiApp, PendingRowAction, ReaderPosition, RightPaneMode,
};
use crate::theme;
use crate::ui::{avatar, window_controls};

/// WP-08：快捷标签槽位（1..9）。
const QUICK_TAGS: [&str; 9] = [
    "重要",
    "工作",
    "个人",
    "待办",
    "稍后",
    "账单",
    "旅行",
    "项目",
    "归档参考",
];

/// WP-08：宽表默认单行高度（36px）。
const WIDE_ROW_HEIGHT: f32 = 36.0;

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
    all_account_folders: Vec<(u32, String, Vec<(String, String)>)>,
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
    let msg_tags = msg.tags.clone();
    let msg_sender = msg.sender.clone();
    let msg_subject = msg.subject.clone();
    let snoozed_until_ts = msg.snoozed_until_ts;
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
        let entity_for_copy_to = entity.clone();
        let entity_for_delete = entity.clone();
        let entity_for_reply = entity.clone();
        let entity_for_reply_all = entity.clone();
        let entity_for_forward = entity.clone();
        let entity_for_edit_new = entity.clone();
        let entity_for_tags = entity.clone();
        let entity_for_snooze = entity.clone();
        let entity_for_add_contact = entity.clone();
        let entity_for_filter = entity.clone();
        let entity_for_copy = entity.clone();
        let entity_for_source = entity.clone();
        let entity_for_eml = entity.clone();
        // 内层 on_click 为 move 闭包，需逐项克隆（外层 builder 是 Fn，可多次调用）。
        let folder_path_for_seen = folder_path.clone();
        let folder_path_for_flag = folder_path.clone();
        let folder_path_for_reply = folder_path.clone();
        let folder_path_for_reply_all = folder_path.clone();
        let folder_path_for_forward = folder_path.clone();
        let folder_path_for_edit_new = folder_path.clone();
        let folder_path_for_copy = folder_path.clone();
        let folder_path_for_source = folder_path.clone();
        let folder_path_for_eml = folder_path.clone();
        let current_folder_path = folder_path.clone();
        let folders_for_move = account_folders.clone();
        let all_folders_for_copy = all_account_folders.clone();
        let tags_for_menu = msg_tags.clone();
        let sender_for_contact = msg_sender.clone();
        let sender_for_filter = msg_sender.clone();
        let subject_for_filter = msg_subject.clone();
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
            .item(
                PopupMenuItem::new("作为新邮件编辑").on_click(move |_, window, cx| {
                    entity_for_edit_new.update(cx, |this, cx| {
                        this.begin_row_action(
                            PendingRowAction::EditAsNew,
                            account_id,
                            message_id,
                            uid,
                            unread,
                            &folder_path_for_edit_new,
                            window,
                            cx,
                        );
                    });
                }),
            )
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
            )
            .submenu("标签", window, cx, move |mut sub, _, _| {
                for (idx, tag_name) in QUICK_TAGS.iter().enumerate() {
                    let has_tag = tags_for_menu.iter().any(|t| t == tag_name);
                    let e_tag = entity_for_tags.clone();
                    let label = format!("{}. {tag_name}", idx + 1);
                    sub = sub.item(
                        PopupMenuItem::new(label)
                            .when(has_tag, |item| item.icon(Icon::new(IconName::Check)))
                            .on_click(move |_, _, cx| {
                                e_tag.update(cx, |this, cx| {
                                    this.toggle_message_tag(account_id, message_id, tag_name, cx);
                                });
                            }),
                    );
                }
                let e_clear_tag = entity_for_tags.clone();
                sub.separator().item(
                    PopupMenuItem::new("清除所有标签")
                        .disabled(tags_for_menu.is_empty())
                        .on_click(move |_, _, cx| {
                            e_clear_tag.update(cx, |this, cx| {
                                this.clear_message_tags(account_id, message_id, cx);
                            });
                        }),
                )
            })
            .submenu("稍后提醒", window, cx, move |mut sub, _, _| {
                let now_ts = chrono::Local::now().timestamp();
                let options: [(&str, i64); 5] = [
                    ("今天晚些时候", now_ts + 3 * 3600),
                    ("明天", now_ts + 24 * 3600),
                    ("本周末", now_ts + 3 * 24 * 3600),
                    ("下周一", now_ts + 7 * 24 * 3600),
                    ("自定义...", now_ts + 2 * 24 * 3600),
                ];
                for (label, until_ts) in options {
                    let e_sn = entity_for_snooze.clone();
                    sub = sub.item(PopupMenuItem::new(label).on_click(move |_, _, cx| {
                        e_sn.update(cx, |this, cx| {
                            this.snooze_message(account_id, message_id, until_ts, label, cx);
                        });
                    }));
                }
                if snoozed_until_ts > 0 {
                    let e_unsnooze = entity_for_snooze.clone();
                    sub = sub
                        .separator()
                        .item(
                            PopupMenuItem::new("取消稍后提醒").on_click(move |_, _, cx| {
                                e_unsnooze.update(cx, |this, cx| {
                                    this.snooze_message(account_id, message_id, 0, "", cx);
                                });
                            }),
                        );
                }
                sub
            });
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
            menu = menu.submenu("复制到文件夹...", window, cx, move |mut sub, _, _| {
                if all_folders_for_copy.is_empty() {
                    return sub.item(PopupMenuItem::new("暂无可用文件夹").disabled(true));
                }
                for (target_acc_id, acc_label, folders) in &all_folders_for_copy {
                    let target_acc_id = *target_acc_id;
                    for (fname, fpath) in folders {
                        let target_path = fpath.clone();
                        let e_cp = entity_for_copy_to.clone();
                        let item_label = if all_folders_for_copy.len() > 1 {
                            format!("{acc_label} / {fname}")
                        } else {
                            fname.clone()
                        };
                        sub = sub.item(PopupMenuItem::new(item_label).on_click(move |_, _, cx| {
                            let p = target_path.clone();
                            e_cp.update(cx, |this, cx| {
                                this.copy_message_to(account_id, message_id, target_acc_id, p, cx);
                            });
                        }));
                    }
                }
                sub
            });
            menu = menu.item(PopupMenuItem::new("删除").on_click(move |_, _, cx| {
                entity_for_delete.update(cx, |this, cx| {
                    this.delete_message(account_id, message_id, cx);
                });
            }));
        }
        // B-2 + WP-08 底部：添加联系人 / 创建过滤器 / 复制正文 / 查看原文 / 导出 .eml。
        menu = menu
            .separator()
            .item(
                PopupMenuItem::new("添加发件人到通讯录").on_click(move |_, _, cx| {
                    let s = sender_for_contact.clone();
                    entity_for_add_contact.update(cx, |this, cx| {
                        this.add_sender_to_contacts(&s, cx);
                    });
                }),
            )
            .item(
                PopupMenuItem::new("以此邮件创建过滤器...").on_click(move |_, _, cx| {
                    let s = sender_for_filter.clone();
                    let sub = subject_for_filter.clone();
                    entity_for_filter.update(cx, |this, cx| {
                        this.create_filter_from_message(&s, &sub, cx);
                    });
                }),
            )
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

fn row_is_wide(app: &HylkiApp, drafts_view: bool) -> bool {
    !drafts_view && (app.list_wide || app.prefs.list_layout == "wide")
}

fn column_sort_for(col: ListColumn, current_sort: MessageSort) -> MessageSort {
    match col {
        ListColumn::Date | ListColumn::Due => {
            if current_sort == MessageSort::DateNewest {
                MessageSort::DateOldest
            } else {
                MessageSort::DateNewest
            }
        }
        ListColumn::Sender | ListColumn::Recipients | ListColumn::Correspondents => {
            if current_sort == MessageSort::SenderAsc {
                MessageSort::SenderDesc
            } else {
                MessageSort::SenderAsc
            }
        }
        ListColumn::Subject => {
            if current_sort == MessageSort::SubjectAsc {
                MessageSort::SubjectDesc
            } else {
                MessageSort::SubjectAsc
            }
        }
        ListColumn::Account => {
            if current_sort == MessageSort::AccountAsc {
                MessageSort::AccountDesc
            } else {
                MessageSort::AccountAsc
            }
        }
        ListColumn::Star => MessageSort::StarredFirst,
        ListColumn::Attachment => MessageSort::AttachmentFirst,
        ListColumn::Importance => MessageSort::ImportanceFirst,
        ListColumn::Tags => MessageSort::TagsAsc,
        ListColumn::Replied => MessageSort::UnreadFirst,
    }
}

fn column_sort_arrow(col: ListColumn, current_sort: MessageSort) -> Option<&'static str> {
    match (col, current_sort) {
        (ListColumn::Date | ListColumn::Due, MessageSort::DateNewest) => Some(" ▾"),
        (ListColumn::Date | ListColumn::Due, MessageSort::DateOldest) => Some(" ▴"),
        (
            ListColumn::Sender | ListColumn::Recipients | ListColumn::Correspondents,
            MessageSort::SenderAsc,
        ) => Some(" ▴"),
        (
            ListColumn::Sender | ListColumn::Recipients | ListColumn::Correspondents,
            MessageSort::SenderDesc,
        ) => Some(" ▾"),
        (ListColumn::Subject, MessageSort::SubjectAsc) => Some(" ▴"),
        (ListColumn::Subject, MessageSort::SubjectDesc) => Some(" ▾"),
        (ListColumn::Account, MessageSort::AccountAsc) => Some(" ▴"),
        (ListColumn::Account, MessageSort::AccountDesc) => Some(" ▾"),
        (ListColumn::Star, MessageSort::StarredFirst) => Some(" ▾"),
        (ListColumn::Attachment, MessageSort::AttachmentFirst) => Some(" ▾"),
        (ListColumn::Importance, MessageSort::ImportanceFirst) => Some(" ▾"),
        (ListColumn::Tags, MessageSort::TagsAsc) => Some(" ▴"),
        _ => None,
    }
}

fn column_resizable(col: ListColumn) -> bool {
    matches!(
        col,
        ListColumn::Sender
            | ListColumn::Recipients
            | ListColumn::Correspondents
            | ListColumn::Account
            | ListColumn::Tags
            | ListColumn::Date
            | ListColumn::Due
    )
}

/// WP-08：宽表列头栏（`.list-headings-bar`）：显示 `config.list_columns`，点击排序，拖拽分隔线调宽。
fn render_headings_bar(
    app: &HylkiApp,
    p: &theme::HylkiPalette,
    cx: &mut Context<HylkiApp>,
) -> gpui_kit::AnyElement {
    let columns = app.prefs.list_columns_shown();
    let current_sort = app.list_sort;
    let show_avatars = app.prefs.show_avatars;
    let show_recipients = app.list_view_shows_recipients();

    let mut bar = h_flex()
        .id("list-headings-bar")
        .w_full()
        .h(px(28.0))
        .flex_shrink_0()
        .px(px(14.0))
        .gap_2()
        .items_center()
        .border_b_1()
        .border_color(p.border)
        .bg(p.surface)
        .on_drag_move(
            cx.listener(|this, event: &DragMoveEvent<ColumnResizeDrag>, _, cx| {
                this.on_column_drag_move(event, cx);
            }),
        );

    if show_avatars {
        bar = bar.child(div().w(px(18.0)).flex_shrink_0());
    }
    // 未读圆点占位
    bar = bar.child(div().w(px(10.0)).flex_shrink_0());

    for col in columns {
        let width = app.column_width(col);
        let arrow = column_sort_arrow(col, current_sort);
        let is_sorted = arrow.is_some();
        let next_sort = column_sort_for(col, current_sort);
        let label_text = match col {
            ListColumn::Sender if show_recipients => "收件人",
            ListColumn::Star => "★",
            ListColumn::Replied => "↩",
            ListColumn::Attachment => "📎",
            ListColumn::Importance => "!",
            _ => col.label(),
        };
        let heading_text = format!("{label_text}{}", arrow.unwrap_or(""));

        let mut cell = h_flex()
            .id(("heading-col", col as usize))
            .h_full()
            .items_center()
            .relative()
            .cursor_pointer()
            .text_xs()
            .font_weight(if is_sorted {
                gpui_kit::FontWeight::SEMIBOLD
            } else {
                gpui_kit::FontWeight::MEDIUM
            })
            .text_color(if is_sorted { p.accent } else { p.text_muted })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.set_list_sort(next_sort, cx);
            }));

        if col == ListColumn::Subject {
            cell = cell.flex_1().min_w(px(80.0));
        } else {
            cell = cell.w(px(width)).flex_shrink_0();
        }

        let align_right = matches!(col, ListColumn::Date | ListColumn::Due);
        let label_el = div()
            .flex_1()
            .min_w_0()
            .truncate()
            .when(align_right, |d| d.text_right())
            .child(SharedString::from(heading_text));
        cell = cell.child(label_el);

        if column_resizable(col) {
            let drag_payload = ColumnResizeDrag {
                column: col,
                start_x: 0.0,
                start_width: width,
            };
            cell = cell.child(
                div()
                    .id(("col-resize", col as usize))
                    .w(px(6.0))
                    .h_full()
                    .flex_shrink_0()
                    .cursor_col_resize()
                    .hover(|s| s.bg(p.accent.opacity(0.6)))
                    .on_drag(drag_payload, move |drag, position, _, cx| {
                        let updated = ColumnResizeDrag {
                            column: drag.column,
                            start_x: position.x.as_f32(),
                            start_width: drag.start_width,
                        };
                        cx.new(|_| updated)
                    }),
            );
        }

        bar = bar.child(cell);
    }

    bar.into_any_element()
}

/// 列表行（圆角卡片 / 宽表单行表格）。
///
/// P3 状态参数：`list_focused` 为「列表持焦点」近似态（见 render 中的计算），
/// `kbd_cursor` 为键盘游标行（对齐 Linux .kbd-cursor）。
#[allow(clippy::too_many_arguments)]
fn message_row(
    app: &HylkiApp,
    msg: &MessageSummary,
    draft: Option<LocalDraft>,
    is_selected: bool,
    show_account_tag: bool,
    list_focused: bool,
    kbd_cursor: bool,
    is_wide: bool,
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
    // WP-01：选中行双态（对齐 Linux theme.rs install_scheme_css 高优先级动态
    // 样式与 styles.css row:selected 区块）：列表持焦点时选中行是 100% accent
    // 纯色底，行上文字 / 图标全部反白 accent_fg；焦点离开列表（阅读器 / 写信
    // 器 / 文本输入）时选中行退为中性灰 alpha(@window_fg_color, 0.14)，文字保
    // 持正文色（dimming, never vanishing，如同侧栏选中项）。
    let active_sel = is_selected && list_focused;
    let inactive_sel = is_selected && !list_focused;
    let is_thread_expanded = app.expanded_threads.contains(&msg_id);
    let msg_for_thread = msg.clone();
    let msg_for_thread_wide = msg.clone();
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

    let row_base = h_flex()
        .id(("msg", msg_id))
        .flex_shrink_0()
        .when(is_wide, |r| {
            r.h(px(WIDE_ROW_HEIGHT))
                .mx(px(6.0))
                .my(px(1.0))
                .px(px(8.0))
                .gap_2()
                .rounded(px(8.0))
        })
        .when(!is_wide, |r| {
            r.min_h(px(if compact {
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
            .rounded(px(12.0))
        })
        // 头像在整行垂直居中（对齐原版 avatar valign center）；文本列 h_full 撑满不受影响。
        .items_center()
        .cursor_pointer()
        .overflow_hidden()
        // WP-01 状态底色矩阵（对齐 Linux theme.rs install_scheme_css 与
        // styles.css .message-row 区块）：
        // - 悬停未选中：中性微灰（原版 alpha(currentColor, 0.05)）；
        // - 聚焦选中行（含悬停经过）：100% accent 纯色药丸（原版
        //   install_scheme_css：selected / :hover / :active 均为
        //   @accent_bg_color，非半透明 0.5）；
        // - 失焦选中行：中性灰 alpha(@window_fg_color, 0.14)，不清除（dimming,
        //   never vanishing），不是淡蓝。
        .hover(move |s| {
            if active_sel {
                s.bg(p.accent)
            } else if inactive_sel {
                s.bg(p.text.opacity(0.18))
            } else {
                s.bg(p.text.opacity(0.05))
            }
        })
        .when(is_selected, |row| {
            row.bg(if active_sel {
                p.accent
            } else {
                p.text.opacity(0.14)
            })
        })
        // P3：键盘游标行（原版 .kbd-cursor，列表持焦点时才画）：非选中行加
        // accent 0.1 底 + accent 内描边；聚焦选中行药丸上改用 accent_fg 0.8
        // 内描边（accent-on-accent 会消失）。gpui 无 inset box-shadow，用 2px
        // 圆角边框近似（行盒内绘制，不改布局）。
        .when(kbd_cursor && list_focused, |row| {
            let ring = if active_sel {
                p.accent_fg.opacity(0.80)
            } else {
                p.accent.opacity(0.45)
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
        });

    let row = if is_wide {
        let mut r = row_base
            .when(app.prefs.show_avatars, |el| {
                el.child(avatar::avatar(&display_name, &avatar_key, 18.0))
            })
            .child(
                div()
                    .w(px(10.0))
                    .h(px(10.0))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(unread, |dot| {
                        dot.child(
                            div()
                                .w(px(8.0))
                                .h(px(8.0))
                                .rounded_full()
                                .bg(if active_sel { p.accent_fg } else { p.accent }),
                        )
                    }),
            );
        let columns = app.prefs.list_columns_shown();
        for col in columns {
            let width = app.column_width(col);
            let cell = match col {
                ListColumn::Star => div()
                    .w(px(width))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(msg.flagged, |el| {
                        el.child(
                            Icon::new(IconName::StarFill)
                                .with_size(px(13.0))
                                .text_color(theme::star_gold()),
                        )
                    }),
                ListColumn::Replied => h_flex()
                    .w(px(width))
                    .flex_shrink_0()
                    .items_center()
                    .justify_center()
                    .gap_0p5()
                    .when(msg.answered, |el| {
                        el.child(
                            div()
                                .text_xs()
                                .text_color(if active_sel {
                                    p.accent_fg
                                } else {
                                    p.text_muted
                                })
                                .child("↩"),
                        )
                    })
                    .when(msg.forwarded, |el| {
                        el.child(
                            div()
                                .text_xs()
                                .text_color(if active_sel {
                                    p.accent_fg
                                } else {
                                    p.text_muted
                                })
                                .child("↪"),
                        )
                    }),
                ListColumn::Attachment => div()
                    .w(px(width))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(msg.has_attachment, |el| {
                        el.child(
                            Icon::new(IconName::Paperclip)
                                .with_size(px(13.0))
                                .text_color(if active_sel {
                                    p.accent_fg
                                } else {
                                    p.text_muted
                                }),
                        )
                    }),
                ListColumn::Importance => div()
                    .w(px(width))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(msg.importance != 0, |el| {
                        el.child(
                            div()
                                .text_xs()
                                .font_weight(gpui_kit::FontWeight::BOLD)
                                .text_color(if active_sel {
                                    p.accent_fg
                                } else if msg.importance > 0 {
                                    p.danger
                                } else {
                                    p.text_muted
                                })
                                .child(if msg.importance > 0 { "!" } else { "↓" }),
                        )
                    }),
                ListColumn::Sender => div()
                    .w(px(width))
                    .flex_shrink_0()
                    .truncate()
                    .text_sm()
                    .font_weight(if unread {
                        gpui_kit::FontWeight::BOLD
                    } else {
                        gpui_kit::FontWeight::NORMAL
                    })
                    .text_color(if active_sel { p.accent_fg } else { p.text })
                    .child(SharedString::from(first_line.clone())),
                ListColumn::Recipients => div()
                    .w(px(width))
                    .flex_shrink_0()
                    .truncate()
                    .text_xs()
                    .text_color(if active_sel {
                        p.accent_fg
                    } else {
                        p.text_muted
                    })
                    .child(SharedString::from(msg.recipients.clone())),
                ListColumn::Correspondents => {
                    let corr = if show_recipients {
                        msg.recipients.clone()
                    } else {
                        display_name.clone()
                    };
                    div()
                        .w(px(width))
                        .flex_shrink_0()
                        .truncate()
                        .text_sm()
                        .text_color(if active_sel { p.accent_fg } else { p.text })
                        .child(SharedString::from(corr))
                }
                ListColumn::Account => {
                    let lbl = app.account_label_for(account_id);
                    div()
                        .w(px(width))
                        .flex_shrink_0()
                        .truncate()
                        .text_xs()
                        .text_color(if active_sel {
                            p.accent_fg
                        } else {
                            p.text_muted
                        })
                        .child(SharedString::from(lbl))
                }
                ListColumn::Tags => h_flex()
                    .w(px(width))
                    .flex_shrink_0()
                    .gap_1()
                    .overflow_hidden()
                    .children(msg.tags.iter().take(2).map(|t| {
                        div()
                            .px_1p5()
                            .py_0p5()
                            .rounded_full()
                            .text_size(px(10.0))
                            .bg(if active_sel {
                                p.accent_fg.opacity(0.20)
                            } else {
                                p.accent_soft()
                            })
                            .text_color(if active_sel { p.accent_fg } else { p.accent })
                            .child(SharedString::from(t.clone()))
                    })),
                ListColumn::Date | ListColumn::Due => div()
                    .w(px(width))
                    .flex_shrink_0()
                    .truncate()
                    .text_right()
                    .text_xs()
                    .text_color(if active_sel {
                        p.accent_fg
                    } else {
                        p.text_muted
                    })
                    .child(SharedString::from(date_text.clone())),
                ListColumn::Subject => {
                    let msg_chip = msg_for_thread_wide.clone();
                    h_flex()
                        .flex_1()
                        .min_w(px(80.0))
                        .items_center()
                        .gap_1p5()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_sm()
                                .font_weight(if unread {
                                    gpui_kit::FontWeight::SEMIBOLD
                                } else {
                                    gpui_kit::FontWeight::NORMAL
                                })
                                .text_color(if active_sel {
                                    p.accent_fg
                                } else if unread {
                                    p.text
                                } else {
                                    p.text.opacity(0.85)
                                })
                                .child(SharedString::from(msg.subject.clone())),
                        )
                        .when(msg.thread_count > 1, |el| {
                            el.child(
                                div()
                                    .id(("thread-chip", msg_id))
                                    .flex_shrink_0()
                                    .px(px(7.0))
                                    .py(px(2.0))
                                    .rounded_full()
                                    .cursor_pointer()
                                    .bg(if active_sel {
                                        p.accent_fg
                                    } else {
                                        p.text.opacity(0.10)
                                    })
                                    .text_size(px(10.0))
                                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                    .text_color(if active_sel {
                                        p.accent
                                    } else if inactive_sel {
                                        p.text
                                    } else {
                                        p.text.opacity(0.70)
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.toggle_thread_expand(&msg_chip, cx);
                                    }))
                                    .child(SharedString::from(format!(
                                        "{} {}",
                                        msg.thread_count,
                                        if is_thread_expanded { "▾" } else { "▸" }
                                    ))),
                            )
                        })
                }
            };
            r = r.child(cell);
        }
        r
    } else {
        row_base
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
                                // 未读圆点 10x10（原版 .unread-dot）：聚焦选中行上反白为
                                // accent_fg（accent-on-accent 会隐形），其余保持 accent。
                                dot.child(
                                    div()
                                        .w(px(10.0))
                                        .h(px(10.0))
                                        .rounded_full()
                                        .bg(if active_sel { p.accent_fg } else { p.accent }),
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
                                    // WP-01：聚焦选中行（100% accent 纯色底）上反白为
                                    // accent_fg（原版 row:selected label 全量反色）。
                                    .text_color(if active_sel { p.accent_fg } else { p.text })
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
                                                // WP-01：聚焦选中行附件图标反白 accent_fg。
                                                .text_color(if active_sel {
                                                    p.accent_fg
                                                } else {
                                                    p.text_muted
                                                }),
                                        )
                                    })
                                    .child(
                                        div()
                                            .text_xs()
                                            // WP-01：聚焦选中行时间反白 accent_fg。
                                            .text_color(if active_sel {
                                                p.accent_fg
                                            } else {
                                                p.text_muted
                                            })
                                            .child(SharedString::from(date_text)),
                                    )
                                    // B-4 + WP-08 会话计数胶囊（点击在列表内联展开/收起子邮件树）。
                                    .when(msg.thread_count > 1, |icons| {
                                        let msg_chip = msg_for_thread.clone();
                                        icons.child(
                                            div()
                                                .id(("thread-chip", msg_id))
                                                .flex_shrink_0()
                                                .px(px(7.0))
                                                .py(px(2.0))
                                                .rounded_full()
                                                .cursor_pointer()
                                                .bg(if active_sel {
                                                    p.accent_fg
                                                } else {
                                                    p.text.opacity(0.10)
                                                })
                                                .text_size(px(10.0))
                                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                                .text_color(if active_sel {
                                                    p.accent
                                                } else if inactive_sel {
                                                    p.text
                                                } else {
                                                    p.text.opacity(0.70)
                                                })
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    cx.stop_propagation();
                                                    this.toggle_thread_expand(&msg_chip, cx);
                                                }))
                                                .child(SharedString::from(format!(
                                                    "{} {}",
                                                    msg.thread_count,
                                                    if is_thread_expanded { "▾" } else { "▸" }
                                                ))),
                                        )
                                    }),
                            ),
                    )
                    // 第 2 行：主题（未读加粗；已读 85% 透明度，复刻原版 .message-subject 层次；
                    // WP-01：聚焦选中行反白 accent_fg，覆盖已读的 85% 透明度）。
                    .child(
                        h_flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_sm()
                                    .truncate()
                                    .font_weight(if unread {
                                        gpui_kit::FontWeight::SEMIBOLD
                                    } else {
                                        gpui_kit::FontWeight::NORMAL
                                    })
                                    .text_color(if active_sel {
                                        p.accent_fg
                                    } else if unread {
                                        p.text
                                    } else {
                                        p.text.opacity(0.85)
                                    })
                                    .child(SharedString::from(msg.subject.clone())),
                            )
                            .when(app.prefs.focus_show_tags && !msg.tags.is_empty(), |el| {
                                el.children(msg.tags.iter().take(2).map(|t| {
                                    div()
                                        .flex_shrink_0()
                                        .px_1p5()
                                        .py_0p5()
                                        .rounded_full()
                                        .text_size(px(10.0))
                                        .bg(if active_sel {
                                            p.accent_fg.opacity(0.20)
                                        } else {
                                            p.accent_soft()
                                        })
                                        .text_color(if active_sel { p.accent_fg } else { p.accent })
                                        .child(SharedString::from(t.clone()))
                                }))
                            }),
                    )
                    // 第 3 行：摘要 + 多账号小胶囊标签；紧凑密度下隐藏。
                    .when(!compact, |row| {
                        let preview_lines = app.prefs.list_preview_lines.clamp(1, 3);
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
                                        .when(preview_lines == 1, |d| d.truncate())
                                        .when(preview_lines > 1, |d| {
                                            d.max_h(px(preview_lines as f32 * 16.0))
                                                .overflow_hidden()
                                        })
                                        // 摘要 50% 透明度（原版 .message-preview）；
                                        // WP-01：聚焦选中行反白 accent_fg。
                                        .text_color(if active_sel {
                                            p.accent_fg
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
                                            .text_color(if is_selected {
                                                p.text
                                            } else {
                                                acc_color
                                            })
                                            .child(SharedString::from(acc_label)),
                                    )
                                }),
                        )
                    }),
            )
    };

    let account_folders: Vec<(String, String)> = app
        .folders_by_account
        .get(&account_id)
        .map(|list| {
            list.iter()
                .map(|f| (f.name.clone(), f.path.clone()))
                .collect()
        })
        .unwrap_or_default();
    let all_account_folders: Vec<(u32, String, Vec<(String, String)>)> = app
        .accounts
        .iter()
        .filter(|a| a.enabled)
        .map(|a| {
            let folders = app
                .folders_by_account
                .get(&a.id)
                .map(|list| {
                    list.iter()
                        .map(|f| (f.name.clone(), f.path.clone()))
                        .collect()
                })
                .unwrap_or_default();
            (a.id, app.account_label_for(a.id), folders)
        })
        .collect();
    // B-2：导出进行中状态与回收站 / 垃圾邮件文件夹语义。
    let exporting_eml = app.pending_eml_export == Some(msg.id);
    let in_trash_or_junk = app.folder_is_trash_or_junk(account_id, msg.folder_id);
    let inbox_path = app.inbox_folder_path(account_id);
    row_context_menu(
        row,
        msg,
        draft,
        account_folders,
        all_account_folders,
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
    // 交互（点击行 / 键盘导航）且当前无文本输入持焦点、右栏未进入写信编辑器
    // 时视为聚焦，驱动选中行 accent 纯色 / 中性灰两档与键盘游标轮廓。对齐
    // Linux theme.rs：只有焦点进入写信编辑器或搜索输入框才移除 list-focused，
    // 点击右侧阅读器查看邮件时列表仍保持聚焦高亮（WP-01）。
    let list_focused = app.list_focused
        && !app.is_text_input_focused(window, cx)
        && app.right_mode != crate::app::RightPaneMode::Compose;

    let search_key = app.search_query.trim().to_lowercase();
    let matches_query = |subject: &str, body: &str| {
        search_key.is_empty()
            || subject.to_lowercase().contains(&search_key)
            || body.to_lowercase().contains(&search_key)
    };

    // 草稿视图行（随搜索过滤）。
    let is_wide = row_is_wide(app, drafts_view);
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

    // 常规邮件行：按时间排序且未关闭分组偏好时在列表行之间插入 5 档可折叠日期分组头
    // （今天 / 昨天 / 最近 7 天 / 最近 14 天 / 更早）。
    let show_date_groups =
        !drafts_view && app.prefs.list_group_by_date && current_sort.uses_date_groups();
    let now_ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let mut group_counts: HashMap<&'static str, usize> = HashMap::new();
    if show_date_groups {
        for msg in &merged_items {
            let g = date_group_label(msg.date_ts, now_ts);
            *group_counts.entry(g).or_insert(0) += 1;
        }
    }
    let mut filtered_rows: Vec<gpui_kit::AnyElement> = Vec::with_capacity(merged_items.len() + 8);
    if !drafts_view {
        let mut last_group: Option<&'static str> = None;
        for msg in &merged_items {
            let mut group_collapsed = false;
            if show_date_groups {
                let group = date_group_label(msg.date_ts, now_ts);
                group_collapsed = app.collapsed_day_groups.contains(group);
                if last_group != Some(group) {
                    last_group = Some(group);
                    let count = group_counts.get(group).copied().unwrap_or(0);
                    let entity_menu = cx.entity();
                    let group_header = h_flex()
                        .id(SharedString::from(format!("day-section-{group}")))
                        .flex_shrink_0()
                        .mx(px(6.0))
                        .px_2p5()
                        .py_1p5()
                        .rounded(px(8.0))
                        .items_center()
                        .justify_between()
                        .cursor_pointer()
                        .hover(|s| s.bg(p.text.opacity(0.05)))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_day_group(group, cx);
                        }))
                        .child(
                            h_flex()
                                .items_center()
                                .gap_1p5()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(muted_fg)
                                        .child(if group_collapsed { "▸" } else { "▾" }),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                        .text_color(muted_fg)
                                        .child(group),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted_fg)
                                .child(SharedString::from(format!("{count}"))),
                        )
                        .context_menu(move |menu, _, _| {
                            let e1 = entity_menu.clone();
                            let e2 = entity_menu.clone();
                            menu.item(PopupMenuItem::new("折叠所有日期分组").on_click(
                                move |_, _, cx| {
                                    e1.update(cx, |this, cx| this.collapse_all_day_groups(cx));
                                },
                            ))
                            .item(
                                PopupMenuItem::new("展开所有日期分组").on_click(move |_, _, cx| {
                                    e2.update(cx, |this, cx| this.expand_all_day_groups(cx));
                                }),
                            )
                        });
                    filtered_rows.push(group_header.into_any_element());
                }
            }
            if group_collapsed {
                continue;
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
                    is_wide,
                    &p,
                    cx,
                )
                .into_any_element(),
            );
            // WP-08：若当前会话已展开，在头行下方内联渲染子邮件树（.thread-child & .thread-node）。
            if msg.thread_count > 1 && app.expanded_threads.contains(&msg.id) {
                if let Some(children) = app.thread_children_cache.get(&msg.id) {
                    let n = children.len();
                    for (idx, child) in children.iter().enumerate() {
                        let is_last = idx + 1 == n;
                        let child_sel = selected_id == Some(child.id)
                            || app.selected_message_ids.contains(&child.id);
                        let child_kbd = app.kbd_cursor_message_id == Some(child.id);
                        let rail_color = p.text.opacity(0.25);
                        let child_wrapper = h_flex()
                            .id(("thread-child", child.id))
                            .w_full()
                            .items_stretch()
                            .pl(px(12.0))
                            .child(
                                // 左侧树形引导线 + 节点圆点（.thread-node）
                                div()
                                    .w(px(12.0))
                                    .flex_shrink_0()
                                    .relative()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .when(!is_last, |r| {
                                        r.child(
                                            div()
                                                .absolute()
                                                .left(px(5.0))
                                                .top_0()
                                                .bottom_0()
                                                .w(px(1.5))
                                                .bg(rail_color),
                                        )
                                    })
                                    .when(is_last, |r| {
                                        r.child(
                                            div()
                                                .absolute()
                                                .left(px(5.0))
                                                .top_0()
                                                .h(px(20.0))
                                                .w(px(1.5))
                                                .bg(rail_color),
                                        )
                                    })
                                    .child(
                                        div()
                                            .w(px(8.0))
                                            .h(px(8.0))
                                            .rounded_full()
                                            .bg(p.text_muted)
                                            .border_1()
                                            .border_color(p.surface),
                                    ),
                            )
                            .child(div().flex_1().min_w_0().child(message_row(
                                app,
                                child,
                                None,
                                child_sel,
                                show_account_tag,
                                list_focused,
                                child_kbd,
                                is_wide,
                                &p,
                                cx,
                            )));
                        filtered_rows.push(child_wrapper.into_any_element());
                    }
                }
            }
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
        let entity_for_popup = entity.clone();
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
            .dropdown_menu(move |menu, window, cx| {
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

                let e_tome = entity.clone();
                menu = menu.item(
                    PopupMenuItem::new("直接发给我")
                        .when(quick_filters.to_me, |item| {
                            item.icon(Icon::new(IconName::Check))
                        })
                        .on_click(move |_, _, cx| {
                            e_tome.update(cx, |this, cx| this.toggle_quick_filter_to_me(cx));
                        }),
                );

                let e_contacts = entity.clone();
                menu = menu.item(
                    PopupMenuItem::new("来自联系人")
                        .when(quick_filters.from_contacts, |item| {
                            item.icon(Icon::new(IconName::Check))
                        })
                        .on_click(move |_, _, cx| {
                            e_contacts.update(cx, |this, cx| {
                                this.toggle_quick_filter_from_contacts(cx);
                            });
                        }),
                );

                let e_imp = entity.clone();
                menu = menu.item(
                    PopupMenuItem::new("高优先级")
                        .when(quick_filters.high_importance, |item| {
                            item.icon(Icon::new(IconName::Check))
                        })
                        .on_click(move |_, _, cx| {
                            e_imp.update(cx, |this, cx| {
                                this.toggle_quick_filter_high_importance(cx);
                            });
                        }),
                );

                let e_tagged = entity.clone();
                menu = menu.item(
                    PopupMenuItem::new("含标签")
                        .when(quick_filters.tagged, |item| {
                            item.icon(Icon::new(IconName::Check))
                        })
                        .on_click(move |_, _, cx| {
                            e_tagged.update(cx, |this, cx| this.toggle_quick_filter_tagged(cx));
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

                let has_date_range =
                    quick_filters.date_from_ts.is_some() || quick_filters.date_to_ts.is_some();
                let e_range = entity.clone();
                menu = menu.submenu("日期范围", window, cx, move |sub, _, _| {
                    let today_start = hylki_mail_core::start_of_today_ts();
                    let e_7d = e_range.clone();
                    let e_30d = e_range.clone();
                    let e_90d = e_range.clone();
                    let e_clr = e_range.clone();
                    sub.item(PopupMenuItem::new("最近 7 天").on_click(move |_, _, cx| {
                        e_7d.update(cx, |this, cx| {
                            this.set_quick_filter_date_range(
                                Some(today_start - 6 * 86_400),
                                None,
                                cx,
                            );
                        });
                    }))
                    .item(PopupMenuItem::new("最近 30 天").on_click(move |_, _, cx| {
                        e_30d.update(cx, |this, cx| {
                            this.set_quick_filter_date_range(
                                Some(today_start - 29 * 86_400),
                                None,
                                cx,
                            );
                        });
                    }))
                    .item(PopupMenuItem::new("最近 90 天").on_click(move |_, _, cx| {
                        e_90d.update(cx, |this, cx| {
                            this.set_quick_filter_date_range(
                                Some(today_start - 89 * 86_400),
                                None,
                                cx,
                            );
                        });
                    }))
                    .separator()
                    .item(
                        PopupMenuItem::new("清除日期范围")
                            .disabled(!has_date_range)
                            .on_click(move |_, _, cx| {
                                e_clr.update(cx, |this, cx| {
                                    this.set_quick_filter_date_range(None, None, cx);
                                });
                            }),
                    )
                });

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
            .on_open_change(move |open, _, cx| {
                entity_for_popup.update(cx, |app, cx| {
                    app.reader_popup_open = *open;
                    cx.notify();
                });
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
        .when(app.reader_position == ReaderPosition::Right, |el| {
            el.border_r_1()
        })
        .when(app.reader_position == ReaderPosition::Below, |el| {
            el.border_b_1()
        })
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
        // WP-08：宽表列头栏（.list-headings-bar）。
        .when(
            !drafts_view && is_wide && app.prefs.list_show_headings,
            |el| el.child(render_headings_bar(app, &p, cx)),
        )
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
