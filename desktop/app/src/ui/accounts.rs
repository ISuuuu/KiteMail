//! 账号管理页（方案文档第三节页面 4）：
//! - 顶部已添加账号卡片列表：头像、显示名、邮箱、协议胶囊
//!   （`Microsoft Graph OAuth2` / `IMAP + SMTP`）、最近同步状态、
//!   编辑 / 重新同步（`SyncAccount`）/ 移除账号（`DeleteAccount`，内联确认）；
//!   右上角「+ 添加新账号」切换到空白表单。
//! - 下方添加与编辑表单：保留现有 Outlook OAuth2 登录卡片、8 大服务商预设、
//!   指引横幅、IMAP / SMTP 配置（TLS / STARTTLS 切换、独立 SMTP 凭据）、
//!   「测试连接」与「保存并同步」及实时反馈。

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    div, px, AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement,
    ParentElement, SharedString, Styled, Window,
};
use hylki_mail_core::config::{self, AccountConfig, AuthBackend, SecurityMode};
use hylki_mail_core::MailRequest;

use crate::app::{HylkiApp, ProviderPreset, PROVIDER_PRESETS};
use crate::theme;
use crate::ui::avatar;
use crate::ui::preferences::SettingsWindow;

/// 账号卡片头像尺寸。
const CARD_AVATAR: f32 = 28.0;

/// 账号表单输入状态（在设置窗口自己的 Window 上创建，避免跨窗口持有 InputState）。
pub(crate) struct AccountFormState {
    pub(crate) editing_account_id: Option<u32>,
    pub(crate) display_name: Entity<InputState>,
    pub(crate) email: Entity<InputState>,
    pub(crate) imap_host: Entity<InputState>,
    pub(crate) imap_port: Entity<InputState>,
    pub(crate) imap_tls: bool,
    pub(crate) imap_user: Entity<InputState>,
    pub(crate) imap_pass: Entity<InputState>,
    pub(crate) smtp_host: Entity<InputState>,
    pub(crate) smtp_port: Entity<InputState>,
    pub(crate) smtp_tls: bool,
    pub(crate) smtp_separate: bool,
    pub(crate) smtp_user: Entity<InputState>,
    pub(crate) smtp_pass: Entity<InputState>,
    pub(crate) oauth_client_id: Entity<InputState>,
    pub(crate) signature: Entity<TextareaState>,
    pub(crate) auth_guide: Option<SharedString>,
}

impl AccountFormState {
    pub(crate) fn new(
        initial: Option<&AccountConfig>,
        window: &mut Window,
        cx: &mut Context<SettingsWindow>,
    ) -> Self {
        let first_cfg = initial.cloned().unwrap_or_default();
        let editing_account_id = initial.map(|c| c.id);

        let display_name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("显示名称（如：张三）")
                .default_value(first_cfg.display_name.clone())
        });
        let email = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("邮箱地址（如：user@qq.com）")
                .default_value(first_cfg.email.clone())
        });
        let imap_host = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("IMAP 主机（如：imap.qq.com）")
                .default_value(first_cfg.imap_host.clone())
        });
        let imap_port = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("993")
                .default_value(first_cfg.imap_port.to_string())
        });
        let imap_user = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("IMAP 用户名（留空默认使用邮箱地址）")
                .default_value(first_cfg.imap_username.clone())
        });
        let imap_pass = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("客户端授权码 / 密码（存入 Windows 凭据管理器）")
        });
        let smtp_host = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("SMTP 主机（如：smtp.qq.com）")
                .default_value(first_cfg.smtp_host.clone())
        });
        let smtp_port = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("465")
                .default_value(first_cfg.smtp_port.to_string())
        });
        let smtp_user = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("SMTP 独立用户名（可选）")
                .default_value(first_cfg.smtp_username.clone())
        });
        let smtp_pass = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("SMTP 独立授权码（启用独立凭据时填写）")
        });
        let oauth_client_id = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("可选：自定义 Microsoft Entra Client ID（留空使用内置）")
                .default_value(first_cfg.oauth_client_id.clone())
        });
        let signature = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(3, 6)
                .placeholder("可选：邮件签名（写信 / 回复 / 转发时自动以 -- 分隔插入正文末尾）")
                .default_value(first_cfg.signature.clone())
        });

        Self {
            editing_account_id,
            display_name,
            email,
            imap_host,
            imap_port,
            imap_tls: first_cfg.imap_security == SecurityMode::Tls,
            imap_user,
            imap_pass,
            smtp_host,
            smtp_port,
            smtp_tls: first_cfg.smtp_security == SecurityMode::Tls,
            smtp_separate: first_cfg.smtp_separate_credentials,
            smtp_user,
            smtp_pass,
            oauth_client_id,
            signature,
            auth_guide: None,
        }
    }

    pub(crate) fn load_account(
        &mut self,
        cfg: Option<&AccountConfig>,
        window: &mut Window,
        cx: &mut Context<SettingsWindow>,
    ) {
        let c = cfg.cloned().unwrap_or_default();
        self.editing_account_id = cfg.map(|a| a.id);
        self.display_name
            .update(cx, |s, cx| s.set_value(c.display_name, window, cx));
        self.email
            .update(cx, |s, cx| s.set_value(c.email, window, cx));
        self.imap_host
            .update(cx, |s, cx| s.set_value(c.imap_host, window, cx));
        self.imap_port
            .update(cx, |s, cx| s.set_value(c.imap_port.to_string(), window, cx));
        self.imap_tls = c.imap_security == SecurityMode::Tls;
        self.imap_user
            .update(cx, |s, cx| s.set_value(c.imap_username, window, cx));
        self.imap_pass
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.smtp_host
            .update(cx, |s, cx| s.set_value(c.smtp_host, window, cx));
        self.smtp_port
            .update(cx, |s, cx| s.set_value(c.smtp_port.to_string(), window, cx));
        self.smtp_tls = c.smtp_security == SecurityMode::Tls;
        self.smtp_separate = c.smtp_separate_credentials;
        self.smtp_user
            .update(cx, |s, cx| s.set_value(c.smtp_username, window, cx));
        self.smtp_pass
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.oauth_client_id
            .update(cx, |s, cx| s.set_value(c.oauth_client_id, window, cx));
        self.signature
            .update(cx, |s, cx| s.set_value(c.signature, window, cx));
        self.auth_guide = None;
    }

    pub(crate) fn apply_preset(
        &mut self,
        preset: &ProviderPreset,
        window: &mut Window,
        cx: &mut Context<SettingsWindow>,
    ) {
        self.imap_host
            .update(cx, |s, cx| s.set_value(preset.imap_host, window, cx));
        self.imap_port
            .update(cx, |s, cx| s.set_value(preset.imap_port, window, cx));
        self.smtp_host
            .update(cx, |s, cx| s.set_value(preset.smtp_host, window, cx));
        self.smtp_port
            .update(cx, |s, cx| s.set_value(preset.smtp_port, window, cx));
        self.imap_tls = preset.imap_tls;
        self.smtp_tls = preset.smtp_tls;
        self.auth_guide = Some(SharedString::from(preset.guide));
    }

    pub(crate) fn collect(
        &self,
        existing_accounts: &[AccountConfig],
        cx: &Context<SettingsWindow>,
    ) -> (AccountConfig, String, Option<String>) {
        let email = self.email.read(cx).value().trim().to_string();
        let imap_user = self.imap_user.read(cx).value().trim().to_string();
        let smtp_user = self.smtp_user.read(cx).value().trim().to_string();
        let imap_port = self
            .imap_port
            .read(cx)
            .value()
            .trim()
            .parse::<u16>()
            .unwrap_or(993);
        let smtp_port = self
            .smtp_port
            .read(cx)
            .value()
            .trim()
            .parse::<u16>()
            .unwrap_or(465);
        let id = self
            .editing_account_id
            .unwrap_or_else(|| config::next_account_id(existing_accounts));

        let cfg = AccountConfig {
            id,
            auth_backend: AuthBackend::Password,
            oauth_client_id: String::new(),
            display_name: self.display_name.read(cx).value().trim().to_string(),
            email: email.clone(),
            imap_host: self.imap_host.read(cx).value().trim().to_string(),
            imap_port,
            imap_security: if self.imap_tls {
                SecurityMode::Tls
            } else {
                SecurityMode::StartTls
            },
            imap_username: if imap_user.is_empty() {
                email
            } else {
                imap_user
            },
            smtp_host: self.smtp_host.read(cx).value().trim().to_string(),
            smtp_port,
            smtp_security: if self.smtp_tls {
                SecurityMode::Tls
            } else {
                SecurityMode::StartTls
            },
            smtp_username: smtp_user,
            smtp_separate_credentials: self.smtp_separate,
            signature: self.signature.read(cx).value().to_string(),
        };
        let imap_pwd = self.imap_pass.read(cx).value().to_string();
        let smtp_pwd = if self.smtp_separate {
            Some(self.smtp_pass.read(cx).value().to_string())
        } else {
            None
        };
        (cfg, imap_pwd, smtp_pwd)
    }
}

/// 协议胶囊文本。
fn protocol_label(acc: &AccountConfig) -> &'static str {
    match acc.auth_backend {
        AuthBackend::MicrosoftGraphOAuth2 => "Microsoft Graph OAuth2",
        AuthBackend::Password => "IMAP + SMTP",
    }
}

/// 账号卡片（头像 + 名称/邮箱 + 协议胶囊 + 同步状态 + 操作按钮）。
fn account_card(
    sync_status: SharedString,
    pending_delete: bool,
    demo_mode: bool,
    app_entity: Entity<HylkiApp>,
    acc: &AccountConfig,
    p: &theme::HylkiPalette,
    cx: &mut Context<SettingsWindow>,
) -> gpui_kit::AnyElement {
    let (border, muted_fg, danger, accent) = (p.border, p.text_muted, p.danger, p.accent);
    let acc_id = acc.id;
    let display_name = if acc.display_name.is_empty() {
        acc.email.clone()
    } else {
        acc.display_name.clone()
    };
    let app_for_edit = app_entity.clone();
    let app_for_resync = app_entity.clone();
    let app_for_remove = app_entity.clone();
    let app_for_confirm = app_entity.clone();
    let app_for_cancel = app_entity;

    v_flex()
        .rounded(px(10.0))
        .border_1()
        .border_color(border)
        .bg(p.surface_raised)
        .p_3()
        .gap_2()
        .child(
            h_flex()
                .items_center()
                .gap_3()
                .child(avatar::account_avatar(
                    &display_name,
                    &acc.email,
                    CARD_AVATAR,
                    0,
                    accent,
                    p.surface_raised,
                ))
                .child(
                    v_flex()
                        .min_w_0()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .truncate()
                                .child(SharedString::from(display_name)),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted_fg)
                                .truncate()
                                .child(SharedString::from(acc.email.clone())),
                        ),
                )
                .child(
                    div()
                        .px_2()
                        .py_0p5()
                        .rounded_full()
                        .bg(p.accent_soft())
                        .text_color(accent)
                        .text_xs()
                        .child(protocol_label(acc)),
                )
                .child(div().flex_1())
                .child(div().text_xs().text_color(muted_fg).child(sync_status)),
        )
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .justify_end()
                .when(!demo_mode, |el| {
                    el.child(
                        Button::new(("edit-acc", acc_id))
                            .outline()
                            .compact()
                            .label("编辑")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let cfg = app_for_edit
                                    .read(cx)
                                    .accounts
                                    .iter()
                                    .find(|a| a.id == acc_id)
                                    .cloned();
                                if let Some(cfg) = cfg.as_ref() {
                                    this.edit_account_in_form(Some(cfg), window, cx);
                                }
                            })),
                    )
                    .child(
                        Button::new(("resync-acc", acc_id))
                            .outline()
                            .compact()
                            .label("重新同步")
                            .on_click(cx.listener(move |_, _, _, cx| {
                                app_for_resync.update(cx, |app, cx| {
                                    if let Some(ref w) = app.worker {
                                        w.send(MailRequest::SyncAccount { account_id: acc_id });
                                        cx.notify();
                                    }
                                });
                            })),
                    )
                    .child(
                        Button::new(("remove-acc", acc_id))
                            .ghost()
                            .compact()
                            .label("移除账号")
                            .on_click(cx.listener(move |_, _, _, cx| {
                                app_for_remove.update(cx, |app, cx| {
                                    app.request_delete_account(acc_id, cx);
                                });
                            })),
                    )
                })
                .when(demo_mode, |el| {
                    el.child(
                        div()
                            .text_xs()
                            .text_color(muted_fg)
                            .child("演示账号（不可修改）"),
                    )
                }),
        )
        .when(pending_delete, |el| {
            el.child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .justify_end()
                    .px_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(danger)
                            .child("确认移除该账号？将同时清理本地缓存与 Windows 凭据。"),
                    )
                    .child(
                        Button::new("confirm-remove-acc")
                            .compact()
                            .label("确认移除")
                            .on_click(cx.listener(move |_, _, _, cx| {
                                app_for_confirm.update(cx, |app, cx| {
                                    app.confirm_delete_account(cx);
                                });
                            })),
                    )
                    .child(
                        Button::new("cancel-remove-acc")
                            .ghost()
                            .compact()
                            .label("取消")
                            .on_click(cx.listener(move |_, _, _, cx| {
                                app_for_cancel.update(cx, |app, cx| {
                                    app.cancel_delete_account(cx);
                                });
                            })),
                    ),
            )
        })
        .into_any_element()
}

/// 带小标题的表单字段容器，避免输入内容后分不清字段含义。
fn labeled_field(
    label: &'static str,
    input: gpui_kit::AnyElement,
    muted_fg: gpui_kit::Hsla,
) -> gpui_kit::Div {
    v_flex()
        .gap_1()
        .min_w_0()
        .child(div().text_xs().text_color(muted_fg).child(label))
        .child(input)
}

pub(crate) fn render_content(
    form: &AccountFormState,
    app_entity: Entity<HylkiApp>,
    cx: &mut Context<SettingsWindow>,
) -> gpui_kit::AnyElement {
    let p = theme::current(cx);
    let (border, muted_fg, accent_soft, primary, danger) =
        (p.border, p.text_muted, p.accent_soft(), p.accent, p.danger);

    let (demo_mode, accounts_list, cfg_feedback) = {
        let app = app_entity.read(cx);
        (
            app.demo_mode,
            app.accounts.clone(),
            app.cfg_feedback.clone(),
        )
    };

    if demo_mode {
        return v_flex()
            .id("accounts-scroll")
            .flex_1()
            .min_h_0()
            .px_6()
            .py_4()
            .gap_3()
            .overflow_y_scrollbar()
            .child(
                v_flex()
                    .w_full()
                    .max_w(px(600.0))
                    .mx_auto()
                    .gap_2()
                    .p_4()
                    .rounded(px(12.0))
                    .border_1()
                    .border_color(border)
                    .bg(p.surface_raised)
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.text)
                            .child("当前处于演示模式"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted_fg)
                            .child("演示模式下使用内置模拟邮件数据，无法添加或编辑真实邮箱账号。"),
                    ),
            )
            .into_any_element();
    }

    let card_snapshots: Vec<(SharedString, bool)> = {
        let app = app_entity.read(cx);
        accounts_list
            .iter()
            .map(|acc| {
                let sync_status = match app.folders_by_account.get(&acc.id) {
                    Some(folders) if !folders.is_empty() => {
                        let total: usize = folders.iter().map(|f| f.total_count).sum();
                        let unread: u32 = folders.iter().map(|f| f.unread_count).sum();
                        SharedString::from(format!("已同步：{total} 封邮件 / {unread} 未读"))
                    }
                    _ => SharedString::from("尚未同步"),
                };
                let pending_delete = app.pending_delete_account_id == Some(acc.id);
                (sync_status, pending_delete)
            })
            .collect()
    };
    let account_cards: Vec<gpui_kit::AnyElement> = accounts_list
        .iter()
        .zip(card_snapshots)
        .map(|(acc, (sync_status, pending_delete))| {
            account_card(
                sync_status,
                pending_delete,
                demo_mode,
                app_entity.clone(),
                acc,
                &p,
                cx,
            )
        })
        .collect();

    let app_for_oauth = app_entity.clone();
    let app_for_verify = app_entity.clone();
    let app_for_save = app_entity.clone();
    let app_for_delete = app_entity;

    v_flex()
        .id("accounts-scroll")
        .flex_1()
        .min_h_0()
        .px_6()
        .py_4()
        .overflow_y_scrollbar()
        .child(
            v_flex()
                .w_full()
                .max_w(px(600.0))
                .mx_auto()
                .gap_3()
                .when(!accounts_list.is_empty(), |el| {
                    el.child(
                        h_flex()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .text_lg()
                                    .font_weight(FontWeight::BOLD)
                                    .child(SharedString::from(format!(
                                        "已添加账号（{}）",
                                        accounts_list.len()
                                    ))),
                            )
                            .child(
                                Button::new("add-new-acc")
                                    .primary()
                                    .label("+ 添加新账号")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.edit_account_in_form(None, window, cx);
                                    })),
                            ),
                    )
                    .children(account_cards)
                    .child(div().h(px(8.0)))
                })
                .child(
                    v_flex()
                        .gap_2()
                        .child(div().text_lg().font_weight(FontWeight::BOLD).child(
                            if form.editing_account_id.is_some() {
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
                                            this.apply_account_preset(preset, window, cx);
                                        }))
                                })),
                        ),
                )
                .child(
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
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            let custom_client_id = this
                                                .account_form
                                                .oauth_client_id
                                                .read(cx)
                                                .value()
                                                .trim()
                                                .to_string();
                                            let editing = this.account_form.editing_account_id;
                                            app_for_oauth.update(cx, |app, cx| {
                                                app.start_outlook_oauth(
                                                    custom_client_id,
                                                    editing,
                                                    cx,
                                                );
                                            });
                                        })),
                                ),
                        )
                        .child(Input::new(&form.oauth_client_id)),
                )
                .when_some(form.auth_guide.as_ref(), |el, guide| {
                    el.child(
                        div()
                            .p_2()
                            .rounded_md()
                            .bg(accent_soft)
                            .text_color(p.text)
                            .text_xs()
                            .child(guide.clone()),
                    )
                })
                .child(div().text_xs().text_color(muted_fg).child(
                    "授权码仅保存在 Windows 凭据管理器中，配置文件 accounts.toml 不记录密码。",
                ))
                .child(
                    v_flex()
                        .rounded(px(10.0))
                        .border_1()
                        .border_color(border)
                        .bg(p.surface_raised)
                        .p_4()
                        .gap_3()
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    labeled_field(
                                        "显示名称",
                                        Input::new(&form.display_name).into_any_element(),
                                        muted_fg,
                                    )
                                    .flex_1(),
                                )
                                .child(
                                    labeled_field(
                                        "邮箱地址",
                                        Input::new(&form.email).into_any_element(),
                                        muted_fg,
                                    )
                                    .flex_1(),
                                ),
                        )
                        .child(
                            div()
                                .pt_1()
                                .text_sm()
                                .font_weight(FontWeight::BOLD)
                                .child("收信设置 (IMAP)"),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .items_end()
                                .child(
                                    labeled_field(
                                        "IMAP 服务器",
                                        Input::new(&form.imap_host).into_any_element(),
                                        muted_fg,
                                    )
                                    .flex_1(),
                                )
                                .child(
                                    labeled_field(
                                        "端口",
                                        Input::new(&form.imap_port).into_any_element(),
                                        muted_fg,
                                    )
                                    .w(px(100.0)),
                                )
                                .child(
                                    Button::new("toggle-imap-tls")
                                        .outline()
                                        .label(if form.imap_tls {
                                            "TLS (993)"
                                        } else {
                                            "STARTTLS (143)"
                                        })
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.account_form.imap_tls =
                                                !this.account_form.imap_tls;
                                            cx.notify();
                                        })),
                                ),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    labeled_field(
                                        "IMAP 用户名（留空默认邮箱地址）",
                                        Input::new(&form.imap_user).into_any_element(),
                                        muted_fg,
                                    )
                                    .flex_1(),
                                )
                                .child(
                                    labeled_field(
                                        "客户端授权码 / 密码",
                                        Input::new(&form.imap_pass).into_any_element(),
                                        muted_fg,
                                    )
                                    .flex_1(),
                                ),
                        )
                        .child(
                            h_flex()
                                .pt_1()
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
                                        .compact()
                                        .label(if form.smtp_separate {
                                            "✓ 使用独立 SMTP 凭据"
                                        } else {
                                            "复用 IMAP 凭据"
                                        })
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.account_form.smtp_separate =
                                                !this.account_form.smtp_separate;
                                            cx.notify();
                                        })),
                                ),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .items_end()
                                .child(
                                    labeled_field(
                                        "SMTP 服务器",
                                        Input::new(&form.smtp_host).into_any_element(),
                                        muted_fg,
                                    )
                                    .flex_1(),
                                )
                                .child(
                                    labeled_field(
                                        "端口",
                                        Input::new(&form.smtp_port).into_any_element(),
                                        muted_fg,
                                    )
                                    .w(px(100.0)),
                                )
                                .child(
                                    Button::new("toggle-smtp-tls")
                                        .outline()
                                        .label(if form.smtp_tls {
                                            "TLS (465)"
                                        } else {
                                            "STARTTLS (587)"
                                        })
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.account_form.smtp_tls =
                                                !this.account_form.smtp_tls;
                                            cx.notify();
                                        })),
                                ),
                        )
                        .when(form.smtp_separate, |el| {
                            el.child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        labeled_field(
                                            "SMTP 独立用户名",
                                            Input::new(&form.smtp_user).into_any_element(),
                                            muted_fg,
                                        )
                                        .flex_1(),
                                    )
                                    .child(
                                        labeled_field(
                                            "SMTP 独立授权码",
                                            Input::new(&form.smtp_pass).into_any_element(),
                                            muted_fg,
                                        )
                                        .flex_1(),
                                    ),
                            )
                        })
                        .child(labeled_field(
                            "邮件签名（新建 / 回复 / 转发时自动插入正文末尾）",
                            div()
                                .rounded(px(6.0))
                                .border_1()
                                .border_color(border)
                                .bg(p.surface)
                                .p_1()
                                .child(Textarea::new(&form.signature))
                                .into_any_element(),
                            muted_fg,
                        )),
                )
                .when_some(cfg_feedback.as_ref(), |el, (ok, msg)| {
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
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let existing = app_for_verify.read(cx).accounts.clone();
                                    let (config, imap_pwd, smtp_pwd) =
                                        this.account_form.collect(&existing, cx);
                                    app_for_verify.update(cx, |app, cx| {
                                        app.verify_account_config(config, imap_pwd, smtp_pwd, cx);
                                    });
                                })),
                        )
                        .child(
                            Button::new("save-account")
                                .primary()
                                .label("保存并同步")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let existing = app_for_save.read(cx).accounts.clone();
                                    let (config, imap_pwd, smtp_pwd) =
                                        this.account_form.collect(&existing, cx);
                                    app_for_save.update(cx, |app, cx| {
                                        app.save_account_config(config, imap_pwd, smtp_pwd, cx);
                                    });
                                })),
                        )
                        .when_some(form.editing_account_id, |el, acc_id| {
                            el.when(!demo_mode, |el| {
                                el.child(
                                    Button::new("delete-account")
                                        .ghost()
                                        .label("移除此账号")
                                        .on_click(cx.listener(move |_, _, _, cx| {
                                            app_for_delete.update(cx, |app, cx| {
                                                app.request_delete_account(acc_id, cx);
                                            });
                                        })),
                                )
                            })
                        }),
                ),
        )
        .into_any_element()
}
