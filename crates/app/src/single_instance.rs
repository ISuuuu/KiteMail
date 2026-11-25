//! 单实例保护：同一登录会话里只允许运行一个 KiteMail 进程。
//!
//! 启动时先抢占一个命名互斥量；若已有实例在运行，就通过 Win32 找到它的主窗口
//! （关闭到托盘、窗口处于隐藏状态时同样能找回）还原到前台，然后由 `main` 直接退出。
//!
//! 与 `tray.rs` 一样直接调用 Win32 FFI，不额外引入 `windows` crate 特性。

/// 命名互斥量名。`Local\` 前缀表示按登录会话隔离：同一台机器上不同用户可以各自运行一份。
const MUTEX_NAME: &str = "Local\\KiteMail_SingleInstance_Mutex";

/// 主窗口标题（`main.rs` 建窗与 `HylkiApp::new` 都以该标题命名主窗口，其它窗口用主题 / 固定文案）。
const MAIN_WINDOW_TITLE: &str = "KiteMail";

#[cfg(target_os = "windows")]
mod win {
    use core::ffi::c_void;

    use super::{MAIN_WINDOW_TITLE, MUTEX_NAME};

    type Handle = *mut c_void;
    type Hwnd = *mut c_void;

    const ERROR_ALREADY_EXISTS: u32 = 183;
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const SW_SHOW: i32 = 5;
    const SW_RESTORE: i32 = 9;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateMutexW(attrs: *const c_void, initial_owner: i32, name: *const u16) -> Handle;
        fn CloseHandle(handle: Handle) -> i32;
        fn GetLastError() -> u32;
        fn GetModuleFileNameW(module: Handle, buf: *mut u16, size: u32) -> u32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
        fn QueryFullProcessImageNameW(
            process: Handle,
            flags: u32,
            buf: *mut u16,
            size: *mut u32,
        ) -> i32;
    }

    #[link(name = "user32")]
    extern "system" {
        fn EnumWindows(callback: extern "system" fn(Hwnd, isize) -> i32, lparam: isize) -> i32;
        fn GetWindowThreadProcessId(hwnd: Hwnd, pid: *mut u32) -> u32;
        fn GetWindowTextW(hwnd: Hwnd, buf: *mut u16, max_count: i32) -> i32;
        fn IsIconic(hwnd: Hwnd) -> i32;
        fn IsWindowVisible(hwnd: Hwnd) -> i32;
        fn SetForegroundWindow(hwnd: Hwnd) -> i32;
        fn ShowWindow(hwnd: Hwnd, cmd_show: i32) -> i32;
    }

    /// 命名互斥量守卫；进程退出（或守卫被丢弃）时释放句柄。
    pub(crate) struct SingleInstanceGuard {
        handle: Handle,
    }

    impl Drop for SingleInstanceGuard {
        fn drop(&mut self) {
            if !self.handle.is_null() {
                unsafe {
                    CloseHandle(self.handle);
                }
            }
        }
    }

    /// 抢占单实例互斥量。
    ///
    /// 返回 `None` 表示已有实例在运行（此时已尝试把它的窗口还原到前台），
    /// 调用方应直接退出，不再创建第二个进程。
    pub(crate) fn acquire_or_activate() -> Option<SingleInstanceGuard> {
        match acquire_named(MUTEX_NAME) {
            Some(guard) => Some(guard),
            None => {
                unsafe {
                    activate_existing_window();
                }
                None
            }
        }
    }

    /// 创建（或打开）命名互斥量；已存在时关闭多余句柄并返回 `None`。
    ///
    /// 互斥量创建本身失败（极少见，如权限不足）时按放行处理：宁可多开一个
    /// 进程，也不要让应用启动不了。
    fn acquire_named(name: &str) -> Option<SingleInstanceGuard> {
        let wide = to_wide_null(name);
        unsafe {
            let handle = CreateMutexW(core::ptr::null(), 1, wide.as_ptr());
            if handle.is_null() {
                return Some(SingleInstanceGuard {
                    handle: core::ptr::null_mut(),
                });
            }
            // 必须紧接 CreateMutexW 读取：GetLastError 是每线程的状态。
            if GetLastError() == ERROR_ALREADY_EXISTS {
                CloseHandle(handle);
                return None;
            }
            Some(SingleInstanceGuard { handle })
        }
    }

    /// 找到已有实例的主窗口并还原、置前；隐藏到托盘时同样有效。
    unsafe fn activate_existing_window() {
        let self_name = current_image_file_name();
        let mut found = FoundWindows {
            self_name: &self_name,
            main: 0,
            fallback: 0,
        };
        EnumWindows(enum_window, (&mut found as *mut FoundWindows) as isize);
        let hwnd = if found.main != 0 {
            found.main
        } else {
            found.fallback
        };
        if hwnd == 0 {
            return;
        }
        let hwnd = hwnd as Hwnd;
        ShowWindow(hwnd, SW_SHOW);
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        SetForegroundWindow(hwnd);
    }

    /// `EnumWindows` 回调里累积的查找结果。
    struct FoundWindows<'a> {
        self_name: &'a str,
        /// 标题正好是 `Hylki` 的主窗口。
        main: usize,
        /// 兜底：已有实例的任意可见窗口。
        fallback: usize,
    }

    extern "system" fn enum_window(hwnd: Hwnd, lparam: isize) -> i32 {
        unsafe {
            let found = &mut *(lparam as *mut FoundWindows);
            if hwnd.is_null() {
                return 1;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            // 只认别的进程：本进程此时还没有任何窗口。
            if pid == 0 || pid == std::process::id() {
                return 1;
            }
            // 同名可执行文件才算已有实例，避免误命中其它程序的同名窗口。
            if !process_matches(pid, found.self_name) {
                return 1;
            }
            if found.main == 0 && window_title(hwnd) == MAIN_WINDOW_TITLE {
                found.main = hwnd as usize;
                return 0;
            }
            if found.fallback == 0 && IsWindowVisible(hwnd) != 0 {
                found.fallback = hwnd as usize;
            }
            1
        }
    }

    /// 该进程的可执行文件名是否与当前进程一致。
    unsafe fn process_matches(pid: u32, self_name: &str) -> bool {
        if self_name.is_empty() {
            return true;
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return false;
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(process, 0, buf.as_mut_ptr(), &mut len);
        CloseHandle(process);
        if ok == 0 || len == 0 {
            return false;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        file_name(&path).eq_ignore_ascii_case(self_name)
    }

    /// 当前进程可执行文件的文件名（不含目录）。
    unsafe fn current_image_file_name() -> String {
        let mut buf = [0u16; 1024];
        let len = GetModuleFileNameW(core::ptr::null_mut(), buf.as_mut_ptr(), buf.len() as u32);
        if len == 0 {
            return String::new();
        }
        file_name(&String::from_utf16_lossy(&buf[..len as usize])).to_string()
    }

    fn file_name(path: &str) -> &str {
        path.rsplit(['\\', '/']).next().unwrap_or(path)
    }

    unsafe fn window_title(hwnd: Hwnd) -> String {
        let mut buf = [0u16; 256];
        let len = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        if len <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..len as usize])
    }

    fn to_wide_null(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(core::iter::once(0)).collect()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn named_mutex_allows_only_one_owner() {
            // 测试专用名字，避免与真实运行中的 KiteMail 互斥量冲突。
            let name = "Local\\KiteMail_SingleInstance_Mutex_Test";
            let first = acquire_named(name);
            assert!(first.is_some(), "首次创建互斥量应当成功");
            assert!(acquire_named(name).is_none(), "重复获取应判定已有实例");
            drop(first);
            assert!(acquire_named(name).is_some(), "释放守卫后应当能重新获取");
        }
    }
}

#[cfg(target_os = "windows")]
pub(crate) use win::acquire_or_activate;

#[cfg(not(target_os = "windows"))]
pub(crate) struct SingleInstanceGuard;

#[cfg(not(target_os = "windows"))]
pub(crate) fn acquire_or_activate() -> Option<SingleInstanceGuard> {
    Some(SingleInstanceGuard)
}
