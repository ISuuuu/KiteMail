//! 独立偏好设置窗口（label = "settings"）的创建与重复打开激活。
//!
//! 对齐 GPUI 旧版 `open_preferences`：单例非模态窗口，默认 920 × 848 逻辑 px、
//! 可缩放、无系统装饰（由前端自绘 46px 双栏顶栏），重复打开时激活已有窗口。

use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::dto::{IpcError, MAIN_WINDOW_LABEL};

pub const SETTINGS_WINDOW_LABEL: &str = "settings";
pub const SETTINGS_WINDOW_TITLE: &str = "偏好设置";
/// GPUI 基准默认窗口尺寸（逻辑 px）。
pub const SETTINGS_WINDOW_WIDTH: f64 = 920.0;
pub const SETTINGS_WINDOW_HEIGHT: f64 = 848.0;
/// 允许缩到窄窗断点（≤640）以下以便验证分支，但不低于内容可用下限。
pub const SETTINGS_WINDOW_MIN_WIDTH: f64 = 420.0;
pub const SETTINGS_WINDOW_MIN_HEIGHT: f64 = 480.0;

fn activate_existing(app: &AppHandle) -> Option<tauri::WebviewWindow> {
    let window = app.get_webview_window(SETTINGS_WINDOW_LABEL)?;
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
    Some(window)
}

/// 页签 id 白名单：仅允许字母数字与下划线，避免 URL 拼注异常
pub(crate) fn sanitize_tab_param(tab: Option<&str>) -> Option<&str> {
    tab.map(str::trim).filter(|t| {
        !t.is_empty() && t.len() <= 40 && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// 打开（或激活）独立设置窗口。由主窗口偏好设置入口（侧栏菜单 / 命令面板 / Ctrl+,）调用。
///
/// 使用 `async fn` 让窗口构建从 Tauri 异步运行时调度到主线程，避免在同步 WebView2 IPC
/// 回调内创建子 WebView2 导致 Windows 死锁白屏；同时先以主题背景色 + `visible(false)` 创建，
/// 待设置页首帧完成绘制后 show，并配 1.5s 后台兜底显窗。
#[tauri::command]
pub async fn open_settings_window(app: AppHandle, tab: Option<String>) -> Result<(), IpcError> {
    let clean_tab = sanitize_tab_param(tab.as_deref()).map(str::to_string);
    if activate_existing(&app).is_some() {
        if let Some(tab_id) = clean_tab {
            let _ = app.emit_to(SETTINGS_WINDOW_LABEL, "settings://select-tab", tab_id);
        }
        return Ok(());
    }

    let tab_segment = clean_tab
        .as_deref()
        .map(|t| format!("&tab={t}"))
        .unwrap_or_default();
    let url = WebviewUrl::App(format!("index.html?window=settings{tab_segment}").into());

    let system_dark = app
        .get_webview_window(MAIN_WINDOW_LABEL)
        .is_some_and(|w| matches!(w.theme(), Ok(tauri::Theme::Dark)));
    let bg_color = if let Some(state) = app.try_state::<crate::commands::AppState>() {
        let raw_prefs = state.prefs.snapshot_raw();
        crate::window_lifecycle::resolve_window_background_color(&raw_prefs, system_dark)
    } else if system_dark {
        tauri::window::Color(19, 19, 21, 255)
    } else {
        tauri::window::Color(241, 241, 241, 255)
    };

    let _window = WebviewWindowBuilder::new(&app, SETTINGS_WINDOW_LABEL, url)
        .title(SETTINGS_WINDOW_TITLE)
        .inner_size(SETTINGS_WINDOW_WIDTH, SETTINGS_WINDOW_HEIGHT)
        .min_inner_size(SETTINGS_WINDOW_MIN_WIDTH, SETTINGS_WINDOW_MIN_HEIGHT)
        .background_color(bg_color)
        .visible(false)
        .resizable(true)
        .decorations(false)
        .center()
        .build()
        .map_err(|e| IpcError::internal(format!("打开设置窗口失败: {e}"), None))?;

    let app_for_fallback = app.clone();
    let _ = std::thread::Builder::new()
        .name("kitemail-settings-reveal-fallback".to_string())
        .spawn(move || {
            std::thread::sleep(Duration::from_millis(1500));
            if let Some(win) = app_for_fallback.get_webview_window(SETTINGS_WINDOW_LABEL) {
                if !win.is_visible().unwrap_or(true) {
                    let _ = win.show();
                    let _ = win.set_focus();
                }
            }
        });

    // 关闭设置窗仅销毁窗口本身；主窗口保留，不影响应用生命周期。
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_window_defaults_match_gpui_baseline() {
        assert_eq!(SETTINGS_WINDOW_LABEL, "settings");
        assert!((SETTINGS_WINDOW_WIDTH - 920.0).abs() < f64::EPSILON);
        assert!((SETTINGS_WINDOW_HEIGHT - 848.0).abs() < f64::EPSILON);
        // 最小宽度必须允许进入窄窗断点（≤640 逻辑 px）验证分支。
        assert!(SETTINGS_WINDOW_MIN_WIDTH <= 640.0);
    }

    #[test]
    fn sanitize_tab_param_allows_valid_ids_only() {
        assert_eq!(sanitize_tab_param(Some(" mailAccounts ")), Some("mailAccounts"));
        assert_eq!(sanitize_tab_param(Some("openPgp_2")), Some("openPgp_2"));
        assert_eq!(sanitize_tab_param(Some("")), None);
        assert_eq!(sanitize_tab_param(Some("../etc")), None);
        assert_eq!(sanitize_tab_param(Some("a&b=1")), None);
        assert_eq!(sanitize_tab_param(None), None);
    }
}
