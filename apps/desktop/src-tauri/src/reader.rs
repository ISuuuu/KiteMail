//! Tauri main-thread native reader lifecycle, layout/visibility coordinator,
//! and isolated Windows WebView2 child-HWND prototype host.
//!
//! [MERGE-POINT: kitemail-windows-mail-reader]
//! Once `crates/windows-mail-reader` (`kitemail-windows-mail-reader`) is merged into
//! the repository:
//! 1. Replace `build_prototype_reader_document` / `ReaderAppearance` below with
//!    `kitemail_windows_mail_reader::{DocumentAppearance, build_body_document, build_document, build_thread_document}`.
//! 2. Replace `WindowsWebView2Reader` in `platform_win` with
//!    `kitemail_windows_mail_reader::WebView2Host` (passing the Tauri `"main"` window HWND,
//!    isolated `reader_webview2_user_data_dir`, bounds/clip/visible, zoom, and status callback).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::dto::{
    BodyStatusDto, IpcError, ReaderStatusEvent, RectDto, SetReaderLayoutInput,
    SetReaderLayoutOutput, EVENT_READER_STATUS, MAIN_WINDOW_LABEL,
};
use crate::window_lifecycle::{
    compute_clipped_rects, logical_rect_to_physical, resolve_effective_dpr, PhysicalRect,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReaderAppearance {
    pub theme_id: String,
    pub dark: bool,
    pub zoom_percent: u32,
    pub block_remote_images: bool,
}

impl ReaderAppearance {
    pub fn body_colors(&self) -> (&'static str, &'static str) {
        if self.dark {
            ("#202124", "#e8eaed")
        } else {
            ("#ffffff", "#202124")
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReaderDocumentSpec {
    pub session_id: String,
    pub account_id: u32,
    pub message_id: u32,
    pub subject: String,
    pub sender: String,
    pub plain_text: String,
    pub html_body: Option<String>,
    pub has_thread: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppliedReaderLayout {
    pub revision: u64,
    pub clip: PhysicalRect,
    pub bounds: PhysicalRect,
    pub visible: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutApplyOutcome {
    Ignored,
    Applied {
        clip: PhysicalRect,
        bounds: PhysicalRect,
        visible: bool,
    },
}

/// Sink for platform-specific main-thread WebView2 operations and status event emission.
pub trait ReaderPlatformSink: Send + Sync {
    fn is_native_supported(&self) -> bool;
    fn load_html(&self, session_id: String, account_id: u32, message_id: u32, html_bytes: Vec<u8>);
    fn apply_layout(&self, clip: PhysicalRect, bounds: PhysicalRect, visible: bool);
    fn set_visible(&self, visible: bool);
    fn set_zoom(&self, zoom_percent: u32);
    fn close(&self);
    fn emit_status(&self, event: ReaderStatusEvent);
}

/// No-op sink used in unit tests or non-Windows/headless environments.
#[cfg_attr(windows, allow(dead_code))]
pub struct NullReaderSink;

impl ReaderPlatformSink for NullReaderSink {
    fn is_native_supported(&self) -> bool {
        false
    }

    fn load_html(
        &self,
        _session_id: String,
        _account_id: u32,
        _message_id: u32,
        _html_bytes: Vec<u8>,
    ) {
    }

    fn apply_layout(&self, _clip: PhysicalRect, _bounds: PhysicalRect, _visible: bool) {}
    fn set_visible(&self, _visible: bool) {}
    fn set_zoom(&self, _zoom_percent: u32) {}
    fn close(&self) {}
    fn emit_status(&self, _event: ReaderStatusEvent) {}
}

#[derive(Default)]
struct ReaderCoordinatorState {
    active_session_id: Option<String>,
    last_layout_revision: Option<u64>,
    active_document: Option<ReaderDocumentSpec>,
    last_layout: Option<AppliedReaderLayout>,
    locked: bool,
    reader_hidden_pref: bool,
}

/// Pure, testable coordinator guarding `readerSessionId` and `layoutRevision` monotonicity.
pub struct ReaderManager {
    state: Mutex<ReaderCoordinatorState>,
    sink: Arc<dyn ReaderPlatformSink>,
}

impl ReaderManager {
    pub fn new(sink: Arc<dyn ReaderPlatformSink>) -> Self {
        Self {
            state: Mutex::new(ReaderCoordinatorState::default()),
            sink,
        }
    }

    /// Start a new reader session for a selected message, resetting the layout revision guard.
    pub fn begin_session(
        &self,
        doc: ReaderDocumentSpec,
        appearance: &ReaderAppearance,
        reader_enabled: bool,
    ) -> BodyStatusDto {
        let should_load_native = doc.html_body.as_deref().is_some_and(supports_document);
        let session_id = doc.session_id.clone();
        let account_id = doc.account_id;
        let message_id = doc.message_id;

        {
            let mut guard = self.state.lock().expect("reader mutex poisoned");
            guard.reader_hidden_pref = !reader_enabled;
            if guard.locked {
                return BodyStatusDto::Unavailable;
            }
            guard.active_session_id = Some(session_id.clone());
            guard.last_layout_revision = None;
            guard.last_layout = None;
            guard.active_document = Some(doc.clone());
        }

        if !reader_enabled {
            self.sink.set_visible(false);
            let status = BodyStatusDto::Ready;
            self.sink.emit_status(ReaderStatusEvent {
                reader_session_id: session_id,
                account_id,
                message_id,
                body_status: status,
                detail: None,
            });
            return status;
        }

        if should_load_native && self.sink.is_native_supported() {
            let html = doc.html_body.as_deref().unwrap_or("");
            let document_bytes = build_prototype_reader_document(html, appearance);
            self.sink
                .load_html(session_id.clone(), account_id, message_id, document_bytes);
            self.sink.set_zoom(appearance.zoom_percent);
            self.sink.emit_status(ReaderStatusEvent {
                reader_session_id: session_id,
                account_id,
                message_id,
                body_status: BodyStatusDto::Loading,
                detail: None,
            });
            BodyStatusDto::Loading
        } else {
            self.sink.set_visible(false);
            self.sink.emit_status(ReaderStatusEvent {
                reader_session_id: session_id,
                account_id,
                message_id,
                body_status: BodyStatusDto::Ready,
                detail: None,
            });
            BodyStatusDto::Ready
        }
    }

    /// Validate and apply a layout/visibility update, discarding stale sessions or out-of-order revisions.
    pub fn apply_layout(
        &self,
        input: &SetReaderLayoutInput,
        window_dpr: f64,
    ) -> Result<SetReaderLayoutOutput, IpcError> {
        if input.reader_session_id.trim().is_empty() {
            return Err(IpcError::invalid_input("readerSessionId 不能为空", None));
        }
        validate_rect(input.content_rect, "contentRect")?;
        validate_rect(input.clip_rect, "clipRect")?;
        if let Some(dpr) = input.device_pixel_ratio {
            if !dpr.is_finite() || dpr <= 0.0 {
                return Err(IpcError::invalid_input(
                    "devicePixelRatio 必须为正有限数",
                    None,
                ));
            }
        }

        let decision = {
            let mut guard = self.state.lock().expect("reader mutex poisoned");
            if guard.locked || guard.reader_hidden_pref {
                LayoutApplyOutcome::Ignored
            } else if guard.active_session_id.as_deref() != Some(input.reader_session_id.as_str()) {
                LayoutApplyOutcome::Ignored
            } else if guard
                .last_layout_revision
                .is_some_and(|prev| input.layout_revision <= prev)
            {
                LayoutApplyOutcome::Ignored
            } else {
                guard.last_layout_revision = Some(input.layout_revision);
                let dpr = resolve_effective_dpr(input.device_pixel_ratio, window_dpr);
                let content_px = logical_rect_to_physical(input.content_rect, dpr);
                let clip_px = logical_rect_to_physical(input.clip_rect, dpr);
                let clipped = compute_clipped_rects(content_px, clip_px);
                let (clip, bounds, visible) = match clipped {
                    Some((clip, bounds)) => (clip, bounds, input.visible),
                    None => (
                        PhysicalRect {
                            left: 0,
                            top: 0,
                            right: 0,
                            bottom: 0,
                        },
                        PhysicalRect {
                            left: 0,
                            top: 0,
                            right: 0,
                            bottom: 0,
                        },
                        false,
                    ),
                };
                guard.last_layout = Some(AppliedReaderLayout {
                    revision: input.layout_revision,
                    clip,
                    bounds,
                    visible,
                });
                LayoutApplyOutcome::Applied {
                    clip,
                    bounds,
                    visible,
                }
            }
        };

        match decision {
            LayoutApplyOutcome::Ignored => Ok(SetReaderLayoutOutput {
                applied: false,
                reader_session_id: input.reader_session_id.clone(),
                layout_revision: input.layout_revision,
            }),
            LayoutApplyOutcome::Applied {
                clip,
                bounds,
                visible,
            } => {
                self.sink.apply_layout(clip, bounds, visible);
                Ok(SetReaderLayoutOutput {
                    applied: true,
                    reader_session_id: input.reader_session_id.clone(),
                    layout_revision: input.layout_revision,
                })
            }
        }
    }

    pub fn set_reader_enabled(&self, enabled: bool) {
        let mut guard = self.state.lock().expect("reader mutex poisoned");
        guard.reader_hidden_pref = !enabled;
        if !enabled {
            guard.last_layout = None;
            drop(guard);
            self.sink.set_visible(false);
        }
    }

    pub fn update_appearance(&self, appearance: &ReaderAppearance) {
        let (doc, reader_enabled, locked) = {
            let guard = self.state.lock().expect("reader mutex poisoned");
            (
                guard.active_document.clone(),
                !guard.reader_hidden_pref,
                guard.locked,
            )
        };
        if locked || !reader_enabled {
            return;
        }
        if let Some(doc) = doc {
            if doc.html_body.as_deref().is_some_and(supports_document)
                && self.sink.is_native_supported()
            {
                let html = doc.html_body.as_deref().unwrap_or("");
                let bytes = build_prototype_reader_document(html, appearance);
                self.sink
                    .load_html(doc.session_id, doc.account_id, doc.message_id, bytes);
                self.sink.set_zoom(appearance.zoom_percent);
            }
        }
    }

    #[allow(dead_code)]
    pub fn set_locked(&self, locked: bool) {
        let mut guard = self.state.lock().expect("reader mutex poisoned");
        guard.locked = locked;
        if locked {
            guard.active_session_id = None;
            guard.last_layout_revision = None;
            guard.active_document = None;
            guard.last_layout = None;
            drop(guard);
            self.sink.set_visible(false);
        }
    }

    pub fn close(&self) {
        let mut guard = self.state.lock().expect("reader mutex poisoned");
        guard.active_session_id = None;
        guard.last_layout_revision = None;
        guard.active_document = None;
        guard.last_layout = None;
        drop(guard);
        self.sink.close();
    }

    #[cfg(test)]
    pub fn snapshot_test_state(
        &self,
    ) -> (Option<String>, Option<u64>, Option<AppliedReaderLayout>) {
        let guard = self.state.lock().expect("reader mutex poisoned");
        (
            guard.active_session_id.clone(),
            guard.last_layout_revision,
            guard.last_layout.clone(),
        )
    }
}

fn validate_rect(rect: RectDto, label: &str) -> Result<(), IpcError> {
    if !rect.x.is_finite()
        || !rect.y.is_finite()
        || !rect.width.is_finite()
        || !rect.height.is_finite()
    {
        return Err(IpcError::invalid_input(
            format!("{label} 包含非法非有限坐标"),
            None,
        ));
    }
    if rect.width < 0.0 || rect.height < 0.0 {
        return Err(IpcError::invalid_input(
            format!("{label} 宽高不能为负数"),
            None,
        ));
    }
    Ok(())
}

pub fn supports_document(html: &str) -> bool {
    !html.trim().is_empty()
}

/// Prototype sandboxed HTML document builder.
/// [MERGE-POINT: kitemail-windows-mail-reader] Replace with `kitemail_windows_mail_reader::build_body_document`
/// and `kitemail_windows_mail_reader::build_thread_document` once merged.
pub fn build_prototype_reader_document(html: &str, appearance: &ReaderAppearance) -> Vec<u8> {
    let escape_attr = |value: &str| {
        value
            .replace('&', "&amp;")
            .replace('"', "&quot;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    let (bg, fg) = appearance.body_colors();
    let scheme = if appearance.dark { "dark" } else { "light" };
    let zoom = appearance.zoom_percent.clamp(50, 250);
    let csp_img = if appearance.block_remote_images {
        "data:"
    } else {
        "data: http: https:"
    };
    let inner = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
        <meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src {csp_img}; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'\">\
        <style>:root{{color-scheme:{scheme};zoom:{zoom}%}}body{{margin:16px;font:14px/1.6 'Microsoft YaHei UI',system-ui,sans-serif;background:{bg};color:{fg};word-break:break-word}}img{{max-width:100%;height:auto}}a{{color:#3584e4}}</style>\
        </head><body>{html}</body></html>"
    );
    let escaped = escape_attr(&inner);
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
        <style>:root{{color-scheme:{scheme}}}html,body{{height:100%;margin:0}}body{{background:{bg};color:{fg};display:flex;flex-direction:column}}iframe{{flex:1;width:100%;border:0;min-height:0}}</style>\
        </head><body><iframe sandbox=\"allow-same-origin allow-popups\" srcdoc=\"{escaped}\"></iframe></body></html>"
    )
    .into_bytes()
}

/// Windows WebView2 child-HWND prototype sink bound to the Tauri main window.
#[cfg(windows)]
pub mod platform_win {
    use super::*;
    use std::cell::RefCell;
    use std::os::windows::ffi::OsStrExt;
    use std::rc::{Rc, Weak};

    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use tauri::Emitter;
    use webview2_com::{
        CreateCoreWebView2ControllerCompletedHandler,
        CreateCoreWebView2EnvironmentCompletedHandler, Microsoft::Web::WebView2::Win32::*,
        NavigationCompletedEventHandler, NavigationStartingEventHandler,
        NewWindowRequestedEventHandler, ProcessFailedEventHandler,
        WebResourceRequestedEventHandler,
    };
    use windows::{
        core::{w, Interface},
        Win32::{
            Foundation::{HWND, RECT},
            UI::Shell::{SHCreateMemStream, ShellExecuteW},
            UI::WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, SetWindowPos, ShowWindow, SWP_NOACTIVATE,
                SWP_NOZORDER, SW_HIDE, SW_SHOWNOACTIVATE, WINDOW_EX_STYLE, WS_CHILD,
                WS_CLIPCHILDREN,
            },
        },
    };

    const READER_DOC_BASE: &str = "https://kitemail.invalid/reader";

    fn reader_doc_url(version: u64) -> String {
        format!("{READER_DOC_BASE}/{version}")
    }

    fn is_allowed_external_url(raw: &str) -> bool {
        let Ok(url) = url::Url::parse(raw) else {
            return false;
        };
        match url.scheme() {
            "http" | "https" => {
                url.host_str().is_some() && url.username().is_empty() && url.password().is_none()
            }
            "mailto" => !url.path().is_empty(),
            _ => false,
        }
    }

    fn open_external_url(raw: &str) {
        if !is_allowed_external_url(raw) {
            return;
        }
        let wide: Vec<u16> = raw.encode_utf16().chain(Some(0)).collect();
        unsafe {
            let _ = ShellExecuteW(
                None,
                w!("open"),
                windows::core::PCWSTR(wide.as_ptr()),
                None,
                None,
                SW_SHOWNOACTIVATE,
            );
        }
    }

    struct HostInner {
        app_handle: tauri::AppHandle,
        clip_window: HWND,
        controller: Option<ICoreWebView2Controller>,
        document: Vec<u8>,
        version: u64,
        navigation_id: Option<u64>,
        active_session_id: String,
        active_account_id: u32,
        active_message_id: u32,
        bounds: RECT,
        clip: RECT,
        visible: bool,
        has_area: bool,
        window_shown: bool,
        loaded: bool,
        failed: bool,
        closed: bool,
        zoom_factor: f64,
    }

    struct WindowsWebView2Reader {
        inner: Rc<RefCell<HostInner>>,
    }

    thread_local! {
        static MAIN_THREAD_HOST: RefCell<Option<WindowsWebView2Reader>> = const { RefCell::new(None) };
        static MAIN_THREAD_ENV: RefCell<Option<ICoreWebView2Environment>> = const { RefCell::new(None) };
    }

    impl WindowsWebView2Reader {
        fn create(
            app_handle: tauri::AppHandle,
            parent_hwnd: HWND,
            user_data_dir: PathBuf,
        ) -> windows::core::Result<Self> {
            let clip_window = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    w!("STATIC"),
                    w!(""),
                    WS_CHILD | WS_CLIPCHILDREN,
                    0,
                    0,
                    0,
                    0,
                    Some(parent_hwnd),
                    None,
                    None,
                    None,
                )?
            };
            let inner = Rc::new(RefCell::new(HostInner {
                app_handle,
                clip_window,
                controller: None,
                document: Vec::new(),
                version: 0,
                navigation_id: None,
                active_session_id: String::new(),
                active_account_id: 0,
                active_message_id: 0,
                bounds: RECT::default(),
                clip: RECT::default(),
                visible: false,
                has_area: false,
                window_shown: false,
                loaded: false,
                failed: false,
                closed: false,
                zoom_factor: 1.0,
            }));

            let existing_env = MAIN_THREAD_ENV.with(|cell| cell.borrow().clone());
            if let Some(env) = existing_env {
                Self::create_controller(&env, clip_window, Rc::downgrade(&inner));
                return Ok(Self { inner });
            }

            let weak = Rc::downgrade(&inner);
            let env_handler = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
                move |result, environment| {
                    let env = environment.filter(|_| result.is_ok());
                    MAIN_THREAD_ENV.with(|cell| {
                        *cell.borrow_mut() = env.clone();
                    });
                    if let (Some(env), Some(inner_rc)) = (env, weak.upgrade()) {
                        let clip_hwnd = inner_rc.borrow().clip_window;
                        Self::create_controller(&env, clip_hwnd, Rc::downgrade(&inner_rc));
                    } else if let Some(inner_rc) = weak.upgrade() {
                        Self::mark_failed(&inner_rc, "WebView2 环境初始化失败");
                    }
                    Ok(())
                },
            ));

            let _ = std::fs::create_dir_all(&user_data_dir);
            let wide_dir: Vec<u16> = user_data_dir
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect();
            let options = webview2_com::CoreWebView2EnvironmentOptions::default();
            unsafe { options.set_is_custom_crash_reporting_enabled(true) };
            let options = ICoreWebView2EnvironmentOptions::from(options);

            let res = unsafe {
                CreateCoreWebView2EnvironmentWithOptions(
                    windows::core::PCWSTR::null(),
                    windows::core::PCWSTR(wide_dir.as_ptr()),
                    &options,
                    &env_handler,
                )
            };
            if let Err(err) = res {
                unsafe {
                    let _ = DestroyWindow(clip_window);
                }
                return Err(err);
            }
            Ok(Self { inner })
        }

        fn create_controller(
            env: &ICoreWebView2Environment,
            parent: HWND,
            weak: Weak<RefCell<HostInner>>,
        ) {
            let env_clone = env.clone();
            let handler_weak = weak.clone();
            let handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
                move |result, controller| {
                    let Some(state) = handler_weak.upgrade() else {
                        if let Some(ctrl) = controller {
                            unsafe {
                                let _ = ctrl.Close();
                            }
                        }
                        return Ok(());
                    };
                    let Some(controller) = controller.filter(|_| result.is_ok()) else {
                        Self::mark_failed(&state, "WebView2 控制器创建失败");
                        return Ok(());
                    };
                    if state.borrow().closed {
                        unsafe {
                            let _ = controller.Close();
                        }
                        return Ok(());
                    }
                    if unsafe { Self::configure(&env_clone, &controller, Rc::downgrade(&state)) }
                        .is_err()
                    {
                        unsafe {
                            let _ = controller.Close();
                        }
                        Self::mark_failed(&state, "WebView2 配置失败");
                        return Ok(());
                    }
                    let (bounds, shown, url, zoom, has_doc) = {
                        let mut inner = state.borrow_mut();
                        inner.controller = Some(controller.clone());
                        (
                            inner.bounds,
                            inner.visible && inner.has_area,
                            reader_doc_url(inner.version),
                            inner.zoom_factor,
                            !inner.document.is_empty(),
                        )
                    };
                    unsafe {
                        let _ = controller.SetBounds(bounds);
                        let _ = controller.SetZoomFactor(zoom);
                        let _ = controller.SetIsVisible(shown);
                    }
                    if has_doc {
                        let _ = unsafe { Self::navigate(&controller, &url) };
                    }
                    Ok(())
                },
            ));

            let created = unsafe {
                if let Ok(env10) = env.cast::<ICoreWebView2Environment10>() {
                    if let Ok(opts) = env10.CreateCoreWebView2ControllerOptions() {
                        if opts.SetIsInPrivateModeEnabled(true).is_ok() {
                            env10.CreateCoreWebView2ControllerWithOptions(parent, &opts, &handler)
                        } else {
                            env.CreateCoreWebView2Controller(parent, &handler)
                        }
                    } else {
                        env.CreateCoreWebView2Controller(parent, &handler)
                    }
                } else {
                    env.CreateCoreWebView2Controller(parent, &handler)
                }
            };
            if created.is_err() {
                if let Some(state) = weak.upgrade() {
                    Self::mark_failed(&state, "无法创建 WebView2 控制器");
                }
            }
        }

        unsafe fn configure(
            env: &ICoreWebView2Environment,
            controller: &ICoreWebView2Controller,
            weak: Weak<RefCell<HostInner>>,
        ) -> windows::core::Result<()> {
            let view = controller.CoreWebView2()?;
            if let Ok(settings) = view.Settings() {
                let _ = settings.SetAreDevToolsEnabled(false);
                let _ = settings.SetIsStatusBarEnabled(false);
            }
            let mut token = 0;

            let nav_weak = weak.clone();
            let nav = NavigationStartingEventHandler::create(Box::new(move |_, args| {
                if let (Some(state), Some(args)) = (nav_weak.upgrade(), args) {
                    let mut uri = windows::core::PWSTR::null();
                    unsafe { args.Uri(&mut uri)? };
                    let url = webview2_com::take_pwstr(uri);
                    let mut inner = state.borrow_mut();
                    let allowed = !inner.loaded && url == reader_doc_url(inner.version);
                    if allowed {
                        let mut id = 0;
                        unsafe { args.NavigationId(&mut id)? };
                        inner.navigation_id = Some(id);
                    }
                    unsafe { args.SetCancel(!allowed)? };
                }
                Ok(())
            }));
            view.add_NavigationStarting(&nav, &mut token)?;

            let comp_weak = weak.clone();
            let comp = NavigationCompletedEventHandler::create(Box::new(move |_, args| {
                if let (Some(state), Some(args)) = (comp_weak.upgrade(), args) {
                    let mut id = 0;
                    let mut success = windows::core::BOOL::default();
                    unsafe {
                        args.NavigationId(&mut id)?;
                        args.IsSuccess(&mut success)?;
                    }
                    let emit = {
                        let mut inner = state.borrow_mut();
                        if !inner.closed && inner.navigation_id == Some(id) {
                            inner.loaded = success.as_bool();
                            inner.failed = !success.as_bool();
                            if success.as_bool() {
                                inner.document.clear();
                            }
                            Some((
                                inner.app_handle.clone(),
                                ReaderStatusEvent {
                                    reader_session_id: inner.active_session_id.clone(),
                                    account_id: inner.active_account_id,
                                    message_id: inner.active_message_id,
                                    body_status: if success.as_bool() {
                                        BodyStatusDto::Ready
                                    } else {
                                        BodyStatusDto::Unavailable
                                    },
                                    detail: (!success.as_bool())
                                        .then(|| "邮件正文加载失败".to_string()),
                                },
                            ))
                        } else {
                            None
                        }
                    };
                    if let Some((handle, event)) = emit {
                        let _ = handle.emit_to(MAIN_WINDOW_LABEL, EVENT_READER_STATUS, event);
                    }
                }
                Ok(())
            }));
            view.add_NavigationCompleted(&comp, &mut token)?;

            let fail_weak = weak.clone();
            let proc_fail = ProcessFailedEventHandler::create(Box::new(move |_, _| {
                if let Some(state) = fail_weak.upgrade() {
                    Self::mark_failed(&state, "WebView2 阅读进程异常退出");
                }
                Ok(())
            }));
            view.add_ProcessFailed(&proc_fail, &mut token)?;

            let new_win = NewWindowRequestedEventHandler::create(Box::new(move |_, args| {
                if let Some(args) = args {
                    let mut initiated = windows::core::BOOL::default();
                    let mut uri = windows::core::PWSTR::null();
                    unsafe {
                        args.IsUserInitiated(&mut initiated)?;
                        args.Uri(&mut uri)?;
                    }
                    let url = webview2_com::take_pwstr(uri);
                    if initiated.as_bool() {
                        open_external_url(&url);
                    }
                    unsafe { args.SetHandled(true)? };
                }
                Ok(())
            }));
            view.add_NewWindowRequested(&new_win, &mut token)?;

            let filter_url: Vec<u16> = format!("{READER_DOC_BASE}/*")
                .encode_utf16()
                .chain(Some(0))
                .collect();
            let _ = view.AddWebResourceRequestedFilter(
                windows::core::PCWSTR(filter_url.as_ptr()),
                COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
            );
            let env_res = env.clone();
            let res_handler = WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else {
                    return Ok(());
                };
                let Some(state) = weak.upgrade() else {
                    return Ok(());
                };
                let req = unsafe { args.Request()? };
                let mut uri = windows::core::PWSTR::null();
                unsafe { req.Uri(&mut uri)? };
                let url = webview2_com::take_pwstr(uri);
                let inner = state.borrow();
                let bytes = if !inner.closed && url == reader_doc_url(inner.version) {
                    inner.document.clone()
                } else {
                    Vec::new()
                };
                drop(inner);
                if let Some(stream) = unsafe { SHCreateMemStream(Some(&bytes)) } {
                    let headers: Vec<u16> =
                        "Content-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\n"
                            .encode_utf16()
                            .chain(Some(0))
                            .collect();
                    if let Ok(resp) = unsafe {
                        env_res.CreateWebResourceResponse(
                            &stream,
                            200,
                            w!("OK"),
                            windows::core::PCWSTR(headers.as_ptr()),
                        )
                    } {
                        unsafe { args.SetResponse(&resp)? };
                    }
                }
                Ok(())
            }));
            view.add_WebResourceRequested(&res_handler, &mut token)?;
            Ok(())
        }

        unsafe fn navigate(
            controller: &ICoreWebView2Controller,
            url: &str,
        ) -> windows::core::Result<()> {
            let wide: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
            controller
                .CoreWebView2()?
                .Navigate(windows::core::PCWSTR(wide.as_ptr()))
        }

        fn mark_failed(state: &Rc<RefCell<HostInner>>, detail: &str) {
            let (handle, event) = {
                let mut inner = state.borrow_mut();
                inner.loaded = false;
                inner.failed = true;
                (
                    inner.app_handle.clone(),
                    ReaderStatusEvent {
                        reader_session_id: inner.active_session_id.clone(),
                        account_id: inner.active_account_id,
                        message_id: inner.active_message_id,
                        body_status: BodyStatusDto::Unavailable,
                        detail: Some(detail.to_string()),
                    },
                )
            };
            if !event.reader_session_id.is_empty() {
                let _ = handle.emit_to(MAIN_WINDOW_LABEL, EVENT_READER_STATUS, event);
            }
        }

        fn load_html(
            &self,
            session_id: String,
            account_id: u32,
            message_id: u32,
            html_bytes: Vec<u8>,
        ) {
            let (ctrl, url) = {
                let mut inner = self.inner.borrow_mut();
                if inner.closed {
                    return;
                }
                inner.active_session_id = session_id;
                inner.active_account_id = account_id;
                inner.active_message_id = message_id;
                inner.version = inner.version.wrapping_add(1);
                inner.document = html_bytes;
                inner.loaded = false;
                inner.failed = false;
                inner.navigation_id = None;
                (inner.controller.clone(), reader_doc_url(inner.version))
            };
            if let Some(ctrl) = ctrl {
                if unsafe { Self::navigate(&ctrl, &url) }.is_err() {
                    Self::mark_failed(&self.inner, "无法导航到邮件文档");
                }
            }
        }

        fn apply_layout(&self, clip: PhysicalRect, bounds: PhysicalRect, visible: bool) {
            let clip_rect = RECT {
                left: clip.left,
                top: clip.top,
                right: clip.right,
                bottom: clip.bottom,
            };
            let bounds_rect = RECT {
                left: bounds.left,
                top: bounds.top,
                right: bounds.right,
                bottom: bounds.bottom,
            };
            let has_area = clip.width() > 0 && clip.height() > 0;
            let (clip_window, ctrl, clip_changed, bounds_changed, vis_changed, shown) = {
                let mut inner = self.inner.borrow_mut();
                let clip_changed = inner.clip != clip_rect;
                let bounds_changed = inner.bounds != bounds_rect;
                inner.clip = clip_rect;
                inner.bounds = bounds_rect;
                inner.visible = visible;
                inner.has_area = has_area;
                let shown = visible && has_area && !inner.failed;
                let vis_changed = inner.window_shown != shown;
                inner.window_shown = shown;
                (
                    inner.clip_window,
                    inner.controller.clone(),
                    clip_changed,
                    bounds_changed,
                    vis_changed,
                    shown,
                )
            };
            unsafe {
                if clip_changed && has_area {
                    let _ = SetWindowPos(
                        clip_window,
                        None,
                        clip.left,
                        clip.top,
                        clip.width(),
                        clip.height(),
                        SWP_NOACTIVATE | SWP_NOZORDER,
                    );
                }
                if vis_changed {
                    let _ =
                        ShowWindow(clip_window, if shown { SW_SHOWNOACTIVATE } else { SW_HIDE });
                }
                if let Some(ctrl) = ctrl {
                    if bounds_changed && has_area {
                        let _ = ctrl.SetBounds(bounds_rect);
                    }
                    if vis_changed {
                        let _ = ctrl.SetIsVisible(shown);
                    }
                }
            }
        }

        fn set_visible(&self, visible: bool) {
            let (clip_window, ctrl, vis_changed, shown) = {
                let mut inner = self.inner.borrow_mut();
                inner.visible = visible;
                let shown = visible && inner.has_area && !inner.failed;
                let vis_changed = inner.window_shown != shown;
                inner.window_shown = shown;
                (
                    inner.clip_window,
                    inner.controller.clone(),
                    vis_changed,
                    shown,
                )
            };
            if !vis_changed {
                return;
            }
            unsafe {
                let _ = ShowWindow(clip_window, if shown { SW_SHOWNOACTIVATE } else { SW_HIDE });
                if let Some(ctrl) = ctrl {
                    let _ = ctrl.SetIsVisible(shown);
                }
            }
        }

        fn set_zoom(&self, zoom_percent: u32) {
            let factor = f64::from(zoom_percent.clamp(50, 250)) / 100.0;
            let ctrl = {
                let mut inner = self.inner.borrow_mut();
                inner.zoom_factor = factor;
                inner.controller.clone()
            };
            if let Some(ctrl) = ctrl {
                unsafe {
                    let _ = ctrl.SetZoomFactor(factor);
                }
            }
        }

        fn close(&self) {
            let (clip_window, ctrl) = {
                let mut inner = self.inner.borrow_mut();
                inner.closed = true;
                inner.document.clear();
                (inner.clip_window, inner.controller.take())
            };
            if let Some(ctrl) = ctrl {
                unsafe {
                    let _ = ctrl.Close();
                }
            }
            unsafe {
                let _ = DestroyWindow(clip_window);
            }
        }
    }

    pub struct TauriWindowsReaderSink {
        app_handle: tauri::AppHandle,
        user_data_dir: PathBuf,
    }

    impl TauriWindowsReaderSink {
        pub fn new(app_handle: tauri::AppHandle, user_data_dir: PathBuf) -> Self {
            Self {
                app_handle,
                user_data_dir,
            }
        }

        fn with_host_on_main_thread<F>(&self, f: F)
        where
            F: FnOnce(&WindowsWebView2Reader) + Send + 'static,
        {
            let handle = self.app_handle.clone();
            let user_data_dir = self.user_data_dir.clone();
            let _ = self.app_handle.run_on_main_thread(move || {
                MAIN_THREAD_HOST.with(|slot| {
                    let mut borrow = slot.borrow_mut();
                    if borrow.is_none() {
                        if let Some(win) =
                            tauri::Manager::get_webview_window(&handle, MAIN_WINDOW_LABEL)
                        {
                            if let Ok(raw) = HasWindowHandle::window_handle(&win) {
                                if let RawWindowHandle::Win32(win32) = raw.as_raw() {
                                    let hwnd = HWND(win32.hwnd.get() as *mut _);
                                    if let Ok(host) = WindowsWebView2Reader::create(
                                        handle.clone(),
                                        hwnd,
                                        user_data_dir,
                                    ) {
                                        *borrow = Some(host);
                                    }
                                }
                            }
                        }
                    }
                    if let Some(host) = borrow.as_ref() {
                        f(host);
                    }
                });
            });
        }
    }

    impl ReaderPlatformSink for TauriWindowsReaderSink {
        fn is_native_supported(&self) -> bool {
            true
        }

        fn load_html(
            &self,
            session_id: String,
            account_id: u32,
            message_id: u32,
            html_bytes: Vec<u8>,
        ) {
            self.with_host_on_main_thread(move |host| {
                host.load_html(session_id, account_id, message_id, html_bytes);
            });
        }

        fn apply_layout(&self, clip: PhysicalRect, bounds: PhysicalRect, visible: bool) {
            self.with_host_on_main_thread(move |host| {
                host.apply_layout(clip, bounds, visible);
            });
        }

        fn set_visible(&self, visible: bool) {
            self.with_host_on_main_thread(move |host| {
                host.set_visible(visible);
            });
        }

        fn set_zoom(&self, zoom_percent: u32) {
            self.with_host_on_main_thread(move |host| {
                host.set_zoom(zoom_percent);
            });
        }

        fn close(&self) {
            let _ = self.app_handle.run_on_main_thread(|| {
                MAIN_THREAD_HOST.with(|slot| {
                    if let Some(host) = slot.borrow_mut().take() {
                        host.close();
                    }
                });
            });
        }

        fn emit_status(&self, event: ReaderStatusEvent) {
            let _ = self
                .app_handle
                .emit_to(MAIN_WINDOW_LABEL, EVENT_READER_STATUS, event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct RecordingSink {
        events: Mutex<Vec<ReaderStatusEvent>>,
        layouts: Mutex<Vec<(PhysicalRect, PhysicalRect, bool)>>,
    }

    impl ReaderPlatformSink for RecordingSink {
        fn is_native_supported(&self) -> bool {
            true
        }

        fn load_html(&self, _: String, _: u32, _: u32, _: Vec<u8>) {}

        fn apply_layout(&self, clip: PhysicalRect, bounds: PhysicalRect, visible: bool) {
            self.layouts.lock().unwrap().push((clip, bounds, visible));
        }

        fn set_visible(&self, _: bool) {}
        fn set_zoom(&self, _: u32) {}
        fn close(&self) {}

        fn emit_status(&self, event: ReaderStatusEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    fn rect(x: f64, y: f64, width: f64, height: f64) -> RectDto {
        RectDto {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn layout_revision_rejects_stale_or_duplicate_and_stale_sessions() {
        let sink = Arc::new(RecordingSink::default());
        let mgr = ReaderManager::new(sink.clone());

        let appearance = ReaderAppearance {
            theme_id: "system".to_string(),
            dark: false,
            zoom_percent: 100,
            block_remote_images: true,
        };

        let status = mgr.begin_session(
            ReaderDocumentSpec {
                session_id: "sess-a".to_string(),
                account_id: 1,
                message_id: 1,
                subject: "Hello".to_string(),
                sender: "a@example.com".to_string(),
                plain_text: "body".to_string(),
                html_body: Some("<p>Hi</p>".to_string()),
                has_thread: false,
            },
            &appearance,
            true,
        );
        assert_eq!(status, BodyStatusDto::Loading);

        // Revision 2 succeeds.
        let r2 = mgr
            .apply_layout(
                &SetReaderLayoutInput {
                    reader_session_id: "sess-a".to_string(),
                    layout_revision: 2,
                    content_rect: rect(600.0, 48.0, 680.0, 752.0),
                    clip_rect: rect(600.0, 48.0, 680.0, 752.0),
                    visible: true,
                    device_pixel_ratio: Some(1.25),
                },
                1.0,
            )
            .expect("rev 2 ok");
        assert!(r2.applied);

        // Out-of-order revision 1 and duplicate revision 2 are discarded.
        let r1 = mgr
            .apply_layout(
                &SetReaderLayoutInput {
                    reader_session_id: "sess-a".to_string(),
                    layout_revision: 1,
                    content_rect: rect(600.0, 48.0, 680.0, 752.0),
                    clip_rect: rect(600.0, 48.0, 680.0, 752.0),
                    visible: true,
                    device_pixel_ratio: Some(1.25),
                },
                1.0,
            )
            .expect("rev 1 ignored");
        assert!(!r1.applied);

        let dup = mgr
            .apply_layout(
                &SetReaderLayoutInput {
                    reader_session_id: "sess-a".to_string(),
                    layout_revision: 2,
                    content_rect: rect(600.0, 48.0, 680.0, 752.0),
                    clip_rect: rect(600.0, 48.0, 680.0, 752.0),
                    visible: true,
                    device_pixel_ratio: Some(1.25),
                },
                1.0,
            )
            .expect("dup rev ignored");
        assert!(!dup.applied);

        // Switching to sess-b resets revision guard and ignores sess-a updates.
        mgr.begin_session(
            ReaderDocumentSpec {
                session_id: "sess-b".to_string(),
                account_id: 1,
                message_id: 2,
                subject: "Second".to_string(),
                sender: "b@example.com".to_string(),
                plain_text: "plain".to_string(),
                html_body: None,
                has_thread: false,
            },
            &appearance,
            true,
        );

        let stale_sess = mgr
            .apply_layout(
                &SetReaderLayoutInput {
                    reader_session_id: "sess-a".to_string(),
                    layout_revision: 99,
                    content_rect: rect(600.0, 48.0, 680.0, 752.0),
                    clip_rect: rect(600.0, 48.0, 680.0, 752.0),
                    visible: true,
                    device_pixel_ratio: Some(1.0),
                },
                1.0,
            )
            .expect("stale session ignored");
        assert!(!stale_sess.applied);

        let fresh_b = mgr
            .apply_layout(
                &SetReaderLayoutInput {
                    reader_session_id: "sess-b".to_string(),
                    layout_revision: 1,
                    content_rect: rect(600.0, 48.0, 680.0, 752.0),
                    clip_rect: rect(600.0, 48.0, 680.0, 752.0),
                    visible: false,
                    device_pixel_ratio: Some(1.0),
                },
                1.0,
            )
            .expect("fresh session rev 1 applied");
        assert!(fresh_b.applied);
        assert_eq!(sink.layouts.lock().unwrap().len(), 2);
    }

    #[test]
    fn invalid_layout_coordinates_fail_without_advancing_revision() {
        let sink = Arc::new(RecordingSink::default());
        let mgr = ReaderManager::new(sink);
        let appearance = ReaderAppearance {
            theme_id: "system".to_string(),
            dark: false,
            zoom_percent: 100,
            block_remote_images: true,
        };
        mgr.begin_session(
            ReaderDocumentSpec {
                session_id: "sess-x".to_string(),
                account_id: 1,
                message_id: 1,
                subject: "S".to_string(),
                sender: "s@example.com".to_string(),
                plain_text: "body".to_string(),
                html_body: Some("<p>body</p>".to_string()),
                has_thread: false,
            },
            &appearance,
            true,
        );

        let err = mgr
            .apply_layout(
                &SetReaderLayoutInput {
                    reader_session_id: "sess-x".to_string(),
                    layout_revision: 5,
                    content_rect: rect(0.0, 0.0, -10.0, 100.0),
                    clip_rect: rect(0.0, 0.0, 100.0, 100.0),
                    visible: true,
                    device_pixel_ratio: None,
                },
                1.0,
            )
            .unwrap_err();
        assert_eq!(err.code, "INVALID_INPUT");
        assert_eq!(mgr.snapshot_test_state().1, None);
    }
}
