//! 首次启动欢迎向导（方案文档第三节页面 2）：
//! - 触发时机：启动且无 `--demo` 且 `accounts.toml` 为空（`RightPaneMode::Welcome`）。
//! - 居中圆角卡片（最大宽度 540px），提供三类入口：
//!   1. 大按钮「使用 Outlook / Microsoft 365 登录 (OAuth2)」→ 现有 `StartMicrosoftOAuth`；
//!   2. 服务商预设快捷行（8 个预设，点击进入配置表单并自动带入主机/端口/加密与指引）
//!      +「自定义 IMAP / SMTP」；
//!   3. 底部辅助入口「先以演示模式浏览 (10,000 封模拟邮件)」（运行时进入 demo）。

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::separator::Separator;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{
    div, img, px, Context, FontWeight, IntoElement, ObjectFit, ParentElement, SharedString, Styled,
    StyledImage, Window,
};

use crate::app::{HylkiApp, PROVIDER_PRESETS};
use crate::theme;
use crate::ui::window_controls;

/// 欢迎卡片最大宽度。
const CARD_MAX_WIDTH: f32 = 540.0;

pub(crate) fn render(
    app: &HylkiApp,
    window: &Window,
    cx: &mut Context<HylkiApp>,
) -> gpui_kit::AnyElement {
    // 避坑清单第 4 条：render 开头先把语义色 Copy 成局部变量。
    let p = theme::current(cx);
    let (border, muted_fg) = (p.border, p.text_muted);
    let win_controls = window_controls::render_window_controls("welcome-wc", window, cx);
    let wordmark_path = if app.dark_mode {
        "welcome/wordmark-white.png"
    } else {
        "welcome/wordmark-black.png"
    };

    // 服务商预设快捷行。
    let preset_buttons: Vec<gpui_kit::AnyElement> = PROVIDER_PRESETS
        .iter()
        .map(|preset| {
            // ElementId 不支持 (&str, &str) 元组。
            let btn_id = SharedString::from(format!("welcome-preset-{}", preset.id));
            Button::new(btn_id)
                .outline()
                .label(preset.label)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.start_preset_account(preset, window, cx);
                }))
                .into_any_element()
        })
        .collect();

    v_flex()
        .flex_1()
        .min_w_0()
        .h_full()
        .bg(p.canvas)
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
                // Logo 图形（亮/暗主题自动切换）与副标题。
                .child(
                    img(wordmark_path)
                        .h(px(46.0))
                        .object_fit(ObjectFit::Contain),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(muted_fg)
                        .child("欢迎使用 Hylki — 快速、干净的桌面邮件客户端"),
                )
                // 居中入口卡片。
                .child(
                    v_flex()
                        .w_full()
                        .mt_3()
                        .rounded(px(12.0))
                        .border_1()
                        .border_color(border)
                        .bg(p.surface_raised)
                        .p(px(24.0))
                        .gap_3()
                        // 入口 1：Outlook / Microsoft 365 OAuth2 一键授权。
                        .child(
                            Button::new("welcome-oauth")
                                .primary()
                                .label("使用 Outlook / Microsoft 365 登录 (OAuth2)")
                                .rounded(px(8.0))
                                .h(px(44.0))
                                .w_full()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.start_outlook_oauth(cx);
                                })),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted_fg)
                                .child(
                                    "微软已禁用传统密码登录，点击后将打开系统浏览器完成授权，并通过 Microsoft Graph 收发邮件。",
                                ),
                        )
                        .child(Separator::horizontal())
                        // 入口 2：常见邮箱快捷添加。
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(muted_fg)
                                .child("常见邮箱快捷添加"),
                        )
                        .child(
                            h_flex().gap_2().flex_wrap().children(preset_buttons),
                        )
                        .child(
                            Button::new("welcome-custom-imap")
                                .outline()
                                .label("自定义 IMAP / SMTP...")
                                .w_full()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.load_account_into_form(None, window, cx);
                                })),
                        )
                        .child(Separator::horizontal())
                        // 入口 3：演示模式。
                        .child(
                            Button::new("welcome-demo")
                                .ghost()
                                .label("先以演示模式浏览 (10,000 封模拟邮件)")
                                .w_full()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.enter_demo_mode(cx);
                                })),
                        ),
                )
                .child(
                    div()
                        .mt_2()
                        .text_xs()
                        .text_color(muted_fg)
                        .child(SharedString::from(
                            "账号授权码仅保存在 Windows 凭据管理器中，配置文件 accounts.toml 不记录密码。",
                        )),
                ),
                ),
        )
        .into_any_element()
}
