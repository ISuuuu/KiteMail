//! Native Windows GPUI desktop client for Hylki — program entry point.
//!
//! 按方案文档第一节的目录拆分，本文件仅保留程序入口：
//! `--demo` 参数解析、WindowOptions（默认 1280x800）、安装主题并启动
//! [`app::HylkiApp`]。全局状态机在 `app.rs`，主题系统在 `theme.rs`，
//! 三栏视图在 `ui/`（sidebar / message_list / reader / avatar）。

mod app;
mod theme;
mod ui;

use gpui_kit::{px, size, AppContext, Bounds, WindowBounds, WindowOptions};

use crate::app::HylkiApp;

fn main() {
    let demo_mode = std::env::args().any(|arg| arg == "--demo");

    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        // 安装 Hylki 语义色板（亮色经典蓝），使 gpui-kit 组件与三栏配色一致。
        theme::apply("default", false, cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1280.0), px(800.0)),
                cx,
            ))),
            ..Default::default()
        };
        cx.open_window(options, move |window, cx| {
            cx.new(|cx| HylkiApp::new(demo_mode, window, cx))
        })
        .expect("failed to open Hylki Windows window");
    });
}
