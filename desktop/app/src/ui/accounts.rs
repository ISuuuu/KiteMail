//! 账号管理页（方案文档第三节页面 4）：
//! - 顶部已添加账号卡片列表：头像、显示名、邮箱、协议胶囊
//!   （`Microsoft Graph OAuth2` / `IMAP + SMTP`）、最近同步状态、
//!   编辑 / 重新同步（`SyncAccount`）/ 移除账号（`DeleteAccount`，内联确认）；
//!   右上角「+ 添加新账号」切换到空白表单。
//! - 下方添加与编辑表单：保留现有 Outlook OAuth2 登录卡片、8 大服务商预设、
//!   指引横幅、IMAP / SMTP 配置（TLS / STARTTLS 切换、独立 SMTP 凭据）、
//!   「测试连接」与「保存并同步」及实时反馈。

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{div, px, Context, FontWeight, IntoElement, ParentElement, SharedString, Styled};
use hylki_mail_core::config::{AccountConfig, AuthBackend};
use hylki_mail_core::MailRequest;

use crate::app::{HylkiApp, PROVIDER_PRESETS};
use crate::theme;
use crate::ui::avatar;

/// 账号卡片头像尺寸。
const CARD_AVATAR: f32 = 28.0;

/// 协议胶囊文本。
fn protocol_label(acc: &AccountConfig) -> &'static str {
    match acc.auth_backend {
        AuthBackend::MicrosoftGraphOAuth2 => "Microsoft Graph OAuth2",
        AuthBackend::Password => "IMAP + SMTP",
    }
}

/// 账号卡片（头像 + 名称/邮箱 + 协议胶囊 + 同步状态 + 操作按钮）。
fn account_card(
    app: &HylkiApp,
    acc: &AccountConfig,
    p: &theme::HylkiPalette,
    cx: &mut Context<HylkiApp>,
) -> gpui_kit::AnyElement {
    let (border, muted_fg, danger, accent) = (p.border, p.text_muted, p.danger, p.accent);
    let acc_id = acc.id;
    let display_name = if acc.display_name.is_empty() {
        acc.email.clone()
    } else {
        acc.display_name.clone()
    };
    // 最近同步状态：以本地缓存的文件夹计数为据（无全局 per-account 时间戳）。
    let sync_status = match app.folders_by_account.get(&acc.id) {
        Some(folders) if !folders.is_empty() => {
            let total: usize = folders.iter().map(|f| f.total_count).sum();
            let unread: u32 = folders.iter().map(|f| f.unread_count).sum();
            SharedString::from(format!("已同步：{total} 封邮件 / {unread} 未读"))
        }
        _ => SharedString::from("尚未同步"),
    };
    let pending_delete = app.pending_delete_account_id == Some(acc_id);

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
        // 操作行（demo 账号不可编辑 / 同步 / 移除）。
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .justify_end()
                .when(!app.demo_mode, |el| {
                    el.child(
                        Button::new(("edit-acc", acc_id))
                            .outline()
                            .compact()
                            .label("编辑")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let cfg = this.accounts.iter().find(|a| a.id == acc_id).cloned();
                                if let Some(cfg) = cfg.as_ref() {
                                    this.load_account_into_form(Some(cfg), window, cx);
                                }
                            })),
                    )
                    .child(
                        Button::new(("resync-acc", acc_id))
                            .outline()
                            .compact()
                            .label("重新同步")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(ref w) = this.worker {
                                    w.send(MailRequest::SyncAccount { account_id: acc_id });
                                    cx.notify();
                                }
                            })),
                    )
                    .child(
                        Button::new(("remove-acc", acc_id))
                            .ghost()
                            .compact()
                            .label("移除账号")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.request_delete_account(acc_id, cx);
                            })),
                    )
                })
                .when(app.demo_mode, |el| {
                    el.child(
                        div()
                            .text_xs()
                            .text_color(muted_fg)
                            .child("演示账号（不可修改）"),
                    )
                }),
        )
        // 内联移除确认。
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
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.confirm_delete_account(cx);
                            })),
                    )
                    .child(
                        Button::new("cancel-remove-acc")
                            .ghost()
                            .compact()
                            .label("取消")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.cancel_delete_account(cx);
                            })),
                    ),
            )
        })
        .into_any_element()
}

pub(crate) fn render(app: &HylkiApp, cx: &mut Context<HylkiApp>) -> gpui_kit::AnyElement {
    // 避坑清单第 4 条：render 开头先把语义色 Copy 成局部变量。
    let p = theme::current(cx);
    let (border, muted_fg, accent_soft, primary, danger) =
        (p.border, p.text_muted, p.accent_soft(), p.accent, p.danger);

    let account_cards: Vec<gpui_kit::AnyElement> = app
        .accounts
        .iter()
        .map(|acc| account_card(app, acc, &p, cx))
        .collect();

    v_flex()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(p.canvas)
        .p_5()
        .gap_3()
        .overflow_y_scrollbar()
        // 顶部：已添加账号卡片列表 +「+ 添加新账号」。
        .when(!app.accounts.is_empty(), |el| {
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
                                app.accounts.len()
                            ))),
                    )
                    .child(
                        Button::new("add-new-acc")
                            .primary()
                            .label("+ 添加新账号")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.load_account_into_form(None, window, cx);
                            })),
                    ),
            )
            .children(account_cards)
            .child(div().h(px(8.0)))
        })
        // ===== 下方：添加与编辑表单（保留现有实现）=====
        .child(
            v_flex()
                .gap_2()
                .child(div().text_lg().font_weight(FontWeight::BOLD).child(
                    if app.editing_account_id.is_some() {
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
        .when(!app.demo_mode, |el| {
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
                    .child(Input::new(&app.cfg_oauth_client_id)),
            )
        })
        .when_some(app.cfg_auth_guide.as_ref(), |el, guide| {
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
            h_flex()
                .gap_2()
                .child(div().flex_1().child(Input::new(&app.cfg_display_name)))
                .child(div().flex_1().child(Input::new(&app.cfg_email))),
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
                .child(div().flex_1().child(Input::new(&app.cfg_imap_host)))
                .child(div().w(px(100.0)).child(Input::new(&app.cfg_imap_port)))
                .child(
                    Button::new("toggle-imap-tls")
                        .outline()
                        .label(if app.cfg_imap_tls {
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
                .child(div().flex_1().child(Input::new(&app.cfg_imap_user)))
                .child(div().flex_1().child(Input::new(&app.cfg_imap_pass))),
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
                        .label(if app.cfg_smtp_separate {
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
                .child(div().flex_1().child(Input::new(&app.cfg_smtp_host)))
                .child(div().w(px(100.0)).child(Input::new(&app.cfg_smtp_port)))
                .child(
                    Button::new("toggle-smtp-tls")
                        .outline()
                        .label(if app.cfg_smtp_tls {
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
        .when(app.cfg_smtp_separate, |el| {
            el.child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().child(Input::new(&app.cfg_smtp_user)))
                    .child(div().flex_1().child(Input::new(&app.cfg_smtp_pass))),
            )
        })
        .when_some(app.cfg_feedback.as_ref(), |el, (ok, msg)| {
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
                .when_some(app.editing_account_id, |el, acc_id| {
                    el.when(!app.demo_mode, |el| {
                        el.child(
                            Button::new("delete-account")
                                .ghost()
                                .label("移除此账号")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.request_delete_account(acc_id, cx);
                                })),
                        )
                    })
                }),
        )
        .into_any_element()
}
