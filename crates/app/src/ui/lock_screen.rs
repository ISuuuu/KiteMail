//! 纯净应用锁屏解锁视图：锁定时完全替换主窗口内容，仅提供密码输入与窗口控制。

use std::time::Instant;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::{h_flex, v_flex, Disableable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    div, img, px, AnyElement, Context, FontWeight, InteractiveElement, IntoElement, MouseButton,
    ObjectFit, ParentElement, SharedString, Styled, StyledImage, Window,
};

use crate::app::{HylkiApp, QuitApp};
use crate::theme;
use crate::ui::window_controls;

/// 渲染锁定后的唯一界面：顶部拖拽与窗口控制栏 + 居中密码解锁卡片。
pub(crate) fn render(
    app: &mut HylkiApp,
    window: &mut Window,
    cx: &mut Context<HylkiApp>,
) -> AnyElement {
    if app.lock_input_needs_focus {
        app.lock_input_needs_focus = false;
        app.lock_passcode_input
            .update(cx, |state, cx| state.focus(window, cx));
    }

    let p = theme::current(cx);
    let wordmark_path = if app.dark_mode {
        "welcome/wordmark-white.png"
    } else {
        "welcome/wordmark-black.png"
    };

    let cooldown_remaining = app
        .lock_cooldown_until
        .and_then(|until| until.checked_duration_since(Instant::now()))
        .map(|d| {
            d.as_secs()
                .saturating_add(if d.subsec_nanos() > 0 { 1 } else { 0 })
        });
    let in_cooldown = cooldown_remaining.is_some_and(|s| s > 0);

    let feedback_text: Option<SharedString> =
        if let Some(secs) = cooldown_remaining.filter(|&s| s > 0) {
            Some(SharedString::from(format!(
                "密码错误次数过多，请 {secs} 秒后再试"
            )))
        } else {
            app.lock_error.clone()
        };

    let unlock_label: &'static str = if app.lock_verifying {
        "正在验证..."
    } else {
        "解锁"
    };
    let unlock_disabled = app.lock_verifying || in_cooldown;

    let win_controls = window_controls::render_window_controls("lock-wc", window, cx);

    v_flex()
        .id("app-lock-screen-root")
        .size_full()
        .bg(p.canvas)
        .text_color(p.text)
        .on_action(cx.listener(|this, _: &QuitApp, _, cx| {
            this.quit_app(cx);
        }))
        .child(
            h_flex()
                .w_full()
                .h(px(window_controls::HEADER_HEIGHT))
                .flex_shrink_0()
                .items_center()
                .child(window_controls::drag_spacer())
                .child(win_controls),
        )
        .child(
            h_flex()
                .flex_1()
                .min_h_0()
                .w_full()
                .items_center()
                .justify_center()
                .px_6()
                .pb_8()
                .child(
                    v_flex()
                        .w_full()
                        .max_w(px(380.0))
                        .p_6()
                        .gap_4()
                        .rounded(px(14.0))
                        .bg(p.surface_raised)
                        .border_1()
                        .border_color(p.border)
                        .shadow_lg()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| {
                            cx.stop_propagation();
                        })
                        .child(
                            v_flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    img(wordmark_path)
                                        .h(px(28.0))
                                        .object_fit(ObjectFit::Contain),
                                )
                                .child(
                                    div()
                                        .pt_1()
                                        .text_base()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(p.text)
                                        .child("应用已锁定"),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(p.text_muted)
                                        .child("请输入锁屏密码以解锁 KiteMail"),
                                ),
                        )
                        .child(
                            v_flex()
                                .w_full()
                                .gap_2()
                                .child(Input::new(&app.lock_passcode_input).mask_toggle())
                                .when_some(feedback_text, |el, msg| {
                                    el.child(
                                        div()
                                            .px_2p5()
                                            .py_1p5()
                                            .rounded(px(6.0))
                                            .bg(p.danger.opacity(0.12))
                                            .text_xs()
                                            .text_color(p.danger)
                                            .child(msg),
                                    )
                                }),
                        )
                        .child(
                            div().w_full().child(
                                Button::new("app-lock-unlock-btn")
                                    .primary()
                                    .w_full()
                                    .disabled(unlock_disabled)
                                    .label(unlock_label)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.submit_unlock_passcode(cx);
                                    })),
                            ),
                        ),
                ),
        )
        .into_any_element()
}
