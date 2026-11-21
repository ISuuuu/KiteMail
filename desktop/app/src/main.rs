//! Native Windows GPUI desktop client for Hylki — program entry point.
//!
//! 按方案文档第一节的目录拆分，本文件仅保留程序入口：
//! `--demo` 参数解析、偏好加载与初始主题安装、WindowOptions（默认 1280x800）、
//! 全局快捷键绑定（方案文档五.3）并启动 [`app::HylkiApp`]。全局状态机在
//! `app.rs`，主题系统在 `theme.rs`，三栏与全局视图在 `ui/`（sidebar /
//! message_list / reader / gallery / contacts / preferences / command_palette）。

mod app;
mod theme;
mod ui;

use gpui_kit::{px, size, AppContext, Bounds, KeyBinding, WindowBounds, WindowOptions};
use hylki_mail_core::config;

use crate::app::{
    CloseOverlay, FocusSearch, HylkiApp, NewCompose, OpenCommandPalette, RefreshAllAccounts,
};

fn main() {
    let demo_mode = std::env::args().any(|arg| arg == "--demo");

    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);

        // 阶段 3：读取持久化偏好并按 theme_id / dark_mode 安装初始主题
        //（dark_mode: None 跟随系统时先保持现有默认亮色，由用户切换时固化）。
        let prefs = config::load_preferences_in(&config::data_dir()).unwrap_or_default();
        theme::apply(
            theme::static_theme_id(&prefs.theme_id),
            prefs.dark_mode.unwrap_or(false),
            cx,
        );

        // 方案文档五.3：全局快捷键（动作监听在 HylkiApp 根节点注册）。
        cx.bind_keys(vec![
            KeyBinding::new("ctrl-n", NewCompose, None),
            KeyBinding::new("ctrl-k", OpenCommandPalette, None),
            KeyBinding::new("ctrl-f", FocusSearch, None),
            KeyBinding::new("ctrl-r", RefreshAllAccounts, None),
            KeyBinding::new("escape", CloseOverlay, None),
        ]);

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(1280.0), px(800.0)),
                cx,
            ))),
            ..Default::default()
        };
        cx.open_window(options, move |window, cx| {
            cx.new(|cx| HylkiApp::new(demo_mode, prefs, window, cx))
        })
        .expect("failed to open Hylki Windows window");
    });
}
