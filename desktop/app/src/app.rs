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

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    actions, black, div, point, px, AnyWindowHandle, AppContext, ClipboardItem, Context, Entity,
    Focusable, InteractiveElement, IntoElement, MouseButton, ParentElement, Render, ScrollHandle,
    SharedString, Styled, Subscription, Window, WindowAppearance, WindowBounds,
};
use hylki_mail_core::cache::{sanitize_filename, CachedAttachment, KnownContact};
use hylki_mail_core::config::{self, AccountConfig, AppPreferences, AuthBackend, SecurityMode};
use hylki_mail_core::demo::{
    DemoStore, DEMO_ARCHIVE_ID, DEMO_DRAFTS_ID, DEMO_INBOX_ID, DEMO_SENT_ID,
};
use hylki_mail_core::worker::MailWorkerHandle;
use hylki_mail_core::{
    contacts_io, ComposeDraft, FolderSummary, LocalDraft, MailPage, MailRequest, MessageDetail,
    MessageFilter, MessageSort, MessageSummary, OutboxItem, QuickFilterSet, SearchScope,
    UnifiedView, UserContact, WorkerEvent,
};

use crate::theme;
use crate::tray::{self, TrayCommand, TrayHandle};
use crate::ui::command_palette::{CommandPaletteEvent, CommandPaletteState, PaletteCommand};
use crate::ui::contacts::{ContactExportKind, ContactsEvent, ContactsState};
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

// 全局快捷键动作（方案文档五.3）；键绑定注册在 main.rs，
// 动作监听挂在根节点 render 上。
//
// F-1 阶段三：`Reply*` / `ViewMessageSource` 为 Ctrl 组合键动作（不受单键偏好
// 控制）；`SingleKey*` 前缀为单键动作，on_action 守卫里统一检查
// `prefs.single_key_shortcuts`（偏好设置「常规」页可关闭）。
actions!(
    hylki,
    [
        NewCompose,
        OpenCommandPalette,
        OpenCommandPaletteActions,
        FocusSearch,
        RefreshAllAccounts,
        CloseOverlay,
        SendCompose,
        OpenPreferences,
        QuitApp,
        DeleteSelectedMessage,
        SelectNextMessage,
        SelectPrevMessage,
        UndoLastMove,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        ToggleFullscreen,
        ToggleAccountsList,
        OpenShortcutsWindow,
        // ---- F-1 阶段三：组合键（始终生效）----
        ReplyCurrent,
        ReplyAllCurrent,
        ViewMessageSource,
        // ---- F-1 阶段三：单键（受 single_key_shortcuts 偏好控制）----
        SingleKeyNextMessage,
        SingleKeyPrevMessage,
        SingleKeyReply,
        SingleKeyReplyAll,
        SingleKeyForward,
        SingleKeyArchive,
        SingleKeyDelete,
        SingleKeyMarkSpam,
        SingleKeyToggleStar,
        SingleKeyToggleRead,
        SingleKeyCompose,
        SingleKeyBackToList,
        SingleKeyNextCard,
        SingleKeyPrevCard
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
    /// 键盘快捷键速查（F1 / ?，对齐原版 win.shortcuts）。
    Shortcuts,
    /// 关于 Hylki（对齐原版 win.about）。
    About,
}

/// F-3：最近一次可撤销的移动 / 删除 / 归档记录（状态栏轻量撤销条，15 秒后自动消失）。
#[derive(Clone, Debug)]
pub(crate) struct UndoRecord {
    pub(crate) account_id: u32,
    pub(crate) message_id: u32,
    /// RFC Message-ID（撤销时用于在服务端当前文件夹定位邮件）。
    pub(crate) rfc_message_id: String,
    /// 撤销时邮件所在的文件夹（回收站 / 归档 / 垃圾 / 移动目标）。
    pub(crate) current_folder_path: String,
    /// 移动 / 删除 / 归档前邮件所在的文件夹路径（撤销时的恢复目标）。
    pub(crate) original_folder_path: String,
    /// 动作描述（如「已删除」「已归档」「已移动」）。
    pub(crate) action_label: SharedString,
    /// 记录时刻（用于 15 秒过期与定时清理比对）。
    pub(crate) at: Instant,
}

/// F-3：请求发出时捕获的撤销前半段信息（成功回执到达后补全为 [`UndoRecord`]）。
#[derive(Clone, Debug)]
struct PendingUndo {
    account_id: u32,
    message_id: u32,
    original_folder_path: String,
    action_label: SharedString,
    at: Instant,
}

impl PendingUndo {
    /// 用成功回执携带的服务端定位信息补全撤销记录。
    fn into_record(self, dest_folder_path: String, rfc_message_id: String) -> UndoRecord {
        UndoRecord {
            account_id: self.account_id,
            message_id: self.message_id,
            rfc_message_id,
            current_folder_path: dest_folder_path,
            original_folder_path: self.original_folder_path,
            action_label: self.action_label,
            at: self.at,
        }
    }
}

/// 撤销条过期时长（秒）。
const UNDO_TTL_SECS: u64 = 15;

/// B-2：列表行菜单触发的「等正文加载完成后执行」的后续动作。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PendingRowAction {
    Reply,
    ReplyAll,
    Forward,
    CopyBody,
    ViewSource,
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
    /// F-4 草稿自动保存：防抖代际（每次输入变更 +1，旧定时器醒来后失效）。
    compose_autosave_generation: u64,
    /// F-4 自上次落库以来写信内容有变更（变更事件仅由真实编辑触发）。
    compose_autosave_dirty: bool,
    /// F-4 自动保存请求已发出（`DraftSaved` 回执到达后显示提示并绑定草稿 ID）。
    compose_autosave_pending: bool,
    /// F-4 当前草稿是否由自动保存静默创建（「放弃」时连同该草稿删除）。
    compose_autosaved_new: bool,
    /// F-4 最近一次自动保存时刻（写信页常驻小字「草稿已自动保存 HH:MM」）。
    pub(crate) compose_autosaved_at: Option<SharedString>,
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
    /// 中栏搜索栏是否展开（对齐原版：顶栏常驻 🔍 图标，点按或 Ctrl+F 展开）。
    pub(crate) search_open: bool,
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
    /// 是否在左栏显示账号列表（对齐原版 show-accounts 偏好，Ctrl+Shift+A / 主菜单切换）。
    pub(crate) show_accounts: bool,
    /// 待二次确认的「清空文件夹」目标 `(account_id, folder_id, folder_path, 显示名)`。
    pub(crate) pending_empty_folder: Option<(u32, u32, String, String)>,
    /// 阅读器是否强制以纯文本渲染（false = 有 HTML 时渲染富文本）。
    pub(crate) reader_prefer_plain: bool,
    /// 本次会话中用户手动放行远程图片的邮件 ID 集合。
    pub(crate) allowed_remote_image_ids: HashSet<u32>,
    /// 当前会话阅读器中处于折叠状态的卡片邮件 ID 集合。
    pub(crate) collapsed_thread_cards: HashSet<u32>,
    /// 当前会话阅读器中处于「查看邮件原文」模式的卡片邮件 ID 集合。
    pub(crate) raw_source_message_ids: HashSet<u32>,
    /// B-2：列表行菜单挂起的后续动作（等对应正文加载完成后执行）。
    pub(crate) pending_row_action: Option<(u32, PendingRowAction)>,
    /// B-2：正在抓取原始 MIME 以导出 .eml 的邮件 ID（「导出中…」状态，防重复点击）。
    pub(crate) pending_eml_export: Option<u32>,
    /// F-3：进行中的移动 / 删除 / 归档（成功回执到达后转成 `last_move_undo`）。
    pending_undo: Option<PendingUndo>,
    /// F-3：最近一次可撤销的移动 / 删除 / 归档记录（状态栏撤销条）。
    pub(crate) last_move_undo: Option<UndoRecord>,

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
    /// F-6 图片预览等待字节回传的附件 ID（`AttachmentBytesFetched` 时匹配）。
    gallery_preview_pending: Option<u32>,
    /// F-7 退订横幅待确认的邮件 ID（横幅内二次确认；切换邮件时清除）。
    pub(crate) pending_unsubscribe: Option<u32>,
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
                this.schedule_draft_autosave(cx);
                cx.notify();
            }
        })
        .detach();
        cx.subscribe(&compose_cc, |this, _input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.compose_autocomplete_field = Some(ComposeAddressField::Cc);
                this.schedule_draft_autosave(cx);
                cx.notify();
            }
        })
        .detach();
        cx.subscribe(&compose_bcc, |this, _input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.compose_autocomplete_field = Some(ComposeAddressField::Bcc);
                this.schedule_draft_autosave(cx);
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
        // F-4：主题 / 正文变更同样重置 5 秒防抖自动保存定时器。
        cx.subscribe(&compose_subject, |this, _input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.schedule_draft_autosave(cx);
                cx.notify();
            }
        })
        .detach();
        cx.subscribe(&compose_body, |this, _input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.schedule_draft_autosave(cx);
                cx.notify();
            }
        })
        .detach();

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
                .map(|d| {
                    compute_default_collapsed_cards(
                        d,
                        prefs.thread_newest_first,
                        prefs.fold_older_messages,
                    )
                })
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
                compose_autosave_generation: 0,
                compose_autosave_dirty: false,
                compose_autosave_pending: false,
                compose_autosaved_new: false,
                compose_autosaved_at: None,
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
                search_open: false,
                list_sort: MessageSort::DateNewest,
                quick_filters: QuickFilterSet::default(),
                search_scope: SearchScope::ThisFolder,
                server_search_loading: false,
                server_search_query: String::new(),
                server_search_results: Vec::new(),
                collapsed_accounts: HashSet::new(),
                show_accounts: true,
                pending_empty_folder: None,
                reader_prefer_plain: false,
                allowed_remote_image_ids: HashSet::new(),
                collapsed_thread_cards: initial_collapsed,
                raw_source_message_ids: HashSet::new(),
                pending_row_action: None,
                pending_eml_export: None,
                pending_undo: None,
                last_move_undo: None,
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
                gallery_preview_pending: None,
                pending_unsubscribe: None,
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
            compose_autosave_generation: 0,
            compose_autosave_dirty: false,
            compose_autosave_pending: false,
            compose_autosaved_new: false,
            compose_autosaved_at: None,
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
            search_open: false,
            list_sort: MessageSort::DateNewest,
            quick_filters: QuickFilterSet::default(),
            search_scope: SearchScope::ThisFolder,
            server_search_loading: false,
            server_search_query: String::new(),
            server_search_results: Vec::new(),
            collapsed_accounts: HashSet::new(),
            show_accounts: true,
            pending_empty_folder: None,
            reader_prefer_plain: false,
            allowed_remote_image_ids: HashSet::new(),
            collapsed_thread_cards: HashSet::new(),
            raw_source_message_ids: HashSet::new(),
            pending_row_action: None,
            pending_eml_export: None,
            pending_undo: None,
            last_move_undo: None,
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
            gallery_preview_pending: None,
            pending_unsubscribe: None,
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
                    let pending = self.pending_row_action.take();
                    self.set_selected_message(detail.clone());
                    self.error_banner = None;
                    // B-2：行菜单挂起的动作（回复 / 复制正文 / 查看原文）在正文就绪后执行。
                    if let Some((msg_id, action)) = pending {
                        if msg_id == detail.id {
                            let entity = cx.entity();
                            let _ = self.window_handle.update(cx, |_, window, cx| {
                                entity.update(cx, |app, cx| {
                                    app.finish_pending_row_action(&detail, window, cx);
                                });
                            });
                        } else {
                            self.pending_row_action = Some((msg_id, action));
                        }
                    }
                }
                WorkerEvent::RawMessageFetched { message_id, bytes } => {
                    // B-2：.eml 原始字节已取回，弹系统保存对话框写出「主题.eml」。
                    self.pending_eml_export = None;
                    let subject = self
                        .selected_message
                        .as_ref()
                        .filter(|d| d.id == message_id)
                        .map(|d| d.subject.clone())
                        .or_else(|| {
                            self.current_page
                                .items
                                .iter()
                                .find(|m| m.id == message_id)
                                .map(|m| m.subject.clone())
                        })
                        .unwrap_or_else(|| "邮件".to_string());
                    self.export_eml_bytes(&subject, bytes, cx);
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
                    // B-2：抓取原文失败时退出「导出中…」状态。
                    self.pending_eml_export = None;
                    self.error_banner = Some(SharedString::from(message.clone()));
                    self.status_line = SharedString::from(message);
                }
                WorkerEvent::MessageArchived {
                    account_id,
                    message_id,
                    dest_folder_path,
                    rfc_message_id,
                }
                | WorkerEvent::MessageDeleted {
                    account_id,
                    message_id,
                    dest_folder_path,
                    rfc_message_id,
                }
                | WorkerEvent::MessageMarkedSpam {
                    account_id,
                    message_id,
                    dest_folder_path,
                    rfc_message_id,
                } => {
                    // F-3：请求时已捕获撤销前半段，成功回执补全定位信息后转为可撤销条；
                    // 永久删除（无回收站）或缺少 Message-ID 时不提供撤销。
                    if let Some(pending) = self.pending_undo.take() {
                        if pending.account_id == account_id
                            && pending.message_id == message_id
                            && !dest_folder_path.is_empty()
                            && !rfc_message_id.is_empty()
                        {
                            let record = pending.into_record(dest_folder_path, rfc_message_id);
                            self.set_last_move_undo(record, cx);
                        }
                    }
                    // 后端回执：随事件先行到达的 FoldersLoaded 已刷新文件夹计数；
                    // 这里刷新当前列表并在阅读态显示该邮件时回到空态。
                    self.after_message_moved(account_id, message_id, cx);
                }
                WorkerEvent::MessageMoved {
                    account_id,
                    message_id,
                    target_folder_path,
                    rfc_message_id,
                } => {
                    if let Some(pending) = self.pending_undo.take() {
                        if pending.account_id == account_id
                            && pending.message_id == message_id
                            && !rfc_message_id.is_empty()
                        {
                            let record = pending.into_record(target_folder_path, rfc_message_id);
                            self.set_last_move_undo(record, cx);
                        }
                    }
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
                // F-4：草稿落库回执——绑定新分配的本地草稿 ID，自动保存时显示提示。
                WorkerEvent::DraftSaved { draft_id } => {
                    if self.right_mode == RightPaneMode::Compose && draft_id > 0 {
                        if self.compose_draft_id.is_none() {
                            self.compose_draft_id = Some(draft_id);
                            if self.compose_autosave_pending {
                                self.compose_autosaved_new = true;
                            }
                        }
                        if self.compose_autosave_pending {
                            let now = chrono::Local::now().format("%H:%M").to_string();
                            self.compose_autosaved_at = Some(SharedString::from(now));
                        }
                    }
                    self.compose_autosave_pending = false;
                    cx.notify();
                }
                // F-5：批量导入回执（`KnownContactsLoaded` 已先行刷新列表）。
                WorkerEvent::ContactsImported { added, skipped } => {
                    self.status_line = SharedString::from(format!(
                        "联系人导入完成：新增 {added} 个，跳过 {skipped} 个（重复或无效）"
                    ));
                    cx.notify();
                }
                // F-6：附件总览图片预览字节到达（仅等待中的附件生效）。
                WorkerEvent::AttachmentBytesFetched {
                    attachment_id,
                    filename,
                    mime_type,
                    bytes,
                } => {
                    if self.gallery_preview_pending == Some(attachment_id) {
                        self.gallery_preview_pending = None;
                        if let Some(ref g) = self.gallery {
                            g.update(cx, |state, cx| {
                                state.set_preview_image(&filename, &mime_type, bytes);
                                cx.notify();
                            });
                        }
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
    pub(crate) fn minimize_window_or_tray(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
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
            // F-7：切换会话时清除未确认的退订横幅。
            self.pending_unsubscribe = None;
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

    /// 归档 / 删除 / 移动 / 标垃圾完成后的公共收尾：清空正在显示的目标邮件并刷新当前列表；
    /// F-2：被移除邮件正显示在阅读态时，自动推进选中相邻下一封（末尾则上一封）并加载其正文。
    fn after_message_moved(&mut self, account_id: u32, message_id: u32, cx: &mut Context<Self>) {
        self.selected_message_ids.remove(&message_id);
        let was_displayed = self
            .selected_message
            .as_ref()
            .is_some_and(|m| m.account_id == account_id && m.id == message_id);
        // 先在旧列表中定位被移除邮件的下标（F-2），再刷新列表。
        let advance_to = if was_displayed {
            let len = self.current_page.items.len();
            self.current_page
                .items
                .iter()
                .position(|m| m.id == message_id)
                .and_then(|idx| next_index_after_removal(idx, len))
                .and_then(|next_idx| self.current_page.items.get(next_idx).cloned())
        } else {
            None
        };
        if was_displayed {
            self.selected_message = None;
        }
        // 先请求相邻邮件正文、后刷新列表：worker 按顺序处理，详情先回、页面后回，
        // 避免 merge_loaded_page 在 selected_message 仍为空时误选第一封。
        if let Some(next) = advance_to.as_ref() {
            self.select_message(
                next.account_id,
                next.id,
                next.uid,
                next.unread,
                &next.folder_path,
                cx,
            );
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
            self.pending_undo = self.capture_undo_record(account_id, message_id, "已归档");
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
            self.pending_undo = self.capture_undo_record(account_id, message_id, "已删除");
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
            self.pending_undo =
                self.capture_undo_record(account_id, message_id, "已标记为垃圾邮件");
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
            self.pending_undo = self.capture_undo_record(account_id, message_id, "已移动");
            self.status_line = SharedString::from("正在移动邮件...");
            w.send(MailRequest::MoveMessage {
                account_id,
                message_id,
                target_folder_path,
            });
        }
        cx.notify();
    }

    // ---- B-2 行菜单挂起动作 / F-3 撤销 / .eml 导出 ----

    /// F-3：请求发出前从当前列表捕获撤销前半段信息（原文件夹路径；定位信息由成功回执补全）。
    fn capture_undo_record(
        &self,
        account_id: u32,
        message_id: u32,
        action_label: &str,
    ) -> Option<PendingUndo> {
        let msg = self
            .current_page
            .items
            .iter()
            .find(|m| m.id == message_id && m.account_id == account_id)?;
        Some(PendingUndo {
            account_id,
            message_id: msg.id,
            original_folder_path: msg.folder_path.clone(),
            action_label: SharedString::from(action_label.to_string()),
            at: Instant::now(),
        })
    }

    /// F-3：写入可撤销记录并安排 15 秒后自动清理（新记录覆盖旧记录）。
    fn set_last_move_undo(&mut self, record: UndoRecord, cx: &mut Context<Self>) {
        let at = record.at;
        self.last_move_undo = Some(record);
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(UNDO_TTL_SECS))
                .await;
            let _ = this.update(cx, |app, cx| {
                if app.last_move_undo.as_ref().is_some_and(|r| r.at == at) {
                    app.last_move_undo = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// F-3：撤销最近一次移动 / 删除 / 归档——邮件已不在本地缓存，走专用
    /// `UndoMessageMove`（服务端按 Message-ID 定位后移回原文件夹），不再记录新的撤销。
    pub(crate) fn undo_last_move(&mut self, cx: &mut Context<Self>) {
        let Some(record) = self.last_move_undo.take() else {
            return;
        };
        if let Some(ref w) = self.worker {
            self.status_line =
                SharedString::from(format!("正在撤销{}，移回原文件夹...", record.action_label));
            w.send(MailRequest::UndoMessageMove {
                account_id: record.account_id,
                message_id: record.message_id,
                rfc_message_id: record.rfc_message_id,
                current_folder_path: record.current_folder_path,
                original_folder_path: record.original_folder_path,
            });
        }
        cx.notify();
    }

    /// B-2：列表行菜单的回复 / 复制正文 / 查看原文入口。
    ///
    /// 详情已就绪（demo 或当前选中）时立即执行；否则挂起动作并按需加载正文。
    pub(crate) fn begin_row_action(
        &mut self,
        action: PendingRowAction,
        account_id: u32,
        message_id: u32,
        uid: u32,
        unread: bool,
        folder_path: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.demo_mode {
            if let Some(detail) = self.demo_message_detail(message_id) {
                self.right_mode = RightPaneMode::Reader;
                // 查看原文 / 回复针对当前显示的邮件：先切换选中再执行动作。
                self.set_selected_message(detail.clone());
                self.run_row_action(action, &detail, window, cx);
            }
            cx.notify();
            return;
        }
        if self
            .selected_message
            .as_ref()
            .is_some_and(|d| d.id == message_id)
        {
            if let Some(detail) = self.selected_message.clone() {
                self.run_row_action(action, &detail, window, cx);
            }
            cx.notify();
            return;
        }
        self.right_mode = RightPaneMode::Reader;
        self.pending_row_action = Some((message_id, action));
        self.request_message_detail(account_id, message_id, uid, unread, folder_path);
        cx.notify();
    }

    /// 执行已就绪的行菜单动作（B-2）。
    fn run_row_action(
        &mut self,
        action: PendingRowAction,
        detail: &MessageDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            PendingRowAction::Reply => self.start_reply(detail, window, cx),
            PendingRowAction::ReplyAll => self.start_reply_all(detail, window, cx),
            PendingRowAction::Forward => self.start_forward(detail, window, cx),
            PendingRowAction::CopyBody => {
                let text = detail.body_text.clone();
                self.copy_text_to_clipboard(text, cx);
            }
            PendingRowAction::ViewSource => {
                self.raw_source_message_ids.insert(detail.id);
                self.collapsed_thread_cards.remove(&detail.id);
                cx.notify();
            }
        }
    }

    /// 正文加载完成后执行挂起的行菜单动作（MessageBodyLoaded 回调入口）。
    fn finish_pending_row_action(
        &mut self,
        detail: &MessageDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((msg_id, action)) = self.pending_row_action.take() else {
            return;
        };
        if msg_id != detail.id {
            // 回执与挂起动作不匹配（用户切换了目标）：放回等待对应正文。
            self.pending_row_action = Some((msg_id, action));
            return;
        }
        self.run_row_action(action, detail, window, cx);
    }

    /// B-2：请求导出 .eml——真实账号异步抓取原文（导出中…），演示模式直接合成。
    pub(crate) fn request_export_eml(
        &mut self,
        account_id: u32,
        message_id: u32,
        uid: u32,
        folder_path: String,
        cx: &mut Context<Self>,
    ) {
        if self.pending_eml_export == Some(message_id) {
            return;
        }
        if self.demo_mode {
            match self.demo_message_detail(message_id) {
                Some(detail) => {
                    let bytes = compose_demo_eml(&detail);
                    self.export_eml_bytes(&detail.subject, bytes, cx);
                }
                None => {
                    self.status_line = SharedString::from("演示数据没有该邮件的原始正文，无法导出");
                }
            }
            cx.notify();
            return;
        }
        self.pending_eml_export = Some(message_id);
        self.status_line = SharedString::from("正在获取邮件原文...");
        if let Some(ref w) = self.worker {
            w.send(MailRequest::FetchRawMessage {
                account_id,
                message_id,
                uid,
                folder_path,
            });
        }
        cx.notify();
    }

    /// B-2：弹系统保存对话框并把 .eml 字节写出（文件名 = 过滤非法字符后的主题）。
    fn export_eml_bytes(&mut self, subject: &str, bytes: Vec<u8>, cx: &mut Context<Self>) {
        let filename = format!("{}.eml", sanitize_filename(subject));
        if let Some(dest) = rfd::FileDialog::new().set_file_name(&filename).save_file() {
            match std::fs::write(&dest, &bytes) {
                Ok(()) => {
                    self.status_line =
                        SharedString::from(format!("已导出邮件原文至：{}", dest.display()));
                }
                Err(e) => {
                    self.error_banner = Some(SharedString::from(format!("保存邮件原文失败: {e}")));
                }
            }
        }
        cx.notify();
    }

    /// 查找账号的收件箱文件夹路径（B-2 Trash/Junk 恢复目标）。
    pub(crate) fn inbox_folder_path(&self, account_id: u32) -> Option<String> {
        self.folders_by_account
            .get(&account_id)?
            .iter()
            .find(|f| {
                f.path.eq_ignore_ascii_case("INBOX")
                    || f.name.eq_ignore_ascii_case("INBOX")
                    || f.name == "收件箱"
            })
            .map(|f| f.path.clone())
    }

    /// 判定指定文件夹是否为回收站 / 垃圾邮件文件夹（B-2 菜单语义切换）。
    pub(crate) fn folder_is_trash_or_junk(&self, account_id: u32, folder_id: u32) -> bool {
        self.folders_by_account
            .get(&account_id)
            .and_then(|list| list.iter().find(|f| f.id == folder_id))
            .is_some_and(|f| is_trash_or_junk_folder(&f.name, &f.path))
    }

    /// B-6：当前列表视图是否显示收件人（统一已发送 / 草稿视图 / 账号 Sent 文件夹）。
    pub(crate) fn list_view_shows_recipients(&self) -> bool {
        match self.current_view {
            CurrentView::Sent | CurrentView::Drafts => true,
            CurrentView::AccountFolder => self
                .folders_by_account
                .values()
                .flatten()
                .find(|f| f.id == self.selected_folder_id)
                .is_some_and(|f| is_sent_folder(&f.name, &f.path)),
            CurrentView::UnifiedInbox | CurrentView::Starred | CurrentView::Archive => false,
        }
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
            // B-6：草稿视图列表行显示「致：收件人」。
            recipients: draft.to_addr.clone(),
            thread_count: 1,
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
        self.begin_compose_autosave_tracking();
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

    // ---- F-4 写信草稿 5 秒防抖自动保存 ----

    /// 进入写信页（新写信 / 回复 / 转发 / 打开草稿）时重置自动保存跟踪。
    fn begin_compose_autosave_tracking(&mut self) {
        self.compose_autosave_generation += 1;
        self.compose_autosave_dirty = false;
        self.compose_autosave_pending = false;
        self.compose_autosaved_new = false;
        self.compose_autosaved_at = None;
    }

    /// 离开写信流程（发送 / 取消 / 存草稿退出）时清除挂起的自动保存。
    fn cancel_draft_autosave(&mut self) {
        self.compose_autosave_generation += 1;
        self.compose_autosave_dirty = false;
        self.compose_autosave_pending = false;
    }

    /// 内容变更后重置 5 秒防抖定时器：代际号 +1 使旧定时器醒来后失效，
    /// 5 秒内无新输入才真正执行静默保存（demo 模式无本地草稿库，跳过）。
    pub(crate) fn schedule_draft_autosave(&mut self, cx: &mut Context<Self>) {
        if self.demo_mode {
            return;
        }
        self.compose_autosave_dirty = true;
        self.compose_autosave_generation += 1;
        let generation = self.compose_autosave_generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(5)).await;
            let _ = this.update(cx, |app, cx| {
                if app.compose_autosave_generation == generation
                    && app.compose_autosave_dirty
                    && app.right_mode == RightPaneMode::Compose
                {
                    app.autosave_draft_now(cx);
                }
            });
        })
        .detach();
    }

    /// 立即执行一次静默草稿保存（`AutosaveLocalDraft`，不回发状态行）。
    fn autosave_draft_now(&mut self, cx: &mut Context<Self>) {
        self.compose_autosave_dirty = false;
        self.compose_autosave_pending = true;
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
            w.send(MailRequest::AutosaveLocalDraft { draft });
        }
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
        let body_with_sig = append_signature(&quoted, &self.account_signature(detail.account_id));

        self.compose_account_id = detail.account_id;
        self.compose_draft_id = None;
        self.compose_in_reply_to.clear();
        self.compose_attachments.clear();
        self.compose_confirm_discard = false;
        self.pending_forward_msg_id = Some(detail.id);
        self.compose_autocomplete_field = None;
        self.begin_compose_autosave_tracking();
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
        let body_with_sig = append_signature(&quoted, &self.account_signature(detail.account_id));

        self.compose_account_id = detail.account_id;
        self.compose_draft_id = None;
        self.compose_in_reply_to = detail.rfc_message_id.clone();
        self.compose_attachments.clear();
        self.compose_confirm_discard = false;
        self.pending_forward_msg_id = None;
        self.compose_autocomplete_field = None;
        self.begin_compose_autosave_tracking();
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
    /// F-4：若该草稿此前由自动保存静默创建，则连同删除（避免留下孤儿草稿）。
    pub(crate) fn discard_compose(&mut self, cx: &mut Context<Self>) {
        self.cancel_draft_autosave();
        if self.compose_autosaved_new {
            if let Some(draft_id) = self.compose_draft_id.take() {
                if let Some(ref w) = self.worker {
                    w.send(MailRequest::DeleteLocalDraft { draft_id });
                }
            }
            self.compose_autosaved_new = false;
            self.compose_autosaved_at = None;
        }
        self.compose_confirm_discard = false;
        self.pending_forward_msg_id = None;
        self.compose_autocomplete_field = None;
        self.right_mode = RightPaneMode::Reader;
        cx.notify();
    }

    /// 保存为草稿并退出写信页。
    pub(crate) fn save_draft_and_close_compose(&mut self, cx: &mut Context<Self>) {
        self.cancel_draft_autosave();
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
        self.begin_compose_autosave_tracking();
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
            // F-4：附件变化也属于写信内容变更，重置自动保存定时器。
            self.schedule_draft_autosave(cx);
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
    pub(crate) fn save_all_attachments(
        &mut self,
        attachment_ids: Vec<u32>,
        cx: &mut Context<Self>,
    ) {
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
        // F-4：发送即清除挂起的自动保存（草稿由发送流程删除，不再落库）。
        self.cancel_draft_autosave();
        self.compose_autosaved_new = false;
        self.compose_autosaved_at = None;
        self.pending_unsubscribe = None;
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
        self.begin_compose_autosave_tracking();
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
            CurrentView::UnifiedInbox => self.folders_by_account.values().flatten().any(|f| {
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

    // ---- 阶段二：主菜单补齐（对齐原版 boot.rs / window.rs 的菜单动作）----

    /// 主菜单「显示 / 隐藏账号列表」（Ctrl+Shift+A）：对齐原版
    /// `app.show-accounts`——隐藏或恢复侧栏的账号区段（统一导航保持不动，
    /// 不是隐藏整个侧栏，故不复用 toggle_sidebar）。
    pub(crate) fn toggle_accounts_list(&mut self, cx: &mut Context<Self>) {
        self.show_accounts = !self.show_accounts;
        self.status_line = SharedString::from(if self.show_accounts {
            "已显示账号列表"
        } else {
            "已隐藏账号列表"
        });
        cx.notify();
    }

    /// F11 全屏切换（对齐原版 `win.fullscreen`）。
    pub(crate) fn toggle_fullscreen(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
        window.toggle_fullscreen();
    }

    /// F1 / ? 快捷键速查弹窗（原版同键可再次按下关闭）。
    pub(crate) fn open_shortcuts(&mut self, cx: &mut Context<Self>) {
        if self.active_modal == ActiveModal::Shortcuts {
            self.active_modal = ActiveModal::None;
        } else {
            self.active_modal = ActiveModal::Shortcuts;
        }
        cx.notify();
    }

    /// 「关于 Hylki」弹窗（对齐原版 win.about）。
    pub(crate) fn open_about(&mut self, cx: &mut Context<Self>) {
        self.active_modal = ActiveModal::About;
        cx.notify();
    }

    // ---- 阶段二：文件夹右键动作（对齐原版 sidebar/menus.rs 的 folder_menu_items）----

    /// 右键「刷新此文件夹」：复用现有 `MailRequest::SyncFolder`（demo 模式禁用）。
    pub(crate) fn refresh_folder(
        &mut self,
        account_id: u32,
        folder_id: u32,
        folder_path: String,
        cx: &mut Context<Self>,
    ) {
        if self.demo_mode {
            return;
        }
        if let Some(ref w) = self.worker {
            self.status_line = SharedString::from("正在刷新文件夹...");
            w.send(MailRequest::SyncFolder {
                account_id,
                folder_id,
                folder_path,
            });
        }
        cx.notify();
    }

    /// 右键「全部标为已读」：对一个账号文件夹批量发起 `MarkFolderAllRead`
    /// 并乐观清除当前列表中该文件夹的未读标记（demo 模式禁用）。
    pub(crate) fn mark_folder_all_read(
        &mut self,
        account_id: u32,
        folder_id: u32,
        folder_path: String,
        cx: &mut Context<Self>,
    ) {
        if self.demo_mode {
            return;
        }
        let current_folder =
            self.current_view == CurrentView::AccountFolder && self.selected_folder_id == folder_id;
        if current_folder || self.current_view == CurrentView::UnifiedInbox {
            for item in self
                .current_page
                .items
                .iter_mut()
                .filter(|m| m.folder_id == folder_id)
            {
                item.unread = false;
            }
        }
        if let Some(ref w) = self.worker {
            self.status_line = SharedString::from("正在把文件夹全部标为已读...");
            w.send(MailRequest::MarkFolderAllRead {
                account_id,
                folder_id,
                folder_path,
            });
        }
        cx.notify();
    }

    /// 统一收件箱行右键「全部标为已读」：对所有账号的收件箱批量发起，
    /// 并乐观清除当前统一收件箱页面的未读标记（demo 模式禁用）。
    pub(crate) fn mark_all_inboxes_read(&mut self, cx: &mut Context<Self>) {
        if self.demo_mode {
            return;
        }
        let targets: Vec<(u32, u32, String)> = self
            .folders_by_account
            .values()
            .flatten()
            .filter(|f| f.path.eq_ignore_ascii_case("INBOX") || f.name.contains("收件箱"))
            .map(|f| (f.account_id, f.id, f.path.clone()))
            .collect();
        if targets.is_empty() {
            return;
        }
        if let Some(ref w) = self.worker {
            for (account_id, folder_id, folder_path) in &targets {
                w.send(MailRequest::MarkFolderAllRead {
                    account_id: *account_id,
                    folder_id: *folder_id,
                    folder_path: folder_path.clone(),
                });
            }
        }
        if self.current_view == CurrentView::UnifiedInbox {
            let inbox_folder_ids: HashSet<u32> = targets.iter().map(|(_, id, _)| *id).collect();
            for item in self.current_page.items.iter_mut() {
                if inbox_folder_ids.contains(&item.folder_id) {
                    item.unread = false;
                }
            }
        }
        self.status_line = SharedString::from(format!(
            "正在把 {} 个账号的收件箱全部标为已读...",
            targets.len()
        ));
        cx.notify();
    }

    /// 右键 Trash / Junk「清空文件夹…」：进入二次确认（对齐原版
    /// Empty Trash… 的确认交互；Graph 账号已在菜单禁用）。
    pub(crate) fn request_empty_folder(
        &mut self,
        account_id: u32,
        folder_id: u32,
        folder_path: String,
        folder_name: String,
        cx: &mut Context<Self>,
    ) {
        self.pending_empty_folder = Some((account_id, folder_id, folder_path, folder_name));
        cx.notify();
    }

    /// 取消「清空文件夹」确认。
    pub(crate) fn cancel_empty_folder(&mut self, cx: &mut Context<Self>) {
        self.pending_empty_folder = None;
        cx.notify();
    }

    /// 确认清空：发 `MailRequest::EmptyFolder` 并乐观清空当前列表中该文件夹的条目。
    pub(crate) fn confirm_empty_folder(&mut self, cx: &mut Context<Self>) {
        let Some((account_id, folder_id, folder_path, _)) = self.pending_empty_folder.take() else {
            return;
        };
        if self.current_view == CurrentView::AccountFolder && self.selected_folder_id == folder_id {
            self.current_page.items.clear();
            self.current_page.total = 0;
            self.selected_message = None;
        }
        if let Some(ref w) = self.worker {
            self.status_line = SharedString::from("正在清空文件夹...");
            w.send(MailRequest::EmptyFolder {
                account_id,
                folder_id,
                folder_path,
            });
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
                    // F-6：图片附件预览——读取本地缓存的附件字节后回填浮层。
                    GalleryEvent::PreviewImage(attachment_id) => {
                        this.fetch_gallery_preview(*attachment_id, cx);
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

    // ---- F-6 附件总览图片预览 ----

    /// 网格 / 列表中的图片附件点击预览：请求读取本地缓存的附件字节。
    /// demo 模式无真实字节，直接以占位浮层展示。
    pub(crate) fn fetch_gallery_preview(&mut self, attachment_id: u32, cx: &mut Context<Self>) {
        self.gallery_preview_pending = Some(attachment_id);
        if self.demo_mode {
            if let Some(ref g) = self.gallery {
                g.update(cx, |state, cx| {
                    state.set_preview_unavailable();
                    cx.notify();
                });
            }
            cx.notify();
            return;
        }
        if let Some(ref w) = self.worker {
            w.send(MailRequest::FetchAttachmentBytes { attachment_id });
        }
        cx.notify();
    }

    // ---- F-7 一键退订 ----

    /// 退订横幅「退订」按钮：进入卡内二次确认（对齐原版 confirm 流程）。
    pub(crate) fn request_unsubscribe(&mut self, message_id: u32, cx: &mut Context<Self>) {
        self.pending_unsubscribe = Some(message_id);
        cx.notify();
    }

    /// 退订确认条「取消」。
    pub(crate) fn cancel_unsubscribe(&mut self, cx: &mut Context<Self>) {
        self.pending_unsubscribe = None;
        cx.notify();
    }

    /// 确认退订：one-click 句柄直接 POST（原版行为）；https 打开浏览器；
    /// mailto 交给系统处理（gpui open_url 走系统 shell，可唤起默认邮件客户端）。
    pub(crate) fn confirm_unsubscribe(&mut self, _message_id: u32, cx: &mut Context<Self>) {
        self.pending_unsubscribe = None;
        let Some(detail) = self.selected_message.as_ref() else {
            return;
        };
        let Some(ref url) = detail.unsubscribe_url else {
            return;
        };
        let lower = url.to_ascii_lowercase();
        if detail.unsubscribe_one_click && lower.starts_with("https://") {
            if self.demo_mode {
                self.status_line =
                    SharedString::from("演示模式：已模拟发送一键退订请求（不访问网络）");
                cx.notify();
                return;
            }
            if let Some(ref w) = self.worker {
                self.status_line = SharedString::from("正在发送一键退订请求...");
                w.send(MailRequest::UnsubscribeOneClick { url: url.clone() });
            }
        } else if lower.starts_with("http://") || lower.starts_with("https://") {
            self.status_line = SharedString::from("已在浏览器打开退订页面");
            cx.open_url(url);
        } else if lower.starts_with("mailto:") {
            // gpui open_url 经系统 shell 打开，mailto 唤起默认邮件客户端。
            self.status_line = SharedString::from("已调用系统邮件客户端发送退订邮件");
            cx.open_url(url);
        } else {
            self.error_banner = Some(SharedString::from(
                "该退订地址不受支持（仅支持 http/https/mailto）",
            ));
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
                    ContactsEvent::SaveContact {
                        id,
                        email,
                        display_name,
                        notes,
                    } => {
                        this.save_user_contact(
                            *id,
                            email.clone(),
                            display_name.clone(),
                            notes.clone(),
                            cx,
                        );
                    }
                    ContactsEvent::DeleteContact(contact_id) => {
                        this.delete_user_contact(*contact_id, cx);
                    }
                    ContactsEvent::ImportContacts => {
                        this.import_contacts_file(cx);
                    }
                    ContactsEvent::ExportContacts { kind, items } => {
                        this.export_contacts(*kind, items.clone(), cx);
                    }
                },
            );
            self.contacts = Some(entity);
            self.contacts_sub = Some(sub);
        }
        if let Some(ref c) = self.contacts {
            c.update(cx, |state, _cx| state.set_demo_mode(self.demo_mode));
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

    // ---- F-5 用户联系人 CRUD 与导入 / 导出 ----

    /// 保存用户联系人（新建或编辑；去重校验已在联系人页完成）。
    fn save_user_contact(
        &mut self,
        id: Option<u32>,
        email: String,
        display_name: String,
        notes: String,
        cx: &mut Context<Self>,
    ) {
        if self.demo_mode {
            self.status_line = SharedString::from("演示模式下不支持修改联系人数据库");
            cx.notify();
            return;
        }
        if let Some(ref w) = self.worker {
            w.send(MailRequest::SaveUserContact {
                contact: UserContact {
                    id: id.unwrap_or(0),
                    email,
                    display_name,
                    notes,
                },
            });
        }
        cx.notify();
    }

    /// 删除用户联系人（仅自定义联系人可删，`contact_id` 为行 ID）。
    fn delete_user_contact(&mut self, contact_id: u32, cx: &mut Context<Self>) {
        if self.demo_mode {
            self.status_line = SharedString::from("演示模式下不支持删除联系人");
            cx.notify();
            return;
        }
        if let Some(ref w) = self.worker {
            w.send(MailRequest::DeleteUserContact { contact_id });
        }
        cx.notify();
    }

    /// 联系人导入：rfd 选择 .vcf / .csv 文件，按内容特征识别格式解析后批量入库。
    fn import_contacts_file(&mut self, cx: &mut Context<Self>) {
        if self.demo_mode {
            self.status_line = SharedString::from("演示模式下不支持导入联系人");
            cx.notify();
            return;
        }
        let Some(path) = rfd::FileDialog::new()
            .add_filter("联系人文件 (vCard / CSV)", &["vcf", "csv"])
            .pick_file()
        else {
            return;
        };
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) => {
                self.error_banner = Some(SharedString::from(format!("读取联系人文件失败: {e}")));
                cx.notify();
                return;
            }
        };
        // 按扩展名或内容特征（BEGIN:VCARD）识别格式；解析失败的行被跳过。
        let contacts = if contacts_io::looks_like_vcard(&path, &text) {
            contacts_io::parse_vcards(&text)
        } else {
            contacts_io::parse_csv_contacts(&text)
        };
        if contacts.is_empty() {
            self.status_line =
                SharedString::from("未在文件中找到带邮箱的联系人（支持 vCard 与 CSV）");
            cx.notify();
            return;
        }
        let count = contacts.len();
        if let Some(ref w) = self.worker {
            self.status_line = SharedString::from(format!("正在导入 {count} 个联系人..."));
            w.send(MailRequest::ImportUserContacts { contacts });
        }
        cx.notify();
    }

    /// 联系人导出：把联系人页当前列表写入 vCard / CSV 文件（demo 模式同样可用）。
    fn export_contacts(
        &mut self,
        kind: ContactExportKind,
        items: Vec<KnownContact>,
        cx: &mut Context<Self>,
    ) {
        let (title, filter, default_name, content) = match kind {
            ContactExportKind::Vcard => (
                "导出 vCard 联系人",
                ("vCard 文件", &["vcf"][..]),
                "contacts.vcf",
                contacts_io::export_vcards(
                    &items
                        .iter()
                        .map(|c| UserContact {
                            id: c.id,
                            email: c.email.clone(),
                            display_name: c.name.clone(),
                            notes: c.notes.clone(),
                        })
                        .collect::<Vec<_>>(),
                ),
            ),
            ContactExportKind::Csv => (
                "导出 CSV 联系人",
                ("CSV 文件", &["csv"][..]),
                "contacts.csv",
                contacts_io::export_csv(
                    &items
                        .iter()
                        .map(|c| UserContact {
                            id: c.id,
                            email: c.email.clone(),
                            display_name: c.name.clone(),
                            notes: c.notes.clone(),
                        })
                        .collect::<Vec<_>>(),
                ),
            ),
        };
        if items.is_empty() {
            self.status_line = SharedString::from("暂无可导出的联系人");
            cx.notify();
            return;
        }
        let Some(dest) = rfd::FileDialog::new()
            .set_title(title)
            .add_filter(filter.0, filter.1)
            .set_file_name(default_name)
            .save_file()
        else {
            return;
        };
        match std::fs::write(&dest, content) {
            Ok(()) => {
                self.status_line = SharedString::from(format!(
                    "已导出 {} 个联系人至 {}",
                    items.len(),
                    dest.display()
                ));
            }
            Err(e) => {
                self.error_banner = Some(SharedString::from(format!("写入联系人文件失败: {e}")));
            }
        }
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

    /// 打开命令面板浮层（侧栏菜单 / Ctrl+K）；`actions_only` 为 true 时
    /// 预填 `>` 仅筛选操作命令（Ctrl+Shift+K，对齐原版「actions only」）。
    pub(crate) fn open_command_palette(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        actions_only: bool,
    ) {
        self.active_modal = ActiveModal::CommandPalette;
        if self.palette.is_none() {
            let commands = self.palette_commands();
            let entity = cx.new(|cx| CommandPaletteState::new(window, cx, commands, actions_only));
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

    /// 固定命令 + 选中邮件操作 + 统一视图 / 联系人 / 附件入口 +
    /// 遍历 folders_by_account 生成的文件夹跳转命令。
    fn palette_commands(&self) -> Vec<PaletteCommand> {
        let mut commands = vec![
            PaletteCommand::NewCompose,
            PaletteCommand::RefreshAll,
            PaletteCommand::ToggleUnifiedInbox,
            PaletteCommand::OpenAccountSettings,
            PaletteCommand::OpenPreferences,
            PaletteCommand::ToggleTheme,
        ];
        // 对当前选中邮件的操作：无选中时不显示（对齐原版 acts_on_selection）；
        // 归档 / 删除仅真实账号可用（demo 模式下右键菜单同样隐藏）。
        if self.selected_message.is_some() {
            commands.push(PaletteCommand::ReplySelected);
            commands.push(PaletteCommand::MarkUnreadSelected);
            commands.push(PaletteCommand::ToggleStarSelected);
            if !self.demo_mode {
                commands.push(PaletteCommand::ArchiveSelected);
                commands.push(PaletteCommand::DeleteSelected);
            }
        }
        // 统一视图与联系人 / 附件入口（原版命令面板的 go-to 分组）。
        commands.push(PaletteCommand::GoStarred);
        commands.push(PaletteCommand::GoSent);
        commands.push(PaletteCommand::GoDrafts);
        commands.push(PaletteCommand::GoArchive);
        commands.push(PaletteCommand::OpenContacts);
        commands.push(PaletteCommand::OpenGallery);
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
            PaletteCommand::GoStarred => {
                self.main_view = MainContentView::Mail;
                self.select_unified_view(CurrentView::Starred, cx);
            }
            PaletteCommand::GoSent => {
                self.main_view = MainContentView::Mail;
                self.select_unified_view(CurrentView::Sent, cx);
            }
            PaletteCommand::GoDrafts => {
                self.main_view = MainContentView::Mail;
                self.select_unified_view(CurrentView::Drafts, cx);
            }
            PaletteCommand::GoArchive => {
                self.main_view = MainContentView::Mail;
                self.select_unified_view(CurrentView::Archive, cx);
            }
            PaletteCommand::OpenContacts => {
                self.close_modal(cx);
                self.open_contacts(window, cx);
                return;
            }
            PaletteCommand::OpenGallery => {
                self.close_modal(cx);
                self.open_gallery(window, cx);
                return;
            }
            PaletteCommand::ReplySelected => {
                let Some(detail) = self.selected_message.clone() else {
                    return;
                };
                self.close_modal(cx);
                self.main_view = MainContentView::Mail;
                self.start_reply(&detail, window, cx);
                return;
            }
            PaletteCommand::ArchiveSelected => {
                self.archive_selected(cx);
            }
            PaletteCommand::DeleteSelected => {
                self.delete_selected(cx);
            }
            PaletteCommand::MarkUnreadSelected => {
                let Some(detail) = self.selected_message.clone() else {
                    return;
                };
                self.set_row_seen(
                    detail.account_id,
                    detail.id,
                    detail.uid,
                    detail.folder_path.clone(),
                    false,
                    cx,
                );
            }
            PaletteCommand::ToggleStarSelected => {
                let Some(detail) = self.selected_message.clone() else {
                    return;
                };
                self.toggle_flagged(&detail, cx);
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

    /// 刷新全部（侧栏刷新按钮 / 命令面板 / F5 共用；阶段三起 Ctrl+R 改为回复）。
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

    /// F-1：会话内步进下一张 (`delta = 1`) / 上一张 (`delta = -1`) 卡片（单键 w / b）。
    ///
    /// 复用现有选择路径：把选中锚点切到目标卡片并加载其正文——`set_selected_message`
    /// 的同会话分支会自动展开目标折叠卡片，阅读器的 is_current 高亮随锚点移动。
    /// 不做程序化滚动：阅读器滚动容器没有逐卡片锚点（如需可后续加 ScrollHandle）。
    /// 单卡 / 空会话时无邻卡，no-op。
    fn step_thread_card(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(detail) = self.selected_message.clone() else {
            return;
        };
        let cards = &detail.thread_messages;
        let current_idx = cards.iter().position(|tm| tm.id == detail.id);
        let Some(next_idx) = next_adjacent_index(current_idx, cards.len(), delta) else {
            return;
        };
        let card = &cards[next_idx];
        // 卡片未读态优先取当前列表行；不在列表中则不触发已读标记（传 false）。
        let unread = self
            .current_page
            .items
            .iter()
            .find(|m| m.id == card.id)
            .map(|m| m.unread)
            .unwrap_or(false);
        let uid = if card.uid > 0 { card.uid } else { detail.uid };
        let folder_path = if card.folder_path.is_empty() {
            detail.folder_path.clone()
        } else {
            card.folder_path.clone()
        };
        self.select_message(detail.account_id, card.id, uid, unread, &folder_path, cx);
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
                let win_controls =
                    window_controls::render_window_controls("compose-wc", window, cx);
                // 写信页唯一标题栏（B-8：合并进 48px 顶栏，compose.rs 不再画第二层标题）：
                // 左侧「取消 / 存草稿」，中间拖拽区，右侧「附件 / 发送」，复用现有动作。
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .bg(p.canvas)
                    .child(
                        h_flex()
                            .h(px(window_controls::HEADER_HEIGHT))
                            .flex_shrink_0()
                            .px_2()
                            .gap_1()
                            .items_center()
                            .border_b_1()
                            .border_color(p.border)
                            .child(
                                Button::new("compose-bar-cancel")
                                    .ghost()
                                    .compact()
                                    .label("取消")
                                    .tooltip("放弃当前邮件（Esc）")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.request_close_compose(cx);
                                    })),
                            )
                            .child(
                                Button::new("compose-bar-save-draft")
                                    .ghost()
                                    .compact()
                                    .label("存草稿")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.save_local_draft(cx);
                                    })),
                            )
                            .child(window_controls::drag_spacer())
                            .child(
                                Button::new("compose-bar-attach")
                                    .ghost()
                                    .compact()
                                    .icon(IconName::Paperclip)
                                    .label("附件")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.pick_attachments(cx);
                                    })),
                            )
                            .child(
                                Button::new("compose-bar-send")
                                    .primary()
                                    .compact()
                                    .label("发送")
                                    .tooltip("发送（Ctrl+Enter）")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.send_compose(cx);
                                    })),
                            )
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
        // F-3：撤销条过期（15 秒）时顺手清理（定时器负责主动通知）。
        if self
            .last_move_undo
            .as_ref()
            .is_some_and(|r| r.at.elapsed() > Duration::from_secs(UNDO_TTL_SECS))
        {
            self.last_move_undo = None;
        }

        let sidebar_el = if self.sidebar_hidden {
            // 折叠为 80px 图标窄轨（对齐原版 rail），不再整体移除左栏。
            Some(sidebar::render_collapsed(self, cx))
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
            // 「清空文件夹」二次确认条（右键 Trash / Junk 触发，Esc 取消）。
            .when_some(self.pending_empty_folder.as_ref(), |el, pending| {
                let (_, _, _, name) = pending.clone();
                el.child(
                    h_flex()
                        .px_3()
                        .py_1p5()
                        .gap_2()
                        .border_b_1()
                        .border_color(border)
                        .bg(p.danger.opacity(0.12))
                        .justify_between()
                        .items_center()
                        .child(
                            div()
                                .text_xs()
                                .font_weight(gpui_kit::FontWeight::MEDIUM)
                                .text_color(p.danger)
                                .truncate()
                                .child(SharedString::from(format!(
                                    "确认清空文件夹「{name}」？其中全部邮件将被永久删除，此操作不可撤销。"
                                ))),
                        )
                        .child(
                            h_flex()
                                .gap_1p5()
                                .flex_shrink_0()
                                .items_center()
                                .child(
                                    Button::new("empty-folder-confirm")
                                        .primary()
                                        .compact()
                                        .label("确认清空")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.confirm_empty_folder(cx);
                                        })),
                                )
                                .child(
                                    Button::new("empty-folder-cancel")
                                        .outline()
                                        .compact()
                                        .label("取消")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.cancel_empty_folder(cx);
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
                                .flex_1()
                                .min_w_0()
                                .text_xs()
                                .text_color(muted_fg)
                                .truncate()
                                .child(self.status_line.clone()),
                        )
                        // F-3：右侧轻量撤销条「已删除 / 已归档 / 已移动 — 撤销」。
                        .when_some(self.last_move_undo.clone(), |el, record| {
                            el.child(
                                h_flex()
                                    .flex_shrink_0()
                                    .gap_1()
                                    .items_center()
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(p.text)
                                            .child(record.action_label.clone()),
                                    )
                                    .child(
                                        Button::new("undo-last-move")
                                            .ghost()
                                            .compact()
                                            .text_size(px(11.0))
                                            .label("撤销")
                                            .tooltip("撤销最近一次移动 / 删除 / 归档（Ctrl+Z）")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.undo_last_move(cx);
                                            })),
                                    ),
                            )
                        }),
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
                    this.open_command_palette(window, cx, false);
                }
            }))
            // Ctrl+Shift+K：命令面板预填 `>`，仅筛选操作命令。
            .on_action(cx.listener(|this, _: &OpenCommandPaletteActions, window, cx| {
                if this.active_modal == ActiveModal::None {
                    this.open_command_palette(window, cx, true);
                }
            }))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                // 仅三栏视图且无浮层时展开并聚焦中栏搜索栏（Ctrl+F / `/`）。
                if this.main_view == MainContentView::Mail && this.active_modal == ActiveModal::None
                {
                    this.search_open = true;
                    this.search_input.update(cx, |state, cx| {
                        state.focus(window, cx);
                    });
                    cx.notify();
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
            // F11 全屏切换（不设浮层守卫：全屏与浮层互不干扰）。
            .on_action(cx.listener(|this, _: &ToggleFullscreen, window, cx| {
                this.toggle_fullscreen(window, cx);
            }))
            // Ctrl+Shift+A 显示 / 隐藏账号列表（对齐原版 show-accounts）。
            .on_action(cx.listener(|this, _: &ToggleAccountsList, _window, cx| {
                this.toggle_accounts_list(cx);
            }))
            // F1 / ? 快捷键速查：无浮层或已在速查浮层时响应（同键再按关闭）。
            .on_action(cx.listener(|this, _: &OpenShortcutsWindow, _window, cx| {
                if this.active_modal == ActiveModal::None
                    || this.active_modal == ActiveModal::Shortcuts
                {
                    this.open_shortcuts(cx);
                }
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
            // F-3：Ctrl+Z 撤销最近一次移动 / 删除 / 归档（条件与 DeleteSelectedMessage 一致）。
            .on_action(cx.listener(|this, _: &UndoLastMove, window, cx| {
                if this.active_modal == ActiveModal::None
                    && this.main_view == MainContentView::Mail
                    && this.right_mode == RightPaneMode::Reader
                    && !this.is_text_input_focused(window, cx)
                {
                    this.undo_last_move(cx);
                }
            }))
            // 阅读器缩放快捷键（Ctrl+= / Ctrl+- / Ctrl+0，复用现有持久化路径）。
            .on_action(cx.listener(|this, _: &ZoomIn, window, cx| {
                if this.active_modal == ActiveModal::None
                    && this.main_view == MainContentView::Mail
                    && this.right_mode == RightPaneMode::Reader
                    && !this.is_text_input_focused(window, cx)
                {
                    this.adjust_reader_zoom(10, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ZoomOut, window, cx| {
                if this.active_modal == ActiveModal::None
                    && this.main_view == MainContentView::Mail
                    && this.right_mode == RightPaneMode::Reader
                    && !this.is_text_input_focused(window, cx)
                {
                    this.adjust_reader_zoom(-10, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ZoomReset, window, cx| {
                if this.active_modal == ActiveModal::None
                    && this.main_view == MainContentView::Mail
                    && this.right_mode == RightPaneMode::Reader
                    && !this.is_text_input_focused(window, cx)
                {
                    this.set_reader_zoom(100, cx);
                }
            }))
            // ---- F-1 阶段三：组合键动作（始终生效，不受单键偏好控制）----
            // Ctrl+R 回复当前邮件（原 RefreshAllAccounts 语义移除，刷新仅保留 F5）。
            .on_action(cx.listener(|this, _: &ReplyCurrent, window, cx| {
                if this.active_modal == ActiveModal::None
                    && this.main_view == MainContentView::Mail
                    && this.right_mode == RightPaneMode::Reader
                    && !this.is_text_input_focused(window, cx)
                {
                    if let Some(detail) = this.selected_message.clone() {
                        this.start_reply(&detail, window, cx);
                    }
                }
            }))
            // Ctrl+Shift+R 回复全部。
            .on_action(cx.listener(|this, _: &ReplyAllCurrent, window, cx| {
                if this.active_modal == ActiveModal::None
                    && this.main_view == MainContentView::Mail
                    && this.right_mode == RightPaneMode::Reader
                    && !this.is_text_input_focused(window, cx)
                {
                    if let Some(detail) = this.selected_message.clone() {
                        this.start_reply_all(&detail, window, cx);
                    }
                }
            }))
            // Ctrl+U 查看原文（再按返回正文，复用 toggle_raw_source 的切换语义）。
            .on_action(cx.listener(|this, _: &ViewMessageSource, window, cx| {
                if this.active_modal == ActiveModal::None
                    && this.main_view == MainContentView::Mail
                    && this.right_mode == RightPaneMode::Reader
                    && !this.is_text_input_focused(window, cx)
                {
                    if let Some(detail) = this.selected_message.clone() {
                        this.toggle_raw_source(detail.id, cx);
                    }
                }
            }))
            // ---- F-1 阶段三：单键动作（single_key_guard_allows 含偏好开关检查）----
            .on_action(cx.listener(|this, _: &SingleKeyNextMessage, window, cx| {
                if single_key_guard_allows(
                    this.active_modal,
                    this.main_view,
                    this.right_mode,
                    this.is_text_input_focused(window, cx),
                    this.prefs.single_key_shortcuts,
                ) {
                    this.select_adjacent_message(1, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SingleKeyPrevMessage, window, cx| {
                if single_key_guard_allows(
                    this.active_modal,
                    this.main_view,
                    this.right_mode,
                    this.is_text_input_focused(window, cx),
                    this.prefs.single_key_shortcuts,
                ) {
                    this.select_adjacent_message(-1, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SingleKeyReply, window, cx| {
                if single_key_guard_allows(
                    this.active_modal,
                    this.main_view,
                    this.right_mode,
                    this.is_text_input_focused(window, cx),
                    this.prefs.single_key_shortcuts,
                ) {
                    if let Some(detail) = this.selected_message.clone() {
                        this.start_reply(&detail, window, cx);
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &SingleKeyReplyAll, window, cx| {
                if single_key_guard_allows(
                    this.active_modal,
                    this.main_view,
                    this.right_mode,
                    this.is_text_input_focused(window, cx),
                    this.prefs.single_key_shortcuts,
                ) {
                    if let Some(detail) = this.selected_message.clone() {
                        this.start_reply_all(&detail, window, cx);
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &SingleKeyForward, window, cx| {
                if single_key_guard_allows(
                    this.active_modal,
                    this.main_view,
                    this.right_mode,
                    this.is_text_input_focused(window, cx),
                    this.prefs.single_key_shortcuts,
                ) {
                    if let Some(detail) = this.selected_message.clone() {
                        this.start_forward(&detail, window, cx);
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &SingleKeyArchive, window, cx| {
                if single_key_guard_allows(
                    this.active_modal,
                    this.main_view,
                    this.right_mode,
                    this.is_text_input_focused(window, cx),
                    this.prefs.single_key_shortcuts,
                ) {
                    this.archive_selected(cx);
                }
            }))
            // d / Backspace 删除：多选时走批量删除（与 DeleteSelectedMessage 一致）。
            .on_action(cx.listener(|this, _: &SingleKeyDelete, window, cx| {
                if single_key_guard_allows(
                    this.active_modal,
                    this.main_view,
                    this.right_mode,
                    this.is_text_input_focused(window, cx),
                    this.prefs.single_key_shortcuts,
                ) {
                    if this.selected_message_ids.len() > 1 {
                        this.batch_delete(cx);
                    } else {
                        this.delete_selected(cx);
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &SingleKeyMarkSpam, window, cx| {
                if single_key_guard_allows(
                    this.active_modal,
                    this.main_view,
                    this.right_mode,
                    this.is_text_input_focused(window, cx),
                    this.prefs.single_key_shortcuts,
                ) {
                    this.mark_spam_selected(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SingleKeyToggleStar, window, cx| {
                if single_key_guard_allows(
                    this.active_modal,
                    this.main_view,
                    this.right_mode,
                    this.is_text_input_focused(window, cx),
                    this.prefs.single_key_shortcuts,
                ) {
                    if let Some(detail) = this.selected_message.clone() {
                        this.toggle_flagged(&detail, cx);
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &SingleKeyToggleRead, window, cx| {
                if single_key_guard_allows(
                    this.active_modal,
                    this.main_view,
                    this.right_mode,
                    this.is_text_input_focused(window, cx),
                    this.prefs.single_key_shortcuts,
                ) {
                    this.toggle_message_seen(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SingleKeyCompose, window, cx| {
                if single_key_guard_allows(
                    this.active_modal,
                    this.main_view,
                    this.right_mode,
                    this.is_text_input_focused(window, cx),
                    this.prefs.single_key_shortcuts,
                ) {
                    this.start_new_compose(window, cx);
                }
            }))
            // u 返回列表：清空选中锚点，阅读态回到「选择一封邮件查看正文」空态。
            .on_action(cx.listener(|this, _: &SingleKeyBackToList, window, cx| {
                if single_key_guard_allows(
                    this.active_modal,
                    this.main_view,
                    this.right_mode,
                    this.is_text_input_focused(window, cx),
                    this.prefs.single_key_shortcuts,
                ) {
                    if this.selected_message.take().is_some() {
                        cx.notify();
                    }
                }
            }))
            // w / b 会话内下一张 / 上一张卡片。
            .on_action(cx.listener(|this, _: &SingleKeyNextCard, window, cx| {
                if single_key_guard_allows(
                    this.active_modal,
                    this.main_view,
                    this.right_mode,
                    this.is_text_input_focused(window, cx),
                    this.prefs.single_key_shortcuts,
                ) {
                    this.step_thread_card(1, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SingleKeyPrevCard, window, cx| {
                if single_key_guard_allows(
                    this.active_modal,
                    this.main_view,
                    this.right_mode,
                    this.is_text_input_focused(window, cx),
                    this.prefs.single_key_shortcuts,
                ) {
                    this.step_thread_card(-1, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &CloseOverlay, window, cx| {
                // Esc 优先级：关闭浮层 > 清空文件夹确认 > 写信退出确认 > 撤销发送 > 取消多选。
                if this.active_modal != ActiveModal::None {
                    this.close_modal(cx);
                } else if this.pending_empty_folder.is_some() {
                    this.cancel_empty_folder(cx);
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
            ActiveModal::Shortcuts => {
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
                                    .w(px(600.0))
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
                                    .child(shortcuts_modal(&p, cx)),
                            ),
                        ),
                );
            }
            ActiveModal::About => {
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
                                    .w(px(440.0))
                                    .h(px(540.0))
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
                                    .child(about_modal(&p, border, cx)),
                            ),
                        ),
                );
            }
            ActiveModal::None => {}
        }
        root
    }
}

/// 快捷键速查分组数据（键位 + 说明）。F-1 阶段三起与 main.rs bind_keys /
/// on_action 实际生效的绑定逐条对应；未实现的键（l / h / x 等）不列出，
/// 避免「写了但不生效」的误导。
const SHORTCUT_GROUPS: &[(&str, &[(&str, &str)])] = &[
    (
        "移动",
        &[
            ("下一条邮件", "j / ↓"),
            ("上一条邮件", "k / ↑"),
            ("返回邮件列表", "u"),
            ("会话内下一张卡片", "w"),
            ("会话内上一张卡片", "b"),
            ("搜索", "/"),
            ("写邮件", "c"),
            ("本清单", "?"),
            ("删除选中邮件", "Delete"),
        ],
    ),
    (
        "对邮件的操作",
        &[
            ("回复", "r"),
            ("回复全部", "R"),
            ("转发", "f"),
            ("归档", "a"),
            ("删除", "d / Backspace / Delete"),
            ("标记为垃圾邮件", "!"),
            ("加星标 / 取消星标", "s"),
            ("已读 / 未读切换", "m"),
        ],
    ),
    (
        "全局快捷键（始终可用）",
        &[
            ("写新邮件", "Ctrl+N"),
            ("命令面板", "Ctrl+K"),
            ("命令面板（仅操作）", "Ctrl+Shift+K"),
            ("回复当前邮件", "Ctrl+R"),
            ("回复全部", "Ctrl+Shift+R"),
            ("查看邮件原文", "Ctrl+U"),
            ("检查新邮件", "F5"),
            ("搜索邮件列表", "Ctrl+F"),
            ("显示 / 隐藏侧栏账号列表", "Ctrl+Shift+A"),
            ("全屏切换", "F11"),
            ("本清单", "F1 / ?"),
            ("打开偏好设置", "Ctrl+,"),
            ("退出 Hylki", "Ctrl+Q"),
            ("关闭浮层 / 返回", "Esc"),
        ],
    ),
];

/// 键盘快捷键速查弹窗内容（F1 / ?，风格复用现有模态卡片）。
fn shortcuts_modal(p: &theme::HylkiPalette, cx: &mut Context<HylkiApp>) -> gpui_kit::AnyElement {
    let (text, muted_fg, accent, accent_soft, border) =
        (p.text, p.text_muted, p.accent, p.accent_soft(), p.border);
    let mut list = v_flex()
        .id("shortcuts-list")
        .flex_1()
        .min_h_0()
        .overflow_y_scrollbar()
        .px_5()
        .py_3()
        .gap_1p5();
    for (group_title, rows) in SHORTCUT_GROUPS {
        list = list
            .child(
                div()
                    .pt_2()
                    .pb_1()
                    .text_sm()
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .text_color(accent)
                    .child(SharedString::from(*group_title)),
            )
            .children(rows.iter().map(|(desc, keys)| {
                h_flex()
                    .justify_between()
                    .items_center()
                    .min_h(px(28.0))
                    .gap_3()
                    .child(
                        div()
                            .text_sm()
                            .text_color(text)
                            .truncate()
                            .child(SharedString::from(*desc)),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .px_1p5()
                            .py_0p5()
                            .rounded(px(6.0))
                            .bg(accent_soft)
                            .text_xs()
                            .text_color(accent)
                            .child(SharedString::from(*keys)),
                    )
            }));
    }
    v_flex()
        .flex_1()
        .min_h_0()
        .child(
            // 标题栏：标题 + 关闭按钮（Esc 亦可关闭）。
            h_flex()
                .flex_shrink_0()
                .px_5()
                .py_3()
                .items_start()
                .justify_between()
                .border_b_1()
                .border_color(border)
                .child(
                    v_flex().gap_1().child(
                        div()
                            .text_base()
                            .font_weight(gpui_kit::FontWeight::BOLD)
                            .text_color(text)
                            .child("键盘快捷键"),
                    ),
                )
                .child(
                    Button::new("shortcuts-close")
                        .ghost()
                        .compact()
                        .icon(IconName::X)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.close_modal(cx);
                        })),
                ),
        )
        .child(list)
        .child(
            div()
                .flex_shrink_0()
                .px_5()
                .py_2()
                .border_t_1()
                .border_color(border)
                .text_xs()
                .text_color(muted_fg)
                .child("单键快捷键可在 偏好设置 → 常规 → 「启用单键快捷键」关闭；关闭后上表单键（j / k / r / a 等）停用，Ctrl 组合键与 Delete / F5 不受影响。"),
        )
        .into_any_element()
}

/// 「关于 Hylki」弹窗内容（对齐原版 about.rs：版本、描述、协议、链接与致谢）。
fn about_modal(
    p: &theme::HylkiPalette,
    border: gpui_kit::Hsla,
    cx: &mut Context<HylkiApp>,
) -> gpui_kit::AnyElement {
    let (text, muted_fg, accent, accent_soft) = (p.text, p.text_muted, p.accent, p.accent_soft());
    let repo = "https://github.com/hyprlab/hylki";
    let issues = "https://github.com/hyprlab/hylki/issues";
    let license_url = "https://www.gnu.org/licenses/agpl-3.0.html";
    let link_row =
        |cx: &mut Context<HylkiApp>, id: &'static str, label: &'static str, url: &'static str| {
            h_flex()
                .w_full()
                .justify_between()
                .items_center()
                .min_h(px(30.0))
                .child(
                    div()
                        .text_sm()
                        .truncate()
                        .text_color(p.text)
                        .child(SharedString::from(label)),
                )
                .child(
                    Button::new(id)
                        .outline()
                        .compact()
                        .label("打开")
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.open_url(url);
                        })),
                )
        };
    v_flex()
        .flex_1()
        .min_h_0()
        .px_6()
        .py_5()
        .gap_2()
        .child(
            h_flex()
                .w_full()
                .justify_between()
                .items_center()
                .child(div())
                .child(
                    Button::new("about-close")
                        .ghost()
                        .compact()
                        .icon(IconName::X)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.close_modal(cx);
                        })),
                ),
        )
        .child(
            v_flex().items_center().gap_1().child(
                div()
                    .text_2xl()
                    .font_weight(gpui_kit::FontWeight::BOLD)
                    .text_color(text)
                    .child("Hylki"),
            ),
        )
        .child(
            div()
                .mx_auto()
                .px_2()
                .py_0p5()
                .rounded_full()
                .bg(accent_soft)
                .text_xs()
                .text_color(accent)
                .child(SharedString::from(format!(
                    "版本 {}",
                    env!("CARGO_PKG_VERSION")
                ))),
        )
        .child(
            div()
                .mt_1()
                .text_center()
                .text_sm()
                .text_color(muted_fg)
                .child("一个快速、原生的电子邮件客户端"),
        )
        .child(
            div()
                .mt_1()
                .text_center()
                .text_xs()
                .text_color(muted_fg)
                .child("本程序基于 GNU AGPL-3.0 或更新版本授权发布，不含任何形式的担保。"),
        )
        .child(div().mt_3().w_full().h(px(1.0)).bg(border))
        .child(
            div()
                .mt_2()
                .text_sm()
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .text_color(accent)
                .child("项目"),
        )
        .children([
            link_row(cx, "about-link-repo", "源代码", repo),
            link_row(cx, "about-link-issues", "问题反馈", issues),
            link_row(
                cx,
                "about-link-license",
                "许可证 (GNU AGPL v3)",
                license_url,
            ),
        ])
        .child(div().mt_3().w_full().h(px(1.0)).bg(border))
        .child(
            div()
                .mt_2()
                .text_center()
                .text_xs()
                .text_color(muted_fg)
                .child("感谢所有贡献者与翻译者（详见仓库 data/CONTRIBUTORS 与 data/TRANSLATORS）"),
        )
        .child(
            div()
                .text_center()
                .text_xs()
                .text_color(muted_fg)
                .child("© 2026 Hyprlab"),
        )
        .into_any_element()
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
    let mut parts: Vec<String> = current.split(',').map(|p| p.trim().to_string()).collect();
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

/// F-2：删除 / 归档后自动推进的目标下标——优先选中同位置的后一封（原 `removed + 1`，
/// 移除后即原位），已是最后一封则选上一封；列表只剩这一封时不选。
pub(crate) fn next_index_after_removal(removed: usize, len: usize) -> Option<usize> {
    if len <= 1 {
        return None;
    }
    if removed + 1 < len {
        Some(removed + 1)
    } else {
        Some(removed - 1)
    }
}

/// B-2：判定文件夹是否为回收站 / 垃圾邮件（语义对齐 cache.rs 的
/// TRASH_FOLDER_MATCH / JUNK_FOLDER_MATCH 与原版 FolderKind::Trash / Junk）。
pub(crate) fn is_trash_or_junk_folder(name: &str, path: &str) -> bool {
    let upper = path.to_ascii_uppercase();
    name == "已删除"
        || name == "垃圾邮件"
        || upper.contains("TRASH")
        || upper.contains("DELETED")
        || upper.contains("JUNK")
        || upper.contains("SPAM")
}

/// B-6：判定文件夹是否为已发送（语义对齐 cache.rs 的 SENT_FOLDER_MATCH）。
pub(crate) fn is_sent_folder(name: &str, path: &str) -> bool {
    name == "已发送" || path.to_ascii_uppercase().contains("SENT")
}

/// F-1：单键快捷键动作的共用守卫（纯函数，便于单测）。
///
/// 条件对齐 `DeleteSelectedMessage` 的既有守卫（Mail 阅读态 + 无浮层 +
/// 非文本输入聚焦），并额外要求单键开关开启。Ctrl 组合键动作不走此守卫
/// （不受单键偏好控制）。
fn single_key_guard_allows(
    active_modal: ActiveModal,
    main_view: MainContentView,
    right_mode: RightPaneMode,
    text_input_focused: bool,
    single_key_enabled: bool,
) -> bool {
    active_modal == ActiveModal::None
        && main_view == MainContentView::Mail
        && right_mode == RightPaneMode::Reader
        && !text_input_focused
        && single_key_enabled
}

/// B-2：演示模式合成 .eml 内容（信头 + 正文；非严格 RFC，仅用于演示导出链路）。
pub(crate) fn compose_demo_eml(detail: &MessageDetail) -> Vec<u8> {
    let eml = format!(
        "From: {}\r\nTo: {}\r\nSubject: {}\r\nDate: {}\r\nMessage-ID: {}\r\nContent-Type: text/plain; charset=UTF-8\r\n\r\n{}",
        detail.sender,
        detail.recipients,
        detail.subject,
        detail.date_display,
        detail.rfc_message_id,
        detail.body_text,
    );
    eml.into_bytes()
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
        assert!(capture_window_geometry(
            &mut prefs,
            WindowBounds::Windowed(b1)
        ));
        assert_eq!(prefs.window_width, Some(1400.0));
        assert_eq!(prefs.window_height, Some(860.0));
        assert!(!prefs.window_maximized);

        // 相同尺寸再次保存应返回 false。
        assert!(!capture_window_geometry(
            &mut prefs,
            WindowBounds::Windowed(b1)
        ));

        // 最大化状态：更新 window_maximized 且保留 restore bounds。
        assert!(capture_window_geometry(
            &mut prefs,
            WindowBounds::Maximized(b1)
        ));
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
            uid: id,
            folder_path: "INBOX".to_string(),
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
            unsubscribe_url: None,
            unsubscribe_one_click: false,
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
        assert_eq!(
            append_signature("", "张三\n研发部"),
            "\n\n-- \n张三\n研发部"
        );
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
                ..KnownContact::default()
            },
            KnownContact {
                name: "陈思远".to_string(),
                email: "siyuan@example.org".to_string(),
                message_count: 8,
                ..KnownContact::default()
            },
            KnownContact {
                name: String::new(),
                email: "alice@example.com".to_string(),
                message_count: 6,
                ..KnownContact::default()
            },
            KnownContact {
                name: "Bob".to_string(),
                email: "bob@example.com".to_string(),
                message_count: 5,
                ..KnownContact::default()
            },
            KnownContact {
                name: "Carol".to_string(),
                email: "carol@example.com".to_string(),
                message_count: 4,
                ..KnownContact::default()
            },
            KnownContact {
                name: "Dave".to_string(),
                email: "dave@example.com".to_string(),
                message_count: 3,
                ..KnownContact::default()
            },
            KnownContact {
                name: "Eve".to_string(),
                email: "eve@example.com".to_string(),
                message_count: 2,
                ..KnownContact::default()
            },
        ];

        // 空输入不弹建议
        assert!(filter_contact_suggestions("", &contacts, 5).is_empty());
        assert!(filter_contact_suggestions("xiaoming@example.com, ", &contacts, 5).is_empty());

        // 最多返回 5 条
        let many = filter_contact_suggestions("example", &contacts, 5);
        assert_eq!(many.len(), 5);

        // 排除前面已经填过的联系人
        let after_first =
            filter_contact_suggestions("林晓明 <xiaoming@example.com>, xiao", &contacts, 5);
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

    /// F-2：删除 / 归档后自动推进的目标下标（优先后一封，末尾则上一封，单封不推进）。
    #[test]
    fn next_index_after_removal_advances_to_neighbor() {
        assert_eq!(next_index_after_removal(0, 0), None);
        assert_eq!(next_index_after_removal(0, 1), None);
        assert_eq!(next_index_after_removal(0, 2), Some(1));
        assert_eq!(next_index_after_removal(1, 2), Some(0));
        assert_eq!(next_index_after_removal(2, 5), Some(3));
        assert_eq!(next_index_after_removal(4, 5), Some(3));
    }

    /// F-1：单键动作共用守卫（Mail 阅读态 + 无浮层 + 非文本输入 + 偏好开关）。
    #[test]
    fn single_key_guard_requires_mail_reader_no_modal_and_pref() {
        let ok = |text_input_focused, single_key_enabled| {
            single_key_guard_allows(
                ActiveModal::None,
                MainContentView::Mail,
                RightPaneMode::Reader,
                text_input_focused,
                single_key_enabled,
            )
        };
        // 成功路径：阅读态、无浮层、无输入焦点、开关开启。
        assert!(ok(false, true));
        // 关闭单键偏好后不生效。
        assert!(!ok(false, false));
        // 文本输入聚焦（搜索框 / 写信输入）时不生效。
        assert!(!ok(true, true));
        // 浮层打开 / 非阅读态 / 非邮件视图均不生效。
        assert!(!single_key_guard_allows(
            ActiveModal::Preferences,
            MainContentView::Mail,
            RightPaneMode::Reader,
            false,
            true
        ));
        assert!(!single_key_guard_allows(
            ActiveModal::None,
            MainContentView::Mail,
            RightPaneMode::Compose,
            false,
            true
        ));
        assert!(!single_key_guard_allows(
            ActiveModal::None,
            MainContentView::Contacts,
            RightPaneMode::Reader,
            false,
            true
        ));
    }

    /// B-2：Trash / Junk 文件夹判定（对齐 cache.rs 匹配规则）。
    #[test]
    fn trash_or_junk_folder_detection_matches_cache_rules() {
        assert!(is_trash_or_junk_folder("已删除", "INBOX/Trash"));
        assert!(is_trash_or_junk_folder("", "Deleted Messages"));
        assert!(is_trash_or_junk_folder("垃圾邮件", "Junk"));
        assert!(is_trash_or_junk_folder("", "spam"));
        // 普通文件夹不误判。
        assert!(!is_trash_or_junk_folder("收件箱", "INBOX"));
        assert!(!is_trash_or_junk_folder("已发送", "Sent"));
        assert!(!is_trash_or_junk_folder("归档", "Archive"));
    }

    /// B-6：已发送文件夹判定（对齐 cache.rs SENT_FOLDER_MATCH）。
    #[test]
    fn sent_folder_detection_matches_cache_rules() {
        assert!(is_sent_folder("已发送", "INBOX/Sent"));
        assert!(is_sent_folder("", "Sent Messages"));
        assert!(is_sent_folder("", "sentitems"));
        assert!(!is_sent_folder("收件箱", "INBOX"));
        assert!(!is_sent_folder("垃圾邮件", "Junk"));
    }

    /// B-2：演示模式合成 .eml 包含基本信头与正文。
    #[test]
    fn compose_demo_eml_includes_headers_and_body() {
        let detail = MessageDetail {
            id: 7,
            account_id: 1,
            folder_id: 1,
            folder_path: "INBOX".to_string(),
            uid: 7,
            rfc_message_id: "<demo-7@hylki.example>".to_string(),
            sender: "林晓明 <xiaoming@example.com>".to_string(),
            recipients: "demo@hylki.example".to_string(),
            subject: "周报".to_string(),
            date_display: "11-21 09:30".to_string(),
            body_text: "正文第一行".to_string(),
            body_html: None,
            flagged: false,
            attachments: Vec::new(),
            thread_messages: Vec::new(),
            unsubscribe_url: None,
            unsubscribe_one_click: false,
        };
        let bytes = compose_demo_eml(&detail);
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.starts_with("From: 林晓明 <xiaoming@example.com>\r\n"));
        assert!(text.contains("To: demo@hylki.example\r\n"));
        assert!(text.contains("Subject: 周报\r\n"));
        assert!(text.contains("Message-ID: <demo-7@hylki.example>\r\n"));
        assert!(text.contains("\r\n\r\n正文第一行"));
    }
}
