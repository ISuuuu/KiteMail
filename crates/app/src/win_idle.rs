//! Windows 系统空闲时长、前台窗口归属检测。

#[cfg(target_os = "windows")]
mod win {
    use core::ffi::c_void;
    use std::time::Duration;

    type Hwnd = *mut c_void;

    #[repr(C)]
    struct LastInputInfo {
        cb_size: u32,
        dw_time: u32,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetTickCount() -> u32;
    }

    #[link(name = "user32")]
    extern "system" {
        fn GetLastInputInfo(plii: *mut LastInputInfo) -> i32;
        fn GetForegroundWindow() -> Hwnd;
        fn GetWindowThreadProcessId(hwnd: Hwnd, lpdw_process_id: *mut u32) -> u32;
        fn IsWindowVisible(hwnd: Hwnd) -> i32;
        fn IsIconic(hwnd: Hwnd) -> i32;
    }

    /// 返回自上次系统级键盘或鼠标输入以来经过的时长；查询失败时返回 `None`。
    pub(crate) fn system_idle_duration() -> Option<Duration> {
        unsafe {
            let mut info = LastInputInfo {
                cb_size: core::mem::size_of::<LastInputInfo>() as u32,
                dw_time: 0,
            };
            if GetLastInputInfo(&mut info) == 0 {
                return None;
            }
            let now_tick = GetTickCount();
            let idle_ms = now_tick.wrapping_sub(info.dw_time);
            Some(Duration::from_millis(u64::from(idle_ms)))
        }
    }

    /// 当前活动前台窗口是否属于本进程且处于可见未最小化状态。
    pub(crate) fn is_current_process_foreground() -> bool {
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.is_null() {
                return false;
            }
            if IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 {
                return false;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            pid != 0 && pid == std::process::id()
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod stub {
    use std::time::Duration;

    pub(crate) fn system_idle_duration() -> Option<Duration> {
        None
    }

    pub(crate) fn is_current_process_foreground() -> bool {
        true
    }
}

#[cfg(target_os = "windows")]
pub(crate) use win::{is_current_process_foreground, system_idle_duration};

#[cfg(not(target_os = "windows"))]
pub(crate) use stub::{is_current_process_foreground, system_idle_duration};
