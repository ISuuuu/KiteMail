//! Windows 系统托盘图标、右键菜单、窗口隐藏/唤醒与气球桌面通知。
//!
//! 采用独立后台线程托管隐藏消息窗口（`HylkiTrayWindowClass`），避免侵入
//! GPUI 主窗口的 `WndProc`；所有托盘交互事件通过 `mpsc` 通道发回主线程由
//! `HylkiApp` 轮询消费。

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};

/// 托盘图标 / 菜单触发的用户动作。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TrayCommand {
    ShowWindow,
    NewCompose,
    SyncAll,
    Quit,
}

/// 构造托盘图标悬停提示文案。
pub(crate) fn format_tray_tooltip(unread: u32) -> String {
    if unread == 0 {
        "KiteMail".to_string()
    } else {
        format!("KiteMail - {unread} 封未读邮件")
    }
}

/// 构造新邮件气球通知的 `(标题, 正文)`；当 `show_content == false` 时仅展示未读数量而不泄露摘要。
pub(crate) fn format_new_mail_notification(
    delta: u32,
    latest_sender: Option<&str>,
    latest_subject: Option<&str>,
    account_label: Option<&str>,
    show_content: bool,
) -> (String, String) {
    let title = if delta <= 1 {
        "KiteMail 收到新邮件".to_string()
    } else {
        format!("KiteMail 收到 {delta} 封新邮件")
    };
    if !show_content {
        let count = delta.max(1);
        let body = format!("您有 {count} 封新未读邮件");
        return (title, body);
    }
    let sender = latest_sender.map(str::trim).filter(|s| !s.is_empty());
    let subject = latest_subject.map(str::trim).filter(|s| !s.is_empty());
    let body = match (sender, subject) {
        (Some(from), Some(subj)) if delta <= 1 => format!("{from}\n{subj}"),
        (Some(from), Some(subj)) => format!("{from}: {subj}（共 {delta} 封新未读）"),
        (Some(from), None) => format!("来自 {from}"),
        (None, Some(subj)) => subj.to_string(),
        (None, None) => match account_label.map(str::trim).filter(|s| !s.is_empty()) {
            Some(acc) => format!("{acc} 有 {delta} 封新未读邮件"),
            None => format!("收件箱新增 {delta} 封未读邮件"),
        },
    };
    (title, body)
}

/// 将 UTF-8 字符串安全编码并截断写入定长 UTF-16 缓冲区（保证末尾 `\0` 终止）。
pub(crate) fn write_wide_truncated<const N: usize>(buf: &mut [u16; N], text: &str) {
    buf.fill(0);
    if N <= 1 {
        return;
    }
    let mut i = 0;
    for ch in text.chars() {
        let mut units = [0u16; 2];
        let encoded = ch.encode_utf16(&mut units);
        if i + encoded.len() >= N {
            break;
        }
        for &u in encoded.iter() {
            buf[i] = u;
            i += 1;
        }
    }
    buf[i] = 0;
}

#[cfg(target_os = "windows")]
mod win {
    use super::*;
    use core::ffi::c_void;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread::JoinHandle;

    type Hwnd = *mut c_void;
    type Hicon = *mut c_void;
    type Hinstance = *mut c_void;
    type Hmenu = *mut c_void;
    type Hbrush = *mut c_void;
    type Hcursor = *mut c_void;

    const WM_DESTROY: u32 = 0x0002;
    const WM_CLOSE: u32 = 0x0010;
    const WM_COMMAND: u32 = 0x0111;
    const WM_LBUTTONUP: u32 = 0x0202;
    const WM_LBUTTONDBLCLK: u32 = 0x0203;
    const WM_RBUTTONUP: u32 = 0x0205;
    const WM_CONTEXTMENU: u32 = 0x007B;
    const WM_USER: u32 = 0x0400;
    const WM_APP: u32 = 0x8000;

    const WM_TRAYICON: u32 = WM_APP + 1;
    const WM_TRAY_BALLOON: u32 = WM_APP + 2;
    const WM_TRAY_TOOLTIP: u32 = WM_APP + 3;
    const NIN_BALLOONUSERCLICK: u32 = WM_USER + 5;

    const TRAY_ICON_ID: u32 = 1;
    const NIM_ADD: u32 = 0x0000_0000;
    const NIM_MODIFY: u32 = 0x0000_0001;
    const NIM_DELETE: u32 = 0x0000_0002;

    const NIF_MESSAGE: u32 = 0x0000_0001;
    const NIF_ICON: u32 = 0x0000_0002;
    const NIF_TIP: u32 = 0x0000_0004;
    const NIF_INFO: u32 = 0x0000_0010;
    const NIIF_INFO: u32 = 0x0000_0001;

    const MF_STRING: u32 = 0x0000_0000;
    const MF_SEPARATOR: u32 = 0x0000_0800;
    const TPM_RIGHTALIGN: u32 = 0x0008;
    const TPM_BOTTOMALIGN: u32 = 0x0020;
    const TPM_RETURNCMD: u32 = 0x0100;
    const TPM_NONOTIFY: u32 = 0x0080;

    const SW_HIDE: i32 = 0;
    const SW_SHOW: i32 = 5;
    const SW_RESTORE: i32 = 9;

    const GWLP_USERDATA: i32 = -21;
    const LR_DEFAULTCOLOR: u32 = 0x0000;

    const MENU_OPEN: u32 = 1001;
    const MENU_COMPOSE: u32 = 1002;
    const MENU_SYNC: u32 = 1003;
    const MENU_QUIT: u32 = 1004;

    #[repr(C)]
    #[derive(Clone, Copy)]
    pub(crate) struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    impl Default for Guid {
        fn default() -> Self {
            Self {
                data1: 0,
                data2: 0,
                data3: 0,
                data4: [0; 8],
            }
        }
    }

    #[repr(C)]
    pub(crate) struct NotifyIconDataW {
        cb_size: u32,
        h_wnd: Hwnd,
        u_id: u32,
        u_flags: u32,
        u_callback_message: u32,
        h_icon: Hicon,
        sz_tip: [u16; 128],
        dw_state: u32,
        dw_state_mask: u32,
        sz_info: [u16; 256],
        u_timeout_or_version: u32,
        sz_info_title: [u16; 64],
        dw_info_flags: u32,
        guid_item: Guid,
        h_balloon_icon: Hicon,
    }

    impl NotifyIconDataW {
        fn zeroed(hwnd: Hwnd) -> Self {
            Self {
                cb_size: core::mem::size_of::<Self>() as u32,
                h_wnd: hwnd,
                u_id: TRAY_ICON_ID,
                u_flags: 0,
                u_callback_message: 0,
                h_icon: core::ptr::null_mut(),
                sz_tip: [0; 128],
                dw_state: 0,
                dw_state_mask: 0,
                sz_info: [0; 256],
                u_timeout_or_version: 0,
                sz_info_title: [0; 64],
                dw_info_flags: 0,
                guid_item: Guid::default(),
                h_balloon_icon: core::ptr::null_mut(),
            }
        }
    }

    #[repr(C)]
    struct Point {
        x: i32,
        y: i32,
    }

    #[repr(C)]
    struct Msg {
        hwnd: Hwnd,
        message: u32,
        wparam: usize,
        lparam: isize,
        time: u32,
        pt: Point,
        l_private: u32,
    }

    #[repr(C)]
    struct WndClassExW {
        cb_size: u32,
        style: u32,
        lpfn_wnd_proc: Option<unsafe extern "system" fn(Hwnd, u32, usize, isize) -> isize>,
        cb_cls_extra: i32,
        cb_wnd_extra: i32,
        h_instance: Hinstance,
        h_icon: Hicon,
        h_cursor: Hcursor,
        hbr_background: Hbrush,
        lpsz_menu_name: *const u16,
        lpsz_class_name: *const u16,
        h_icon_sm: Hicon,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(lp_module_name: *const u16) -> Hinstance;
        fn GetCurrentThreadId() -> u32;
    }

    #[link(name = "user32")]
    extern "system" {
        fn RegisterClassExW(lpwcx: *const WndClassExW) -> u16;
        fn UnregisterClassW(lp_class_name: *const u16, h_instance: Hinstance) -> i32;
        fn CreateWindowExW(
            dw_ex_style: u32,
            lp_class_name: *const u16,
            lp_window_name: *const u16,
            dw_style: u32,
            x: i32,
            y: i32,
            n_width: i32,
            n_height: i32,
            h_wnd_parent: Hwnd,
            h_menu: Hmenu,
            h_instance: Hinstance,
            lp_param: *mut c_void,
        ) -> Hwnd;
        fn DestroyWindow(h_wnd: Hwnd) -> i32;
        fn DefWindowProcW(h_wnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize;
        fn GetMessageW(
            lp_msg: *mut Msg,
            h_wnd: Hwnd,
            w_msg_filter_min: u32,
            w_msg_filter_max: u32,
        ) -> i32;
        fn TranslateMessage(lp_msg: *const Msg) -> i32;
        fn DispatchMessageW(lp_msg: *const Msg) -> isize;
        fn PostMessageW(h_wnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> i32;
        fn PostQuitMessage(n_exit_code: i32);
        fn SetWindowLongPtrW(h_wnd: Hwnd, n_index: i32, dw_new_long: isize) -> isize;
        fn GetWindowLongPtrW(h_wnd: Hwnd, n_index: i32) -> isize;
        fn RegisterWindowMessageW(lp_string: *const u16) -> u32;
        fn CreateIconFromResourceEx(
            presbits: *const u8,
            dw_res_size: u32,
            f_icon: i32,
            dw_ver: u32,
            cx_desired: i32,
            cy_desired: i32,
            flags: u32,
        ) -> Hicon;
        fn DestroyIcon(h_icon: Hicon) -> i32;
        fn CreatePopupMenu() -> Hmenu;
        fn AppendMenuW(
            h_menu: Hmenu,
            u_flags: u32,
            u_id_new_item: usize,
            lp_new_item: *const u16,
        ) -> i32;
        fn TrackPopupMenuEx(
            h_menu: Hmenu,
            u_flags: u32,
            x: i32,
            y: i32,
            hwnd: Hwnd,
            lptpm: *const c_void,
        ) -> i32;
        fn DestroyMenu(h_menu: Hmenu) -> i32;
        fn GetCursorPos(lp_point: *mut Point) -> i32;
        fn SetForegroundWindow(h_wnd: Hwnd) -> i32;
        fn ShowWindow(h_wnd: Hwnd, n_cmd_show: i32) -> i32;
        fn IsIconic(h_wnd: Hwnd) -> i32;
        fn IsWindowVisible(h_wnd: Hwnd) -> i32;
        fn EnumThreadWindows(
            dw_thread_id: u32,
            lpfn: extern "system" fn(Hwnd, isize) -> i32,
            lparam: isize,
        ) -> i32;
    }

    #[link(name = "shell32")]
    extern "system" {
        fn Shell_NotifyIconW(dw_message: u32, lp_data: *const NotifyIconDataW) -> i32;
    }

    fn to_wide_null(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(core::iter::once(0)).collect()
    }

    struct TrayWindowState {
        tx: Sender<TrayCommand>,
        hicon: Hicon,
        taskbar_created_msg: u32,
        pending_balloon: Arc<Mutex<Option<(String, String)>>>,
        pending_tooltip: Arc<Mutex<Option<String>>>,
        tooltip: String,
    }

    impl TrayWindowState {
        unsafe fn add_tray_icon(&self, hwnd: Hwnd) {
            let mut nid = NotifyIconDataW::zeroed(hwnd);
            nid.u_flags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            nid.u_callback_message = WM_TRAYICON;
            nid.h_icon = self.hicon;
            write_wide_truncated(&mut nid.sz_tip, &self.tooltip);
            Shell_NotifyIconW(NIM_ADD, &nid);
        }

        unsafe fn delete_tray_icon(&self, hwnd: Hwnd) {
            let nid = NotifyIconDataW::zeroed(hwnd);
            Shell_NotifyIconW(NIM_DELETE, &nid);
        }

        unsafe fn update_tooltip(&mut self, hwnd: Hwnd, tip: String) {
            self.tooltip = tip;
            let mut nid = NotifyIconDataW::zeroed(hwnd);
            nid.u_flags = NIF_TIP;
            write_wide_truncated(&mut nid.sz_tip, &self.tooltip);
            Shell_NotifyIconW(NIM_MODIFY, &nid);
        }

        unsafe fn show_balloon(&self, hwnd: Hwnd, title: &str, body: &str) {
            let mut nid = NotifyIconDataW::zeroed(hwnd);
            nid.u_flags = NIF_INFO;
            nid.dw_info_flags = NIIF_INFO;
            write_wide_truncated(&mut nid.sz_info_title, title);
            write_wide_truncated(&mut nid.sz_info, body);
            Shell_NotifyIconW(NIM_MODIFY, &nid);
        }

        unsafe fn show_context_menu(&self, hwnd: Hwnd) {
            let hmenu = CreatePopupMenu();
            if hmenu.is_null() {
                return;
            }
            let open_label = to_wide_null("打开 KiteMail");
            let compose_label = to_wide_null("写新邮件");
            let sync_label = to_wide_null("同步所有账号");
            let quit_label = to_wide_null("退出 KiteMail");

            AppendMenuW(hmenu, MF_STRING, MENU_OPEN as usize, open_label.as_ptr());
            AppendMenuW(
                hmenu,
                MF_STRING,
                MENU_COMPOSE as usize,
                compose_label.as_ptr(),
            );
            AppendMenuW(hmenu, MF_STRING, MENU_SYNC as usize, sync_label.as_ptr());
            AppendMenuW(hmenu, MF_SEPARATOR, 0, core::ptr::null());
            AppendMenuW(hmenu, MF_STRING, MENU_QUIT as usize, quit_label.as_ptr());

            let mut pt = Point { x: 0, y: 0 };
            GetCursorPos(&mut pt);
            SetForegroundWindow(hwnd);
            let cmd = TrackPopupMenuEx(
                hmenu,
                TPM_RIGHTALIGN | TPM_BOTTOMALIGN | TPM_RETURNCMD | TPM_NONOTIFY,
                pt.x,
                pt.y,
                hwnd,
                core::ptr::null(),
            ) as u32;
            DestroyMenu(hmenu);

            match cmd {
                MENU_OPEN => {
                    let _ = self.tx.send(TrayCommand::ShowWindow);
                }
                MENU_COMPOSE => {
                    let _ = self.tx.send(TrayCommand::NewCompose);
                }
                MENU_SYNC => {
                    let _ = self.tx.send(TrayCommand::SyncAll);
                }
                MENU_QUIT => {
                    let _ = self.tx.send(TrayCommand::Quit);
                }
                _ => {}
            }
        }
    }

    unsafe extern "system" fn tray_wnd_proc(
        hwnd: Hwnd,
        msg: u32,
        wparam: usize,
        lparam: isize,
    ) -> isize {
        let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut TrayWindowState;
        if !state_ptr.is_null() {
            let state = &mut *state_ptr;
            if msg == state.taskbar_created_msg && state.taskbar_created_msg != 0 {
                state.add_tray_icon(hwnd);
                return 0;
            }
            match msg {
                WM_TRAYICON => {
                    let event = (lparam as u32) & 0xFFFF;
                    match event {
                        WM_LBUTTONUP | WM_LBUTTONDBLCLK | NIN_BALLOONUSERCLICK => {
                            let _ = state.tx.send(TrayCommand::ShowWindow);
                        }
                        WM_RBUTTONUP | WM_CONTEXTMENU => {
                            state.show_context_menu(hwnd);
                        }
                        _ => {}
                    }
                    return 0;
                }
                WM_TRAY_BALLOON => {
                    let next = state
                        .pending_balloon
                        .lock()
                        .ok()
                        .and_then(|mut guard| guard.take());
                    if let Some((title, body)) = next {
                        state.show_balloon(hwnd, &title, &body);
                    }
                    return 0;
                }
                WM_TRAY_TOOLTIP => {
                    let next = state
                        .pending_tooltip
                        .lock()
                        .ok()
                        .and_then(|mut guard| guard.take());
                    if let Some(tip) = next {
                        state.update_tooltip(hwnd, tip);
                    }
                    return 0;
                }
                WM_COMMAND => {
                    return 0;
                }
                WM_CLOSE => {
                    DestroyWindow(hwnd);
                    return 0;
                }
                WM_DESTROY => {
                    state.delete_tray_icon(hwnd);
                    if !state.hicon.is_null() {
                        DestroyIcon(state.hicon);
                        state.hicon = core::ptr::null_mut();
                    }
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    drop(Box::from_raw(state_ptr));
                    PostQuitMessage(0);
                    return 0;
                }
                _ => {}
            }
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }

    /// 捕获当前 UI 线程的顶层主窗口 HWND。
    pub(crate) fn find_current_thread_main_hwnd() -> usize {
        extern "system" fn enum_proc(hwnd: Hwnd, lparam: isize) -> i32 {
            unsafe {
                let slot = &mut *(lparam as *mut usize);
                if *slot == 0 && !hwnd.is_null() && IsWindowVisible(hwnd) != 0 {
                    *slot = hwnd as usize;
                    return 0;
                }
            }
            1
        }
        extern "system" fn enum_any_proc(hwnd: Hwnd, lparam: isize) -> i32 {
            unsafe {
                let slot = &mut *(lparam as *mut usize);
                if *slot == 0 && !hwnd.is_null() {
                    *slot = hwnd as usize;
                    return 0;
                }
            }
            1
        }
        let mut found: usize = 0;
        unsafe {
            let tid = GetCurrentThreadId();
            EnumThreadWindows(tid, enum_proc, (&mut found as *mut usize) as isize);
            if found == 0 {
                EnumThreadWindows(tid, enum_any_proc, (&mut found as *mut usize) as isize);
            }
        }
        found
    }

    pub(crate) struct TrayHandle {
        rx: Mutex<Receiver<TrayCommand>>,
        tray_hwnd: Arc<AtomicUsize>,
        main_hwnd: AtomicUsize,
        pending_balloon: Arc<Mutex<Option<(String, String)>>>,
        pending_tooltip: Arc<Mutex<Option<String>>>,
        thread: Mutex<Option<JoinHandle<()>>>,
    }

    impl TrayHandle {
        pub(crate) fn start(icon_png: &'static [u8]) -> Option<Self> {
            let (tx, rx) = mpsc::channel();
            let tray_hwnd = Arc::new(AtomicUsize::new(0));
            let tray_hwnd_bg = Arc::clone(&tray_hwnd);
            let pending_balloon = Arc::new(Mutex::new(None));
            let pending_balloon_bg = Arc::clone(&pending_balloon);
            let pending_tooltip = Arc::new(Mutex::new(None));
            let pending_tooltip_bg = Arc::clone(&pending_tooltip);

            let thread = std::thread::Builder::new()
                .name("kitemail-win-tray".to_string())
                .spawn(move || unsafe {
                    let hinstance = GetModuleHandleW(core::ptr::null());
                    let class_name = to_wide_null("KiteMailTrayWindowClass");
                    let wc = WndClassExW {
                        cb_size: core::mem::size_of::<WndClassExW>() as u32,
                        style: 0,
                        lpfn_wnd_proc: Some(tray_wnd_proc),
                        cb_cls_extra: 0,
                        cb_wnd_extra: 0,
                        h_instance: hinstance,
                        h_icon: core::ptr::null_mut(),
                        h_cursor: core::ptr::null_mut(),
                        hbr_background: core::ptr::null_mut(),
                        lpsz_menu_name: core::ptr::null(),
                        lpsz_class_name: class_name.as_ptr(),
                        h_icon_sm: core::ptr::null_mut(),
                    };
                    RegisterClassExW(&wc);

                    let window_title = to_wide_null("KiteMail Tray");
                    let hwnd = CreateWindowExW(
                        0,
                        class_name.as_ptr(),
                        window_title.as_ptr(),
                        0,
                        0,
                        0,
                        0,
                        0,
                        core::ptr::null_mut(),
                        core::ptr::null_mut(),
                        hinstance,
                        core::ptr::null_mut(),
                    );
                    if hwnd.is_null() {
                        UnregisterClassW(class_name.as_ptr(), hinstance);
                        return;
                    }

                    let hicon = CreateIconFromResourceEx(
                        icon_png.as_ptr(),
                        icon_png.len() as u32,
                        1,
                        0x0003_0000,
                        32,
                        32,
                        LR_DEFAULTCOLOR,
                    );
                    let taskbar_msg_name = to_wide_null("TaskbarCreated");
                    let taskbar_created_msg = RegisterWindowMessageW(taskbar_msg_name.as_ptr());

                    let state = Box::new(TrayWindowState {
                        tx,
                        hicon,
                        taskbar_created_msg,
                        pending_balloon: pending_balloon_bg,
                        pending_tooltip: pending_tooltip_bg,
                        tooltip: "KiteMail".to_string(),
                    });
                    let state_ptr = Box::into_raw(state);
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, state_ptr as isize);
                    (*state_ptr).add_tray_icon(hwnd);
                    tray_hwnd_bg.store(hwnd as usize, Ordering::Release);

                    let mut msg = core::mem::zeroed::<Msg>();
                    while GetMessageW(&mut msg, core::ptr::null_mut(), 0, 0) > 0 {
                        TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                    tray_hwnd_bg.store(0, Ordering::Release);
                    UnregisterClassW(class_name.as_ptr(), hinstance);
                })
                .ok()?;

            Some(Self {
                rx: Mutex::new(rx),
                tray_hwnd,
                main_hwnd: AtomicUsize::new(0),
                pending_balloon,
                pending_tooltip,
                thread: Mutex::new(Some(thread)),
            })
        }

        /// 记录 GPUI 主窗口 HWND（须在主 UI 线程调用）。
        pub(crate) fn set_main_hwnd(&self, hwnd: usize) {
            if hwnd != 0 {
                self.main_hwnd.store(hwnd, Ordering::Release);
            }
        }

        fn ensure_main_hwnd(&self) -> Hwnd {
            let mut raw = self.main_hwnd.load(Ordering::Acquire);
            if raw == 0 {
                raw = find_current_thread_main_hwnd();
                if raw != 0 {
                    self.main_hwnd.store(raw, Ordering::Release);
                }
            }
            raw as Hwnd
        }

        /// 将主窗口隐藏到托盘（不在任务栏显示）。
        pub(crate) fn hide_main_window(&self) -> bool {
            let hwnd = self.ensure_main_hwnd();
            if hwnd.is_null() {
                return false;
            }
            unsafe {
                ShowWindow(hwnd, SW_HIDE);
            }
            true
        }

        /// 恢复显示主窗口并前置聚焦。
        pub(crate) fn show_main_window(&self) -> bool {
            let hwnd = self.ensure_main_hwnd();
            if hwnd.is_null() {
                return false;
            }
            unsafe {
                ShowWindow(hwnd, SW_SHOW);
                if IsIconic(hwnd) != 0 {
                    ShowWindow(hwnd, SW_RESTORE);
                }
                SetForegroundWindow(hwnd);
            }
            true
        }

        /// 主窗口当前是否处于可见但最小化状态（供 minimize_to_tray 自动隐藏到托盘）。
        pub(crate) fn is_main_window_minimized(&self) -> bool {
            let hwnd = self.main_hwnd.load(Ordering::Acquire) as Hwnd;
            if hwnd.is_null() {
                return false;
            }
            unsafe { IsWindowVisible(hwnd) != 0 && IsIconic(hwnd) != 0 }
        }

        /// 弹出 Windows 托盘气球通知（`NIF_INFO`）。
        pub(crate) fn show_notification(&self, title: &str, body: &str) {
            let hwnd = self.tray_hwnd.load(Ordering::Acquire) as Hwnd;
            if hwnd.is_null() {
                return;
            }
            if let Ok(mut guard) = self.pending_balloon.lock() {
                *guard = Some((title.to_string(), body.to_string()));
            }
            unsafe {
                PostMessageW(hwnd, WM_TRAY_BALLOON, 0, 0);
            }
        }

        /// 更新托盘图标悬停提示文案。
        pub(crate) fn set_tooltip(&self, tooltip: String) {
            let hwnd = self.tray_hwnd.load(Ordering::Acquire) as Hwnd;
            if hwnd.is_null() {
                return;
            }
            if let Ok(mut guard) = self.pending_tooltip.lock() {
                *guard = Some(tooltip);
            }
            unsafe {
                PostMessageW(hwnd, WM_TRAY_TOOLTIP, 0, 0);
            }
        }

        /// 拉取所有尚未处理的托盘菜单 / 点击命令。
        pub(crate) fn drain_commands(&self) -> Vec<TrayCommand> {
            let mut out = Vec::new();
            if let Ok(rx) = self.rx.lock() {
                while let Ok(cmd) = rx.try_recv() {
                    out.push(cmd);
                }
            }
            out
        }

        /// 主动销毁托盘图标并回收后台线程（退出时调用，避免残留幽灵图标）。
        pub(crate) fn shutdown(&self) {
            let hwnd = self.tray_hwnd.swap(0, Ordering::AcqRel) as Hwnd;
            if !hwnd.is_null() {
                unsafe {
                    PostMessageW(hwnd, WM_CLOSE, 0, 0);
                }
            }
            if let Ok(mut guard) = self.thread.lock() {
                if let Some(handle) = guard.take() {
                    let _ = handle.join();
                }
            }
        }
    }

    impl Drop for TrayHandle {
        fn drop(&mut self) {
            self.shutdown();
        }
    }

    #[cfg(test)]
    pub(crate) fn notify_icon_data_size() -> usize {
        core::mem::size_of::<NotifyIconDataW>()
    }
}

#[cfg(not(target_os = "windows"))]
mod stub {
    use super::*;

    pub(crate) fn find_current_thread_main_hwnd() -> usize {
        0
    }

    pub(crate) struct TrayHandle;

    impl TrayHandle {
        pub(crate) fn start(_icon_png: &'static [u8]) -> Option<Self> {
            None
        }
        pub(crate) fn set_main_hwnd(&self, _hwnd: usize) {}
        pub(crate) fn hide_main_window(&self) -> bool {
            false
        }
        pub(crate) fn show_main_window(&self) -> bool {
            false
        }
        pub(crate) fn is_main_window_minimized(&self) -> bool {
            false
        }
        pub(crate) fn show_notification(&self, _title: &str, _body: &str) {}
        pub(crate) fn set_tooltip(&self, _tooltip: String) {}
        pub(crate) fn drain_commands(&self) -> Vec<TrayCommand> {
            Vec::new()
        }
        pub(crate) fn shutdown(&self) {}
    }
}

#[cfg(target_os = "windows")]
pub(crate) use win::{find_current_thread_main_hwnd, TrayHandle};

#[cfg(not(target_os = "windows"))]
pub(crate) use stub::{find_current_thread_main_hwnd, TrayHandle};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_wide_truncated_preserves_null_terminator_and_utf16_boundaries() {
        let mut buf = [0u16; 11];
        write_wide_truncated(&mut buf, "KiteMail 邮件客户端通知");
        assert_eq!(buf[10], 0);
        let decoded = String::from_utf16_lossy(&buf[..10]);
        assert_eq!(decoded, "KiteMail 邮");

        // 含代理对字符（4 字节 UTF-8 / 2 个 UTF-16 码元）不应截断到半个代理对。
        let mut emoji_buf = [0u16; 4];
        write_wide_truncated(&mut emoji_buf, "ab🎉c");
        // "ab" 占 2，加 "🎉" 需要 2，总长 4 超过 N-1=3，因此只保留 "ab\0"。
        assert_eq!(&emoji_buf, &[b'a' as u16, b'b' as u16, 0, 0]);
    }

    #[test]
    fn tray_tooltip_and_notification_formatting() {
        assert_eq!(format_tray_tooltip(0), "KiteMail");
        assert_eq!(format_tray_tooltip(5), "KiteMail - 5 封未读邮件");

        let (title, body) = format_new_mail_notification(
            1,
            Some("Alice"),
            Some("项目进度更新"),
            Some("work@example.com"),
            true,
        );
        assert_eq!(title, "KiteMail 收到新邮件");
        assert_eq!(body, "Alice\n项目进度更新");

        let (title, body) = format_new_mail_notification(
            3,
            Some("Bob"),
            Some("周报"),
            Some("work@example.com"),
            true,
        );
        assert_eq!(title, "KiteMail 收到 3 封新邮件");
        assert_eq!(body, "Bob: 周报（共 3 封新未读）");

        let (title, body) =
            format_new_mail_notification(2, None, None, Some("work@example.com"), true);
        assert_eq!(title, "KiteMail 收到 2 封新邮件");
        assert_eq!(body, "work@example.com 有 2 封新未读邮件");

        // 关闭内容隐私展示时不得泄露发件人、主题与账号信息。
        let (title, body) = format_new_mail_notification(
            1,
            Some("Alice"),
            Some("机密合同"),
            Some("work@example.com"),
            false,
        );
        assert_eq!(title, "KiteMail 收到新邮件");
        assert_eq!(body, "您有 1 封新未读邮件");

        let (title, body) = format_new_mail_notification(
            4,
            Some("Bob"),
            Some("财报"),
            Some("work@example.com"),
            false,
        );
        assert_eq!(title, "KiteMail 收到 4 封新邮件");
        assert_eq!(body, "您有 4 封新未读邮件");
    }

    #[cfg(all(target_os = "windows", target_pointer_width = "64"))]
    #[test]
    fn notify_icon_data_w_matches_win64_abi_size() {
        // Vista+ NOTIFYICONDATAW 在 64 位 Windows 下的标准结构大小为 976 字节。
        assert_eq!(win::notify_icon_data_size(), 976);
    }
}
