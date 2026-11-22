//! 首次启动欢迎向导（WP-10，对齐原版 `src/ui/welcome.rs:111-343` 的 5 步轮播向导）：
//! - 步骤 0：Welcome（品牌蓝底 `#005bff` + Wordmark + 「开始设置」与「演示模式」入口）
//! - 步骤 1：Theme & Layout（亮色 / 暗色 / 跟随系统、阅读窗格位置、紧凑列表）
//! - 步骤 2：Add Account（Outlook / Microsoft 365 OAuth2、常见邮箱预设、自定义 IMAP / SMTP）
//! - 步骤 3：Sync & AI（即时推送、远程图片拦截、发件人头像、桌面通知、AI 翻译提示）
//! - 步骤 4：Ready（配置摘要与「进入收件箱 / 演示模式」）

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants};
use gpui_kit::component::separator::Separator;
use gpui_kit::component::{h_flex, v_flex, Icon, Selectable, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    div, img, px, rgb, white, Context, FontWeight, Hsla, InteractiveElement, IntoElement,
    ObjectFit, ParentElement, SharedString, StatefulInteractiveElement, Styled, StyledImage,
    Window,
};

use crate::app::{HylkiApp, ReaderPosition};
use crate::ui::accounts::{ProviderKind, ACCOUNT_PROVIDERS};
use crate::ui::window_controls;

/// 欢迎卡片最大宽度。
const CARD_MAX_WIDTH: f32 = 560.0;
/// 总步数：0=Welcome, 1=Theme & Layout, 2=Add Account, 3=Sync & AI, 4=Ready。
const STEP_COUNT: usize = 5;

/// 全屏品牌蓝（Linux 原版欢迎页固定 `#005bff`，不随亮/暗主题变化）。
const BRAND_BLUE: u32 = 0x005bff;
/// 主按钮琥珀黄与深棕文字（原版 `.welcome-window button.suggested-action`）。
const OAUTH_BTN_BG: u32 = 0xfec11e;
const OAUTH_BTN_FG: u32 = 0x3a2b00;
const OAUTH_BTN_HOVER: u32 = 0xffd151;
const OAUTH_BTN_ACTIVE: u32 = 0xe6ac0a;
/// 白卡片内的固定深灰/深黑文字（不随主题变化，保证白底下两种主题均可读）。
const CARD_TEXT: u32 = 0x101828;
const CARD_MUTED: u32 = 0x667085;
const CARD_BORDER: u32 = 0xe4e7ec;

/// `#rrggbb` 十六进制 → `Hsla`（欢迎页固定品牌色专用，不随主题变化）。
fn hx(hex: u32) -> Hsla {
    Hsla::from(rgb(hex))
}

/// 欢迎页步骤标题与副标题。
fn step_header(step: usize) -> (&'static str, &'static str) {
    match step {
        0 => ("欢迎使用 Hylki", "快速、干净的现代桌面邮件客户端"),
        1 => ("外观与布局", "选择您喜欢的界面明暗与阅读窗格排列方式"),
        2 => (
            "添加邮箱账号",
            "支持 Microsoft 365 OAuth2 及主流 IMAP / SMTP 邮箱",
        ),
        3 => ("同步、隐私与智能辅助", "随时可在偏好设置中修改这些选项"),
        _ => ("一切准备就绪", "您的 Hylki 邮件工作区已完成配置"),
    }
}

/// 卡片内开关选项行（固定白底可读配色）。
fn pref_toggle_row(
    id: &'static str,
    title: &'static str,
    subtitle: &'static str,
    enabled: bool,
    on_toggle: impl Fn(&mut HylkiApp, &mut Context<HylkiApp>) + 'static,
    cx: &mut Context<HylkiApp>,
) -> gpui_kit::AnyElement {
    h_flex()
        .id(id)
        .w_full()
        .px_3()
        .py_2p5()
        .gap_3()
        .items_center()
        .justify_between()
        .rounded(px(8.0))
        .border_1()
        .border_color(hx(CARD_BORDER))
        .cursor_pointer()
        .hover(|s| s.bg(hx(0xf9fafb)))
        .on_click(cx.listener(move |this, _, _, cx| {
            on_toggle(this, cx);
        }))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(hx(CARD_TEXT))
                        .child(title),
                )
                .child(div().text_xs().text_color(hx(CARD_MUTED)).child(subtitle)),
        )
        .child(
            div()
                .px_2p5()
                .py_1()
                .rounded(px(9999.0))
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .bg(if enabled {
                    hx(BRAND_BLUE)
                } else {
                    hx(0xe4e7ec)
                })
                .text_color(if enabled { white() } else { hx(CARD_MUTED) })
                .child(if enabled { "已开启" } else { "已关闭" }),
        )
        .into_any_element()
}

/// 渲染步骤 0：Welcome。
fn render_step_welcome(cx: &mut Context<HylkiApp>) -> gpui_kit::AnyElement {
    v_flex()
        .w_full()
        .gap_3()
        .child(
            div().text_sm().text_color(hx(CARD_TEXT)).child(
                "只需几步即可完成外观、邮箱账号与同步隐私偏好配置。您也可以先以演示模式浏览。",
            ),
        )
        .child(
            div()
                .id("welcome-get-started")
                .w_full()
                .h(px(44.0))
                .rounded(px(8.0))
                .bg(hx(OAUTH_BTN_BG))
                .text_color(hx(OAUTH_BTN_FG))
                .font_weight(FontWeight::SEMIBOLD)
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.bg(hx(OAUTH_BTN_HOVER)))
                .active(|s| s.bg(hx(OAUTH_BTN_ACTIVE)))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.welcome_step = 1;
                    cx.notify();
                }))
                .child("开始配置 →"),
        )
        .child(Separator::horizontal())
        .child(
            Button::new("welcome-demo-0")
                .custom(
                    ButtonCustomVariant::new(cx)
                        .foreground(hx(CARD_MUTED))
                        .hover(hx(CARD_MUTED).opacity(0.10)),
                )
                .label("先以演示模式浏览 (10,000 封模拟邮件)")
                .w_full()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.enter_demo_mode(cx);
                })),
        )
        .into_any_element()
}

/// 渲染步骤 1：Theme & Layout。
fn render_step_theme_layout(app: &HylkiApp, cx: &mut Context<HylkiApp>) -> gpui_kit::AnyElement {
    let dark_mode = app.prefs.dark_mode;
    let reader_pos = app.reader_position;
    let compact = app.prefs.compact_list;

    let theme_btn = |id: &'static str,
                     label: &'static str,
                     target: Option<bool>,
                     cx: &mut Context<HylkiApp>| {
        let active = dark_mode == target;
        Button::new(id)
            .outline()
            .selected(active)
            .label(label)
            .flex_1()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.set_welcome_dark_mode(target, cx);
            }))
    };

    let layout_btn =
        |id: &'static str, label: &'static str, pos: ReaderPosition, cx: &mut Context<HylkiApp>| {
            let active = reader_pos == pos;
            Button::new(id)
                .outline()
                .selected(active)
                .label(label)
                .flex_1()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_reader_position(pos, cx);
                }))
        };

    v_flex()
        .w_full()
        .gap_3()
        .child(
            v_flex()
                .gap_1p5()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(hx(CARD_MUTED))
                        .child("界面明暗模式"),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(theme_btn("w-theme-sys", "跟随系统", None, cx))
                        .child(theme_btn("w-theme-light", "浅色", Some(false), cx))
                        .child(theme_btn("w-theme-dark", "深色", Some(true), cx)),
                ),
        )
        .child(
            v_flex()
                .gap_1p5()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(hx(CARD_MUTED))
                        .child("阅读窗格位置"),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(layout_btn(
                            "w-pos-right",
                            "列表右侧",
                            ReaderPosition::Right,
                            cx,
                        ))
                        .child(layout_btn(
                            "w-pos-below",
                            "列表下方",
                            ReaderPosition::Below,
                            cx,
                        ))
                        .child(layout_btn(
                            "w-pos-hidden",
                            "仅列表",
                            ReaderPosition::Hidden,
                            cx,
                        )),
                ),
        )
        .child(pref_toggle_row(
            "w-compact-list",
            "紧凑邮件列表",
            "每行显示更少留白，同屏浏览更多邮件",
            compact,
            |this, cx| {
                this.prefs.compact_list = !this.prefs.compact_list;
                this.save_welcome_prefs(cx);
            },
            cx,
        ))
        .into_any_element()
}

/// 渲染步骤 2：Add Account。
fn render_step_add_account(app: &HylkiApp, cx: &mut Context<HylkiApp>) -> gpui_kit::AnyElement {
    let preset_buttons: Vec<gpui_kit::AnyElement> = ACCOUNT_PROVIDERS
        .iter()
        .enumerate()
        .filter(|(_, p)| p.kind == ProviderKind::Preset && p.featured_on_welcome)
        .map(|(idx, preset)| {
            let btn_id = SharedString::from(format!("welcome-preset-{}", preset.id));
            Button::new(btn_id)
                .outline()
                .label(preset.label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.open_settings_preset_account(idx, cx);
                }))
                .into_any_element()
        })
        .collect();

    v_flex()
        .w_full()
        .gap_3()
        .when(!app.accounts.is_empty(), |el| {
            el.child(
                h_flex()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .rounded(px(8.0))
                    .bg(hx(0xecfdf3))
                    .border_1()
                    .border_color(hx(0xa6f4c5))
                    .child(Icon::new(IconName::Check).with_size(px(14.0)).text_color(hx(0x027a48)))
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(hx(0x027a48))
                            .child(SharedString::from(format!(
                                "已配置 {} 个邮箱账号，您可以继续添加或直接进入下一步",
                                app.accounts.len()
                            ))),
                    ),
            )
        })
        .child(
            div()
                .id("welcome-oauth")
                .w_full()
                .h(px(44.0))
                .rounded(px(8.0))
                .bg(hx(OAUTH_BTN_BG))
                .text_color(hx(OAUTH_BTN_FG))
                .font_weight(FontWeight::SEMIBOLD)
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.bg(hx(OAUTH_BTN_HOVER)))
                .active(|s| s.bg(hx(OAUTH_BTN_ACTIVE)))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.start_outlook_oauth(String::new(), String::new(), None, cx);
                }))
                .child("使用 Outlook / Microsoft 365 登录 (OAuth2)"),
        )
        .child(
            div()
                .text_xs()
                .text_color(hx(CARD_MUTED))
                .child("微软已禁用传统密码登录，点击后将打开系统浏览器完成授权，并通过 Microsoft Graph 收发邮件。"),
        )
        .child(Separator::horizontal())
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(hx(CARD_MUTED))
                .child("常见邮箱快捷添加"),
        )
        .child(h_flex().gap_2().flex_wrap().children(preset_buttons))
        .child(
            Button::new("welcome-custom-imap")
                .outline()
                .label("自定义 IMAP / SMTP...")
                .w_full()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.open_settings_edit_account(None, cx);
                })),
        )
        .into_any_element()
}

/// 渲染步骤 3：Sync & AI。
fn render_step_sync_ai(app: &HylkiApp, cx: &mut Context<HylkiApp>) -> gpui_kit::AnyElement {
    v_flex()
        .w_full()
        .gap_2()
        .child(pref_toggle_row(
            "w-pref-push",
            "实时邮件推送 (IMAP IDLE / Graph)",
            "保持长连接，收到新邮件时立即同步",
            app.prefs.push_enabled,
            |this, cx| {
                this.prefs.push_enabled = !this.prefs.push_enabled;
                this.save_welcome_prefs(cx);
            },
            cx,
        ))
        .child(pref_toggle_row(
            "w-pref-remote-img",
            "拦截外部远程图片",
            "防止追踪像素泄露您的阅读时间与 IP，可按发件人放行",
            app.prefs.block_remote_images,
            |this, cx| {
                this.prefs.block_remote_images = !this.prefs.block_remote_images;
                this.save_welcome_prefs(cx);
            },
            cx,
        ))
        .child(pref_toggle_row(
            "w-pref-avatars",
            "显示发件人头像",
            "在邮件列表与阅读器头部显示字母头像或品牌图标",
            app.prefs.show_avatars,
            |this, cx| {
                this.prefs.show_avatars = !this.prefs.show_avatars;
                this.save_welcome_prefs(cx);
            },
            cx,
        ))
        .child(pref_toggle_row(
            "w-pref-notif",
            "桌面新邮件通知",
            "后台同步到新未读邮件时弹出 Windows 系统通知",
            app.prefs.desktop_notifications,
            |this, cx| {
                this.prefs.desktop_notifications = !this.prefs.desktop_notifications;
                this.save_welcome_prefs(cx);
            },
            cx,
        ))
        .child(pref_toggle_row(
            "w-pref-ai-trans",
            "智能翻译与辅助提示",
            "在阅读外文邮件或写信时提供一键翻译入口",
            app.prefs.translation_offer_reader,
            |this, cx| {
                let next = !this.prefs.translation_offer_reader;
                this.prefs.translation_offer_reader = next;
                this.prefs.translation_offer_composer = next;
                this.save_welcome_prefs(cx);
            },
            cx,
        ))
        .into_any_element()
}

/// 渲染步骤 4：Ready。
fn render_step_ready(app: &HylkiApp, cx: &mut Context<HylkiApp>) -> gpui_kit::AnyElement {
    let theme_label = match app.prefs.dark_mode {
        None => "跟随系统",
        Some(true) => "深色模式",
        Some(false) => "浅色模式",
    };
    let layout_label = match app.reader_position {
        ReaderPosition::Right => "阅读器在右侧",
        ReaderPosition::Below => "阅读器在下方",
        ReaderPosition::Hidden => "仅列表模式",
    };
    let acct_summary = if app.accounts.is_empty() {
        "尚未添加账号（将进入演示模式）".to_string()
    } else {
        format!("已就绪 {} 个邮箱账号", app.accounts.len())
    };

    v_flex()
        .w_full()
        .gap_3()
        .child(
            v_flex()
                .p_3()
                .gap_2()
                .rounded(px(8.0))
                .bg(hx(0xf9fafb))
                .border_1()
                .border_color(hx(CARD_BORDER))
                .child(
                    h_flex()
                        .justify_between()
                        .child(
                            div()
                                .text_xs()
                                .text_color(hx(CARD_MUTED))
                                .child("外观与布局"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(hx(CARD_TEXT))
                                .child(SharedString::from(format!(
                                    "{theme_label} · {layout_label}"
                                ))),
                        ),
                )
                .child(
                    h_flex()
                        .justify_between()
                        .child(div().text_xs().text_color(hx(CARD_MUTED)).child("邮箱账号"))
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(hx(CARD_TEXT))
                                .child(SharedString::from(acct_summary)),
                        ),
                )
                .child(
                    h_flex()
                        .justify_between()
                        .child(
                            div()
                                .text_xs()
                                .text_color(hx(CARD_MUTED))
                                .child("实时推送 / 远程图拦截"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(hx(CARD_TEXT))
                                .child(SharedString::from(format!(
                                    "{} / {}",
                                    if app.prefs.push_enabled {
                                        "开启"
                                    } else {
                                        "关闭"
                                    },
                                    if app.prefs.block_remote_images {
                                        "开启"
                                    } else {
                                        "关闭"
                                    }
                                ))),
                        ),
                ),
        )
        .child(
            div()
                .id("welcome-finish-cta")
                .w_full()
                .h(px(44.0))
                .rounded(px(8.0))
                .bg(hx(OAUTH_BTN_BG))
                .text_color(hx(OAUTH_BTN_FG))
                .font_weight(FontWeight::SEMIBOLD)
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.bg(hx(OAUTH_BTN_HOVER)))
                .active(|s| s.bg(hx(OAUTH_BTN_ACTIVE)))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.finish_welcome_wizard(cx);
                }))
                .child(if app.accounts.is_empty() {
                    "进入演示收件箱"
                } else {
                    "进入收件箱"
                }),
        )
        .into_any_element()
}

pub(crate) fn render(
    app: &HylkiApp,
    window: &Window,
    cx: &mut Context<HylkiApp>,
) -> gpui_kit::AnyElement {
    let win_controls = window_controls::render_window_controls("welcome-wc", window, cx);
    let step = app.welcome_step.min(STEP_COUNT - 1);
    let (title, subtitle) = step_header(step);

    let body = match step {
        0 => render_step_welcome(cx),
        1 => render_step_theme_layout(app, cx),
        2 => render_step_add_account(app, cx),
        3 => render_step_sync_ai(app, cx),
        _ => render_step_ready(app, cx),
    };

    let dots: Vec<gpui_kit::AnyElement> = (0..STEP_COUNT)
        .map(|i| {
            let active = i == step;
            div()
                .id(("welcome-dot", i))
                .w(px(if active { 18.0 } else { 8.0 }))
                .h(px(8.0))
                .rounded(px(9999.0))
                .bg(if active {
                    white()
                } else {
                    white().opacity(0.40)
                })
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.welcome_step = i;
                    cx.notify();
                }))
                .into_any_element()
        })
        .collect();

    v_flex()
        .flex_1()
        .min_w_0()
        .h_full()
        // 全屏品牌蓝底（原版 window.welcome-window #005bff）。
        .bg(hx(BRAND_BLUE))
        // 顶栏保留窗口控制按钮。
        .child(
            h_flex()
                .h(px(window_controls::HEADER_HEIGHT))
                .flex_shrink_0()
                .items_center()
                .child(window_controls::drag_spacer())
                .child(win_controls),
        )
        .child(
            v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .px_4()
                .child(
                    v_flex()
                        .w_full()
                        .max_w(px(CARD_MAX_WIDTH))
                        .items_center()
                        .gap_2()
                        // 白色 Wordmark（蓝底固定用白色版）与当前步骤标题。
                        .child(
                            img("welcome/wordmark-white.png")
                                .h(px(44.0))
                                .object_fit(ObjectFit::Contain),
                        )
                        .child(
                            div()
                                .text_lg()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(white())
                                .child(title),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(white().opacity(0.85))
                                .child(subtitle),
                        )
                        // 居中悬浮白色卡片（原版 .welcome-card：14px 圆角、固定白底）。
                        .child(
                            v_flex()
                                .w_full()
                                .mt_2()
                                .rounded(px(14.0))
                                .bg(white())
                                .shadow_md()
                                .p(px(24.0))
                                .gap_3()
                                .child(body),
                        )
                        // 底部步骤导航（上一步 + 圆点指示器 + 下一步/跳过）。
                        .child(
                            h_flex()
                                .w_full()
                                .mt_2()
                                .items_center()
                                .justify_between()
                                .child(if step > 0 {
                                    Button::new("welcome-prev")
                                        .custom(
                                            ButtonCustomVariant::new(cx)
                                                .foreground(white())
                                                .hover(white().opacity(0.15)),
                                        )
                                        .label("← 上一步")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.welcome_step = this.welcome_step.saturating_sub(1);
                                            cx.notify();
                                        }))
                                        .into_any_element()
                                } else {
                                    div().w(px(72.0)).into_any_element()
                                })
                                .child(h_flex().gap_1p5().items_center().children(dots))
                                .child(if step + 1 < STEP_COUNT && step > 0 {
                                    Button::new("welcome-next")
                                        .custom(
                                            ButtonCustomVariant::new(cx)
                                                .foreground(white())
                                                .hover(white().opacity(0.15)),
                                        )
                                        .label(if step == 2 && app.accounts.is_empty() {
                                            "跳过 →"
                                        } else {
                                            "下一步 →"
                                        })
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.welcome_step = (this.welcome_step + 1).min(STEP_COUNT - 1);
                                            cx.notify();
                                        }))
                                        .into_any_element()
                                } else {
                                    div().w(px(72.0)).into_any_element()
                                }),
                        )
                        .child(
                            div()
                                .mt_1()
                                .text_xs()
                                .text_color(white().opacity(0.85))
                                .child(SharedString::from(
                                    "账号授权码仅保存在 Windows 凭据管理器中，配置文件 accounts.toml 不记录密码。",
                                )),
                        ),
                ),
        )
        .into_any_element()
}
