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
use hylki_mail_core::config::{self, DEFAULT_ATTACHMENT_KEYWORDS, UI_TEXT_SCALE_STEPS};

use crate::app::HylkiApp;
use crate::theme::HylkiPalette;
use crate::ui::preferences::widgets::{
    arm_search_scroll_anchor, bool_switch, combo_button, combo_button_dynamic, info_row,
    search_scroll_anchor_consumed, setting_group, setting_row, take_search_scroll_pending,
    theme_gallery, CARDS_WIDE_MAX_WIDTH, CARD_MAX_WIDTH,
};
use crate::ui::preferences::{PrefTab, SettingsWindow};

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
    ("80%", 80),
    ("90%", 90),
    ("100%（默认）", 100),
    ("110%", 110),
    ("125%", 125),
    ("150%", 150),
];

const FOLD_MODE_OPTIONS: &[(&str, &str)] = &[
    ("折叠较早邮件", "all_but_newest"),
    ("仅折叠已读邮件", "read"),
    ("全部展开（不折叠）", "never"),
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
    ("15 秒", 15),
];

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
        PrefTab::Senders => render_senders(win, app_entity, p, cx),
        PrefTab::General => render_general(win, app_entity, p, cx),
        PrefTab::Appearance => render_appearance(win, app_entity, p, cx),
        PrefTab::FocusMode => render_focus_mode(win, app_entity, p, cx),
        PrefTab::Sidebar => render_sidebar(win, app_entity, p, cx),
        PrefTab::MessageList => render_message_list(win, app_entity, p, cx),
        PrefTab::Conversations => render_conversations(win, app_entity, p, cx),
        PrefTab::Reading => render_reading(win, app_entity, p, cx),
        PrefTab::Composing => render_composing(win, app_entity, p, cx),
        PrefTab::Privacy => render_privacy(win, app_entity, p, cx),
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
    let mut sorted_senders = prefs.allowed_remote_senders.clone();
    sorted_senders.sort();
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
                .child(div().text_xs().text_color(p.text_muted).child(
                    "白名单暂无发件人；阅读邮件点击「始终允许来自此发件人」也会自动收录到此处。",
                ))
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

    let mut stack = v_flex().w_full().max_w(px(CARDS_WIDE_MAX_WIDTH)).gap_3();

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
            format!(
                "远程图像发件人白名单（{}）",
                prefs.allowed_remote_senders.len()
            ),
            Some("列出的邮箱地址将在阅读 HTML 邮件时自动放行外链图片"),
            CARDS_WIDE_MAX_WIDTH,
            rows,
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
    let app6 = app_entity.clone();
    let app7 = app_entity.clone();
    let app8 = app_entity.clone();
    let app9 = app_entity.clone();
    let app10 = app_entity.clone();
    let app11 = app_entity.clone();

    v_flex()
        .w_full()
        .max_w(px(CARD_MAX_WIDTH))
        .gap_5()
        .child(setting_group(
            "窗口与系统托盘",
            None,
            CARD_MAX_WIDTH,
            vec![
                setting_row(
                    "pref-close-to-minimize",
                    "关闭主窗口时最小化到托盘",
                    Some("点击关闭按钮时不退出程序，保持系统托盘后台收取新邮件"),
                    bool_switch(
                        "sw-close-tray",
                        prefs.close_to_minimize,
                        false,
                        move |val, _, cx| {
                            app1.update(cx, |app, cx| {
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
                            app2.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.minimize_to_tray = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-minimize-to-tray"),
                    p,
                ),
            ],
            p,
        ))
        .child(setting_group(
            "启动与分页",
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
                            app3.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.default_view = val.to_string());
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-default-view"),
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
                            app4.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.start_fullscreen = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-start-fullscreen"),
                    p,
                ),
                setting_row(
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
                            app5.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.page_size = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-page-size"),
                    p,
                ),
            ],
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
                            app6.update(cx, |app, cx| {
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
                            app7.update(cx, |app, cx| {
                                app.set_push_enabled(val, cx);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-push-enabled"),
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
                            app8.update(cx, |app, cx| {
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
                            app9.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.notification_show_content = val);
                            });
                        },
                    ),
                    !prefs.desktop_notifications,
                    is_hit(win, "pref-notification-content"),
                    p,
                ),
            ],
            p,
        ))
        .child(setting_group(
            "快捷操作",
            None,
            CARD_MAX_WIDTH,
            vec![
                setting_row(
                    "pref-single-key-shortcuts",
                    "启用单键快捷键",
                    Some("在非输入焦点下响应 j/k/r/a/d/s/m（按 ? 查看速查）"),
                    bool_switch(
                        "sw-single-key",
                        prefs.single_key_shortcuts,
                        false,
                        move |val, _, cx| {
                            app10.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.single_key_shortcuts = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-single-key-shortcuts"),
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
                            app11.update(cx, |app, cx| {
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
    let app_zoom = app_entity.clone();
    let app_status = app_entity.clone();

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
            app_entity,
            p,
            cx,
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
                            app_zoom.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.reader_zoom_percent = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-reader-zoom"),
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
        .into_any_element()
}

fn render_focus_mode(
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

    v_flex()
        .w_full()
        .max_w(px(CARD_MAX_WIDTH))
        .gap_5()
        .child(setting_group(
            "专注模式行为",
            Some("随时按 Ctrl+Shift+F 或阅读器顶栏专注图标切换沉浸阅读状态"),
            CARD_MAX_WIDTH,
            vec![
                setting_row(
                    "pref-focus-start",
                    "启动时自动进入专注模式",
                    None,
                    bool_switch(
                        "sw-focus-start",
                        prefs.focus_start_enabled,
                        false,
                        move |val, _, cx| {
                            a1.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.focus_start_enabled = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-focus-start"),
                    p,
                ),
                setting_row(
                    "pref-focus-hide-sidebar",
                    "专注模式下隐藏左侧边栏",
                    None,
                    bool_switch(
                        "sw-focus-sb",
                        prefs.focus_hide_sidebar,
                        false,
                        move |val, _, cx| {
                            a2.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.focus_hide_sidebar = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-focus-hide-sidebar"),
                    p,
                ),
                setting_row(
                    "pref-focus-hide-list",
                    "专注模式下隐藏中间邮件列表",
                    None,
                    bool_switch(
                        "sw-focus-list",
                        prefs.focus_hide_list,
                        false,
                        move |val, _, cx| {
                            a3.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.focus_hide_list = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-focus-hide-list"),
                    p,
                ),
                setting_row(
                    "pref-focus-reader-view",
                    "专注模式下自动开启简化阅读视图",
                    Some("进入专注模式时切至纯文本精简视图，退出专注模式自动恢复"),
                    bool_switch(
                        "sw-focus-rv",
                        prefs.focus_reader_view,
                        false,
                        move |val, _, cx| {
                            a4.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.focus_reader_view = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-focus-reader-view"),
                    p,
                ),
                setting_row(
                    "pref-focus-fold-toolbar",
                    "专注模式下折叠阅读器右侧工具栏",
                    Some("将低频辅助操作收入「更多」弹出菜单以减少按钮干扰"),
                    bool_switch(
                        "sw-focus-ft",
                        prefs.focus_fold_toolbar,
                        false,
                        move |val, _, cx| {
                            a5.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.focus_fold_toolbar = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-focus-fold-toolbar"),
                    p,
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

    v_flex()
        .w_full()
        .max_w(px(CARD_MAX_WIDTH))
        .gap_5()
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
    let a_fold = app_entity.clone();

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
                setting_row(
                    "pref-reader-fold-right",
                    "始终折叠右侧工具栏辅助按钮",
                    Some("将已读切换、原文预览、导出 EML、一键退订等收敛至更多菜单"),
                    bool_switch(
                        "sw-rd-fold",
                        prefs.reader_toolbar_fold_right,
                        false,
                        move |val, _, cx| {
                            a_fold.update(cx, |app, cx| {
                                app.mutate_preferences(cx, |p| p.reader_toolbar_fold_right = val);
                            });
                        },
                    ),
                    false,
                    is_hit(win, "pref-reader-fold-right"),
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
                    "pref-signature-above-quote",
                    "回复与转发时签名放在引用正文上方",
                    Some("推荐在长对话中让新写正文与签名紧靠在一起"),
                    bool_switch("sw-sig-above", prefs.signature_above_quote, false, move |val, _, cx| {
                        a_above.update(cx, |app, cx| {
                            app.mutate_preferences(cx, |p| p.signature_above_quote = val);
                        });
                    }),
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
    let data_dir_str = config::data_dir().display().to_string();
    let app_clear = app_entity.clone();

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

    v_flex()
        .w_full()
        .max_w(px(CARD_MAX_WIDTH))
        .gap_5()
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
