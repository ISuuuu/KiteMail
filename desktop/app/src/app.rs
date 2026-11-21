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
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    div, AppContext, Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled,
    Window,
};
use hylki_mail_core::config::{self, AccountConfig, AuthBackend, SecurityMode};
use hylki_mail_core::demo::{
    DemoStore, DEMO_ARCHIVE_ID, DEMO_DRAFTS_ID, DEMO_INBOX_ID, DEMO_SENT_ID,
};
use hylki_mail_core::worker::MailWorkerHandle;
use hylki_mail_core::{
    ComposeDraft, FolderSummary, LocalDraft, MailPage, MailRequest, MessageDetail, MessageSummary,
    OutboxItem, UnifiedView, WorkerEvent,
};

use crate::theme;
use crate::ui::{accounts, compose, message_list, reader, sidebar, welcome};

pub(crate) const PAGE_SIZE: usize = 100;
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
    AccountSetup,
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

    /// 元素 id（gpui `ElementId` 不支持 (&str, &str) 元组）。
    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::All => "filter-all",
            Self::Unread => "filter-unread",
            Self::Starred => "filter-starred",
        }
    }
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
    pub(crate) page_offset: usize,
    pub(crate) current_page: MailPage,
    pub(crate) selected_message: Option<MessageDetail>,
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
    pub(crate) cfg_auth_guide: Option<SharedString>,
    pub(crate) cfg_feedback: Option<(bool, SharedString)>,

    pub(crate) status_line: SharedString,
    pub(crate) error_banner: Option<SharedString>,

    // 迭代 1 新增：纯视觉状态（不影响后端行为）
    /// 当前主题（`theme::THEME_LIST` 中的 id），本轮固定 "default"。
    pub(crate) theme_id: &'static str,
    /// 亮 / 暗模式（菜单「切换亮/暗主题」即时生效）。
    pub(crate) dark_mode: bool,
    /// 左栏是否隐藏（中栏顶部开关）。
    pub(crate) sidebar_hidden: bool,
    /// 中栏列表客户端过滤。
    pub(crate) list_filter: ListFilter,
    /// 账号分组折叠状态（缺省展开）。
    pub(crate) collapsed_accounts: HashSet<u32>,
}

impl HylkiApp {
    pub(crate) fn new(demo_mode: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let demo_store = Arc::new(DemoStore::new());
        let data_dir = config::data_dir();
        let existing_accounts = if demo_mode {
            Vec::new()
        } else {
            config::load_accounts_in(&data_dir).unwrap_or_default()
        };

        let search_input = cx
            .new(|cx| InputState::new(window, cx).placeholder("搜索本地缓存（主题 / 发件人）..."));

        cx.subscribe(&search_input, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.search_query = input.read(cx).value().to_string();
                this.page_offset = 0;
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

        if demo_mode {
            let current_page = demo_store.list_messages_page(DEMO_INBOX_ID, "", 0, PAGE_SIZE);
            let selected_message = current_page
                .items
                .first()
                .and_then(|msg| demo_store.get_message_detail(msg.id));
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
                page_offset: 0,
                current_page,
                selected_message,
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
                cfg_auth_guide: None,
                cfg_feedback: None,
                status_line: SharedString::from("演示模式 (--demo)：已加载 10,000 封模拟邮件"),
                error_banner: None,
                theme_id: "default",
                dark_mode: false,
                sidebar_hidden: false,
                list_filter: ListFilter::All,
                collapsed_accounts: HashSet::new(),
            };
        }

        let worker = MailWorkerHandle::spawn(data_dir).ok().map(Arc::new);
        if let Some(ref w) = worker {
            if !existing_accounts.is_empty() {
                w.send(MailRequest::LoadAllCached);
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
        }

        let has_account = !existing_accounts.is_empty();
        let compose_account_id = existing_accounts.first().map(|a| a.id).unwrap_or(1);
        let status_line = if has_account {
            SharedString::from("正在从本地缓存加载并同步所有账号...")
        } else {
            SharedString::from("请先添加至少一个 IMAP / SMTP 邮箱账号")
        };

        Self {
            demo_mode: false,
            demo_store,
            worker,
            accounts: existing_accounts,
            folders_by_account: BTreeMap::new(),
            selected_folder_id: UNIFIED_INBOX_FOLDER_ID,
            current_view: CurrentView::UnifiedInbox,
            page_offset: 0,
            current_page: MailPage {
                folder_id: UNIFIED_INBOX_FOLDER_ID,
                offset: 0,
                total: 0,
                items: Vec::new(),
            },
            selected_message: None,
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
            cfg_auth_guide: None,
            cfg_feedback: None,
            status_line,
            error_banner: None,
            theme_id: "default",
            dark_mode: false,
            sidebar_hidden: false,
            list_filter: ListFilter::All,
            collapsed_accounts: HashSet::new(),
        }
    }

    fn apply_worker_events(&mut self, events: Vec<WorkerEvent>, cx: &mut Context<Self>) {
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
                    self.error_banner = None;
                    self.status_line =
                        SharedString::from(format!("账号 {} 已保存，正在同步...", account.email));
                }
                WorkerEvent::AccountDeleted { account_id } => {
                    self.accounts.retain(|a| a.id != account_id);
                    self.folders_by_account.remove(&account_id);
                    self.outbox_by_account.remove(&account_id);
                    self.selected_folder_id = UNIFIED_INBOX_FOLDER_ID;
                    self.current_view = CurrentView::UnifiedInbox;
                    self.selected_message = None;
                    self.pending_delete_account_id = None;
                    self.editing_account_id = self.accounts.first().map(|a| a.id);
                    self.right_mode = if self.accounts.is_empty() {
                        RightPaneMode::AccountSetup
                    } else {
                        RightPaneMode::Reader
                    };
                    self.status_line = SharedString::from("账号及本地缓存已移除");
                }
                WorkerEvent::FoldersLoaded {
                    account_id,
                    folders,
                } => {
                    self.folders_by_account.insert(account_id, folders);
                }
                WorkerEvent::MessagesPageLoaded(page) => {
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
                        self.current_page = page;
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
                    } else if matches!(
                        self.current_view,
                        CurrentView::UnifiedInbox | CurrentView::AccountFolder
                    ) {
                        self.reload_current_page(cx);
                    }
                }
                WorkerEvent::MessageBodyLoaded(detail) => {
                    self.selected_message = Some(detail);
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
                } => {
                    // 后端回执：随事件先行到达的 FoldersLoaded 已刷新文件夹计数；
                    // 这里刷新当前列表并在阅读态显示该邮件时回到空态。
                    self.after_message_moved(account_id, message_id, cx);
                }
                WorkerEvent::UnifiedViewLoaded { view, page } => {
                    if self.current_view.unified() == Some(view) {
                        self.current_page = page;
                        self.page_offset = self.current_page.offset;
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
                }
                WorkerEvent::CachedAttachmentsLoaded { .. }
                | WorkerEvent::KnownContactsLoaded { .. } => {
                    // TODO(phase3): wire gallery and contacts views.
                }
            }
        }
        cx.notify();
    }

    pub(crate) fn reload_current_page(&mut self, cx: &mut Context<Self>) {
        if self.demo_mode {
            // 演示模式：统一视图映射到对应的演示文件夹；
            // DemoStore 没有星标视图 → 空列表（中栏渲染「演示模式不支持该视图」提示）。
            let folder_id = match self.current_view {
                CurrentView::UnifiedInbox | CurrentView::AccountFolder => self.selected_folder_id,
                CurrentView::Sent => DEMO_SENT_ID,
                CurrentView::Drafts => DEMO_DRAFTS_ID,
                CurrentView::Archive => DEMO_ARCHIVE_ID,
                CurrentView::Starred => {
                    self.current_page = MailPage {
                        folder_id: 0,
                        offset: 0,
                        total: 0,
                        items: Vec::new(),
                    };
                    cx.notify();
                    return;
                }
            };
            self.current_page = self.demo_store.list_messages_page(
                folder_id,
                &self.search_query,
                self.page_offset,
                PAGE_SIZE,
            );
            cx.notify();
            return;
        }
        // 本地草稿视图：直接由渲染层从 `self.drafts` 合成，无需 worker 请求。
        if self.current_view == CurrentView::Drafts {
            cx.notify();
            return;
        }
        if let Some(ref w) = self.worker {
            match self.current_view {
                CurrentView::UnifiedInbox => {
                    w.send(MailRequest::LoadUnifiedInbox {
                        query: self.search_query.clone(),
                        offset: self.page_offset,
                        limit: PAGE_SIZE,
                    });
                }
                CurrentView::Starred | CurrentView::Sent | CurrentView::Archive => {
                    // unified() 在这三个视图下恒为 Some。
                    if let Some(view) = self.current_view.unified() {
                        w.send(MailRequest::LoadUnifiedView {
                            view,
                            query: self.search_query.clone(),
                            offset: self.page_offset,
                            limit: PAGE_SIZE,
                        });
                    }
                }
                CurrentView::AccountFolder => {
                    w.send(MailRequest::LoadMessagesPage {
                        folder_id: self.selected_folder_id,
                        query: self.search_query.clone(),
                        offset: self.page_offset,
                        limit: PAGE_SIZE,
                    });
                }
                CurrentView::Drafts => {}
            }
        }
    }

    /// 切换左栏统一导航视图（收件箱 / 星标 / 已发送 / 草稿 / 归档）。
    pub(crate) fn select_unified_view(&mut self, view: CurrentView, cx: &mut Context<Self>) {
        self.current_view = view;
        self.page_offset = 0;
        self.selected_message = None;
        self.right_mode = RightPaneMode::Reader;
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
        self.page_offset = 0;
        self.selected_message = None;
        self.right_mode = RightPaneMode::Reader;

        if self.demo_mode {
            self.current_page =
                self.demo_store
                    .list_messages_page(folder_id, &self.search_query, 0, PAGE_SIZE);
            self.selected_message = self
                .current_page
                .items
                .first()
                .and_then(|m| self.demo_store.get_message_detail(m.id));
            cx.notify();
            return;
        }

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
            self.selected_message = self.demo_store.get_message_detail(message_id);
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

    /// 标记已读 / 未读（复用现有 `MailRequest::SetSeen`）。
    pub(crate) fn toggle_message_seen(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.selected_message.clone() else {
            return;
        };
        if self.demo_mode {
            return;
        }
        let next_seen = false; // 阅读态下点击「标记为未读」
        if let Some(item) = self
            .current_page
            .items
            .iter_mut()
            .find(|m| m.id == detail.id)
        {
            item.unread = !next_seen;
        }
        if let Some(ref w) = self.worker {
            w.send(MailRequest::SetSeen {
                account_id: detail.account_id,
                folder_path: detail.folder_path.clone(),
                message_id: detail.id,
                uid: detail.uid,
                seen: next_seen,
            });
        }
        cx.notify();
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

    /// 归档 / 删除完成后的公共收尾：清空正在显示的目标邮件并刷新当前列表。
    fn after_message_moved(&mut self, account_id: u32, message_id: u32, cx: &mut Context<Self>) {
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

    pub(crate) fn start_new_compose(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.compose_account_id = self.accounts.first().map(|a| a.id).unwrap_or(1);
        self.compose_draft_id = None;
        self.compose_in_reply_to.clear();
        self.compose_attachments.clear();
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
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.right_mode = RightPaneMode::Compose;
        cx.notify();
    }

    pub(crate) fn start_reply(
        &mut self,
        detail: &MessageDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let reply_subject = if detail.subject.to_lowercase().starts_with("re:") {
            detail.subject.clone()
        } else {
            format!("Re: {}", detail.subject)
        };
        let quoted = format!(
            "\n\n------------------ 原始邮件 ------------------\n发件人：{}\n时间：{}\n主题：{}\n\n{}",
            detail.sender, detail.date_display, detail.subject, detail.body_text
        );

        self.compose_account_id = detail.account_id;
        self.compose_draft_id = None;
        self.compose_in_reply_to = detail.rfc_message_id.clone();
        self.compose_attachments.clear();
        self.compose_to
            .update(cx, |s, cx| s.set_value(detail.sender.clone(), window, cx));
        self.clear_compose_cc_bcc(window, cx);
        self.compose_subject
            .update(cx, |s, cx| s.set_value(reply_subject, window, cx));
        self.compose_body
            .update(cx, |s, cx| s.set_value(quoted, window, cx));
        self.right_mode = RightPaneMode::Compose;
        cx.notify();
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

    /// 转发：主题加 Fwd: 前缀，正文引用原邮件，收件人留空（沿用现有写信状态）。
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

        self.compose_account_id = detail.account_id;
        self.compose_draft_id = None;
        self.compose_in_reply_to.clear();
        self.compose_attachments.clear();
        self.compose_to
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.clear_compose_cc_bcc(window, cx);
        self.compose_subject
            .update(cx, |s, cx| s.set_value(forward_subject, window, cx));
        self.compose_body
            .update(cx, |s, cx| s.set_value(quoted, window, cx));
        self.right_mode = RightPaneMode::Compose;
        cx.notify();
    }

    /// start_reply 的共用主体。
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

        self.compose_account_id = detail.account_id;
        self.compose_draft_id = None;
        self.compose_in_reply_to = detail.rfc_message_id.clone();
        self.compose_attachments.clear();
        self.compose_to
            .update(cx, |s, cx| s.set_value(to.to_string(), window, cx));
        self.clear_compose_cc_bcc(window, cx);
        self.compose_subject
            .update(cx, |s, cx| s.set_value(reply_subject, window, cx));
        self.compose_body
            .update(cx, |s, cx| s.set_value(quoted, window, cx));
        self.right_mode = RightPaneMode::Compose;
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

    pub(crate) fn send_compose(&mut self, cx: &mut Context<Self>) {
        let to = self.compose_to.read(cx).value().to_string();
        let subject = self.compose_subject.read(cx).value().to_string();
        let body = self.compose_body.read(cx).value().to_string();

        if self.demo_mode {
            self.status_line = SharedString::from(format!(
                "演示发件成功：收件人=\"{}\"，主题=\"{}\"，附件={} 个",
                if to.is_empty() { "(未填写)" } else { &to },
                if subject.is_empty() {
                    "(无主题)"
                } else {
                    &subject
                },
                self.compose_attachments.len()
            ));
            self.right_mode = RightPaneMode::Reader;
            cx.notify();
            return;
        }

        if self.accounts.is_empty() {
            self.error_banner = Some(SharedString::from("请先配置并保存邮箱账号"));
            cx.notify();
            return;
        }

        if let Some(ref w) = self.worker {
            self.status_line = SharedString::from("正在持久化至发件箱并发送...");
            w.send(MailRequest::SendDraft {
                account_id: self.compose_account_id,
                draft: ComposeDraft {
                    to,
                    cc: self.compose_cc.read(cx).value().to_string(),
                    bcc: self.compose_bcc.read(cx).value().to_string(),
                    subject,
                    body,
                    in_reply_to: self.compose_in_reply_to.clone(),
                    attachment_paths: self.compose_attachments.clone(),
                },
                local_draft_id: self.compose_draft_id,
            });
        }
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
        self.cfg_feedback = None;
        self.cfg_auth_guide = None;
        self.right_mode = RightPaneMode::AccountSetup;
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
        let current_page = self
            .demo_store
            .list_messages_page(DEMO_INBOX_ID, "", 0, PAGE_SIZE);
        let selected_message = current_page
            .items
            .first()
            .and_then(|msg| self.demo_store.get_message_detail(msg.id));
        self.current_page = current_page;
        self.selected_message = selected_message;
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
        self.page_offset = 0;
        self.compose_account_id = 1;
        self.editing_account_id = Some(1);
        self.pending_delete_account_id = None;
        self.right_mode = RightPaneMode::Reader;
        self.error_banner = None;
        self.status_line = SharedString::from("演示模式：已加载 10,000 封模拟邮件");
        cx.notify();
    }

    /// 打开账号管理页（菜单 / 欢迎页入口），保留当前表单选择。
    pub(crate) fn open_account_manager(&mut self, cx: &mut Context<Self>) {
        self.pending_delete_account_id = None;
        self.right_mode = RightPaneMode::AccountSetup;
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

    pub(crate) fn prev_page(&mut self, cx: &mut Context<Self>) {
        if self.page_offset == 0 {
            return;
        }
        self.page_offset = self.page_offset.saturating_sub(PAGE_SIZE);
        self.reload_current_page(cx);
    }

    pub(crate) fn next_page(&mut self, cx: &mut Context<Self>) {
        let next = self.page_offset + PAGE_SIZE;
        if next >= self.current_page.total {
            return;
        }
        self.page_offset = next;
        self.reload_current_page(cx);
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

    /// 菜单「切换亮/暗主题」：即时生效（纯视觉，不落盘，phase3 由偏好设置持久化）。
    pub(crate) fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        self.dark_mode = !self.dark_mode;
        theme::apply(self.theme_id, self.dark_mode, cx);
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

    pub(crate) fn set_list_filter(&mut self, filter: ListFilter, cx: &mut Context<Self>) {
        self.list_filter = filter;
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

    // ---- 右栏渲染（阅读态 / 写信 / 账号管理 / 欢迎页由 ui:: 渲染）----

    fn render_right_pane(&mut self, cx: &mut Context<Self>) -> gpui_kit::AnyElement {
        match self.right_mode {
            RightPaneMode::Compose => compose::render(self, cx),
            RightPaneMode::AccountSetup => accounts::render(self, cx),
            RightPaneMode::Welcome => welcome::render(self, cx),
            RightPaneMode::Reader => reader::render(self, cx),
        }
    }
}

impl Render for HylkiApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 避坑清单第 4 条：先把语义色 Copy 成局部变量再构建 UI。
        let p = theme::current(cx);
        let border = p.border;
        let muted_fg = p.text_muted;

        let sidebar_el = if self.sidebar_hidden {
            None
        } else {
            Some(sidebar::render(self, cx))
        };
        let list_el = message_list::render(self, cx);
        let right_el = self.render_right_pane(cx);

        v_flex()
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
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .children(sidebar_el)
                    .child(list_el)
                    .child(right_el),
            )
            .child(
                h_flex()
                    .px_3()
                    .py_1()
                    .border_t_1()
                    .border_color(border)
                    .bg(p.sidebar)
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted_fg)
                            .child(self.status_line.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted_fg)
                            .child(if self.demo_mode { "DEMO" } else { "WINDOWS" }),
                    ),
            )
    }
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
}
