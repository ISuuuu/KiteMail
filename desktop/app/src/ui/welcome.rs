//! 首次启动欢迎向导（方案文档第三节页面 2）：
//! - 触发时机：启动且无 `--demo` 且 `accounts.toml` 为空（`RightPaneMode::Welcome`）。
//! - 全屏品牌蓝底（复刻 Linux 原版 `#005bff`，不随亮/暗主题变化），居中悬浮白色
//!   卡片（最大宽度 540px），提供三类入口：
//!   1. 大按钮「使用 Outlook / Microsoft 365 登录 (OAuth2)」→ 现有 `StartMicrosoftOAuth`；
//!   2. 服务商预设快捷行（8 个预设，点击进入配置表单并自动带入主机/端口/加密与指引）
//!      +「自定义 IMAP / SMTP」；
//!   3. 底部辅助入口「先以演示模式浏览 (10,000 封模拟邮件)」（运行时进入 demo）。

use gpui_kit::component::button::{Button, ButtonCustomVariant, ButtonVariants};
use gpui_kit::component::separator::Separator;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::{
    div, img, px, rgb, white, Context, FontWeight, Hsla, InteractiveElement, IntoElement,
    ObjectFit, ParentElement, SharedString, StatefulInteractiveElement, Styled, StyledImage,
    Window,
};

use crate::app::HylkiApp;
use crate::ui::accounts::{ProviderKind, ACCOUNT_PROVIDERS};
use crate::ui::window_controls;

/// 欢迎卡片最大宽度。
const CARD_MAX_WIDTH: f32 = 540.0;

/// 全屏品牌蓝（Linux 原版欢迎页固定 `#005bff`，不随亮/暗主题变化）。
const BRAND_BLUE: u32 = 0x005bff;
/// 主按钮琥珀黄与深棕文字（原版 `.welcome-window button.suggested-action`）。
const OAUTH_BTN_BG: u32 = 0xfec11e;
const OAUTH_BTN_FG: u32 = 0x3a2b00;
const OAUTH_BTN_HOVER: u32 = 0xffd151;
const OAUTH_BTN_ACTIVE: u32 = 0xe6ac0a;
/// 白卡片内的固定深灰说明文字（不随主题变化，保证白底下两种主题均可读）。
const CARD_MUTED: u32 = 0x667085;

/// `#rrggbb` 十六进制 → `Hsla`（欢迎页固定品牌色专用，不随主题变化）。
fn hx(hex: u32) -> Hsla {
    Hsla::from(rgb(hex))
}

pub(crate) fn render(
    _app: &HylkiApp,
    window: &Window,
    cx: &mut Context<HylkiApp>,
) -> gpui_kit::AnyElement {
    let win_controls = window_controls::render_window_controls("welcome-wc", window, cx);

    // 服务商预设快捷行（欢迎页仅放高频国内 / 主流预设，其余可在设置页下拉选择）。
    let preset_buttons: Vec<gpui_kit::AnyElement> = ACCOUNT_PROVIDERS
        .iter()
        .enumerate()
        .filter(|(_, p)| p.kind == ProviderKind::Preset && p.featured_on_welcome)
        .map(|(idx, preset)| {
            // ElementId 不支持 (&str, &str) 元组。
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
        .flex_1()
        .min_w_0()
        .h_full()
        // 全屏品牌蓝底（原版 window.welcome-window #005bff）。
        .bg(hx(BRAND_BLUE))
        // 顶栏保留窗口控制按钮（蓝底上深浅主题图标均可辨认，不另行处理）。
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
                // 白色 Wordmark（蓝底固定用白色版）与白色副标题。
                .child(
                    img("welcome/wordmark-white.png")
                        .h(px(46.0))
                        .object_fit(ObjectFit::Contain),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(white().opacity(0.85))
                        .child("欢迎使用 Hylki — 快速、干净的桌面邮件客户端"),
                )
                // 居中悬浮白色卡片（原版 .welcome-card：14px 圆角、固定白底、无边框）。
                .child(
                    v_flex()
                        .w_full()
                        .mt_3()
                        .rounded(px(14.0))
                        .bg(white())
                        .shadow_md()
                        .p(px(24.0))
                        .gap_3()
                        // 入口 1：Outlook / Microsoft 365 OAuth2 一键授权。
                        // 主按钮为琥珀黄底深棕字（原版 suggested-action 配色）；gpui-kit
                        // Button 不支持固定实心底色，按方案用 div 手写并保留点击回调。
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
                                    this.start_outlook_oauth(String::new(), None, cx);
                                }))
                                .child("使用 Outlook / Microsoft 365 登录 (OAuth2)"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(hx(CARD_MUTED))
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
                                .text_color(hx(CARD_MUTED))
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
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.open_settings_edit_account(None, cx);
                                })),
                        )
                        .child(Separator::horizontal())
                        // 入口 3：演示模式（白卡上固定深灰前景，避免暗色主题下白底浅字不可读）。
                        .child(
                            Button::new("welcome-demo")
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
                        ),
                )
                .child(
                    div()
                        .mt_2()
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
