//! Windows 原生风格合并顶栏的窗口控制按钮（最小化 / 最大化或还原 / 关闭）。
//!
//! 交互语义：
//! - **最小化**：走 `HylkiApp::minimize_window_or_tray`（偏好 `minimize_to_tray` 为 true 时
//!   直接隐藏到系统托盘，否则最小化到任务栏）。
//! - **最大化 / 还原**：标记 `WindowControlArea::Max`，在 Windows 11 上原生支持单击切换
//!   最大化/还原以及悬停 Snap Layouts 贴靠布局。
//! - **关闭**：标记 `WindowControlArea::Close`，原生派发 `WM_CLOSE` 触发
//!   `window.on_window_should_close`（按 `close_to_minimize` 隐藏到托盘或退出）。

use gpui_kit::assets::IconName;
use gpui_kit::component::{h_flex, Icon, Sizable};
use gpui_kit::{
    div, px, Context, InteractiveElement, IntoElement, MouseButton, ParentElement,
    StatefulInteractiveElement, Styled, Window, WindowControlArea,
};

use crate::app::HylkiApp;
use crate::theme;

/// 合并顶栏标准高度（与左栏 / 中栏 / 右栏 48px 严格齐平）。
pub(crate) const HEADER_HEIGHT: f32 = 48.0;
/// Windows 原生风格单颗控制按钮宽度。
const CONTROL_BTN_WIDTH: f32 = 46.0;

/// 渲染 Windows 原生风格的「最小化 / 最大化(还原) / 关闭」三联控制按钮。
pub(crate) fn render_window_controls(
    id_prefix: &'static str,
    window: &Window,
    cx: &mut Context<HylkiApp>,
) -> gpui_kit::AnyElement {
    let p = theme::current(cx);
    let (text, hover_bg, danger, danger_fg) = (p.text, p.accent_soft(), p.danger, p.accent_fg);
    let is_maximized = window.is_maximized();

    h_flex()
        .flex_shrink_0()
        .h_full()
        .items_center()
        // 最小化：不设 WindowControlArea::Min，改为走托盘隐藏偏好逻辑。
        .child(
            div()
                .id((id_prefix, 0u32))
                .w(px(CONTROL_BTN_WIDTH))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .text_color(text)
                .hover(move |s| s.bg(hover_bg))
                .on_mouse_down(MouseButton::Left, |_, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                })
                .on_click(cx.listener(|this, _, window, cx| {
                    this.minimize_window_or_tray(window, cx);
                }))
                .child(Icon::new(IconName::Minus).with_size(px(14.0))),
        )
        // 最大化 / 还原：原生 HTMAXBUTTON（支持 Windows 11 Snap Layout）。
        .child(
            div()
                .id((id_prefix, 1u32))
                .w(px(CONTROL_BTN_WIDTH))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .text_color(text)
                .hover(move |s| s.bg(hover_bg))
                .window_control_area(WindowControlArea::Max)
                .child(
                    Icon::new(if is_maximized {
                        IconName::Copy
                    } else {
                        IconName::Square
                    })
                    .with_size(px(12.0)),
                ),
        )
        // 关闭：原生 HTCLOSE，触发 WM_CLOSE -> on_window_should_close。
        .child(
            div()
                .id((id_prefix, 2u32))
                .w(px(CONTROL_BTN_WIDTH))
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .text_color(text)
                .hover(move |s| s.bg(danger).text_color(danger_fg))
                .window_control_area(WindowControlArea::Close)
                .child(Icon::new(IconName::X).with_size(px(14.0))),
        )
        .into_any_element()
}

/// 空白拖拽区（占据顶栏剩余水平空间，支持鼠标拖动窗口与双击最大化/还原）。
pub(crate) fn drag_spacer() -> gpui_kit::Div {
    div()
        .flex_1()
        .h_full()
        .window_control_area(WindowControlArea::Drag)
}
