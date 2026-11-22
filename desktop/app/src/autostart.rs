//! WP-05：Windows 开机自启（登录时启动）。
//!
//! 通过 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 注册表项实现，
//! 值为当前可执行文件路径。仅操作当前用户的 HKCU，不写注册表其他位置。

use std::path::PathBuf;

use winreg::enums::{HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE};
use winreg::RegKey;

/// 自启动注册表键路径（当前用户）。
pub(crate) const RUN_KEY_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// 本应用在 Run 键中的值名。
pub(crate) const RUN_VALUE_NAME: &str = "Hylki";

/// 写入 Run 键的命令行：带引号的当前可执行文件路径。
pub(crate) fn autostart_command() -> Option<String> {
    let exe: PathBuf = std::env::current_exe().ok()?;
    Some(format!("\"{}\"", exe.display()))
}

/// 当前是否已启用开机自启（Run 键中存在本应用值）。
pub(crate) fn is_autostart_enabled() -> bool {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    // 只读打开：键或值不存在即未启用，不创建任何内容。
    let Ok(run_key) = hkcu.open_subkey_with_flags(RUN_KEY_PATH, KEY_QUERY_VALUE) else {
        return false;
    };
    run_key.get_value::<String, _>(RUN_VALUE_NAME).is_ok()
}

/// 启用 / 停用开机自启。返回错误消息（含注册表 IO 失败）。
pub(crate) fn set_autostart_enabled(enable: bool) -> Result<(), String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let run_key = hkcu
        .open_subkey_with_flags(RUN_KEY_PATH, KEY_SET_VALUE | KEY_QUERY_VALUE)
        .or_else(|_| {
            if enable {
                hkcu.create_subkey(RUN_KEY_PATH).map(|(k, _)| k)
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Run key missing",
                ))
            }
        })
        .map_err(|e| format!("打开注册表 Run 键失败: {e}"))?;
    if enable {
        let cmd = autostart_command().ok_or("无法获取当前可执行文件路径")?;
        run_key
            .set_value(RUN_VALUE_NAME, &cmd)
            .map_err(|e| format!("写入自启注册表值失败: {e}"))?;
    } else {
        // 值不存在时删除返回错误，视为已停用而非失败。
        match run_key.delete_value(RUN_VALUE_NAME) {
            Ok(()) => {}
            Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("删除自启注册表值失败: {e}")),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_key_path_and_command_shape() {
        // 注册表键路径与值名（不真实写入 HKCU）。
        assert_eq!(
            RUN_KEY_PATH,
            r"Software\Microsoft\Windows\CurrentVersion\Run"
        );
        assert_eq!(RUN_VALUE_NAME, "Hylki");
        // 命令行是带引号的可执行文件绝对路径。
        if let Some(cmd) = autostart_command() {
            assert!(cmd.starts_with('"'), "{cmd}");
            assert!(cmd.ends_with(".exe\""), "{cmd}");
        }
        // 只读查询不写注册表，能安全调用。
        let _ = is_autostart_enabled();
    }
}
