//! 统一设置页（对齐原版 Linux Settings 窗口）：
//! - 左分类栏 200px，分「账号」与「设置」两大分组：
//!   - 账号：邮件账号
//!   - 设置：常规 / 外观 / 专注模式 / 侧边栏 / 邮件列表 / 会话 / 阅读 / 数据与存储
//! - 右内容区 max-width 600px（邮件账号页复用 `accounts::render_content`）。
//!
//! `PreferencesState` 内部持有 [`AppPreferences`] 副本，任何改动都更新副本并发
//! [`PreferencesEvent::Changed`]（携带完整新值）；**不**直接修改全局主题，
//! 由集成者负责应用与持久化。

use std::path::PathBuf;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::Icon;
use gpui_kit::component::{h_flex, v_flex, Selectable, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    div, px, Context, EventEmitter, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled,
};
use hylki_mail_core::config::{self, AppPreferences};

use crate::app::HylkiApp;
use crate::theme;
use crate::ui::accounts;

/// 左分类栏宽度。
const CATEGORY_WIDTH: f32 = 200.0;
/// 分类行高度。
const CATEGORY_ROW_HEIGHT: f32 = 34.0;
/// 顶部栏高度。
const HEADER_HEIGHT: f32 = 48.0;
/// 设置卡片最大宽度。
const CARD_MAX_WIDTH: f32 = 600.0;

/// 设置分类（左栏可切换，对齐原版 Settings 左侧分组）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PrefTab {
    MailAccounts,
    General,
    Appearance,
    FocusMode,
    Sidebar,
    MessageList,
    Conversations,
    Reading,
    Storage,
}

impl PrefTab {
    const GROUPS: [(&'static str, &'static [PrefTab]); 2] = [
        ("账号", &[PrefTab::MailAccounts]),
        (
            "设置",
            &[
                PrefTab::General,
                PrefTab::Appearance,
                PrefTab::FocusMode,
                PrefTab::Sidebar,
                PrefTab::MessageList,
                PrefTab::Conversations,
                PrefTab::Reading,
                PrefTab::Storage,
            ],
        ),
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            PrefTab::MailAccounts => "邮件账号",
            PrefTab::General => "常规",
            PrefTab::Appearance => "外观",
            PrefTab::FocusMode => "专注模式",
            PrefTab::Sidebar => "侧边栏",
            PrefTab::MessageList => "邮件列表",
            PrefTab::Conversations => "会话",
            PrefTab::Reading => "阅读",
            PrefTab::Storage => "数据与存储",
        }
    }

    fn icon(self) -> IconName {
        match self {
            PrefTab::MailAccounts => IconName::CircleUser,
            PrefTab::General => IconName::Settings,
            PrefTab::Appearance => IconName::Palette,
            PrefTab::FocusMode => IconName::Eye,
            PrefTab::Sidebar => IconName::PanelLeft,
            PrefTab::MessageList => IconName::List,
            PrefTab::Conversations => IconName::MailOpen,
            PrefTab::Reading => IconName::BookOpen,
            PrefTab::Storage => IconName::HardDrive,
        }
    }
}

/// 偏好设置事件。
#[derive(Debug, Clone, PartialEq)]
pub enum PreferencesEvent {
    /// 任一持久化设置改动（携带完整新值，集成者负责应用与持久化）。
    Changed(AppPreferences),
    /// 用户点击「清理正文缓存」（保留账号与配置）。
    ClearBodyCache,
    /// 会话级开关：专注模式。
    SetFocusMode(bool),
    /// 会话级开关：侧栏显示账号列表。
    SetShowAccounts(bool),
}

/// 偏好设置视图状态（独立 Entity，由 HylkiApp 挂载）。
pub struct PreferencesState {
    /// 内部持有的偏好副本（每次改动后整体发出）。
    prefs: AppPreferences,
    /// 当前展示的分类。
    active_tab: PrefTab,
    /// 草稿数量（由集成者通过 [`Self::set_storage_stats`] 填充）。
    drafts: usize,
    /// 发件箱待处理数量（同上）。
    outbox: usize,
    /// 会话级：侧栏是否显示账号列表。
    show_accounts: bool,
    /// 会话级：专注模式是否开启。
    focus_mode: bool,
    /// 数据目录（构造时确定）。
    data_dir: PathBuf,
}

impl EventEmitter<PreferencesEvent> for PreferencesState {}

impl PreferencesState {
    pub fn new(prefs: AppPreferences) -> Self {
        Self {
            prefs,
            active_tab: PrefTab::General,
            drafts: 0,
            outbox: 0,
            show_accounts: true,
            focus_mode: false,
            data_dir: config::data_dir(),
        }
    }

    pub(crate) fn active_tab(&self) -> PrefTab {
        self.active_tab
    }

    pub(crate) fn set_active_tab(&mut self, tab: PrefTab, cx: &mut Context<Self>) {
        if self.active_tab != tab {
            self.active_tab = tab;
            cx.notify();
        }
    }

    /// 同步会话级布局状态（打开设置时从 HylkiApp 注入）。
    pub fn sync_session_flags(&mut self, show_accounts: bool, focus_mode: bool) {
        self.show_accounts = show_accounts;
        self.focus_mode = focus_mode;
    }

    /// 填充草稿 / 发件箱数量统计（数据与存储页展示）。
    pub fn set_storage_stats(&mut self, drafts: usize, outbox: usize) {
        self.drafts = drafts;
        self.outbox = outbox;
    }

    /// 当前偏好副本（供集成者在收到 Changed 前读取；当前集成链路由
    /// HylkiApp 持有偏好权威副本，此 getter 保留作组件语义）。
    #[allow(dead_code)]
    pub fn preferences(&self) -> &AppPreferences {
        &self.prefs
    }

    /// 更新副本并发 Changed(完整新值)。
    fn emit_changed(&mut self, cx: &mut Context<Self>) {
        let prefs = self.prefs.clone();
        cx.emit(PreferencesEvent::Changed(prefs));
        cx.notify();
    }

    fn set_default_view(&mut self, value: &str, cx: &mut Context<Self>) {
        if self.prefs.default_view != value {
            self.prefs.default_view = value.to_string();
            self.emit_changed(cx);
        }
    }

    fn set_auto_sync_minutes(&mut self, minutes: u32, cx: &mut Context<Self>) {
        if self.prefs.auto_sync_minutes != minutes {
            self.prefs.auto_sync_minutes = minutes;
            self.emit_changed(cx);
        }
    }

    fn set_dark_mode(&mut self, dark: Option<bool>, cx: &mut Context<Self>) {
        if self.prefs.dark_mode != dark {
            self.prefs.dark_mode = dark;
            self.emit_changed(cx);
        }
    }

    fn set_theme_id(&mut self, theme_id: &str, cx: &mut Context<Self>) {
        if self.prefs.theme_id != theme_id {
            self.prefs.theme_id = theme_id.to_string();
            self.emit_changed(cx);
        }
    }

    fn set_compact_list(&mut self, compact: bool, cx: &mut Context<Self>) {
        if self.prefs.compact_list != compact {
            self.prefs.compact_list = compact;
            self.emit_changed(cx);
        }
    }

    fn set_page_size(&mut self, size: usize, cx: &mut Context<Self>) {
        if self.prefs.page_size != size {
            self.prefs.page_size = size;
            self.emit_changed(cx);
        }
    }

    fn set_thread_newest_first(&mut self, newest_first: bool, cx: &mut Context<Self>) {
        if self.prefs.thread_newest_first != newest_first {
            self.prefs.thread_newest_first = newest_first;
            self.emit_changed(cx);
        }
    }

    fn set_reader_zoom_percent(&mut self, zoom: u32, cx: &mut Context<Self>) {
        let clamped = config::clamp_reader_zoom(zoom);
        if self.prefs.reader_zoom_percent != clamped {
            self.prefs.reader_zoom_percent = clamped;
            self.emit_changed(cx);
        }
    }

    fn set_reader_position(&mut self, position: &'static str, cx: &mut Context<Self>) {
        if self.prefs.reader_position != position {
            self.prefs.reader_position = position.to_string();
            self.emit_changed(cx);
        }
    }
}

/// 亮暗模式选项标签（渲染走 option_pills 内联文案；此助手仅单元测试引用）。
#[allow(dead_code)]
fn dark_mode_label(dark: Option<bool>) -> &'static str {
    match dark {
        Some(false) => "亮色",
        Some(true) => "暗色",
        None => "跟随系统",
    }
}

/// 启动视图选项标签（同上，仅单元测试引用）。
#[allow(dead_code)]
fn default_view_label(view: &str) -> &'static str {
    match view {
        "last" => "上次文件夹",
        _ => "统一收件箱",
    }
}

/// 自动刷新间隔选项标签（同上，仅单元测试引用）。
#[allow(dead_code)]
fn sync_interval_label(minutes: u32) -> &'static str {
    match minutes {
        1 => "1 分钟",
        5 => "5 分钟",
        15 => "15 分钟",
        _ => "手动",
    }
}

/// 设置卡片：标题 + 若干行。
fn setting_card(
    title: &str,
    p: &theme::HylkiPalette,
    rows: Vec<gpui_kit::AnyElement>,
) -> gpui_kit::AnyElement {
    let (surface_raised, text, border) = (p.surface_raised, p.text, p.border);
    v_flex()
        .w_full()
        .max_w(px(CARD_MAX_WIDTH))
        .gap_3()
        .px_3()
        .py_3()
        .bg(surface_raised)
        .rounded(px(12.0))
        .border_1()
        .border_color(border)
        .child(
            div()
                .text_sm()
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .text_color(text)
                .child(SharedString::from(title)),
        )
        .children(rows)
        .into_any_element()
}

/// 单行设置项：左侧标签（+ 可选说明）与右侧控件。
fn setting_row(
    label: &str,
    desc: Option<&str>,
    control: impl IntoElement,
    p: &theme::HylkiPalette,
) -> gpui_kit::AnyElement {
    let (text, muted_fg) = (p.text, p.text_muted);
    h_flex()
        .items_center()
        .justify_between()
        .gap_2()
        .child(
            v_flex()
                .min_w_0()
                .gap_0p5()
                .child(
                    div()
                        .text_sm()
                        .text_color(text)
                        .child(SharedString::from(label)),
                )
                .when_some(desc, |el, d| {
                    el.child(
                        div()
                            .text_xs()
                            .text_color(muted_fg)
                            .child(SharedString::from(d)),
                    )
                }),
        )
        .child(control)
        .into_any_element()
}

/// 选项胶囊组（单选）：当前值高亮。
fn option_pills<T: Copy + PartialEq + 'static>(
    id_prefix: &'static str,
    options: &[(T, &'static str)],
    current: T,
    cx: &mut Context<PreferencesState>,
    on_select: fn(&mut PreferencesState, T, &mut Context<PreferencesState>),
) -> gpui_kit::AnyElement {
    h_flex()
        .gap_1()
        .children(options.iter().enumerate().map(|(ix, (value, label))| {
            let value = *value;
            let is_selected = value == current;
            Button::new((id_prefix, ix))
                .ghost()
                .compact()
                .selected(is_selected)
                .label(*label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    on_select(this, value, cx);
                }))
        }))
        .into_any_element()
}

impl Render for PreferencesState {
    fn render(
        &mut self,
        _window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // 避坑清单第 4 条：render 开头先把语义色 Copy 成局部变量。
        let p = theme::current(cx);
        let (border, text, muted_fg, accent) = (p.border, p.text, p.text_muted, p.accent);

        let active_tab = self.active_tab;
        let prefs = self.prefs.clone();

        // 按分类渲染右侧设置卡片（邮件账号页由 render_settings_modal 直接挂载 accounts::render_content）。
        let content: gpui_kit::AnyElement = match active_tab {
            PrefTab::MailAccounts => div().into_any_element(),
            PrefTab::General => {
                let default_view_pills = option_pills(
                    "pref-default-view",
                    &[("unified", "统一收件箱"), ("last", "上次文件夹")],
                    if prefs.default_view == "last" {
                        "last"
                    } else {
                        "unified"
                    },
                    cx,
                    PreferencesState::set_default_view,
                );
                let sync_pills = option_pills(
                    "pref-sync",
                    &[(0u32, "手动"), (1, "1 分钟"), (5, "5 分钟"), (15, "15 分钟")],
                    prefs.auto_sync_minutes,
                    cx,
                    PreferencesState::set_auto_sync_minutes,
                );
                let entity_for_close = cx.entity();
                let close_minimize_switch = Switch::new("pref-close-to-minimize")
                    .checked(prefs.close_to_minimize)
                    .on_click(move |checked, _, cx| {
                        entity_for_close.update(cx, |this, cx| {
                            this.prefs.close_to_minimize = *checked;
                            this.emit_changed(cx);
                        });
                    });
                let entity_for_min_tray = cx.entity();
                let min_to_tray_switch = Switch::new("pref-minimize-to-tray")
                    .checked(prefs.minimize_to_tray)
                    .on_click(move |checked, _, cx| {
                        entity_for_min_tray.update(cx, |this, cx| {
                            this.prefs.minimize_to_tray = *checked;
                            this.emit_changed(cx);
                        });
                    });
                let entity_for_notify = cx.entity();
                let desktop_notify_switch = Switch::new("pref-desktop-notifications")
                    .checked(prefs.desktop_notifications)
                    .on_click(move |checked, _, cx| {
                        entity_for_notify.update(cx, |this, cx| {
                            this.prefs.desktop_notifications = *checked;
                            this.emit_changed(cx);
                        });
                    });
                // F-1：单键快捷键总开关（关闭后 j / r / a 等无修饰键快捷键停用）。
                let entity_for_single_key = cx.entity();
                let single_key_switch = Switch::new("pref-single-key-shortcuts")
                    .checked(prefs.single_key_shortcuts)
                    .on_click(move |checked, _, cx| {
                        entity_for_single_key.update(cx, |this, cx| {
                            this.prefs.single_key_shortcuts = *checked;
                            this.emit_changed(cx);
                        });
                    });
                setting_card(
                    "常规",
                    &p,
                    vec![
                        setting_row(
                            "默认启动视图",
                            Some("启动 Hylki 时展示统一收件箱或上次浏览的文件夹"),
                            default_view_pills,
                            &p,
                        ),
                        setting_row(
                            "后台自动刷新间隔",
                            Some("按间隔在后台同步所有账号；「手动」仅手动刷新"),
                            sync_pills,
                            &p,
                        ),
                        setting_row(
                            "点击关闭按钮时最小化到托盘",
                            Some("关闭窗口时隐藏到系统托盘，可在菜单或托盘右键选择「退出 Hylki」"),
                            close_minimize_switch,
                            &p,
                        ),
                        setting_row(
                            "最小化时隐藏到托盘",
                            Some("最小化窗口时隐藏到系统托盘，不在任务栏保留图标"),
                            min_to_tray_switch,
                            &p,
                        ),
                        setting_row(
                            "新邮件系统通知",
                            Some("后台同步收到新未读邮件时通过系统托盘弹出桌面通知"),
                            desktop_notify_switch,
                            &p,
                        ),
                        setting_row(
                            "启用单键快捷键",
                            Some("关闭后 j / k / r / a 等无修饰键快捷键不再生效，Ctrl 组合键不受影响"),
                            single_key_switch,
                            &p,
                        ),
                    ],
                )
            }
            PrefTab::Appearance => {
                // 主题色板选择：全部内置主题，圆点显示该主题 accent 色。
                let dark_for_swatch = prefs.dark_mode.unwrap_or(false);
                let swatches = h_flex().flex_wrap().gap_2().children(
                    theme::THEME_LIST
                        .iter()
                        .enumerate()
                        .map(|(ix, (theme_id, theme_label))| {
                            let theme_id = *theme_id;
                            let is_selected = prefs.theme_id == theme_id;
                            let swatch = theme::palette_for(theme_id, dark_for_swatch).accent;
                            h_flex()
                                .id(("pref-theme", ix))
                                .px_2()
                                .py_1()
                                .gap_1p5()
                                .items_center()
                                .rounded(px(8.0))
                                .cursor_pointer()
                                .border_1()
                                .border_color(if is_selected { accent } else { border })
                                .when(is_selected, |el| el.bg(p.accent_soft()))
                                .when(!is_selected, |el| el.hover(|s| s.bg(p.accent_soft())))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.set_theme_id(theme_id, cx);
                                }))
                                .child(
                                    div()
                                        .w(px(14.0))
                                        .h(px(14.0))
                                        .rounded_full()
                                        .bg(swatch)
                                        .border_1()
                                        .border_color(border),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(if is_selected { accent } else { text })
                                        .child(SharedString::from(*theme_label)),
                                )
                        }),
                );
                let mode_pills = option_pills(
                    "pref-dark-mode",
                    &[
                        (Some(false), "亮色"),
                        (Some(true), "暗色"),
                        (None, "跟随系统"),
                    ],
                    prefs.dark_mode,
                    cx,
                    PreferencesState::set_dark_mode,
                );
                v_flex()
                    .w_full()
                    .max_w(px(CARD_MAX_WIDTH))
                    .gap_4()
                    .child(setting_card(
                        "外观",
                        &p,
                        vec![
                            setting_row(
                                "界面模式",
                                Some("亮色 / 暗色，或跟随系统"),
                                mode_pills,
                                &p,
                            ),
                            setting_row("主题色板", Some("选择内置主题配色"), swatches, &p),
                        ],
                    ))
                    .into_any_element()
            }
            PrefTab::FocusMode => {
                let entity_focus = cx.entity();
                let focus_switch = Switch::new("pref-focus-mode")
                    .checked(self.focus_mode)
                    .on_click(move |checked, _, cx| {
                        entity_focus.update(cx, |this, cx| {
                            this.focus_mode = *checked;
                            cx.emit(PreferencesEvent::SetFocusMode(*checked));
                            cx.notify();
                        });
                    });
                setting_card(
                    "专注模式",
                    &p,
                    vec![setting_row(
                        "启用专注模式",
                        Some("隐藏左侧边栏与邮件列表，仅保留阅读器（快捷键 Ctrl+Shift+F）"),
                        focus_switch,
                        &p,
                    )],
                )
            }
            PrefTab::Sidebar => {
                let entity_accounts = cx.entity();
                let accounts_switch = Switch::new("pref-show-accounts")
                    .checked(self.show_accounts)
                    .on_click(move |checked, _, cx| {
                        entity_accounts.update(cx, |this, cx| {
                            this.show_accounts = *checked;
                            cx.emit(PreferencesEvent::SetShowAccounts(*checked));
                            cx.notify();
                        });
                    });
                setting_card(
                    "侧边栏",
                    &p,
                    vec![setting_row(
                        "在侧边栏显示账号列表",
                        Some("控制左侧边栏是否展开各邮箱账号与文件夹（快捷键 Ctrl+Shift+A）"),
                        accounts_switch,
                        &p,
                    )],
                )
            }
            PrefTab::MessageList => {
                let density_pills = option_pills(
                    "pref-density",
                    &[(false, "标准"), (true, "紧凑")],
                    prefs.compact_list,
                    cx,
                    PreferencesState::set_compact_list,
                );
                let page_size_pills = option_pills(
                    "pref-page-size",
                    &[(50usize, "50 条"), (100, "100 条"), (200, "200 条")],
                    prefs.page_size,
                    cx,
                    PreferencesState::set_page_size,
                );
                setting_card(
                    "邮件列表",
                    &p,
                    vec![
                        setting_row(
                            "列表密度",
                            Some("紧凑模式显示更多邮件行"),
                            density_pills,
                            &p,
                        ),
                        setting_row(
                            "每页加载数量",
                            Some("邮件列表一次从缓存加载的邮件数"),
                            page_size_pills,
                            &p,
                        ),
                    ],
                )
            }
            PrefTab::Conversations => {
                let thread_order_pills = option_pills(
                    "pref-thread-order",
                    &[(true, "最新在前（降序）"), (false, "最早在前（升序）")],
                    prefs.thread_newest_first,
                    cx,
                    PreferencesState::set_thread_newest_first,
                );
                let entity_fold = cx.entity();
                let fold_switch = Switch::new("pref-fold-older-messages")
                    .checked(prefs.fold_older_messages)
                    .on_click(move |checked, _, cx| {
                        entity_fold.update(cx, |this, cx| {
                            this.prefs.fold_older_messages = *checked;
                            this.emit_changed(cx);
                        });
                    });
                setting_card(
                    "会话",
                    &p,
                    vec![
                        setting_row(
                            "会话邮件排序",
                            Some("选择多封往来会话在阅读器中的排列顺序"),
                            thread_order_pills,
                            &p,
                        ),
                        setting_row(
                            "自动折叠较早会话邮件",
                            Some("打开多封往来会话时，默认折叠较早的邮件卡片"),
                            fold_switch,
                            &p,
                        ),
                    ],
                )
            }
            PrefTab::Reading => {
                let reader_pos_current = match prefs.reader_position.as_str() {
                    "below" => "below",
                    "off" => "off",
                    _ => "right",
                };
                let reader_pos_pills = option_pills(
                    "pref-reader-pos",
                    &[("right", "右侧"), ("below", "下方"), ("off", "关闭")],
                    reader_pos_current,
                    cx,
                    PreferencesState::set_reader_position,
                );
                let zoom_pills = option_pills(
                    "pref-reader-zoom",
                    &[(90u32, "90%"), (100, "100%"), (110, "110%"), (125, "125%")],
                    prefs.reader_zoom_percent,
                    cx,
                    PreferencesState::set_reader_zoom_percent,
                );
                let entity = cx.entity();
                let remote_switch = Switch::new("pref-block-remote-images")
                    .checked(prefs.block_remote_images)
                    .on_click(move |checked, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.prefs.block_remote_images = *checked;
                            this.emit_changed(cx);
                        });
                    });
                setting_card(
                    "阅读",
                    &p,
                    vec![
                        setting_row(
                            "阅读器位置",
                            Some("选择阅读窗格相对邮件列表的停靠位置（Ctrl+Alt+1/2/3）"),
                            reader_pos_pills,
                            &p,
                        ),
                        setting_row(
                            "默认阅读字号",
                            Some("调整阅读器正文默认缩放比例"),
                            zoom_pills,
                            &p,
                        ),
                        setting_row(
                            "远程图片默认关闭",
                            Some("HTML 邮件默认不加载远程图片，保护隐私"),
                            remote_switch,
                            &p,
                        ),
                    ],
                )
            }
            PrefTab::Storage => {
                // 缓存数据库大小：不存在则显示「未创建」。
                let cache_size_text = match std::fs::metadata(config::cache_db_in(&self.data_dir)) {
                    Ok(meta) => format_size(meta.len() as usize),
                    Err(_) => "未创建".to_string(),
                };
                let info_row = |label: &str, value: String| {
                    h_flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .text_sm()
                                .text_color(text)
                                .child(SharedString::from(label)),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted_fg)
                                .max_w(px(360.0))
                                .truncate()
                                .child(SharedString::from(value)),
                        )
                        .into_any_element()
                };
                setting_card(
                    "数据与存储",
                    &p,
                    vec![
                        info_row("数据目录", self.data_dir.display().to_string()),
                        info_row("SQLite 缓存文件大小", cache_size_text),
                        info_row("本地草稿数量", self.drafts.to_string()),
                        info_row("发件箱待处理数量", self.outbox.to_string()),
                        setting_row(
                            "正文缓存",
                            Some("清理已缓存的邮件正文，保留账号与配置"),
                            Button::new("pref-clear-body-cache")
                                .danger()
                                .label("清理正文缓存")
                                .on_click(cx.listener(|_, _, _, cx| {
                                    cx.emit(PreferencesEvent::ClearBodyCache);
                                    cx.notify();
                                })),
                            &p,
                        ),
                    ],
                )
            }
        };

        v_flex()
            .id("prefs-content")
            .size_full()
            .overflow_y_scrollbar()
            .px_6()
            .py_4()
            .gap_4()
            .child(content)
    }
}

/// 统一「设置」窗口外壳（左分组导航 + 右标题栏 + 邮件账号或偏好内容）。
pub(crate) fn render_settings_modal(
    app: &HylkiApp,
    cx: &mut Context<HylkiApp>,
) -> gpui_kit::AnyElement {
    let p = theme::current(cx);
    let (canvas, sidebar_bg, text, muted_fg, border, accent) = (
        p.canvas,
        p.sidebar,
        p.text,
        p.text_muted,
        p.border,
        p.accent,
    );

    let active_tab = app
        .preferences
        .as_ref()
        .map(|e| e.read(cx).active_tab())
        .unwrap_or(PrefTab::General);

    let sidebar = v_flex()
        .w(px(CATEGORY_WIDTH))
        .h_full()
        .flex_shrink_0()
        .bg(sidebar_bg)
        .border_r_1()
        .border_color(border)
        .child(
            h_flex()
                .h(px(HEADER_HEIGHT))
                .flex_shrink_0()
                .px_4()
                .items_center()
                .border_b_1()
                .border_color(border)
                .child(
                    div()
                        .text_base()
                        .font_weight(gpui_kit::FontWeight::BOLD)
                        .text_color(text)
                        .child("设置"),
                ),
        )
        .child(
            v_flex()
                .id("prefs-categories")
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .px_2()
                .py_2()
                .gap_3()
                .children(PrefTab::GROUPS.iter().map(|(group_title, tabs)| {
                    v_flex()
                        .gap_0p5()
                        .child(
                            div()
                                .px_2()
                                .pt_1()
                                .pb_1()
                                .text_xs()
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .text_color(muted_fg)
                                .child(SharedString::from(*group_title)),
                        )
                        .children(tabs.iter().copied().map(|tab| {
                            let is_active = tab == active_tab;
                            h_flex()
                                .id(SharedString::from(format!("prefs-tab-{}", tab.label())))
                                .h(px(CATEGORY_ROW_HEIGHT))
                                .px_2()
                                .gap_2()
                                .items_center()
                                .rounded(px(8.0))
                                .cursor_pointer()
                                .text_color(if is_active { accent } else { text })
                                .when(is_active, |row| {
                                    row.bg(p.accent_soft())
                                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                })
                                .when(!is_active, |row| row.hover(|s| s.bg(p.accent_soft())))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.set_settings_tab(tab, cx);
                                }))
                                .child(
                                    Icon::new(tab.icon())
                                        .with_size(px(16.0))
                                        .text_color(if is_active { accent } else { muted_fg }),
                                )
                                .child(div().text_sm().child(SharedString::from(tab.label())))
                        }))
                })),
        );

    let right_body: gpui_kit::AnyElement = if active_tab == PrefTab::MailAccounts {
        accounts::render_content(app, cx)
    } else if let Some(entity) = app.preferences.clone() {
        div().flex_1().min_h_0().child(entity).into_any_element()
    } else {
        div().flex_1().into_any_element()
    };

    h_flex()
        .size_full()
        .bg(canvas)
        .child(sidebar)
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .h_full()
                // 顶部 48px 栏：当前分类标题 + 全局关闭按钮。
                .child(
                    h_flex()
                        .h(px(HEADER_HEIGHT))
                        .flex_shrink_0()
                        .px_6()
                        .items_center()
                        .justify_between()
                        .border_b_1()
                        .border_color(border)
                        .child(
                            div()
                                .text_base()
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .text_color(text)
                                .child(SharedString::from(active_tab.label())),
                        )
                        .child(
                            Button::new("pref-close")
                                .ghost()
                                .compact()
                                .icon(IconName::X)
                                .tooltip("关闭设置 (Esc)")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.close_modal(cx);
                                })),
                        ),
                )
                .child(right_body),
        )
        .into_any_element()
}

/// 字节数 → 人类可读大小（B / KB / MB，进率 1024）。
pub fn format_size(bytes: usize) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    let value = bytes as f64;
    if value < KB {
        format!("{bytes} B")
    } else if value < MB {
        format!("{:.1} KB", value / KB)
    } else {
        format!("{:.1} MB", value / MB)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_and_view_labels_are_visible() {
        assert_eq!(dark_mode_label(Some(false)), "亮色");
        assert_eq!(dark_mode_label(Some(true)), "暗色");
        assert_eq!(dark_mode_label(None), "跟随系统");
        assert_eq!(default_view_label("unified"), "统一收件箱");
        assert_eq!(default_view_label("last"), "上次文件夹");
        assert_eq!(default_view_label("unknown"), "统一收件箱");
        assert_eq!(sync_interval_label(0), "手动");
        assert_eq!(sync_interval_label(1), "1 分钟");
        assert_eq!(sync_interval_label(5), "5 分钟");
        assert_eq!(sync_interval_label(15), "15 分钟");
    }

    #[test]
    fn format_size_uses_b_kb_mb() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(2048), "2.0 KB");
        assert_eq!(format_size(3 * 1024 * 1024), "3.0 MB");
    }

    #[test]
    fn prefs_cover_all_theme_ids() {
        let prefs = AppPreferences::default();
        assert!(theme::THEME_LIST
            .iter()
            .any(|(id, _)| *id == prefs.theme_id));
    }

    #[test]
    fn settings_groups_include_accounts_and_settings() {
        assert_eq!(PrefTab::GROUPS.len(), 2);
        assert_eq!(PrefTab::GROUPS[0].0, "账号");
        assert_eq!(PrefTab::GROUPS[0].1, &[PrefTab::MailAccounts]);
        assert_eq!(PrefTab::GROUPS[1].0, "设置");
        assert!(PrefTab::GROUPS[1].1.contains(&PrefTab::General));
        assert!(PrefTab::GROUPS[1].1.contains(&PrefTab::Appearance));
        assert!(PrefTab::GROUPS[1].1.contains(&PrefTab::FocusMode));
        assert!(PrefTab::GROUPS[1].1.contains(&PrefTab::Sidebar));
        assert!(PrefTab::GROUPS[1].1.contains(&PrefTab::MessageList));
        assert!(PrefTab::GROUPS[1].1.contains(&PrefTab::Conversations));
        assert!(PrefTab::GROUPS[1].1.contains(&PrefTab::Reading));
        assert!(PrefTab::GROUPS[1].1.contains(&PrefTab::Storage));
    }
}
