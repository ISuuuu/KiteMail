//! 独立写信窗口（WP-10-4，对齐原版 `src/app/compose_host.rs` 的 pop-out）：
//!
//! 写信状态仍然只存在于 [`HylkiApp`] 里（收件人胶囊、附件、草稿自动保存、
//! 发送流程全都不用复制），本窗口只是把同一份写信页渲染到另一个窗口，
//! 与偏好设置窗口的做法一致。
//!
//! 主窗口在 `compose_detached` 为真时改画阅读器，因此同一份表单不会在两处
//! 同时出现；点「收回」后写信回到主窗口右栏；直接关闭本窗口（右上角 `×` 或
//! `Alt+F4`）时，若有未保存内容则拦截并弹出「保存为草稿 / 放弃 / 继续编辑」确认条，
//! 不自动收回主窗口。

use gpui_kit::component::v_flex;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    px, Context, Entity, Focusable, InteractiveElement, IntoElement, ParentElement, Render, Styled,
    Window,
};
use kitemail_mail_core::config;

use crate::app::{CloseOverlay, HylkiApp, SendCompose};
use crate::theme;

pub(crate) struct ComposeWindow {
    app: Entity<HylkiApp>,
    /// 首次渲染后把焦点交给正文，避免每帧抢占（对齐原版 `FocusInitial`）。
    focused_once: bool,
    last_rem_scale: u32,
}

impl ComposeWindow {
    pub(crate) fn new(app: Entity<HylkiApp>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let initial_scale = config::clamp_ui_text_scale(app.read(cx).prefs.ui_text_scale_percent);
        window.set_rem_size(px(16.0 * initial_scale as f32 / 100.0));

        // 关窗（× / Alt+F4）：有未保存内容时拦截并弹出确认条，不自动收回主窗口；
        // 无未保存内容时直接结束写信会话并允许关闭。
        let this_handle = window.window_handle();
        let weak_app = app.downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            let Some(app) = weak_app.upgrade() else {
                return true;
            };
            app.update(cx, |app, cx| {
                if app.compose_window != Some(this_handle) {
                    return true;
                }
                if app.compose_has_unsaved_content(cx) {
                    app.compose_confirm_discard = true;
                    cx.notify();
                    false
                } else {
                    app.compose_window = None;
                    app.discard_compose(cx);
                    true
                }
            })
        });

        cx.observe(&app, |_, _, cx| cx.notify()).detach();
        cx.observe_global::<theme::HylkiThemeState>(|_, cx| cx.notify())
            .detach();

        Self {
            app,
            focused_once: false,
            last_rem_scale: initial_scale,
        }
    }
}

impl Render for ComposeWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let scale_percent =
            config::clamp_ui_text_scale(self.app.read(cx).prefs.ui_text_scale_percent);
        if self.last_rem_scale != scale_percent {
            window.set_rem_size(px(16.0 * scale_percent as f32 / 100.0));
            self.last_rem_scale = scale_percent;
        }
        let p = theme::current(cx);
        let app = self.app.clone();
        let focus_body = !self.focused_once;
        self.focused_once = true;
        // 直接借用主应用实体的写信状态渲染；标题栏画在本窗口里，
        // 因此窗口控制按钮由它承载（`pane_top = true`）。
        let (pane, toasts_overlay) = app.update(cx, |app, cx| {
            if focus_body {
                app.compose_body.focus_handle(cx).focus(window, cx);
            }
            let pane = app.render_compose_pane(window, cx, true, true);
            let overlay = app.render_compose_window_toasts(window, cx);
            (pane, overlay)
        });
        v_flex()
            .relative()
            .size_full()
            .bg(p.canvas)
            .text_color(p.text)
            .on_action(cx.listener(|this, _: &SendCompose, window, cx| {
                this.app
                    .update(cx, |app, cx| app.send_compose_in_window(window, cx));
            }))
            .on_action(cx.listener(|this, _: &CloseOverlay, _, cx| {
                this.app.update(cx, |app, cx| app.request_close_compose(cx));
            }))
            .child(pane)
            .when_some(toasts_overlay, |root, overlay| root.child(overlay))
    }
}
