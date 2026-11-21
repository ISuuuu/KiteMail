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

use std::borrow::Cow;

use gpui_kit::assets::AllAssets;
use gpui_kit::{
    px, size, AppContext, AssetSource, Bounds, KeyBinding, Result, SharedString, WindowBounds,
    WindowOptions,
};
use hylki_mail_core::config;

use crate::app::{
    CloseOverlay, FocusSearch, HylkiApp, NewCompose, OpenCommandPalette, RefreshAllAccounts,
};

const WORDMARK_BLACK: &[u8] = include_bytes!("../../../data/welcome/wordmark-black.png");
const WORDMARK_WHITE: &[u8] = include_bytes!("../../../data/welcome/wordmark-white.png");
#[cfg(target_os = "windows")]
const APP_ICON_PNG: &[u8] =
    include_bytes!("../../../data/icons/hicolor/256x256/apps/co.hyprlab.Hylki.png");

/// 组合 GPUI Kit 完整 Lucide 图标库与 Hylki 内嵌欢迎页 Logo 资源。
struct HylkiAssets;

impl AssetSource for HylkiAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match path {
            "welcome/wordmark-black.png" => Ok(Some(Cow::Borrowed(WORDMARK_BLACK))),
            "welcome/wordmark-white.png" => Ok(Some(Cow::Borrowed(WORDMARK_WHITE))),
            _ => AllAssets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut items = AllAssets.list(path)?;
        for extra in ["welcome/wordmark-black.png", "welcome/wordmark-white.png"] {
            if extra.starts_with(path) {
                items.push(extra.into());
            }
        }
        Ok(items)
    }
}

/// 在 Windows 运行时把嵌入的 256×256 PNG 图标直接设置给当前 UI 线程的顶层窗口，
/// 覆盖窗口左上角标题栏、任务栏与 Alt+Tab 图标（Vista+ CreateIconFromResourceEx 原生支持 PNG）。
#[cfg(target_os = "windows")]
fn apply_window_icon() {
    type Hwnd = *mut core::ffi::c_void;
    type Hicon = *mut core::ffi::c_void;
    const WM_SETICON: u32 = 0x0080;
    const ICON_SMALL: usize = 0;
    const ICON_BIG: usize = 1;
    const LR_DEFAULTCOLOR: u32 = 0x0000;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentThreadId() -> u32;
    }

    #[link(name = "user32")]
    extern "system" {
        fn CreateIconFromResourceEx(
            presbits: *const u8,
            dw_res_size: u32,
            f_icon: i32,
            dw_ver: u32,
            cx_desired: i32,
            cy_desired: i32,
            flags: u32,
        ) -> Hicon;
        fn EnumThreadWindows(
            dw_thread_id: u32,
            lpfn: extern "system" fn(Hwnd, isize) -> i32,
            lparam: isize,
        ) -> i32;
        fn SendMessageW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize;
    }

    struct Icons {
        big: Hicon,
        small: Hicon,
    }

    extern "system" fn enum_proc(hwnd: Hwnd, lparam: isize) -> i32 {
        unsafe {
            let icons = &*(lparam as *const Icons);
            if !icons.big.is_null() {
                SendMessageW(hwnd, WM_SETICON, ICON_BIG, icons.big as isize);
            }
            if !icons.small.is_null() {
                SendMessageW(hwnd, WM_SETICON, ICON_SMALL, icons.small as isize);
            }
        }
        1
    }

    unsafe {
        let big = CreateIconFromResourceEx(
            APP_ICON_PNG.as_ptr(),
            APP_ICON_PNG.len() as u32,
            1,
            0x0003_0000,
            64,
            64,
            LR_DEFAULTCOLOR,
        );
        let small = CreateIconFromResourceEx(
            APP_ICON_PNG.as_ptr(),
            APP_ICON_PNG.len() as u32,
            1,
            0x0003_0000,
            32,
            32,
            LR_DEFAULTCOLOR,
        );
        let icons = Icons { big, small };
        EnumThreadWindows(
            GetCurrentThreadId(),
            enum_proc,
            (&icons as *const Icons) as isize,
        );
    }
}

fn main() {
    let demo_mode = std::env::args().any(|arg| arg == "--demo");

    // 注册 Hylki 欢迎页 Logo 与完整 Lucide 图标资源。
    gpui_kit::application()
        .with_assets(HylkiAssets)
        .run(move |cx| {
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
            gpui_kit::open_window(options, cx, move |window, cx| {
                cx.new(|cx| HylkiApp::new(demo_mode, prefs, window, cx))
            })
            .expect("failed to open Hylki Windows window");

            #[cfg(target_os = "windows")]
            apply_window_icon();
        });
}

#[cfg(test)]
mod tests {
    use gpui_kit::assets::IconName;
    use gpui_kit::AssetSource;

    use super::HylkiAssets;

    #[test]
    fn all_assets_serves_ui_lucide_icons_and_wordmarks() {
        assert!(HylkiAssets
            .load("welcome/wordmark-black.png")
            .unwrap()
            .is_some());
        assert!(HylkiAssets
            .load("welcome/wordmark-white.png")
            .unwrap()
            .is_some());
        for icon in [
            IconName::RefreshCw,
            IconName::Menu,
            IconName::Plus,
            IconName::Inbox,
            IconName::Star,
            IconName::StarFill,
            IconName::Send,
            IconName::FilePen,
            IconName::Archive,
            IconName::Trash,
            IconName::Reply,
            IconName::ReplyAll,
            IconName::Forward,
            IconName::Mail,
            IconName::MailOpen,
            IconName::Copy,
            IconName::Ellipsis,
            IconName::ListFilter,
            IconName::Paperclip,
            IconName::CircleUser,
            IconName::LogOut,
            IconName::X,
        ] {
            let bytes = HylkiAssets
                .load(&icon.path())
                .expect("asset source load should not error");
            assert!(
                bytes.is_some(),
                "icon {:?} ({}) must be present in HylkiAssets",
                icon,
                icon.path()
            );
        }
    }
}
