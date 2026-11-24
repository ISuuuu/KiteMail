//! 独立非模态偏好设置窗口（严格复刻 Linux 版 2 分组 14 项导航 + 左侧搜索 + 640px 窄窗单栏折叠）。

pub(crate) mod extra_pages;
pub(crate) mod pages;
pub(crate) mod search;
pub(crate) mod widgets;

use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{h_flex, v_flex, Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    black, div, px, AnyElement, AppContext, Context, Entity, FontWeight, InteractiveElement,
    IntoElement, MouseButton, ParentElement, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement, Styled, Window, WindowControlArea,
};
use hylki_mail_core::cache::{self, MailDirInspection, MailDirSwitchMode, TagItem};
use hylki_mail_core::config::{
    self, AccountConfig, AppPreferences, FilterField, FilterMatch, SettingsBackupBundle,
    DEFAULT_ATTACHMENT_KEYWORDS,
};

use crate::app::{CloseOverlay, CloseSettingsWindow, FocusSearch, HylkiApp};
use crate::theme::{self, HylkiPalette};
use crate::ui::accounts::{self, AccountFormState};
use crate::ui::preferences::search::{filter_search_items, PrefSearchItem};
use crate::ui::preferences::widgets::{NARROW_BREAKPOINT, SEARCH_HIGHLIGHT_MS};

/// 账号编辑页未保存时尝试离开的目标动作。
#[derive(Clone, Copy, Debug)]
pub(crate) enum EditorLeaveAction {
    BackToAccountList,
    SwitchTab(PrefTab),
    ActivateSearchHit(&'static PrefSearchItem),
    CloseWindow,
    /// 主窗口 / 账号列表请求编辑指定账号（任务二：跳转前接入未保存确认）。
    EditAccount(u32),
    /// 主窗口 / 欢迎页请求新建账号（任务二：跳转前接入未保存确认）。
    NewAccount,
    PresetAccount(usize),
}

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

/// 偏好设置 22 个独立分页枚举（严格对齐 Linux 版两分组结构）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PrefTab {
    MailAccounts,
    Tags,
    Filters,
    SmartFolders,
    Senders,
    OpenPgp,
    CloudStorage,
    Translation,
    ContactsSources,
    General,
    Appearance,
    FocusMode,
    Sidebar,
    MessageList,
    Conversations,
    Reading,
    Composing,
    Privacy,
    Mcp,
    DateTime,
    System,
    Backup,
}

impl PrefTab {
    pub(crate) const ALL: &'static [Self] = &[
        Self::MailAccounts,
        Self::Tags,
        Self::Filters,
        Self::SmartFolders,
        Self::Senders,
        Self::OpenPgp,
        Self::CloudStorage,
        Self::Translation,
        Self::ContactsSources,
        Self::General,
        Self::Appearance,
        Self::FocusMode,
        Self::Sidebar,
        Self::MessageList,
        Self::Conversations,
        Self::Reading,
        Self::Composing,
        Self::Privacy,
        Self::Mcp,
        Self::DateTime,
        Self::System,
        Self::Backup,
    ];

    pub(crate) fn group(self) -> NavGroup {
        match self {
            Self::MailAccounts
            | Self::Tags
            | Self::Filters
            | Self::SmartFolders
            | Self::Senders
            | Self::OpenPgp
            | Self::CloudStorage
            | Self::Translation
            | Self::ContactsSources => NavGroup::Accounts,
            _ => NavGroup::Settings,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::MailAccounts => "邮箱账号",
            Self::Tags => "标签",
            Self::Filters => "过滤器",
            Self::SmartFolders => "智能文件夹",
            Self::Senders => "发件人",
            Self::OpenPgp => "OpenPGP 密钥",
            Self::CloudStorage => "云存储",
            Self::Translation => "翻译",
            Self::ContactsSources => "通讯录源",
            Self::General => "常规",
            Self::Appearance => "外观",
            Self::FocusMode => "专注模式",
            Self::Sidebar => "侧边栏",
            Self::MessageList => "邮件列表",
            Self::Conversations => "会话",
            Self::Reading => "阅读",
            Self::Composing => "写信",
            Self::Privacy => "隐私",
            Self::Mcp => "MCP 服务",
            Self::DateTime => "日期和时间",
            Self::System => "系统",
            Self::Backup => "备份",
        }
    }

    pub(crate) fn subtitle(self) -> &'static str {
        match self {
            Self::MailAccounts => "添加与管理 IMAP / SMTP / Outlook OAuth2 邮箱账号",
            Self::Tags => "增删改自定义邮件标签、调色板颜色与快捷键 1..9 绑定",
            Self::Filters => "创建自动化邮件过滤规则，并在现有缓存邮件上立即执行",
            Self::SmartFolders => "基于搜索表达式创建动态虚拟文件夹",
            Self::Senders => "管理发件人黑名单",
            Self::OpenPgp => "管理本地 OpenPGP 密钥信息与按邮箱绑定的签名/加密策略",
            Self::CloudStorage => {
                "配置超大附件云盘账号（Nextcloud / WebDAV / S3 / Dropbox）与自动上传阈值"
            }
            Self::Translation => "配置阅读器与写信器一键翻译引擎及目标语言",
            Self::ContactsSources => "管理 CardDAV 远程通讯录同步源与自动收集发件人开关",
            Self::General => "系统托盘、启动默认视图、自动同步与通知提示",
            Self::Appearance => "明暗模式、主题配色、界面文字缩放与状态栏位置",
            Self::FocusMode => "沉浸阅读时自动调整边栏、列表、简化视图及工具栏",
            Self::Sidebar => "控制导航区段与统一文件夹入口的显示及折叠记忆",
            Self::MessageList => "紧凑列表、头像、日期分组及搜索 / 批量操作栏位置",
            Self::Conversations => "对话线程排序方向、自动折叠策略与卡片附件",
            Self::Reading => "自动标为已读时机、阅读器布局位置、信纸主题与顶栏开关",
            Self::Composing => "默认发件人、签名位置、撤销发送秒数、回复头与附件检测",
            Self::Privacy => "默认拦截外部图片、提示横幅及链接追踪参数清理",
            Self::Mcp => "管理本地 MCP Server 开关、命名管道路径与 AI 工具权限",
            Self::DateTime => "统一配置日期年月顺序与 12 / 24 小时时间格式",
            Self::System => "查看本地配置/缓存根目录、清理缓存与版本引擎信息",
            Self::Backup => "导出或导入便携 TOML 配置与账号服务器参数备份",
        }
    }

    pub(crate) fn icon(self) -> IconName {
        match self {
            Self::MailAccounts => IconName::CircleUser,
            Self::Tags => IconName::Tag,
            Self::Filters => IconName::Search,
            Self::SmartFolders => IconName::Folder,
            Self::Senders => IconName::Users,
            Self::OpenPgp => IconName::Shield,
            Self::CloudStorage => IconName::Upload,
            Self::Translation => IconName::Globe,
            Self::ContactsSources => IconName::BookOpen,
            Self::General => IconName::Settings,
            Self::Appearance => IconName::Palette,
            Self::FocusMode => IconName::Eye,
            Self::Sidebar => IconName::PanelLeft,
            Self::MessageList => IconName::List,
            Self::Conversations => IconName::BookOpen,
            Self::Reading => IconName::MailOpen,
            Self::Composing => IconName::Pencil,
            Self::Privacy => IconName::Ban,
            Self::Mcp => IconName::Bot,
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
    pub(crate) file_dialog_open: bool,
    pub(crate) account_form: AccountFormState,
    pub(crate) account_editor_open: bool,
    pub(crate) confirm_remove_in_editor: bool,
    pub(crate) pending_leave_action: Option<EditorLeaveAction>,
    pub(crate) after_save_action: Option<EditorLeaveAction>,
    pub(crate) sender_feedback: Option<(bool, SharedString)>,
    /// WP-05：发件人页实时过滤关键词（用于黑名单列表）。
    pub(crate) senders_filter: String,
    pub(crate) sender_search_input: Entity<InputState>,
    /// WP-05：发件人页黑名单添加输入框。
    pub(crate) blocked_input: Entity<InputState>,
    pub(crate) reply_header_input: Entity<InputState>,
    pub(crate) attachment_keywords_input: Entity<InputState>,
    pub(crate) compose_inputs_feedback: Option<SharedString>,
    pub(crate) confirm_clear_cache: bool,
    /// 系统页双目录及分项磁盘占用摘要（进入系统页时计算一次，清除缓存或切换目录后失效）。
    pub(crate) storage_summary: Option<config::AppStorageSummary>,
    /// 待二次确认的邮件存储目录切换 `(target_dir_override, target_inspection)`。
    pub(crate) pending_mail_dir_switch: Option<(Option<std::path::PathBuf>, MailDirInspection)>,
    /// 后台正在执行邮件存储目录切换或迁移。
    pub(crate) mail_dir_switching: bool,
    /// 正在执行的目录切换模式与目标路径（用于在确认弹窗内平滑切为进行中态）。
    pub(crate) mail_dir_switch_active: Option<(MailDirSwitchMode, std::path::PathBuf)>,
    /// 邮件存储目录校验/切换结果提示（就地展示在「邮件存储目录」行紧下方）。
    pub(crate) mail_dir_feedback: Option<(bool, SharedString)>,
    /// 当前生效邮件目录关联的外部配置中可一键导入的未配置账号。
    pub(crate) current_mail_dir_importable: Vec<AccountConfig>,
    pub(crate) pending_backup_import: Option<(std::path::PathBuf, SettingsBackupBundle)>,
    pub(crate) backup_feedback: Option<(bool, SharedString)>,
    // ---- WP-06：8 个新增分页控件状态 ----
    pub(crate) pgp_name_input: Entity<InputState>,
    pub(crate) pgp_email_input: Entity<InputState>,
    pub(crate) pgp_fp_input: Entity<InputState>,
    pub(crate) pgp_feedback: Option<(bool, SharedString)>,
    pub(crate) cloud_provider: String,
    pub(crate) cloud_name_input: Entity<InputState>,
    pub(crate) cloud_url_input: Entity<InputState>,
    pub(crate) cloud_user_input: Entity<InputState>,
    pub(crate) cloud_bucket_input: Entity<InputState>,
    pub(crate) cloud_feedback: Option<(bool, SharedString)>,
    pub(crate) cs_provider: String,
    pub(crate) cs_name_input: Entity<InputState>,
    pub(crate) cs_url_input: Entity<InputState>,
    pub(crate) cs_user_input: Entity<InputState>,
    pub(crate) cs_feedback: Option<(bool, SharedString)>,
    pub(crate) tag_name_input: Entity<InputState>,
    pub(crate) tag_color: String,
    pub(crate) tag_shortcut: Option<u8>,
    pub(crate) editing_tag_id: Option<u32>,
    pub(crate) cached_tags: Vec<TagItem>,
    pub(crate) tags_loaded: bool,
    pub(crate) tag_feedback: Option<(bool, SharedString)>,
    pub(crate) filter_name_input: Entity<InputState>,
    pub(crate) filter_value_input: Entity<InputState>,
    pub(crate) filter_folder_input: Entity<InputState>,
    pub(crate) filter_tag_input: Entity<InputState>,
    pub(crate) filter_forward_input: Entity<InputState>,
    pub(crate) filter_field: FilterField,
    pub(crate) filter_matcher: FilterMatch,
    pub(crate) filter_star: bool,
    pub(crate) filter_mark_read: bool,
    pub(crate) filter_archive: bool,
    pub(crate) filter_account_email: String,
    pub(crate) filter_feedback: Option<(bool, SharedString)>,
    pub(crate) smart_name_input: Entity<InputState>,
    pub(crate) smart_query_input: Entity<InputState>,
    pub(crate) smart_icon: String,
    pub(crate) smart_account_email: String,
    pub(crate) smart_feedback: Option<(bool, SharedString)>,
    pub(crate) trans_url_input: Entity<InputState>,
    pub(crate) trans_key_input: Entity<InputState>,
    pub(crate) trans_feedback: Option<(bool, SharedString)>,
    pub(crate) mcp_socket_input: Entity<InputState>,
    pub(crate) mcp_feedback: Option<(bool, SharedString)>,
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
        preset_idx: Option<usize>,
        open_account_editor: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let initial_scale = config::clamp_ui_text_scale(initial_prefs.ui_text_scale_percent);
        window.set_rem_size(px(16.0 * initial_scale as f32 / 100.0));

        // 任务二：Alt+F4 / 系统关闭与窗口关闭按钮走同一套离开确认——账号编辑
        // 页有未保存改动时先拦截并弹「保存 / 放弃 / 继续编辑」确认条。
        let weak_self = cx.entity().downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            weak_self
                .update(cx, |this, cx| {
                    if this.mail_dir_switching {
                        false
                    } else if this.is_account_editor_dirty(cx) {
                        this.pending_leave_action = Some(EditorLeaveAction::CloseWindow);
                        this.confirm_remove_in_editor = false;
                        cx.notify();
                        false
                    } else {
                        true
                    }
                })
                .unwrap_or(true)
        });

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

        let sender_search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("搜索发件人..."));
        cx.subscribe(
            &sender_search_input,
            |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.senders_filter = input.read(cx).value().to_string();
                    cx.notify();
                }
            },
        )
        .detach();
        let blocked_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("输入邮箱或域名，如 spam.example.com")
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

        let pgp_name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("姓名（如：张三）"));
        let pgp_email_input = cx
            .new(|cx| InputState::new(window, cx).placeholder("绑定邮箱（如：user@example.com）"));
        let pgp_fp_input = cx
            .new(|cx| InputState::new(window, cx).placeholder("40 位指纹（留空自动生成本地标识）"));
        let cloud_name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("账号名称（如：公司 Nextcloud）"));
        let cloud_url_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("服务器或 Endpoint URL（如：https://cloud.example.com）")
        });
        let cloud_user_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("用户名 / Access Key"));
        let cloud_bucket_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("目录路径或 Bucket（可选）"));
        let cs_name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("通讯录名称（如：工作通讯录）"));
        let cs_url_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("CardDAV 地址（如：https://contacts.icloud.com）")
        });
        let cs_user_input = cx.new(|cx| InputState::new(window, cx).placeholder("用户名 / 邮箱"));
        let tag_name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("标签名称（如：重要 / 待办）"));
        let filter_name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("规则名称（如：账单归档）"));
        let filter_value_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("匹配关键词或邮箱"));
        let filter_folder_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("目标文件夹（如：Archive）"));
        let filter_tag_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("附加标签（可选）"));
        let filter_forward_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("转发邮箱（可选）"));
        let smart_name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("智能文件夹名称（如：未读账单）"));
        let smart_query_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("搜索表达式（如：from:billing is:unread）")
        });
        let trans_url_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("服务地址（如：http://127.0.0.1:5000）")
                .default_value(initial_prefs.translation_api_url.clone())
        });
        let trans_key_input = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("API Key（如需鉴权）")
        });
        let mcp_socket_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(r"\\.\pipe\hylki-mcp")
                .default_value(initial_prefs.mcp_socket_path.clone())
        });

        let mut account_form = AccountFormState::new(edit_account.as_ref(), window, cx);
        if let Some(idx) = preset_idx {
            account_form.apply_provider_for_new(idx, window, cx);
        }

        let editor_open = initial_tab == PrefTab::MailAccounts && open_account_editor;
        let narrow_show_content = editor_open || edit_account.is_some() || preset_idx.is_some();

        Self {
            app,
            active_tab: initial_tab,
            narrow_show_content,
            search_open: false,
            search_query: String::new(),
            search_input,
            highlighted_row_id: None,
            highlight_token: 0,
            file_dialog_open: false,
            account_form,
            account_editor_open: editor_open,
            confirm_remove_in_editor: false,
            pending_leave_action: None,
            after_save_action: None,
            sender_feedback: None,
            senders_filter: String::new(),
            sender_search_input,
            blocked_input,
            reply_header_input,
            attachment_keywords_input,
            compose_inputs_feedback: None,
            confirm_clear_cache: false,
            storage_summary: None,
            pending_mail_dir_switch: None,
            mail_dir_switching: false,
            mail_dir_switch_active: None,
            mail_dir_feedback: None,
            current_mail_dir_importable: Vec::new(),
            pending_backup_import: None,
            backup_feedback: None,
            pgp_name_input,
            pgp_email_input,
            pgp_fp_input,
            pgp_feedback: None,
            cloud_provider: "Nextcloud".to_string(),
            cloud_name_input,
            cloud_url_input,
            cloud_user_input,
            cloud_bucket_input,
            cloud_feedback: None,
            cs_provider: "CardDAV".to_string(),
            cs_name_input,
            cs_url_input,
            cs_user_input,
            cs_feedback: None,
            tag_name_input,
            tag_color: "#3584e4".to_string(),
            tag_shortcut: None,
            editing_tag_id: None,
            cached_tags: Vec::new(),
            tags_loaded: false,
            tag_feedback: None,
            filter_name_input,
            filter_value_input,
            filter_folder_input,
            filter_tag_input,
            filter_forward_input,
            filter_field: FilterField::From,
            filter_matcher: FilterMatch::Contains,
            filter_star: false,
            filter_mark_read: false,
            filter_archive: false,
            filter_account_email: String::new(),
            filter_feedback: None,
            smart_name_input,
            smart_query_input,
            smart_icon: "folder-search".to_string(),
            smart_account_email: String::new(),
            smart_feedback: None,
            trans_url_input,
            trans_key_input,
            trans_feedback: None,
            mcp_socket_input,
            mcp_feedback: None,
            sidebar_scroll: ScrollHandle::new(),
            content_scroll: ScrollHandle::new(),
            last_rem_scale: initial_scale,
        }
    }

    fn is_account_editor_dirty(&self, cx: &Context<Self>) -> bool {
        self.active_tab == PrefTab::MailAccounts
            && self.account_editor_open
            && self.account_form.is_dirty(cx)
    }

    pub(crate) fn open_new_account_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.active_tab = PrefTab::MailAccounts;
        self.narrow_show_content = true;
        self.account_editor_open = true;
        self.confirm_remove_in_editor = false;
        self.pending_leave_action = None;
        self.after_save_action = None;
        self.account_form.load_account(None, window, cx);
        self.app.update(cx, |app, _| {
            app.cfg_feedback = None;
        });
        cx.notify();
    }

    pub(crate) fn open_edit_account_editor(
        &mut self,
        cfg: &AccountConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_tab = PrefTab::MailAccounts;
        self.narrow_show_content = true;
        self.account_editor_open = true;
        self.confirm_remove_in_editor = false;
        self.pending_leave_action = None;
        self.after_save_action = None;
        self.account_form.load_account(Some(cfg), window, cx);
        self.app.update(cx, |app, _| {
            app.cfg_feedback = None;
        });
        cx.notify();
    }

    pub(crate) fn open_preset_account_editor(
        &mut self,
        provider_idx: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_tab = PrefTab::MailAccounts;
        self.narrow_show_content = true;
        self.account_editor_open = true;
        self.confirm_remove_in_editor = false;
        self.pending_leave_action = None;
        self.after_save_action = None;
        self.account_form
            .apply_provider_for_new(provider_idx, window, cx);
        self.app.update(cx, |app, _| {
            app.cfg_feedback = None;
        });
        cx.notify();
    }

    pub(crate) fn request_leave_editor(
        &mut self,
        action: EditorLeaveAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_account_editor_dirty(cx) {
            self.pending_leave_action = Some(action);
            self.confirm_remove_in_editor = false;
            cx.notify();
        } else {
            self.execute_leave_action(action, window, cx);
        }
    }

    /// 主窗口 / 账号列表请求编辑指定账号（任务二）：表单有未保存改动时先弹
    /// 「保存 / 放弃 / 继续编辑」确认，保存成功后再执行跳转。
    pub(crate) fn request_edit_account_editor(
        &mut self,
        account_id: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_leave_editor(EditorLeaveAction::EditAccount(account_id), window, cx);
    }

    /// 主窗口 / 欢迎页请求新建账号（任务二）：同样接入未保存确认。
    pub(crate) fn request_new_account_editor(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_leave_editor(EditorLeaveAction::NewAccount, window, cx);
    }

    pub(crate) fn request_preset_account_editor(
        &mut self,
        provider_idx: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_leave_editor(EditorLeaveAction::PresetAccount(provider_idx), window, cx);
    }

    fn execute_leave_action(
        &mut self,
        action: EditorLeaveAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending_leave_action = None;
        self.confirm_remove_in_editor = false;
        match action {
            EditorLeaveAction::BackToAccountList => {
                self.account_editor_open = false;
                self.after_save_action = None;
                self.account_form.load_account(None, window, cx);
                self.app.update(cx, |app, _| {
                    app.cfg_feedback = None;
                });
                cx.notify();
            }
            EditorLeaveAction::SwitchTab(tab) => {
                self.account_editor_open = false;
                self.after_save_action = None;
                self.active_tab = tab;
                self.narrow_show_content = true;
                self.confirm_clear_cache = false;
                // 系统页存储占用统计在每次进入时重新计算。
                self.storage_summary = None;
                if !self.mail_dir_switching {
                    self.pending_mail_dir_switch = None;
                    self.mail_dir_feedback = None;
                    self.current_mail_dir_importable.clear();
                }
                self.sender_feedback = None;
                self.compose_inputs_feedback = None;
                cx.notify();
            }
            EditorLeaveAction::ActivateSearchHit(item) => {
                self.account_editor_open = false;
                self.after_save_action = None;
                self.apply_search_hit_immediate(item, window, cx);
            }
            EditorLeaveAction::CloseWindow => {
                self.account_editor_open = false;
                self.after_save_action = None;
                self.force_close_window(window, cx);
            }
            EditorLeaveAction::EditAccount(account_id) => {
                let cfg = self
                    .app
                    .read(cx)
                    .accounts
                    .iter()
                    .find(|a| a.id == account_id)
                    .cloned();
                if let Some(cfg) = cfg {
                    self.open_edit_account_editor(&cfg, window, cx);
                } else {
                    // 账号已被移除等罕见竞态：保留当前界面。
                    cx.notify();
                }
            }
            EditorLeaveAction::NewAccount => {
                self.open_new_account_editor(window, cx);
            }
            EditorLeaveAction::PresetAccount(provider_idx) => {
                self.open_preset_account_editor(provider_idx, window, cx);
            }
        }
    }

    pub(crate) fn commit_account_editor_save(
        &mut self,
        after_action: Option<EditorLeaveAction>,
        cx: &mut Context<Self>,
    ) {
        if self.app.read(cx).demo_mode {
            return;
        }
        if let Err(msg) = self.account_form.validate_before_save(cx) {
            self.account_form.validation_error = Some(SharedString::from(msg));
            self.pending_leave_action = None;
            self.after_save_action = None;
            cx.notify();
            return;
        }
        self.account_form.validation_error = None;
        self.pending_leave_action = None;
        self.after_save_action = after_action;
        let existing = self.app.read(cx).accounts.clone();
        let (cfg, imap_pwd, smtp_pwd) = self.account_form.collect(&existing, cx);
        self.app.update(cx, |app, cx| {
            app.save_account_config(cfg, imap_pwd, smtp_pwd, cx);
        });
    }

    pub(crate) fn on_account_saved(
        &mut self,
        account: &AccountConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm_remove_in_editor = false;
        self.pending_leave_action = None;
        self.account_form.load_account(Some(account), window, cx);
        if self.active_tab == PrefTab::MailAccounts && self.account_editor_open {
            let next = self
                .after_save_action
                .take()
                .unwrap_or(EditorLeaveAction::BackToAccountList);
            self.execute_leave_action(next, window, cx);
        } else {
            self.after_save_action = None;
            cx.notify();
        }
    }

    pub(crate) fn on_account_deleted(
        &mut self,
        account_id: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.account_editor_open && self.account_form.editing_account_id == Some(account_id) {
            self.after_save_action = None;
            self.execute_leave_action(EditorLeaveAction::BackToAccountList, window, cx);
        } else {
            cx.notify();
        }
    }

    pub(crate) fn on_account_save_failed(&mut self, cx: &mut Context<Self>) {
        self.after_save_action = None;
        cx.notify();
    }

    pub(crate) fn select_tab(&mut self, tab: PrefTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_account_editor_dirty(cx) {
            self.request_leave_editor(EditorLeaveAction::SwitchTab(tab), window, cx);
            return;
        }
        self.execute_leave_action(EditorLeaveAction::SwitchTab(tab), window, cx);
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
        item: &'static PrefSearchItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_leave_editor(EditorLeaveAction::ActivateSearchHit(item), window, cx);
    }

    fn apply_search_hit_immediate(
        &mut self,
        item: &PrefSearchItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.active_tab = item.tab;
        if item.tab == PrefTab::System {
            self.storage_summary = None;
        }
        self.narrow_show_content = true;
        self.highlighted_row_id = Some(item.row_id);
        // 切换 Tab 后请求滚动：下一次渲染把锚点挂到高亮行，记录行位置后滚进视口。
        widgets::request_search_scroll();
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

    /// WP-05：把输入框中的邮箱或域名加入黑名单（非法输入给出行内反馈）。
    pub(crate) fn add_blocked_sender(
        &mut self,
        app_entity: &Entity<HylkiApp>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let raw = self.blocked_input.read(cx).value().trim().to_string();
        if raw.is_empty() {
            self.sender_feedback = Some((
                false,
                SharedString::from("请输入邮箱地址或域名（如 example.com）"),
            ));
            cx.notify();
            return;
        }
        let mut added = None;
        app_entity.update(cx, |app, cx| {
            app.mutate_preferences(cx, |prefs| {
                added = prefs.block_sender(&raw);
            });
        });
        if let Some(entry) = added {
            self.blocked_input
                .update(cx, |s, cx| s.set_value("", window, cx));
            self.sender_feedback = Some((true, SharedString::from(format!("已屏蔽：{entry}"))));
        } else {
            self.sender_feedback = Some((
                false,
                SharedString::from("请输入合法的邮箱地址或域名（如 example.com）"),
            ));
        }
        cx.notify();
    }

    /// WP-05：导出诊断信息文本（无日志文件，导出环境 / 配置摘要，邮箱地址
    /// 只保留域名以免泄露隐私）。保存到用户选择的位置。
    pub(crate) fn export_diagnostics_log(&mut self, cx: &mut Context<Self>) {
        let app = self.app.read(cx);
        let summary = config::app_storage_summary_in(&config::data_dir());
        let mut domains = 0usize;
        for acc in &app.accounts {
            if let Some((_, domain)) = acc.email.rsplit_once('@') {
                if !domain.is_empty() {
                    domains += 1;
                }
            }
        }
        let run_mode = if app.demo_mode {
            "演示模式（内存模拟）"
        } else if summary.is_portable {
            "便携版（EXE 同级 data 配置 + 分库存储）"
        } else {
            "安装版（LocalAppData 配置 + 分库存储）"
        };
        let mail_dir_kind = if summary.is_custom_mail_dir {
            "自定义"
        } else {
            "默认"
        };
        let report = format!(
            "Hylki Windows 诊断信息\n=======================\n\
             版本: v{}\n\
             运行模式: {}\n\
             操作系统: {} {}\n\
             程序配置目录: {}\n\
             邮件存储目录 ({}): {}\n\
             配置与全局索引占用: {}（{} 个文件）\n\
             邮件与附件占用: {}（{} 个文件）\n\
             已配置账号: {} 个（{} 个含邮箱域名）\n\
             偏好设置: 主题={} 明暗={:?} 列表布局={} 字号缩放={}%\n\
             自动同步: {} 分钟 推送={} 计费省流={}\n\
             黑名单发件人: {} 条\n\
             导出时间: {}\n",
            crate::app::app_version(),
            run_mode,
            std::env::consts::OS,
            std::env::consts::ARCH,
            summary.app_data_dir.display(),
            mail_dir_kind,
            summary.mail_data_dir.display(),
            config::format_bytes(summary.config_stats.total_bytes),
            summary.config_stats.file_count,
            config::format_bytes(summary.mail_stats.total_bytes),
            summary.mail_stats.file_count,
            app.accounts.len(),
            domains,
            app.prefs.theme_id,
            app.prefs.dark_mode,
            app.prefs.list_layout,
            app.prefs.ui_text_scale_percent,
            app.prefs.auto_sync_minutes,
            app.prefs.push_enabled,
            app.prefs.metered_save_data,
            app.prefs.blocked_senders.len(),
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        );
        if self.file_dialog_open {
            return;
        }
        self.file_dialog_open = true;
        cx.spawn(async move |this, cx| {
            let dest = cx
                .background_executor()
                .spawn(async move {
                    rfd::FileDialog::new()
                        .set_file_name("hylki-diagnostics.txt")
                        .add_filter("诊断信息文本 (*.txt)", &["txt"])
                        .save_file()
                })
                .await;
            let _ = this.update(cx, |this, _| this.file_dialog_open = false);
            if let Some(dest) = dest {
                let _ = this.update(cx, |this, cx| {
                    match std::fs::write(&dest, report) {
                        Ok(()) => {
                            this.backup_feedback = Some((
                                true,
                                SharedString::from(format!("诊断信息已导出至：{}", dest.display())),
                            ));
                        }
                        Err(e) => {
                            this.backup_feedback =
                                Some((false, SharedString::from(format!("写出诊断信息失败: {e}"))));
                        }
                    }
                    cx.notify();
                });
            }
        })
        .detach();
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

    pub(crate) fn open_dir_in_explorer(&mut self, path: &std::path::Path, cx: &mut Context<Self>) {
        if let Err(e) = std::fs::create_dir_all(path) {
            self.backup_feedback = Some((
                false,
                SharedString::from(format!("无法访问或创建目录 ({}): {e}", path.display())),
            ));
            cx.notify();
            return;
        }
        #[cfg(windows)]
        let res = std::process::Command::new("explorer.exe").arg(path).spawn();
        #[cfg(not(windows))]
        let res = std::process::Command::new("xdg-open").arg(path).spawn();
        if let Err(e) = res {
            self.backup_feedback = Some((
                false,
                SharedString::from(format!("打开文件管理器失败 ({}): {e}", path.display())),
            ));
            cx.notify();
        }
    }

    pub(crate) fn pick_mail_data_dir(&mut self, cx: &mut Context<Self>) {
        if self.file_dialog_open || self.mail_dir_switching {
            return;
        }
        self.file_dialog_open = true;
        cx.spawn(async move |this, cx| {
            let picked = cx
                .background_executor()
                .spawn(async move {
                    rfd::FileDialog::new()
                        .set_title("选择邮件存储目录")
                        .pick_folder()
                })
                .await;
            let _ = this.update(cx, |this, _| this.file_dialog_open = false);
            let Some(picked) = picked else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                let app_dir = config::data_dir();
                let current_dir = config::resolve_mail_data_dir_in(&app_dir);
                if let Err(err) = cache::validate_target_mail_dir(&current_dir, &picked) {
                    this.pending_mail_dir_switch = None;
                    this.mail_dir_feedback = Some((false, SharedString::from(err)));
                    cx.notify();
                    return;
                }
                let accounts = this.app.read(cx).accounts.clone();
                let inspection =
                    cache::inspect_mail_data_dir_with_app_dir(&app_dir, &picked, &accounts);
                this.mail_dir_feedback = None;
                this.pending_mail_dir_switch = Some((Some(picked), inspection));
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn request_reset_mail_data_dir(&mut self, cx: &mut Context<Self>) {
        if self.mail_dir_switching {
            return;
        }
        let app_dir = config::data_dir();
        let current_dir = config::resolve_mail_data_dir_in(&app_dir);
        let default_dir = config::default_mail_data_dir_in(&app_dir);
        if let Err(err) = cache::validate_target_mail_dir(&current_dir, &default_dir) {
            self.pending_mail_dir_switch = None;
            self.mail_dir_feedback = Some((false, SharedString::from(err)));
            cx.notify();
            return;
        }
        let accounts = self.app.read(cx).accounts.clone();
        let inspection =
            cache::inspect_mail_data_dir_with_app_dir(&app_dir, &default_dir, &accounts);
        self.mail_dir_feedback = None;
        self.pending_mail_dir_switch = Some((None, inspection));
        cx.notify();
    }

    pub(crate) fn confirm_mail_dir_switch(
        &mut self,
        mode: MailDirSwitchMode,
        import_discovered: bool,
        cx: &mut Context<Self>,
    ) {
        if self.mail_dir_switching {
            return;
        }
        let Some((target_opt, inspection)) = self.pending_mail_dir_switch.take() else {
            return;
        };
        let import_accounts = if import_discovered {
            inspection.importable_accounts
        } else {
            Vec::new()
        };
        self.mail_dir_switch_active = Some((mode, inspection.target_dir));
        self.mail_dir_switching = true;
        self.mail_dir_feedback = None;
        self.app.update(cx, |app, cx| {
            app.change_mail_data_dir(target_opt, mode, import_accounts, cx);
        });
        cx.notify();
    }

    pub(crate) fn import_current_mail_dir_accounts(&mut self, cx: &mut Context<Self>) {
        if self.mail_dir_switching || self.current_mail_dir_importable.is_empty() {
            return;
        }
        let to_import = self.current_mail_dir_importable.clone();
        let mut res = Ok(0usize);
        self.app.update(cx, |app, cx| {
            res = app.import_discovered_accounts(to_import, cx);
        });
        match res {
            Ok(added) if added > 0 => {
                self.storage_summary = None;
                self.current_mail_dir_importable.clear();
                self.mail_dir_feedback = Some((
                    true,
                    SharedString::from(format!(
                        "已导入 {added} 个关联账号配置，并自动复用本机已保存的邮箱凭据与本地邮件"
                    )),
                ));
            }
            Ok(_) => {
                self.storage_summary = None;
                self.current_mail_dir_importable.clear();
                self.mail_dir_feedback =
                    Some((true, SharedString::from("关联账号已存在于当前配置中")));
            }
            Err(err) => {
                self.mail_dir_feedback = Some((false, SharedString::from(err)));
            }
        }
        cx.notify();
    }

    pub(crate) fn cancel_mail_dir_switch(&mut self, cx: &mut Context<Self>) {
        if self.mail_dir_switching {
            return;
        }
        self.pending_mail_dir_switch = None;
        cx.notify();
    }

    pub(crate) fn on_mail_data_dir_changed(
        &mut self,
        ok: bool,
        _dir: &std::path::Path,
        message: &str,
        cx: &mut Context<Self>,
    ) {
        self.mail_dir_switching = false;
        self.pending_mail_dir_switch = None;
        self.mail_dir_switch_active = None;
        self.storage_summary = None;
        self.current_mail_dir_importable.clear();
        self.mail_dir_feedback = Some((ok, SharedString::from(message.to_string())));
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
        if self.file_dialog_open {
            return;
        }
        self.file_dialog_open = true;
        cx.spawn(async move |this, cx| {
            let dest = cx
                .background_executor()
                .spawn(async move {
                    rfd::FileDialog::new()
                        .set_file_name("hylki-settings-backup.toml")
                        .add_filter("Hylki 配置备份 (*.toml)", &["toml"])
                        .save_file()
                })
                .await;
            let _ = this.update(cx, |this, _| this.file_dialog_open = false);
            if let Some(dest) = dest {
                let _ = this.update(cx, |this, cx| {
                    match std::fs::write(&dest, exported) {
                        Ok(()) => {
                            this.backup_feedback = Some((
                                true,
                                SharedString::from(format!("配置备份已保存至：{}", dest.display())),
                            ));
                        }
                        Err(e) => {
                            this.backup_feedback =
                                Some((false, SharedString::from(format!("写出备份文件失败: {e}"))));
                        }
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(crate) fn pick_settings_backup(&mut self, cx: &mut Context<Self>) {
        if self.file_dialog_open {
            return;
        }
        self.file_dialog_open = true;
        cx.spawn(async move |this, cx| {
            let src = cx
                .background_executor()
                .spawn(async move {
                    rfd::FileDialog::new()
                        .add_filter("Hylki 配置备份 (*.toml)", &["toml"])
                        .pick_file()
                })
                .await;
            let _ = this.update(cx, |this, _| this.file_dialog_open = false);
            let Some(src) = src else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                let text = match std::fs::read_to_string(&src) {
                    Ok(t) => t,
                    Err(e) => {
                        this.backup_feedback =
                            Some((false, SharedString::from(format!("读取失败: {e}"))));
                        cx.notify();
                        return;
                    }
                };
                match config::parse_settings_bundle(&text) {
                    Ok(bundle) => {
                        this.pending_backup_import = Some((src, bundle));
                        this.backup_feedback = None;
                    }
                    Err(e) => {
                        this.pending_backup_import = None;
                        this.backup_feedback = Some((false, SharedString::from(e)));
                    }
                }
                cx.notify();
            });
        })
        .detach();
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

    fn force_close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.app.update(cx, |app, _| {
            app.clear_settings_window_ref();
        });
        window.remove_window();
    }

    fn close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mail_dir_switching {
            return;
        }
        self.request_leave_editor(EditorLeaveAction::CloseWindow, window, cx);
    }

    fn render_mail_dir_modal(
        &self,
        p: &HylkiPalette,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.mail_dir_switching {
            let (mode, target_dir) = self.mail_dir_switch_active.clone().unwrap_or_else(|| {
                (
                    MailDirSwitchMode::ReuseExisting,
                    config::resolve_mail_data_dir_in(&config::data_dir()),
                )
            });
            let title = match mode {
                MailDirSwitchMode::MigrateCurrent => "正在迁移邮件存储目录...",
                MailDirSwitchMode::ReuseExisting => "正在切换邮件存储目录...",
            };
            return Some(
                div()
                    .id("mail-dir-switch-modal")
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .bg(black().opacity(0.45))
                    .occlude()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                        cx.stop_propagation();
                    })
                    .child(
                        h_flex()
                            .size_full()
                            .items_center()
                            .justify_center()
                            .p_6()
                            .child(
                                v_flex()
                                    .w(px(520.0))
                                    .max_w(px(520.0))
                                    .bg(p.surface)
                                    .rounded(px(12.0))
                                    .border_1()
                                    .border_color(p.border)
                                    .shadow_xl()
                                    .p_5()
                                    .gap_4()
                                    .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                        cx.stop_propagation();
                                    })
                                    .child(
                                        h_flex()
                                            .items_center()
                                            .gap_2p5()
                                            .child(
                                                Icon::new(IconName::RefreshCw)
                                                    .with_size(px(18.0))
                                                    .text_color(p.accent),
                                            )
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(p.text)
                                                    .child(title),
                                            ),
                                    )
                                    .child(
                                        v_flex()
                                            .rounded(px(8.0))
                                            .bg(p.canvas)
                                            .border_1()
                                            .border_color(p.border)
                                            .px_3()
                                            .py_2p5()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(p.text_muted)
                                                    .child("目标存储目录"),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(p.text)
                                                    .child(target_dir.display().to_string()),
                                            ),
                                    )
                                    .child(div().text_xs().text_color(p.text_muted).child(
                                        "正在关闭旧连接、处理本地邮件库并重载主窗口，请勿关闭窗口…",
                                    )),
                            ),
                    )
                    .into_any_element(),
            );
        }

        let (ref target_opt, ref inspection) = self.pending_mail_dir_switch.as_ref()?;
        let current_dir = config::resolve_mail_data_dir_in(&config::data_dir());
        let has_existing = !inspection.existing_accounts.is_empty();
        let has_importable = !inspection.importable_accounts.is_empty();
        let importable_count = inspection.importable_accounts.len();
        let find_importable_email = |dir_name: &str| -> Option<String> {
            inspection
                .importable_accounts
                .iter()
                .find(|acc| {
                    config::resolve_account_storage_dir(acc, &inspection.importable_accounts)
                        .eq_ignore_ascii_case(dir_name)
                        || format!("account_{}", acc.id).eq_ignore_ascii_case(dir_name)
                })
                .map(|acc| acc.email.clone())
        };
        let has_unmatched_existing = inspection.existing_accounts.iter().any(|item| {
            item.matched_email.is_none() && find_importable_email(&item.dir_name).is_none()
        });
        let title = if target_opt.is_some() {
            "确认切换邮件存储目录"
        } else {
            "确认恢复默认邮件存储目录"
        };
        let desc = if has_existing && has_importable {
            format!(
                "目标目录中已检测到 {} 个账号的邮件库（共 {}），并在关联配置目录发现 {} 个当前尚未导入的账号配置。导入后将自动复用本机已保存的邮箱凭据并立即加载邮件：",
                inspection.existing_accounts.len(),
                config::format_bytes(inspection.total_bytes),
                importable_count
            )
        } else if has_importable {
            format!(
                "在目标目录关联配置中发现 {} 个当前尚未导入的账号配置。选择「切换并导入」可在切换目录的同时恢复账号配置：",
                importable_count
            )
        } else if has_existing {
            if has_unmatched_existing {
                format!(
                    "目标目录中已检测到 {} 个账号的邮件库（共 {}），但当前客户端未配置对应邮箱账号。需先在「邮箱账号」添加或通过「备份与恢复」导入同邮箱账号后，邮件才会在主窗体显示：",
                    inspection.existing_accounts.len(),
                    config::format_bytes(inspection.total_bytes)
                )
            } else {
                format!(
                    "目标目录中已检测到 {} 个账号的邮件库（共 {}）。请选择复用已有邮件数据，或将当前邮件迁移到此目录：",
                    inspection.existing_accounts.len(),
                    config::format_bytes(inspection.total_bytes)
                )
            }
        } else if inspection.current_has_mail_data {
            if inspection.has_any_entries {
                "目标目录非空，未检测到现成 Hylki 账号邮件库。可选择迁移当前全部邮件，或直接启用该目录并从服务器重新同步：".to_string()
            } else {
                "目标目录为空（就绪）。可选择迁移当前全部邮件，或直接启用该目录并从服务器重新同步："
                    .to_string()
            }
        } else if inspection.has_any_entries {
            "目标目录非空且无现成 Hylki 账号邮件库，当前邮件目录也暂无本地缓存邮件。切换后后续新邮件与缓存将保存到目标目录：".to_string()
        } else {
            "目标目录为空（就绪），当前邮件目录也暂无本地缓存邮件。切换后后续新邮件与缓存将保存到目标目录：".to_string()
        };

        let mut card = v_flex()
            .w(px(560.0))
            .max_w(px(560.0))
            .bg(p.surface)
            .rounded(px(12.0))
            .border_1()
            .border_color(p.border)
            .shadow_xl()
            .p_5()
            .gap_4()
            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                cx.stop_propagation();
            })
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .text_base()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.text)
                            .child(title),
                    )
                    .child(
                        Button::new("mail-dir-modal-close")
                            .ghost()
                            .compact()
                            .icon(IconName::Close)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.cancel_mail_dir_switch(cx);
                            })),
                    ),
            )
            .child(
                v_flex()
                    .rounded(px(8.0))
                    .bg(p.canvas)
                    .border_1()
                    .border_color(p.border)
                    .p_3()
                    .gap_2()
                    .child(
                        v_flex()
                            .gap_0p5()
                            .child(div().text_xs().text_color(p.text_muted).child("当前目录"))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(p.text)
                                    .child(current_dir.display().to_string()),
                            ),
                    )
                    .child(div().w_full().h(px(1.0)).bg(p.border))
                    .child(
                        v_flex()
                            .gap_0p5()
                            .child(div().text_xs().text_color(p.text_muted).child("目标目录"))
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(p.text)
                                    .child(inspection.target_dir.display().to_string()),
                            ),
                    ),
            )
            .child(div().text_xs().text_color(p.text_muted).child(desc));

        if has_existing {
            let mut acc_list = v_flex()
                .id("mail-dir-modal-accounts")
                .rounded(px(8.0))
                .bg(p.surface_raised)
                .border_1()
                .border_color(p.border)
                .px_3()
                .py_2()
                .gap_1p5()
                .max_h(px(180.0))
                .overflow_y_scrollbar();
            for item in &inspection.existing_accounts {
                let right_text = if let Some(ref email) = item.matched_email {
                    format!(
                        "匹配当前账号 {} · {}",
                        email,
                        config::format_bytes(item.total_bytes)
                    )
                } else if let Some(email) = find_importable_email(&item.dir_name) {
                    format!(
                        "可导入关联账号 {} · {}",
                        email,
                        config::format_bytes(item.total_bytes)
                    )
                } else {
                    format!(
                        "未关联本地账号 · {}",
                        config::format_bytes(item.total_bytes)
                    )
                };
                acc_list = acc_list.child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(p.text)
                                .child(format!("文件夹 {}", item.dir_name)),
                        )
                        .child(div().text_xs().text_color(p.text_muted).child(right_text)),
                );
            }
            card = card.child(acc_list);
        } else if has_importable {
            let mut acc_list = v_flex()
                .id("mail-dir-modal-importable")
                .rounded(px(8.0))
                .bg(p.surface_raised)
                .border_1()
                .border_color(p.border)
                .px_3()
                .py_2()
                .gap_1p5()
                .max_h(px(160.0))
                .overflow_y_scrollbar();
            for acc in &inspection.importable_accounts {
                acc_list = acc_list.child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(p.text)
                                .child(acc.email.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(p.text_muted)
                                .child("关联配置目录中待导入"),
                        ),
                );
            }
            card = card.child(acc_list);
        }

        let actions = if has_importable {
            h_flex()
                .flex_wrap()
                .justify_end()
                .gap_2()
                .child(
                    Button::new("mail-switch-cancel")
                        .outline()
                        .compact()
                        .label("取消")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.cancel_mail_dir_switch(cx);
                        })),
                )
                .when(inspection.current_has_mail_data, |el| {
                    el.child(
                        Button::new("mail-switch-migrate")
                            .outline()
                            .compact()
                            .disabled(self.mail_dir_switching)
                            .label("迁移当前邮件")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.confirm_mail_dir_switch(
                                    MailDirSwitchMode::MigrateCurrent,
                                    false,
                                    cx,
                                );
                            })),
                    )
                })
                .child(
                    Button::new("mail-switch-reuse-only")
                        .outline()
                        .compact()
                        .disabled(self.mail_dir_switching)
                        .label("仅切换目录")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.confirm_mail_dir_switch(
                                MailDirSwitchMode::ReuseExisting,
                                false,
                                cx,
                            );
                        })),
                )
                .child(
                    Button::new("mail-switch-reuse-import")
                        .primary()
                        .compact()
                        .disabled(self.mail_dir_switching)
                        .label(format!("切换并导入 {importable_count} 个账号"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.confirm_mail_dir_switch(
                                MailDirSwitchMode::ReuseExisting,
                                true,
                                cx,
                            );
                        })),
                )
        } else if has_existing {
            h_flex()
                .flex_wrap()
                .justify_end()
                .gap_2()
                .child(
                    Button::new("mail-switch-cancel")
                        .outline()
                        .compact()
                        .label("取消")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.cancel_mail_dir_switch(cx);
                        })),
                )
                .when(inspection.current_has_mail_data, |el| {
                    el.child(
                        Button::new("mail-switch-migrate")
                            .outline()
                            .compact()
                            .disabled(self.mail_dir_switching)
                            .label("迁移当前邮件到此目录")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.confirm_mail_dir_switch(
                                    MailDirSwitchMode::MigrateCurrent,
                                    false,
                                    cx,
                                );
                            })),
                    )
                })
                .child(
                    Button::new("mail-switch-reuse")
                        .primary()
                        .compact()
                        .disabled(self.mail_dir_switching)
                        .label("直接使用已有邮件数据（复用）")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.confirm_mail_dir_switch(
                                MailDirSwitchMode::ReuseExisting,
                                false,
                                cx,
                            );
                        })),
                )
        } else if inspection.current_has_mail_data {
            h_flex()
                .flex_wrap()
                .justify_end()
                .gap_2()
                .child(
                    Button::new("mail-switch-cancel")
                        .outline()
                        .compact()
                        .label("取消")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.cancel_mail_dir_switch(cx);
                        })),
                )
                .child(
                    Button::new("mail-switch-reuse")
                        .outline()
                        .compact()
                        .disabled(self.mail_dir_switching)
                        .label("直接使用该目录（不迁移）")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.confirm_mail_dir_switch(
                                MailDirSwitchMode::ReuseExisting,
                                false,
                                cx,
                            );
                        })),
                )
                .child(
                    Button::new("mail-switch-migrate")
                        .primary()
                        .compact()
                        .disabled(self.mail_dir_switching)
                        .label("确认迁移当前邮件到此目录")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.confirm_mail_dir_switch(
                                MailDirSwitchMode::MigrateCurrent,
                                false,
                                cx,
                            );
                        })),
                )
        } else {
            h_flex()
                .justify_end()
                .gap_2()
                .child(
                    Button::new("mail-switch-cancel")
                        .outline()
                        .compact()
                        .label("取消")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.cancel_mail_dir_switch(cx);
                        })),
                )
                .child(
                    Button::new("mail-switch-reuse")
                        .primary()
                        .compact()
                        .disabled(self.mail_dir_switching)
                        .label("确认切换到该目录")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.confirm_mail_dir_switch(
                                MailDirSwitchMode::ReuseExisting,
                                false,
                                cx,
                            );
                        })),
                )
        };
        card = card.child(actions);

        Some(
            div()
                .id("mail-dir-switch-modal")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .bg(black().opacity(0.45))
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.cancel_mail_dir_switch(cx);
                    }),
                )
                .child(
                    h_flex()
                        .size_full()
                        .items_center()
                        .justify_center()
                        .p_6()
                        .child(card),
                )
                .into_any_element(),
        )
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

    fn render_unsaved_banner(&self, p: &HylkiPalette, cx: &mut Context<Self>) -> AnyElement {
        let Some(action) = self.pending_leave_action else {
            return div().into_any_element();
        };
        let demo_mode = self.app.read(cx).demo_mode;
        h_flex()
            .w_full()
            .px_6()
            .py_2p5()
            .gap_3()
            .items_center()
            .justify_between()
            .bg(p.accent_soft())
            .border_b_1()
            .border_color(p.accent.opacity(0.4))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.text)
                            .child("当前账号有未保存的更改"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.text_muted)
                            .child("是否保存更改后再离开编辑页面？"),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .flex_shrink_0()
                    .child(
                        Button::new("acct-leave-cancel")
                            .outline()
                            .compact()
                            .label("取消")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.pending_leave_action = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("acct-leave-discard")
                            .danger()
                            .compact()
                            .label("放弃更改")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.execute_leave_action(action, window, cx);
                            })),
                    )
                    .when(!demo_mode, |el| {
                        el.child(
                            Button::new("acct-leave-save")
                                .primary()
                                .compact()
                                .label("保存")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.commit_account_editor_save(Some(action), cx);
                                })),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_content_header(
        &mut self,
        narrow: bool,
        p: &HylkiPalette,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let in_editor = self.active_tab == PrefTab::MailAccounts && self.account_editor_open;
        let demo_mode = self.app.read(cx).demo_mode;
        let can_remove = in_editor && self.account_form.editing_account_id.is_some() && !demo_mode;
        let can_save = in_editor && !demo_mode;
        let header_title: SharedString = if in_editor {
            if self.account_form.editing_account_id.is_some() {
                "编辑账号".into()
            } else {
                "添加账号".into()
            }
        } else {
            SharedString::from(self.active_tab.label())
        };

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
                    .when(in_editor, |el| {
                        el.child(
                            Button::new("acct-editor-back")
                                .ghost()
                                .compact()
                                .icon(IconName::ArrowLeft)
                                .label("返回")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.request_leave_editor(
                                        EditorLeaveAction::BackToAccountList,
                                        window,
                                        cx,
                                    );
                                })),
                        )
                    })
                    .when(!in_editor && narrow && self.narrow_show_content, |el| {
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
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .items_center()
                            .window_control_area(WindowControlArea::Drag)
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(p.text)
                                    .truncate()
                                    .child(header_title),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .items_center()
                    .gap_1p5()
                    .when(can_remove, |el| {
                        el.child(
                            Button::new("acct-editor-remove")
                                .danger()
                                .compact()
                                .icon(IconName::Trash)
                                .label("移除")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.confirm_remove_in_editor = !this.confirm_remove_in_editor;
                                    this.pending_leave_action = None;
                                    cx.notify();
                                })),
                        )
                    })
                    .when(can_save, |el| {
                        el.child(
                            Button::new("acct-editor-save")
                                .primary()
                                .compact()
                                .label("保存")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.commit_account_editor_save(
                                        Some(EditorLeaveAction::BackToAccountList),
                                        cx,
                                    );
                                })),
                        )
                    })
                    .child(self.render_window_controls(cx)),
            )
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
        let show_tab_subtitle = self.active_tab != PrefTab::MailAccounts;
        let show_unsaved_banner = self.active_tab == PrefTab::MailAccounts
            && self.account_editor_open
            && self.pending_leave_action.is_some();
        let unsaved_banner = if show_unsaved_banner {
            Some(self.render_unsaved_banner(&p, cx))
        } else {
            None
        };
        let content_area: AnyElement = if self.active_tab == PrefTab::MailAccounts {
            // 账号页不经 `render_tab`（行无统一高亮机制）：丢弃搜索滚动请求，避免悬挂到下次渲染。
            let _ = widgets::take_search_scroll_pending();
            let app_entity = self.app.clone();
            if self.account_editor_open {
                accounts::render_editor_page(self, app_entity, &p, cx)
            } else {
                accounts::render_list_page(self, app_entity, &p, cx)
            }
        } else {
            if self.active_tab == PrefTab::System && self.storage_summary.is_none() {
                let app_dir = config::data_dir();
                let summary = config::app_storage_summary_in(&app_dir);
                let accounts = self.app.read(cx).accounts.clone();
                let inspection = cache::inspect_mail_data_dir_with_app_dir(
                    &app_dir,
                    &summary.mail_data_dir,
                    &accounts,
                );
                self.current_mail_dir_importable = inspection.importable_accounts;
                self.storage_summary = Some(summary);
            }
            let app_entity = self.app.clone();
            pages::render_tab(self, app_entity, &p, window, cx)
        };

        let mail_dir_modal = self.render_mail_dir_modal(&p, cx);

        h_flex()
            .relative()
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
                if this.mail_dir_switching {
                    return;
                } else if this.pending_mail_dir_switch.is_some() {
                    this.cancel_mail_dir_switch(cx);
                } else if this.search_open {
                    this.close_search(window, cx);
                } else if this.confirm_remove_in_editor {
                    this.confirm_remove_in_editor = false;
                    cx.notify();
                } else if this.pending_leave_action.is_some() {
                    this.pending_leave_action = None;
                    cx.notify();
                } else if this.active_tab == PrefTab::MailAccounts && this.account_editor_open {
                    this.request_leave_editor(EditorLeaveAction::BackToAccountList, window, cx);
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
                        .when_some(unsaved_banner, |el, banner| el.child(banner))
                        .when(show_tab_subtitle, |el| {
                            el.child(
                                div()
                                    .px_6()
                                    .pt_3()
                                    .text_xs()
                                    .text_color(p.text_muted)
                                    .child(SharedString::from(self.active_tab.subtitle())),
                            )
                        })
                        .child(content_area),
                )
            })
            .when_some(mail_dir_modal, |el, modal| el.child(modal))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pref_tab_groups_and_sidebar_width_match_linux_spec() {
        assert_eq!(PrefTab::ALL.len(), 22);
        assert_eq!(
            PrefTab::ALL
                .iter()
                .filter(|t| t.group() == NavGroup::Accounts)
                .count(),
            9
        );
        assert_eq!(
            PrefTab::ALL
                .iter()
                .filter(|t| t.group() == NavGroup::Settings)
                .count(),
            13
        );
        assert_eq!(compute_sidebar_width(500.0), 180.0);
        assert!((compute_sidebar_width(920.0) - 248.4).abs() < 0.01);
        assert_eq!(compute_sidebar_width(1200.0), 250.0);
    }
}
