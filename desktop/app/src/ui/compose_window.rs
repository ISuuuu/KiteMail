//! 独立写信窗口（WP-10-4，对齐原版 `src/app/compose_host.rs` 的 pop-out）：
//!
//! 写信状态仍然只存在于 [`HylkiApp`] 里（收件人胶囊、附件、草稿自动保存、
//! 发送流程全都不用复制），本窗口只是把同一份写信页渲染到另一个窗口，
//! 与偏好设置窗口的做法一致。
//!
//! 主窗口在 `compose_detached` 为真时改画阅读器，因此同一份表单不会在两处
//! 同时出现；窗口被关闭（右上角 ×）或点击「收回」后，写信回到主窗口右栏。

use gpui_kit::component::v_flex;
use gpui_kit::{Context, Entity, Focusable, IntoElement, ParentElement, Render, Styled, Window};

use crate::app::HylkiApp;
use crate::theme;

pub(crate) struct ComposeWindow {
    app: Entity<HylkiApp>,
    /// 首次渲染后把焦点交给正文，避免每帧抢占（对齐原版 `FocusInitial`）。
    focused_once: bool,
}

impl ComposeWindow {
    pub(crate) fn new(app: Entity<HylkiApp>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // 关窗（× / Alt+F4）与「收回」都要让主窗口重新接管写信页；
        // 允许关闭，返回 true。
        let weak_app = app.downgrade();
        window.on_window_should_close(cx, move |_, cx| {
            if let Some(app) = weak_app.upgrade() {
                app.update(cx, |app, cx| app.set_compose_detached(false, cx));
            }
            true
        });
        Self {
            app,
            focused_once: false,
        }
    }
}

impl Render for ComposeWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = theme::current(cx);
        let app = self.app.clone();
        let focus_body = !self.focused_once;
        self.focused_once = true;
        // 直接借用主应用实体的写信状态渲染；标题栏画在本窗口里，
        // 因此窗口控制按钮由它承载（`pane_top = true`）。
        app.update(cx, |app, cx| {
            if focus_body {
                app.compose_body.focus_handle(cx).focus(window, cx);
            }
            v_flex()
                .size_full()
                .bg(p.canvas)
                .child(app.render_compose_pane(window, cx, true, true))
        })
    }
}
