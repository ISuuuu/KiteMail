//! 单实例保护与同资料目录单进程写入锁。
//!
//! 启动时先抢占预览版独立命名互斥量 `Local\KiteMail_Preview_SingleInstance_Mutex`；
//! 若已有实例在运行，则尝试将其主窗口还原到前台并退出当前进程。
//! 同时按解析后的资料目录路径派生独立互斥量，防止两个不同配置/实例并发写入同一资料目录。

use std::path::Path;

/// 预览版独立命名互斥量名（与 GPUI 主程序 `Local\KiteMail_SingleInstance_Mutex` 隔离）。
pub const PREVIEW_MUTEX_NAME: &str = "Local\\KiteMail_Preview_SingleInstance_Mutex";
const MAIN_WINDOW_TITLE: &str = "KiteMail Preview";

/// 根据资料目录生成唯一的局部命名互斥量名，保证同一资料目录同一时间仅单进程写入。
pub fn data_dir_mutex_name(data_dir: &Path) -> String {
    let resolved = std::fs::canonicalize(data_dir).unwrap_or_else(|_| data_dir.to_path_buf());
    let raw = resolved.to_string_lossy();
    let stripped = raw
        .strip_prefix(r"\\?\")
        .unwrap_or(&raw)
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase();
    // FNV-1a 64-bit hash (deterministic across runs, no extra crate needed).
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in stripped.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("Local\\KiteMail_Preview_DataDir_{hash:016x}")
}

#[cfg(target_os = "windows")]
mod win {
    use core::ffi::c_void;
    use std::path::Path;

    use super::{data_dir_mutex_name, MAIN_WINDOW_TITLE, PREVIEW_MUTEX_NAME};

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

    /// 命名互斥量守卫；进程退出或守卫释放时关闭句柄。
    pub struct SingleInstanceGuard {
        handle: Handle,
    }

    // Win32 mutex handles are process-wide kernel objects safe to move/drop across threads.
    unsafe impl Send for SingleInstanceGuard {}
    unsafe impl Sync for SingleInstanceGuard {}

    impl Drop for SingleInstanceGuard {
        fn drop(&mut self) {
            if !self.handle.is_null() {
                unsafe {
                    CloseHandle(self.handle);
                }
            }
        }
    }

    /// 抢占预览版单实例互斥量；若已有实例则唤醒其主窗口并返回 `None`。
    pub fn acquire_or_activate() -> Option<SingleInstanceGuard> {
        match acquire_named(PREVIEW_MUTEX_NAME) {
            Some(guard) => Some(guard),
            None => {
                unsafe {
                    activate_existing_window();
                }
                None
            }
        }
    }

    /// 抢占指定资料目录的独占写入锁，确保同一资料目录同一时刻仅允许单进程写入。
    pub fn acquire_data_dir_lock(data_dir: &Path) -> Option<SingleInstanceGuard> {
        let name = data_dir_mutex_name(data_dir);
        acquire_named(&name)
    }

    pub(crate) fn acquire_named(name: &str) -> Option<SingleInstanceGuard> {
        let wide = to_wide_null(name);
        unsafe {
            let handle = CreateMutexW(core::ptr::null(), 1, wide.as_ptr());
            if handle.is_null() {
                return Some(SingleInstanceGuard {
                    handle: core::ptr::null_mut(),
                });
            }
            if GetLastError() == ERROR_ALREADY_EXISTS {
                CloseHandle(handle);
                return None;
            }
            Some(SingleInstanceGuard { handle })
        }
    }

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

    struct FoundWindows<'a> {
        self_name: &'a str,
        main: usize,
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
            if pid == 0 || pid == std::process::id() {
                return 1;
            }
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
            let name = format!("Local\\KiteMail_Preview_Test_{}", uuid::Uuid::new_v4());
            let first = acquire_named(&name);
            assert!(first.is_some(), "首次创建互斥量应当成功");
            assert!(acquire_named(&name).is_none(), "重复获取应判定已有实例");
            drop(first);
            assert!(acquire_named(&name).is_some(), "释放守卫后应当能重新获取");
        }

        #[test]
        fn data_dir_lock_allows_only_one_writer_per_dir() {
            let dir_a = std::env::temp_dir().join(format!("km-lock-a-{}", uuid::Uuid::new_v4()));
            let dir_b = std::env::temp_dir().join(format!("km-lock-b-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir_a).unwrap();
            std::fs::create_dir_all(&dir_b).unwrap();

            let guard_a = acquire_data_dir_lock(&dir_a);
            assert!(guard_a.is_some());
            assert!(
                acquire_data_dir_lock(&dir_a).is_none(),
                "同一资料目录不允许第二个进程写入"
            );
            assert!(
                acquire_data_dir_lock(&dir_b).is_some(),
                "不同资料目录互不干扰"
            );

            drop(guard_a);
            assert!(acquire_data_dir_lock(&dir_a).is_some());

            let _ = std::fs::remove_dir_all(&dir_a);
            let _ = std::fs::remove_dir_all(&dir_b);
        }
    }
}

#[cfg(target_os = "windows")]
pub use win::{acquire_data_dir_lock, acquire_or_activate};

#[cfg(not(target_os = "windows"))]
pub struct SingleInstanceGuard;

#[cfg(not(target_os = "windows"))]
pub fn acquire_or_activate() -> Option<SingleInstanceGuard> {
    Some(SingleInstanceGuard)
}

#[cfg(not(target_os = "windows"))]
pub fn acquire_data_dir_lock(_data_dir: &Path) -> Option<SingleInstanceGuard> {
    Some(SingleInstanceGuard)
}
