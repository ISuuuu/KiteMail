//! Native Windows GPUI desktop client for Hylki — program entry point.
//!
//! 按方案文档第一节的目录拆分，本文件仅保留程序入口：
//! `--demo` 参数解析、偏好加载与初始主题安装、WindowOptions（默认 1280x800）、
//! 全局快捷键绑定（方案文档五.3）并启动 [`app::HylkiApp`]。全局状态机在
//! `app.rs`，主题系统在 `theme.rs`，三栏与全局视图在 `ui/`（sidebar /
//! message_list / reader / gallery / contacts / preferences / command_palette）。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod autostart;
mod compose_schedule;
mod single_instance;
mod theme;
mod tray;
mod ui;
#[cfg(windows)]
mod webview_host;

use std::borrow::Cow;

use gpui_kit::assets::AllAssets;
use gpui_kit::{
    px, size, AppContext, AssetSource, Bounds, KeyBinding, Result, SharedString, TitlebarOptions,
    WindowBounds, WindowOptions,
};
use hylki_mail_core::config;

use crate::app::{
    CloseOverlay, CloseSettingsWindow, DeleteSelectedMessage, FocusSearch, HylkiApp, NewCompose,
    OpenCommandPalette, OpenCommandPaletteActions, OpenPreferences, OpenShortcutsWindow, QuitApp,
    RefreshAllAccounts, ReplyAllCurrent, ReplyCurrent, SelectNextMessage, SelectPrevMessage,
    SendCompose, SetReaderPositionBelow, SetReaderPositionHidden, SetReaderPositionRight,
    SingleKeyArchive, SingleKeyBackToList, SingleKeyCompose, SingleKeyDelete, SingleKeyForward,
    SingleKeyMarkSpam, SingleKeyNextCard, SingleKeyNextMessage, SingleKeyPrevCard,
    SingleKeyPrevMessage, SingleKeyReply, SingleKeyReplyAll, SingleKeyToggleRead,
    SingleKeyToggleStar, ToggleAccountsList, ToggleFocusMode, ToggleFullscreen, UndoLastMove,
    ViewMessageSource, ZoomIn, ZoomOut, ZoomReset,
};

const WORDMARK_BLACK: &[u8] = include_bytes!("../../../data/welcome/wordmark-black.png");
const WORDMARK_WHITE: &[u8] = include_bytes!("../../../data/welcome/wordmark-white.png");
const BRAND_ASSETS: &[(&str, &[u8])] = &[
    (
        "brands/outlook.png",
        include_bytes!("../../../data/brands/outlook.png"),
    ),
    (
        "brands/gmail.png",
        include_bytes!("../../../data/brands/gmail.png"),
    ),
    (
        "brands/icloud.png",
        include_bytes!("../../../data/brands/icloud.png"),
    ),
    (
        "brands/yahoo.png",
        include_bytes!("../../../data/brands/yahoo.png"),
    ),
    (
        "brands/proton.png",
        include_bytes!("../../../data/brands/proton.png"),
    ),
    (
        "brands/fastmail.png",
        include_bytes!("../../../data/brands/fastmail.png"),
    ),
    (
        "brands/aol.png",
        include_bytes!("../../../data/brands/aol.png"),
    ),
    (
        "brands/zoho.png",
        include_bytes!("../../../data/brands/zoho.png"),
    ),
    (
        "brands/gmx.png",
        include_bytes!("../../../data/brands/gmx.png"),
    ),
    (
        "brands/yandex.png",
        include_bytes!("../../../data/brands/yandex.png"),
    ),
    (
        "brands/mailcom.png",
        include_bytes!("../../../data/brands/mailcom.png"),
    ),
    (
        "brands/stalwart.png",
        include_bytes!("../../../data/brands/stalwart.png"),
    ),
];

// GPUI Kit 给文本控件设置 `Input` 键盘上下文。匹配阶段就让出这些按键，
// 文本控件才能收到字母和标点。
const OUTSIDE_TEXT_INPUT: Option<&str> = Some("!Input");
#[cfg(target_os = "windows")]
pub(crate) const APP_ICON_PNG: &[u8] =
    include_bytes!("../../../data/icons/hicolor/256x256/apps/co.hyprlab.Hylki.png");
#[cfg(not(target_os = "windows"))]
pub(crate) const APP_ICON_PNG: &[u8] = &[];

const DEFAULT_WINDOW_WIDTH: f32 = 1280.0;
const DEFAULT_WINDOW_HEIGHT: f32 = 800.0;
const MIN_WINDOW_WIDTH: f32 = 640.0;
const MIN_WINDOW_HEIGHT: f32 = 480.0;
const MAX_WINDOW_WIDTH: f32 = 7680.0;
const MAX_WINDOW_HEIGHT: f32 = 4320.0;

/// 从持久化偏好解析初始窗口逻辑尺寸（异常/缺失值回落默认 1280x800）。
fn resolve_window_size(prefs: &config::AppPreferences) -> (f32, f32) {
    match (prefs.window_width, prefs.window_height) {
        (Some(w), Some(h)) if w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0 => (
            w.clamp(MIN_WINDOW_WIDTH, MAX_WINDOW_WIDTH),
            h.clamp(MIN_WINDOW_HEIGHT, MAX_WINDOW_HEIGHT),
        ),
        _ => (DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT),
    }
}

/// 组合 GPUI Kit 完整 Lucide 图标库与 Hylki 内嵌欢迎页 Logo 资源。
struct HylkiAssets;

impl AssetSource for HylkiAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match path {
            "welcome/wordmark-black.png" => Ok(Some(Cow::Borrowed(WORDMARK_BLACK))),
            "welcome/wordmark-white.png" => Ok(Some(Cow::Borrowed(WORDMARK_WHITE))),
            _ => {
                if let Some((_, bytes)) = BRAND_ASSETS
                    .iter()
                    .find(|(asset_path, _)| *asset_path == path)
                {
                    return Ok(Some(Cow::Borrowed(bytes)));
                }
                AllAssets.load(path)
            }
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut items = AllAssets.list(path)?;
        for extra in ["welcome/wordmark-black.png", "welcome/wordmark-white.png"]
            .into_iter()
            .chain(BRAND_ASSETS.iter().map(|(asset_path, _)| *asset_path))
        {
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
pub(crate) fn apply_window_icon() {
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

struct HylkiHttpClient;

impl gpui_kit::http_client::HttpClient for HylkiHttpClient {
    fn user_agent(&self) -> Option<&gpui_kit::http_client::http::HeaderValue> {
        None
    }

    fn proxy(&self) -> Option<&gpui_kit::http_client::Url> {
        None
    }

    fn send(
        &self,
        req: gpui_kit::http_client::Request<gpui_kit::http_client::AsyncBody>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = gpui_kit::Result<
                        gpui_kit::http_client::Response<gpui_kit::http_client::AsyncBody>,
                    >,
                > + Send
                + 'static,
        >,
    > {
        let url = req.uri().to_string();
        Box::pin(async move {
            let (status, content_type, bytes) =
                hylki_mail_core::remote_http::fetch_remote_image_async(url)
                    .await
                    .map_err(std::io::Error::other)?;
            let mut builder = gpui_kit::http_client::Response::builder().status(status);
            if !content_type.is_empty() {
                builder = builder.header("Content-Type", content_type);
            }
            builder
                .body(gpui_kit::http_client::AsyncBody::from(bytes))
                .map_err(|e| std::io::Error::other(e).into())
        })
    }
}

fn main() {
    // 单实例：已有 Hylki 在运行时把它的窗口还原到前台（含隐藏到托盘的情况），
    // 本进程直接退出，不再启动第二个实例。守卫持有到进程结束。
    let Some(_instance_guard) = single_instance::acquire_or_activate() else {
        return;
    };

    let demo_mode = std::env::args().any(|arg| arg == "--demo");

    // 注册 Hylki 欢迎页 Logo 与完整 Lucide 图标资源。
    gpui_kit::application()
        .with_assets(HylkiAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            cx.set_http_client(std::sync::Arc::new(HylkiHttpClient));

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
                // Ctrl+Shift+K：命令面板预填 `>`，仅筛选操作命令（对齐原版）。
                KeyBinding::new("ctrl-shift-k", OpenCommandPaletteActions, None),
                KeyBinding::new("ctrl-f", FocusSearch, None),
                // `/` 亦展开中栏搜索栏（输入框聚焦时正常输入斜杠）。
                KeyBinding::new("/", FocusSearch, OUTSIDE_TEXT_INPUT),
                // F-1 阶段三组合键修正：Ctrl+R 是「回复」（原误绑刷新），刷新仅保留 F5。
                KeyBinding::new("ctrl-r", ReplyCurrent, None),
                KeyBinding::new("ctrl-shift-r", ReplyAllCurrent, None),
                // Ctrl+U 查看 / 收起当前阅读邮件的原文（再按返回正文）。
                KeyBinding::new("ctrl-u", ViewMessageSource, None),
                KeyBinding::new("f5", RefreshAllAccounts, None),
                KeyBinding::new("ctrl-enter", SendCompose, None),
                KeyBinding::new("ctrl-,", OpenPreferences, None),
                KeyBinding::new("ctrl-q", QuitApp, None),
                // 阅读器正文字号缩放（Ctrl+= 放大 / Ctrl+- 缩小 / Ctrl+0 重置）。
                KeyBinding::new("ctrl-=", ZoomIn, None),
                KeyBinding::new("ctrl--", ZoomOut, None),
                KeyBinding::new("ctrl-0", ZoomReset, None),
                KeyBinding::new("delete", DeleteSelectedMessage, OUTSIDE_TEXT_INPUT),
                // F-3：撤销最近一次移动 / 删除 / 归档（状态栏轻量撤销条的快捷键）。
                KeyBinding::new("ctrl-z", UndoLastMove, OUTSIDE_TEXT_INPUT),
                KeyBinding::new("down", SelectNextMessage, OUTSIDE_TEXT_INPUT),
                KeyBinding::new("up", SelectPrevMessage, OUTSIDE_TEXT_INPUT),
                KeyBinding::new("escape", CloseOverlay, None),
                // 阶段二主菜单动作：显示/隐藏账号列表、全屏、快捷键帮助。
                KeyBinding::new("ctrl-shift-a", ToggleAccountsList, None),
                KeyBinding::new("f11", ToggleFullscreen, None),
                KeyBinding::new("ctrl-w", CloseSettingsWindow, None),
                // L-6：阅读器位置（右侧 / 下方 / 关闭）与专注模式（对齐原版 #475）。
                KeyBinding::new("ctrl-alt-1", SetReaderPositionRight, None),
                KeyBinding::new("ctrl-alt-2", SetReaderPositionBelow, None),
                KeyBinding::new("ctrl-alt-3", SetReaderPositionHidden, None),
                KeyBinding::new("ctrl-shift-f", ToggleFocusMode, None),
                // F1 与 `?` 都绑：Windows 上 Shift+/ 实际产生 `?` 键（gpui 会
                // 清除 shift 修饰），而部分布局/平台仍按 shift-/ 上报（原版
                // 同样对两种写法都注册兜底）。
                KeyBinding::new("f1", OpenShortcutsWindow, None),
                KeyBinding::new("shift-/", OpenShortcutsWindow, OUTSIDE_TEXT_INPUT),
                KeyBinding::new("?", OpenShortcutsWindow, OUTSIDE_TEXT_INPUT),
                // ---- F-1 阶段三：单键快捷键（偏好「启用单键快捷键」可整体关闭）----
                // j / k 与 ↓ / ↑ 并存（↓↑ 绑定原动作不受偏好控制）。
                KeyBinding::new("j", SingleKeyNextMessage, OUTSIDE_TEXT_INPUT),
                KeyBinding::new("k", SingleKeyPrevMessage, OUTSIDE_TEXT_INPUT),
                KeyBinding::new("r", SingleKeyReply, OUTSIDE_TEXT_INPUT),
                // 回复全部：Windows 上 Shift+R 上报 {key:"r", shift:true}，"R" 解析后
                // 归一化为同一目标键（parse 对单字符大写键自动加 shift），二者等价；
                // 同时注册 "shift-r" 兜底部分布局 / 平台（与上方 `?` 双绑做法一致）。
                KeyBinding::new("R", SingleKeyReplyAll, OUTSIDE_TEXT_INPUT),
                KeyBinding::new("shift-r", SingleKeyReplyAll, OUTSIDE_TEXT_INPUT),
                KeyBinding::new("f", SingleKeyForward, OUTSIDE_TEXT_INPUT),
                KeyBinding::new("a", SingleKeyArchive, OUTSIDE_TEXT_INPUT),
                KeyBinding::new("d", SingleKeyDelete, OUTSIDE_TEXT_INPUT),
                KeyBinding::new("backspace", SingleKeyDelete, OUTSIDE_TEXT_INPUT),
                // `!` 同 `?`：Windows 上 Shift+1 实际产生 `!` 键（shift 修饰被清除）。
                KeyBinding::new("!", SingleKeyMarkSpam, OUTSIDE_TEXT_INPUT),
                KeyBinding::new("s", SingleKeyToggleStar, OUTSIDE_TEXT_INPUT),
                KeyBinding::new("m", SingleKeyToggleRead, OUTSIDE_TEXT_INPUT),
                KeyBinding::new("c", SingleKeyCompose, OUTSIDE_TEXT_INPUT),
                KeyBinding::new("u", SingleKeyBackToList, OUTSIDE_TEXT_INPUT),
                KeyBinding::new("w", SingleKeyNextCard, OUTSIDE_TEXT_INPUT),
                KeyBinding::new("b", SingleKeyPrevCard, OUTSIDE_TEXT_INPUT),
            ]);

            let (win_w, win_h) = resolve_window_size(&prefs);
            let centered = Bounds::centered(None, size(px(win_w), px(win_h)), cx);
            let window_bounds = if prefs.window_maximized {
                WindowBounds::Maximized(centered)
            } else {
                WindowBounds::Windowed(centered)
            };
            let options = WindowOptions {
                window_bounds: Some(window_bounds),
                titlebar: Some(TitlebarOptions {
                    title: Some("Hylki".into()),
                    appears_transparent: true,
                    traffic_light_position: None,
                }),
                ..Default::default()
            };
            let start_fullscreen = prefs.start_fullscreen;
            let (_window_handle, app_entity) =
                gpui_kit::open_window(options, cx, move |window, cx| {
                    if start_fullscreen {
                        window.toggle_fullscreen();
                    }
                    cx.new(|cx| HylkiApp::new(demo_mode, prefs, window, cx))
                })
                .expect("failed to open Hylki Windows window");

            #[cfg(target_os = "windows")]
            apply_window_icon();

            let main_hwnd = tray::find_current_thread_main_hwnd();
            app_entity.read(cx).attach_main_hwnd(main_hwnd);
        });
}

#[cfg(test)]
mod tests {
    use gpui_kit::assets::IconName;
    use gpui_kit::{AssetSource, KeyBinding, KeyContext, Keymap, Keystroke};

    use super::{
        FocusSearch, HylkiAssets, SingleKeyArchive, SingleKeyDelete, SingleKeyForward,
        SingleKeyToggleStar, OUTSIDE_TEXT_INPUT,
    };

    #[test]
    fn typing_in_text_inputs_bypasses_mail_shortcuts() {
        let keymap = Keymap::new(vec![
            KeyBinding::new("a", SingleKeyArchive, OUTSIDE_TEXT_INPUT),
            KeyBinding::new("s", SingleKeyToggleStar, OUTSIDE_TEXT_INPUT),
            KeyBinding::new("d", SingleKeyDelete, OUTSIDE_TEXT_INPUT),
            KeyBinding::new("f", SingleKeyForward, OUTSIDE_TEXT_INPUT),
            KeyBinding::new("/", FocusSearch, OUTSIDE_TEXT_INPUT),
        ]);
        let root = KeyContext::parse("Root").unwrap();
        let input = KeyContext::parse("Input").unwrap();
        for key in ["a", "s", "d", "f", "/"] {
            let stroke = [Keystroke::parse(key).unwrap()];
            assert_eq!(
                keymap.bindings_for_input(&stroke, &[root.clone()]).0.len(),
                1
            );
            let (bindings, pending) =
                keymap.bindings_for_input(&stroke, &[root.clone(), input.clone()]);
            assert!(bindings.is_empty());
            assert!(!pending);
        }
    }

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
            IconName::Minus,
            IconName::Square,
            IconName::X,
            IconName::Ban,
            IconName::FolderClosed,
            IconName::ArrowLeft,
            IconName::GripVertical,
            IconName::Image,
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

    #[test]
    fn bundled_brand_icons_exist() {
        for (path, _) in super::BRAND_ASSETS {
            let bytes = HylkiAssets
                .load(path)
                .expect("asset source load should not error");
            assert!(
                bytes.is_some_and(|b| !b.is_empty()),
                "brand asset {path} must be present and non-empty in HylkiAssets"
            );
        }
    }

    #[test]
    fn resolve_window_size_uses_persisted_values_and_clamps_outliers() {
        use super::{
            resolve_window_size, DEFAULT_WINDOW_HEIGHT, DEFAULT_WINDOW_WIDTH, MAX_WINDOW_HEIGHT,
            MAX_WINDOW_WIDTH, MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH,
        };
        use hylki_mail_core::config::AppPreferences;

        let defaults = AppPreferences::default();
        assert_eq!(
            resolve_window_size(&defaults),
            (DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT)
        );

        let custom = AppPreferences {
            window_width: Some(1440.0),
            window_height: Some(920.0),
            ..Default::default()
        };
        assert_eq!(resolve_window_size(&custom), (1440.0, 920.0));

        let tiny = AppPreferences {
            window_width: Some(100.0),
            window_height: Some(100.0),
            ..Default::default()
        };
        assert_eq!(
            resolve_window_size(&tiny),
            (MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT)
        );

        let huge = AppPreferences {
            window_width: Some(99999.0),
            window_height: Some(99999.0),
            ..Default::default()
        };
        assert_eq!(
            resolve_window_size(&huge),
            (MAX_WINDOW_WIDTH, MAX_WINDOW_HEIGHT)
        );

        let invalid = AppPreferences {
            window_width: Some(f32::NAN),
            window_height: Some(800.0),
            ..Default::default()
        };
        assert_eq!(
            resolve_window_size(&invalid),
            (DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT)
        );
    }
}
