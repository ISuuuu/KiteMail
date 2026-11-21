//! Native Windows GPUI desktop client for Hylki.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{h_flex, v_flex, ActiveTheme};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use hylki_mail_core::config::{self, AccountConfig, AuthBackend, SecurityMode};
use hylki_mail_core::demo::{DemoStore, DEMO_INBOX_ID};
use hylki_mail_core::worker::MailWorkerHandle;
use hylki_mail_core::{
    ComposeDraft, FolderSummary, LocalDraft, MailPage, MailRequest, MessageDetail, OutboxItem,
    OutboxStatus, WorkerEvent,
};

const PAGE_SIZE: usize = 100;
const UNIFIED_INBOX_FOLDER_ID: u32 = 0;

struct ProviderPreset {
    id: &'static str,
    label: &'static str,
    imap_host: &'static str,
    imap_port: &'static str,
    imap_tls: bool,
    smtp_host: &'static str,
    smtp_port: &'static str,
    smtp_tls: bool,
    guide: &'static str,
}

const PROVIDER_PRESETS: &[ProviderPreset] = &[
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
enum RightPaneMode {
    Reader,
    Compose,
    AccountSetup,
}

struct HylkiApp {
    demo_mode: bool,
    demo_store: Arc<DemoStore>,
    worker: Option<Arc<MailWorkerHandle>>,
    accounts: Vec<AccountConfig>,
    folders_by_account: BTreeMap<u32, Vec<FolderSummary>>,
    /// `0` means Unified Inbox across all accounts.
    selected_folder_id: u32,
    page_offset: usize,
    current_page: MailPage,
    selected_message: Option<MessageDetail>,
    drafts: Vec<LocalDraft>,
    outbox_by_account: BTreeMap<u32, Vec<OutboxItem>>,
    right_mode: RightPaneMode,
    search_input: Entity<InputState>,
    search_query: String,

    // Compose / Reply fields
    compose_account_id: u32,
    compose_draft_id: Option<u32>,
    compose_to: Entity<InputState>,
    compose_subject: Entity<InputState>,
    compose_body: Entity<TextareaState>,
    compose_in_reply_to: String,
    compose_attachments: Vec<String>,

    // Account configuration fields
    editing_account_id: Option<u32>,
    cfg_display_name: Entity<InputState>,
    cfg_email: Entity<InputState>,
    cfg_imap_host: Entity<InputState>,
    cfg_imap_port: Entity<InputState>,
    cfg_imap_tls: bool,
    cfg_imap_user: Entity<InputState>,
    cfg_imap_pass: Entity<InputState>,
    cfg_smtp_host: Entity<InputState>,
    cfg_smtp_port: Entity<InputState>,
    cfg_smtp_tls: bool,
    cfg_smtp_separate: bool,
    cfg_smtp_user: Entity<InputState>,
    cfg_smtp_pass: Entity<InputState>,
    cfg_oauth_client_id: Entity<InputState>,
    cfg_auth_guide: Option<SharedString>,
    cfg_feedback: Option<(bool, SharedString)>,

    status_line: SharedString,
    error_banner: Option<SharedString>,
}

impl HylkiApp {
    fn new(demo_mode: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
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
                compose_subject,
                compose_body,
                compose_in_reply_to: String::new(),
                compose_attachments: Vec::new(),
                editing_account_id: Some(1),
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
            right_mode: if has_account {
                RightPaneMode::Reader
            } else {
                RightPaneMode::AccountSetup
            },
            search_input,
            search_query: String::new(),
            compose_account_id,
            compose_draft_id: None,
            compose_to,
            compose_subject,
            compose_body,
            compose_in_reply_to: String::new(),
            compose_attachments: Vec::new(),
            editing_account_id,
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
                    self.selected_message = None;
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
                    if page.folder_id == self.selected_folder_id {
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
                    } else if self.selected_folder_id == UNIFIED_INBOX_FOLDER_ID {
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
            }
        }
        cx.notify();
    }

    fn reload_current_page(&mut self, cx: &mut Context<Self>) {
        if self.demo_mode {
            self.current_page = self.demo_store.list_messages_page(
                self.selected_folder_id,
                &self.search_query,
                self.page_offset,
                PAGE_SIZE,
            );
            cx.notify();
            return;
        }
        if let Some(ref w) = self.worker {
            if self.selected_folder_id == UNIFIED_INBOX_FOLDER_ID {
                w.send(MailRequest::LoadUnifiedInbox {
                    query: self.search_query.clone(),
                    offset: self.page_offset,
                    limit: PAGE_SIZE,
                });
            } else {
                w.send(MailRequest::LoadMessagesPage {
                    folder_id: self.selected_folder_id,
                    query: self.search_query.clone(),
                    offset: self.page_offset,
                    limit: PAGE_SIZE,
                });
            }
        }
    }

    fn select_unified_inbox(&mut self, cx: &mut Context<Self>) {
        self.selected_folder_id = UNIFIED_INBOX_FOLDER_ID;
        self.page_offset = 0;
        self.selected_message = None;
        self.right_mode = RightPaneMode::Reader;
        self.reload_current_page(cx);
        cx.notify();
    }

    fn select_folder(
        &mut self,
        account_id: u32,
        folder_id: u32,
        folder_path: String,
        cx: &mut Context<Self>,
    ) {
        self.selected_folder_id = folder_id;
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

    fn select_message(
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

    fn toggle_flagged(&mut self, detail: &MessageDetail, cx: &mut Context<Self>) {
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

    fn start_new_compose(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.compose_account_id = self.accounts.first().map(|a| a.id).unwrap_or(1);
        self.compose_draft_id = None;
        self.compose_in_reply_to.clear();
        self.compose_attachments.clear();
        self.compose_to
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.compose_subject
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.compose_body
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.right_mode = RightPaneMode::Compose;
        cx.notify();
    }

    fn start_reply(&mut self, detail: &MessageDetail, window: &mut Window, cx: &mut Context<Self>) {
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
        self.compose_subject
            .update(cx, |s, cx| s.set_value(reply_subject, window, cx));
        self.compose_body
            .update(cx, |s, cx| s.set_value(quoted, window, cx));
        self.right_mode = RightPaneMode::Compose;
        cx.notify();
    }

    fn open_local_draft(
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
        self.compose_subject
            .update(cx, |s, cx| s.set_value(draft.subject.clone(), window, cx));
        self.compose_body
            .update(cx, |s, cx| s.set_value(draft.body_text.clone(), window, cx));
        self.right_mode = RightPaneMode::Compose;
        cx.notify();
    }

    fn save_local_draft(&mut self, cx: &mut Context<Self>) {
        if self.demo_mode {
            self.status_line = SharedString::from("演示模式下不支持写入本地草稿数据库");
            cx.notify();
            return;
        }
        let draft = LocalDraft {
            id: self.compose_draft_id.unwrap_or(0),
            account_id: self.compose_account_id,
            to_addr: self.compose_to.read(cx).value().to_string(),
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

    fn pick_attachments(&mut self, cx: &mut Context<Self>) {
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

    fn save_attachment(&self, att_id: u32, filename: &str) {
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

    fn send_compose(&mut self, cx: &mut Context<Self>) {
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

    fn load_account_into_form(
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

    fn apply_preset(
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

    fn start_outlook_oauth(&mut self, cx: &mut Context<Self>) {
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

    fn verify_account_form(&mut self, cx: &mut Context<Self>) {
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

    fn save_account_form(&mut self, cx: &mut Context<Self>) {
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

    fn prev_page(&mut self, cx: &mut Context<Self>) {
        if self.page_offset == 0 {
            return;
        }
        self.page_offset = self.page_offset.saturating_sub(PAGE_SIZE);
        self.reload_current_page(cx);
    }

    fn next_page(&mut self, cx: &mut Context<Self>) {
        let next = self.page_offset + PAGE_SIZE;
        if next >= self.current_page.total {
            return;
        }
        self.page_offset = next;
        self.reload_current_page(cx);
    }

    fn account_email_for(&self, account_id: u32) -> &str {
        self.accounts
            .iter()
            .find(|a| a.id == account_id)
            .map(|a| a.email.as_str())
            .unwrap_or("")
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        let sidebar_bg = cx.theme().sidebar;
        h_flex()
            .px_3()
            .py_2()
            .gap_2()
            .items_center()
            .justify_between()
            .bg(sidebar_bg)
            .border_b_1()
            .border_color(border)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("toolbar-compose")
                            .primary()
                            .label("写邮件")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.start_new_compose(window, cx);
                            })),
                    )
                    .when(!self.demo_mode, |el| {
                        el.child(
                            Button::new("toolbar-sync")
                                .outline()
                                .label("全部刷新")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(ref w) = this.worker {
                                        this.error_banner = None;
                                        this.status_line =
                                            SharedString::from("正在独立刷新所有账号...");
                                        w.send(MailRequest::SyncAllAccounts);
                                        cx.notify();
                                    }
                                })),
                        )
                    })
                    .child(
                        Button::new("toolbar-account")
                            .ghost()
                            .label(if self.right_mode == RightPaneMode::AccountSetup {
                                "返回邮件"
                            } else {
                                "账号管理"
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this.right_mode == RightPaneMode::AccountSetup {
                                    this.right_mode = RightPaneMode::Reader;
                                    cx.notify();
                                } else {
                                    let first = this.accounts.first().cloned();
                                    this.load_account_into_form(first.as_ref(), window, cx);
                                }
                            })),
                    ),
            )
            .child(div().w(px(300.0)).child(Input::new(&self.search_input)))
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        let sidebar_bg = cx.theme().sidebar;
        let muted_fg = cx.theme().muted_foreground;
        let accent = cx.theme().accent;
        let primary = cx.theme().primary;
        let primary_fg = cx.theme().primary_foreground;
        let danger = cx.theme().danger;
        let selected_folder_id = self.selected_folder_id;

        v_flex()
            .w(px(240.0))
            .h_full()
            .border_r_1()
            .border_color(border)
            .bg(sidebar_bg)
            .p_3()
            .gap_3()
            .overflow_y_scrollbar()
            .when(!self.demo_mode && !self.accounts.is_empty(), |el| {
                el.child(
                    div()
                        .id("unified-inbox-btn")
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .cursor_pointer()
                        .when(selected_folder_id == UNIFIED_INBOX_FOLDER_ID, |b| {
                            b.bg(accent)
                        })
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::BOLD)
                                .child("统一收件箱"),
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.select_unified_inbox(cx);
                        })),
                )
            })
            .when(self.accounts.is_empty(), |el| {
                el.child(
                    div()
                        .text_xs()
                        .text_color(muted_fg)
                        .child("尚未配置邮箱，请在右侧添加账号。"),
                )
            })
            .children(self.accounts.iter().map(|acc| {
                let acc_id = acc.id;
                let folders = self
                    .folders_by_account
                    .get(&acc_id)
                    .cloned()
                    .unwrap_or_default();
                let title = if acc.display_name.is_empty() {
                    acc.email.clone()
                } else {
                    format!("{} ({})", acc.display_name, acc.email)
                };
                v_flex()
                    .gap_1()
                    .pb_2()
                    .border_b_1()
                    .border_color(border)
                    .child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(muted_fg)
                                    .child(title),
                            )
                            .when(!self.demo_mode, |e| {
                                e.child(
                                    Button::new(("sync-acc", acc_id))
                                        .ghost()
                                        .label("↻")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if let Some(ref w) = this.worker {
                                                w.send(MailRequest::SyncAccount {
                                                    account_id: acc_id,
                                                });
                                                cx.notify();
                                            }
                                        })),
                                )
                            }),
                    )
                    .children(folders.into_iter().map(|folder| {
                        let is_selected = folder.id == selected_folder_id;
                        let folder_id = folder.id;
                        let folder_path = folder.path.clone();
                        h_flex()
                            .id(("folder", folder_id))
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .justify_between()
                            .items_center()
                            .cursor_pointer()
                            .when(is_selected, |el| el.bg(accent))
                            .child(div().text_sm().child(folder.name))
                            .when(folder.unread_count > 0 || folder.total_count > 0, |el| {
                                let count_text = if folder.unread_count > 0 {
                                    folder.unread_count.to_string()
                                } else {
                                    folder.total_count.to_string()
                                };
                                el.child(
                                    div()
                                        .px_2()
                                        .rounded_full()
                                        .text_xs()
                                        .when(folder.unread_count > 0, |b| {
                                            b.bg(primary).text_color(primary_fg)
                                        })
                                        .when(folder.unread_count == 0, |b| b.text_color(muted_fg))
                                        .child(count_text),
                                )
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_folder(acc_id, folder_id, folder_path.clone(), cx);
                            }))
                    }))
            }))
            .when(!self.drafts.is_empty(), |el| {
                el.child(
                    v_flex()
                        .gap_1()
                        .pb_2()
                        .border_b_1()
                        .border_color(border)
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::BOLD)
                                .text_color(muted_fg)
                                .child(format!("本地草稿 ({})", self.drafts.len())),
                        )
                        .children(self.drafts.iter().map(|draft| {
                            let d = draft.clone();
                            let title = if d.subject.trim().is_empty() {
                                "(无主题草稿)".to_string()
                            } else {
                                d.subject.clone()
                            };
                            div()
                                .id(("draft", d.id))
                                .px_2()
                                .py_1()
                                .rounded_md()
                                .cursor_pointer()
                                .child(div().text_xs().child(title))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.open_local_draft(&d, window, cx);
                                }))
                        })),
                )
            })
            .when(!self.outbox_by_account.is_empty(), |el| {
                let all_outbox: Vec<OutboxItem> = self
                    .outbox_by_account
                    .values()
                    .flat_map(|v| v.iter().cloned())
                    .collect();
                el.child(
                    v_flex()
                        .gap_2()
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::BOLD)
                                .text_color(danger)
                                .child(format!("发件箱待处理 ({})", all_outbox.len())),
                        )
                        .children(all_outbox.into_iter().map(|item| {
                            let outbox_id = item.id;
                            let account_id = item.account_id;
                            let label = match item.status {
                                OutboxStatus::Uncertain => "发送结果待确认",
                                OutboxStatus::Failed => "发送失败",
                                OutboxStatus::Sending => "正在发送",
                                OutboxStatus::Queued => "排队中",
                            };
                            v_flex()
                                .p_2()
                                .rounded_md()
                                .border_1()
                                .border_color(border)
                                .gap_1()
                                .child(div().text_xs().font_weight(FontWeight::BOLD).child(label))
                                .child(div().text_xs().child(item.subject))
                                .when(!item.last_error.is_empty(), |e| {
                                    e.child(
                                        div().text_xs().text_color(muted_fg).child(item.last_error),
                                    )
                                })
                                .when(item.status != OutboxStatus::Sending, |e| {
                                    e.child(
                                        Button::new(("retry-outbox", outbox_id))
                                            .ghost()
                                            .label("手动重发")
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if let Some(ref w) = this.worker {
                                                    w.send(MailRequest::RetryOutboxItem {
                                                        account_id,
                                                        outbox_id,
                                                    });
                                                    cx.notify();
                                                }
                                            })),
                                    )
                                })
                        })),
                )
            })
    }

    fn render_message_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        let muted_fg = cx.theme().muted_foreground;
        let accent = cx.theme().accent;
        let primary = cx.theme().primary;
        let selected_id = self.selected_message.as_ref().map(|m| m.id);
        let start = if self.current_page.items.is_empty() {
            0
        } else {
            self.current_page.offset + 1
        };
        let end = self.current_page.offset + self.current_page.items.len();
        let total = self.current_page.total;
        let show_account_tag =
            self.selected_folder_id == UNIFIED_INBOX_FOLDER_ID && self.accounts.len() > 1;

        v_flex()
            .w(px(370.0))
            .h_full()
            .border_r_1()
            .border_color(border)
            .child(
                h_flex()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(border)
                    .justify_between()
                    .items_center()
                    .child(div().text_xs().text_color(muted_fg).child(if total == 0 {
                        "暂无邮件".to_string()
                    } else {
                        format!("第 {start}-{end} / 共 {total} 封")
                    }))
                    .child(
                        h_flex()
                            .gap_1()
                            .child(Button::new("prev-page").ghost().label("上一页").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.prev_page(cx);
                                }),
                            ))
                            .child(Button::new("next-page").ghost().label("下一页").on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.next_page(cx);
                                }),
                            )),
                    ),
            )
            .child(
                div()
                    .id("message-list-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .children(self.current_page.items.iter().map(|msg| {
                        let account_id = msg.account_id;
                        let msg_id = msg.id;
                        let uid = msg.uid;
                        let unread = msg.unread;
                        let folder_path = msg.folder_path.clone();
                        let is_selected = selected_id == Some(msg_id);
                        let avatar_char = sender_initial(&msg.sender);
                        let acc_email = if show_account_tag {
                            self.account_email_for(account_id).to_string()
                        } else {
                            String::new()
                        };

                        h_flex()
                            .id(("msg", msg_id))
                            .px_3()
                            .py_2()
                            .gap_3()
                            .items_start()
                            .border_b_1()
                            .border_color(border)
                            .cursor_pointer()
                            .when(is_selected, |el| el.bg(accent))
                            .child(
                                div()
                                    .w(px(30.0))
                                    .h(px(30.0))
                                    .rounded_full()
                                    .bg(accent)
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_xs()
                                    .font_weight(FontWeight::BOLD)
                                    .child(avatar_char),
                            )
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1()
                                    .child(
                                        h_flex()
                                            .justify_between()
                                            .items_center()
                                            .gap_2()
                                            .child(
                                                h_flex()
                                                    .gap_1()
                                                    .items_center()
                                                    .when(msg.unread, |el| {
                                                        el.child(
                                                            div()
                                                                .w(px(7.0))
                                                                .h(px(7.0))
                                                                .rounded_full()
                                                                .bg(primary),
                                                        )
                                                    })
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .font_weight(if msg.unread {
                                                                FontWeight::BOLD
                                                            } else {
                                                                FontWeight::NORMAL
                                                            })
                                                            .child(msg.sender.clone()),
                                                    ),
                                            )
                                            .child(div().text_xs().text_color(muted_fg).child(
                                                format!(
                                                    "{}{}{}",
                                                    if msg.flagged { "★ " } else { "" },
                                                    if msg.has_attachment { "📎 " } else { "" },
                                                    msg.date_display
                                                ),
                                            )),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(if msg.unread {
                                                FontWeight::SEMIBOLD
                                            } else {
                                                FontWeight::NORMAL
                                            })
                                            .child(msg.subject.clone()),
                                    )
                                    .child(div().text_xs().text_color(muted_fg).child(
                                        if acc_email.is_empty() {
                                            msg.preview.clone()
                                        } else {
                                            format!("[{acc_email}] {}", msg.preview)
                                        },
                                    )),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.select_message(
                                    account_id,
                                    msg_id,
                                    uid,
                                    unread,
                                    &folder_path,
                                    cx,
                                );
                            }))
                    })),
            )
    }

    fn render_right_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        let muted_fg = cx.theme().muted_foreground;
        let danger = cx.theme().danger;
        let primary = cx.theme().primary;
        let accent = cx.theme().accent;

        match self.right_mode {
            RightPaneMode::Compose => v_flex()
                .flex_1()
                .h_full()
                .p_4()
                .gap_3()
                .child(
                    h_flex()
                        .justify_between()
                        .items_center()
                        .child(
                            v_flex()
                                .gap_1()
                                .child(div().text_base().font_weight(FontWeight::BOLD).child(
                                    if self.compose_in_reply_to.is_empty() {
                                        "写新邮件"
                                    } else {
                                        "回复邮件"
                                    },
                                ))
                                .when(!self.accounts.is_empty(), |el| {
                                    el.child(
                                        h_flex()
                                            .gap_1()
                                            .items_center()
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(muted_fg)
                                                    .child("发件账号："),
                                            )
                                            .children(self.accounts.iter().map(|acc| {
                                                let acc_id = acc.id;
                                                let active = acc_id == self.compose_account_id;
                                                Button::new(("compose-from", acc_id))
                                                    .ghost()
                                                    .label(if active {
                                                        format!("● {}", acc.email)
                                                    } else {
                                                        acc.email.clone()
                                                    })
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        this.compose_account_id = acc_id;
                                                        cx.notify();
                                                    }))
                                            })),
                                    )
                                }),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    Button::new("compose-save-draft")
                                        .outline()
                                        .label("存草稿")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.save_local_draft(cx);
                                        })),
                                )
                                .child(
                                    Button::new("compose-attach")
                                        .outline()
                                        .label("添加附件")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.pick_attachments(cx);
                                        })),
                                )
                                .child(
                                    Button::new("compose-cancel")
                                        .ghost()
                                        .label("取消")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.right_mode = RightPaneMode::Reader;
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Button::new("compose-send")
                                        .primary()
                                        .label("发送")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.send_compose(cx);
                                        })),
                                ),
                        ),
                )
                .child(Input::new(&self.compose_to))
                .child(Input::new(&self.compose_subject))
                .when(!self.compose_attachments.is_empty(), |el| {
                    el.child(
                        h_flex().gap_2().flex_wrap().children(
                            self.compose_attachments
                                .iter()
                                .enumerate()
                                .map(|(idx, path)| {
                                    let name = std::path::Path::new(path)
                                        .file_name()
                                        .and_then(|n| n.to_str())
                                        .unwrap_or(path)
                                        .to_string();
                                    Button::new(("remove-att", idx))
                                        .outline()
                                        .label(format!("📎 {name} ×"))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if idx < this.compose_attachments.len() {
                                                this.compose_attachments.remove(idx);
                                                cx.notify();
                                            }
                                        }))
                                }),
                        ),
                    )
                })
                .child(div().flex_1().child(Textarea::new(&self.compose_body)))
                .into_any_element(),

            RightPaneMode::AccountSetup => v_flex()
                .flex_1()
                .h_full()
                .p_5()
                .gap_3()
                .overflow_y_scrollbar()
                .when(!self.accounts.is_empty(), |el| {
                    el.child(
                        h_flex()
                            .gap_2()
                            .flex_wrap()
                            .items_center()
                            .pb_2()
                            .border_b_1()
                            .border_color(border)
                            .children(self.accounts.iter().map(|acc| {
                                let cfg_clone = acc.clone();
                                let selected = self.editing_account_id == Some(acc.id);
                                Button::new(("edit-acc", acc.id))
                                    .outline()
                                    .label(if selected {
                                        format!("● {}", acc.email)
                                    } else {
                                        acc.email.clone()
                                    })
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.load_account_into_form(Some(&cfg_clone), window, cx);
                                    }))
                            }))
                            .child(
                                Button::new("add-new-acc")
                                    .primary()
                                    .label("+ 添加新账号")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.load_account_into_form(None, window, cx);
                                    })),
                            ),
                    )
                })
                .child(
                    v_flex()
                        .gap_2()
                        .child(div().text_lg().font_weight(FontWeight::BOLD).child(
                            if self.editing_account_id.is_some() {
                                "编辑邮箱账号（IMAP / SMTP）"
                            } else {
                                "添加新邮箱账号（IMAP / SMTP）"
                            },
                        ))
                        .child(
                            h_flex()
                                .gap_2()
                                .flex_wrap()
                                .children(PROVIDER_PRESETS.iter().map(|preset| {
                                    Button::new(preset.id)
                                        .outline()
                                        .label(preset.label)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.apply_preset(preset, window, cx);
                                        }))
                                })),
                        ),
                )
                .when(!self.demo_mode, |el| {
                    el.child(
                        v_flex()
                            .p_3()
                            .gap_2()
                            .rounded_md()
                            .border_1()
                            .border_color(border)
                            .child(
                                h_flex()
                                    .justify_between()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        v_flex()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .font_weight(FontWeight::BOLD)
                                                    .child("Outlook / Microsoft 365 (OAuth2 登录)"),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(muted_fg)
                                                    .child("微软已禁用传统密码登录，点击右侧按钮通过系统浏览器授权并通过 Microsoft Graph 收发邮件。"),
                                            ),
                                    )
                                    .child(
                                        Button::new("outlook-oauth-btn")
                                            .primary()
                                            .label("Outlook OAuth2 登录")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.start_outlook_oauth(cx);
                                            })),
                                    ),
                            )
                            .child(Input::new(&self.cfg_oauth_client_id)),
                    )
                })
                .when_some(self.cfg_auth_guide.as_ref(), |el, guide| {
                    el.child(
                        div()
                            .p_2()
                            .rounded_md()
                            .bg(accent)
                            .text_xs()
                            .child(guide.clone()),
                    )
                })
                .child(div().text_xs().text_color(muted_fg).child(
                    "授权码仅保存在 Windows 凭据管理器中，配置文件 accounts.toml 不记录密码。",
                ))
                .child(
                    h_flex()
                        .gap_2()
                        .child(div().flex_1().child(Input::new(&self.cfg_display_name)))
                        .child(div().flex_1().child(Input::new(&self.cfg_email))),
                )
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::BOLD)
                        .child("收信设置 (IMAP)"),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(div().flex_1().child(Input::new(&self.cfg_imap_host)))
                        .child(div().w(px(100.0)).child(Input::new(&self.cfg_imap_port)))
                        .child(
                            Button::new("toggle-imap-tls")
                                .outline()
                                .label(if self.cfg_imap_tls {
                                    "TLS (993)"
                                } else {
                                    "STARTTLS (143)"
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cfg_imap_tls = !this.cfg_imap_tls;
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(div().flex_1().child(Input::new(&self.cfg_imap_user)))
                        .child(div().flex_1().child(Input::new(&self.cfg_imap_pass))),
                )
                .child(
                    h_flex()
                        .justify_between()
                        .items_center()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::BOLD)
                                .child("发信设置 (SMTP)"),
                        )
                        .child(
                            Button::new("toggle-smtp-sep")
                                .ghost()
                                .label(if self.cfg_smtp_separate {
                                    "✓ 使用独立 SMTP 凭据"
                                } else {
                                    "复用 IMAP 凭据"
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cfg_smtp_separate = !this.cfg_smtp_separate;
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(div().flex_1().child(Input::new(&self.cfg_smtp_host)))
                        .child(div().w(px(100.0)).child(Input::new(&self.cfg_smtp_port)))
                        .child(
                            Button::new("toggle-smtp-tls")
                                .outline()
                                .label(if self.cfg_smtp_tls {
                                    "TLS (465)"
                                } else {
                                    "STARTTLS (587)"
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.cfg_smtp_tls = !this.cfg_smtp_tls;
                                    cx.notify();
                                })),
                        ),
                )
                .when(self.cfg_smtp_separate, |el| {
                    el.child(
                        h_flex()
                            .gap_2()
                            .child(div().flex_1().child(Input::new(&self.cfg_smtp_user)))
                            .child(div().flex_1().child(Input::new(&self.cfg_smtp_pass))),
                    )
                })
                .when_some(self.cfg_feedback.as_ref(), |el, (ok, msg)| {
                    el.child(
                        div()
                            .text_xs()
                            .text_color(if *ok { primary } else { danger })
                            .child(msg.clone()),
                    )
                })
                .child(
                    h_flex()
                        .gap_2()
                        .pt_2()
                        .child(
                            Button::new("verify-account")
                                .outline()
                                .label("测试连接")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.verify_account_form(cx);
                                })),
                        )
                        .child(
                            Button::new("save-account")
                                .primary()
                                .label("保存并同步")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.save_account_form(cx);
                                })),
                        )
                        .when_some(self.editing_account_id, |el, acc_id| {
                            el.child(
                                Button::new("delete-account")
                                    .ghost()
                                    .label("移除此账号")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if let Some(ref w) = this.worker {
                                            w.send(MailRequest::DeleteAccount {
                                                account_id: acc_id,
                                            });
                                            cx.notify();
                                        }
                                    })),
                            )
                        }),
                )
                .into_any_element(),

            RightPaneMode::Reader => match &self.selected_message {
                Some(detail) => {
                    let reply_detail = detail.clone();
                    let star_detail = detail.clone();
                    let owner_email = self.account_email_for(detail.account_id).to_string();
                    v_flex()
                        .flex_1()
                        .h_full()
                        .p_5()
                        .gap_3()
                        .overflow_y_scrollbar()
                        .child(
                            h_flex()
                                .justify_between()
                                .items_start()
                                .gap_3()
                                .child(
                                    div()
                                        .flex_1()
                                        .text_lg()
                                        .font_weight(FontWeight::BOLD)
                                        .child(detail.subject.clone()),
                                )
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .child(
                                            Button::new("star-btn")
                                                .outline()
                                                .label(if detail.flagged {
                                                    "★ 已加星标"
                                                } else {
                                                    "☆ 加星标"
                                                })
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.toggle_flagged(&star_detail, cx);
                                                })),
                                        )
                                        .child(
                                            Button::new("reply-btn")
                                                .primary()
                                                .label("回复")
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.start_reply(&reply_detail, window, cx);
                                                    },
                                                )),
                                        ),
                                ),
                        )
                        .child(
                            v_flex()
                                .gap_1()
                                .pb_3()
                                .border_b_1()
                                .border_color(border)
                                .child(div().text_sm().child(format!("发件人：{}", detail.sender)))
                                .child(div().text_xs().text_color(muted_fg).child(format!(
                                    "收件人：{}  ·  时间：{}{} ",
                                    detail.recipients,
                                    detail.date_display,
                                    if owner_email.is_empty() {
                                        String::new()
                                    } else {
                                        format!("  ·  所属账号：{owner_email}")
                                    }
                                ))),
                        )
                        .when(!detail.attachments.is_empty(), |el| {
                            el.child(h_flex().gap_2().flex_wrap().children(
                                detail.attachments.iter().map(|att| {
                                    let att_id = att.id;
                                    let filename = att.filename.clone();
                                    let size_kb = (att.size_bytes + 1023) / 1024;
                                    Button::new(("save-att", att_id))
                                        .outline()
                                        .label(format!("📎 {} ({size_kb} KB)", att.filename))
                                        .on_click(cx.listener(move |this, _, _, _| {
                                            this.save_attachment(att_id, &filename);
                                        }))
                                }),
                            ))
                        })
                        .child(div().text_sm().child(detail.body_text.clone()))
                        .into_any_element()
                }
                None => v_flex()
                    .flex_1()
                    .h_full()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted_fg)
                            .child("请选择一封邮件查看正文"),
                    )
                    .into_any_element(),
            },
        }
    }
}

impl Render for HylkiApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bg = cx.theme().background;
        let fg = cx.theme().foreground;
        let border = cx.theme().border;
        let muted_fg = cx.theme().muted_foreground;
        let danger = cx.theme().danger;

        v_flex()
            .size_full()
            .bg(bg)
            .text_color(fg)
            .child(self.render_toolbar(cx))
            .when_some(self.error_banner.as_ref(), |el, err| {
                el.child(
                    h_flex()
                        .px_3()
                        .py_1()
                        .border_b_1()
                        .border_color(border)
                        .justify_between()
                        .items_center()
                        .child(div().text_xs().text_color(danger).child(err.clone()))
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
                    .child(self.render_sidebar(cx))
                    .child(self.render_message_list(cx))
                    .child(self.render_right_pane(cx)),
            )
            .child(
                h_flex()
                    .px_3()
                    .py_1()
                    .border_t_1()
                    .border_color(border)
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

fn sender_initial(sender: &str) -> String {
    sender
        .trim()
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().collect::<String>())
        .unwrap_or_else(|| "?".to_string())
}

fn main() {
    let demo_mode = std::env::args().any(|arg| arg == "--demo");

    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1280.0), px(800.0)),
                cx,
            ))),
            ..Default::default()
        };
        cx.open_window(options, move |window, cx| {
            cx.new(|cx| HylkiApp::new(demo_mode, window, cx))
        })
        .expect("failed to open Hylki Windows window");
    });
}
