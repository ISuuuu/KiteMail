//! HylkiApp 全局状态机（从单文件原型 `main.rs` 迁入）。
//!
//! 职责：视图路由（[`RightPaneMode`]）、账号 / 文件夹 / 邮件状态、
//! WorkerEvent 轮询分发、写信与账号配置的动作逻辑，以及主窗口三栏
//! 组装（左栏 / 中栏由 `ui::` 模块渲染）。
//!
//! 本轮（迭代 1）只做「纯重构 + 视觉升级」，所有既有行为不变：
//! --demo、多账号同步、OAuth、搜索、分页、草稿、Outbox 重试、附件保存、
//! 标记已读 / 星标、账号增删配置全部保留。

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    actions, black, div, point, px, AnyWindowHandle, AppContext, ClipboardItem, Context, Entity,
    Focusable, InteractiveElement, IntoElement, MouseButton, ParentElement, Render, ScrollHandle,
    SharedString, Styled, Subscription, Window, WindowAppearance, WindowBounds,
};
use hylki_mail_core::cache::{CachedAttachment, KnownContact};
use hylki_mail_core::config::{self, AccountConfig, AppPreferences, AuthBackend, SecurityMode};
use hylki_mail_core::demo::{
    DemoStore, DEMO_ARCHIVE_ID, DEMO_DRAFTS_ID, DEMO_INBOX_ID, DEMO_SENT_ID,
};
use hylki_mail_core::worker::MailWorkerHandle;
use hylki_mail_core::{
    ComposeDraft, FolderSummary, LocalDraft, MailPage, MailRequest, MessageDetail, MessageFilter,
    MessageSort, MessageSummary, OutboxItem, QuickFilterSet, SearchScope, UnifiedView, WorkerEvent,
};

use crate::theme;
use crate::tray::{self, TrayCommand, TrayHandle};
use crate::ui::command_palette::{CommandPaletteEvent, CommandPaletteState, PaletteCommand};
use crate::ui::contacts::{ContactsEvent, ContactsState};
use crate::ui::gallery::{GalleryEvent, GalleryState};
use crate::ui::preferences::{PreferencesEvent, PreferencesState};
use crate::ui::{accounts, compose, message_list, reader, sidebar, welcome, window_controls};

pub(crate) const UNIFIED_INBOX_FOLDER_ID: u32 = 0;

/// 邮箱服务商预设（原样迁自原型）。
pub(crate) struct ProviderPreset {
    pub(crate) id: &'static str,
    pub(crate) label: &'static str,
    pub(crate) imap_host: &'static str,
    pub(crate) imap_port: &'static str,
    pub(crate) imap_tls: bool,
    pub(crate) smtp_host: &'static str,
    pub(crate) smtp_port: &'static str,
    pub(crate) smtp_tls: bool,
    pub(crate) guide: &'static str,
}

pub(crate) const PROVIDER_PRESETS: &[ProviderPreset] = &[
    ProviderPreset {
        id: "preset-qq",
        label: "QQ 邮箱",
        imap_host: "imap.qq.com",
        imap_port: "993",
        imap_tls: true,
        smtp_host: "smtp.qq.com",
        smtp_port: "465",
        smtp_tls: true,
        guide: "QQ 邮箱指引：登录网页版 QQ 邮箱 → 设置 → 账号 → 开启 IMAP/SMTP 服务，并生成 16 位授权码填在下方密码框（非 QQ 登录密码）。",
    },
    ProviderPreset {
        id: "preset-163",
        label: "网易 163",
        imap_host: "imap.163.com",
        imap_port: "993",
        imap_tls: true,
        smtp_host: "smtp.163.com",
        smtp_port: "465",
        smtp_tls: true,
        guide: "网易 163 指引：登录网页版 163 邮箱 → 设置 → POP3/SMTP/IMAP → 开启 IMAP/SMTP 服务，并填写新增的客户端授权密码。",
    },
    ProviderPreset {
        id: "preset-126",
        label: "网易 126",
        imap_host: "imap.126.com",
        imap_port: "993",
        imap_tls: true,
        smtp_host: "smtp.126.com",
        smtp_port: "465",
        smtp_tls: true,
        guide: "网易 126 指引：登录网页版 126 邮箱 → 设置 → POP3/SMTP/IMAP → 开启 IMAP/SMTP 服务，并填写客户端授权密码。",
    },
    ProviderPreset {
        id: "preset-gmail",
        label: "Gmail",
        imap_host: "imap.gmail.com",
        imap_port: "993",
        imap_tls: true,
        smtp_host: "smtp.gmail.com",
        smtp_port: "465",
        smtp_tls: true,
        guide: "Gmail 指引：需在 Google 账号开启「两步验证」，并在安全性设置中生成 16 位「应用专用密码 (App Password)」填入下方密码框（普通账号密码会被拒绝）。",
    },
    ProviderPreset {
        id: "preset-outlook",
        label: "Outlook",
        imap_host: "outlook.office365.com",
        imap_port: "993",
        imap_tls: true,
        smtp_host: "smtp.office365.com",
        smtp_port: "587",
        smtp_tls: false,
        guide: "Outlook / Hotmail 指引：IMAP 使用 TLS (993)，SMTP 使用 STARTTLS (587)。注意：微软目前已逐步停用个人/企业账号的传统密码登录，若验证失败通常需等待后续 OAuth2 登录支持。",
    },
    ProviderPreset {
        id: "preset-icloud",
        label: "iCloud",
        imap_host: "imap.mail.me.com",
        imap_port: "993",
        imap_tls: true,
        smtp_host: "smtp.mail.me.com",
        smtp_port: "587",
        smtp_tls: false,
        guide: "iCloud 指引：请前往 account.apple.com → 登录和安全 →「App 专用密码」生成专用密码填入下方，SMTP 使用 STARTTLS (587)。",
    },
    ProviderPreset {
        id: "preset-exmail",
        label: "腾讯企业邮",
        imap_host: "imap.exmail.qq.com",
        imap_port: "993",
        imap_tls: true,
        smtp_host: "smtp.exmail.qq.com",
        smtp_port: "465",
        smtp_tls: true,
        guide: "腾讯企业邮指引：若管理员开启了安全登录/微信绑定，请在企业邮箱设置 → 账号安全中生成客户端专用密码。",
    },
    ProviderPreset {
        id: "preset-aliyun",
        label: "阿里企业邮",
        imap_host: "imap.qiye.aliyun.com",
        imap_port: "993",
        imap_tls: true,
        smtp_host: "smtp.qiye.aliyun.com",
        smtp_port: "465",
        smtp_tls: true,
        guide: "阿里企业邮指引：请使用完整企业邮箱地址作为用户名；如开启了三方客户端安全密码，请在网页端生成专用密码。",
    },
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RightPaneMode {
    Reader,
    Compose,
    /// 首次启动欢迎向导（无账号且非 demo 时默认进入，方案文档五.1）。
    Welcome,
}

/// 左栏统一导航的当前视图（方案文档五.1）。
///
/// `AccountFolder` 表示正在浏览某个账号的具体文件夹（此时 `selected_folder_id`
/// 才参与列表请求）；其余视图为跨账号统一视图。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CurrentView {
    UnifiedInbox,
    Starred,
    Sent,
    Drafts,
    Archive,
    AccountFolder,
}

impl CurrentView {
    /// 对应的 mail-core 统一视图查询；`UnifiedInbox`/`Drafts` 走独立链路。
    pub(crate) fn unified(self) -> Option<UnifiedView> {
        match self {
            Self::Starred => Some(UnifiedView::Starred),
            Self::Sent => Some(UnifiedView::Sent),
            Self::Archive => Some(UnifiedView::Archive),
            Self::UnifiedInbox | Self::Drafts | Self::AccountFolder => None,
        }
    }

    /// 是否为跨账号统一视图（中栏多账号标签胶囊的显示条件）。
    pub(crate) fn is_unified(self) -> bool {
        !matches!(self, Self::AccountFolder)
    }
}

/// 中栏列表客户端过滤（对当前页数据）。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ListFilter {
    All,
    Unread,
    Starred,
}

impl ListFilter {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::All => "全部",
            Self::Unread => "仅未读",
            Self::Starred => "仅星标",
        }
    }

    pub(crate) fn to_mail_filter(self) -> MessageFilter {
        match self {
            Self::All => MessageFilter::All,
            Self::Unread => MessageFilter::Unread,
            Self::Starred => MessageFilter::Starred,
        }
    }
}

// 全局快捷键动作（方案文档五.3）；键绑定注册在 main.rs，
// 动作监听挂在根节点 render 上。
actions!(
    hylki,
    [
        NewCompose,
        OpenCommandPalette,
        FocusSearch,
        RefreshAllAccounts,
        CloseOverlay,
        SendCompose,
        OpenPreferences,
        QuitApp,
        DeleteSelectedMessage,
        SelectNextMessage,
        SelectPrevMessage
    ]
);

/// 写信页当前激活联系人自动补全的地址输入行。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ComposeAddressField {
    To,
    Cc,
    Bcc,
}

/// 撤销发送（Undo Send）5 秒缓冲中的待投递邮件。
#[derive(Clone, Debug)]
pub(crate) struct PendingSend {
    pub(crate) token: u64,
    pub(crate) account_id: u32,
    pub(crate) draft: ComposeDraft,
    pub(crate) local_draft_id: Option<u32>,
    pub(crate) remaining_secs: u8,
}

/// 主内容区路由（方案文档五.1）：`Mail` 默认三栏，`Gallery` 附件总览，
/// `Contacts` 联系人（后两者保留左栏，中栏 + 右栏整体替换）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MainContentView {
    Mail,
    Gallery,
    Contacts,
}

/// 当前全局浮层（方案文档五.1）：账号管理、偏好设置、命令面板均以居中模态展示。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ActiveModal {
    None,
    Accounts,
    Preferences,
    CommandPalette,
}

pub(crate) struct HylkiApp {
    pub(crate) demo_mode: bool,
    pub(crate) demo_store: Arc<DemoStore>,
    pub(crate) worker: Option<Arc<MailWorkerHandle>>,
    pub(crate) accounts: Vec<AccountConfig>,
    pub(crate) folders_by_account: BTreeMap<u32, Vec<FolderSummary>>,
    /// `0` means Unified Inbox across all accounts.
    pub(crate) selected_folder_id: u32,
    /// 左栏统一导航当前视图（决定列表请求走统一视图还是具体文件夹）。
    pub(crate) current_view: CurrentView,
    /// 当前列表已展开的总条数上限（初始为偏好 `page_size`，滚到底部每次递增一批）。
    pub(crate) loaded_limit: usize,
    /// 中栏邮件列表滚动句柄（用于检测滚动到底部自动预加载下一批）。
    pub(crate) list_scroll_handle: ScrollHandle,
    /// 正在从本地缓存增量追加下一批邮件（防止重复触发）。
    pub(crate) loading_more_local: bool,
    /// 正在从远端服务器（IMAP / Graph）预加载更早历史邮件（防止重复触发）。
    pub(crate) loading_more_remote: bool,
    pub(crate) current_page: MailPage,
    pub(crate) selected_message: Option<MessageDetail>,
    /// Ctrl+点击多选的邮件 ID 集合。
    pub(crate) selected_message_ids: HashSet<u32>,
    pub(crate) drafts: Vec<LocalDraft>,
    pub(crate) outbox_by_account: BTreeMap<u32, Vec<OutboxItem>>,
    pub(crate) right_mode: RightPaneMode,
    pub(crate) search_input: Entity<InputState>,
    pub(crate) search_query: String,

    // Compose / Reply fields
    pub(crate) compose_account_id: u32,
    pub(crate) compose_draft_id: Option<u32>,
    pub(crate) compose_to: Entity<InputState>,
    pub(crate) compose_cc: Entity<InputState>,
    pub(crate) compose_bcc: Entity<InputState>,
    /// Cc / Bcc 输入行是否展开（默认折叠，方案文档页面 3）。
    pub(crate) compose_show_cc_bcc: bool,
    pub(crate) compose_subject: Entity<InputState>,
    pub(crate) compose_body: Entity<TextareaState>,
    pub(crate) compose_in_reply_to: String,
    pub(crate) compose_attachments: Vec<String>,
    /// 写信退出前未保存确认条显示状态。
    pub(crate) compose_confirm_discard: bool,
    /// 当前正在等待导出原附件的转发邮件 ID。
    pub(crate) pending_forward_msg_id: Option<u32>,
    /// 当前触发联系人自动补全的地址框。
    pub(crate) compose_autocomplete_field: Option<ComposeAddressField>,
    /// 已知联系人本地缓存（供写信收件人自动补全）。
    pub(crate) known_contacts: Vec<KnownContact>,
    /// 撤销发送（5 秒缓冲）状态。
    pub(crate) pending_send: Option<PendingSend>,
    next_send_token: u64,

    // Account configuration fields
    pub(crate) editing_account_id: Option<u32>,
    /// 账号管理页等待确认移除的账号 ID（内联二次确认）。
    pub(crate) pending_delete_account_id: Option<u32>,
    pub(crate) cfg_display_name: Entity<InputState>,
    pub(crate) cfg_email: Entity<InputState>,
    pub(crate) cfg_imap_host: Entity<InputState>,
    pub(crate) cfg_imap_port: Entity<InputState>,
    pub(crate) cfg_imap_tls: bool,
    pub(crate) cfg_imap_user: Entity<InputState>,
    pub(crate) cfg_imap_pass: Entity<InputState>,
    pub(crate) cfg_smtp_host: Entity<InputState>,
    pub(crate) cfg_smtp_port: Entity<InputState>,
    pub(crate) cfg_smtp_tls: bool,
    pub(crate) cfg_smtp_separate: bool,
    pub(crate) cfg_smtp_user: Entity<InputState>,
    pub(crate) cfg_smtp_pass: Entity<InputState>,
    pub(crate) cfg_oauth_client_id: Entity<InputState>,
    pub(crate) cfg_signature: Entity<TextareaState>,
    pub(crate) cfg_auth_guide: Option<SharedString>,
    pub(crate) cfg_feedback: Option<(bool, SharedString)>,

    pub(crate) status_line: SharedString,
    pub(crate) error_banner: Option<SharedString>,

    // 迭代 1 新增：纯视觉状态（不影响后端行为）
    /// 当前主题（`theme::THEME_LIST` 中的 id）。
    pub(crate) theme_id: &'static str,
    /// 亮 / 暗模式（菜单「切换亮/暗主题」即时生效）。
    pub(crate) dark_mode: bool,
    /// 系统当前的深浅色外观（由 `window.appearance()` 检测，供 `dark_mode: None` 跟随系统使用）。
    pub(crate) system_dark_mode: bool,
    /// 左栏是否隐藏（中栏顶部开关）。
    pub(crate) sidebar_hidden: bool,
    /// 中栏列表客户端过滤（向后兼容单选枚举）。
    pub(crate) list_filter: ListFilter,
    /// 列表排序方式（6 种，默认最新优先）。
    pub(crate) list_sort: MessageSort,
    /// 多条件快捷筛选（仅未读 / 仅星标 / 含附件 / 今天）。
    pub(crate) quick_filters: QuickFilterSet,
    /// 本地搜索范围（当前视图 / 全部文件夹）。
    pub(crate) search_scope: SearchScope,
    /// 服务端在线搜索状态与远端命中缓存。
    pub(crate) server_search_loading: bool,
    pub(crate) server_search_query: String,
    pub(crate) server_search_results: Vec<MessageSummary>,
    /// 账号分组折叠状态（缺省展开）。
    pub(crate) collapsed_accounts: HashSet<u32>,
    /// 阅读器是否强制以纯文本渲染（false = 有 HTML 时渲染富文本）。
    pub(crate) reader_prefer_plain: bool,
    /// 本次会话中用户手动放行远程图片的邮件 ID 集合。
    pub(crate) allowed_remote_image_ids: HashSet<u32>,
    /// 当前会话阅读器中处于折叠状态的卡片邮件 ID 集合。
    pub(crate) collapsed_thread_cards: HashSet<u32>,
    /// 当前会话阅读器中处于「查看邮件原文」模式的卡片邮件 ID 集合。
    pub(crate) raw_source_message_ids: HashSet<u32>,

    // ---- 阶段 3：全局路由 / 模态 / 偏好（方案文档五.1、五.2）----
    /// 持久化偏好副本（preferences.toml；last_view / last_folder 随切换落盘）。
    pub(crate) prefs: AppPreferences,
    /// 主内容区当前视图（Mail 三栏 / Gallery / Contacts）。
    pub(crate) main_view: MainContentView,
    /// 当前全局浮层。
    pub(crate) active_modal: ActiveModal,
    /// 偏好设置模态实体（关闭时卸载）。
    preferences: Option<Entity<PreferencesState>>,
    preferences_sub: Option<Subscription>,
    /// 命令面板浮层实体（关闭时卸载）。
    palette: Option<Entity<CommandPaletteState>>,
    palette_sub: Option<Subscription>,
    /// 附件总览视图实体（首次进入时懒创建，常驻）。
    gallery: Option<Entity<GalleryState>>,
    gallery_sub: Option<Subscription>,
    /// 联系人视图实体（同上）。
    contacts: Option<Entity<ContactsState>>,
    contacts_sub: Option<Subscription>,
    /// 附件总览最近一次查询结果副本（供保存 / 定位原邮件）。
    gallery_items: Vec<CachedAttachment>,
    /// 附件总览当前搜索词（QueryChanged 时更新，重进视图重新请求）。
    gallery_query: String,
    /// 联系人当前搜索词（同上）。
    contacts_query: String,
    /// 后台自动刷新到期时刻（`None` 表示待重算；demo 模式不轮询）。
    auto_sync_deadline: Option<Instant>,
    /// Windows 系统托盘句柄（退出时显式销毁图标避免残留）。
    tray: Option<Arc<TrayHandle>>,
    /// 主窗口句柄（供托盘菜单触发写信 / 退出前保存窗口几何）。
    window_handle: AnyWindowHandle,
    /// 各账号最近一次收件箱未读数基线（首次加载只建基线不弹窗，后续增长才触发桌面通知）。
    last_unread_by_account: BTreeMap<u32, u32>,
}

impl HylkiApp {
    pub(crate) fn new(
        demo_mode: bool,
        prefs: AppPreferences,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let demo_store = Arc::new(DemoStore::new());
        let window_handle = window.window_handle();
        let tray = TrayHandle::start(crate::APP_ICON_PNG).map(Arc::new);
        window.set_window_title("Hylki");
        // 点右上角关闭按钮时先保存窗口尺寸，再按偏好隐藏到系统托盘（不直接退出），
        // 真正退出由菜单或托盘右键「退出 Hylki」触发。
        let weak_self = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            if let Some(app) = weak_self.upgrade() {
                let should_hide = app.update(cx, |this, _| {
                    this.save_window_geometry(window);
                    this.prefs.close_to_minimize
                });
                if should_hide {
                    let hidden = app
                        .read(cx)
                        .tray
                        .as_ref()
                        .is_some_and(|t| t.hide_main_window());
                    if !hidden {
                        window.minimize_window();
                    }
                    return false;
                }
                app.read(cx).shutdown_tray();
            }
            true
        });

        // 轮询 Windows 系统托盘命令，并在 minimize_to_tray 开启时将最小化窗口自动隐藏到托盘。
        if let Some(ref tray_handle) = tray {
            let weak_tray = Arc::downgrade(tray_handle);
            cx.spawn(async move |this, cx| loop {
                cx.background_executor()
                    .timer(Duration::from_millis(80))
                    .await;
                let Some(tray_handle) = weak_tray.upgrade() else {
                    break;
                };
                let cmds = tray_handle.drain_commands();
                let minimized = tray_handle.is_main_window_minimized();
                if cmds.is_empty() && !minimized {
                    continue;
                }
                let res = this.update(cx, |app, cx| {
                    if minimized && app.prefs.minimize_to_tray {
                        tray_handle.hide_main_window();
                    }
                    for cmd in cmds {
                        app.handle_tray_command(cmd, cx);
                    }
                });
                if res.is_err() {
                    break;
                }
            })
            .detach();
        }
        let data_dir = config::data_dir();
        let existing_accounts = if demo_mode {
            Vec::new()
        } else {
            config::load_accounts_in(&data_dir).unwrap_or_default()
        };

        // 方案文档五.2：default_view == "last" 时恢复上次浏览的文件夹 / 统一视图
        //（演示模式忽略恢复，仍从统一收件箱开始）。
        let mut start_view = CurrentView::UnifiedInbox;
        let mut start_folder_id = UNIFIED_INBOX_FOLDER_ID;
        if prefs.default_view == "last" && !demo_mode {
            if let Some((account_id, folder_id)) = prefs.last_folder {
                if existing_accounts.iter().any(|a| a.id == account_id) {
                    start_view = CurrentView::AccountFolder;
                    start_folder_id = folder_id;
                }
            } else if let Some(view) = Self::current_view_from_last(prefs.last_view.as_deref()) {
                start_view = view;
            }
        }

        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("搜索主题、发件人、正文（支持 from: / has:attachment）...")
        });

        cx.subscribe(&search_input, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.search_query = input.read(cx).value().to_string();
                this.clear_server_search();
                this.reset_list_pagination();
                this.reload_current_page(cx);
            }
        })
        .detach();

        let compose_to =
            cx.new(|cx| InputState::new(window, cx).placeholder("收件人：name@example.com"));
        let compose_cc = cx.new(|cx| {
            InputState::new(window, cx).placeholder("抄送：cc1@example.com, cc2@example.com")
        });
        let compose_bcc =
            cx.new(|cx| InputState::new(window, cx).placeholder("密送：bcc@example.com"));
        cx.subscribe(&compose_to, |this, _input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.compose_autocomplete_field = Some(ComposeAddressField::To);
                cx.notify();
            }
        })
        .detach();
        cx.subscribe(&compose_cc, |this, _input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.compose_autocomplete_field = Some(ComposeAddressField::Cc);
                cx.notify();
            }
        })
        .detach();
        cx.subscribe(&compose_bcc, |this, _input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.compose_autocomplete_field = Some(ComposeAddressField::Bcc);
                cx.notify();
            }
        })
        .detach();
        let compose_subject = cx.new(|cx| {
            InputState::new(window, cx).placeholder("主题：输入邮件主题（支持中文 IME）")
        });
        let compose_body = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(10, 24)
                .placeholder("在此输入多行中文正文，支持保存草稿、附件与持久化发件...")
        });

        let first_cfg = existing_accounts.first().cloned().unwrap_or_default();
        let editing_account_id = existing_accounts.first().map(|c| c.id);

        let cfg_display_name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("显示名称（如：张三）")
                .default_value(first_cfg.display_name.clone())
        });
        let cfg_email = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("邮箱地址（如：user@qq.com）")
                .default_value(first_cfg.email.clone())
        });
        let cfg_imap_host = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("IMAP 主机（如：imap.qq.com）")
                .default_value(first_cfg.imap_host.clone())
        });
        let cfg_imap_port = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("993")
                .default_value(first_cfg.imap_port.to_string())
        });
        let cfg_imap_user = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("IMAP 用户名（留空默认使用邮箱地址）")
                .default_value(first_cfg.imap_username.clone())
        });
        let cfg_imap_pass = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("客户端授权码 / 密码（存入 Windows 凭据管理器）")
        });
        let cfg_smtp_host = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("SMTP 主机（如：smtp.qq.com）")
                .default_value(first_cfg.smtp_host.clone())
        });
        let cfg_smtp_port = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("465")
                .default_value(first_cfg.smtp_port.to_string())
        });
        let cfg_smtp_user = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("SMTP 独立用户名（可选）")
                .default_value(first_cfg.smtp_username.clone())
        });
        let cfg_smtp_pass = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("SMTP 独立授权码（启用独立凭据时填写）")
        });
        let cfg_oauth_client_id = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("可选：自定义 Microsoft Entra Client ID（留空使用内置）")
                .default_value(first_cfg.oauth_client_id.clone())
        });
        let cfg_signature = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(3, 6)
                .placeholder("可选：邮件签名（写信 / 回复 / 转发时自动以 -- 分隔插入正文末尾）")
                .default_value(first_cfg.signature.clone())
        });

        let system_dark_mode = matches!(
            window.appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        );
        let initial_theme_id = theme::static_theme_id(&prefs.theme_id);
        let initial_dark_mode = prefs.dark_mode.unwrap_or(system_dark_mode);
        // 当偏好为「跟随系统」且系统是暗色时，补一次主题应用对齐 Windows 外观。
        if prefs.dark_mode.is_none() && system_dark_mode {
            theme::apply(initial_theme_id, true, cx);
        }

        if demo_mode {
            let demo_contacts = demo_store.list_known_contacts("", 0, 500);
            let current_page = demo_store.list_messages_page(DEMO_INBOX_ID, "", 0, prefs.page_size);
            let mut selected_message = current_page
                .items
                .first()
                .and_then(|msg| demo_store.get_message_detail(msg.id));
            if !prefs.thread_newest_first {
                if let Some(ref mut d) = selected_message {
                    d.thread_messages.reverse();
                }
            }
            let initial_collapsed = selected_message
                .as_ref()
                .map(|d| compute_default_collapsed_cards(d, prefs.thread_newest_first, prefs.fold_older_messages))
                .unwrap_or_default();
            let mut folders_by_account = BTreeMap::new();
            folders_by_account.insert(1, demo_store.folders.clone());
            let demo_acc = AccountConfig {
                id: 1,
                display_name: demo_store.accounts[0].display_name.clone(),
                email: demo_store.accounts[0].email.clone(),
                ..Default::default()
            };
            return Self {
                demo_mode: true,
                demo_store,
                worker: None,
                accounts: vec![demo_acc],
                folders_by_account,
                selected_folder_id: DEMO_INBOX_ID,
                current_view: CurrentView::UnifiedInbox,
                loaded_limit: prefs.page_size,
                list_scroll_handle: ScrollHandle::new(),
                loading_more_local: false,
                loading_more_remote: false,
                current_page,
                selected_message,
                selected_message_ids: HashSet::new(),
                drafts: Vec::new(),
                outbox_by_account: BTreeMap::new(),
                right_mode: RightPaneMode::Reader,
                search_input,
                search_query: String::new(),
                compose_account_id: 1,
                compose_draft_id: None,
                compose_to,
                compose_cc,
                compose_bcc,
                compose_show_cc_bcc: false,
                compose_subject,
                compose_body,
                compose_in_reply_to: String::new(),
                compose_attachments: Vec::new(),
                compose_confirm_discard: false,
                pending_forward_msg_id: None,
                compose_autocomplete_field: None,
                known_contacts: demo_contacts,
                pending_send: None,
                next_send_token: 1,
                editing_account_id: Some(1),
                pending_delete_account_id: None,
                cfg_display_name,
                cfg_email,
                cfg_imap_host,
                cfg_imap_port,
                cfg_imap_tls: true,
                cfg_imap_user,
                cfg_imap_pass,
                cfg_smtp_host,
                cfg_smtp_port,
                cfg_smtp_tls: true,
                cfg_smtp_separate: false,
                cfg_smtp_user,
                cfg_smtp_pass,
                cfg_oauth_client_id,
                cfg_signature,
                cfg_auth_guide: None,
                cfg_feedback: None,
                status_line: SharedString::from("演示模式 (--demo)：已加载 10,000 封模拟邮件"),
                error_banner: None,
                theme_id: initial_theme_id,
                dark_mode: initial_dark_mode,
                system_dark_mode,
                sidebar_hidden: false,
                list_filter: ListFilter::All,
                list_sort: MessageSort::DateNewest,
                quick_filters: QuickFilterSet::default(),
                search_scope: SearchScope::ThisFolder,
                server_search_loading: false,
                server_search_query: String::new(),
                server_search_results: Vec::new(),
                collapsed_accounts: HashSet::new(),
                reader_prefer_plain: false,
                allowed_remote_image_ids: HashSet::new(),
                collapsed_thread_cards: initial_collapsed,
                raw_source_message_ids: HashSet::new(),
                prefs,
                main_view: MainContentView::Mail,
                active_modal: ActiveModal::None,
                preferences: None,
                preferences_sub: None,
                palette: None,
                palette_sub: None,
                gallery: None,
                gallery_sub: None,
                contacts: None,
                contacts_sub: None,
                gallery_items: Vec::new(),
                gallery_query: String::new(),
                contacts_query: String::new(),
                auto_sync_deadline: None,
                tray,
                window_handle,
                last_unread_by_account: BTreeMap::new(),
            };
        }

        let worker = MailWorkerHandle::spawn(data_dir).ok().map(Arc::new);
        if let Some(ref w) = worker {
            if !existing_accounts.is_empty() {
                w.send(MailRequest::LoadAllCached);
                w.send(MailRequest::ListKnownContacts {
                    query: String::new(),
                    offset: 0,
                    limit: 500,
                });
                w.send(MailRequest::SyncAllAccounts);
            }
            let weak_worker = Arc::downgrade(w);
            cx.spawn(async move |this, cx| loop {
                cx.background_executor()
                    .timer(Duration::from_millis(80))
                    .await;
                let Some(w) = weak_worker.upgrade() else {
                    break;
                };
                let events = w.drain_events();
                if !events.is_empty() {
                    let res = this.update(cx, |app, cx| {
                        app.apply_worker_events(events, cx);
                    });
                    if res.is_err() {
                        break;
                    }
                }
            })
            .detach();

            // 方案文档五.2：后台定时刷新——每 30 秒醒来检查一次到期状态，
            // 间隔由偏好 auto_sync_minutes 控制（0 = 手动），demo 模式不轮询。
            cx.spawn(async move |this, cx| loop {
                cx.background_executor()
                    .timer(Duration::from_secs(30))
                    .await;
                let res = this.update(cx, |app, _cx| app.auto_sync_tick());
                if res.is_err() {
                    break;
                }
            })
            .detach();
        }

        let has_account = !existing_accounts.is_empty();
        let compose_account_id = existing_accounts.first().map(|a| a.id).unwrap_or(1);
        let status_line = if has_account {
            SharedString::from("正在从本地缓存加载并同步所有账号...")
        } else {
            SharedString::from("请先添加至少一个 IMAP / SMTP 邮箱账号")
        };

        let mut app = Self {
            demo_mode: false,
            demo_store,
            worker,
            accounts: existing_accounts,
            folders_by_account: BTreeMap::new(),
            selected_folder_id: start_folder_id,
            current_view: start_view,
            loaded_limit: prefs.page_size,
            list_scroll_handle: ScrollHandle::new(),
            loading_more_local: false,
            loading_more_remote: false,
            current_page: MailPage {
                folder_id: UNIFIED_INBOX_FOLDER_ID,
                offset: 0,
                total: 0,
                items: Vec::new(),
            },
            selected_message: None,
            selected_message_ids: HashSet::new(),
            drafts: Vec::new(),
            outbox_by_account: BTreeMap::new(),
            // 方案文档页面 2：启动且无账号（非 demo）时进入欢迎向导。
            right_mode: if has_account {
                RightPaneMode::Reader
            } else {
                RightPaneMode::Welcome
            },
            search_input,
            search_query: String::new(),
            compose_account_id,
            compose_draft_id: None,
            compose_to,
            compose_cc,
            compose_bcc,
            compose_show_cc_bcc: false,
            compose_subject,
            compose_body,
            compose_in_reply_to: String::new(),
            compose_attachments: Vec::new(),
            compose_confirm_discard: false,
            pending_forward_msg_id: None,
            compose_autocomplete_field: None,
            known_contacts: Vec::new(),
            pending_send: None,
            next_send_token: 1,
            editing_account_id,
            pending_delete_account_id: None,
            cfg_display_name,
            cfg_email,
            cfg_imap_host,
            cfg_imap_port,
            cfg_imap_tls: first_cfg.imap_security == SecurityMode::Tls,
            cfg_imap_user,
            cfg_imap_pass,
            cfg_smtp_host,
            cfg_smtp_port,
            cfg_smtp_tls: first_cfg.smtp_security == SecurityMode::Tls,
            cfg_smtp_separate: first_cfg.smtp_separate_credentials,
            cfg_smtp_user,
            cfg_smtp_pass,
            cfg_oauth_client_id,
            cfg_signature,
            cfg_auth_guide: None,
            cfg_feedback: None,
            status_line,
            error_banner: None,
            theme_id: initial_theme_id,
            dark_mode: initial_dark_mode,
            system_dark_mode,
            sidebar_hidden: false,
            list_filter: ListFilter::All,
            list_sort: MessageSort::DateNewest,
            quick_filters: QuickFilterSet::default(),
            search_scope: SearchScope::ThisFolder,
            server_search_loading: false,
            server_search_query: String::new(),
            server_search_results: Vec::new(),
            collapsed_accounts: HashSet::new(),
            reader_prefer_plain: false,
            allowed_remote_image_ids: HashSet::new(),
            collapsed_thread_cards: HashSet::new(),
            raw_source_message_ids: HashSet::new(),
            prefs,
            main_view: MainContentView::Mail,
            active_modal: ActiveModal::None,
            preferences: None,
            preferences_sub: None,
            palette: None,
            palette_sub: None,
            gallery: None,
            gallery_sub: None,
            contacts: None,
            contacts_sub: None,
            gallery_items: Vec::new(),
            gallery_query: String::new(),
            contacts_query: String::new(),
            auto_sync_deadline: None,
            tray,
            window_handle,
            last_unread_by_account: BTreeMap::new(),
        };

        // 启动视图非统一收件箱时立即请求对应列表（LoadAllCached 只覆盖收件箱）。
        if app.current_view != CurrentView::UnifiedInbox {
            app.reload_current_page(cx);
        }
        app
    }

    /// 把持久化的 last_view 字符串映射回统一视图（未知值回落 None）。
    fn current_view_from_last(last: Option<&str>) -> Option<CurrentView> {
        match last? {
            "unified" => Some(CurrentView::UnifiedInbox),
            "starred" => Some(CurrentView::Starred),
            "sent" => Some(CurrentView::Sent),
            "drafts" => Some(CurrentView::Drafts),
            "archive" => Some(CurrentView::Archive),
            _ => None,
        }
    }

    fn apply_worker_events(&mut self, events: Vec<WorkerEvent>, cx: &mut Context<Self>) {
        let mut folders_updated = false;
        let mut total_new_unread: u32 = 0;
        let mut notify_account_id: Option<u32> = None;
        let mut latest_unread_preview: Option<(String, String)> = None;

        for evt in events {
            match evt {
                WorkerEvent::AccountVerified { ok, message } => {
                    self.cfg_feedback = Some((ok, SharedString::from(message.clone())));
                    self.status_line = SharedString::from(message);
                }
                WorkerEvent::AccountSaved { account } => {
                    if let Some(existing) = self.accounts.iter_mut().find(|a| a.id == account.id) {
                        *existing = account.clone();
                    } else {
                        self.accounts.push(account.clone());
                    }
                    self.editing_account_id = Some(account.id);
                    self.compose_account_id = account.id;
                    self.pending_delete_account_id = None;
                    self.right_mode = RightPaneMode::Reader;
                    if self.active_modal == ActiveModal::Accounts {
                        self.active_modal = ActiveModal::None;
                    }
                    self.error_banner = None;
                    self.status_line =
                        SharedString::from(format!("账号 {} 已保存，正在同步...", account.email));
                }
                WorkerEvent::AccountDeleted { account_id } => {
                    self.accounts.retain(|a| a.id != account_id);
                    self.folders_by_account.remove(&account_id);
                    self.outbox_by_account.remove(&account_id);
                    self.last_unread_by_account.remove(&account_id);
                    folders_updated = true;
                    self.selected_folder_id = UNIFIED_INBOX_FOLDER_ID;
                    self.current_view = CurrentView::UnifiedInbox;
                    self.selected_message = None;
                    self.pending_delete_account_id = None;
                    self.editing_account_id = self.accounts.first().map(|a| a.id);
                    self.right_mode = if self.accounts.is_empty() {
                        RightPaneMode::Welcome
                    } else {
                        RightPaneMode::Reader
                    };
                    self.status_line = SharedString::from("账号及本地缓存已移除");
                }
                WorkerEvent::FoldersLoaded {
                    account_id,
                    folders,
                } => {
                    let inbox_unread: u32 = folders
                        .iter()
                        .filter(|f| {
                            f.path.eq_ignore_ascii_case("INBOX") || f.name.contains("收件箱")
                        })
                        .map(|f| f.unread_count)
                        .sum();
                    let delta = detect_new_unread(
                        &mut self.last_unread_by_account,
                        account_id,
                        inbox_unread,
                    );
                    if delta > 0 {
                        total_new_unread = total_new_unread.saturating_add(delta);
                        notify_account_id = Some(account_id);
                    }
                    self.folders_by_account.insert(account_id, folders);
                    folders_updated = true;
                }
                WorkerEvent::MessagesPageLoaded(page) => {
                    self.loading_more_local = false;
                    if latest_unread_preview.is_none() {
                        if let Some(msg) = page.items.iter().find(|m| m.unread) {
                            let (name, email) = split_sender(&msg.sender);
                            let from = if name.is_empty() {
                                if email.is_empty() {
                                    msg.sender.clone()
                                } else {
                                    email
                                }
                            } else {
                                name
                            };
                            latest_unread_preview = Some((from, msg.subject.clone()));
                        }
                    }
                    // 仅当事件页与当前视图匹配时才采用；统一视图（星标/已发送/归档）
                    // 忽略文件夹级页面事件。
                    let page_matches = match self.current_view {
                        CurrentView::UnifiedInbox => {
                            !self.demo_mode && page.folder_id == UNIFIED_INBOX_FOLDER_ID
                        }
                        CurrentView::AccountFolder => page.folder_id == self.selected_folder_id,
                        CurrentView::Starred
                        | CurrentView::Sent
                        | CurrentView::Drafts
                        | CurrentView::Archive => false,
                    };
                    if page_matches {
                        self.merge_loaded_page(page, cx);
                    } else if matches!(
                        self.current_view,
                        CurrentView::UnifiedInbox | CurrentView::AccountFolder
                    ) {
                        self.reload_current_page(cx);
                    }
                }
                WorkerEvent::OlderMessagesLoaded {
                    folder_id: _,
                    fetched_count,
                    has_more_remote: _,
                } => {
                    self.loading_more_remote = false;
                    if fetched_count > 0 {
                        self.status_line =
                            SharedString::from(format!("已预加载 {fetched_count} 封更早邮件"));
                        self.reload_current_page(cx);
                    } else {
                        self.status_line = SharedString::from("已加载全部历史邮件");
                    }
                }
                WorkerEvent::MessageBodyLoaded(detail) => {
                    self.set_selected_message(detail);
                    self.error_banner = None;
                }
                WorkerEvent::DraftsUpdated { drafts } => {
                    self.drafts = drafts;
                }
                WorkerEvent::OutboxUpdated { account_id, items } => {
                    if items.is_empty() {
                        self.outbox_by_account.remove(&account_id);
                    } else {
                        self.outbox_by_account.insert(account_id, items);
                    }
                }
                WorkerEvent::DraftSent {
                    account_id: _,
                    subject,
                } => {
                    self.right_mode = RightPaneMode::Reader;
                    self.compose_draft_id = None;
                    self.compose_attachments.clear();
                    self.compose_in_reply_to.clear();
                    self.error_banner = None;
                    self.status_line = SharedString::from(format!("邮件已发出：{}", subject));
                }
                WorkerEvent::AttachmentSaved { path } => {
                    self.status_line =
                        SharedString::from(format!("附件已保存至：{}", path.display()));
                }
                WorkerEvent::AttachmentsSaved { count, dest_dir } => {
                    self.status_line = SharedString::from(format!(
                        "已保存 {count} 个附件至：{}",
                        dest_dir.display()
                    ));
                }
                WorkerEvent::Status {
                    account_id: _,
                    text,
                } => {
                    self.status_line = SharedString::from(text);
                }
                WorkerEvent::Error {
                    account_id: _,
                    message,
                } => {
                    self.error_banner = Some(SharedString::from(message.clone()));
                    self.status_line = SharedString::from(message);
                }
                WorkerEvent::MessageArchived {
                    account_id,
                    message_id,
                }
                | WorkerEvent::MessageDeleted {
                    account_id,
                    message_id,
                }
                | WorkerEvent::MessageMarkedSpam {
                    account_id,
                    message_id,
                }
                | WorkerEvent::MessageMoved {
                    account_id,
                    message_id,
                    ..
                } => {
                    // 后端回执：随事件先行到达的 FoldersLoaded 已刷新文件夹计数；
                    // 这里刷新当前列表并在阅读态显示该邮件时回到空态。
                    self.after_message_moved(account_id, message_id, cx);
                }
                WorkerEvent::AllFoldersSearchLoaded(page) => {
                    self.loading_more_local = false;
                    if self.search_scope == SearchScope::AllFolders
                        && !self.search_query.trim().is_empty()
                    {
                        self.merge_loaded_page(page, cx);
                    }
                }
                WorkerEvent::ServerSearchResults { query, items } => {
                    if query.trim() == self.search_query.trim() {
                        self.server_search_loading = false;
                        self.server_search_query = query;
                        self.server_search_results = items;
                    }
                }
                WorkerEvent::ForwardAttachmentsExported { message_id, paths } => {
                    if self.pending_forward_msg_id == Some(message_id) {
                        let mut added = 0usize;
                        for path in paths {
                            if !self.compose_attachments.contains(&path) {
                                self.compose_attachments.push(path);
                                added += 1;
                            }
                        }
                        if added > 0 {
                            self.status_line =
                                SharedString::from(format!("已自动携带原邮件 {added} 个附件"));
                        }
                    }
                }
                WorkerEvent::UnifiedViewLoaded { view, page } => {
                    self.loading_more_local = false;
                    if self.current_view.unified() == Some(view) {
                        self.merge_loaded_page(page, cx);
                    }
                }
                WorkerEvent::CachedAttachmentsLoaded { items } => {
                    self.gallery_items = items.clone();
                    if let Some(ref g) = self.gallery {
                        g.update(cx, |state, cx| {
                            state.set_items(items.clone());
                            cx.notify();
                        });
                    }
                }
                WorkerEvent::KnownContactsLoaded { items } => {
                    if self.contacts_query.trim().is_empty() || self.known_contacts.is_empty() {
                        self.known_contacts = items.clone();
                    }
                    if let Some(ref c) = self.contacts {
                        c.update(cx, |state, cx| {
                            state.set_contacts(items);
                            cx.notify();
                        });
                    }
                }
            }
        }
        if folders_updated {
            if let Some(ref tray) = self.tray {
                tray.set_tooltip(tray::format_tray_tooltip(self.unified_unread_count()));
                if total_new_unread > 0 && self.prefs.desktop_notifications && !self.demo_mode {
                    let account_label = notify_account_id
                        .map(|id| self.account_email_for(id))
                        .filter(|s| !s.is_empty());
                    let (sender, subject) = match latest_unread_preview.as_ref() {
                        Some((s, subj)) => (Some(s.as_str()), Some(subj.as_str())),
                        None => (None, None),
                    };
                    let (title, body) = tray::format_new_mail_notification(
                        total_new_unread,
                        sender,
                        subject,
                        account_label,
                    );
                    tray.show_notification(&title, &body);
                }
            }
        }
        cx.notify();
    }

    fn merge_loaded_page(&mut self, page: MailPage, cx: &mut Context<Self>) {
        if page.offset > 0 && page.offset == self.current_page.items.len() {
            let existing: HashSet<u32> = self.current_page.items.iter().map(|m| m.id).collect();
            for item in page.items {
                if !existing.contains(&item.id) {
                    self.current_page.items.push(item);
                }
            }
            self.current_page.total = page.total;
        } else {
            self.current_page = page;
            // 若后台同步推回了默认首屏（如 100 条），而用户此前已向下滚动加载了更多条，
            // 则按当前 loaded_limit 补刷一次以保持已展开列表不回弹。
            if self.loaded_limit > self.prefs.page_size
                && self.current_page.items.len() < self.loaded_limit
                && self.current_page.items.len() < self.current_page.total
            {
                self.reload_current_page(cx);
            }
        }
        if self.selected_message.is_none() {
            if let Some(first) = self.current_page.items.first() {
                self.request_message_detail(
                    first.account_id,
                    first.id,
                    first.uid,
                    first.unread,
                    &first.folder_path.clone(),
                );
            }
        }
    }

    pub(crate) fn reset_list_pagination(&mut self) {
        self.loaded_limit = self.prefs.page_size;
        self.loading_more_local = false;
        self.loading_more_remote = false;
        self.selected_message_ids.clear();
        self.list_scroll_handle.set_offset(point(px(0.0), px(0.0)));
    }

    pub(crate) fn reload_current_page(&mut self, cx: &mut Context<Self>) {
        let limit = self.loaded_limit.max(self.prefs.page_size);
        let cross_folder_search =
            !self.search_query.trim().is_empty() && self.search_scope == SearchScope::AllFolders;
        if self.demo_mode {
            // 演示模式：支持跨文件夹搜索、多条件快捷筛选与 6 种排序。
            self.current_page = if cross_folder_search {
                self.demo_store.search_all_folders(
                    &self.search_query,
                    self.quick_filters,
                    self.list_sort,
                    0,
                    limit,
                )
            } else {
                match self.current_view {
                    CurrentView::Starred => self.demo_store.list_starred_page_with_options(
                        &self.search_query,
                        self.quick_filters,
                        self.list_sort,
                        0,
                        limit,
                    ),
                    other => {
                        let folder_id = match other {
                            CurrentView::UnifiedInbox | CurrentView::AccountFolder => {
                                self.selected_folder_id
                            }
                            CurrentView::Sent => DEMO_SENT_ID,
                            CurrentView::Drafts => DEMO_DRAFTS_ID,
                            CurrentView::Archive => DEMO_ARCHIVE_ID,
                            CurrentView::Starred => DEMO_INBOX_ID,
                        };
                        self.demo_store.list_messages_page_with_options(
                            folder_id,
                            &self.search_query,
                            self.quick_filters,
                            self.list_sort,
                            0,
                            limit,
                        )
                    }
                }
            };
            cx.notify();
            return;
        }
        // 本地草稿视图：直接由渲染层从 `self.drafts` 合成，无需 worker 请求。
        if self.current_view == CurrentView::Drafts && !cross_folder_search {
            cx.notify();
            return;
        }
        self.send_page_request(0, limit);
    }

    fn send_page_request(&self, offset: usize, limit: usize) {
        let quick_filters = self.quick_filters;
        let sort = self.list_sort;
        let cross_folder_search =
            !self.search_query.trim().is_empty() && self.search_scope == SearchScope::AllFolders;
        if let Some(ref w) = self.worker {
            if cross_folder_search {
                w.send(MailRequest::SearchAllFolders {
                    query: self.search_query.clone(),
                    quick_filters,
                    sort,
                    offset,
                    limit,
                });
                return;
            }
            match self.current_view {
                CurrentView::UnifiedInbox => {
                    w.send(MailRequest::LoadUnifiedInbox {
                        query: self.search_query.clone(),
                        filter: MessageFilter::All,
                        quick_filters,
                        sort,
                        offset,
                        limit,
                    });
                }
                CurrentView::Starred | CurrentView::Sent | CurrentView::Archive => {
                    // unified() 在这三个视图下恒为 Some。
                    if let Some(view) = self.current_view.unified() {
                        w.send(MailRequest::LoadUnifiedView {
                            view,
                            query: self.search_query.clone(),
                            filter: MessageFilter::All,
                            quick_filters,
                            sort,
                            offset,
                            limit,
                        });
                    }
                }
                CurrentView::AccountFolder => {
                    w.send(MailRequest::LoadMessagesPage {
                        folder_id: self.selected_folder_id,
                        query: self.search_query.clone(),
                        filter: MessageFilter::All,
                        quick_filters,
                        sort,
                        offset,
                        limit,
                    });
                }
                CurrentView::Drafts => {}
            }
        }
    }

    /// 关联 GPUI 主窗口 HWND 到系统托盘（用于隐藏/恢复主窗口）。
    pub(crate) fn attach_main_hwnd(&self, hwnd: usize) {
        if let Some(ref tray) = self.tray {
            tray.set_main_hwnd(hwnd);
        }
    }

    /// 销毁系统托盘图标（真正退出前调用，避免托盘残留幽灵图标）。
    pub(crate) fn shutdown_tray(&self) {
        if let Some(ref tray) = self.tray {
            tray.shutdown();
        }
    }

    /// 持久化当前窗口尺寸与最大化状态到 `preferences.toml`。
    pub(crate) fn save_window_geometry(&mut self, window: &Window) {
        if capture_window_geometry(&mut self.prefs, window.window_bounds()) {
            let _ = config::save_preferences_in(&config::data_dir(), &self.prefs);
        }
    }

    /// 顶栏最小化按钮动作：偏好 `minimize_to_tray` 为 true 时隐藏到托盘，否则常规最小化。
    pub(crate) fn minimize_window_or_tray(
        &mut self,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        self.save_window_geometry(window);
        if self.prefs.minimize_to_tray {
            if let Some(ref tray) = self.tray {
                if tray.hide_main_window() {
                    return;
                }
            }
        }
        window.minimize_window();
    }

    /// 消费 Windows 系统托盘菜单 / 左键单击事件。
    fn handle_tray_command(&mut self, cmd: TrayCommand, cx: &mut Context<Self>) {
        match cmd {
            TrayCommand::ShowWindow => {
                if let Some(ref tray) = self.tray {
                    tray.show_main_window();
                }
            }
            TrayCommand::NewCompose => {
                if let Some(ref tray) = self.tray {
                    tray.show_main_window();
                }
                let entity = cx.entity();
                let _ = self.window_handle.update(cx, |_, window, cx| {
                    entity.update(cx, |this, cx| {
                        this.active_modal = ActiveModal::None;
                        this.main_view = MainContentView::Mail;
                        this.start_new_compose(window, cx);
                    });
                });
            }
            TrayCommand::SyncAll => {
                self.refresh_all(cx);
            }
            TrayCommand::Quit => {
                self.quit_app(cx);
            }
        }
    }

    /// 彻底退出应用（菜单「退出 Hylki」或托盘右键「退出 Hylki」入口）。
    pub(crate) fn quit_app(&mut self, cx: &mut Context<Self>) {
        let mut bounds_snapshot = None;
        let _ = self.window_handle.update(cx, |_, window, _| {
            bounds_snapshot = Some(window.window_bounds());
        });
        if let Some(wb) = bounds_snapshot {
            if capture_window_geometry(&mut self.prefs, wb) {
                let _ = config::save_preferences_in(&config::data_dir(), &self.prefs);
            }
        }
        self.shutdown_tray();
        cx.quit();
    }

    /// 切换左栏统一导航视图（收件箱 / 星标 / 已发送 / 草稿 / 归档）。
    pub(crate) fn select_unified_view(&mut self, view: CurrentView, cx: &mut Context<Self>) {
        self.current_view = view;
        self.reset_list_pagination();
        self.selected_message = None;
        self.right_mode = RightPaneMode::Reader;
        self.main_view = MainContentView::Mail;
        if self.demo_mode {
            // 演示模式下统一视图映射到对应的演示文件夹。
            self.selected_folder_id = match view {
                CurrentView::UnifiedInbox | CurrentView::Starred => DEMO_INBOX_ID,
                CurrentView::Sent => DEMO_SENT_ID,
                CurrentView::Drafts => DEMO_DRAFTS_ID,
                CurrentView::Archive => DEMO_ARCHIVE_ID,
                CurrentView::AccountFolder => self.selected_folder_id,
            };
        }
        self.remember_last_location(None);
        self.reload_current_page(cx);
        cx.notify();
    }

    pub(crate) fn select_folder(
        &mut self,
        account_id: u32,
        folder_id: u32,
        folder_path: String,
        cx: &mut Context<Self>,
    ) {
        self.selected_folder_id = folder_id;
        self.current_view = CurrentView::AccountFolder;
        self.reset_list_pagination();
        self.selected_message = None;
        self.right_mode = RightPaneMode::Reader;
        self.main_view = MainContentView::Mail;

        if self.demo_mode {
            self.current_page = self.demo_store.list_messages_page_with_options(
                folder_id,
                &self.search_query,
                self.quick_filters,
                self.list_sort,
                0,
                self.prefs.page_size,
            );
            let first_detail = self
                .current_page
                .items
                .first()
                .and_then(|m| self.demo_message_detail(m.id));
            if let Some(d) = first_detail {
                self.set_selected_message(d);
            } else {
                self.selected_message = None;
            }
            cx.notify();
            return;
        }

        self.remember_last_location(Some(account_id));
        self.reload_current_page(cx);
        if let Some(ref w) = self.worker {
            w.send(MailRequest::SyncFolder {
                account_id,
                folder_id,
                folder_path,
            });
        }
        cx.notify();
    }

    pub(crate) fn select_message(
        &mut self,
        account_id: u32,
        message_id: u32,
        uid: u32,
        unread: bool,
        folder_path: &str,
        cx: &mut Context<Self>,
    ) {
        self.right_mode = RightPaneMode::Reader;
        if self.demo_mode {
            if let Some(detail) = self.demo_message_detail(message_id) {
                self.set_selected_message(detail);
            } else {
                self.selected_message = None;
            }
            cx.notify();
            return;
        }
        self.request_message_detail(account_id, message_id, uid, unread, folder_path);
        cx.notify();
    }

    fn request_message_detail(
        &mut self,
        account_id: u32,
        message_id: u32,
        uid: u32,
        unread: bool,
        folder_path: &str,
    ) {
        if let Some(ref w) = self.worker {
            w.send(MailRequest::LoadMessageBody {
                account_id,
                message_id,
            });
            if unread {
                w.send(MailRequest::SetSeen {
                    account_id,
                    folder_path: folder_path.to_string(),
                    message_id,
                    uid,
                    seen: true,
                });
                if let Some(item) = self
                    .current_page
                    .items
                    .iter_mut()
                    .find(|m| m.id == message_id)
                {
                    item.unread = false;
                }
            }
        }
    }

    /// 当前阅读邮件在列表中的未读状态（默认已读）。
    pub(crate) fn selected_message_unread(&self) -> bool {
        let Some(detail) = self.selected_message.as_ref() else {
            return false;
        };
        self.current_page
            .items
            .iter()
            .find(|m| m.id == detail.id)
            .map(|m| m.unread)
            .unwrap_or(false)
    }

    /// 标记已读 / 未读（双向切换，复用现有 `MailRequest::SetSeen`）。
    pub(crate) fn toggle_message_seen(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.selected_message.clone() else {
            return;
        };
        // 当前未读则标为已读（seen = true），当前已读则标为未读（seen = false）。
        let next_seen = self.selected_message_unread();
        if let Some(item) = self
            .current_page
            .items
            .iter_mut()
            .find(|m| m.id == detail.id)
        {
            item.unread = !next_seen;
        }
        if !self.demo_mode {
            if let Some(ref w) = self.worker {
                w.send(MailRequest::SetSeen {
                    account_id: detail.account_id,
                    folder_path: detail.folder_path.clone(),
                    message_id: detail.id,
                    uid: detail.uid,
                    seen: next_seen,
                });
            }
        }
        self.status_line = SharedString::from(if next_seen {
            "已标记为已读"
        } else {
            "已标记为未读"
        });
        cx.notify();
    }

    /// 切换阅读器「原样 HTML / 纯文本」显示模式。
    pub(crate) fn toggle_reader_plain_mode(&mut self, cx: &mut Context<Self>) {
        self.reader_prefer_plain = !self.reader_prefer_plain;
        self.status_line = SharedString::from(if self.reader_prefer_plain {
            "阅读器已切换为纯文本模式"
        } else {
            "阅读器已切换为富文本 HTML 模式"
        });
        cx.notify();
    }

    /// 临时允许指定邮件加载远程图片。
    pub(crate) fn allow_remote_images_for(&mut self, message_id: u32, cx: &mut Context<Self>) {
        self.allowed_remote_image_ids.insert(message_id);
        cx.notify();
    }

    /// 永久信任该发件人（写入 `prefs.allowed_remote_senders` 并落盘）。
    pub(crate) fn trust_remote_sender(&mut self, sender: &str, cx: &mut Context<Self>) {
        if let Some(email) = self.prefs.allow_remote_sender(sender) {
            if let Err(e) = config::save_preferences_in(&config::data_dir(), &self.prefs) {
                self.error_banner = Some(SharedString::from(e));
            } else {
                self.status_line =
                    SharedString::from(format!("已将 {email} 加入远程图片发件人白名单"));
            }
            cx.notify();
        }
    }

    /// 按偏好顺序返回演示邮件详情。
    fn demo_message_detail(&self, message_id: u32) -> Option<MessageDetail> {
        let mut detail = self.demo_store.get_message_detail(message_id)?;
        if !self.prefs.thread_newest_first && detail.thread_messages.len() > 1 {
            detail.thread_messages.reverse();
        }
        Some(detail)
    }

    /// 更新当前选中邮件详情并初始化会话卡片折叠态 / 原文态。
    fn set_selected_message(&mut self, detail: MessageDetail) {
        let same_thread = self.selected_message.as_ref().is_some_and(|prev| {
            prev.id == detail.id || prev.thread_messages.iter().any(|m| m.id == detail.id)
        });
        if same_thread {
            // 同一会话内加载单张卡片正文：保留其他卡片状态，但确保刚加载的卡片处于展开态。
            self.collapsed_thread_cards.remove(&detail.id);
        } else {
            self.raw_source_message_ids.clear();
            self.collapsed_thread_cards = compute_default_collapsed_cards(
                &detail,
                self.prefs.thread_newest_first,
                self.prefs.fold_older_messages,
            );
        }
        self.selected_message = Some(detail);
    }

    /// 切换指定会话卡片的折叠/展开态。
    pub(crate) fn toggle_thread_card_collapsed(&mut self, msg_id: u32, cx: &mut Context<Self>) {
        if !self.collapsed_thread_cards.remove(&msg_id) {
            self.collapsed_thread_cards.insert(msg_id);
        }
        cx.notify();
    }

    /// 展开当前会话中的全部卡片。
    pub(crate) fn expand_all_thread_cards(&mut self, cx: &mut Context<Self>) {
        self.collapsed_thread_cards.clear();
        cx.notify();
    }

    /// 折叠当前会话中的较早卡片（保留最新一封展开）。
    pub(crate) fn collapse_older_thread_cards(&mut self, cx: &mut Context<Self>) {
        if let Some(ref detail) = self.selected_message {
            if detail.thread_messages.len() >= 2 {
                let newest_id = if self.prefs.thread_newest_first {
                    detail.thread_messages.first().map(|m| m.id)
                } else {
                    detail.thread_messages.last().map(|m| m.id)
                };
                self.collapsed_thread_cards = detail
                    .thread_messages
                    .iter()
                    .map(|m| m.id)
                    .filter(|&id| Some(id) != newest_id)
                    .collect();
            }
        }
        cx.notify();
    }

    /// 切换指定卡片的「查看邮件原文 / 返回正文」视图。
    pub(crate) fn toggle_raw_source(&mut self, msg_id: u32, cx: &mut Context<Self>) {
        if !self.raw_source_message_ids.remove(&msg_id) {
            self.raw_source_message_ids.insert(msg_id);
            self.collapsed_thread_cards.remove(&msg_id);
        }
        cx.notify();
    }

    /// 调整阅读器正文字号缩放（步进 ±10%，范围 70..=160）并持久化。
    pub(crate) fn adjust_reader_zoom(&mut self, delta: i32, cx: &mut Context<Self>) {
        let next = (self.prefs.reader_zoom_percent as i32 + delta).clamp(70, 160) as u32;
        self.set_reader_zoom(next, cx);
    }

    /// 设置阅读器正文字号缩放百分比（范围 70..=160）并持久化。
    pub(crate) fn set_reader_zoom(&mut self, percent: u32, cx: &mut Context<Self>) {
        let clamped = config::clamp_reader_zoom(percent);
        if self.prefs.reader_zoom_percent != clamped {
            self.prefs.reader_zoom_percent = clamped;
            if let Err(e) = config::save_preferences_in(&config::data_dir(), &self.prefs) {
                self.error_banner = Some(SharedString::from(e));
            }
            self.status_line = SharedString::from(format!("阅读字号已调整为 {clamped}%"));
            cx.notify();
        }
    }

    /// 复制任意文本到系统剪贴板（供单张会话卡片复制正文使用）。
    pub(crate) fn copy_text_to_clipboard(&mut self, text: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.status_line = SharedString::from("已复制邮件正文到剪贴板");
        cx.notify();
    }

    /// 复制当前阅读邮件的正文到系统剪贴板。
    pub(crate) fn copy_selected_body(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.selected_message.as_ref() else {
            return;
        };
        let text = detail.body_text.clone();
        self.copy_text_to_clipboard(text, cx);
    }

    pub(crate) fn toggle_flagged(&mut self, detail: &MessageDetail, cx: &mut Context<Self>) {
        let next_flagged = !detail.flagged;
        if let Some(item) = self
            .current_page
            .items
            .iter_mut()
            .find(|m| m.id == detail.id)
        {
            item.flagged = next_flagged;
        }
        if let Some(sel) = self.selected_message.as_mut() {
            sel.flagged = next_flagged;
        }
        if !self.demo_mode {
            if let Some(ref w) = self.worker {
                w.send(MailRequest::SetFlagged {
                    account_id: detail.account_id,
                    folder_path: detail.folder_path.clone(),
                    message_id: detail.id,
                    uid: detail.uid,
                    flagged: next_flagged,
                });
            }
        }
        cx.notify();
    }

    // ---- 迭代 2：归档 / 删除与列表行操作（方案文档五.3）----

    /// 归档 / 删除 / 移动 / 标垃圾完成后的公共收尾：清空正在显示的目标邮件并刷新当前列表。
    fn after_message_moved(&mut self, account_id: u32, message_id: u32, cx: &mut Context<Self>) {
        self.selected_message_ids.remove(&message_id);
        if self
            .selected_message
            .as_ref()
            .is_some_and(|m| m.account_id == account_id && m.id == message_id)
        {
            self.selected_message = None;
        }
        self.reload_current_page(cx);
    }

    /// 阅读态归档当前邮件（发 `MailRequest::ArchiveMessage`）。
    pub(crate) fn archive_selected(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.selected_message.clone() else {
            return;
        };
        self.archive_message(detail.account_id, detail.id, cx);
    }

    /// 阅读态删除当前邮件（发 `MailRequest::DeleteMessage`）。
    pub(crate) fn delete_selected(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.selected_message.clone() else {
            return;
        };
        self.delete_message(detail.account_id, detail.id, cx);
    }

    /// 阅读态标记当前邮件为垃圾邮件（发 `MailRequest::MarkSpam`）。
    pub(crate) fn mark_spam_selected(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.selected_message.clone() else {
            return;
        };
        self.mark_spam_message(detail.account_id, detail.id, cx);
    }

    /// 归档指定邮件（右键菜单入口）。
    pub(crate) fn archive_message(
        &mut self,
        account_id: u32,
        message_id: u32,
        cx: &mut Context<Self>,
    ) {
        if self.demo_mode {
            return;
        }
        if let Some(ref w) = self.worker {
            self.status_line = SharedString::from("正在归档邮件...");
            w.send(MailRequest::ArchiveMessage {
                account_id,
                message_id,
            });
        }
        cx.notify();
    }

    /// 删除指定邮件（右键菜单入口）。
    pub(crate) fn delete_message(
        &mut self,
        account_id: u32,
        message_id: u32,
        cx: &mut Context<Self>,
    ) {
        if self.demo_mode {
            return;
        }
        if let Some(ref w) = self.worker {
            self.status_line = SharedString::from("正在删除邮件...");
            w.send(MailRequest::DeleteMessage {
                account_id,
                message_id,
            });
        }
        cx.notify();
    }

    /// 标记指定邮件为垃圾邮件（移动到账号 Junk / Spam 文件夹）。
    pub(crate) fn mark_spam_message(
        &mut self,
        account_id: u32,
        message_id: u32,
        cx: &mut Context<Self>,
    ) {
        if self.demo_mode {
            return;
        }
        if let Some(ref w) = self.worker {
            self.status_line = SharedString::from("正在标记为垃圾邮件...");
            w.send(MailRequest::MarkSpam {
                account_id,
                message_id,
            });
        }
        cx.notify();
    }

    /// 移动指定邮件到目标文件夹（顶栏下拉菜单 / 右键子菜单入口）。
    pub(crate) fn move_message(
        &mut self,
        account_id: u32,
        message_id: u32,
        target_folder_path: String,
        cx: &mut Context<Self>,
    ) {
        if self.demo_mode {
            return;
        }
        if let Some(ref w) = self.worker {
            self.status_line = SharedString::from("正在移动邮件...");
            w.send(MailRequest::MoveMessage {
                account_id,
                message_id,
                target_folder_path,
            });
        }
        cx.notify();
    }

    // ---- 多选与批量操作 ----

    /// Ctrl+点击切换某封邮件的多选状态。
    pub(crate) fn toggle_message_selection(&mut self, message_id: u32, cx: &mut Context<Self>) {
        if self.selected_message_ids.is_empty() {
            if let Some(sel) = self.selected_message.as_ref() {
                if sel.id != message_id {
                    self.selected_message_ids.insert(sel.id);
                }
            }
        }
        if !self.selected_message_ids.remove(&message_id) {
            self.selected_message_ids.insert(message_id);
        }
        cx.notify();
    }

    /// 取消当前多选。
    pub(crate) fn clear_message_selection(&mut self, cx: &mut Context<Self>) {
        self.selected_message_ids.clear();
        cx.notify();
    }

    /// 批量将当前多选邮件标为已读。
    pub(crate) fn batch_mark_read(&mut self, cx: &mut Context<Self>) {
        let targets: Vec<MessageSummary> = self
            .current_page
            .items
            .iter()
            .filter(|m| self.selected_message_ids.contains(&m.id))
            .cloned()
            .collect();
        if targets.is_empty() {
            return;
        }
        for msg in &targets {
            if let Some(item) = self.current_page.items.iter_mut().find(|m| m.id == msg.id) {
                item.unread = false;
            }
            if !self.demo_mode {
                if let Some(ref w) = self.worker {
                    w.send(MailRequest::SetSeen {
                        account_id: msg.account_id,
                        folder_path: msg.folder_path.clone(),
                        message_id: msg.id,
                        uid: msg.uid,
                        seen: true,
                    });
                }
            }
        }
        self.status_line = SharedString::from(format!("已批量将 {} 封邮件标为已读", targets.len()));
        self.selected_message_ids.clear();
        cx.notify();
    }

    /// 批量加星标（若全部已加星则批量取消星标）。
    pub(crate) fn batch_toggle_flagged(&mut self, cx: &mut Context<Self>) {
        let targets: Vec<MessageSummary> = self
            .current_page
            .items
            .iter()
            .filter(|m| self.selected_message_ids.contains(&m.id))
            .cloned()
            .collect();
        if targets.is_empty() {
            return;
        }
        let next_flagged = !targets.iter().all(|m| m.flagged);
        for msg in &targets {
            if let Some(item) = self.current_page.items.iter_mut().find(|m| m.id == msg.id) {
                item.flagged = next_flagged;
            }
            if let Some(sel) = self.selected_message.as_mut() {
                if sel.id == msg.id {
                    sel.flagged = next_flagged;
                }
            }
            if !self.demo_mode {
                if let Some(ref w) = self.worker {
                    w.send(MailRequest::SetFlagged {
                        account_id: msg.account_id,
                        folder_path: msg.folder_path.clone(),
                        message_id: msg.id,
                        uid: msg.uid,
                        flagged: next_flagged,
                    });
                }
            }
        }
        self.status_line = SharedString::from(if next_flagged {
            format!("已批量为 {} 封邮件加星标", targets.len())
        } else {
            format!("已批量取消 {} 封邮件的星标", targets.len())
        });
        self.selected_message_ids.clear();
        cx.notify();
    }

    /// 批量归档当前多选邮件。
    pub(crate) fn batch_archive(&mut self, cx: &mut Context<Self>) {
        if self.demo_mode {
            return;
        }
        let targets: Vec<(u32, u32)> = self
            .current_page
            .items
            .iter()
            .filter(|m| self.selected_message_ids.contains(&m.id))
            .map(|m| (m.account_id, m.id))
            .collect();
        if targets.is_empty() {
            return;
        }
        if let Some(ref w) = self.worker {
            for (account_id, message_id) in &targets {
                w.send(MailRequest::ArchiveMessage {
                    account_id: *account_id,
                    message_id: *message_id,
                });
            }
        }
        self.status_line = SharedString::from(format!("正在批量归档 {} 封邮件...", targets.len()));
        self.selected_message_ids.clear();
        cx.notify();
    }

    /// 批量删除当前多选邮件。
    pub(crate) fn batch_delete(&mut self, cx: &mut Context<Self>) {
        if self.demo_mode {
            return;
        }
        let targets: Vec<(u32, u32)> = self
            .current_page
            .items
            .iter()
            .filter(|m| self.selected_message_ids.contains(&m.id))
            .map(|m| (m.account_id, m.id))
            .collect();
        if targets.is_empty() {
            return;
        }
        if let Some(ref w) = self.worker {
            for (account_id, message_id) in &targets {
                w.send(MailRequest::DeleteMessage {
                    account_id: *account_id,
                    message_id: *message_id,
                });
            }
        }
        self.status_line = SharedString::from(format!("正在批量删除 {} 封邮件...", targets.len()));
        self.selected_message_ids.clear();
        cx.notify();
    }

    /// 列表行「标记已读 / 未读」：真实账号走 `SetSeen`，演示模式仅本地切换。
    pub(crate) fn set_row_seen(
        &mut self,
        account_id: u32,
        message_id: u32,
        uid: u32,
        folder_path: String,
        seen: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(item) = self
            .current_page
            .items
            .iter_mut()
            .find(|m| m.id == message_id)
        {
            item.unread = !seen;
        }
        if self.demo_mode {
            cx.notify();
            return;
        }
        if let Some(ref w) = self.worker {
            w.send(MailRequest::SetSeen {
                account_id,
                folder_path,
                message_id,
                uid,
                seen,
            });
        }
        cx.notify();
    }

    /// 列表行「加星 / 取消星标」：真实账号走 `SetFlagged`，演示模式仅本地切换。
    pub(crate) fn set_row_flagged(
        &mut self,
        account_id: u32,
        message_id: u32,
        uid: u32,
        folder_path: String,
        flagged: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(item) = self
            .current_page
            .items
            .iter_mut()
            .find(|m| m.id == message_id)
        {
            item.flagged = flagged;
        }
        if let Some(sel) = self.selected_message.as_mut() {
            if sel.id == message_id {
                sel.flagged = flagged;
            }
        }
        if self.demo_mode {
            cx.notify();
            return;
        }
        if let Some(ref w) = self.worker {
            w.send(MailRequest::SetFlagged {
                account_id,
                folder_path,
                message_id,
                uid,
                flagged,
            });
        }
        cx.notify();
    }

    /// 删除本地草稿（草稿视图右键菜单入口，走现有 `DeleteLocalDraft`）。
    pub(crate) fn delete_row_draft(&mut self, draft_id: u32, cx: &mut Context<Self>) {
        if self.demo_mode {
            return;
        }
        if let Some(ref w) = self.worker {
            w.send(MailRequest::DeleteLocalDraft { draft_id });
        }
        cx.notify();
    }

    /// 把本地草稿合成为中栏列表行（草稿视图，真实账号模式）。
    pub(crate) fn draft_row_summary(&self, draft: &LocalDraft) -> MessageSummary {
        let subject = if draft.subject.trim().is_empty() {
            "(无主题草稿)".to_string()
        } else {
            draft.subject.clone()
        };
        let preview = draft
            .body_text
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .chars()
            .take(120)
            .collect::<String>();
        let owner_email = self.account_email_for(draft.account_id).to_string();
        MessageSummary {
            id: draft.id,
            account_id: draft.account_id,
            folder_id: 0,
            folder_path: String::new(),
            uid: 0,
            sender: if owner_email.is_empty() {
                "草稿".to_string()
            } else {
                format!("草稿 · {owner_email}")
            },
            subject,
            preview,
            date_ts: draft.updated_at,
            // 本地草稿不展示时钟时间（避免 UI 层引入时区处理）。
            date_display: String::new(),
            unread: false,
            flagged: false,
            has_attachment: !draft.attachment_paths.is_empty(),
        }
    }

    /// 展开 / 折叠 Cc / Bcc 输入行（方案文档页面 3）。
    pub(crate) fn toggle_compose_cc_bcc(&mut self, cx: &mut Context<Self>) {
        self.compose_show_cc_bcc = !self.compose_show_cc_bcc;
        cx.notify();
    }

    /// 查询指定账号配置的邮件签名。
    fn account_signature(&self, account_id: u32) -> String {
        self.accounts
            .iter()
            .find(|a| a.id == account_id)
            .map(|a| a.signature.clone())
            .unwrap_or_default()
    }

    /// 确保写信自动补全有联系人缓存。
    fn ensure_contacts_loaded(&self) {
        if !self.demo_mode && self.known_contacts.is_empty() {
            if let Some(ref w) = self.worker {
                w.send(MailRequest::ListKnownContacts {
                    query: String::new(),
                    offset: 0,
                    limit: 500,
                });
            }
        }
    }

    pub(crate) fn start_new_compose(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.main_view = MainContentView::Mail;
        self.compose_account_id = self.accounts.first().map(|a| a.id).unwrap_or(1);
        self.compose_draft_id = None;
        self.compose_in_reply_to.clear();
        self.compose_attachments.clear();
        self.compose_confirm_discard = false;
        self.pending_forward_msg_id = None;
        self.compose_autocomplete_field = None;
        let initial_body = append_signature("", &self.account_signature(self.compose_account_id));
        self.compose_to
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.compose_cc
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.compose_bcc
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.compose_show_cc_bcc = false;
        self.compose_subject
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.compose_body
            .update(cx, |s, cx| s.set_value(initial_body, window, cx));
        self.right_mode = RightPaneMode::Compose;
        self.ensure_contacts_loaded();
        cx.notify();
    }

    pub(crate) fn start_reply(
        &mut self,
        detail: &MessageDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_reply_with(detail, "Re:", &detail.sender, window, cx);
    }

    /// 回复全部：收件人 = 原发件人 + 原收件人（沿用现有写信状态，无新 API）。
    pub(crate) fn start_reply_all(
        &mut self,
        detail: &MessageDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut to = detail.sender.clone();
        let recipients = detail.recipients.trim();
        if !recipients.is_empty() {
            to.push_str(", ");
            to.push_str(recipients);
        }
        self.start_reply_with(detail, "Re:", &to, window, cx);
    }

    /// 转发：主题加 Fwd: 前缀，正文引用原邮件，收件人留空，并自动携带原邮件已缓存附件。
    pub(crate) fn start_forward(
        &mut self,
        detail: &MessageDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let forward_subject = if detail.subject.to_lowercase().starts_with("fwd:") {
            detail.subject.clone()
        } else {
            format!("Fwd: {}", detail.subject)
        };
        let quoted = format!(
            "\n\n------------------ 转发邮件 ------------------\n发件人：{}\n时间：{}\n主题：{}\n\n{}",
            detail.sender, detail.date_display, detail.subject, detail.body_text
        );
        let body_with_sig =
            append_signature(&quoted, &self.account_signature(detail.account_id));

        self.compose_account_id = detail.account_id;
        self.compose_draft_id = None;
        self.compose_in_reply_to.clear();
        self.compose_attachments.clear();
        self.compose_confirm_discard = false;
        self.pending_forward_msg_id = Some(detail.id);
        self.compose_autocomplete_field = None;
        self.compose_to
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.clear_compose_cc_bcc(window, cx);
        self.compose_subject
            .update(cx, |s, cx| s.set_value(forward_subject, window, cx));
        self.compose_body
            .update(cx, |s, cx| s.set_value(body_with_sig, window, cx));
        self.right_mode = RightPaneMode::Compose;
        self.ensure_contacts_loaded();

        if !detail.attachments.is_empty() {
            if self.demo_mode {
                let dest_dir = std::env::temp_dir().join(format!("hylki-forward-{}", detail.id));
                if std::fs::create_dir_all(&dest_dir).is_ok() {
                    for att in &detail.attachments {
                        let path = dest_dir.join(&att.filename);
                        if std::fs::write(&path, format!("Demo attachment: {}", att.filename))
                            .is_ok()
                        {
                            self.compose_attachments
                                .push(path.to_string_lossy().to_string());
                        }
                    }
                }
            } else if let Some(ref w) = self.worker {
                w.send(MailRequest::ExportForwardAttachments {
                    message_id: detail.id,
                });
            }
        }
        cx.notify();
    }

    /// start_reply / start_reply_all 的共用主体。
    fn start_reply_with(
        &mut self,
        detail: &MessageDetail,
        prefix: &str,
        to: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let reply_subject = if detail
            .subject
            .to_lowercase()
            .starts_with(&prefix.to_lowercase())
        {
            detail.subject.clone()
        } else {
            format!("{} {}", prefix, detail.subject)
        };
        let quoted = format!(
            "\n\n------------------ 原始邮件 ------------------\n发件人：{}\n时间：{}\n主题：{}\n\n{}",
            detail.sender, detail.date_display, detail.subject, detail.body_text
        );
        let body_with_sig =
            append_signature(&quoted, &self.account_signature(detail.account_id));

        self.compose_account_id = detail.account_id;
        self.compose_draft_id = None;
        self.compose_in_reply_to = detail.rfc_message_id.clone();
        self.compose_attachments.clear();
        self.compose_confirm_discard = false;
        self.pending_forward_msg_id = None;
        self.compose_autocomplete_field = None;
        self.compose_to
            .update(cx, |s, cx| s.set_value(to.to_string(), window, cx));
        self.clear_compose_cc_bcc(window, cx);
        self.compose_subject
            .update(cx, |s, cx| s.set_value(reply_subject, window, cx));
        self.compose_body
            .update(cx, |s, cx| s.set_value(body_with_sig, window, cx));
        self.right_mode = RightPaneMode::Compose;
        self.ensure_contacts_loaded();
        cx.notify();
    }

    /// 判断当前写信表单是否存在未保存内容。
    pub(crate) fn compose_has_unsaved_content(&self, cx: &Context<Self>) -> bool {
        has_unsaved_compose_content(
            self.compose_to.read(cx).value().as_ref(),
            self.compose_cc.read(cx).value().as_ref(),
            self.compose_bcc.read(cx).value().as_ref(),
            self.compose_subject.read(cx).value().as_ref(),
            self.compose_body.read(cx).value().as_ref(),
            self.compose_attachments.len(),
        )
    }

    /// 点击「取消」或按 Esc 尝试离开写信页：有内容时弹出内联确认条；
    /// 已在确认态再次按 Esc 则继续编辑（与 Linux 一致）。
    pub(crate) fn request_close_compose(&mut self, cx: &mut Context<Self>) {
        if self.compose_confirm_discard {
            self.compose_confirm_discard = false;
            cx.notify();
            return;
        }
        if self.compose_has_unsaved_content(cx) {
            self.compose_confirm_discard = true;
            cx.notify();
        } else {
            self.discard_compose(cx);
        }
    }

    /// 放弃当前写信内容并返回阅读态。
    pub(crate) fn discard_compose(&mut self, cx: &mut Context<Self>) {
        self.compose_confirm_discard = false;
        self.pending_forward_msg_id = None;
        self.compose_autocomplete_field = None;
        self.right_mode = RightPaneMode::Reader;
        cx.notify();
    }

    /// 保存为草稿并退出写信页。
    pub(crate) fn save_draft_and_close_compose(&mut self, cx: &mut Context<Self>) {
        self.save_local_draft(cx);
        self.compose_confirm_discard = false;
        self.pending_forward_msg_id = None;
        self.compose_autocomplete_field = None;
        self.right_mode = RightPaneMode::Reader;
        cx.notify();
    }

    /// 点击收件人联系人补全建议项：替换最后一段并追加逗号。
    pub(crate) fn apply_contact_suggestion(
        &mut self,
        field: ComposeAddressField,
        contact: &KnownContact,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = match field {
            ComposeAddressField::To => &self.compose_to,
            ComposeAddressField::Cc => &self.compose_cc,
            ComposeAddressField::Bcc => &self.compose_bcc,
        };
        let current = target.read(cx).value().to_string();
        let next = apply_contact_completion(&current, contact);
        target.update(cx, |s, cx| s.set_value(next, window, cx));
        self.compose_autocomplete_field = None;
        cx.notify();
    }

    /// 清空 Cc / Bcc 输入（新写信 / 回复 / 转发入口共用）。
    fn clear_compose_cc_bcc(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.compose_cc
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.compose_bcc
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.compose_show_cc_bcc = false;
    }

    pub(crate) fn open_local_draft(
        &mut self,
        draft: &LocalDraft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.compose_account_id = draft.account_id;
        self.compose_draft_id = Some(draft.id);
        self.compose_in_reply_to = draft.in_reply_to.clone();
        self.compose_attachments = draft.attachment_paths.clone();
        self.compose_to
            .update(cx, |s, cx| s.set_value(draft.to_addr.clone(), window, cx));
        self.compose_cc
            .update(cx, |s, cx| s.set_value(draft.cc_addr.clone(), window, cx));
        self.compose_bcc
            .update(cx, |s, cx| s.set_value(draft.bcc_addr.clone(), window, cx));
        self.compose_show_cc_bcc =
            !draft.cc_addr.trim().is_empty() || !draft.bcc_addr.trim().is_empty();
        self.compose_subject
            .update(cx, |s, cx| s.set_value(draft.subject.clone(), window, cx));
        self.compose_body
            .update(cx, |s, cx| s.set_value(draft.body_text.clone(), window, cx));
        self.right_mode = RightPaneMode::Compose;
        cx.notify();
    }

    pub(crate) fn save_local_draft(&mut self, cx: &mut Context<Self>) {
        if self.demo_mode {
            self.status_line = SharedString::from("演示模式下不支持写入本地草稿数据库");
            cx.notify();
            return;
        }
        let draft = LocalDraft {
            id: self.compose_draft_id.unwrap_or(0),
            account_id: self.compose_account_id,
            to_addr: self.compose_to.read(cx).value().to_string(),
            cc_addr: self.compose_cc.read(cx).value().to_string(),
            bcc_addr: self.compose_bcc.read(cx).value().to_string(),
            subject: self.compose_subject.read(cx).value().to_string(),
            body_text: self.compose_body.read(cx).value().to_string(),
            in_reply_to: self.compose_in_reply_to.clone(),
            attachment_paths: self.compose_attachments.clone(),
            updated_at: 0,
        };
        if let Some(ref w) = self.worker {
            w.send(MailRequest::SaveLocalDraft { draft });
        }
        cx.notify();
    }

    pub(crate) fn pick_attachments(&mut self, cx: &mut Context<Self>) {
        if let Some(files) = rfd::FileDialog::new().pick_files() {
            for file in files {
                let p = file.to_string_lossy().to_string();
                if !self.compose_attachments.contains(&p) {
                    self.compose_attachments.push(p);
                }
            }
            cx.notify();
        }
    }

    pub(crate) fn save_attachment(&self, att_id: u32, filename: &str) {
        if self.demo_mode {
            return;
        }
        if let Some(dest) = rfd::FileDialog::new().set_file_name(filename).save_file() {
            if let Some(ref w) = self.worker {
                w.send(MailRequest::SaveAttachment {
                    attachment_id: att_id,
                    dest_path: dest,
                });
            }
        }
    }

    /// 选择目标目录并批量导出指定附件列表（重名自动追加 `(1)` 后缀）。
    pub(crate) fn save_all_attachments(&mut self, attachment_ids: Vec<u32>, cx: &mut Context<Self>) {
        if attachment_ids.is_empty() {
            return;
        }
        if self.demo_mode {
            self.status_line = SharedString::from("演示模式下不支持导出真实附件文件");
            cx.notify();
            return;
        }
        if let Some(dest_dir) = rfd::FileDialog::new().pick_folder() {
            if let Some(ref w) = self.worker {
                self.status_line = SharedString::from(format!(
                    "正在保存 {} 个附件至 {}...",
                    attachment_ids.len(),
                    dest_dir.display()
                ));
                w.send(MailRequest::SaveAllAttachments {
                    attachment_ids,
                    dest_dir,
                });
            }
            cx.notify();
        }
    }

    pub(crate) fn send_compose(&mut self, cx: &mut Context<Self>) {
        let to = self.compose_to.read(cx).value().to_string();
        let cc = self.compose_cc.read(cx).value().to_string();
        let bcc = self.compose_bcc.read(cx).value().to_string();
        let subject = self.compose_subject.read(cx).value().to_string();
        let body = self.compose_body.read(cx).value().to_string();

        if !self.demo_mode && self.accounts.is_empty() {
            self.error_banner = Some(SharedString::from("请先配置并保存邮箱账号"));
            cx.notify();
            return;
        }
        if !self.demo_mode && to.trim().is_empty() {
            self.error_banner = Some(SharedString::from("请填写收件人邮箱地址"));
            cx.notify();
            return;
        }

        // 若已有正在倒计时的待发邮件，先立即投递，避免覆盖丢信。
        if let Some(prev) = self.pending_send.as_ref().map(|p| p.token) {
            self.commit_pending_send(prev, cx);
        }

        let token = self.next_send_token;
        self.next_send_token = self.next_send_token.wrapping_add(1).max(1);
        let pending = PendingSend {
            token,
            account_id: self.compose_account_id,
            draft: ComposeDraft {
                to,
                cc,
                bcc,
                subject,
                body,
                in_reply_to: self.compose_in_reply_to.clone(),
                attachment_paths: self.compose_attachments.clone(),
            },
            local_draft_id: self.compose_draft_id,
            remaining_secs: 5,
        };
        self.pending_send = Some(pending);
        self.compose_confirm_discard = false;
        self.compose_autocomplete_field = None;
        self.pending_forward_msg_id = None;
        self.right_mode = RightPaneMode::Reader;
        self.error_banner = None;
        self.status_line = SharedString::from("邮件将在 5 秒后发出，可随时点击「撤销发送」");

        cx.spawn(async move |this, cx| {
            for _ in 0..5 {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let keep_going = this
                    .update(cx, |app, cx| {
                        let Some(pending) = app.pending_send.as_mut() else {
                            return false;
                        };
                        if pending.token != token {
                            return false;
                        }
                        if pending.remaining_secs > 1 {
                            pending.remaining_secs -= 1;
                            cx.notify();
                            true
                        } else {
                            app.commit_pending_send(token, cx);
                            false
                        }
                    })
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        })
        .detach();

        cx.notify();
    }

    /// 真正将 `pending_send` 投递给 Worker（倒计时到期或点击「立即发送」）。
    pub(crate) fn commit_pending_send(&mut self, token: u64, cx: &mut Context<Self>) {
        if self.pending_send.as_ref().map(|p| p.token) != Some(token) {
            return;
        }
        let Some(pending) = self.pending_send.take() else {
            return;
        };

        if self.demo_mode {
            self.status_line = SharedString::from(format!(
                "演示发件成功：收件人=\"{}\"，主题=\"{}\"，附件={} 个",
                if pending.draft.to.is_empty() {
                    "(未填写)"
                } else {
                    &pending.draft.to
                },
                if pending.draft.subject.is_empty() {
                    "(无主题)"
                } else {
                    &pending.draft.subject
                },
                pending.draft.attachment_paths.len()
            ));
            cx.notify();
            return;
        }

        if let Some(ref w) = self.worker {
            self.status_line = SharedString::from("正在持久化至发件箱并发送...");
            w.send(MailRequest::SendDraft {
                account_id: pending.account_id,
                draft: pending.draft,
                local_draft_id: pending.local_draft_id,
            });
        }
        cx.notify();
    }

    /// 撤销发送：取消 5 秒缓冲并恢复全部内容回到写信页。
    pub(crate) fn undo_pending_send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_send.take() else {
            return;
        };
        self.main_view = MainContentView::Mail;
        self.compose_account_id = pending.account_id;
        self.compose_draft_id = pending.local_draft_id;
        self.compose_in_reply_to = pending.draft.in_reply_to;
        self.compose_attachments = pending.draft.attachment_paths;
        self.compose_show_cc_bcc =
            !pending.draft.cc.trim().is_empty() || !pending.draft.bcc.trim().is_empty();
        self.compose_to
            .update(cx, |s, cx| s.set_value(pending.draft.to, window, cx));
        self.compose_cc
            .update(cx, |s, cx| s.set_value(pending.draft.cc, window, cx));
        self.compose_bcc
            .update(cx, |s, cx| s.set_value(pending.draft.bcc, window, cx));
        self.compose_subject
            .update(cx, |s, cx| s.set_value(pending.draft.subject, window, cx));
        self.compose_body
            .update(cx, |s, cx| s.set_value(pending.draft.body, window, cx));
        self.compose_confirm_discard = false;
        self.right_mode = RightPaneMode::Compose;
        self.status_line = SharedString::from("已撤销发送，返回编辑邮件");
        cx.notify();
    }

    pub(crate) fn load_account_into_form(
        &mut self,
        cfg: Option<&AccountConfig>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let c = cfg.cloned().unwrap_or_default();
        self.editing_account_id = cfg.map(|a| a.id);
        self.cfg_display_name
            .update(cx, |s, cx| s.set_value(c.display_name, window, cx));
        self.cfg_email
            .update(cx, |s, cx| s.set_value(c.email, window, cx));
        self.cfg_imap_host
            .update(cx, |s, cx| s.set_value(c.imap_host, window, cx));
        self.cfg_imap_port
            .update(cx, |s, cx| s.set_value(c.imap_port.to_string(), window, cx));
        self.cfg_imap_tls = c.imap_security == SecurityMode::Tls;
        self.cfg_imap_user
            .update(cx, |s, cx| s.set_value(c.imap_username, window, cx));
        self.cfg_imap_pass
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.cfg_smtp_host
            .update(cx, |s, cx| s.set_value(c.smtp_host, window, cx));
        self.cfg_smtp_port
            .update(cx, |s, cx| s.set_value(c.smtp_port.to_string(), window, cx));
        self.cfg_smtp_tls = c.smtp_security == SecurityMode::Tls;
        self.cfg_smtp_separate = c.smtp_separate_credentials;
        self.cfg_smtp_user
            .update(cx, |s, cx| s.set_value(c.smtp_username, window, cx));
        self.cfg_smtp_pass
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.cfg_signature
            .update(cx, |s, cx| s.set_value(c.signature, window, cx));
        self.cfg_feedback = None;
        self.cfg_auth_guide = None;
        self.active_modal = ActiveModal::Accounts;
        cx.notify();
    }

    /// 欢迎页服务商预设入口：载入空白表单并自动带入主机 / 端口 / 加密与指引。
    pub(crate) fn start_preset_account(
        &mut self,
        preset: &ProviderPreset,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.load_account_into_form(None, window, cx);
        self.apply_preset(preset, window, cx);
    }

    /// 运行时进入演示模式（欢迎页入口 3）：复用 `--demo` 启动路径的初始化。
    pub(crate) fn enter_demo_mode(&mut self, cx: &mut Context<Self>) {
        if self.demo_mode {
            return;
        }
        self.demo_mode = true;
        let current_page =
            self.demo_store
                .list_messages_page(DEMO_INBOX_ID, "", 0, self.prefs.page_size);
        let selected_message = current_page
            .items
            .first()
            .and_then(|msg| self.demo_message_detail(msg.id));
        self.current_page = current_page;
        if let Some(detail) = selected_message {
            self.set_selected_message(detail);
        } else {
            self.selected_message = None;
        }
        let mut folders_by_account = BTreeMap::new();
        folders_by_account.insert(1, self.demo_store.folders.clone());
        self.folders_by_account = folders_by_account;
        let demo_acc = AccountConfig {
            id: 1,
            display_name: self.demo_store.accounts[0].display_name.clone(),
            email: self.demo_store.accounts[0].email.clone(),
            ..Default::default()
        };
        self.accounts = vec![demo_acc];
        self.selected_folder_id = DEMO_INBOX_ID;
        self.current_view = CurrentView::UnifiedInbox;
        self.reset_list_pagination();
        self.compose_account_id = 1;
        self.editing_account_id = Some(1);
        self.pending_delete_account_id = None;
        self.right_mode = RightPaneMode::Reader;
        self.main_view = MainContentView::Mail;
        self.error_banner = None;
        self.status_line = SharedString::from("演示模式：已加载 10,000 封模拟邮件");
        cx.notify();
    }

    /// 打开账号管理独立居中弹窗（菜单 / 欢迎页 / 命令面板入口），保留当前表单选择。
    pub(crate) fn open_account_manager(&mut self, cx: &mut Context<Self>) {
        self.pending_delete_account_id = None;
        self.active_modal = ActiveModal::Accounts;
        cx.notify();
    }

    /// 账号卡片「移除账号」：先进入内联确认态（含确认）。
    pub(crate) fn request_delete_account(&mut self, account_id: u32, cx: &mut Context<Self>) {
        self.pending_delete_account_id = Some(account_id);
        cx.notify();
    }

    pub(crate) fn cancel_delete_account(&mut self, cx: &mut Context<Self>) {
        self.pending_delete_account_id = None;
        cx.notify();
    }

    /// 确认移除账号：发 `MailRequest::DeleteAccount`（清理配置、缓存与凭据）。
    pub(crate) fn confirm_delete_account(&mut self, cx: &mut Context<Self>) {
        let Some(account_id) = self.pending_delete_account_id.take() else {
            return;
        };
        if self.demo_mode {
            return;
        }
        if let Some(ref w) = self.worker {
            w.send(MailRequest::DeleteAccount { account_id });
        }
        cx.notify();
    }

    pub(crate) fn apply_preset(
        &mut self,
        preset: &ProviderPreset,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cfg_imap_host
            .update(cx, |s, cx| s.set_value(preset.imap_host, window, cx));
        self.cfg_imap_port
            .update(cx, |s, cx| s.set_value(preset.imap_port, window, cx));
        self.cfg_smtp_host
            .update(cx, |s, cx| s.set_value(preset.smtp_host, window, cx));
        self.cfg_smtp_port
            .update(cx, |s, cx| s.set_value(preset.smtp_port, window, cx));
        self.cfg_imap_tls = preset.imap_tls;
        self.cfg_smtp_tls = preset.smtp_tls;
        self.cfg_auth_guide = Some(SharedString::from(preset.guide));
        cx.notify();
    }

    fn collect_account_form(&self, cx: &Context<Self>) -> (AccountConfig, String, Option<String>) {
        let email = self.cfg_email.read(cx).value().trim().to_string();
        let imap_user = self.cfg_imap_user.read(cx).value().trim().to_string();
        let smtp_user = self.cfg_smtp_user.read(cx).value().trim().to_string();
        let imap_port = self
            .cfg_imap_port
            .read(cx)
            .value()
            .trim()
            .parse::<u16>()
            .unwrap_or(993);
        let smtp_port = self
            .cfg_smtp_port
            .read(cx)
            .value()
            .trim()
            .parse::<u16>()
            .unwrap_or(465);
        let id = self
            .editing_account_id
            .unwrap_or_else(|| config::next_account_id(&self.accounts));

        let config = AccountConfig {
            id,
            auth_backend: AuthBackend::Password,
            oauth_client_id: String::new(),
            display_name: self.cfg_display_name.read(cx).value().trim().to_string(),
            email: email.clone(),
            imap_host: self.cfg_imap_host.read(cx).value().trim().to_string(),
            imap_port,
            imap_security: if self.cfg_imap_tls {
                SecurityMode::Tls
            } else {
                SecurityMode::StartTls
            },
            imap_username: if imap_user.is_empty() {
                email
            } else {
                imap_user
            },
            smtp_host: self.cfg_smtp_host.read(cx).value().trim().to_string(),
            smtp_port,
            smtp_security: if self.cfg_smtp_tls {
                SecurityMode::Tls
            } else {
                SecurityMode::StartTls
            },
            smtp_username: smtp_user,
            smtp_separate_credentials: self.cfg_smtp_separate,
            signature: self.cfg_signature.read(cx).value().to_string(),
        };
        let imap_pwd = self.cfg_imap_pass.read(cx).value().to_string();
        let smtp_pwd = if self.cfg_smtp_separate {
            Some(self.cfg_smtp_pass.read(cx).value().to_string())
        } else {
            None
        };
        (config, imap_pwd, smtp_pwd)
    }

    pub(crate) fn start_outlook_oauth(&mut self, cx: &mut Context<Self>) {
        if self.demo_mode {
            return;
        }
        let custom_client_id = self.cfg_oauth_client_id.read(cx).value().trim().to_string();
        self.cfg_feedback = Some((
            true,
            SharedString::from("正在打开系统浏览器，请在微软页面完成 Outlook 授权..."),
        ));
        if let Some(ref w) = self.worker {
            w.send(MailRequest::StartMicrosoftOAuth {
                custom_client_id,
                editing_account_id: self.editing_account_id,
            });
        }
        cx.notify();
    }

    pub(crate) fn verify_account_form(&mut self, cx: &mut Context<Self>) {
        let (config, imap_password, smtp_password) = self.collect_account_form(cx);
        self.cfg_feedback = Some((true, SharedString::from("正在验证 IMAP / SMTP 连接...")));
        if let Some(ref w) = self.worker {
            w.send(MailRequest::VerifyAccount {
                config,
                imap_password,
                smtp_password,
            });
        }
        cx.notify();
    }

    pub(crate) fn save_account_form(&mut self, cx: &mut Context<Self>) {
        let (config, imap_password, smtp_password) = self.collect_account_form(cx);
        self.cfg_feedback = Some((true, SharedString::from("正在保存凭据并启动同步...")));
        if let Some(ref w) = self.worker {
            w.send(MailRequest::SaveAndSyncAccount {
                config,
                imap_password,
                smtp_password,
            });
        }
        cx.notify();
    }

    /// 当前视图对应的远端邮箱是否仍有未拉取的更早历史邮件。
    pub(crate) fn current_view_has_more_remote(&self) -> bool {
        if self.demo_mode || !self.search_query.trim().is_empty() {
            return false;
        }
        match self.current_view {
            CurrentView::UnifiedInbox => self
                .folders_by_account
                .values()
                .flatten()
                .any(|f| {
                    (f.path.eq_ignore_ascii_case("INBOX")
                        || f.name.eq_ignore_ascii_case("INBOX")
                        || f.name == "收件箱")
                        && f.has_more_remote
                }),
            CurrentView::AccountFolder => self
                .folders_by_account
                .values()
                .flatten()
                .find(|f| f.id == self.selected_folder_id)
                .is_some_and(|f| f.has_more_remote),
            CurrentView::Starred
            | CurrentView::Sent
            | CurrentView::Drafts
            | CurrentView::Archive => false,
        }
    }

    /// 滚动位置接近底部（距底不足约 3 行高度）时自动预加载下一批 100 条。
    pub(crate) fn maybe_preload_more(&mut self, cx: &mut Context<Self>) {
        if self.current_view == CurrentView::Drafts && !self.demo_mode {
            return;
        }
        let max_y = self.list_scroll_handle.max_offset().y.abs();
        let offset_y = (-self.list_scroll_handle.offset().y).max(px(0.0));
        if max_y > px(0.0) && offset_y + px(240.0) >= max_y {
            self.trigger_load_more(cx);
        }
    }

    /// 立即预加载下一批邮件（优先从本地缓存/Demo 追加，本地见底后再向远端拉取更早批次）。
    pub(crate) fn trigger_load_more(&mut self, cx: &mut Context<Self>) {
        if self.current_view == CurrentView::Drafts && !self.demo_mode {
            return;
        }
        let batch = self.prefs.page_size.max(1);
        let loaded = self.current_page.items.len();

        // 1. 本地缓存或演示数据还有剩余：直接追加下一批。
        if loaded < self.current_page.total {
            if self.loading_more_local {
                return;
            }
            self.loaded_limit = loaded + batch;
            if self.demo_mode {
                self.reload_current_page(cx);
            } else {
                self.loading_more_local = true;
                self.send_page_request(loaded, batch);
                cx.notify();
            }
            return;
        }

        // 2. 本地已见底，但远端服务器仍有更早历史邮件：发起增量拉取。
        if self.current_view_has_more_remote() && !self.loading_more_remote {
            self.loading_more_remote = true;
            self.loaded_limit = loaded + batch;
            let target_folder = match self.current_view {
                CurrentView::UnifiedInbox => UNIFIED_INBOX_FOLDER_ID,
                CurrentView::AccountFolder => self.selected_folder_id,
                _ => return,
            };
            self.status_line = SharedString::from("正在预加载更早的邮件...");
            if let Some(ref w) = self.worker {
                w.send(MailRequest::LoadOlderMessages {
                    folder_id: target_folder,
                    limit: batch,
                });
            }
            cx.notify();
        }
    }

    pub(crate) fn account_email_for(&self, account_id: u32) -> &str {
        self.accounts
            .iter()
            .find(|a| a.id == account_id)
            .map(|a| a.email.as_str())
            .unwrap_or("")
    }

    /// 统一收件箱未读总数（各账号 INBOX 未读之和）。
    pub(crate) fn unified_unread_count(&self) -> u32 {
        self.folders_by_account
            .values()
            .flatten()
            .filter(|f| {
                f.path.eq_ignore_ascii_case("INBOX")
                    || f.name.eq_ignore_ascii_case("INBOX")
                    || f.name.contains("收件箱")
            })
            .map(|f| f.unread_count)
            .sum()
    }

    pub(crate) fn account_expanded(&self, account_id: u32) -> bool {
        !self.collapsed_accounts.contains(&account_id)
    }

    // ---- 迭代 1 新增的纯视觉动作 ----

    /// 菜单「切换亮/暗主题」：即时生效，并把选择固化为 `Some(bool)` 持久化。
    pub(crate) fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        self.dark_mode = !self.dark_mode;
        self.prefs.dark_mode = Some(self.dark_mode);
        theme::apply(self.theme_id, self.dark_mode, cx);
        if let Err(e) = config::save_preferences_in(&config::data_dir(), &self.prefs) {
            self.error_banner = Some(SharedString::from(e));
        }
        self.status_line = SharedString::from(if self.dark_mode {
            "已切换到暗色主题"
        } else {
            "已切换到亮色主题"
        });
        cx.notify();
    }

    pub(crate) fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_hidden = !self.sidebar_hidden;
        cx.notify();
    }

    #[allow(dead_code)]
    pub(crate) fn set_list_filter(&mut self, filter: ListFilter, cx: &mut Context<Self>) {
        if self.list_filter != filter {
            self.list_filter = filter;
            self.quick_filters = QuickFilterSet::from_message_filter(filter.to_mail_filter());
            self.reset_list_pagination();
            self.reload_current_page(cx);
        } else {
            cx.notify();
        }
    }

    pub(crate) fn set_list_sort(&mut self, sort: MessageSort, cx: &mut Context<Self>) {
        if self.list_sort != sort {
            self.list_sort = sort;
            self.reset_list_pagination();
            self.reload_current_page(cx);
        } else {
            cx.notify();
        }
    }

    pub(crate) fn toggle_quick_filter_unread(&mut self, cx: &mut Context<Self>) {
        self.quick_filters.unread = !self.quick_filters.unread;
        self.reset_list_pagination();
        self.reload_current_page(cx);
    }

    pub(crate) fn toggle_quick_filter_starred(&mut self, cx: &mut Context<Self>) {
        self.quick_filters.starred = !self.quick_filters.starred;
        self.reset_list_pagination();
        self.reload_current_page(cx);
    }

    pub(crate) fn toggle_quick_filter_attachment(&mut self, cx: &mut Context<Self>) {
        self.quick_filters.has_attachment = !self.quick_filters.has_attachment;
        self.reset_list_pagination();
        self.reload_current_page(cx);
    }

    pub(crate) fn toggle_quick_filter_today(&mut self, cx: &mut Context<Self>) {
        self.quick_filters.today = !self.quick_filters.today;
        self.reset_list_pagination();
        self.reload_current_page(cx);
    }

    pub(crate) fn clear_quick_filters(&mut self, cx: &mut Context<Self>) {
        self.quick_filters.clear();
        self.list_filter = ListFilter::All;
        self.reset_list_pagination();
        self.reload_current_page(cx);
    }

    pub(crate) fn set_search_scope(&mut self, scope: SearchScope, cx: &mut Context<Self>) {
        if self.search_scope != scope {
            self.search_scope = scope;
            self.reset_list_pagination();
            self.reload_current_page(cx);
        } else {
            cx.notify();
        }
    }

    pub(crate) fn clear_server_search(&mut self) {
        self.server_search_loading = false;
        self.server_search_query.clear();
        self.server_search_results.clear();
    }

    pub(crate) fn trigger_server_search(&mut self, cx: &mut Context<Self>) {
        let query = self.search_query.trim().to_string();
        if query.chars().count() < 2 || self.demo_mode || self.server_search_loading {
            return;
        }
        let account_id = if self.search_scope == SearchScope::ThisFolder
            && self.current_view == CurrentView::AccountFolder
        {
            self.folders_by_account
                .iter()
                .find(|(_, fs)| fs.iter().any(|f| f.id == self.selected_folder_id))
                .map(|(aid, _)| *aid)
        } else {
            None
        };
        self.server_search_loading = true;
        self.server_search_query = query.clone();
        self.status_line = SharedString::from(format!("正在搜索服务器：{query}..."));
        if let Some(ref w) = self.worker {
            w.send(MailRequest::SearchServer { query, account_id });
        }
        cx.notify();
    }

    pub(crate) fn toggle_account_expanded(&mut self, account_id: u32, cx: &mut Context<Self>) {
        if self.collapsed_accounts.contains(&account_id) {
            self.collapsed_accounts.remove(&account_id);
        } else {
            self.collapsed_accounts.insert(account_id);
        }
        cx.notify();
    }

    // ---- 阶段 3：全局路由、偏好、命令面板、快捷键（方案文档第五节）----

    /// 附件总览数据请求（进入视图 / 搜索词变化时触发；demo 模式直接加载模拟附件）。
    fn request_gallery_items(&mut self, cx: &mut Context<Self>) {
        if self.demo_mode {
            let items = self
                .demo_store
                .list_cached_attachments(&self.gallery_query, 0, 200);
            self.gallery_items = items.clone();
            if let Some(ref g) = self.gallery {
                g.update(cx, |state, cx| {
                    state.set_items(items);
                    cx.notify();
                });
            }
            return;
        }
        if let Some(ref w) = self.worker {
            w.send(MailRequest::ListCachedAttachments {
                query: self.gallery_query.clone(),
                offset: 0,
                limit: 200,
            });
        }
    }

    /// 进入附件总览主视图（侧栏底部入口）。
    pub(crate) fn open_gallery(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.gallery.is_none() {
            let entity = cx.new(|cx| GalleryState::new(window, cx));
            let sub = cx.subscribe_in(
                &entity,
                window,
                |this, _gallery, event: &GalleryEvent, _window, cx| match event {
                    GalleryEvent::QueryChanged(query) => {
                        this.gallery_query = query.clone();
                        this.request_gallery_items(cx);
                    }
                    // 账号筛选由组件内部做客户端过滤，无需重新请求。
                    GalleryEvent::AccountFilterChanged(_) => {}
                    GalleryEvent::SaveAttachment(attachment_id) => {
                        this.save_gallery_attachment(*attachment_id);
                    }
                    GalleryEvent::LocateMessage(message_id) => {
                        this.locate_gallery_message(*message_id, cx);
                    }
                },
            );
            self.gallery = Some(entity);
            self.gallery_sub = Some(sub);
        }
        self.main_view = MainContentView::Gallery;
        // 同步账号筛选列表并按当前搜索词重新拉取缓存数据。
        let accounts = self
            .accounts
            .iter()
            .map(|a| {
                let name = if a.display_name.is_empty() {
                    a.email.clone()
                } else {
                    a.display_name.clone()
                };
                (a.id, name)
            })
            .collect();
        if let Some(ref g) = self.gallery {
            g.update(cx, |state, _cx| state.set_accounts(accounts));
        }
        self.request_gallery_items(cx);
        cx.notify();
    }

    /// 附件总览「保存」：复用现有 rfd 保存对话框 + `SaveAttachment` 流程。
    fn save_gallery_attachment(&self, attachment_id: u32) {
        let filename = self
            .gallery_items
            .iter()
            .find(|item| item.id == attachment_id)
            .map(|item| item.filename.clone())
            .unwrap_or_else(|| "attachment".to_string());
        self.save_attachment(attachment_id, &filename);
    }

    /// 附件总览「定位到原邮件」：切回三栏并加载该邮件正文。
    fn locate_gallery_message(&mut self, message_id: u32, cx: &mut Context<Self>) {
        if self.demo_mode {
            self.main_view = MainContentView::Mail;
            self.right_mode = RightPaneMode::Reader;
            if let Some(detail) = self.demo_message_detail(message_id) {
                self.set_selected_message(detail);
            } else {
                self.selected_message = None;
            }
            cx.notify();
            return;
        }
        let Some(item) = self
            .gallery_items
            .iter()
            .find(|item| item.message_id == message_id)
        else {
            return;
        };
        let account_id = item.account_id;
        self.main_view = MainContentView::Mail;
        self.right_mode = RightPaneMode::Reader;
        if let Some(ref w) = self.worker {
            w.send(MailRequest::LoadMessageBody {
                account_id,
                message_id,
            });
        }
        cx.notify();
    }

    /// 联系人数据请求（进入视图 / 搜索词变化时触发；demo 模式直接聚合模拟发件人）。
    fn request_known_contacts(&mut self, cx: &mut Context<Self>) {
        if self.demo_mode {
            let items = self
                .demo_store
                .list_known_contacts(&self.contacts_query, 0, 500);
            if let Some(ref c) = self.contacts {
                c.update(cx, |state, cx| {
                    state.set_contacts(items);
                    cx.notify();
                });
            }
            return;
        }
        if let Some(ref w) = self.worker {
            w.send(MailRequest::ListKnownContacts {
                query: self.contacts_query.clone(),
                offset: 0,
                limit: 500,
            });
        }
    }

    /// 进入联系人主视图（侧栏底部入口）。
    pub(crate) fn open_contacts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.contacts.is_none() {
            let entity = cx.new(|cx| ContactsState::new(window, cx));
            let sub = cx.subscribe_in(
                &entity,
                window,
                |this, _contacts, event: &ContactsEvent, window, cx| match event {
                    ContactsEvent::QueryChanged(query) => {
                        this.contacts_query = query.clone();
                        this.request_known_contacts(cx);
                    }
                    ContactsEvent::ComposeTo(email) => {
                        this.compose_to_contact(email.clone(), window, cx);
                    }
                },
            );
            self.contacts = Some(entity);
            self.contacts_sub = Some(sub);
        }
        self.main_view = MainContentView::Contacts;
        self.request_known_contacts(cx);
        cx.notify();
    }

    /// 联系人页「给 TA 写信」：切回三栏并预填收件人。
    fn compose_to_contact(&mut self, email: String, window: &mut Window, cx: &mut Context<Self>) {
        self.main_view = MainContentView::Mail;
        self.start_new_compose(window, cx);
        self.compose_to
            .update(cx, |state, cx| state.set_value(email, window, cx));
        cx.notify();
    }

    /// 打开偏好设置模态（侧栏菜单 / 命令面板）。
    pub(crate) fn open_preferences(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.active_modal = ActiveModal::Preferences;
        if self.preferences.is_none() {
            let entity = cx.new(|cx| PreferencesState::new(window, cx, self.prefs.clone()));
            let sub = cx.subscribe(
                &entity,
                |this, _prefs, event: &PreferencesEvent, cx| match event {
                    PreferencesEvent::Changed(prefs) => this.apply_preferences(prefs.clone(), cx),
                    PreferencesEvent::Closed => this.close_modal(cx),
                    PreferencesEvent::ClearBodyCache => {
                        if let Some(ref w) = this.worker {
                            this.status_line = SharedString::from("正在清理正文缓存...");
                            w.send(MailRequest::ClearBodyCaches);
                        }
                        cx.notify();
                    }
                },
            );
            self.preferences = Some(entity);
            self.preferences_sub = Some(sub);
        }
        // 打开时刷新草稿 / 发件箱统计。
        let drafts = self.drafts.len();
        let outbox: usize = self
            .outbox_by_account
            .values()
            .map(|items| items.len())
            .sum();
        if let Some(ref p) = self.preferences {
            p.update(cx, |state, _cx| state.set_storage_stats(drafts, outbox));
        }
        cx.notify();
    }

    /// 应用偏好变更（`PreferencesEvent::Changed`）：落盘 + 主题 / 列表 / 定时器即时生效。
    fn apply_preferences(&mut self, prefs: AppPreferences, cx: &mut Context<Self>) {
        let page_size_changed = prefs.page_size != self.prefs.page_size;
        let sync_changed = prefs.auto_sync_minutes != self.prefs.auto_sync_minutes;
        let theme_changed =
            prefs.theme_id != self.prefs.theme_id || prefs.dark_mode != self.prefs.dark_mode;
        let thread_order_changed = prefs.thread_newest_first != self.prefs.thread_newest_first;
        let fold_changed = prefs.fold_older_messages != self.prefs.fold_older_messages;

        self.prefs = prefs;
        if let Err(e) = config::save_preferences_in(&config::data_dir(), &self.prefs) {
            self.error_banner = Some(SharedString::from(e));
        }
        if theme_changed {
            self.theme_id = theme::static_theme_id(&self.prefs.theme_id);
            self.dark_mode = self.prefs.dark_mode.unwrap_or(self.system_dark_mode);
            theme::apply(self.theme_id, self.dark_mode, cx);
        }
        if sync_changed {
            // 间隔变更：重置后台刷新定时（auto_sync_deadline 由下一次 tick 重算）。
            self.auto_sync_deadline = None;
        }
        if page_size_changed {
            self.reset_list_pagination();
            self.reload_current_page(cx);
        }
        if thread_order_changed {
            if let Some(ref mut detail) = self.selected_message {
                if detail.thread_messages.len() > 1 {
                    detail.thread_messages.reverse();
                }
            }
        }
        if fold_changed || thread_order_changed {
            if let Some(ref detail) = self.selected_message {
                self.collapsed_thread_cards = compute_default_collapsed_cards(
                    detail,
                    self.prefs.thread_newest_first,
                    self.prefs.fold_older_messages,
                );
            }
        }
        self.status_line = SharedString::from("偏好设置已保存");
        cx.notify();
    }

    /// 关闭当前浮层并卸载对应实体（偏好设置 / 命令面板）。
    pub(crate) fn close_modal(&mut self, cx: &mut Context<Self>) {
        self.active_modal = ActiveModal::None;
        self.preferences = None;
        self.preferences_sub = None;
        self.palette = None;
        self.palette_sub = None;
        cx.notify();
    }

    /// 打开命令面板浮层（侧栏菜单 / Ctrl+K）。
    pub(crate) fn open_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.active_modal = ActiveModal::CommandPalette;
        if self.palette.is_none() {
            let commands = self.palette_commands();
            let entity = cx.new(|cx| CommandPaletteState::new(window, cx, commands));
            let sub = cx.subscribe_in(
                &entity,
                window,
                |this, _palette, event: &CommandPaletteEvent, window, cx| match event {
                    CommandPaletteEvent::CommandExecuted(command) => {
                        this.execute_palette_command(*command, window, cx);
                    }
                    CommandPaletteEvent::Closed => this.close_modal(cx),
                },
            );
            self.palette = Some(entity);
            self.palette_sub = Some(sub);
        }
        let names = self.palette_folder_names();
        if let Some(ref p) = self.palette {
            p.update(cx, |state, _cx| state.set_folder_names(names));
        }
        cx.notify();
    }

    /// 固定命令 + 遍历 folders_by_account 生成的文件夹跳转命令。
    fn palette_commands(&self) -> Vec<PaletteCommand> {
        let mut commands = vec![
            PaletteCommand::NewCompose,
            PaletteCommand::RefreshAll,
            PaletteCommand::ToggleUnifiedInbox,
            PaletteCommand::OpenAccountSettings,
            PaletteCommand::OpenPreferences,
            PaletteCommand::ToggleTheme,
        ];
        for (&account_id, folders) in &self.folders_by_account {
            for folder in folders {
                commands.push(PaletteCommand::GoFolder {
                    account_id,
                    folder_id: folder.id,
                });
            }
        }
        commands
    }

    /// GoFolder 命令的显示名（多账号时带账号前缀）。
    fn palette_folder_names(&self) -> Vec<(u32, u32, String)> {
        let multi_account = self.accounts.len() > 1;
        self.folders_by_account
            .iter()
            .flat_map(|(&account_id, folders)| {
                let account_label = self
                    .accounts
                    .iter()
                    .find(|a| a.id == account_id)
                    .map(|a| {
                        if a.display_name.is_empty() {
                            a.email.clone()
                        } else {
                            a.display_name.clone()
                        }
                    })
                    .unwrap_or_default();
                folders.iter().map(move |folder| {
                    let name = if multi_account {
                        format!("{account_label} / {}", folder.name)
                    } else {
                        folder.name.clone()
                    };
                    (account_id, folder.id, name)
                })
            })
            .collect()
    }

    /// 分发命令面板选中的命令到现有动作。
    fn execute_palette_command(
        &mut self,
        command: PaletteCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match command {
            PaletteCommand::NewCompose => {
                self.main_view = MainContentView::Mail;
                self.start_new_compose(window, cx);
            }
            PaletteCommand::RefreshAll => {
                self.main_view = MainContentView::Mail;
                self.refresh_all(cx);
            }
            PaletteCommand::ToggleUnifiedInbox => {
                self.main_view = MainContentView::Mail;
                self.select_unified_view(CurrentView::UnifiedInbox, cx);
            }
            PaletteCommand::OpenAccountSettings => {
                self.close_modal(cx);
                self.open_account_manager(cx);
                return;
            }
            PaletteCommand::OpenPreferences => {
                // 先关闭命令面板再打开偏好设置，避免下方 close_modal 误关刚打开的偏好设置。
                self.close_modal(cx);
                self.main_view = MainContentView::Mail;
                self.open_preferences(window, cx);
                return;
            }
            PaletteCommand::ToggleTheme => {
                self.toggle_theme(cx);
            }
            PaletteCommand::GoFolder {
                account_id,
                folder_id,
            } => {
                self.main_view = MainContentView::Mail;
                let folder_path = self
                    .folders_by_account
                    .get(&account_id)
                    .and_then(|folders| folders.iter().find(|f| f.id == folder_id))
                    .map(|folder| folder.path.clone())
                    .unwrap_or_default();
                self.select_folder(account_id, folder_id, folder_path, cx);
            }
        }
        self.close_modal(cx);
    }

    /// 刷新全部（侧栏刷新按钮 / 命令面板 / Ctrl+R 共用）。
    pub(crate) fn refresh_all(&mut self, cx: &mut Context<Self>) {
        if self.demo_mode {
            self.reload_current_page(cx);
            cx.notify();
            return;
        }
        if let Some(ref w) = self.worker {
            self.error_banner = None;
            self.status_line = SharedString::from("正在独立刷新所有账号...");
            w.send(MailRequest::SyncAllAccounts);
        }
        cx.notify();
    }

    /// 后台定时刷新 tick（每 30 秒醒来，到期发 `SyncAllAccounts`）。
    fn auto_sync_tick(&mut self) {
        if self.demo_mode || self.prefs.auto_sync_minutes == 0 {
            return;
        }
        let Some(ref w) = self.worker else {
            return;
        };
        let interval = Duration::from_secs(u64::from(self.prefs.auto_sync_minutes) * 60);
        let now = Instant::now();
        let deadline = self.auto_sync_deadline.unwrap_or(now + interval);
        self.auto_sync_deadline = Some(deadline);
        if now >= deadline {
            w.send(MailRequest::SyncAllAccounts);
            self.auto_sync_deadline = Some(now + interval);
        }
    }

    /// 切换视图 / 文件夹时把「上次位置」写入偏好并落盘（切换频率低，同步写可接受）。
    fn remember_last_location(&mut self, folder_account_id: Option<u32>) {
        if self.demo_mode {
            return;
        }
        match self.current_view {
            CurrentView::UnifiedInbox => {
                self.prefs.last_view = Some("unified".to_string());
                self.prefs.last_folder = None;
            }
            CurrentView::Starred => {
                self.prefs.last_view = Some("starred".to_string());
                self.prefs.last_folder = None;
            }
            CurrentView::Sent => {
                self.prefs.last_view = Some("sent".to_string());
                self.prefs.last_folder = None;
            }
            CurrentView::Drafts => {
                self.prefs.last_view = Some("drafts".to_string());
                self.prefs.last_folder = None;
            }
            CurrentView::Archive => {
                self.prefs.last_view = Some("archive".to_string());
                self.prefs.last_folder = None;
            }
            CurrentView::AccountFolder => {
                if let Some(account_id) = folder_account_id {
                    self.prefs.last_view = None;
                    self.prefs.last_folder = Some((account_id, self.selected_folder_id));
                }
            }
        }
        if let Err(e) = config::save_preferences_in(&config::data_dir(), &self.prefs) {
            self.error_banner = Some(SharedString::from(e));
        }
    }

    /// 检查当前窗口焦点是否位于任意文本输入框（防止 Delete / 方向键与打字冲突）。
    fn is_text_input_focused(&self, window: &Window, cx: &Context<Self>) -> bool {
        self.search_input.focus_handle(cx).is_focused(window)
            || self.compose_to.focus_handle(cx).is_focused(window)
            || self.compose_cc.focus_handle(cx).is_focused(window)
            || self.compose_bcc.focus_handle(cx).is_focused(window)
            || self.compose_subject.focus_handle(cx).is_focused(window)
            || self.compose_body.focus_handle(cx).is_focused(window)
            || self.cfg_display_name.focus_handle(cx).is_focused(window)
            || self.cfg_email.focus_handle(cx).is_focused(window)
            || self.cfg_imap_host.focus_handle(cx).is_focused(window)
            || self.cfg_imap_port.focus_handle(cx).is_focused(window)
            || self.cfg_imap_user.focus_handle(cx).is_focused(window)
            || self.cfg_imap_pass.focus_handle(cx).is_focused(window)
            || self.cfg_smtp_host.focus_handle(cx).is_focused(window)
            || self.cfg_smtp_port.focus_handle(cx).is_focused(window)
            || self.cfg_smtp_user.focus_handle(cx).is_focused(window)
            || self.cfg_smtp_pass.focus_handle(cx).is_focused(window)
            || self.cfg_oauth_client_id.focus_handle(cx).is_focused(window)
            || self.cfg_signature.focus_handle(cx).is_focused(window)
    }

    /// 键盘方向键切换中栏下一封 (`delta = 1`) / 上一封 (`delta = -1`) 邮件。
    fn select_adjacent_message(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.current_view == CurrentView::Drafts && !self.demo_mode {
            return;
        }
        let items = &self.current_page.items;
        let current_idx = self
            .selected_message
            .as_ref()
            .and_then(|sel| items.iter().position(|m| m.id == sel.id));
        let Some(next_idx) = next_adjacent_index(current_idx, items.len(), delta) else {
            return;
        };
        let msg = items[next_idx].clone();
        self.selected_message_ids.clear();
        self.select_message(
            msg.account_id,
            msg.id,
            msg.uid,
            msg.unread,
            &msg.folder_path,
            cx,
        );
    }

    // ---- 右栏渲染（阅读态 / 写信 / 账号管理 / 欢迎页由 ui:: 渲染）----

    fn render_right_pane(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        match self.right_mode {
            RightPaneMode::Compose => {
                let p = theme::current(cx);
                let title = if self.compose_in_reply_to.is_empty() {
                    "写新邮件"
                } else {
                    "回复邮件"
                };
                let win_controls = window_controls::render_window_controls("compose-wc", window, cx);
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .bg(p.canvas)
                    .child(
                        h_flex()
                            .h(px(window_controls::HEADER_HEIGHT))
                            .flex_shrink_0()
                            .pl_4()
                            .items_center()
                            .border_b_1()
                            .border_color(p.border)
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                    .text_color(p.text_muted)
                                    .child(title),
                            )
                            .child(window_controls::drag_spacer())
                            .child(win_controls),
                    )
                    .child(compose::render(self, cx))
                    .into_any_element()
            }
            RightPaneMode::Welcome => welcome::render(self, window, cx),
            RightPaneMode::Reader => reader::render(self, window, cx),
        }
    }
}

impl Render for HylkiApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 避坑清单第 4 条：先把语义色 Copy 成局部变量再构建 UI。
        let p = theme::current(cx);
        let border = p.border;
        let muted_fg = p.text_muted;

        let sidebar_el = if self.sidebar_hidden {
            None
        } else {
            Some(sidebar::render(self, cx))
        };

        // 主内容区路由（方案文档五.1）：Gallery / Contacts 保留左栏，
        // 中栏 + 右栏区域整体替换为对应组件。
        let body = match self.main_view {
            MainContentView::Mail => {
                let list_el = message_list::render(self, cx);
                let right_el = self.render_right_pane(window, cx);
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .children(sidebar_el)
                    .child(list_el)
                    .child(right_el)
            }
            MainContentView::Gallery => {
                let win_controls =
                    window_controls::render_window_controls("gallery-wc", window, cx);
                let gallery_el = match self.gallery.clone() {
                    Some(entity) => div()
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .h_full()
                        .child(entity)
                        .child(
                            h_flex()
                                .absolute()
                                .top_0()
                                .right_0()
                                .h(px(window_controls::HEADER_HEIGHT))
                                .items_center()
                                .child(win_controls),
                        )
                        .into_any_element(),
                    None => div().flex_1().into_any_element(),
                };
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .children(sidebar_el)
                    .child(gallery_el)
            }
            MainContentView::Contacts => {
                let win_controls =
                    window_controls::render_window_controls("contacts-wc", window, cx);
                let contacts_el = match self.contacts.clone() {
                    Some(entity) => div()
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .h_full()
                        .child(entity)
                        .child(
                            h_flex()
                                .absolute()
                                .top_0()
                                .left(px(300.0))
                                .right_0()
                                .h(px(window_controls::HEADER_HEIGHT))
                                .items_center()
                                .child(window_controls::drag_spacer())
                                .child(win_controls),
                        )
                        .into_any_element(),
                    None => div().flex_1().into_any_element(),
                };
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .children(sidebar_el)
                    .child(contacts_el)
            }
        };

        let mut root = v_flex()
            .relative()
            .size_full()
            .bg(p.canvas)
            .text_color(p.text)
            .when_some(self.error_banner.as_ref(), |el, err| {
                el.child(
                    h_flex()
                        .px_3()
                        .py_1()
                        .border_b_1()
                        .border_color(border)
                        .justify_between()
                        .items_center()
                        .child(div().text_xs().text_color(p.danger).child(err.clone()))
                        .child(Button::new("dismiss-err").ghost().label("关闭").on_click(
                            cx.listener(|this, _, _, cx| {
                                this.error_banner = None;
                                cx.notify();
                            }),
                        )),
                )
            })
            .when_some(self.pending_send.as_ref(), |el, pending| {
                let token = pending.token;
                let recipient = if pending.draft.to.trim().is_empty() {
                    "(未填写收件人)".to_string()
                } else {
                    pending.draft.to.trim().to_string()
                };
                let secs = pending.remaining_secs;
                el.child(
                    h_flex()
                        .px_3()
                        .py_1p5()
                        .gap_2()
                        .border_b_1()
                        .border_color(border)
                        .bg(p.accent_soft())
                        .justify_between()
                        .items_center()
                        .child(
                            div()
                                .text_xs()
                                .font_weight(gpui_kit::FontWeight::MEDIUM)
                                .text_color(p.text)
                                .truncate()
                                .child(SharedString::from(format!(
                                    "正在准备发送给 {recipient}（{secs} 秒后发出）..."
                                ))),
                        )
                        .child(
                            h_flex()
                                .gap_1p5()
                                .flex_shrink_0()
                                .items_center()
                                .child(
                                    Button::new("undo-send-btn")
                                        .primary()
                                        .compact()
                                        .label("撤销发送")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.undo_pending_send(window, cx);
                                        })),
                                )
                                .child(
                                    Button::new("send-now-btn")
                                        .outline()
                                        .compact()
                                        .label("立即发送")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.commit_pending_send(token, cx);
                                        })),
                                ),
                        ),
                )
            })
            .child(body)
            .when(!self.status_line.is_empty(), |el| {
                el.child(
                    h_flex()
                        .h(px(24.0))
                        .flex_shrink_0()
                        .px_3()
                        .items_center()
                        .border_t_1()
                        .border_color(border)
                        .bg(p.sidebar)
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted_fg)
                                .truncate()
                                .child(self.status_line.clone()),
                        ),
                )
            })
            // 全局快捷键动作（键绑定注册在 main.rs，方案文档五.3）。
            .on_action(cx.listener(|this, _: &NewCompose, window, cx| {
                // 浮层打开时不响应（先 Esc 关闭），避免动作穿透。
                if this.active_modal == ActiveModal::None {
                    this.main_view = MainContentView::Mail;
                    this.start_new_compose(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &OpenCommandPalette, window, cx| {
                if this.active_modal == ActiveModal::None {
                    this.open_command_palette(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                // 仅三栏视图且无浮层时聚焦中栏搜索框。
                if this.main_view == MainContentView::Mail && this.active_modal == ActiveModal::None
                {
                    this.search_input.update(cx, |state, cx| {
                        state.focus(window, cx);
                    });
                }
            }))
            .on_action(cx.listener(|this, _: &RefreshAllAccounts, _window, cx| {
                if this.active_modal == ActiveModal::None {
                    this.refresh_all(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SendCompose, _window, cx| {
                if this.active_modal == ActiveModal::None
                    && this.main_view == MainContentView::Mail
                    && this.right_mode == RightPaneMode::Compose
                {
                    this.send_compose(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &OpenPreferences, window, cx| {
                if this.active_modal == ActiveModal::None {
                    this.open_preferences(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &QuitApp, _window, cx| {
                this.quit_app(cx);
            }))
            .on_action(cx.listener(|this, _: &DeleteSelectedMessage, window, cx| {
                if this.active_modal == ActiveModal::None
                    && this.main_view == MainContentView::Mail
                    && this.right_mode == RightPaneMode::Reader
                    && !this.is_text_input_focused(window, cx)
                {
                    if this.selected_message_ids.len() > 1 {
                        this.batch_delete(cx);
                    } else {
                        this.delete_selected(cx);
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &SelectNextMessage, window, cx| {
                if this.active_modal == ActiveModal::None
                    && this.main_view == MainContentView::Mail
                    && this.right_mode == RightPaneMode::Reader
                    && !this.is_text_input_focused(window, cx)
                {
                    this.select_adjacent_message(1, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SelectPrevMessage, window, cx| {
                if this.active_modal == ActiveModal::None
                    && this.main_view == MainContentView::Mail
                    && this.right_mode == RightPaneMode::Reader
                    && !this.is_text_input_focused(window, cx)
                {
                    this.select_adjacent_message(-1, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &CloseOverlay, window, cx| {
                // Esc 优先级：关闭浮层 > 写信退出确认 > 撤销发送 > 取消多选。
                if this.active_modal != ActiveModal::None {
                    this.close_modal(cx);
                } else if this.main_view == MainContentView::Mail
                    && this.right_mode == RightPaneMode::Compose
                {
                    this.request_close_compose(cx);
                } else if this.pending_send.is_some() && !this.is_text_input_focused(window, cx) {
                    this.undo_pending_send(window, cx);
                } else if !this.selected_message_ids.is_empty() {
                    this.clear_message_selection(cx);
                }
            }));

        // 全局浮层（方案文档五.1）：偏好设置为遮罩 + 居中卡片；
        // 命令面板组件自带遮罩 + 居中卡片，直接挂载即可。
        match self.active_modal {
            ActiveModal::Accounts => {
                let accounts_el = accounts::render(self, cx);
                root = root.child(
                    div()
                        .absolute()
                        .top(px(0.0))
                        .left(px(0.0))
                        .size_full()
                        .bg(black().opacity(0.4))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.close_modal(cx);
                            }),
                        )
                        .child(
                            h_flex().size_full().items_center().justify_center().child(
                                v_flex()
                                    .w(px(780.0))
                                    .h(px(620.0))
                                    .bg(p.surface)
                                    .rounded(px(12.0))
                                    .border_1()
                                    .border_color(border)
                                    .shadow_xl()
                                    .overflow_hidden()
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|_, _, _, cx| {
                                            cx.stop_propagation();
                                        }),
                                    )
                                    .child(accounts_el),
                            ),
                        ),
                );
            }
            ActiveModal::Preferences => {
                if let Some(entity) = self.preferences.clone() {
                    root = root.child(
                        div()
                            .absolute()
                            .top(px(0.0))
                            .left(px(0.0))
                            .size_full()
                            .bg(black().opacity(0.4))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    this.close_modal(cx);
                                }),
                            )
                            .child(
                                h_flex().size_full().items_center().justify_center().child(
                                    v_flex()
                                        .w(px(760.0))
                                        .h(px(560.0))
                                        .bg(p.surface)
                                        .rounded(px(12.0))
                                        .border_1()
                                        .border_color(border)
                                        .shadow_xl()
                                        .overflow_hidden()
                                        .on_mouse_down(
                                            MouseButton::Left,
                                            cx.listener(|_, _, _, cx| {
                                                cx.stop_propagation();
                                            }),
                                        )
                                        .child(entity),
                                ),
                            ),
                    );
                }
            }
            ActiveModal::CommandPalette => {
                if let Some(entity) = self.palette.clone() {
                    root = root.child(entity);
                }
            }
            ActiveModal::None => {}
        }
        root
    }
}

/// 检测单个账号的收件箱未读增长量：首次出现仅建立基线返回 0，避免启动加载误报通知。
pub(crate) fn detect_new_unread(
    prev_by_account: &mut BTreeMap<u32, u32>,
    account_id: u32,
    current_inbox_unread: u32,
) -> u32 {
    let delta = match prev_by_account.get(&account_id).copied() {
        Some(prev) if current_inbox_unread > prev => current_inbox_unread - prev,
        _ => 0,
    };
    prev_by_account.insert(account_id, current_inbox_unread);
    delta
}

/// 将 GPUI `WindowBounds` 写入偏好中的窗口尺寸与最大化标记（忽略最小化时的退化尺寸）。
pub(crate) fn capture_window_geometry(prefs: &mut AppPreferences, bounds: WindowBounds) -> bool {
    let (maximized, rect) = match bounds {
        WindowBounds::Windowed(b) => (false, b),
        WindowBounds::Maximized(b) => (true, b),
        WindowBounds::Fullscreen(b) => (prefs.window_maximized, b),
    };
    let w: f32 = rect.size.width.into();
    let h: f32 = rect.size.height.into();
    let mut changed = false;
    if prefs.window_maximized != maximized {
        prefs.window_maximized = maximized;
        changed = true;
    }
    if w.is_finite() && h.is_finite() && w >= 320.0 && h >= 240.0 {
        if prefs.window_width != Some(w) || prefs.window_height != Some(h) {
            prefs.window_width = Some(w);
            prefs.window_height = Some(h);
            changed = true;
        }
    }
    changed
}

/// 计算多封会话打开时默认折叠的卡片 ID 集合：
/// 当会话有 >=2 封邮件且 `fold_older_messages == true` 时，默认展开最新一封与当前选中封，其余折叠。
fn compute_default_collapsed_cards(
    detail: &MessageDetail,
    thread_newest_first: bool,
    fold_older_messages: bool,
) -> HashSet<u32> {
    if !fold_older_messages || detail.thread_messages.len() < 2 {
        return HashSet::new();
    }
    let newest_id = if thread_newest_first {
        detail.thread_messages.first().map(|m| m.id)
    } else {
        detail.thread_messages.last().map(|m| m.id)
    };
    detail
        .thread_messages
        .iter()
        .map(|m| m.id)
        .filter(|&id| id != detail.id && Some(id) != newest_id)
        .collect()
}

/// 拆分发件人字段为 (显示名, 邮箱)：`"Marcus Chen <marcus@x>"` →
/// `("Marcus Chen", "marcus@x")`；无尖括号时整体为显示名。
pub(crate) fn split_sender(sender: &str) -> (String, String) {
    if let (Some(open), Some(close)) = (sender.find('<'), sender.rfind('>')) {
        if close > open {
            let name = sender[..open].trim().to_string();
            let email = sender[open + 1..close].trim().to_string();
            return (name, email);
        }
    }
    (sender.trim().to_string(), String::new())
}

/// 若账号配置了非空签名，则使用标准 `-- \n` 分隔符将其插入正文末尾。
pub(crate) fn append_signature(body: &str, signature: &str) -> String {
    let sig = signature.trim_end();
    if sig.trim().is_empty() {
        return body.to_string();
    }
    if body.is_empty() {
        format!("\n\n-- \n{sig}")
    } else {
        format!("{}\n\n-- \n{sig}", body.trim_end_matches('\n'))
    }
}

/// 判断写信表单是否存在未保存内容（任意地址/主题/正文非空或有附件）。
pub(crate) fn has_unsaved_compose_content(
    to: &str,
    cc: &str,
    bcc: &str,
    subject: &str,
    body: &str,
    attachment_count: usize,
) -> bool {
    !to.trim().is_empty()
        || !cc.trim().is_empty()
        || !bcc.trim().is_empty()
        || !subject.trim().is_empty()
        || !body.trim().is_empty()
        || attachment_count > 0
}

/// 根据收件人输入框最后一段（最后一个逗号后）过滤匹配的已知联系人，最多返回 `max_items` 条。
pub(crate) fn filter_contact_suggestions<'a>(
    input: &str,
    contacts: &'a [KnownContact],
    max_items: usize,
) -> Vec<&'a KnownContact> {
    if max_items == 0 {
        return Vec::new();
    }
    let parts: Vec<&str> = input.split(',').collect();
    let Some(last) = parts.last() else {
        return Vec::new();
    };
    let token = last.trim().to_lowercase();
    if token.is_empty() {
        return Vec::new();
    }
    // 排除前面已输入的邮箱，避免重复建议。
    let existing: HashSet<String> = parts[..parts.len().saturating_sub(1)]
        .iter()
        .map(|p| {
            let (_, email) = split_sender(p);
            if email.is_empty() {
                p.trim().to_lowercase()
            } else {
                email.to_lowercase()
            }
        })
        .filter(|s| !s.is_empty())
        .collect();

    contacts
        .iter()
        .filter(|c| !existing.contains(&c.email.to_lowercase()))
        .filter(|c| {
            c.name.to_lowercase().contains(&token) || c.email.to_lowercase().contains(&token)
        })
        .take(max_items)
        .collect()
}

/// 将选中的联系人补全到当前收件人输入字符串末尾，并追加 `, `。
pub(crate) fn apply_contact_completion(current: &str, contact: &KnownContact) -> String {
    let formatted = if contact.name.trim().is_empty() {
        contact.email.clone()
    } else {
        format!("{} <{}>", contact.name.trim(), contact.email)
    };
    let mut parts: Vec<String> = current
        .split(',')
        .map(|p| p.trim().to_string())
        .collect();
    if !parts.is_empty() {
        parts.pop();
    }
    parts.retain(|p| !p.is_empty());
    parts.push(formatted);
    format!("{}, ", parts.join(", "))
}

/// 计算方向键切换上一封 / 下一封邮件的目标索引。
pub(crate) fn next_adjacent_index(
    current: Option<usize>,
    len: usize,
    delta: isize,
) -> Option<usize> {
    if len == 0 {
        return None;
    }
    match current {
        None => {
            if delta >= 0 {
                Some(0)
            } else {
                Some(len - 1)
            }
        }
        Some(idx) => {
            let next = idx as isize + delta;
            if next < 0 || next >= len as isize {
                None
            } else {
                Some(next as usize)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_sender_parses_display_name_and_email() {
        let (name, email) = split_sender("Marcus Chen <marcus@studio.dev>");
        assert_eq!(name, "Marcus Chen");
        assert_eq!(email, "marcus@studio.dev");
        let (name, email) = split_sender("marcus@studio.dev");
        assert_eq!(name, "marcus@studio.dev");
        assert_eq!(email, "");
        let (name, email) = split_sender(" 张三 <zhang@exmail.qq.com> ");
        assert_eq!(name, "张三");
        assert_eq!(email, "zhang@exmail.qq.com");
    }

    #[test]
    fn list_filter_labels_are_visible() {
        assert_eq!(ListFilter::All.label(), "全部");
        assert_eq!(ListFilter::Unread.label(), "仅未读");
        assert_eq!(ListFilter::Starred.label(), "仅星标");
    }

    #[test]
    fn unified_inbox_constant_stays_zero() {
        assert_eq!(UNIFIED_INBOX_FOLDER_ID, 0);
    }

    /// 阶段 3：last_view 字符串 ↔ 统一视图映射（未知值回落 None）。
    #[test]
    fn last_view_strings_map_back_to_views() {
        assert_eq!(
            HylkiApp::current_view_from_last(Some("unified")),
            Some(CurrentView::UnifiedInbox)
        );
        assert_eq!(
            HylkiApp::current_view_from_last(Some("starred")),
            Some(CurrentView::Starred)
        );
        assert_eq!(
            HylkiApp::current_view_from_last(Some("sent")),
            Some(CurrentView::Sent)
        );
        assert_eq!(
            HylkiApp::current_view_from_last(Some("drafts")),
            Some(CurrentView::Drafts)
        );
        assert_eq!(
            HylkiApp::current_view_from_last(Some("archive")),
            Some(CurrentView::Archive)
        );
        assert_eq!(HylkiApp::current_view_from_last(Some("bogus")), None);
        assert_eq!(HylkiApp::current_view_from_last(None), None);
    }

    #[test]
    fn detect_new_unread_skips_initial_load_and_reports_subsequent_growth() {
        let mut baseline = BTreeMap::new();
        // 首次启动加载缓存或初始同步：只记录基线，不触发新邮件通知。
        assert_eq!(detect_new_unread(&mut baseline, 1, 10), 0);
        // 未读数不变：不触发通知。
        assert_eq!(detect_new_unread(&mut baseline, 1, 10), 0);
        // 后台同步新增 3 封未读：返回增量 3。
        assert_eq!(detect_new_unread(&mut baseline, 1, 13), 3);
        // 用户阅读后未读下降：更新基线且不触发通知。
        assert_eq!(detect_new_unread(&mut baseline, 1, 8), 0);
        // 再次收到 1 封新邮件：基于最新基线 8 返回 1。
        assert_eq!(detect_new_unread(&mut baseline, 1, 9), 1);
    }

    #[test]
    fn capture_window_geometry_updates_windowed_and_maximized_states() {
        use gpui_kit::{size, Bounds};
        let mut prefs = AppPreferences::default();

        let b1 = Bounds::new(point(px(10.0), px(20.0)), size(px(1400.0), px(860.0)));
        assert!(capture_window_geometry(&mut prefs, WindowBounds::Windowed(b1)));
        assert_eq!(prefs.window_width, Some(1400.0));
        assert_eq!(prefs.window_height, Some(860.0));
        assert!(!prefs.window_maximized);

        // 相同尺寸再次保存应返回 false。
        assert!(!capture_window_geometry(&mut prefs, WindowBounds::Windowed(b1)));

        // 最大化状态：更新 window_maximized 且保留 restore bounds。
        assert!(capture_window_geometry(&mut prefs, WindowBounds::Maximized(b1)));
        assert!(prefs.window_maximized);
        assert_eq!(prefs.window_width, Some(1400.0));
        assert_eq!(prefs.window_height, Some(860.0));

        // 最小化退化尺寸 (0x0) 不应覆盖已记录的有效宽高。
        let degenerate = Bounds::new(point(px(0.0), px(0.0)), size(px(0.0), px(0.0)));
        assert!(!capture_window_geometry(
            &mut prefs,
            WindowBounds::Maximized(degenerate)
        ));
        assert_eq!(prefs.window_width, Some(1400.0));
        assert_eq!(prefs.window_height, Some(860.0));
    }

    #[test]
    fn compute_default_collapsed_cards_keeps_newest_and_current_expanded() {
        let make_tm = |id: u32| hylki_mail_core::ThreadMessage {
            id,
            rfc_message_id: format!("<{id}@test>"),
            subject: "Test".to_string(),
            sender: "a@test".to_string(),
            recipients: "b@test".to_string(),
            date_display: "10:00".to_string(),
            body_text: "body".to_string(),
            body_html: None,
            body_loaded: true,
            flagged: false,
            attachments: Vec::new(),
        };
        let detail = MessageDetail {
            id: 2,
            account_id: 1,
            folder_id: 1,
            folder_path: "INBOX".to_string(),
            uid: 2,
            rfc_message_id: "<2@test>".to_string(),
            sender: "a@test".to_string(),
            recipients: "b@test".to_string(),
            subject: "Test".to_string(),
            date_display: "10:00".to_string(),
            body_text: "body".to_string(),
            body_html: None,
            flagged: false,
            attachments: Vec::new(),
            // 降序：3 最新，2 当前，1 最早。
            thread_messages: vec![make_tm(3), make_tm(2), make_tm(1)],
        };

        let folded_desc = compute_default_collapsed_cards(&detail, true, true);
        assert!(folded_desc.contains(&1));
        assert!(!folded_desc.contains(&2));
        assert!(!folded_desc.contains(&3));

        let unfolded = compute_default_collapsed_cards(&detail, true, false);
        assert!(unfolded.is_empty());
    }

    #[test]
    fn append_signature_inserts_standard_separator() {
        assert_eq!(append_signature("hello", ""), "hello");
        assert_eq!(append_signature("hello", "   \n "), "hello");
        assert_eq!(append_signature("", "张三\n研发部"), "\n\n-- \n张三\n研发部");
        assert_eq!(
            append_signature("\n\n--- 原始邮件 ---\n正文\n", "Best,\nAlice  "),
            "\n\n--- 原始邮件 ---\n正文\n\n-- \nBest,\nAlice"
        );
    }

    #[test]
    fn has_unsaved_compose_content_detects_any_non_empty_field_or_attachment() {
        assert!(!has_unsaved_compose_content("", " ", "", "\n", "  ", 0));
        assert!(has_unsaved_compose_content("a@b.com", "", "", "", "", 0));
        assert!(has_unsaved_compose_content("", "", "", "主题", "", 0));
        assert!(has_unsaved_compose_content("", "", "", "", "正文", 0));
        assert!(has_unsaved_compose_content("", "", "", "", "", 1));
    }

    #[test]
    fn contact_autocomplete_filters_limits_dedupes_and_applies() {
        let contacts = vec![
            KnownContact {
                name: "林晓明".to_string(),
                email: "xiaoming@example.com".to_string(),
                message_count: 10,
            },
            KnownContact {
                name: "陈思远".to_string(),
                email: "siyuan@example.org".to_string(),
                message_count: 8,
            },
            KnownContact {
                name: String::new(),
                email: "alice@example.com".to_string(),
                message_count: 6,
            },
            KnownContact {
                name: "Bob".to_string(),
                email: "bob@example.com".to_string(),
                message_count: 5,
            },
            KnownContact {
                name: "Carol".to_string(),
                email: "carol@example.com".to_string(),
                message_count: 4,
            },
            KnownContact {
                name: "Dave".to_string(),
                email: "dave@example.com".to_string(),
                message_count: 3,
            },
            KnownContact {
                name: "Eve".to_string(),
                email: "eve@example.com".to_string(),
                message_count: 2,
            },
        ];

        // 空输入不弹建议
        assert!(filter_contact_suggestions("", &contacts, 5).is_empty());
        assert!(filter_contact_suggestions("xiaoming@example.com, ", &contacts, 5).is_empty());

        // 最多返回 5 条
        let many = filter_contact_suggestions("example", &contacts, 5);
        assert_eq!(many.len(), 5);

        // 排除前面已经填过的联系人
        let after_first = filter_contact_suggestions("林晓明 <xiaoming@example.com>, xiao", &contacts, 5);
        assert!(after_first.is_empty());

        // 中文姓名匹配 + 补全追加逗号
        let matched = filter_contact_suggestions("思远", &contacts, 5);
        assert_eq!(matched.len(), 1);
        assert_eq!(
            apply_contact_completion("林晓明 <xiaoming@example.com>, 思远", matched[0]),
            "林晓明 <xiaoming@example.com>, 陈思远 <siyuan@example.org>, "
        );

        // 无姓名联系人直接补全邮箱并追加逗号
        assert_eq!(
            apply_contact_completion("ali", &contacts[2]),
            "alice@example.com, "
        );
    }

    #[test]
    fn next_adjacent_index_respects_bounds() {
        assert_eq!(next_adjacent_index(None, 0, 1), None);
        assert_eq!(next_adjacent_index(None, 3, 1), Some(0));
        assert_eq!(next_adjacent_index(None, 3, -1), Some(2));
        assert_eq!(next_adjacent_index(Some(0), 3, 1), Some(1));
        assert_eq!(next_adjacent_index(Some(0), 3, -1), None);
        assert_eq!(next_adjacent_index(Some(2), 3, 1), None);
        assert_eq!(next_adjacent_index(Some(2), 3, -1), Some(1));
    }
}
