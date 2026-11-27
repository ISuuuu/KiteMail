//! Mail account management, provider presets, and Microsoft OAuth2 IPC handlers.

use std::sync::mpsc;
use std::time::Duration;

use kitemail_mail_core::config::{
    self, AccountConfig, AuthBackend, SecurityMode, SpecialFoldersConfig,
};
use kitemail_mail_core::{secrets, MailRequest};
use tauri::State;

use crate::commands::AppState;
use crate::compose::{parse_optional_positive_u32, parse_positive_u32};
use crate::dto::{
    AccountConfigDto, AccountEvent, AccountEventKindDto, AccountSummaryDto, AppMode,
    AuthBackendDto, DeleteAccountInput, DeleteAccountOutput, IpcError, ListAccountsOutput,
    MailChangedEvent, ProviderPresetDto, SaveAccountInput, SaveAccountOutput, SecurityModeDto,
    SetAccountEnabledInput, SetAccountEnabledOutput, StartMicrosoftOAuthInput,
    StartMicrosoftOAuthOutput, VerifyAccountOutput,
};
use crate::mail_session::MailSession;

const ACCOUNT_VERIFY_TIMEOUT: Duration = Duration::from_secs(35);
const ACCOUNT_OP_TIMEOUT: Duration = Duration::from_secs(10);

struct BuiltinPreset {
    id: &'static str,
    name: &'static str,
    domains: &'static [&'static str],
    auth_backend: AuthBackendDto,
    imap_host: &'static str,
    imap_port: u16,
    imap_security: SecurityModeDto,
    smtp_host: &'static str,
    smtp_port: u16,
    smtp_security: SecurityModeDto,
    note: &'static str,
}

const BUILTIN_PRESETS: &[BuiltinPreset] = &[
    BuiltinPreset {
        id: "microsoft",
        name: "Microsoft 365 / Outlook",
        domains: &[
            "outlook.com",
            "hotmail.com",
            "live.com",
            "msn.com",
            "office365.com",
        ],
        auth_backend: AuthBackendDto::MicrosoftGraphOAuth2,
        imap_host: "outlook.office365.com",
        imap_port: 993,
        imap_security: SecurityModeDto::Tls,
        smtp_host: "smtp.office365.com",
        smtp_port: 587,
        smtp_security: SecurityModeDto::StartTls,
        note: "通过系统浏览器完成 Microsoft OAuth2 登录授权（支持 Outlook.com / Hotmail / Live / Microsoft 365）。",
    },
    BuiltinPreset {
        id: "qq",
        name: "QQ 邮箱",
        domains: &["qq.com", "foxmail.com", "vip.qq.com"],
        auth_backend: AuthBackendDto::Password,
        imap_host: "imap.qq.com",
        imap_port: 993,
        imap_security: SecurityModeDto::Tls,
        smtp_host: "smtp.qq.com",
        smtp_port: 465,
        smtp_security: SecurityModeDto::Tls,
        note: "QQ 邮箱需在网页版「设置 → 账号」开启 IMAP/SMTP 服务，并填写生成的 16 位授权码（非 QQ 登录密码）。",
    },
    BuiltinPreset {
        id: "netease163",
        name: "网易 163 邮箱",
        domains: &["163.com", "vip.163.com"],
        auth_backend: AuthBackendDto::Password,
        imap_host: "imap.163.com",
        imap_port: 993,
        imap_security: SecurityModeDto::Tls,
        smtp_host: "smtp.163.com",
        smtp_port: 465,
        smtp_security: SecurityModeDto::Tls,
        note: "网易 163 需在网页版「设置 → POP3/SMTP/IMAP」开启 IMAP/SMTP 服务，并填写客户端授权密码。",
    },
    BuiltinPreset {
        id: "netease126",
        name: "网易 126 邮箱",
        domains: &["126.com", "yeah.net", "vip.126.com"],
        auth_backend: AuthBackendDto::Password,
        imap_host: "imap.126.com",
        imap_port: 993,
        imap_security: SecurityModeDto::Tls,
        smtp_host: "smtp.126.com",
        smtp_port: 465,
        smtp_security: SecurityModeDto::Tls,
        note: "网易 126 需在网页版「设置 → POP3/SMTP/IMAP」开启 IMAP/SMTP 服务，并填写客户端授权密码。",
    },
    BuiltinPreset {
        id: "exmail",
        name: "腾讯企业邮",
        domains: &["exmail.qq.com"],
        auth_backend: AuthBackendDto::Password,
        imap_host: "imap.exmail.qq.com",
        imap_port: 993,
        imap_security: SecurityModeDto::Tls,
        smtp_host: "smtp.exmail.qq.com",
        smtp_port: 465,
        smtp_security: SecurityModeDto::Tls,
        note: "若管理员开启了安全登录或微信绑定，请在腾讯企业邮网页端「设置 → 账号安全」生成客户端专用密码。",
    },
    BuiltinPreset {
        id: "aliyun_qiye",
        name: "阿里企业邮",
        domains: &["qiye.aliyun.com", "mxhichina.com"],
        auth_backend: AuthBackendDto::Password,
        imap_host: "imap.qiye.aliyun.com",
        imap_port: 993,
        imap_security: SecurityModeDto::Tls,
        smtp_host: "smtp.qiye.aliyun.com",
        smtp_port: 465,
        smtp_security: SecurityModeDto::Tls,
        note: "请使用完整企业邮箱地址作为用户名；如开启三方客户端安全密码，请在阿里企业邮网页端生成专用密码。",
    },
    BuiltinPreset {
        id: "gmail",
        name: "Gmail",
        domains: &["gmail.com", "googlemail.com"],
        auth_backend: AuthBackendDto::Password,
        imap_host: "imap.gmail.com",
        imap_port: 993,
        imap_security: SecurityModeDto::Tls,
        smtp_host: "smtp.gmail.com",
        smtp_port: 465,
        smtp_security: SecurityModeDto::Tls,
        note: "Gmail 需在 Google 账号开启「两步验证」，并在安全性设置中生成 16 位「应用专用密码 (App Password)」。",
    },
    BuiltinPreset {
        id: "icloud",
        name: "iCloud",
        domains: &["icloud.com", "me.com", "mac.com"],
        auth_backend: AuthBackendDto::Password,
        imap_host: "imap.mail.me.com",
        imap_port: 993,
        imap_security: SecurityModeDto::Tls,
        smtp_host: "smtp.mail.me.com",
        smtp_port: 587,
        smtp_security: SecurityModeDto::StartTls,
        note: "iCloud 需在 account.apple.com 生成「App 专用密码」后填写，不支持直接使用 Apple ID 登录密码。",
    },
    BuiltinPreset {
        id: "yahoo",
        name: "Yahoo Mail",
        domains: &["yahoo.com", "yahoo.co.jp", "yahoo.co.uk"],
        auth_backend: AuthBackendDto::Password,
        imap_host: "imap.mail.yahoo.com",
        imap_port: 993,
        imap_security: SecurityModeDto::Tls,
        smtp_host: "smtp.mail.yahoo.com",
        smtp_port: 465,
        smtp_security: SecurityModeDto::Tls,
        note: "Yahoo 需在账号安全设置中开启「生成应用密码」并使用应用专用密码登录。",
    },
    BuiltinPreset {
        id: "fastmail",
        name: "Fastmail",
        domains: &["fastmail.com", "fastmail.fm"],
        auth_backend: AuthBackendDto::Password,
        imap_host: "imap.fastmail.com",
        imap_port: 993,
        imap_security: SecurityModeDto::Tls,
        smtp_host: "smtp.fastmail.com",
        smtp_port: 465,
        smtp_security: SecurityModeDto::Tls,
        note: "Fastmail 需在「设置 → 隐私与安全 → 第三方应用密码」生成专用密码。",
    },
    BuiltinPreset {
        id: "zoho",
        name: "Zoho Mail",
        domains: &["zoho.com", "zoho.com.cn", "zoho.eu"],
        auth_backend: AuthBackendDto::Password,
        imap_host: "imap.zoho.com",
        imap_port: 993,
        imap_security: SecurityModeDto::Tls,
        smtp_host: "smtp.zoho.com",
        smtp_port: 465,
        smtp_security: SecurityModeDto::Tls,
        note: "Zoho 建议启用双因素认证并使用应用专用密码；部分地区域名对应 imap.zoho.com.cn / imap.zoho.eu。",
    },
    BuiltinPreset {
        id: "manual",
        name: "通用 IMAP / SMTP",
        domains: &[],
        auth_backend: AuthBackendDto::Password,
        imap_host: "",
        imap_port: 993,
        imap_security: SecurityModeDto::Tls,
        smtp_host: "",
        smtp_port: 465,
        smtp_security: SecurityModeDto::Tls,
        note: "手动填写邮件服务商提供的 IMAP 与 SMTP 服务器地址和端口。",
    },
];

pub fn provider_presets() -> Vec<ProviderPresetDto> {
    BUILTIN_PRESETS
        .iter()
        .map(|p| ProviderPresetDto {
            id: p.id.to_string(),
            name: p.name.to_string(),
            domains: p.domains.iter().map(|d| (*d).to_string()).collect(),
            auth_backend: p.auth_backend,
            imap_host: p.imap_host.to_string(),
            imap_port: p.imap_port,
            imap_security: p.imap_security,
            smtp_host: p.smtp_host.to_string(),
            smtp_port: p.smtp_port,
            smtp_security: p.smtp_security,
            note: p.note.to_string(),
        })
        .collect()
}

fn is_valid_email(email: &str) -> bool {
    let trimmed = email.trim();
    if trimmed.is_empty() || trimmed.contains(char::is_whitespace) {
        return false;
    }
    let Some((local, domain)) = trimmed.split_once('@') else {
        return false;
    };
    !local.is_empty() && !domain.is_empty() && domain.contains('.') && !domain.contains('@')
}

fn validate_account_input(
    input: &SaveAccountInput,
    require_password_for_new: bool,
    is_new: bool,
) -> Result<(), IpcError> {
    if !is_valid_email(&input.email) {
        return Err(IpcError::invalid_input("请输入有效的邮箱地址", None));
    }
    if input.auth_backend == AuthBackendDto::Password {
        if input.imap_host.trim().is_empty() {
            return Err(IpcError::invalid_input("请填写 IMAP 服务器地址", None));
        }
        if input.smtp_host.trim().is_empty() {
            return Err(IpcError::invalid_input("请填写 SMTP 服务器地址", None));
        }
        if input.imap_port == 0 {
            return Err(IpcError::invalid_input(
                "IMAP 端口需为 1–65535 的有效整数",
                None,
            ));
        }
        if input.smtp_port == 0 {
            return Err(IpcError::invalid_input(
                "SMTP 端口需为 1–65535 的有效整数",
                None,
            ));
        }
        let imap_pwd_empty = input.imap_password.as_deref().is_none_or(str::is_empty);
        if require_password_for_new && is_new && imap_pwd_empty {
            return Err(IpcError::invalid_input(
                "新增 IMAP 账号时请填写密码或客户端授权码",
                None,
            ));
        }
        if input.smtp_separate_credentials
            && is_new
            && require_password_for_new
            && input.smtp_password.as_deref().is_none_or(str::is_empty)
        {
            return Err(IpcError::invalid_input(
                "启用独立 SMTP 凭据时请填写 SMTP 密码或授权码",
                None,
            ));
        }
    }
    Ok(())
}

fn build_account_config(
    input: &SaveAccountInput,
    assigned_id: u32,
    existing: Option<&AccountConfig>,
) -> AccountConfig {
    let email = input.email.trim().to_string();
    let label = input
        .label
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != email)
        .map(ToOwned::to_owned);
    let imap_username = {
        let u = input.imap_username.trim();
        if u.is_empty() {
            email.clone()
        } else {
            u.to_string()
        }
    };
    let smtp_username = input.smtp_username.trim().to_string();
    let mut cfg = existing.cloned().unwrap_or_default();
    cfg.id = assigned_id;
    cfg.auth_backend = AuthBackend::from(input.auth_backend);
    cfg.oauth_client_id = input
        .oauth_client_id
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .to_string();
    cfg.display_name = input.display_name.trim().to_string();
    cfg.email = email.clone();
    cfg.label = label;
    cfg.enabled = input
        .enabled
        .unwrap_or(existing.map(|e| e.enabled).unwrap_or(true));
    cfg.in_unified = input
        .in_unified
        .unwrap_or(existing.map(|e| e.in_unified).unwrap_or(true));

    if cfg.auth_backend == AuthBackend::MicrosoftGraphOAuth2 {
        if cfg.imap_host.trim().is_empty() {
            cfg.imap_host = if input.imap_host.trim().is_empty() {
                "outlook.office365.com".to_string()
            } else {
                input.imap_host.trim().to_string()
            };
        }
        if cfg.imap_port == 0 {
            cfg.imap_port = if input.imap_port == 0 {
                993
            } else {
                input.imap_port
            };
        }
        cfg.imap_security = if input.imap_host.trim().is_empty() {
            SecurityMode::Tls
        } else {
            SecurityMode::from(input.imap_security)
        };
        cfg.imap_username = imap_username;
        if cfg.smtp_host.trim().is_empty() {
            cfg.smtp_host = if input.smtp_host.trim().is_empty() {
                "smtp.office365.com".to_string()
            } else {
                input.smtp_host.trim().to_string()
            };
        }
        if cfg.smtp_port == 0 {
            cfg.smtp_port = if input.smtp_port == 0 {
                587
            } else {
                input.smtp_port
            };
        }
        cfg.smtp_security = if input.smtp_host.trim().is_empty() {
            SecurityMode::StartTls
        } else {
            SecurityMode::from(input.smtp_security)
        };
        cfg.smtp_username = if smtp_username.is_empty() {
            email.clone()
        } else {
            smtp_username
        };
        cfg.smtp_separate_credentials = false;
    } else {
        cfg.imap_host = input.imap_host.trim().to_string();
        cfg.imap_port = input.imap_port;
        cfg.imap_security = SecurityMode::from(input.imap_security);
        cfg.imap_username = imap_username;
        cfg.smtp_host = input.smtp_host.trim().to_string();
        cfg.smtp_port = input.smtp_port;
        cfg.smtp_security = SecurityMode::from(input.smtp_security);
        cfg.smtp_username = smtp_username;
        cfg.smtp_separate_credentials = input.smtp_separate_credentials;
    }

    cfg.signature = input.signature.clone();
    if existing.is_none() {
        cfg.special_folders = SpecialFoldersConfig {
            server_saves_sent: config::default_server_saves_sent_for_email(&email),
            ..Default::default()
        };
    }
    cfg
}

fn live_account_has_password(
    data_dir: &std::path::Path,
    cfg: &AccountConfig,
    known_map: &std::collections::HashMap<u32, bool>,
) -> bool {
    if let Some(&known) = known_map.get(&cfg.id) {
        if known {
            return true;
        }
    }
    match cfg.auth_backend {
        AuthBackend::Password => secrets::load_account_secrets_for_config_in(data_dir, cfg).is_ok(),
        AuthBackend::MicrosoftGraphOAuth2 => {
            secrets::load_bound_oauth_token_in(data_dir, cfg).is_ok()
        }
    }
}

impl MailSession {
    pub fn list_accounts(&self) -> Result<ListAccountsOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        let accounts = match self.mode {
            AppMode::Demo => {
                let demo = self.demo.lock().expect("demo mutex poisoned");
                demo.account_configs
                    .iter()
                    .map(|cfg| {
                        let has_pwd = demo
                            .account_has_password
                            .get(&cfg.id)
                            .copied()
                            .unwrap_or(true);
                        AccountConfigDto::from_config(cfg, has_pwd)
                    })
                    .collect()
            }
            AppMode::Live => {
                let (data_dir, configs, known) = {
                    let live = self.live.lock().expect("live mutex poisoned");
                    (
                        live.data_dir.clone(),
                        live.account_configs.clone(),
                        live.known_has_password.clone(),
                    )
                };
                configs
                    .iter()
                    .map(|cfg| {
                        let has_pwd = live_account_has_password(&data_dir, cfg, &known);
                        AccountConfigDto::from_config(cfg, has_pwd)
                    })
                    .collect()
            }
        };
        Ok(ListAccountsOutput {
            accounts,
            presets: provider_presets(),
            state_version: self.state_version(),
        })
    }

    pub fn verify_account(
        &self,
        input: &SaveAccountInput,
    ) -> Result<VerifyAccountOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        let editing_id = parse_optional_positive_u32(input.id.as_deref(), "id")?;
        validate_account_input(input, editing_id.is_none(), editing_id.is_none())?;

        match self.mode {
            AppMode::Demo => {
                let msg = if input.auth_backend == AuthBackendDto::MicrosoftGraphOAuth2 {
                    "当前账号使用 OAuth2 浏览器授权，无需测试 IMAP/SMTP 密码".to_string()
                } else {
                    "IMAP 与 SMTP 验证均通过（演示模式）".to_string()
                };
                let ver = self.bump_state_version();
                self.emitter.emit_account_event(AccountEvent {
                    state_version: ver,
                    kind: AccountEventKindDto::Verified,
                    ok: true,
                    account_id: editing_id.map(|id| id.to_string()),
                    email: Some(input.email.trim().to_string()),
                    message: msg.clone(),
                });
                Ok(VerifyAccountOutput {
                    ok: true,
                    message: msg,
                })
            }
            AppMode::Live => {
                let (worker, async_worker, rx_opt, cfg) = {
                    let mut live = self.live.lock().expect("live mutex poisoned");
                    let existing =
                        editing_id.and_then(|id| live.account_configs.iter().find(|a| a.id == id));
                    let assigned_id = editing_id
                        .unwrap_or_else(|| config::next_account_id(&live.account_configs));
                    let cfg = build_account_config(input, assigned_id, existing);
                    let worker = live.worker.clone();
                    let is_async = worker.as_ref().is_some_and(|w| w.is_async_worker());
                    let rx_opt = if is_async {
                        let (tx, rx) = mpsc::channel();
                        live.waiters.verify_account.push_back(tx);
                        Some(rx)
                    } else {
                        None
                    };
                    (worker, is_async, rx_opt, cfg)
                };

                let Some(w) = worker else {
                    return Err(IpcError::unavailable("邮件后台服务尚未就绪", None));
                };

                let imap_password = input.imap_password.clone().unwrap_or_default();
                let smtp_password = input
                    .smtp_password
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .map(ToOwned::to_owned);
                w.send(MailRequest::VerifyAccount {
                    config: cfg,
                    imap_password,
                    smtp_password,
                });

                if async_worker {
                    let rx = rx_opt.expect("async verify waiter missing");
                    return rx.recv_timeout(ACCOUNT_VERIFY_TIMEOUT).map_err(|_| {
                        IpcError::unavailable("连接验证超时，请检查服务器地址与端口", None)
                    });
                }

                let msg = if input.auth_backend == AuthBackendDto::MicrosoftGraphOAuth2 {
                    "当前账号使用 OAuth2 浏览器授权，无需测试 IMAP/SMTP 密码".to_string()
                } else {
                    "已提交 IMAP 与 SMTP 连接验证".to_string()
                };
                Ok(VerifyAccountOutput {
                    ok: true,
                    message: msg,
                })
            }
        }
    }

    pub fn save_account(&self, input: &SaveAccountInput) -> Result<SaveAccountOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        let editing_id = parse_optional_positive_u32(input.id.as_deref(), "id")?;
        validate_account_input(input, true, editing_id.is_none())?;

        match self.mode {
            AppMode::Demo => {
                let (cfg, has_pwd) = {
                    let mut demo = self.demo.lock().expect("demo mutex poisoned");
                    let existing = editing_id
                        .and_then(|id| demo.account_configs.iter().find(|a| a.id == id))
                        .cloned();
                    if editing_id.is_some() && existing.is_none() {
                        return Err(IpcError::invalid_input("要编辑的账号不存在", None));
                    }
                    let assigned_id = if let Some(id) = editing_id {
                        id
                    } else {
                        let id = demo.next_account_id;
                        demo.next_account_id = demo.next_account_id.saturating_add(1);
                        id
                    };
                    let cfg = build_account_config(input, assigned_id, existing.as_ref());
                    let had_pwd = demo
                        .account_has_password
                        .get(&assigned_id)
                        .copied()
                        .unwrap_or(false);
                    let provided_pwd = input
                        .imap_password
                        .as_deref()
                        .is_some_and(|s| !s.is_empty());
                    let has_pwd = had_pwd
                        || provided_pwd
                        || cfg.auth_backend == AuthBackend::MicrosoftGraphOAuth2;
                    demo.account_has_password.insert(assigned_id, has_pwd);
                    if let Some(slot) = demo
                        .account_configs
                        .iter_mut()
                        .find(|a| a.id == assigned_id)
                    {
                        *slot = cfg.clone();
                    } else {
                        demo.account_configs.push(cfg.clone());
                    }
                    demo.sync_account_summaries();
                    (cfg, has_pwd)
                };

                let ver = self.bump_state_version();
                self.emitter.emit_account_event(AccountEvent {
                    state_version: ver,
                    kind: AccountEventKindDto::Saved,
                    ok: true,
                    account_id: Some(cfg.id.to_string()),
                    email: Some(cfg.email.clone()),
                    message: "账号配置已保存".to_string(),
                });
                self.emitter.emit_mail_changed(MailChangedEvent {
                    state_version: ver,
                    reason: "accountsUpdated".to_string(),
                    account_id: Some(cfg.id),
                    folder_id: None,
                    message_id: None,
                });
                Ok(SaveAccountOutput {
                    account: AccountConfigDto::from_config(&cfg, has_pwd),
                    state_version: ver,
                })
            }
            AppMode::Live => {
                let (worker, async_worker, rx_opt, cfg, data_dir, has_pwd_hint) = {
                    let mut live = self.live.lock().expect("live mutex poisoned");
                    let existing = editing_id
                        .and_then(|id| live.account_configs.iter().find(|a| a.id == id))
                        .cloned();
                    if editing_id.is_some() && existing.is_none() {
                        return Err(IpcError::invalid_input("要编辑的账号不存在", None));
                    }
                    let assigned_id = editing_id
                        .unwrap_or_else(|| config::next_account_id(&live.account_configs));
                    let cfg = build_account_config(input, assigned_id, existing.as_ref());
                    let provided_pwd = input
                        .imap_password
                        .as_deref()
                        .is_some_and(|s| !s.is_empty());
                    let prev_known = live
                        .known_has_password
                        .get(&assigned_id)
                        .copied()
                        .unwrap_or(false);
                    let has_pwd_hint = provided_pwd
                        || prev_known
                        || cfg.auth_backend == AuthBackend::MicrosoftGraphOAuth2;

                    let worker = live.worker.clone();
                    let is_async = worker.as_ref().is_some_and(|w| w.is_async_worker());
                    let rx_opt = if is_async {
                        let (tx, rx) = mpsc::channel();
                        live.waiters.save_account.insert(assigned_id, tx);
                        Some(rx)
                    } else {
                        None
                    };
                    (
                        worker,
                        is_async,
                        rx_opt,
                        cfg,
                        live.data_dir.clone(),
                        has_pwd_hint,
                    )
                };

                let Some(w) = worker else {
                    return Err(IpcError::unavailable("邮件后台服务尚未就绪", None));
                };

                let imap_password = input.imap_password.clone().unwrap_or_default();
                let smtp_password = input
                    .smtp_password
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .map(ToOwned::to_owned);
                let cfg_id = cfg.id;
                w.send(MailRequest::SaveAndSyncAccount {
                    config: cfg.clone(),
                    imap_password,
                    smtp_password,
                });

                if async_worker {
                    let rx = rx_opt.expect("async save waiter missing");
                    let res = rx.recv_timeout(ACCOUNT_OP_TIMEOUT).map_err(|_| {
                        let mut live = self.live.lock().expect("live mutex poisoned");
                        live.waiters.save_account.remove(&cfg_id);
                        IpcError::unavailable("保存账号超时，请稍后重试", None)
                    })?;
                    let saved = res.map_err(|msg| IpcError::invalid_input(msg, None))?;
                    let known = {
                        let mut live = self.live.lock().expect("live mutex poisoned");
                        if has_pwd_hint {
                            live.known_has_password.insert(saved.id, true);
                        }
                        live.known_has_password.clone()
                    };
                    let has_pwd = live_account_has_password(&data_dir, &saved, &known);
                    return Ok(SaveAccountOutput {
                        account: AccountConfigDto::from_config(&saved, has_pwd),
                        state_version: self.state_version(),
                    });
                }

                // Mock/recording worker mode: update in-memory configs deterministically.
                {
                    let mut live = self.live.lock().expect("live mutex poisoned");
                    if has_pwd_hint {
                        live.known_has_password.insert(cfg.id, true);
                    }
                    if let Some(slot) = live.account_configs.iter_mut().find(|a| a.id == cfg.id) {
                        *slot = cfg.clone();
                    } else {
                        live.account_configs.push(cfg.clone());
                    }
                    live.accounts = live
                        .account_configs
                        .iter()
                        .map(AccountSummaryDto::from)
                        .collect();
                }
                let ver = self.bump_state_version();
                self.emitter.emit_account_event(AccountEvent {
                    state_version: ver,
                    kind: AccountEventKindDto::Saved,
                    ok: true,
                    account_id: Some(cfg.id.to_string()),
                    email: Some(cfg.email.clone()),
                    message: "账号配置已保存".to_string(),
                });
                self.emitter.emit_mail_changed(MailChangedEvent {
                    state_version: ver,
                    reason: "accountsUpdated".to_string(),
                    account_id: Some(cfg.id),
                    folder_id: None,
                    message_id: None,
                });
                Ok(SaveAccountOutput {
                    account: AccountConfigDto::from_config(&cfg, has_pwd_hint),
                    state_version: ver,
                })
            }
        }
    }

    pub fn delete_account(
        &self,
        input: &DeleteAccountInput,
    ) -> Result<DeleteAccountOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        let account_id = parse_positive_u32(&input.account_id, "accountId")?;

        match self.mode {
            AppMode::Demo => {
                let removed_email = {
                    let mut demo = self.demo.lock().expect("demo mutex poisoned");
                    let email = demo
                        .account_configs
                        .iter()
                        .find(|a| a.id == account_id)
                        .map(|a| a.email.clone());
                    demo.account_configs.retain(|a| a.id != account_id);
                    demo.account_has_password.remove(&account_id);
                    demo.local_drafts.retain(|d| d.account_id != account_id);
                    demo.outbox.retain(|o| o.item.account_id != account_id);
                    demo.sync_account_summaries();
                    email
                };
                let Some(email) = removed_email else {
                    return Ok(DeleteAccountOutput {
                        deleted: false,
                        state_version: self.state_version(),
                    });
                };
                let ver = self.bump_state_version();
                self.emitter.emit_account_event(AccountEvent {
                    state_version: ver,
                    kind: AccountEventKindDto::Deleted,
                    ok: true,
                    account_id: Some(account_id.to_string()),
                    email: Some(email),
                    message: "账号已删除".to_string(),
                });
                self.emitter.emit_mail_changed(MailChangedEvent {
                    state_version: ver,
                    reason: "accountsUpdated".to_string(),
                    account_id: Some(account_id),
                    folder_id: None,
                    message_id: None,
                });
                Ok(DeleteAccountOutput {
                    deleted: true,
                    state_version: ver,
                })
            }
            AppMode::Live => {
                let (worker, async_worker, rx_opt, existed, email) = {
                    let mut live = self.live.lock().expect("live mutex poisoned");
                    let found = live
                        .account_configs
                        .iter()
                        .find(|a| a.id == account_id)
                        .map(|a| a.email.clone());
                    let Some(email) = found else {
                        return Ok(DeleteAccountOutput {
                            deleted: false,
                            state_version: self.state_version(),
                        });
                    };
                    let worker = live.worker.clone();
                    let is_async = worker.as_ref().is_some_and(|w| w.is_async_worker());
                    let rx_opt = if is_async {
                        let (tx, rx) = mpsc::channel();
                        live.waiters.delete_account.insert(account_id, tx);
                        Some(rx)
                    } else {
                        None
                    };
                    (worker, is_async, rx_opt, true, email)
                };

                if !existed {
                    return Ok(DeleteAccountOutput {
                        deleted: false,
                        state_version: self.state_version(),
                    });
                }
                let Some(w) = worker else {
                    return Err(IpcError::unavailable("邮件后台服务尚未就绪", None));
                };
                w.send(MailRequest::DeleteAccount { account_id });

                if async_worker {
                    let rx = rx_opt.expect("async delete waiter missing");
                    rx.recv_timeout(ACCOUNT_OP_TIMEOUT).map_err(|_| {
                        let mut live = self.live.lock().expect("live mutex poisoned");
                        live.waiters.delete_account.remove(&account_id);
                        IpcError::unavailable("删除账号超时，请稍后重试", None)
                    })?;
                    return Ok(DeleteAccountOutput {
                        deleted: true,
                        state_version: self.state_version(),
                    });
                }

                {
                    let mut live = self.live.lock().expect("live mutex poisoned");
                    live.account_configs.retain(|a| a.id != account_id);
                    live.accounts = live
                        .account_configs
                        .iter()
                        .map(AccountSummaryDto::from)
                        .collect();
                    live.folders_by_account.remove(&account_id);
                    live.outbox_cache.remove(&account_id);
                    live.known_has_password.remove(&account_id);
                }
                let ver = self.bump_state_version();
                self.emitter.emit_account_event(AccountEvent {
                    state_version: ver,
                    kind: AccountEventKindDto::Deleted,
                    ok: true,
                    account_id: Some(account_id.to_string()),
                    email: Some(email),
                    message: "账号已删除".to_string(),
                });
                self.emitter.emit_mail_changed(MailChangedEvent {
                    state_version: ver,
                    reason: "accountsUpdated".to_string(),
                    account_id: Some(account_id),
                    folder_id: None,
                    message_id: None,
                });
                Ok(DeleteAccountOutput {
                    deleted: true,
                    state_version: ver,
                })
            }
        }
    }

    pub fn set_account_enabled(
        &self,
        input: &SetAccountEnabledInput,
    ) -> Result<SetAccountEnabledOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        let account_id = parse_positive_u32(&input.account_id, "accountId")?;

        match self.mode {
            AppMode::Demo => {
                let email = {
                    let mut demo = self.demo.lock().expect("demo mutex poisoned");
                    let Some(acc) = demo.account_configs.iter_mut().find(|a| a.id == account_id)
                    else {
                        return Err(IpcError::invalid_input(
                            format!("未找到账号 #{account_id}"),
                            None,
                        ));
                    };
                    acc.enabled = input.enabled;
                    let email = acc.email.clone();
                    demo.sync_account_summaries();
                    email
                };
                let ver = self.bump_state_version();
                self.emitter.emit_account_event(AccountEvent {
                    state_version: ver,
                    kind: AccountEventKindDto::Updated,
                    ok: true,
                    account_id: Some(account_id.to_string()),
                    email: Some(email),
                    message: if input.enabled {
                        "账号已启用".to_string()
                    } else {
                        "账号已停用".to_string()
                    },
                });
                self.emitter.emit_mail_changed(MailChangedEvent {
                    state_version: ver,
                    reason: "accountsUpdated".to_string(),
                    account_id: Some(account_id),
                    folder_id: None,
                    message_id: None,
                });
                Ok(SetAccountEnabledOutput {
                    account_id: account_id.to_string(),
                    enabled: input.enabled,
                    state_version: ver,
                })
            }
            AppMode::Live => {
                let (worker, async_worker, rx_opt, email) = {
                    let mut live = self.live.lock().expect("live mutex poisoned");
                    let Some(acc) = live.account_configs.iter().find(|a| a.id == account_id) else {
                        return Err(IpcError::invalid_input(
                            format!("未找到账号 #{account_id}"),
                            None,
                        ));
                    };
                    let email = acc.email.clone();
                    let worker = live.worker.clone();
                    let is_async = worker.as_ref().is_some_and(|w| w.is_async_worker());
                    let rx_opt = if is_async {
                        let (tx, rx) = mpsc::channel();
                        live.waiters.set_account_enabled.push_back(tx);
                        Some(rx)
                    } else {
                        None
                    };
                    (worker, is_async, rx_opt, email)
                };

                let Some(w) = worker else {
                    return Err(IpcError::unavailable("邮件后台服务尚未就绪", None));
                };
                w.send(MailRequest::SetAccountEnabled {
                    account_id,
                    enabled: input.enabled,
                });

                if async_worker {
                    let rx = rx_opt.expect("async enable waiter missing");
                    rx.recv_timeout(ACCOUNT_OP_TIMEOUT)
                        .map_err(|_| IpcError::unavailable("切换账号状态超时，请稍后重试", None))?;
                    return Ok(SetAccountEnabledOutput {
                        account_id: account_id.to_string(),
                        enabled: input.enabled,
                        state_version: self.state_version(),
                    });
                }

                {
                    let mut live = self.live.lock().expect("live mutex poisoned");
                    if let Some(acc) = live.account_configs.iter_mut().find(|a| a.id == account_id)
                    {
                        acc.enabled = input.enabled;
                    }
                    live.accounts = live
                        .account_configs
                        .iter()
                        .map(AccountSummaryDto::from)
                        .collect();
                }
                let ver = self.bump_state_version();
                self.emitter.emit_account_event(AccountEvent {
                    state_version: ver,
                    kind: AccountEventKindDto::Updated,
                    ok: true,
                    account_id: Some(account_id.to_string()),
                    email: Some(email),
                    message: if input.enabled {
                        "账号已启用".to_string()
                    } else {
                        "账号已停用".to_string()
                    },
                });
                self.emitter.emit_mail_changed(MailChangedEvent {
                    state_version: ver,
                    reason: "accountsUpdated".to_string(),
                    account_id: Some(account_id),
                    folder_id: None,
                    message_id: None,
                });
                Ok(SetAccountEnabledOutput {
                    account_id: account_id.to_string(),
                    enabled: input.enabled,
                    state_version: ver,
                })
            }
        }
    }

    pub fn start_microsoft_oauth(
        &self,
        input: &StartMicrosoftOAuthInput,
    ) -> Result<StartMicrosoftOAuthOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        let editing_account_id =
            parse_optional_positive_u32(input.editing_account_id.as_deref(), "editingAccountId")?;
        let custom_client_id = input
            .custom_client_id
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .to_string();
        let login_hint_email = input
            .login_hint_email
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned);

        match self.mode {
            AppMode::Demo => {
                let (kind, acc_id, email, message) = {
                    let mut demo = self.demo.lock().expect("demo mutex poisoned");
                    if let Some(id) = editing_account_id {
                        let Some(acc) = demo.account_configs.iter().find(|a| a.id == id) else {
                            return Err(IpcError::invalid_input(format!("未找到账号 #{id}"), None));
                        };
                        (
                            AccountEventKindDto::OauthReauthorized,
                            Some(id.to_string()),
                            Some(acc.email.clone()),
                            "演示模式已完成 Microsoft OAuth2 重新授权".to_string(),
                        )
                    } else {
                        let hint_email = login_hint_email
                            .unwrap_or_else(|| "demo.outlook@outlook.com".to_string());
                        let new_id = demo.next_account_id;
                        demo.next_account_id = demo.next_account_id.saturating_add(1);
                        let cfg = AccountConfig {
                            id: new_id,
                            auth_backend: AuthBackend::MicrosoftGraphOAuth2,
                            oauth_client_id: custom_client_id,
                            display_name: "Outlook 演示账号".to_string(),
                            email: hint_email.clone(),
                            imap_host: "outlook.office365.com".to_string(),
                            imap_port: 993,
                            imap_security: SecurityMode::Tls,
                            imap_username: hint_email.clone(),
                            smtp_host: "smtp.office365.com".to_string(),
                            smtp_port: 587,
                            smtp_security: SecurityMode::StartTls,
                            smtp_username: hint_email.clone(),
                            special_folders: SpecialFoldersConfig {
                                server_saves_sent: true,
                                ..Default::default()
                            },
                            ..AccountConfig::default()
                        };
                        demo.account_configs.push(cfg);
                        demo.account_has_password.insert(new_id, true);
                        demo.sync_account_summaries();
                        (
                            AccountEventKindDto::Saved,
                            Some(new_id.to_string()),
                            Some(hint_email),
                            "演示模式已添加 Microsoft OAuth2 账号".to_string(),
                        )
                    }
                };
                let ver = self.bump_state_version();
                self.emitter.emit_account_event(AccountEvent {
                    state_version: ver,
                    kind,
                    ok: true,
                    account_id: acc_id,
                    email,
                    message: message.clone(),
                });
                self.emitter.emit_mail_changed(MailChangedEvent {
                    state_version: ver,
                    reason: "accountsUpdated".to_string(),
                    account_id: None,
                    folder_id: None,
                    message_id: None,
                });
                Ok(StartMicrosoftOAuthOutput {
                    started: true,
                    message,
                })
            }
            AppMode::Live => {
                let worker = {
                    let live = self.live.lock().expect("live mutex poisoned");
                    if let Some(id) = editing_account_id {
                        if !live.account_configs.iter().any(|a| a.id == id) {
                            return Err(IpcError::invalid_input(format!("未找到账号 #{id}"), None));
                        }
                    }
                    live.worker.clone()
                };
                let Some(w) = worker else {
                    return Err(IpcError::unavailable("邮件后台服务尚未就绪", None));
                };
                w.send(MailRequest::StartMicrosoftOAuth {
                    custom_client_id,
                    login_hint_email,
                    editing_account_id,
                });
                Ok(StartMicrosoftOAuthOutput {
                    started: true,
                    message: "已打开浏览器，请在微软页面完成 Outlook 授权...".to_string(),
                })
            }
        }
    }
}

#[tauri::command]
pub fn list_accounts(state: State<'_, AppState>) -> Result<ListAccountsOutput, IpcError> {
    state.session.list_accounts()
}

#[tauri::command]
pub fn verify_account(
    state: State<'_, AppState>,
    input: SaveAccountInput,
) -> Result<VerifyAccountOutput, IpcError> {
    state.session.verify_account(&input)
}

#[tauri::command]
pub fn save_account(
    state: State<'_, AppState>,
    input: SaveAccountInput,
) -> Result<SaveAccountOutput, IpcError> {
    state.session.save_account(&input)
}

#[tauri::command]
pub fn delete_account(
    state: State<'_, AppState>,
    input: DeleteAccountInput,
) -> Result<DeleteAccountOutput, IpcError> {
    state.session.delete_account(&input)
}

#[tauri::command]
pub fn set_account_enabled(
    state: State<'_, AppState>,
    input: SetAccountEnabledInput,
) -> Result<SetAccountEnabledOutput, IpcError> {
    state.session.set_account_enabled(&input)
}

#[tauri::command]
pub fn start_microsoft_oauth(
    state: State<'_, AppState>,
    input: StartMicrosoftOAuthInput,
) -> Result<StartMicrosoftOAuthOutput, IpcError> {
    state.session.start_microsoft_oauth(&input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::UnlockAppInput;
    use crate::mail_session::tests::{make_live_test_session, make_test_session};
    use kitemail_mail_core::WorkerEvent;

    fn sample_save_input(id: Option<&str>, imap_pwd: Option<&str>) -> SaveAccountInput {
        SaveAccountInput {
            id: id.map(ToOwned::to_owned),
            auth_backend: AuthBackendDto::Password,
            oauth_client_id: None,
            display_name: "测试用户".to_string(),
            email: "user@qq.com".to_string(),
            label: Some("QQ 工作邮箱".to_string()),
            enabled: Some(true),
            in_unified: Some(true),
            imap_host: "imap.qq.com".to_string(),
            imap_port: 993,
            imap_security: SecurityModeDto::Tls,
            imap_username: "user@qq.com".to_string(),
            smtp_host: "smtp.qq.com".to_string(),
            smtp_port: 465,
            smtp_security: SecurityModeDto::Tls,
            smtp_username: "user@qq.com".to_string(),
            smtp_separate_credentials: false,
            signature: "Best regards".to_string(),
            imap_password: imap_pwd.map(ToOwned::to_owned),
            smtp_password: None,
        }
    }

    #[test]
    fn presets_cover_major_providers_and_dto_never_exposes_plaintext_secrets() {
        let presets = provider_presets();
        for required in ["qq", "netease163", "gmail", "microsoft", "icloud", "manual"] {
            assert!(
                presets.iter().any(|p| p.id == required),
                "missing preset {required}"
            );
        }

        let secret = "super-secret-auth-code-9988";
        let smtp_secret = "smtp-secret-7766";
        let input = SaveAccountInput {
            smtp_separate_credentials: true,
            smtp_password: Some(smtp_secret.to_string()),
            ..sample_save_input(None, Some(secret))
        };
        let debug_str = format!("{input:?}");
        assert!(!debug_str.contains(secret));
        assert!(!debug_str.contains(smtp_secret));
        assert!(debug_str.contains("[REDACTED]"));

        let (session, emitter, dir) = make_test_session();
        let saved = session.save_account(&input).expect("demo save account");
        assert!(saved.account.has_password);
        let serialized_dto = serde_json::to_string(&saved).unwrap();
        assert!(!serialized_dto.contains(secret));
        assert!(!serialized_dto.contains(smtp_secret));

        let listed = session.list_accounts().expect("list accounts");
        let serialized_list = serde_json::to_string(&listed).unwrap();
        assert!(!serialized_list.contains(secret));
        assert!(!serialized_list.contains(smtp_secret));

        let events_json = serde_json::to_string(&*emitter.account_events.lock().unwrap()).unwrap();
        assert!(!events_json.contains(secret));
        assert!(!events_json.contains(smtp_secret));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn demo_account_lifecycle_verify_save_toggle_delete_and_oauth() {
        let (session, emitter, dir) = make_test_session();

        let initial = session.list_accounts().unwrap();
        assert_eq!(initial.accounts.len(), 1);
        assert_eq!(initial.accounts[0].id, "1");

        let verify = session
            .verify_account(&sample_save_input(None, Some("auth-code")))
            .unwrap();
        assert!(verify.ok);

        let saved = session
            .save_account(&sample_save_input(None, Some("auth-code")))
            .unwrap();
        assert_eq!(saved.account.id, "2");
        assert_eq!(saved.account.email, "user@qq.com");

        // Editing without password keeps existing hasPassword=true.
        let edited = session
            .save_account(&SaveAccountInput {
                display_name: "新昵称".to_string(),
                ..sample_save_input(Some("2"), None)
            })
            .unwrap();
        assert_eq!(edited.account.display_name, "新昵称");
        assert!(edited.account.has_password);

        let toggled = session
            .set_account_enabled(&SetAccountEnabledInput {
                account_id: "2".to_string(),
                enabled: false,
            })
            .unwrap();
        assert!(!toggled.enabled);

        let oauth = session
            .start_microsoft_oauth(&StartMicrosoftOAuthInput {
                custom_client_id: None,
                login_hint_email: Some("ms@outlook.com".to_string()),
                editing_account_id: None,
            })
            .unwrap();
        assert!(oauth.started);
        assert_eq!(session.list_accounts().unwrap().accounts.len(), 3);

        let deleted = session
            .delete_account(&DeleteAccountInput {
                account_id: "2".to_string(),
            })
            .unwrap();
        assert!(deleted.deleted);
        assert_eq!(session.list_accounts().unwrap().accounts.len(), 2);
        assert!(!emitter.account_events.lock().unwrap().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn live_accounts_commands_enforce_lock_validate_inputs_and_translate_worker_events() {
        let (session, emitter, _, worker, dir) = make_live_test_session(false, Some("1234"));

        session.lock_app(Some("manual"));
        assert_eq!(session.list_accounts().unwrap_err().code, "LOCKED");
        assert_eq!(
            session
                .verify_account(&sample_save_input(None, Some("pw")))
                .unwrap_err()
                .code,
            "LOCKED"
        );
        assert_eq!(
            session
                .save_account(&sample_save_input(None, Some("pw")))
                .unwrap_err()
                .code,
            "LOCKED"
        );
        assert_eq!(
            session
                .delete_account(&DeleteAccountInput {
                    account_id: "1".to_string()
                })
                .unwrap_err()
                .code,
            "LOCKED"
        );
        assert_eq!(
            session
                .set_account_enabled(&SetAccountEnabledInput {
                    account_id: "1".to_string(),
                    enabled: false
                })
                .unwrap_err()
                .code,
            "LOCKED"
        );
        assert_eq!(
            session
                .start_microsoft_oauth(&StartMicrosoftOAuthInput {
                    custom_client_id: None,
                    login_hint_email: None,
                    editing_account_id: None
                })
                .unwrap_err()
                .code,
            "LOCKED"
        );

        session.unlock_app(&UnlockAppInput {
            passcode: "1234".to_string(),
        });

        // Reject invalid email, missing password for new account, and zero port.
        let bad_email = session
            .save_account(&SaveAccountInput {
                email: "not-an-email".to_string(),
                ..sample_save_input(None, Some("pw"))
            })
            .unwrap_err();
        assert_eq!(bad_email.code, "INVALID_INPUT");

        let no_pwd = session
            .save_account(&sample_save_input(None, None))
            .unwrap_err();
        assert_eq!(no_pwd.code, "INVALID_INPUT");

        let zero_port = session
            .verify_account(&SaveAccountInput {
                imap_port: 0,
                ..sample_save_input(None, Some("pw"))
            })
            .unwrap_err();
        assert_eq!(zero_port.code, "INVALID_INPUT");

        // Valid Live dispatches
        session
            .verify_account(&sample_save_input(None, Some("pw")))
            .unwrap();
        let saved = session
            .save_account(&sample_save_input(None, Some("pw")))
            .unwrap();
        assert_eq!(saved.account.id, "2");
        session
            .set_account_enabled(&SetAccountEnabledInput {
                account_id: "2".to_string(),
                enabled: false,
            })
            .unwrap();
        session
            .start_microsoft_oauth(&StartMicrosoftOAuthInput {
                custom_client_id: None,
                login_hint_email: Some("hint@outlook.com".to_string()),
                editing_account_id: Some("1".to_string()),
            })
            .unwrap();
        session
            .delete_account(&DeleteAccountInput {
                account_id: "2".to_string(),
            })
            .unwrap();

        {
            let reqs = worker.requests.lock().unwrap();
            assert!(reqs
                .iter()
                .any(|r| matches!(r, MailRequest::VerifyAccount { .. })));
            assert!(reqs
                .iter()
                .any(|r| matches!(r, MailRequest::SaveAndSyncAccount { .. })));
            assert!(reqs.iter().any(|r| matches!(
                r,
                MailRequest::SetAccountEnabled {
                    account_id: 2,
                    enabled: false
                }
            )));
            assert!(reqs.iter().any(|r| matches!(
                r,
                MailRequest::StartMicrosoftOAuth {
                    editing_account_id: Some(1),
                    ..
                }
            )));
            assert!(reqs
                .iter()
                .any(|r| matches!(r, MailRequest::DeleteAccount { account_id: 2 })));
        }

        session.process_worker_events(vec![
            WorkerEvent::OAuthReauthorized {
                account_id: 1,
                email: "tester@example.com".to_string(),
            },
            WorkerEvent::OAuthRebindRejected {
                account_id: Some(1),
                returned_email: "other@outlook.com".to_string(),
            },
        ]);
        let evs = emitter.account_events.lock().unwrap();
        assert!(evs
            .iter()
            .any(|e| e.kind == AccountEventKindDto::OauthReauthorized && e.ok));
        assert!(evs
            .iter()
            .any(|e| e.kind == AccountEventKindDto::OauthRebindRejected && !e.ok));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn all_wave2_commands_are_registered_in_main_build_and_capabilities() {
        let main_rs = include_str!("main.rs");
        let build_rs = include_str!("../build.rs");
        let cap_json = include_str!("../capabilities/main.json");

        let wave2_cmds = [
            "list_drafts",
            "save_draft",
            "delete_draft",
            "send_mail",
            "undo_send_mail",
            "list_outbox",
            "retry_outbox_item",
            "delete_outbox_item",
            "list_accounts",
            "verify_account",
            "save_account",
            "delete_account",
            "set_account_enabled",
            "start_microsoft_oauth",
        ];

        for cmd in wave2_cmds {
            assert!(
                main_rs.contains(cmd),
                "command `{cmd}` missing in src-tauri/src/main.rs"
            );
            assert!(
                build_rs.contains(&format!("\"{cmd}\"")),
                "command `{cmd}` missing in src-tauri/build.rs"
            );
            let perm = format!("\"allow-{}\"", cmd.replace('_', "-"));
            assert!(
                cap_json.contains(&perm),
                "permission {perm} missing in src-tauri/capabilities/main.json"
            );
        }
    }
}
