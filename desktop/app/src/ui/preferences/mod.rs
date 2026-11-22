//! 独立非模态偏好设置窗口（严格复刻 Linux 版 2 分组 14 项导航 + 左侧搜索 + 640px 窄窗单栏折叠）。

pub(crate) mod pages;
pub(crate) mod search;
pub(crate) mod widgets;

use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{h_flex, v_flex, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    div, px, AnyElement, AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Render, ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Window,
    WindowControlArea,
};
use hylki_mail_core::config::{
    self, AccountConfig, AppPreferences, SettingsBackupBundle, DEFAULT_ATTACHMENT_KEYWORDS,
};

use crate::app::{CloseOverlay, CloseSettingsWindow, FocusSearch, HylkiApp, ProviderPreset};
use crate::theme::{self, HylkiPalette};
use crate::ui::accounts::{self, AccountFormState};
use crate::ui::preferences::search::{filter_search_items, PrefSearchItem};
use crate::ui::preferences::widgets::{NARROW_BREAKPOINT, SEARCH_HIGHLIGHT_MS};

/// 侧栏分组（对齐 Linux 版 `账号` / `设置` 2 个分组）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NavGroup {
    Accounts,
    Settings,
}

impl NavGroup {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Accounts => "账号",
            Self::Settings => "设置",
        }
    }
}

/// 偏好设置 14 个独立分页枚举（严格对齐 Linux 版两分组结构）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrefTab {
    MailAccounts,
    Senders,
    General,
    Appearance,
    FocusMode,
    Sidebar,
    MessageList,
    Conversations,
    Reading,
    Composing,
    Privacy,
    DateTime,
    System,
    Backup,
}

impl PrefTab {
    pub(crate) const ALL: &'static [Self] = &[
        Self::MailAccounts,
        Self::Senders,
        Self::General,
        Self::Appearance,
        Self::FocusMode,
        Self::Sidebar,
        Self::MessageList,
        Self::Conversations,
        Self::Reading,
        Self::Composing,
        Self::Privacy,
        Self::DateTime,
        Self::System,
        Self::Backup,
    ];

    pub(crate) fn group(self) -> NavGroup {
        match self {
            Self::MailAccounts | Self::Senders => NavGroup::Accounts,
            _ => NavGroup::Settings,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::MailAccounts => "邮箱账号",
            Self::Senders => "发件人",
            Self::General => "常规",
            Self::Appearance => "外观",
            Self::FocusMode => "专注模式",
            Self::Sidebar => "侧边栏",
            Self::MessageList => "邮件列表",
            Self::Conversations => "会话",
            Self::Reading => "阅读",
            Self::Composing => "写信",
            Self::Privacy => "隐私",
            Self::DateTime => "日期和时间",
            Self::System => "系统",
            Self::Backup => "备份",
        }
    }

    pub(crate) fn subtitle(self) -> &'static str {
        match self {
            Self::MailAccounts => "添加与管理 IMAP / SMTP / Outlook OAuth2 邮箱账号",
            Self::Senders => "配置始终允许加载远程图片的信任发件人列表",
            Self::General => "系统托盘、启动默认视图、自动同步与通知提示",
            Self::Appearance => "明暗模式、主题配色、界面文字缩放与状态栏位置",
            Self::FocusMode => "沉浸阅读时自动调整边栏、列表、简化视图及工具栏",
            Self::Sidebar => "控制导航区段与统一文件夹入口的显示及折叠记忆",
            Self::MessageList => "紧凑列表、头像、日期分组及搜索 / 批量操作栏位置",
            Self::Conversations => "对话线程排序方向、自动折叠策略与卡片附件",
            Self::Reading => "自动标为已读时机、阅读器布局位置、信纸主题与顶栏开关",
            Self::Composing => "默认发件人、签名位置、撤销发送秒数、回复头与附件检测",
            Self::Privacy => "默认拦截外部图片、提示横幅及链接追踪参数清理",
            Self::DateTime => "统一配置日期年月顺序与 12 / 24 小时时间格式",
            Self::System => "查看本地配置/缓存根目录、清理缓存与版本引擎信息",
            Self::Backup => "导出或导入便携 TOML 配置与账号服务器参数备份",
        }
    }

    pub(crate) fn icon(self) -> IconName {
        match self {
            Self::MailAccounts => IconName::CircleUser,
            Self::Senders => IconName::Users,
            Self::General => IconName::Settings,
            Self::Appearance => IconName::Palette,
            Self::FocusMode => IconName::Eye,
            Self::Sidebar => IconName::PanelLeft,
            Self::MessageList => IconName::List,
            Self::Conversations => IconName::BookOpen,
            Self::Reading => IconName::MailOpen,
            Self::Composing => IconName::Pencil,
            Self::Privacy => IconName::Ban,
            Self::DateTime => IconName::Info,
            Self::System => IconName::HardDrive,
            Self::Backup => IconName::Archive,
        }
    }
}

/// 根据当前设置窗口总宽度计算左导航栏宽度（27% 并钳制到 `[180.0, 250.0]`，对齐 Linux 版 `27% 180..=250`）。
pub(crate) fn compute_sidebar_width(window_width: f32) -> f32 {
    (window_width * 0.27).clamp(180.0, 250.0)
}

/// 独立偏好设置窗口主视图（单实例、非模态，持有子窗口级输入控件并双向订阅 `HylkiApp`）。
pub(crate) struct SettingsWindow {
    pub(crate) app: Entity<HylkiApp>,
    pub(crate) active_tab: PrefTab,
    pub(crate) narrow_show_content: bool,
    pub(crate) search_open: bool,
    pub(crate) search_query: String,
    pub(crate) search_input: Entity<InputState>,
    pub(crate) highlighted_row_id: Option<&'static str>,
    pub(crate) highlight_token: u64,
    pub(crate) account_form: AccountFormState,
    pub(crate) sender_input: Entity<InputState>,
    pub(crate) sender_feedback: Option<(bool, SharedString)>,
    pub(crate) reply_header_input: Entity<InputState>,
    pub(crate) attachment_keywords_input: Entity<InputState>,
    pub(crate) compose_inputs_feedback: Option<SharedString>,
    pub(crate) confirm_clear_cache: bool,
    pub(crate) pending_backup_import: Option<(std::path::PathBuf, SettingsBackupBundle)>,
    pub(crate) backup_feedback: Option<(bool, SharedString)>,
    pub(crate) sidebar_scroll: ScrollHandle,
    pub(crate) content_scroll: ScrollHandle,
    last_rem_scale: u32,
}

impl SettingsWindow {
    pub(crate) fn new(
        app: Entity<HylkiApp>,
        initial_prefs: AppPreferences,
        initial_tab: PrefTab,
        edit_account: Option<AccountConfig>,
        preset: Option<ProviderPreset>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let initial_scale = config::clamp_ui_text_scale(initial_prefs.ui_text_scale_percent);
        window.set_rem_size(px(16.0 * initial_scale as f32 / 100.0));

        // 只要主应用状态更新（如同步、账号保存回执、偏好变更），自动刷新本窗口。
        cx.observe(&app, |_, _, cx| cx.notify()).detach();

        let search_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("搜索设置项，如 字体 / 托盘 / 签名...")
        });
        cx.subscribe(&search_input, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.search_query = input.read(cx).value().to_string();
                cx.notify();
            }
        })
        .detach();

        let sender_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("输入邮箱地址添加，如 newsletter@example.com")
        });
        let reply_header_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("留空用默认中文格式，支持 {name} {address} {date} {time} {subject}")
                .default_value(initial_prefs.reply_header_template.clone())
        });
        let attachment_keywords_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(DEFAULT_ATTACHMENT_KEYWORDS)
                .default_value(initial_prefs.attachment_reminder_keywords.clone())
        });

        let mut account_form = AccountFormState::new(edit_account.as_ref(), window, cx);
        if let Some(p) = preset {
            account_form.apply_preset(&p, window, cx);
        }

        Self {
            app,
            active_tab: initial_tab,
            narrow_show_content: edit_account.is_some() || preset.is_some(),
            search_open: false,
            search_query: String::new(),
            search_input,
            highlighted_row_id: None,
            highlight_token: 0,
            account_form,
            sender_input,
            sender_feedback: None,
            reply_header_input,
            attachment_keywords_input,
            compose_inputs_feedback: None,
            confirm_clear_cache: false,
            pending_backup_import: None,
            backup_feedback: None,
            sidebar_scroll: ScrollHandle::new(),
            content_scroll: ScrollHandle::new(),
            last_rem_scale: initial_scale,
        }
    }

    pub(crate) fn select_tab(
        &mut self,
        tab: PrefTab,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_tab = tab;
        self.narrow_show_content = true;
        self.confirm_clear_cache = false;
        self.sender_feedback = None;
        self.compose_inputs_feedback = None;
        cx.notify();
    }

    pub(crate) fn edit_account_in_form(
        &mut self,
        cfg: Option<&AccountConfig>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_tab = PrefTab::MailAccounts;
        self.narrow_show_content = true;
        self.account_form.load_account(cfg, window, cx);
        cx.notify();
    }

    pub(crate) fn apply_account_preset(
        &mut self,
        preset: &ProviderPreset,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_tab = PrefTab::MailAccounts;
        self.narrow_show_content = true;
        self.account_form.apply_preset(preset, window, cx);
        cx.notify();
    }

    pub(crate) fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_open = true;
        self.search_input
            .update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    pub(crate) fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_open = false;
        self.search_query.clear();
        self.search_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        cx.notify();
    }

    pub(crate) fn activate_search_hit(
        &mut self,
        item: &PrefSearchItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_tab = item.tab;
        self.narrow_show_content = true;
        self.highlighted_row_id = Some(item.row_id);
        self.highlight_token = self.highlight_token.wrapping_add(1);
        let token = self.highlight_token;
        let win_width = window.viewport_size().width.as_f32();
        if win_width <= NARROW_BREAKPOINT {
            self.search_open = false;
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(SEARCH_HIGHLIGHT_MS))
                .await;
            let _ = this.update(cx, |w, cx| {
                if w.highlight_token == token {
                    w.highlighted_row_id = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn sync_compose_inputs_from_prefs(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let prefs = self.app.read(cx).prefs.clone();
        self.reply_header_input.update(cx, |s, cx| {
            s.set_value(prefs.reply_header_template, window, cx)
        });
        self.attachment_keywords_input.update(cx, |s, cx| {
            s.set_value(prefs.attachment_reminder_keywords, window, cx);
        });
    }

    pub(crate) fn add_allowed_sender(
        &mut self,
        app_entity: &Entity<HylkiApp>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let raw = self.sender_input.read(cx).value().trim().to_string();
        if raw.is_empty() || !raw.contains('@') {
            self.sender_feedback = Some((false, SharedString::from("请输入包含 @ 的合法邮箱地址")));
            cx.notify();
            return;
        }
        let mut added = None;
        app_entity.update(cx, |app, cx| {
            app.mutate_preferences(cx, |prefs| {
                added = prefs.allow_remote_sender(&raw);
            });
        });
        if let Some(email) = added {
            self.sender_input
                .update(cx, |s, cx| s.set_value("", window, cx));
            self.sender_feedback =
                Some((true, SharedString::from(format!("已添加到白名单：{email}"))));
        } else {
            self.sender_feedback = Some((false, SharedString::from("请输入包含 @ 的合法邮箱地址")));
        }
        cx.notify();
    }

    pub(crate) fn apply_reply_header_input(
        &mut self,
        app_entity: &Entity<HylkiApp>,
        cx: &mut Context<Self>,
    ) {
        let value = self.reply_header_input.read(cx).value().trim().to_string();
        app_entity.update(cx, |app, cx| {
            app.mutate_preferences(cx, |p| p.reply_header_template = value);
        });
        self.compose_inputs_feedback = Some(SharedString::from("已应用自定义回复引用头模板"));
        cx.notify();
    }

    pub(crate) fn reset_reply_header_input(
        &mut self,
        app_entity: &Entity<HylkiApp>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reply_header_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        app_entity.update(cx, |app, cx| {
            app.mutate_preferences(cx, |p| p.reply_header_template.clear());
        });
        self.compose_inputs_feedback = Some(SharedString::from("已恢复默认中文回复引用头"));
        cx.notify();
    }

    pub(crate) fn apply_attachment_keywords_input(
        &mut self,
        app_entity: &Entity<HylkiApp>,
        cx: &mut Context<Self>,
    ) {
        let value = self
            .attachment_keywords_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        app_entity.update(cx, |app, cx| {
            app.mutate_preferences(cx, |p| p.attachment_reminder_keywords = value);
        });
        self.compose_inputs_feedback = Some(SharedString::from("已更新附件遗漏检测关键词"));
        cx.notify();
    }

    pub(crate) fn reset_attachment_keywords_input(
        &mut self,
        app_entity: &Entity<HylkiApp>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.attachment_keywords_input.update(cx, |s, cx| {
            s.set_value(DEFAULT_ATTACHMENT_KEYWORDS, window, cx)
        });
        app_entity.update(cx, |app, cx| {
            app.mutate_preferences(cx, |p| {
                p.attachment_reminder_keywords = DEFAULT_ATTACHMENT_KEYWORDS.to_string();
            });
        });
        self.compose_inputs_feedback = Some(SharedString::from("已恢复默认附件检测关键词"));
        cx.notify();
    }

    pub(crate) fn export_settings_backup(&mut self, cx: &mut Context<Self>) {
        let exported = match config::export_settings_bundle_in(&config::data_dir()) {
            Ok(s) => s,
            Err(e) => {
                self.backup_feedback = Some((false, SharedString::from(e)));
                cx.notify();
                return;
            }
        };
        if let Some(dest) = rfd::FileDialog::new()
            .set_file_name("hylki-settings-backup.toml")
            .add_filter("Hylki 配置备份 (*.toml)", &["toml"])
            .save_file()
        {
            match std::fs::write(&dest, exported) {
                Ok(()) => {
                    self.backup_feedback = Some((
                        true,
                        SharedString::from(format!("配置备份已保存至：{}", dest.display())),
                    ));
                }
                Err(e) => {
                    self.backup_feedback =
                        Some((false, SharedString::from(format!("写出备份文件失败: {e}"))));
                }
            }
            cx.notify();
        }
    }

    pub(crate) fn pick_settings_backup(&mut self, cx: &mut Context<Self>) {
        let Some(src) = rfd::FileDialog::new()
            .add_filter("Hylki 配置备份 (*.toml)", &["toml"])
            .pick_file()
        else {
            return;
        };
        let text = match std::fs::read_to_string(&src) {
            Ok(t) => t,
            Err(e) => {
                self.backup_feedback = Some((false, SharedString::from(format!("读取失败: {e}"))));
                cx.notify();
                return;
            }
        };
        match config::parse_settings_bundle(&text) {
            Ok(bundle) => {
                self.pending_backup_import = Some((src, bundle));
                self.backup_feedback = None;
            }
            Err(e) => {
                self.pending_backup_import = None;
                self.backup_feedback = Some((false, SharedString::from(e)));
            }
        }
        cx.notify();
    }

    pub(crate) fn confirm_import_backup(
        &mut self,
        app_entity: &Entity<HylkiApp>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((_, bundle)) = self.pending_backup_import.take() else {
            return;
        };
        let mut import_res = Ok(0usize);
        app_entity.update(cx, |app, cx| {
            import_res = app.apply_imported_backup(bundle, cx);
        });
        match import_res {
            Ok(acc_count) => {
                self.sync_compose_inputs_from_prefs(window, cx);
                self.backup_feedback = Some((
                    true,
                    SharedString::from(format!(
                        "备份导入成功：已恢复偏好设置及 {acc_count} 个账号服务器配置"
                    )),
                ));
            }
            Err(e) => {
                self.backup_feedback = Some((false, SharedString::from(e)));
            }
        }
        cx.notify();
    }

    fn close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.app.update(cx, |app, _| {
            app.clear_settings_window_ref();
        });
        window.remove_window();
    }

    fn render_sidebar(
        &mut self,
        width: f32,
        narrow_only_sidebar: bool,
        p: &HylkiPalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let results = if self.search_open && !self.search_query.trim().is_empty() {
            Some(filter_search_items(&self.search_query))
        } else {
            None
        };

        v_flex()
            .w(px(width))
            .h_full()
            .flex_shrink_0()
            .bg(p.sidebar)
            .border_r_1()
            .border_color(p.border)
            .child(
                h_flex()
                    .h(px(46.0))
                    .px_3()
                    .items_center()
                    .justify_between()
                    .gap_1()
                    .border_b_1()
                    .border_color(p.border)
                    .child(
                        h_flex()
                            .flex_1()
                            .h_full()
                            .items_center()
                            .window_control_area(WindowControlArea::Drag)
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(p.text)
                                    .child("偏好设置"),
                            ),
                    )
                    .child(
                        Button::new("pref-search-toggle")
                            .ghost()
                            .compact()
                            .icon(if self.search_open {
                                IconName::Close
                            } else {
                                IconName::Search
                            })
                            .tooltip("搜索设置 (Ctrl+F)")
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this.search_open {
                                    this.close_search(window, cx);
                                } else {
                                    this.open_search(window, cx);
                                }
                            })),
                    )
                    .when(narrow_only_sidebar, |el| {
                        el.child(self.render_window_controls(cx))
                    }),
            )
            .when(self.search_open, |el| {
                el.child(
                    div()
                        .px_2p5()
                        .py_2()
                        .border_b_1()
                        .border_color(p.border)
                        .child(Input::new(&self.search_input)),
                )
            })
            .child(if let Some(hits) = results {
                self.render_search_results(hits, p, cx)
            } else {
                self.render_nav_groups(p, cx)
            })
            .into_any_element()
    }

    fn render_search_results(
        &mut self,
        hits: Vec<&'static PrefSearchItem>,
        p: &HylkiPalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if hits.is_empty() {
            return v_flex()
                .flex_1()
                .min_h_0()
                .items_center()
                .justify_center()
                .p_4()
                .gap_1p5()
                .child(Icon::new(IconName::Search).small().text_color(p.text_muted))
                .child(
                    div()
                        .text_xs()
                        .text_color(p.text_muted)
                        .child("未找到匹配的设置项"),
                )
                .into_any_element();
        }

        let mut list = v_flex()
            .id("pref-search-results")
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .p_2()
            .gap_1();
        for (idx, item) in hits.into_iter().enumerate() {
            list = list.child(
                v_flex()
                    .id(("pref-hit", idx))
                    .w_full()
                    .px_2p5()
                    .py_2()
                    .rounded(px(8.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(p.surface))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.activate_search_hit(item, window, cx);
                    }))
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(p.text)
                            .truncate()
                            .child(SharedString::from(item.title)),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.text_muted)
                            .truncate()
                            .child(SharedString::from(item.tab.label())),
                    ),
            );
        }
        list.into_any_element()
    }

    fn render_nav_groups(&mut self, p: &HylkiPalette, cx: &mut Context<Self>) -> AnyElement {
        let mut list = v_flex()
            .id("pref-nav-scroll")
            .track_scroll(&self.sidebar_scroll)
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .px_2()
            .py_2p5()
            .gap_3();

        for group in [NavGroup::Accounts, NavGroup::Settings] {
            let mut section = v_flex().w_full().gap_0p5().child(
                div()
                    .px_2p5()
                    .pb_1()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.text_muted)
                    .child(SharedString::from(group.label())),
            );
            for &tab in PrefTab::ALL.iter().filter(|t| t.group() == group) {
                let active = self.active_tab == tab;
                let bg = if active { p.accent_soft() } else { p.sidebar };
                let fg = if active { p.accent } else { p.text };
                section = section.child(
                    h_flex()
                        .id(SharedString::from(format!("nav-tab-{tab:?}")))
                        .h(px(34.0))
                        .px_2p5()
                        .gap_2()
                        .rounded(px(8.0))
                        .items_center()
                        .bg(bg)
                        .text_color(fg)
                        .cursor_pointer()
                        .when(!active, |el| el.hover(|s| s.bg(p.surface)))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.select_tab(tab, window, cx);
                        }))
                        .child(Icon::new(tab.icon()).xsmall().text_color(if active {
                            p.accent
                        } else {
                            p.text_muted
                        }))
                        .child(
                            div()
                                .text_xs()
                                .font_weight(if active {
                                    FontWeight::SEMIBOLD
                                } else {
                                    FontWeight::MEDIUM
                                })
                                .truncate()
                                .child(SharedString::from(tab.label())),
                        ),
                );
            }
            list = list.child(section);
        }

        list.into_any_element()
    }

    fn render_window_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .items_center()
            .gap_0p5()
            .child(
                Button::new("pref-win-min")
                    .ghost()
                    .compact()
                    .icon(IconName::Minus)
                    .tooltip("最小化")
                    .on_click(|_, window, _| window.minimize_window()),
            )
            .child(
                div().window_control_area(WindowControlArea::Max).child(
                    Button::new("pref-win-max")
                        .ghost()
                        .compact()
                        .icon(IconName::Square)
                        .tooltip("最大化 / 还原"),
                ),
            )
            .child(
                Button::new("pref-win-close")
                    .ghost()
                    .compact()
                    .icon(IconName::Close)
                    .tooltip("关闭设置 (Esc / Ctrl+W)")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.close_window(window, cx);
                    })),
            )
            .into_any_element()
    }

    fn render_content_header(
        &mut self,
        narrow: bool,
        p: &HylkiPalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        h_flex()
            .h(px(46.0))
            .w_full()
            .px_4()
            .items_center()
            .justify_between()
            .gap_2()
            .border_b_1()
            .border_color(p.border)
            .bg(p.canvas)
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .items_center()
                    .gap_2()
                    .window_control_area(WindowControlArea::Drag)
                    .when(narrow && self.narrow_show_content, |el| {
                        el.child(
                            Button::new("pref-narrow-back")
                                .ghost()
                                .compact()
                                .label("← 返回")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.narrow_show_content = false;
                                    cx.notify();
                                })),
                        )
                    })
                    .child(
                        v_flex().min_w_0().child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(p.text)
                                .truncate()
                                .child(SharedString::from(self.active_tab.label())),
                        ),
                    ),
            )
            .child(self.render_window_controls(cx))
            .into_any_element()
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let scale_percent =
            config::clamp_ui_text_scale(self.app.read(cx).prefs.ui_text_scale_percent);
        if self.last_rem_scale != scale_percent {
            window.set_rem_size(px(16.0 * scale_percent as f32 / 100.0));
            self.last_rem_scale = scale_percent;
        }
        let p = theme::current(cx);

        let viewport_w = window.viewport_size().width.as_f32().max(400.0);
        let narrow = viewport_w <= NARROW_BREAKPOINT;
        let show_sidebar = !narrow || !self.narrow_show_content;
        let show_content = !narrow || self.narrow_show_content;
        let sidebar_w = if narrow {
            viewport_w
        } else {
            compute_sidebar_width(viewport_w)
        };

        let header = self.render_content_header(narrow, &p, cx);
        let content_area: AnyElement = if self.active_tab == PrefTab::MailAccounts {
            let app_entity = self.app.clone();
            accounts::render_content(&self.account_form, app_entity, cx)
        } else {
            let app_entity = self.app.clone();
            pages::render_tab(self, app_entity, &p, window, cx)
        };

        h_flex()
            .size_full()
            .bg(p.canvas)
            .text_color(p.text)
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                this.open_search(window, cx);
            }))
            .on_action(cx.listener(|this, _: &CloseSettingsWindow, window, cx| {
                this.close_window(window, cx);
            }))
            .on_action(cx.listener(|this, _: &CloseOverlay, window, cx| {
                if this.search_open {
                    this.close_search(window, cx);
                } else if window.viewport_size().width.as_f32() <= NARROW_BREAKPOINT
                    && this.narrow_show_content
                {
                    this.narrow_show_content = false;
                    cx.notify();
                } else {
                    this.close_window(window, cx);
                }
            }))
            .when(show_sidebar, |el| {
                el.child(self.render_sidebar(sidebar_w, narrow && !show_content, &p, cx))
            })
            .when(show_content, |el| {
                el.child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .bg(p.canvas)
                        .child(header)
                        .child(
                            div()
                                .px_6()
                                .pt_3()
                                .text_xs()
                                .text_color(p.text_muted)
                                .child(SharedString::from(self.active_tab.subtitle())),
                        )
                        .child(content_area),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pref_tab_groups_and_sidebar_width_match_linux_spec() {
        assert_eq!(PrefTab::ALL.len(), 14);
        assert_eq!(
            PrefTab::ALL
                .iter()
                .filter(|t| t.group() == NavGroup::Accounts)
                .count(),
            2
        );
        assert_eq!(
            PrefTab::ALL
                .iter()
                .filter(|t| t.group() == NavGroup::Settings)
                .count(),
            12
        );
        assert_eq!(compute_sidebar_width(500.0), 180.0);
        assert!((compute_sidebar_width(920.0) - 248.4).abs() < 0.01);
        assert_eq!(compute_sidebar_width(1200.0), 250.0);
    }
}
