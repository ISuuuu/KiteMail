//! 独立偏好设置窗口（label = "settings"）的创建与重复打开激活。
//!
//! 对齐 GPUI 旧版 `open_preferences`：单例非模态窗口，默认 920 × 848 逻辑 px、
//! 可缩放、无系统装饰（由前端自绘 46px 双栏顶栏），重复打开时激活已有窗口。

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::dto::IpcError;

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

/// 打开（或激活）独立设置窗口。由主窗口偏好设置入口（侧栏菜单 / 命令面板 / Ctrl+,）调用。
///
/// `tab`（WP-10）：入口定位载体（对应 GPUI `SettingsOpenTarget::Tab`，如账号管理入口
/// 携带 "mailAccounts"），拼入设置窗 URL 由前端 SettingsWindow 解析 `?tab=`；
/// 仅在新建窗口时生效，重复打开仅激活既有窗口。
#[tauri::command]
pub fn open_settings_window(app: AppHandle, tab: Option<String>) -> Result<(), IpcError> {
    if activate_existing(&app).is_some() {
        return Ok(());
    }

    // 页签 id 白名单：仅允许字母数字与下划线，避免 URL 拼注异常
    let tab_segment = match tab.as_deref().map(str::trim) {
        Some(t)
            if !t.is_empty()
                && t.len() <= 40
                && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') =>
        {
            format!("&tab={t}")
        }
        _ => String::new(),
    };
    let url = WebviewUrl::App(format!("index.html?window=settings{tab_segment}").into());
    let _window = WebviewWindowBuilder::new(&app, SETTINGS_WINDOW_LABEL, url)
        .title(SETTINGS_WINDOW_TITLE)
        .inner_size(SETTINGS_WINDOW_WIDTH, SETTINGS_WINDOW_HEIGHT)
        .min_inner_size(SETTINGS_WINDOW_MIN_WIDTH, SETTINGS_WINDOW_MIN_HEIGHT)
        .resizable(true)
        .decorations(false)
        .center()
        .build()
        .map_err(|e| IpcError::internal(format!("打开设置窗口失败: {e}"), None))?;

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
}
