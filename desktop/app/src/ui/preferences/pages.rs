//! 偏好设置各分页内容渲染（除「邮箱账号」由 `accounts.rs` 负责外的其余 13 个设置页）。

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::DropdownMenu;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{h_flex, v_flex, Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    div, px, AnyElement, Context, Entity, FontWeight, InteractiveElement, IntoElement,
    ParentElement, ScrollAnchor, SharedString, StatefulInteractiveElement, Styled, Window,
};
use hylki_mail_core::config::{
    self, ListColumn, NotificationButton, NotificationButtons, ToolbarItem, ToolbarSide,
    DEFAULT_ATTACHMENT_KEYWORDS, UI_TEXT_SCALE_STEPS,
};

use crate::app::HylkiApp;
use crate::theme::HylkiPalette;
use crate::ui::preferences::widgets::{
    arm_search_scroll_anchor, bool_switch, combo_button, combo_button_dynamic, info_row,
    search_scroll_anchor_consumed, setting_group, setting_row, take_search_scroll_pending,
    theme_gallery, CARDS_WIDE_MAX_WIDTH, CARD_MAX_WIDTH,
};
use crate::ui::preferences::{extra_pages, PrefTab, SettingsWindow};

const PAGE_SIZE_OPTIONS: &[(&str, usize)] = &[
    ("50 封 / 页", 50),
    ("100 封 / 页", 100),
    ("200 封 / 页", 200),
];

const SYNC_INTERVAL_OPTIONS: &[(&str, u32)] = &[
    ("仅手动同步", 0),
    ("每 1 分钟", 1),
    ("每 5 分钟", 5),
    ("每 15 分钟", 15),
    ("每 30 分钟", 30),
];

const DEFAULT_VIEW_OPTIONS: &[(&str, &str)] =
    &[("统一收件箱", "unified"), ("恢复上次位置", "last")];

const READER_ZOOM_OPTIONS: &[(&str, u32)] = &[
    ("50%", 50),
    ("60%", 60),
    ("70%", 70),
    ("80%", 80),
    ("90%", 90),
    ("100%（默认）", 100),
    ("110%", 110),
    ("125%", 125),
    ("150%", 150),
    ("175%", 175),
    ("200%", 200),
    ("250%", 250),
];

const FOLD_MODE_OPTIONS: &[(&str, &str)] = &[
    ("全部展开（不折叠）", "never"),
    ("仅折叠已读邮件", "read"),
    ("折叠较早邮件", "all_but_newest"),
];

const READ_MARK_OPTIONS: &[(&str, &str)] = &[
    ("打开即标记已读", "shown"),
    ("停留 2 秒后标为已读", "delay"),
    ("仅手动标记", "manual"),
];

const READER_POSITION_OPTIONS: &[(&str, &str)] = &[
    ("列表右侧（推荐）", "right"),
    ("列表下方", "below"),
    ("隐藏阅读器", "off"),
];

const MESSAGE_THEME_OPTIONS: &[(&str, &str)] = &[
    ("跟随界面主题", "system"),
    ("始终浅色信纸", "light"),
    ("始终深色信纸", "dark"),
];

const READER_DEFAULT_MODE_OPTIONS: &[(&str, &str)] = &[
    ("保持当前选择", "remember"),
    ("打开时默认简化视图", "on"),
    ("打开时默认完整 HTML", "off"),
];

const UNDO_SEND_OPTIONS: &[(&str, u32)] = &[
    ("关闭（立即发送）", 0),
    ("3 秒", 3),
    ("5 秒（推荐）", 5),
    ("10 秒", 10),
    ("20 秒", 20),
    ("30 秒", 30),
];

const SIGNATURE_POSITION_OPTIONS: &[(&str, bool)] =
    &[("在引用内容上方", true), ("在引用内容下方", false)];

const DATE_STYLE_OPTIONS: &[(&str, &str)] = &[
    ("跟随系统 (YYYY-MM-DD)", "system"),
    ("月 / 日 / 年 (Nov 22, 2026)", "month_first"),
    ("日 / 月 / 年 (22 Nov 2026)", "day_first"),
    ("年 - 月 - 日 (2026-11-22)", "year_first"),
];

const CLOCK_STYLE_OPTIONS: &[(&str, &str)] = &[
    ("跟随系统", "system"),
    ("12 小时制 (AM/PM)", "12h"),
    ("24 小时制", "24h"),
];

// ---- WP-05 新增选项表 ----

/// 专注模式消息列表布局（对齐 Linux `ListLayout`）。
const FOCUS_LAYOUT_OPTIONS: &[(&str, &str)] = &[
    ("卡片双行", "cards"),
    ("单行表格", "single_line"),
    ("自动（宽窗单行）", "automatic"),
];

/// 专注模式摘要行数（对齐 Linux：关闭 / 1-3 行）。
const FOCUS_PREVIEW_OPTIONS: &[(&str, u32)] = &[("关闭", 0), ("1 行", 1), ("2 行", 2), ("3 行", 3)];

/// 消息列表布局（宽表完整渲染随 WP-08 落地）。
const LIST_LAYOUT_OPTIONS: &[(&str, &str)] = &[("经典双行", "cards"), ("宽表列视图", "wide")];

/// 消息列表摘要行数。
const LIST_PREVIEW_OPTIONS: &[(&str, u32)] = &[("1 行", 1), ("2 行", 2), ("3 行", 3)];

/// 阅读器工具栏按钮所在分区。
const TOOLBAR_ZONE_OPTIONS: &[(&str, Option<ToolbarSide>)] = &[
    ("左组（固定显示）", Some(ToolbarSide::Left)),
    ("右组（窄时折叠进 ⋯）", Some(ToolbarSide::Right)),
    ("不显示", None),
];

/// 已读回执策略（对齐 Linux `ReceiptPolicy`）。
const READ_RECEIPT_OPTIONS: &[(&str, &str)] = &[
    ("每次询问", "ask"),
    ("总是发送", "always"),
    ("从不发送", "never"),
];

/// 已下载邮件保留时长（对齐 Linux `KEEP_DOWNLOADS_DAYS = [0, 365, 90, 30]`）。
const KEEP_DOWNLOADS_OPTIONS: &[(&str, u32)] =
    &[("永久保留", 0), ("1 年", 365), ("90 天", 90), ("30 天", 30)];

/// 邮件内链接打开方式。
const LINK_OPEN_OPTIONS: &[(&str, &str)] = &[
    ("系统默认浏览器", "system"),
    ("Microsoft Edge", "edge"),
    ("Google Chrome", "chrome"),
    ("Mozilla Firefox", "firefox"),
];

/// 内置提示音（对齐 Linux `BUILTIN_SOUNDS`）。
const NOTIFICATION_SOUND_OPTIONS: &[(&str, &str)] = &[
    ("点击 (Click)", "click"),
    ("低鸣 (Hum)", "hum"),
    ("弦音 (String)", "string"),
    ("摇摆 (Swing)", "swing"),
];

/// 通知操作按钮的行 / 控件 id（与 `NotificationButton::ALL` 顺序一致）。
const NOTIFY_BUTTON_IDS: [(&str, &str); 6] = [
    ("mark_read", "pref-notify-btn-mark-read"),
    ("archive", "pref-notify-btn-archive"),
    ("delete", "pref-notify-btn-delete"),
    ("reply", "pref-notify-btn-reply"),
    ("forward", "pref-notify-btn-forward"),
    ("spam", "pref-notify-btn-spam"),
];

fn lookup_label<'a, T: PartialEq>(
    options: &[(&'a str, T)],
    current: &T,
    fallback: &'a str,
) -> &'a str {
    options
        .iter()
        .find(|(_, v)| v == current)
        .map(|(l, _)| *l)
        .unwrap_or(fallback)
}

fn is_hit(win: &SettingsWindow, row_id: &str) -> bool {
    win.highlighted_row_id == Some(row_id)
}

/// WP-05：布尔偏好开关行的通用构造（读取当前值 + 写回偏好并落盘）。
/// 行元素 id 为 `{switch_id}-row`，开关控件 id 即 `switch_id`（与搜索索引的
/// `row_id` 对应，命中时由 `is_hit` 高亮）。
#[allow(clippy::too_many_arguments)]
fn prefs_switch_row(
    app_entity: &Entity<HylkiApp>,
    switch_id: &'static str,
    label: impl Into<SharedString>,
    description: Option<&str>,
    value: bool,
    disabled: bool,
    highlighted: bool,
    set: impl Fn(&mut hylki_mail_core::config::AppPreferences, bool) + 'static,
    p: &HylkiPalette,
) -> AnyElement {
    let app = app_entity.clone();
    setting_row(
        SharedString::from(format!("{switch_id}-row")),
        label,
        description,
        bool_switch(switch_id, value, disabled, move |val, _, cx| {
            app.update(cx, |app, cx| {
                app.mutate_preferences(cx, |p| set(p, val));
            });
        }),
        disabled,
        highlighted,
        p,
    )
}

/// WP-05：把编辑后的工具栏左右两组写回偏好（key 顺序表）。
fn write_toolbar_keys(
    prefs: &mut hylki_mail_core::config::AppPreferences,
    layout: &hylki_mail_core::config::ReaderToolbarLayout,
) {
    prefs.reader_toolbar_left = layout.left.iter().map(|i| i.key().to_string()).collect();
    prefs.reader_toolbar_right = layout.right.iter().map(|i| i.key().to_string()).collect();
}

/// WP-05：把编辑后的显示列写回偏好（key 顺序表）。
fn write_column_keys(prefs: &mut hylki_mail_core::config::AppPreferences, columns: &[ListColumn]) {
    prefs.list_columns = ListColumn::to_keys(columns);
}

/// 渲染 `MailAccounts` 之外的其余 13 个偏好设置页。
pub(crate) fn render_tab(
    win: &SettingsWindow,
    app_entity: Entity<HylkiApp>,
    p: &HylkiPalette,
    window: &mut Window,
    cx: &mut Context<SettingsWindow>,
) -> AnyElement {
    // 搜索跳转来的本帧渲染：把滚动锚点交给高亮行（`setting_row` 领取挂载），
    // 行位置在渲染帧中被记录，随后 `scroll_to` 在下一帧把该行滚进视口。
    let mut scroll_anchor = None;
    if take_search_scroll_pending() {
        let anchor = ScrollAnchor::for_handle(win.content_scroll.clone());
        arm_search_scroll_anchor(Some(anchor.clone()));
        scroll_anchor = Some(anchor);
    }

    let body = match win.active_tab {
        PrefTab::MailAccounts => div().into_any_element(),
        PrefTab::Tags => extra_pages::render_tags(win, app_entity, p, cx),
        PrefTab::Filters => extra_pages::render_filters(win, app_entity, p, cx),
        PrefTab::SmartFolders => extra_pages::render_smart_folders(win, app_entity, p, cx),
        PrefTab::Senders => render_senders(win, app_entity, p, cx),
        PrefTab::OpenPgp => extra_pages::render_openpgp(win, app_entity, p, cx),
        PrefTab::CloudStorage => extra_pages::render_cloud_storage(win, app_entity, p, cx),
        PrefTab::Translation => extra_pages::render_translation(win, app_entity, p, cx),
        PrefTab::ContactsSources => extra_pages::render_contacts_sources(win, app_entity, p, cx),
        PrefTab::General => render_general(win, app_entity, p, cx),
        PrefTab::Appearance => render_appearance(win, app_entity, p, cx),
        PrefTab::FocusMode => render_focus_mode(win, app_entity, p, cx),
        PrefTab::Sidebar => render_sidebar(win, app_entity, p, cx),
        PrefTab::MessageList => render_message_list(win, app_entity, p, cx),
        PrefTab::Conversations => render_conversations(win, app_entity, p, cx),
        PrefTab::Reading => render_reading(win, app_entity, p, cx),
        PrefTab::Composing => render_composing(win, app_entity, p, cx),
        PrefTab::Privacy => render_privacy(win, app_entity, p, cx),
        PrefTab::Mcp => extra_pages::render_mcp(win, app_entity, p, cx),
        PrefTab::DateTime => render_date_time(win, app_entity, p, cx),
        PrefTab::System => render_system(win, app_entity, p, cx),
        PrefTab::Backup => render_backup(win, app_entity, p, window, cx),
    };

    if let Some(anchor) = scroll_anchor {
        if search_scroll_anchor_consumed() {
            anchor.scroll_to(window, cx);
        } else {
            // 高亮行未出现（如账号编辑页），丢弃锚点避免悬挂到下一帧。
            arm_search_scroll_anchor(None);
        }
    }

    v_flex()
        .id(SharedString::from(format!(
            "pref-scroll-{:?}",
            win.active_tab
        )))
        .track_scroll(&win.content_scroll)
        .flex_1()
        .min_h_0()
        .overflow_y_scrollbar()
        .px_6()
        .py_5()
        .items_center()
        .child(v_flex().w_full().items_center().gap_5().child(body))
        .into_any_element()
}

fn render_senders(
    win: &SettingsWindow,
    app_entity: Entity<HylkiApp>,
    p: &HylkiPalette,
    cx: &mut Context<SettingsWindow>,
) -> AnyElement {
    let prefs = &app_entity.read(cx).prefs;
    let filter = win.senders_filter.trim().to_ascii_lowercase();
    let total_allowed = prefs.allowed_remote_senders.len();
    let total_blocked = prefs.blocked_senders.len();
    let mut sorted_senders = prefs.allowed_remote_senders.clone();
    sorted_senders.sort();
    if !filter.is_empty() {
        sorted_senders.retain(|s| s.to_ascii_lowercase().contains(&filter));
    }
    let mut sorted_blocked = prefs.blocked_senders.clone();
    sorted_blocked.sort();
    if !filter.is_empty() {
        sorted_blocked.retain(|s| s.to_ascii_lowercase().contains(&filter));
    }
    let block_images = prefs.block_remote_images;

    let app_for_add = app_entity.clone();
    let mut rows: Vec<AnyElement> = Vec::new();
    rows.push(
        h_flex()
            .id("senders-add-bar")
            .px_4()
            .py_3()
            .gap_2()
            .items_center()
            .when(is_hit(win, "senders-allowlist"), |el| {
                el.bg(p.accent.opacity(0.18))
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&win.sender_input)),
            )
            .child(
                Button::new("sender-add-btn")
                    .primary()
                    .compact()
                    .label("添加到白名单")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.add_allowed_sender(&app_for_add, window, cx);
                    })),
            )
            .into_any_element(),
    );

    if sorted_senders.is_empty() {
        let empty_msg = if total_allowed == 0 {
            "白名单暂无发件人；阅读邮件点击「始终允许来自此发件人」也会自动收录到此处。"
        } else {
            "没有匹配当前搜索词的白名单发件人。"
        };
        rows.push(
            h_flex()
                .px_4()
                .py_6()
                .justify_center()
                .items_center()
                .gap_2()
                .child(
                    Icon::new(IconName::CircleUser)
                        .small()
                        .text_color(p.text_muted),
                )
                .child(div().text_xs().text_color(p.text_muted).child(empty_msg))
                .into_any_element(),
        );
    } else {
        for (idx, email) in sorted_senders.into_iter().enumerate() {
            let email_for_remove = email.clone();
            let app_for_remove = app_entity.clone();
            let remove_btn = Button::new(("sender-remove", idx))
                .ghost()
                .compact()
                .icon(IconName::Trash)
                .label("移除")
                .on_click(move |_, _, cx| {
                    app_for_remove.update(cx, |app, cx| {
                        app.mutate_preferences(cx, |prefs| {
                            prefs.remove_remote_sender(&email_for_remove);
                        });
                    });
                })
                .into_any_element();
            rows.push(setting_row(
                ("sender-row", idx),
                email,
                None,
                remove_btn,
                false,
                false,
                p,
            ));
        }
    }

    let app_for_block_add = app_entity.clone();
    let mut blocked_rows: Vec<AnyElement> = Vec::new();
    blocked_rows.push(
        h_flex()
            .id("senders-blocked-add-bar")
            .px_4()
            .py_3()
            .gap_2()
            .items_center()
            .when(is_hit(win, "pref-blocked-senders"), |el| {
                el.bg(p.accent.opacity(0.18))
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&win.blocked_input)),
            )
            .child(
                Button::new("blocked-add-btn")
                    .primary()
                    .compact()
                    .label("添加到黑名单")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.add_blocked_sender(&app_for_block_add, window, cx);
                    })),
            )
            .into_any_element(),
    );

    if sorted_blocked.is_empty() {
        let empty_msg = if total_blocked == 0 {
            "黑名单暂无发件人或域名；添加后将拦截来自该发件人或域名的邮件。"
        } else {
            "没有匹配当前搜索词的已屏蔽发件人。"
        };
        blocked_rows.push(
            h_flex()
                .px_4()
                .py_6()
                .justify_center()
                .items_center()
                .gap_2()
                .child(Icon::new(IconName::Ban).small().text_color(p.text_muted))
                .child(div().text_xs().text_color(p.text_muted).child(empty_msg))
                .into_any_element(),
        );
    } else {
        for (idx, entry) in sorted_blocked.into_iter().enumerate() {
            let entry_for_remove = entry.clone();
            let app_for_remove = app_entity.clone();
            let remove_btn = Button::new(("blocked-remove", idx))
                .ghost()
                .compact()
                .icon(IconName::Trash)
                .label("移除")
                .on_click(move |_, _, cx| {
                    app_for_remove.update(cx, |app, cx| {
                        app.mutate_preferences(cx, |prefs| {
                            prefs.unblock_sender(&entry_for_remove);
                        });
                    });
                })
                .into_any_element();
            blocked_rows.push(setting_row(
                ("blocked-row", idx),
                entry,
                None,
                remove_btn,
                false,
                false,
                p,
            ));
        }
    }

    let mut stack = v_flex().w_full().max_w(px(CARDS_WIDE_MAX_WIDTH)).gap_3();

    stack = stack.child(
        h_flex()
            .id("senders-filter-bar")
            .w_full()
            .gap_2()
            .items_center()
            .child(Icon::new(IconName::Search).small().text_color(p.text_muted))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&win.sender_search_input)),
            )
            .when(!win.senders_filter.is_empty(), |el| {
                el.child(
                    Button::new("senders-filter-clear")
                        .ghost()
                        .compact()
                        .label("清除筛选")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.senders_filter.clear();
                            this.sender_search_input
                                .update(cx, |s, cx| s.set_value("", window, cx));
                            cx.notify();
                        })),
                )
            }),
    );

    if !block_images {
        let app_for_jump = cx.entity().clone();
        stack = stack.child(
            h_flex()
                .px_4()
                .py_2p5()
                .rounded(px(10.0))
                .bg(p.accent.opacity(0.10))
                .border_1()
                .border_color(p.accent.opacity(0.35))
                .justify_between()
                .items_center()
                .gap_3()
                .child(div().text_xs().text_color(p.text).child(
                    "提示：当前已关闭「默认屏蔽远程图像」，所有发件人的外链图片均会直接加载。",
                ))
                .child(
                    Button::new("senders-goto-privacy")
                        .outline()
                        .compact()
                        .label("前往隐私设置")
                        .on_click(move |_, window, cx| {
                            app_for_jump.update(cx, |win, cx| {
                                win.select_tab(PrefTab::Privacy, window, cx);
                            });
                        }),
                ),
        );
    }

    if let Some((ok, ref msg)) = win.sender_feedback {
        stack = stack.child(
            div()
                .px_3()
                .py_2()
                .rounded(px(8.0))
                .bg(if ok {
                    p.accent.opacity(0.12)
                } else {
                    p.danger.opacity(0.12)
                })
                .text_xs()
                .text_color(if ok { p.accent } else { p.danger })
                .child(msg.clone()),
        );
    }

    stack
        .child(setting_group(
            format!("远程图像发件人白名单（{}）", total_allowed),
            Some("列出的邮箱地址将在阅读 HTML 邮件时自动放行外链图片"),
            CARDS_WIDE_MAX_WIDTH,
            rows,
            p,
        ))
        .child(setting_group(
            format!("已屏蔽发件人黑名单（{}）", total_blocked),
            Some("支持填写完整邮箱（user@example.com）或域名（example.com）"),
            CARDS_WIDE_MAX_WIDTH,
            blocked_rows,
            p,
        ))
        .into_any_element()
}

fn render_general(
    win: &SettingsWindow,
    app_entity: Entity<HylkiApp>,
    p: &HylkiPalette,
    cx: &mut Context<SettingsWindow>,
) -> AnyElement {
    let prefs = app_entity.read(cx).prefs.clone();
    let app1 = app_entity.clone();
    let app2 = app_entity.clone();
    let app3 = app_entity.clone();
    let app4 = app_entity.clone();
    let app5 = app_entity.clone();
    let a_sound = app_entity.clone();

    v_flex()
        .w_full()
        .max_w(px(CARD_MAX_WIDTH))
        .gap_5()
        .child(setting_group(
            "分页加载",
            None,
            CARD_MAX_WIDTH,
            vec![setting_row(
                "pref-page-size",
                "单页加载邮件数",
                None,
                combo_button(
                    "cb-page-size",
                    lookup_label(PAGE_SIZE_OPTIONS, &prefs.page_size, "100 封 / 页"),
                    PAGE_SIZE_OPTIONS,
                    prefs.page_size,
                    false,
                    move |val: usize, _, cx| {
                        app1.update(cx, |app, cx| {
                            app.mutate_preferences(cx, |p| p.page_size = val);
                        });
                    },
                ),
                false,
                is_hit(win, "pref-page-size"),
                p,
            )],
            p,
        ))
        .child(setting_group(
            "同步与通知",
            None,
            CARD_MAX_WIDTH,
            vec![
                setting_row(
                    "pref-auto-sync",
                    "自动后台同步间隔",
                    Some("按设定分钟轮询所有已配置邮箱的新邮件"),
                    combo_button(
                        "cb-auto-sync",
                        lookup_label(SYNC_INTERVAL_OPTIONS, &prefs.auto_sync_minutes, "每 5 分钟"),
                        SYNC_INTERVAL_OPTIONS,
                        prefs.auto_sync_minutes,
                        false,
                        move |val: u32, _, cx| {
                            app2.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.auto_sync_minutes = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-auto-sync"),
                    p,
                ),
                setting_row(
                    "pref-push-enabled",
                    "启用实时推送（IMAP IDLE / 增量脉冲）",
                    Some("在后台维持实时监听连接以便秒级感知新邮件到达"),
                    bool_switch(
                        "sw-push-enabled",
                        prefs.push_enabled,
                        false,
                        move |val, _, cx| {
                            app3.update(cx, |app, cx| {
                                app.set_push_enabled(val, cx);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-push-enabled"),
                    p,
                ),
                prefs_switch_row(
                    &app_entity,
                    "pref-metered-save-data",
                    "按流量计费连接省流",
                    Some("使用按流量计费的网络时仅收取新邮件，不预取其余内容"),
                    prefs.metered_save_data,
                    false,
                    is_hit(win, "pref-metered-save-data"),
                    |p, v| p.metered_save_data = v,
                    p,
                ),
                setting_row(
                    "pref-desktop-notifications",
                    "新邮件桌面通知",
                    Some("同步发现新增未读邮件时通过 Windows 托盘弹出提示"),
                    bool_switch(
                        "sw-desktop-notif",
                        prefs.desktop_notifications,
                        false,
                        move |val, _, cx| {
                            app4.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.desktop_notifications = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-desktop-notifications"),
                    p,
                ),
                setting_row(
                    "pref-notification-content",
                    "在桌面通知中显示发件人与主题",
                    Some("关闭后通知仅提示「收到新邮件 / 封数」，保护锁屏及旁观隐私"),
                    bool_switch(
                        "sw-notif-content",
                        prefs.notification_show_content,
                        !prefs.desktop_notifications,
                        move |val, _, cx| {
                            app5.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.notification_show_content = val);
                            });
                        },
                    ),
                    !prefs.desktop_notifications,
                    is_hit(win, "pref-notification-content"),
                    p,
                ),
                prefs_switch_row(
                    &app_entity,
                    "pref-notification-sound",
                    "新邮件提示音",
                    Some("收到新邮件时随通知播放提示音（音频后端暂缺，先保存选择）"),
                    prefs.notification_sound_enabled,
                    !prefs.desktop_notifications,
                    is_hit(win, "pref-notification-sound"),
                    |p, v| p.notification_sound_enabled = v,
                    p,
                ),
                setting_row(
                    "pref-notification-sound-choice",
                    "提示音选择",
                    Some("内置 4 种提示音（对齐 Linux 版）"),
                    h_flex()
                        .gap_1p5()
                        .items_center()
                        .child(combo_button(
                            "cb-notification-sound",
                            lookup_label(
                                NOTIFICATION_SOUND_OPTIONS,
                                &prefs.notification_sound.as_str(),
                                NOTIFICATION_SOUND_OPTIONS[0].0,
                            ),
                            NOTIFICATION_SOUND_OPTIONS,
                            match prefs.notification_sound.as_str() {
                                "hum" => "hum",
                                "string" => "string",
                                "swing" => "swing",
                                _ => "click",
                            },
                            !prefs.desktop_notifications || !prefs.notification_sound_enabled,
                            move |val: &str, _, cx| {
                                a_sound.update(cx, |app, cx| {
                                    app.mutate_preferences(cx, |p| {
                                        p.notification_sound = val.to_string();
                                    });
                                });
                            },
                        ))
                        .child(
                            Button::new("btn-sound-preview")
                                .outline()
                                .compact()
                                .icon(IconName::Play)
                                .tooltip("试听（音频后端暂未接入）")
                                .disabled(true),
                        )
                        .into_any_element(),
                    !prefs.desktop_notifications,
                    is_hit(win, "pref-notification-sound-choice"),
                    p,
                ),
                prefs_switch_row(
                    &app_entity,
                    "pref-send-sound",
                    "发送邮件提示音",
                    Some("邮件发出时播放提示音（音频后端暂缺，先保存选择）"),
                    prefs.send_sound_enabled,
                    false,
                    is_hit(win, "pref-send-sound"),
                    |p, v| p.send_sound_enabled = v,
                    p,
                ),
            ],
            p,
        ))
        // WP-05：通知操作按钮（对齐 Linux general.rs "Notification Buttons"
        // 组：6 选 3，超出置灰）。
        .child(setting_group(
            "通知操作按钮",
            Some("单封邮件的通知最多显示 3 个操作按钮，超过后其余按钮置灰（通知按钮的实际渲染随后续通知工作落地）"),
            CARD_MAX_WIDTH,
            {
                let buttons = NotificationButtons::from_list(&prefs.notification_buttons);
                let mut rows: Vec<AnyElement> = Vec::new();
                for (idx, btn) in NotificationButton::ALL.iter().copied().enumerate() {
                    let (key, row_id) = NOTIFY_BUTTON_IDS[idx];
                    debug_assert_eq!(key, btn.key());
                    rows.push(prefs_switch_row(
                        &app_entity,
                        row_id,
                        btn.label(),
                        None,
                        buttons.get(btn),
                        !prefs.desktop_notifications || !buttons.row_sensitive(btn),
                        is_hit(win, row_id),
                        move |p: &mut hylki_mail_core::config::AppPreferences, v: bool| {
                            let mut b = NotificationButtons::from_list(&p.notification_buttons);
                            b.set(btn, v);
                            p.notification_buttons = b.to_list();
                        },
                        p,
                    ));
                }
                rows
            },
            p,
        ))
        .into_any_element()
}

fn render_appearance(
    win: &SettingsWindow,
    app_entity: Entity<HylkiApp>,
    p: &HylkiPalette,
    cx: &mut Context<SettingsWindow>,
) -> AnyElement {
    let prefs = app_entity.read(cx).prefs.clone();
    let app_dark = app_entity.clone();
    let app_scale = app_entity.clone();
    let app_open_acc = app_entity.clone();
    let app_fold = app_entity.clone();

    let dark_mode_buttons = h_flex().gap_1p5().children(
        [
            ("跟随系统", None),
            ("浅色", Some(false)),
            ("深色", Some(true)),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (label, mode))| {
            let active = prefs.dark_mode == mode;
            let app_c = app_dark.clone();
            let mut btn = Button::new(("dm-btn", i)).compact().label(label);
            if active {
                btn = btn.primary();
            } else {
                btn = btn.outline();
            }
            btn.on_click(move |_, _, cx| {
                app_c.update(cx, |app, cx| {
                    app.mutate_preferences(cx, |p| p.dark_mode = mode);
                });
            })
        }),
    );

    let scale_label = format!("{}%", prefs.ui_text_scale_percent);
    let scale_btn = Button::new("cb-ui-text-scale")
        .outline()
        .compact()
        .label(scale_label)
        .icon(IconName::ChevronDown)
        .dropdown_menu(move |mut menu, _, _| {
            for &step in UI_TEXT_SCALE_STEPS {
                let checked = prefs.ui_text_scale_percent == step;
                let app_c = app_scale.clone();
                let label = if step == 100 {
                    "100%（默认）".to_string()
                } else {
                    format!("{step}%")
                };
                menu = menu.item(
                    gpui_kit::component::menu::PopupMenuItem::new(label)
                        .checked(checked)
                        .on_click(move |_, _, cx| {
                            app_c.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| {
                                    p.ui_text_scale_percent = step;
                                });
                            });
                        }),
                );
            }
            menu
        })
        .into_any_element();

    v_flex()
        .w_full()
        .max_w(px(CARDS_WIDE_MAX_WIDTH))
        .gap_5()
        .items_center()
        .child(setting_group(
            "明暗外观",
            None,
            CARD_MAX_WIDTH,
            vec![setting_row(
                "pref-dark-mode",
                "外观模式",
                Some("控制主窗口与设置窗口使用浅色、深色或跟随 Windows 系统设置"),
                dark_mode_buttons.into_any_element(),
                false,
                is_hit(win, "pref-dark-mode"),
                p,
            )],
            p,
        ))
        .child(theme_gallery(
            &prefs.theme_id,
            is_hit(win, "pref-theme-gallery"),
            app_entity.clone(),
            p,
            cx,
        ))
        // WP-05：应用图标选择器（对齐 Linux icon_picker；当前资源库仅内置
        // 默认图标，其余选择回退默认，不伪造图标变体）。
        .child(setting_group(
            "应用图标",
            Some("窗口与任务栏使用的应用图标。当前资源库仅内置默认图标，后续图标包发布后可在此选择"),
            CARD_MAX_WIDTH,
            vec![
                h_flex()
                    .px_4()
                    .py_3()
                    .gap_3()
                    .child(
                        v_flex()
                            .id("app-icon-default")
                            .w(px(120.0))
                            .p_2()
                            .gap_1p5()
                            .rounded(px(10.0))
                            .bg(p.accent.opacity(0.10))
                            .border_2()
                            .border_color(p.accent)
                            .cursor_pointer()
                            .child(
                                div()
                                    .h(px(44.0))
                                    .rounded(px(6.0))
                                    .bg(p.accent)
                                    .items_center()
                                    .justify_center()
                                    .child(
                                        Icon::new(IconName::MailOpen)
                                            .small()
                                            .text_color(p.accent_fg),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .justify_center()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(p.accent)
                                            .child("默认图标"),
                                    )
                                    .child(
                                        Icon::new(IconName::Check).xsmall().text_color(p.accent),
                                    ),
                            ),
                    )
                    .into_any_element(),
            ],
            p,
        ))
        .child(setting_group(
            "字体缩放与窗口 chrome",
            None,
            CARD_MAX_WIDTH,
            vec![
                setting_row(
                    "pref-ui-text-scale",
                    "界面字体缩放",
                    Some("调整全应用控件字号比例（不影响操作系统 DPI）"),
                    scale_btn,
                    false,
                    is_hit(win, "pref-ui-text-scale"),
                    p,
                ),
                setting_row(
                    "pref-settings-open-accounts",
                    "打开偏好设置时优先显示「邮箱账号」",
                    None,
                    bool_switch(
                        "sw-open-acc",
                        prefs.settings_open_accounts,
                        false,
                        move |val, _, cx| {
                            app_open_acc.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.settings_open_accounts = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-settings-open-accounts"),
                    p,
                ),
            ],
            p,
        ))
        .child(setting_group(
            "阅读器工具栏",
            Some("每个按钮可选择左组（固定显示）/ 右组（窄时折叠进 ⋯）/ 不显示，并可用箭头在组内排序（完整渲染接线随后续工具栏工作落地）"),
            CARD_MAX_WIDTH,
            {
                let layout = prefs.reader_toolbar();
                let mut rows: Vec<AnyElement> = Vec::new();
                rows.push(setting_row(
                    "pref-reader-fold-right",
                    "始终折叠右侧工具栏辅助按钮",
                    Some("将已读切换、原文预览、导出 EML、一键退订等收敛至更多菜单"),
                    bool_switch(
                        "sw-rd-fold",
                        prefs.reader_toolbar_fold_right,
                        false,
                        move |val, _, cx| {
                            app_fold.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.reader_toolbar_fold_right = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-reader-fold-right"),
                    p,
                ));
                for (idx, item) in ToolbarItem::ALL.iter().copied().enumerate() {
                    let a_zone = app_entity.clone();
                    let a_up = app_entity.clone();
                    let a_down = app_entity.clone();
                    let side = layout.side(item);
                    let in_zone_len = match side {
                        Some(ToolbarSide::Left) => layout.left.len(),
                        Some(ToolbarSide::Right) => layout.right.len(),
                        None => 0,
                    };
                    let controls = h_flex()
                        .gap_1()
                        .child(
                            Button::new(("tb-up", idx))
                                .ghost()
                                .compact()
                                .disabled(side.is_none() || idx_at(&layout, item) == 0)
                                .icon(IconName::ArrowUp)
                                .tooltip("上移")
                                .on_click(move |_, _, cx| {
                                    a_up.update(cx, |app, cx| {
                                        app.mutate_preferences(cx, |p| {
                                            let mut t = p.reader_toolbar();
                                            t.move_within(item, -1);
                                            write_toolbar_keys(p, &t);
                                        });
                                    });
                                }),
                        )
                        .child(
                            Button::new(("tb-down", idx))
                                .ghost()
                                .compact()
                                .disabled(
                                    side.is_none()
                                        || idx_at(&layout, item) + 1 >= in_zone_len,
                                )
                                .icon(IconName::ArrowDown)
                                .tooltip("下移")
                                .on_click(move |_, _, cx| {
                                    a_down.update(cx, |app, cx| {
                                        app.mutate_preferences(cx, |p| {
                                            let mut t = p.reader_toolbar();
                                            t.move_within(item, 1);
                                            write_toolbar_keys(p, &t);
                                        });
                                    });
                                }),
                        )
                        .child(combo_button(
                            ("tb-zone", idx),
                            lookup_label(
                                TOOLBAR_ZONE_OPTIONS,
                                &layout.side(item),
                                "不显示",
                            ),
                            TOOLBAR_ZONE_OPTIONS,
                            layout.side(item),
                            false,
                            move |zone: Option<ToolbarSide>, _, cx| {
                                a_zone.update(cx, |app, cx| {
                                    app.mutate_preferences(cx, |p| {
                                        let mut t = p.reader_toolbar();
                                        t.place(item, zone, usize::MAX);
                                        write_toolbar_keys(p, &t);
                                    });
                                });
                            },
                        ));
                    rows.push(setting_row(
                        SharedString::from(format!("pref-toolbar-{}", item.key())),
                        item.label(),
                        None,
                        controls.into_any_element(),
                        false,
                        false,
                        p,
                    ));
                }
                let a_restore = app_entity.clone();
                rows.push(
                    h_flex()
                        .px_4()
                        .py_2p5()
                        .justify_end()
                        .child(
                            Button::new("btn-toolbar-restore")
                                .outline()
                                .compact()
                                .label("恢复默认布局")
                                .on_click(move |_, _, cx| {
                                    a_restore.update(cx, |app, cx| {
                                        app.mutate_preferences(cx, |p| {
                                            let t = hylki_mail_core::config::ReaderToolbarLayout::default();
                                            write_toolbar_keys(p, &t);
                                        });
                                    });
                                }),
                        )
                        .into_any_element(),
                );
                rows
            },
            p,
        ))
        .into_any_element()
}

/// WP-05：按钮在某组内的位置（隐藏时返回 0）。
fn idx_at(layout: &hylki_mail_core::config::ReaderToolbarLayout, item: ToolbarItem) -> usize {
    match layout.side(item) {
        Some(ToolbarSide::Left) => layout.left.iter().position(|i| *i == item).unwrap_or(0),
        Some(ToolbarSide::Right) => layout.right.iter().position(|i| *i == item).unwrap_or(0),
        None => 0,
    }
}

fn render_focus_mode(
    win: &SettingsWindow,
    app_entity: Entity<HylkiApp>,
    p: &HylkiPalette,
    cx: &mut Context<SettingsWindow>,
) -> AnyElement {
    let prefs = app_entity.read(cx).prefs.clone();
    let focus_mode_on = app_entity.read(cx).focus_mode;
    let a_focus = app_entity.clone();
    let a_layout = app_entity.clone();
    let a_preview = app_entity.clone();

    // WP-05：通用开关行（value 读当前偏好，set 写回；命中高亮按 id 匹配）。
    let sw = |id: &'static str,
              label: &'static str,
              desc: Option<&'static str>,
              val: bool,
              set: fn(&mut hylki_mail_core::config::AppPreferences, bool)|
     -> AnyElement {
        prefs_switch_row(
            &app_entity,
            id,
            label,
            desc,
            val,
            false,
            is_hit(win, id),
            set,
            p,
        )
    };

    v_flex()
        .w_full()
        .max_w(px(CARD_MAX_WIDTH))
        .gap_5()
        // 分组 1：专注模式（对齐 Linux 页 "Focus Mode" 组）。
        .child(setting_group(
            "专注模式",
            Some("开启后由本页决定侧边栏、列表与阅读器的布局，替代其他设置页的同类项；Ctrl+Shift+F 或主菜单可随时切换"),
            CARD_MAX_WIDTH,
            vec![
                setting_row(
                    SharedString::from("pref-focus-enabled-row"),
                    "启用专注模式",
                    Some("沉浸阅读：按下方各分组开关裁剪界面（当前会话生效，随主窗口即时切换）"),
                    bool_switch(
                        "sw-focus-enabled",
                        focus_mode_on,
                        false,
                        move |val, _, cx| {
                            a_focus.update(cx, |app, cx| app.set_focus_mode(val, cx));
                        },
                    ),
                    false,
                    is_hit(win, "pref-focus-enabled"),
                    p,
                ),
                sw(
                    "pref-focus-start",
                    "启动时自动进入专注模式",
                    None,
                    prefs.focus_start_enabled,
                    |p, v| p.focus_start_enabled = v,
                ),
            ],
            p,
        ))
        // 分组 2：侧边栏（对齐 Linux "Sidebar" 组 + Windows 专属整栏隐藏）。
        .child(setting_group(
            "侧边栏",
            None,
            CARD_MAX_WIDTH,
            vec![
                sw(
                    "pref-focus-rail-sidebar",
                    "折叠侧边栏为图标栏",
                    None,
                    prefs.focus_rail_sidebar,
                    |p, v| p.focus_rail_sidebar = v,
                ),
                sw(
                    "pref-focus-unread-counts",
                    "统一视图行显示未读计数",
                    None,
                    prefs.focus_unread_counts,
                    |p, v| p.focus_unread_counts = v,
                ),
                sw(
                    "pref-focus-rail-dots",
                    "图标栏以未读圆点代替计数",
                    None,
                    prefs.focus_rail_dots,
                    |p, v| p.focus_rail_dots = v,
                ),
                sw(
                    "pref-focus-hover-expand",
                    "悬停图标栏时滑出完整侧边栏",
                    None,
                    prefs.focus_hover_expand,
                    |p, v| p.focus_hover_expand = v,
                ),
                sw(
                    "pref-focus-hide-accounts",
                    "隐藏账号分区",
                    Some("仅保留统一视图分区。"),
                    prefs.focus_hide_accounts,
                    |p, v| p.focus_hide_accounts = v,
                ),
                sw(
                    "pref-focus-fold-accounts",
                    "折叠账号为头像",
                    Some("展开的账号收起为其头像"),
                    prefs.focus_fold_accounts,
                    |p, v| p.focus_fold_accounts = v,
                ),
                sw(
                    "pref-focus-click-folds",
                    "单击账号即折叠",
                    None,
                    prefs.focus_click_folds,
                    |p, v| p.focus_click_folds = v,
                ),
                sw(
                    "pref-focus-fold-unified",
                    "折叠统一视图行",
                    None,
                    prefs.focus_fold_unified,
                    |p, v| p.focus_fold_unified = v,
                ),
                sw(
                    "pref-focus-hide-sidebar",
                    "隐藏整个侧边栏（Windows 行为）",
                    Some("保留既有专注模式行为：整栏隐藏而非折叠为图标栏"),
                    prefs.focus_hide_sidebar,
                    |p, v| p.focus_hide_sidebar = v,
                ),
            ],
            p,
        ))
        // 分组 3：统一视图（对齐 Linux "Unified" 组）。
        .child(setting_group(
            "统一视图",
            None,
            CARD_MAX_WIDTH,
            vec![
                sw(
                    "pref-focus-show-inboxes",
                    "统一收件箱",
                    None,
                    prefs.focus_show_inboxes,
                    |p, v| p.focus_show_inboxes = v,
                ),
                sw(
                    "pref-focus-unified-starred",
                    "已加星标",
                    None,
                    prefs.focus_unified_starred,
                    |p, v| p.focus_unified_starred = v,
                ),
                sw(
                    "pref-focus-unified-sent",
                    "已发送",
                    None,
                    prefs.focus_unified_sent,
                    |p, v| p.focus_unified_sent = v,
                ),
                sw(
                    "pref-focus-unified-drafts",
                    "草稿箱",
                    None,
                    prefs.focus_unified_drafts,
                    |p, v| p.focus_unified_drafts = v,
                ),
                sw(
                    "pref-focus-unified-archive",
                    "已归档",
                    None,
                    prefs.focus_unified_archive,
                    |p, v| p.focus_unified_archive = v,
                ),
                sw(
                    "pref-focus-unified-junk",
                    "垃圾邮件",
                    None,
                    prefs.focus_unified_junk,
                    |p, v| p.focus_unified_junk = v,
                ),
                sw(
                    "pref-focus-unified-trash",
                    "废纸篓",
                    None,
                    prefs.focus_unified_trash,
                    |p, v| p.focus_unified_trash = v,
                ),
                sw(
                    "pref-focus-unified-snoozed",
                    "稍后处理",
                    None,
                    prefs.focus_unified_snoozed,
                    |p, v| p.focus_unified_snoozed = v,
                ),
            ],
            p,
        ))
        // 分组 4：分区（对齐 Linux "Sections" 组）。
        .child(setting_group(
            "分区",
            None,
            CARD_MAX_WIDTH,
            vec![
                sw(
                    "pref-focus-show-favorites",
                    "收藏",
                    None,
                    prefs.focus_show_favorites,
                    |p, v| p.focus_show_favorites = v,
                ),
                sw(
                    "pref-focus-show-filters",
                    "筛选文件夹",
                    None,
                    prefs.focus_show_filters,
                    |p, v| p.focus_show_filters = v,
                ),
                sw(
                    "pref-focus-show-tags",
                    "标签",
                    None,
                    prefs.focus_show_tags,
                    |p, v| p.focus_show_tags = v,
                ),
                sw(
                    "pref-focus-show-contacts",
                    "联系人",
                    None,
                    prefs.focus_show_contacts,
                    |p, v| p.focus_show_contacts = v,
                ),
                sw(
                    "pref-focus-show-attachments",
                    "附件",
                    None,
                    prefs.focus_show_attachments,
                    |p, v| p.focus_show_attachments = v,
                ),
            ],
            p,
        ))
        // 分组 5：消息列表（对齐 Linux "Message List" 组）。
        .child(setting_group(
            "消息列表",
            None,
            CARD_MAX_WIDTH,
            vec![
                setting_row(
                    "pref-focus-layout",
                    "布局",
                    Some("卡片双行或单行表格；自动在宽窗使用单行"),
                    combo_button(
                        "cb-focus-layout",
                        lookup_label(
                            FOCUS_LAYOUT_OPTIONS,
                            &match prefs.focus_layout.as_str() {
                                "single_line" => "single_line",
                                "automatic" => "automatic",
                                _ => "cards",
                            },
                            "卡片双行",
                        ),
                        FOCUS_LAYOUT_OPTIONS,
                        match prefs.focus_layout.as_str() {
                            "single_line" => "single_line",
                            "automatic" => "automatic",
                            _ => "cards",
                        },
                        false,
                        move |val: &str, _, cx| {
                            a_layout.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.focus_layout = val.to_string());
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-focus-layout"),
                    p,
                ),
                sw(
                    "pref-focus-column-headings",
                    "显示列标题",
                    None,
                    prefs.focus_column_headings,
                    |p, v| p.focus_column_headings = v,
                ),
                setting_row(
                    "pref-focus-preview-lines",
                    "摘要行数",
                    Some("列表卡片显示的正文预览行数"),
                    combo_button(
                        "cb-focus-preview-lines",
                        lookup_label(
                            FOCUS_PREVIEW_OPTIONS,
                            &prefs.focus_preview_lines.min(3),
                            "关闭",
                        ),
                        FOCUS_PREVIEW_OPTIONS,
                        prefs.focus_preview_lines.min(3),
                        false,
                        move |val: u32, _, cx| {
                            a_preview.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.focus_preview_lines = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-focus-preview-lines"),
                    p,
                ),
                sw(
                    "pref-focus-list-header",
                    "折叠列表头部",
                    Some("除侧边栏开关外的全部收进 ⋯。"),
                    prefs.focus_list_header,
                    |p, v| p.focus_list_header = v,
                ),
                sw(
                    "pref-focus-hide-avatars",
                    "隐藏发件人头像",
                    None,
                    prefs.focus_hide_avatars,
                    |p, v| p.focus_hide_avatars = v,
                ),
                sw(
                    "pref-focus-hide-subject",
                    "隐藏主题",
                    Some("标签胶囊随之隐藏。"),
                    prefs.focus_hide_subject,
                    |p, v| p.focus_hide_subject = v,
                ),
                sw(
                    "pref-focus-tags-below",
                    "标签置于文本下方",
                    None,
                    prefs.focus_tags_below,
                    |p, v| p.focus_tags_below = v,
                ),
                sw(
                    "pref-focus-hide-stars",
                    "隐藏星标",
                    None,
                    prefs.focus_hide_stars,
                    |p, v| p.focus_hide_stars = v,
                ),
                sw(
                    "pref-focus-hide-attachment-icons",
                    "隐藏附件图标",
                    None,
                    prefs.focus_hide_attachment_icons,
                    |p, v| p.focus_hide_attachment_icons = v,
                ),
                sw(
                    "pref-focus-hide-list",
                    "隐藏中间邮件列表（Windows 行为）",
                    Some("聚焦阅读当前邮件，隐藏列表窗格"),
                    prefs.focus_hide_list,
                    |p, v| p.focus_hide_list = v,
                ),
            ],
            p,
        ))
        // 分组 6：阅读（对齐 Linux "Reading" 组；复用旧字段不丢数据）。
        .child(setting_group(
            "阅读",
            None,
            CARD_MAX_WIDTH,
            vec![
                sw(
                    "pref-focus-fold-toolbar",
                    "折叠阅读器工具栏",
                    Some("每个按钮收进其 ⋯ 菜单。"),
                    prefs.focus_fold_toolbar,
                    |p, v| p.focus_fold_toolbar = v,
                ),
                sw(
                    "pref-focus-reader-view",
                    "简化阅读视图",
                    Some("打开邮件即进入简化阅读视图。"),
                    prefs.focus_reader_view,
                    |p, v| p.focus_reader_view = v,
                ),
            ],
            p,
        ))
        .into_any_element()
}

fn render_sidebar(
    win: &SettingsWindow,
    app_entity: Entity<HylkiApp>,
    p: &HylkiPalette,
    cx: &mut Context<SettingsWindow>,
) -> AnyElement {
    let prefs = app_entity.read(cx).prefs.clone();
    let a1 = app_entity.clone();
    let a2 = app_entity.clone();
    let a3 = app_entity.clone();
    let a4 = app_entity.clone();
    let a5 = app_entity.clone();
    let a6 = app_entity.clone();
    let u1 = app_entity.clone();
    let u2 = app_entity.clone();
    let u3 = app_entity.clone();
    let u4 = app_entity.clone();
    let u5 = app_entity.clone();

    v_flex()
        .w_full()
        .max_w(px(CARD_MAX_WIDTH))
        .gap_5()
        .child(setting_group(
            "侧边栏区段与折叠",
            None,
            CARD_MAX_WIDTH,
            vec![
                setting_row(
                    "pref-default-view",
                    "启动时默认打开视图",
                    Some("选择每次冷启动定位的视图或恢复上一次阅读的文件夹"),
                    combo_button(
                        "cb-default-view",
                        lookup_label(
                            DEFAULT_VIEW_OPTIONS,
                            &prefs.default_view.as_str(),
                            "统一收件箱",
                        ),
                        DEFAULT_VIEW_OPTIONS,
                        if prefs.default_view == "last" {
                            "last"
                        } else {
                            "unified"
                        },
                        false,
                        move |val: &str, _, cx| {
                            a6.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.default_view = val.to_string());
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-default-view"),
                    p,
                ),
                setting_row(
                    "pref-show-accounts",
                    "显示账号与文件夹列表",
                    Some("可用 Ctrl+Shift+A 快速显隐账号树"),
                    bool_switch(
                        "sw-show-acc",
                        prefs.show_accounts,
                        false,
                        move |val, _, cx| {
                            a1.update(cx, |app, cx| {
                                app.set_show_accounts(val, cx);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-show-accounts"),
                    p,
                ),
                setting_row(
                    "pref-sidebar-contacts",
                    "显示「联系人」入口",
                    None,
                    bool_switch(
                        "sw-sb-contacts",
                        prefs.sidebar_show_contacts,
                        false,
                        move |val, _, cx| {
                            a2.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.sidebar_show_contacts = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-sidebar-contacts"),
                    p,
                ),
                setting_row(
                    "pref-sidebar-attachments",
                    "显示「附件总览」入口",
                    None,
                    bool_switch(
                        "sw-sb-att",
                        prefs.sidebar_show_attachments,
                        false,
                        move |val, _, cx| {
                            a3.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.sidebar_show_attachments = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-sidebar-attachments"),
                    p,
                ),
                setting_row(
                    "pref-remember-sidebar",
                    "记住侧边栏折叠状态",
                    Some("重启时恢复上次完整侧边栏或窄图标栏状态"),
                    bool_switch(
                        "sw-sb-remember",
                        prefs.remember_sidebar_collapsed,
                        false,
                        move |val, _, cx| {
                            a4.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.remember_sidebar_collapsed = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-remember-sidebar"),
                    p,
                ),
                setting_row(
                    "pref-rail-dots",
                    "窄栏模式下用未读圆点代替数字计数",
                    None,
                    bool_switch(
                        "sw-rail-dots",
                        prefs.rail_unread_dots,
                        false,
                        move |val, _, cx| {
                            a5.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.rail_unread_dots = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-rail-dots"),
                    p,
                ),
            ],
            p,
        ))
        .child(setting_group(
            "统一视图导航项",
            Some("控制侧边栏顶部跨账号统一视图入口的显示与隐藏"),
            CARD_MAX_WIDTH,
            vec![
                setting_row(
                    "pref-unified-inbox",
                    "显示「统一收件箱」",
                    None,
                    bool_switch(
                        "sw-u-inbox",
                        prefs.unified_show_inbox,
                        false,
                        move |val, _, cx| {
                            u1.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.unified_show_inbox = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-unified-inbox"),
                    p,
                ),
                setting_row(
                    "pref-unified-starred",
                    "显示「已加星标」",
                    None,
                    bool_switch(
                        "sw-u-starred",
                        prefs.unified_show_starred,
                        false,
                        move |val, _, cx| {
                            u2.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.unified_show_starred = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-unified-starred"),
                    p,
                ),
                setting_row(
                    "pref-unified-sent",
                    "显示「已发送」",
                    None,
                    bool_switch(
                        "sw-u-sent",
                        prefs.unified_show_sent,
                        false,
                        move |val, _, cx| {
                            u3.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.unified_show_sent = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-unified-sent"),
                    p,
                ),
                setting_row(
                    "pref-unified-drafts",
                    "显示「草稿箱」",
                    None,
                    bool_switch(
                        "sw-u-drafts",
                        prefs.unified_show_drafts,
                        false,
                        move |val, _, cx| {
                            u4.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.unified_show_drafts = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-unified-drafts"),
                    p,
                ),
                setting_row(
                    "pref-unified-archive",
                    "显示「已归档」",
                    None,
                    bool_switch(
                        "sw-u-archive",
                        prefs.unified_show_archive,
                        false,
                        move |val, _, cx| {
                            u5.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.unified_show_archive = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-unified-archive"),
                    p,
                ),
                prefs_switch_row(
                    &app_entity,
                    "pref-unified-junk",
                    "显示「垃圾邮件」",
                    None,
                    prefs.unified_show_junk,
                    false,
                    is_hit(win, "pref-unified-junk"),
                    |p, v| p.unified_show_junk = v,
                    p,
                ),
                prefs_switch_row(
                    &app_entity,
                    "pref-unified-trash",
                    "显示「废纸篓」",
                    None,
                    prefs.unified_show_trash,
                    false,
                    is_hit(win, "pref-unified-trash"),
                    |p, v| p.unified_show_trash = v,
                    p,
                ),
                prefs_switch_row(
                    &app_entity,
                    "pref-unified-snoozed",
                    "显示「稍后处理」",
                    None,
                    prefs.unified_show_snoozed,
                    false,
                    is_hit(win, "pref-unified-snoozed"),
                    |p, v| p.unified_show_snoozed = v,
                    p,
                ),
            ],
            p,
        ))
        // WP-05：分区分组（对齐 Linux sidebar.rs 页 "Sections" 组）。
        .child(setting_group(
            "分区分组",
            Some("控制筛选文件夹 / 智能文件夹 / 标签等分区入口的显示（分区的实际渲染随侧边栏分区工作落地）"),
            CARD_MAX_WIDTH,
            vec![
                prefs_switch_row(
                    &app_entity,
                    "pref-sidebar-filtered",
                    "显示「筛选文件夹」分区",
                    None,
                    prefs.sidebar_show_filtered,
                    false,
                    is_hit(win, "pref-sidebar-filtered"),
                    |p, v| p.sidebar_show_filtered = v,
                    p,
                ),
                prefs_switch_row(
                    &app_entity,
                    "pref-sidebar-smart",
                    "显示「智能文件夹」分区",
                    None,
                    prefs.sidebar_show_smart,
                    false,
                    is_hit(win, "pref-sidebar-smart"),
                    |p, v| p.sidebar_show_smart = v,
                    p,
                ),
                prefs_switch_row(
                    &app_entity,
                    "pref-sidebar-tags",
                    "显示「标签」分区",
                    None,
                    prefs.sidebar_show_tags,
                    false,
                    is_hit(win, "pref-sidebar-tags"),
                    |p, v| p.sidebar_show_tags = v,
                    p,
                ),
            ],
            p,
        ))
        .into_any_element()
}

fn render_message_list(
    win: &SettingsWindow,
    app_entity: Entity<HylkiApp>,
    p: &HylkiPalette,
    cx: &mut Context<SettingsWindow>,
) -> AnyElement {
    let prefs = app_entity.read(cx).prefs.clone();
    let a1 = app_entity.clone();
    let a2 = app_entity.clone();
    let a3 = app_entity.clone();
    let a4 = app_entity.clone();
    let a5 = app_entity.clone();
    let a6 = app_entity.clone();

    let a_layout = app_entity.clone();
    let a_preview = app_entity.clone();

    v_flex()
        .w_full()
        .max_w(px(CARD_MAX_WIDTH))
        .gap_5()
        // WP-05：布局与摘要行数（对齐 Linux list.rs 页 Layout / Preview lines；
        // 宽表的完整列渲染随宽表模式工作落地）。
        .child(setting_group(
            "布局与摘要行数",
            None,
            CARD_MAX_WIDTH,
            vec![
                setting_row(
                    "pref-list-layout",
                    "布局",
                    Some("经典双行卡片，或宽表列视图（选宽表时自动切换为紧凑单行密度）"),
                    combo_button(
                        "cb-list-layout",
                        lookup_label(
                            LIST_LAYOUT_OPTIONS,
                            &if prefs.list_layout == "wide" {
                                "wide"
                            } else {
                                "cards"
                            },
                            "经典双行",
                        ),
                        LIST_LAYOUT_OPTIONS,
                        if prefs.list_layout == "wide" {
                            "wide"
                        } else {
                            "cards"
                        },
                        false,
                        move |val: &str, _, cx| {
                            a_layout.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| {
                                    p.list_layout = val.to_string();
                                    // 行为映射：宽表列视图即紧凑单行密度（现有逻辑），
                                    // 经典双行恢复卡片密度；宽表列头渲染随宽表工作落地。
                                    p.compact_list = val == "wide";
                                });
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-list-layout"),
                    p,
                ),
                setting_row(
                    "pref-list-preview-lines",
                    "摘要行数",
                    Some("列表卡片显示的正文预览行数（多行渲染随宽表模式工作落地）"),
                    combo_button(
                        "cb-list-preview-lines",
                        lookup_label(
                            LIST_PREVIEW_OPTIONS,
                            &prefs.list_preview_lines.clamp(1, 3),
                            "1 行",
                        ),
                        LIST_PREVIEW_OPTIONS,
                        prefs.list_preview_lines.clamp(1, 3),
                        false,
                        move |val: u32, _, cx| {
                            a_preview.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.list_preview_lines = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-list-preview-lines"),
                    p,
                ),
            ],
            p,
        ))
        .child(setting_group(
            "行样式与分组",
            None,
            CARD_MAX_WIDTH,
            vec![
                setting_row(
                    "pref-compact-list",
                    "紧凑列表行",
                    Some("以更紧凑的单/双行高度展示更多邮件条目"),
                    bool_switch(
                        "sw-compact-list",
                        prefs.compact_list,
                        false,
                        move |val, _, cx| {
                            a1.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.compact_list = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-compact-list"),
                    p,
                ),
                setting_row(
                    "pref-show-avatars",
                    "显示发件人头像",
                    Some("隐藏后保留左侧未读状态蓝点与对齐间距"),
                    bool_switch(
                        "sw-avatars",
                        prefs.show_avatars,
                        false,
                        move |val, _, cx| {
                            a2.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.show_avatars = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-show-avatars"),
                    p,
                ),
                setting_row(
                    "pref-list-group-date",
                    "按日期分组显示",
                    Some("按时间排序时显示「今天 / 昨天 / 本周 / 更早」分组头"),
                    bool_switch(
                        "sw-list-date-group",
                        prefs.list_group_by_date,
                        false,
                        move |val, _, cx| {
                            a3.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.list_group_by_date = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-list-group-date"),
                    p,
                ),
            ],
            p,
        ))
        .child(setting_group(
            "搜索栏与批量操作栏位置",
            None,
            CARD_MAX_WIDTH,
            vec![
                setting_row(
                    "pref-list-search-always",
                    "始终显示搜索栏",
                    None,
                    bool_switch(
                        "sw-search-always",
                        prefs.list_search_always_open,
                        false,
                        move |val, _, cx| {
                            a4.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.list_search_always_open = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-list-search-always"),
                    p,
                ),
                setting_row(
                    "pref-list-search-bottom",
                    "搜索栏放在列表底部",
                    None,
                    bool_switch(
                        "sw-search-bottom",
                        prefs.list_search_bottom,
                        false,
                        move |val, _, cx| {
                            a5.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.list_search_bottom = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-list-search-bottom"),
                    p,
                ),
                setting_row(
                    "pref-list-bulk-bottom",
                    "多选批量操作栏放在列表底部",
                    None,
                    bool_switch(
                        "sw-bulk-bottom",
                        prefs.list_bulk_bar_bottom,
                        false,
                        move |val, _, cx| {
                            a6.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.list_bulk_bar_bottom = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-list-bulk-bottom"),
                    p,
                ),
            ],
            p,
        ))
        // WP-05：列顺序与显隐编辑器（对齐 Linux `chips.rs::ColumnEditor` 的
        // 12 列；上下移排序 + 显隐，完整列渲染随宽表模式工作落地）。
        .child(setting_group(
            "列顺序与显隐",
            Some("单行宽表模式显示的列，从左到右排序；主题列为弹性列恒在显示"),
            CARD_MAX_WIDTH,
            {
                let mut rows: Vec<AnyElement> = Vec::new();
                rows.push(prefs_switch_row(
                    &app_entity,
                    "pref-list-show-headings",
                    "显示列标题栏",
                    Some("宽表模式在列表顶部显示可点击排序的列头"),
                    prefs.list_show_headings,
                    false,
                    is_hit(win, "pref-list-show-headings"),
                    |p, v| p.list_show_headings = v,
                    p,
                ));
                let shown = prefs.list_columns_shown();
                for (idx, col) in shown.iter().enumerate() {
                    let col_key = col.key();
                    let a_up = app_entity.clone();
                    let a_down = app_entity.clone();
                    let a_remove = app_entity.clone();
                    let controls = h_flex()
                        .gap_1()
                        .child(
                            Button::new(("col-up", idx))
                                .ghost()
                                .compact()
                                .disabled(idx == 0)
                                .icon(IconName::ArrowUp)
                                .tooltip("上移")
                                .on_click(move |_, _, cx| {
                                    a_up.update(cx, |app, cx| {
                                        app.mutate_preferences(cx, |p| {
                                            let mut cols = p.list_columns_shown();
                                            if idx > 0 {
                                                cols.swap(idx, idx - 1);
                                                write_column_keys(p, &cols);
                                            }
                                        });
                                    });
                                }),
                        )
                        .child(
                            Button::new(("col-down", idx))
                                .ghost()
                                .compact()
                                .disabled(idx + 1 >= shown.len())
                                .icon(IconName::ArrowDown)
                                .tooltip("下移")
                                .on_click(move |_, _, cx| {
                                    a_down.update(cx, |app, cx| {
                                        app.mutate_preferences(cx, |p| {
                                            let mut cols = p.list_columns_shown();
                                            if idx + 1 < cols.len() {
                                                cols.swap(idx, idx + 1);
                                                write_column_keys(p, &cols);
                                            }
                                        });
                                    });
                                }),
                        )
                        .child(
                            Button::new(("col-remove", idx))
                                .ghost()
                                .compact()
                                .icon(IconName::Trash)
                                .label("移除")
                                .on_click(move |_, _, cx| {
                                    a_remove.update(cx, |app, cx| {
                                        app.mutate_preferences(cx, |p| {
                                            let mut cols = p.list_columns_shown();
                                            cols.retain(|c| c.key() != col_key);
                                            write_column_keys(p, &cols);
                                        });
                                    });
                                }),
                        );
                    rows.push(setting_row(
                        SharedString::from(format!("pref-list-col-{}", col.key())),
                        col.label(),
                        None,
                        controls.into_any_element(),
                        false,
                        false,
                        p,
                    ));
                }
                for hidden_col in ListColumn::ALL
                    .iter()
                    .copied()
                    .filter(|c| !shown.contains(c))
                {
                    let a_add = app_entity.clone();
                    rows.push(setting_row(
                        SharedString::from(format!("pref-list-col-hidden-{}", hidden_col.key())),
                        hidden_col.label(),
                        None,
                        Button::new(SharedString::from(format!("col-add-{}", hidden_col.key())))
                            .outline()
                            .compact()
                            .icon(IconName::Plus)
                            .label("显示")
                            .on_click(move |_, _, cx| {
                                a_add.update(cx, |app, cx| {
                                    app.mutate_preferences(cx, |p| {
                                        let mut cols = p.list_columns_shown();
                                        cols.push(hidden_col);
                                        write_column_keys(p, &cols);
                                    });
                                });
                            })
                            .into_any_element(),
                        false,
                        false,
                        p,
                    ));
                }
                let a_restore = app_entity.clone();
                rows.push(
                    h_flex()
                        .px_4()
                        .py_2p5()
                        .justify_end()
                        .child(
                            Button::new("btn-columns-restore")
                                .outline()
                                .compact()
                                .label("恢复默认列")
                                .on_click(move |_, _, cx| {
                                    a_restore.update(cx, |app, cx| {
                                        app.mutate_preferences(cx, |p| {
                                            p.list_columns =
                                                ListColumn::to_keys(&ListColumn::DEFAULT);
                                        });
                                    });
                                }),
                        )
                        .into_any_element(),
                );
                rows
            },
            p,
        ))
        .into_any_element()
}

fn render_conversations(
    win: &SettingsWindow,
    app_entity: Entity<HylkiApp>,
    p: &HylkiPalette,
    cx: &mut Context<SettingsWindow>,
) -> AnyElement {
    let prefs = app_entity.read(cx).prefs.clone();
    let fold_mode = prefs.effective_fold_messages_mode();
    let a1 = app_entity.clone();
    let a2 = app_entity.clone();
    let a3 = app_entity.clone();

    v_flex()
        .w_full()
        .max_w(px(CARD_MAX_WIDTH))
        .gap_5()
        .child(setting_group(
            "会话线程与卡片显示",
            None,
            CARD_MAX_WIDTH,
            vec![
                setting_row(
                    "pref-thread-newest-first",
                    "按最新回复置顶排列会话卡片",
                    Some("关闭则按历史顺序从最早一封向下浏览"),
                    bool_switch(
                        "sw-thread-newest",
                        prefs.thread_newest_first,
                        false,
                        move |val, _, cx| {
                            a1.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.thread_newest_first = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-thread-newest-first"),
                    p,
                ),
                setting_row(
                    "pref-fold-messages-mode",
                    "自动折叠多封会话卡片",
                    Some("切换后会立即重新应用当前打开会话的默认折叠状态"),
                    combo_button(
                        "cb-fold-mode",
                        lookup_label(FOLD_MODE_OPTIONS, &fold_mode, "折叠较早邮件"),
                        FOLD_MODE_OPTIONS,
                        fold_mode,
                        false,
                        move |mode: &str, _, cx| {
                            a2.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.set_fold_messages_mode(mode));
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-fold-messages-mode"),
                    p,
                ),
                setting_row(
                    "pref-card-attachments",
                    "在各会话卡片中展示附件",
                    Some("每张卡片单独列出其附件预览、大小与保存动作"),
                    bool_switch(
                        "sw-card-att",
                        prefs.card_attachments,
                        false,
                        move |val, _, cx| {
                            a3.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.card_attachments = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-card-attachments"),
                    p,
                ),
            ],
            p,
        ))
        .into_any_element()
}

fn render_reading(
    win: &SettingsWindow,
    app_entity: Entity<HylkiApp>,
    p: &HylkiPalette,
    cx: &mut Context<SettingsWindow>,
) -> AnyElement {
    let prefs = app_entity.read(cx).prefs.clone();
    let a_mark = app_entity.clone();
    let a_pos = app_entity.clone();
    let a_card = app_entity.clone();
    let a_theme = app_entity.clone();
    let a_recip = app_entity.clone();
    let a_chip = app_entity.clone();
    let a_tsw = app_entity.clone();
    let a_vsw = app_entity.clone();
    let a_def = app_entity.clone();
    let a_zoom = app_entity.clone();

    v_flex()
        .w_full()
        .max_w(px(CARD_MAX_WIDTH))
        .gap_5()
        .child(setting_group(
            "已读行为",
            None,
            CARD_MAX_WIDTH,
            vec![setting_row(
                "pref-read-mark-mode",
                "自动标记为已读时机",
                Some("控制点击选中未读邮件后何时提交已读状态"),
                combo_button(
                    "cb-read-mark",
                    lookup_label(
                        READ_MARK_OPTIONS,
                        &prefs.read_mark_mode.as_str(),
                        "打开即标记已读",
                    ),
                    READ_MARK_OPTIONS,
                    match prefs.read_mark_mode.as_str() {
                        "delay" => "delay",
                        "manual" => "manual",
                        _ => "shown",
                    },
                    false,
                    move |mode: &str, _, cx| {
                        a_mark.update(cx, |app, cx| {
                            app.mutate_preferences(cx, |p| p.read_mark_mode = mode.to_string());
                        });
                    },
                ),
                false,
                is_hit(win, "pref-read-mark-mode"),
                p,
            )],
            p,
        ))
        .child(setting_group(
            "布局与正文字体主题",
            None,
            CARD_MAX_WIDTH,
            vec![
                setting_row(
                    "pref-reader-position",
                    "阅读器窗格位置",
                    Some("可用 Ctrl+Alt+1 / 2 / 3 快捷键实时切换"),
                    combo_button(
                        "cb-reader-pos",
                        lookup_label(
                            READER_POSITION_OPTIONS,
                            &prefs.reader_position.as_str(),
                            "列表右侧（推荐）",
                        ),
                        READER_POSITION_OPTIONS,
                        match prefs.reader_position.as_str() {
                            "below" => "below",
                            "off" => "off",
                            _ => "right",
                        },
                        false,
                        move |pos: &str, _, cx| {
                            a_pos.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.reader_position = pos.to_string());
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-reader-position"),
                    p,
                ),
                setting_row(
                    "pref-single-message-card",
                    "单封邮件显示卡片框线",
                    Some("关闭后单封邮件将边缘贴合铺满阅读区（仅单封会话生效）"),
                    bool_switch(
                        "sw-single-card",
                        prefs.single_message_card,
                        false,
                        move |val, _, cx| {
                            a_card.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.single_message_card = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-single-message-card"),
                    p,
                ),
                setting_row(
                    "pref-message-theme",
                    "默认信纸主题",
                    Some("控制 HTML/文本正文底色（可被单封邮件顶部切换临时覆盖）"),
                    combo_button(
                        "cb-msg-theme",
                        lookup_label(
                            MESSAGE_THEME_OPTIONS,
                            &prefs.message_theme.as_str(),
                            "跟随界面主题",
                        ),
                        MESSAGE_THEME_OPTIONS,
                        match prefs.message_theme.as_str() {
                            "light" => "light",
                            "dark" => "dark",
                            _ => "system",
                        },
                        false,
                        move |val: &str, _, cx| {
                            a_theme.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.message_theme = val.to_string());
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-message-theme"),
                    p,
                ),
                setting_row(
                    "pref-reader-zoom",
                    "阅读器正文字号",
                    Some("同时也可在阅读邮件时使用 Ctrl+= / Ctrl+- / Ctrl+0 快速调节"),
                    combo_button(
                        "cb-reader-zoom",
                        lookup_label(
                            READER_ZOOM_OPTIONS,
                            &prefs.reader_zoom_percent,
                            "100%（默认）",
                        ),
                        READER_ZOOM_OPTIONS,
                        prefs.reader_zoom_percent,
                        false,
                        move |val: u32, _, cx| {
                            a_zoom.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.reader_zoom_percent = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-reader-zoom"),
                    p,
                ),
                setting_row(
                    "pref-always-show-recipients",
                    "始终显示收件人行",
                    None,
                    bool_switch(
                        "sw-show-recip",
                        prefs.always_show_recipients,
                        false,
                        move |val, _, cx| {
                            a_recip.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.always_show_recipients = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-always-show-recipients"),
                    p,
                ),
            ],
            p,
        ))
        .child(setting_group(
            "阅读器顶栏与默认视图",
            None,
            CARD_MAX_WIDTH,
            vec![
                setting_row(
                    "pref-reader-show-account-chip",
                    "顶栏显示账号邮箱标签",
                    None,
                    bool_switch(
                        "sw-rd-chip",
                        prefs.reader_show_account_chip,
                        false,
                        move |val, _, cx| {
                            a_chip.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.reader_show_account_chip = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-reader-show-account-chip"),
                    p,
                ),
                setting_row(
                    "pref-reader-show-theme-switch",
                    "顶栏显示明暗背景切换按钮",
                    None,
                    bool_switch(
                        "sw-rd-tsw",
                        prefs.reader_show_theme_switch,
                        false,
                        move |val, _, cx| {
                            a_tsw.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.reader_show_theme_switch = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-reader-show-theme-switch"),
                    p,
                ),
                setting_row(
                    "pref-reader-show-view-switch",
                    "顶栏显示简化阅读视图切换按钮",
                    None,
                    bool_switch(
                        "sw-rd-vsw",
                        prefs.reader_show_view_switch,
                        false,
                        move |val, _, cx| {
                            a_vsw.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.reader_show_view_switch = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-reader-show-view-switch"),
                    p,
                ),
                setting_row(
                    "pref-reader-default-mode",
                    "打开邮件时默认视图",
                    None,
                    combo_button(
                        "cb-rd-def-mode",
                        lookup_label(
                            READER_DEFAULT_MODE_OPTIONS,
                            &prefs.reader_default_mode.as_str(),
                            "保持当前选择",
                        ),
                        READER_DEFAULT_MODE_OPTIONS,
                        match prefs.reader_default_mode.as_str() {
                            "on" => "on",
                            "off" => "off",
                            _ => "remember",
                        },
                        false,
                        move |mode: &str, _, cx| {
                            a_def.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| {
                                    p.reader_default_mode = mode.to_string();
                                });
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-reader-default-mode"),
                    p,
                ),
            ],
            p,
        ))
        .into_any_element()
}

fn render_composing(
    win: &SettingsWindow,
    app_entity: Entity<HylkiApp>,
    p: &HylkiPalette,
    cx: &mut Context<SettingsWindow>,
) -> AnyElement {
    let app_ref = app_entity.read(cx);
    let prefs = app_ref.prefs.clone();
    let mut from_options: Vec<(SharedString, String)> = vec![(
        SharedString::from("自动（使用当前查看账号）"),
        String::new(),
    )];
    for acc in app_ref.accounts.iter().filter(|a| a.enabled) {
        let primary = acc.display_label();
        let label = if primary == acc.email {
            if acc.display_name.trim().is_empty() {
                acc.email.clone()
            } else {
                format!("{} <{}>", acc.display_name.trim(), acc.email)
            }
        } else {
            format!("{primary} <{}>", acc.email)
        };
        from_options.push((SharedString::from(label), acc.email.clone()));
    }
    let current_from_label = from_options
        .iter()
        .find(|(_, v)| v.eq_ignore_ascii_case(&prefs.compose_default_from))
        .map(|(l, _)| l.clone())
        .unwrap_or_else(|| SharedString::from("自动（使用当前查看账号）"));

    let a_from = app_entity.clone();
    let a_inline = app_entity.clone();
    let a_above = app_entity.clone();
    let a_dash = app_entity.clone();
    let a_undo = app_entity.clone();
    let a_hdr_save = app_entity.clone();
    let a_hdr_reset = app_entity.clone();
    let a_kw_save = app_entity.clone();
    let a_kw_reset = app_entity.clone();

    let mut stack = v_flex().w_full().max_w(px(CARD_MAX_WIDTH)).gap_5();
    if let Some(ref feedback) = win.compose_inputs_feedback {
        stack = stack.child(
            div()
                .px_3()
                .py_2()
                .rounded(px(8.0))
                .bg(p.accent.opacity(0.12))
                .text_xs()
                .text_color(p.accent)
                .child(feedback.clone()),
        );
    }

    stack
        .child(setting_group(
            "发件身份与签名",
            None,
            CARD_MAX_WIDTH,
            vec![
                setting_row(
                    "pref-compose-default-from",
                    "新邮件默认发件账号",
                    None,
                    combo_button_dynamic(
                        "cb-compose-from",
                        current_from_label,
                        from_options,
                        prefs.compose_default_from.clone(),
                        false,
                        move |email, _, cx| {
                            a_from.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.compose_default_from = email);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-compose-default-from"),
                    p,
                ),
                setting_row(
                    "pref-compose-inline",
                    "回复与转发在阅读器内联分屏",
                    Some(
                        "开启后原邮件留在上半部分可随时对照滚动，写信面板贴在下半部分；\
                         也可随时点写信栏右上角的 ↗ 弹出为独立窗口",
                    ),
                    bool_switch(
                        "sw-compose-inline",
                        prefs.compose_inline,
                        false,
                        move |val, _, cx| {
                            a_inline.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.compose_inline = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-compose-inline"),
                    p,
                ),
                setting_row(
                    "pref-signature-above-quote",
                    "回复与转发时的签名位置",
                    Some("默认在引用内容上方，使新写正文与签名紧靠在一起"),
                    combo_button(
                        "cb-sig-position",
                        lookup_label(
                            SIGNATURE_POSITION_OPTIONS,
                            &prefs.signature_above_quote,
                            "在引用内容上方",
                        ),
                        SIGNATURE_POSITION_OPTIONS,
                        prefs.signature_above_quote,
                        false,
                        move |above, _, cx| {
                            a_above.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.signature_above_quote = above);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-signature-above-quote"),
                    p,
                ),
                setting_row(
                    "pref-signature-dashes",
                    "签名前插入 \"-- \" 分隔符",
                    None,
                    bool_switch("sw-sig-dash", prefs.signature_dashes, false, move |val, _, cx| {
                        a_dash.update(cx, |app, cx| {
                            app.mutate_preferences(cx, |p| p.signature_dashes = val);
                        });
                    }),
                    false,
                    is_hit(win, "pref-signature-dashes"),
                    p,
                ),
            ],
            p,
        ))
        .child(setting_group(
            "发送与引用头格式",
            None,
            CARD_MAX_WIDTH,
            vec![
                setting_row(
                    "pref-undo-send-secs",
                    "撤销发送缓冲时间",
                    Some("点击发送后在指定秒数内可点击状态栏「撤回」中止发出"),
                    combo_button(
                        "cb-undo-send",
                        lookup_label(UNDO_SEND_OPTIONS, &prefs.undo_send_secs, "5 秒（推荐）"),
                        UNDO_SEND_OPTIONS,
                        prefs.undo_send_secs,
                        false,
                        move |secs: u32, _, cx| {
                            a_undo.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.undo_send_secs = secs);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-undo-send-secs"),
                    p,
                ),
                v_flex()
                    .id("pref-reply-header-template")
                    .px_4()
                    .py_3()
                    .gap_2()
                    .when(is_hit(win, "pref-reply-header-template"), |el| {
                        el.bg(p.accent.opacity(0.18))
                    })
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(p.text)
                            .child("自定义回复引用头模板"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.text_muted)
                            .child("支持 {name}、{address}、{date}、{time}、{subject}；留空即用默认中文格式"),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(div().flex_1().min_w_0().child(Input::new(&win.reply_header_input)))
                            .child(
                                Button::new("apply-reply-hdr")
                                    .primary()
                                    .compact()
                                    .label("应用")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.apply_reply_header_input(&a_hdr_save, cx);
                                    })),
                            )
                            .child(
                                Button::new("reset-reply-hdr")
                                    .outline()
                                    .compact()
                                    .label("恢复默认")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.reset_reply_header_input(&a_hdr_reset, window, cx);
                                    })),
                            ),
                    )
                    .into_any_element(),
            ],
            p,
        ))
        .child(setting_group(
            "附件遗漏检测",
            Some("多个关键词用逗号分隔；若发件主题或输入正文中出现关键词但附件列表为空，将在发送前提醒"),
            CARD_MAX_WIDTH,
            vec![
                v_flex()
                    .id("pref-attachment-keywords")
                    .px_4()
                    .py_3()
                    .gap_2()
                    .when(is_hit(win, "pref-attachment-keywords"), |el| {
                        el.bg(p.accent.opacity(0.18))
                    })
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(div().flex_1().min_w_0().child(Input::new(&win.attachment_keywords_input)))
                            .child(
                                Button::new("apply-att-kw")
                                    .primary()
                                    .compact()
                                    .label("应用")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.apply_attachment_keywords_input(&a_kw_save, cx);
                                    })),
                            )
                            .child(
                                Button::new("reset-att-kw")
                                    .outline()
                                    .compact()
                                    .label("恢复默认")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.reset_attachment_keywords_input(&a_kw_reset, window, cx);
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.text_muted)
                            .child(format!("默认列表：{DEFAULT_ATTACHMENT_KEYWORDS}")),
                    )
                    .into_any_element(),
            ],
            p,
        ))
        .into_any_element()
}

fn render_privacy(
    win: &SettingsWindow,
    app_entity: Entity<HylkiApp>,
    p: &HylkiPalette,
    cx: &mut Context<SettingsWindow>,
) -> AnyElement {
    let prefs = app_entity.read(cx).prefs.clone();
    let a1 = app_entity.clone();
    let a2 = app_entity.clone();
    let a3 = app_entity.clone();
    let a_receipt = app_entity.clone();
    let win_entity = cx.entity().clone();

    v_flex()
        .w_full()
        .max_w(px(CARD_MAX_WIDTH))
        .gap_5()
        .child(setting_group(
            "远程图像与跟踪保护",
            None,
            CARD_MAX_WIDTH,
            vec![
                setting_row(
                    "pref-block-remote-images",
                    "默认屏蔽远程图像",
                    Some("阻止未知发件人通过外部图片链接追踪您的阅读时间与 IP"),
                    bool_switch(
                        "sw-block-img",
                        prefs.block_remote_images,
                        false,
                        move |val, _, cx| {
                            a1.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.block_remote_images = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-block-remote-images"),
                    p,
                ),
                setting_row(
                    "pref-show-remote-banner",
                    "显示已屏蔽远程图像提示横幅",
                    Some("关闭后静默屏蔽外链图片，不再显示「信任发件人」横幅"),
                    bool_switch(
                        "sw-remote-banner",
                        prefs.show_remote_banner,
                        false,
                        move |val, _, cx| {
                            a2.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.show_remote_banner = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-show-remote-banner"),
                    p,
                ),
                setting_row(
                    "pref-clean-links",
                    "自动剥离链接中的追踪参数",
                    Some("渲染邮件链接时移除 utm_* / fbclid / gclid / msclkid / spm 等参数"),
                    bool_switch(
                        "sw-clean-links",
                        prefs.clean_links,
                        false,
                        move |val, _, cx| {
                            a3.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.clean_links = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-clean-links"),
                    p,
                ),
                prefs_switch_row(
                    &app_entity,
                    "pref-spoof-banner",
                    "警告仿冒域名发件人",
                    Some("检测到显示名称或域名疑似钓鱼仿冒时在阅读器顶部显示警告横幅"),
                    prefs.show_spoof_banner,
                    false,
                    is_hit(win, "pref-spoof-banner"),
                    |p, v| p.show_spoof_banner = v,
                    p,
                ),
                setting_row(
                    "senders-link",
                    "管理允许加载远程图像的发件人白名单",
                    Some(
                        format!(
                            "当前已收录 {} 位可信发件人",
                            prefs.allowed_remote_senders.len()
                        )
                        .as_str(),
                    ),
                    Button::new("privacy-open-senders")
                        .outline()
                        .compact()
                        .icon(IconName::ChevronRight)
                        .label("前往管理")
                        .on_click(move |_, window, cx| {
                            win_entity.update(cx, |w, cx| {
                                w.select_tab(PrefTab::Senders, window, cx);
                            });
                        })
                        .into_any_element(),
                    false,
                    false,
                    p,
                ),
            ],
            p,
        ))
        .child(setting_group(
            "头像与公司徽标",
            Some("从第三方服务拉取发件人头像或品牌徽标（可能向对应服务发送邮箱哈希或域名请求）"),
            CARD_MAX_WIDTH,
            vec![
                prefs_switch_row(
                    &app_entity,
                    "pref-use-gravatar",
                    "加载 Gravatar 头像",
                    Some("根据发件人邮箱哈希请求 Gravatar 公开头像"),
                    prefs.use_gravatar,
                    false,
                    is_hit(win, "pref-use-gravatar"),
                    |p, v| p.use_gravatar = v,
                    p,
                ),
                prefs_switch_row(
                    &app_entity,
                    "pref-sender-logos",
                    "加载公司徽标",
                    Some("按发件人域名展示对应的品牌或企业图标"),
                    prefs.sender_logos,
                    false,
                    is_hit(win, "pref-sender-logos"),
                    |p, v| p.sender_logos = v,
                    p,
                ),
            ],
            p,
        ))
        .child(setting_group(
            "已读回执",
            None,
            CARD_MAX_WIDTH,
            vec![setting_row(
                "pref-read-receipt",
                "发送已读回执",
                Some("当发件人请求阅读回执（Disposition-Notification-To）时的处理策略"),
                combo_button(
                    "cb-read-receipt",
                    lookup_label(
                        READ_RECEIPT_OPTIONS,
                        &prefs.read_receipt_policy.as_str(),
                        "每次询问",
                    ),
                    READ_RECEIPT_OPTIONS,
                    match prefs.read_receipt_policy.as_str() {
                        "always" => "always",
                        "never" => "never",
                        _ => "ask",
                    },
                    false,
                    move |val: &str, _, cx| {
                        a_receipt.update(cx, |app, cx| {
                            app.mutate_preferences(cx, |p| p.read_receipt_policy = val.to_string());
                        });
                    },
                ),
                false,
                is_hit(win, "pref-read-receipt"),
                p,
            )],
            p,
        ))
        .child(setting_group(
            "本地存储加密",
            Some(if hylki_mail_core::at_rest::is_sqlcipher_available() {
                "SQLCipher 4 加密后端已就绪，密钥安全存储于 Windows 凭据管理器"
            } else {
                "已接入密钥管理与状态检测；当前离线构建的 rusqlite 未启用 sqlcipher 特性，暂以明文 SQLite 运行"
            }),
            CARD_MAX_WIDTH,
            vec![
                prefs_switch_row(
                    &app_entity,
                    "pref-encrypt-stored-mail",
                    "加密本地存储的邮件",
                    Some("使用本地密钥（Windows 凭据管理器托管）保护缓存数据库"),
                    prefs.encrypt_stored_mail,
                    false,
                    is_hit(win, "pref-encrypt-stored-mail"),
                    |p, v| p.encrypt_stored_mail = v,
                    p,
                ),
                setting_row(
                    "pref-storage-password",
                    "SQLCipher 密钥状态",
                    Some(if hylki_mail_core::at_rest::is_sqlcipher_available() {
                        "SQLCipher 已启用，可管理本地存储密钥"
                    } else {
                        "密钥生成与 Windows 凭据管理器接口已就绪（等待 SQLCipher 运行时特性）"
                    }),
                    Button::new("btn-storage-password")
                        .outline()
                        .compact()
                        .label(if hylki_mail_core::at_rest::is_sqlcipher_available() {
                            "更改密码"
                        } else {
                            "检查密钥"
                        })
                        .disabled(!hylki_mail_core::at_rest::is_sqlcipher_available())
                        .tooltip("密钥托管于 Windows 凭据管理器")
                        .into_any_element(),
                    true,
                    false,
                    p,
                ),
            ],
            p,
        ))
        .into_any_element()
}

fn render_date_time(
    win: &SettingsWindow,
    app_entity: Entity<HylkiApp>,
    p: &HylkiPalette,
    cx: &mut Context<SettingsWindow>,
) -> AnyElement {
    let prefs = app_entity.read(cx).prefs.clone();
    let now_ts = chrono::Local::now().timestamp();
    let preview = hylki_mail_core::format_message_datetime(
        now_ts,
        &chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
        &prefs.date_style,
        &prefs.clock_style,
    );
    let a1 = app_entity.clone();
    let a2 = app_entity.clone();

    v_flex()
        .w_full()
        .max_w(px(CARD_MAX_WIDTH))
        .gap_5()
        .child(setting_group(
            "日期与时间格式",
            Some("实时影响邮件列表时间列与阅读器邮件头日期展示"),
            CARD_MAX_WIDTH,
            vec![
                info_row(
                    "pref-datetime-preview",
                    "当前格式实时预览",
                    preview,
                    false,
                    p,
                ),
                setting_row(
                    "pref-date-style",
                    "日期排列顺序",
                    None,
                    combo_button(
                        "cb-date-style",
                        lookup_label(
                            DATE_STYLE_OPTIONS,
                            &prefs.date_style.as_str(),
                            "跟随系统 (YYYY-MM-DD)",
                        ),
                        DATE_STYLE_OPTIONS,
                        match prefs.date_style.as_str() {
                            "month_first" => "month_first",
                            "day_first" => "day_first",
                            "year_first" => "year_first",
                            _ => "system",
                        },
                        false,
                        move |style: &str, _, cx| {
                            a1.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.date_style = style.to_string());
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-date-style"),
                    p,
                ),
                setting_row(
                    "pref-clock-style",
                    "时间制式",
                    None,
                    combo_button(
                        "cb-clock-style",
                        lookup_label(CLOCK_STYLE_OPTIONS, &prefs.clock_style.as_str(), "跟随系统"),
                        CLOCK_STYLE_OPTIONS,
                        match prefs.clock_style.as_str() {
                            "12h" => "12h",
                            "24h" => "24h",
                            _ => "system",
                        },
                        false,
                        move |style: &str, _, cx| {
                            a2.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.clock_style = style.to_string());
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-clock-style"),
                    p,
                ),
            ],
            p,
        ))
        .into_any_element()
}

fn render_system(
    win: &SettingsWindow,
    app_entity: Entity<HylkiApp>,
    p: &HylkiPalette,
    cx: &mut Context<SettingsWindow>,
) -> AnyElement {
    let demo_mode = app_entity.read(cx).demo_mode;
    let prefs = app_entity.read(cx).prefs.clone();
    let data_dir_str = config::data_dir().display().to_string();
    let stats = win
        .storage_stats
        .unwrap_or_else(|| config::storage_stats_in(&config::data_dir()));
    let storage_text = format!(
        "{}（{} 个文件）",
        config::format_bytes(stats.total_bytes),
        stats.file_count
    );
    let autostart_on = crate::autostart::is_autostart_enabled();
    let app_clear = app_entity.clone();
    let app_close = app_entity.clone();
    let app_min_tray = app_entity.clone();
    let app_fullscreen = app_entity.clone();
    let app_single_key = app_entity.clone();
    let app_status = app_entity.clone();
    let app_autostart = app_entity.clone();
    let app_keep = app_entity.clone();
    let app_links = app_entity.clone();
    let win_for_autostart = cx.entity().clone();

    let cache_action: AnyElement = if win.confirm_clear_cache {
        h_flex()
            .gap_2()
            .child(
                Button::new("confirm-clear-cache")
                    .primary()
                    .compact()
                    .label("确认清除")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.confirm_clear_cache = false;
                        this.storage_stats = None;
                        app_clear.update(cx, |app, cx| {
                            app.clear_body_caches(cx);
                        });
                        cx.notify();
                    })),
            )
            .child(
                Button::new("cancel-clear-cache")
                    .outline()
                    .compact()
                    .label("取消")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.confirm_clear_cache = false;
                        cx.notify();
                    })),
            )
            .into_any_element()
    } else {
        Button::new("btn-clear-cache")
            .outline()
            .compact()
            .disabled(demo_mode)
            .label("清除正文缓存")
            .on_click(cx.listener(|this, _, _, cx| {
                this.confirm_clear_cache = true;
                cx.notify();
            }))
            .into_any_element()
    };

    let mut stack = v_flex().w_full().max_w(px(CARD_MAX_WIDTH)).gap_5();

    if let Some((ok, ref msg)) = win.backup_feedback {
        stack = stack.child(
            div()
                .px_3()
                .py_2()
                .rounded(px(8.0))
                .bg(if ok {
                    p.accent.opacity(0.12)
                } else {
                    p.danger.opacity(0.12)
                })
                .text_xs()
                .text_color(if ok { p.accent } else { p.danger })
                .child(msg.clone()),
        );
    }

    stack
        .child(setting_group(
            "窗口、托盘与快捷键",
            None,
            CARD_MAX_WIDTH,
            vec![
                setting_row(
                    "pref-start-at-login",
                    "登录 Windows 时自动启动",
                    Some("读写 HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run 注册表项"),
                    bool_switch(
                        "sw-start-at-login",
                        autostart_on,
                        false,
                        move |val, _, cx| match crate::autostart::set_autostart_enabled(val) {
                            Ok(()) => {
                                app_autostart.update(cx, |app, cx| {
                                    app.mutate_preferences(cx, |p| p.start_at_login = val);
                                });
                            }
                            Err(err) => {
                                win_for_autostart.update(cx, |this, cx| {
                                    this.backup_feedback = Some((false, SharedString::from(err)));
                                    cx.notify();
                                });
                            }
                        },
                    ),
                    false,
                    is_hit(win, "pref-start-at-login"),
                    p,
                ),
                setting_row(
                    "pref-link-open-mode",
                    "邮件内链接打开方式",
                    Some("选择点击外部链接或退订页面时使用的浏览器"),
                    combo_button(
                        "cb-link-open-mode",
                        lookup_label(
                            LINK_OPEN_OPTIONS,
                            &prefs.link_open_mode.as_str(),
                            "系统默认浏览器",
                        ),
                        LINK_OPEN_OPTIONS,
                        match prefs.link_open_mode.as_str() {
                            "edge" => "edge",
                            "chrome" => "chrome",
                            "firefox" => "firefox",
                            _ => "system",
                        },
                        false,
                        move |val: &str, _, cx| {
                            app_links.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.link_open_mode = val.to_string());
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-link-open-mode"),
                    p,
                ),
                setting_row(
                    "pref-close-to-minimize",
                    "关闭主窗口时最小化到托盘",
                    Some("点击关闭按钮时不退出程序，保持系统托盘后台收取新邮件"),
                    bool_switch(
                        "sw-close-tray",
                        prefs.close_to_minimize,
                        false,
                        move |val, _, cx| {
                            app_close.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.close_to_minimize = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-close-to-minimize"),
                    p,
                ),
                setting_row(
                    "pref-minimize-to-tray",
                    "最小化主窗口时隐藏到托盘",
                    Some("最小化主窗口后从任务栏隐藏，可通过托盘图标单/双击恢复"),
                    bool_switch(
                        "sw-min-tray",
                        prefs.minimize_to_tray,
                        false,
                        move |val, _, cx| {
                            app_min_tray.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.minimize_to_tray = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-minimize-to-tray"),
                    p,
                ),
                setting_row(
                    "pref-start-fullscreen",
                    "启动时进入全屏模式",
                    Some("冷启动创建主窗口后自动切换全屏（可随时按 F11 还原）"),
                    bool_switch(
                        "sw-start-fullscreen",
                        prefs.start_fullscreen,
                        false,
                        move |val, _, cx| {
                            app_fullscreen.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.start_fullscreen = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-start-fullscreen"),
                    p,
                ),
                setting_row(
                    "pref-single-key-shortcuts",
                    "启用单键快捷键",
                    Some("在非输入焦点下响应 j/k/r/a/d/s/m（按 ? 查看速查）"),
                    bool_switch(
                        "sw-single-key",
                        prefs.single_key_shortcuts,
                        false,
                        move |val, _, cx| {
                            app_single_key.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.single_key_shortcuts = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-single-key-shortcuts"),
                    p,
                ),
                setting_row(
                    "pref-status-bar-top",
                    "将状态栏置于窗口顶部",
                    Some("将同步状态及快速提示条从窗口底部移至顶栏下方"),
                    bool_switch(
                        "sw-status-top",
                        prefs.status_bar_top,
                        false,
                        move |val, _, cx| {
                            app_status.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.status_bar_top = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-status-bar-top"),
                    p,
                ),
            ],
            p,
        ))
        .child(setting_group(
            "本地数据与缓存维护",
            None,
            CARD_MAX_WIDTH,
            vec![
                info_row(
                    "pref-data-dir",
                    "配置与缓存根目录",
                    data_dir_str,
                    is_hit(win, "pref-data-dir"),
                    p,
                ),
                info_row(
                    "pref-storage-stats",
                    "本地缓存与附件占用",
                    storage_text,
                    is_hit(win, "pref-storage-stats"),
                    p,
                ),
                setting_row(
                    "pref-keep-downloads",
                    "已下载邮件保留时长",
                    Some("控制本地 SQLite 缓存的邮件正文保留期限"),
                    combo_button(
                        "cb-keep-downloads",
                        lookup_label(
                            KEEP_DOWNLOADS_OPTIONS,
                            &prefs.keep_downloads_days,
                            "永久保留",
                        ),
                        KEEP_DOWNLOADS_OPTIONS,
                        prefs.keep_downloads_days,
                        false,
                        move |val: u32, _, cx| {
                            app_keep.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.keep_downloads_days = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-keep-downloads"),
                    p,
                ),
                info_row(
                    "pref-run-mode",
                    "运行模式",
                    if demo_mode {
                        "演示模式（内存模拟）"
                    } else {
                        "标准模式（SQLite + TOML）"
                    },
                    false,
                    p,
                ),
                setting_row(
                    "pref-clear-body-cache",
                    "清除本地邮件正文缓存",
                    Some("仅移除已缓存的 HTML / 纯文本正文，保留账号配置与邮件列表索引"),
                    cache_action,
                    demo_mode,
                    is_hit(win, "pref-clear-body-cache"),
                    p,
                ),
                setting_row(
                    "pref-export-log",
                    "导出诊断日志",
                    Some("导出脱敏后的环境、缓存统计与偏好摘要文本（不含密码与邮件正文）"),
                    Button::new("btn-export-log")
                        .outline()
                        .compact()
                        .icon(IconName::Download)
                        .label("导出日志")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.export_diagnostics_log(cx);
                        }))
                        .into_any_element(),
                    false,
                    is_hit(win, "pref-export-log"),
                    p,
                ),
            ],
            p,
        ))
        .child(setting_group(
            "关于 Hylki Windows",
            None,
            CARD_MAX_WIDTH,
            vec![
                info_row(
                    "pref-about-version",
                    "版本号",
                    format!("v{}", env!("CARGO_PKG_VERSION")),
                    is_hit(win, "pref-about-version"),
                    p,
                ),
                info_row(
                    "pref-about-engine",
                    "界面与通信内核",
                    "GPUI Kit + IMAP IDLE + Microsoft Graph OAuth2",
                    false,
                    p,
                ),
            ],
            p,
        ))
        .into_any_element()
}

fn render_backup(
    win: &SettingsWindow,
    app_entity: Entity<HylkiApp>,
    p: &HylkiPalette,
    _window: &mut Window,
    cx: &mut Context<SettingsWindow>,
) -> AnyElement {
    let app_apply = app_entity.clone();
    let mut stack = v_flex().w_full().max_w(px(CARD_MAX_WIDTH)).gap_5();

    if let Some((ok, ref msg)) = win.backup_feedback {
        stack = stack.child(
            div()
                .px_3()
                .py_2()
                .rounded(px(8.0))
                .bg(if ok {
                    p.accent.opacity(0.12)
                } else {
                    p.danger.opacity(0.12)
                })
                .text_xs()
                .text_color(if ok { p.accent } else { p.danger })
                .child(msg.clone()),
        );
    }

    stack = stack
        .child(setting_group(
            "导出配置备份",
            Some("生成可移植的 TOML 备份文件（仅包含偏好设置与账号服务器参数；不包含密码、OAuth 令牌和本地邮件）"),
            CARD_MAX_WIDTH,
            vec![setting_row(
                "pref-backup-export",
                "导出当前偏好与账号配置",
                Some("保存为 .toml 文件以便迁移或重装后快速恢复"),
                Button::new("btn-backup-export")
                    .primary()
                    .compact()
                    .icon(IconName::Download)
                    .label("导出备份...")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.export_settings_backup(cx);
                    }))
                    .into_any_element(),
                false,
                is_hit(win, "pref-backup-export"),
                p,
            )],
            p,
        ))
        .child(setting_group(
            "导入与恢复配置",
            Some("选择先前导出的 .toml 备份文件，预览内容后确认写回配置文件并立即应用"),
            CARD_MAX_WIDTH,
            vec![setting_row(
                "pref-backup-import",
                "从备份文件恢复",
                Some("不会覆盖现有 Windows 凭据管理器中的密码或令牌"),
                Button::new("btn-backup-pick")
                    .outline()
                    .compact()
                    .icon(IconName::Upload)
                    .label("选择备份文件...")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.pick_settings_backup(cx);
                    }))
                    .into_any_element(),
                false,
                is_hit(win, "pref-backup-import"),
                p,
            )],
            p,
        ));

    if let Some((ref path, ref bundle)) = win.pending_backup_import {
        stack = stack.child(setting_group(
            "待确认恢复的备份预览",
            Some("确认后将立即覆盖偏好设置与账号清单，并重启同步相关配置"),
            CARD_MAX_WIDTH,
            vec![
                info_row("bk-path", "源文件", path.display().to_string(), false, p),
                info_row(
                    "bk-acc",
                    "包含邮箱账号",
                    format!("{} 个", bundle.accounts.len()),
                    false,
                    p,
                ),
                info_row(
                    "bk-theme",
                    "主题方案",
                    bundle.preferences.theme_id.clone(),
                    false,
                    p,
                ),
                h_flex()
                    .px_4()
                    .py_3()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("bk-cancel")
                            .outline()
                            .compact()
                            .label("取消")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.pending_backup_import = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("bk-confirm")
                            .primary()
                            .compact()
                            .label("确认导入并生效")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.confirm_import_backup(&app_apply, window, cx);
                            })),
                    )
                    .into_any_element(),
            ],
            p,
        ));
    }

    stack.into_any_element()
}
