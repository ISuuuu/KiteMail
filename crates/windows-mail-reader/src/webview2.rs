//! Native Windows WebView2 controller host for isolated email body rendering.
//!
//! All COM objects and `Rc<RefCell<...>>` state MUST remain on the STA UI thread
//! that owns the parent `HWND`.

use std::{
    cell::RefCell,
    collections::HashMap,
    os::windows::ffi::OsStrExt,
    path::PathBuf,
    rc::{Rc, Weak},
};

use webview2_com::{
    CreateCoreWebView2ControllerCompletedHandler, CreateCoreWebView2EnvironmentCompletedHandler,
    DOMContentLoadedEventHandler, LaunchingExternalUriSchemeEventHandler,
    Microsoft::Web::WebView2::Win32::*, NavigationCompletedEventHandler,
    NavigationStartingEventHandler, NewWindowRequestedEventHandler, ProcessFailedEventHandler,
    TrySuspendCompletedHandler, WebMessageReceivedEventHandler, WebResourceRequestedEventHandler,
};
use windows::{
    core::{w, Interface},
    Win32::{
        Foundation::{HWND, RECT},
        System::Com::IStream,
        UI::{
            HiDpi::GetDpiForWindow,
            Input::KeyboardAndMouse::{GetFocus, SetFocus},
            Shell::SHCreateMemStream,
            WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, GetParent, IsChild, SetWindowPos, ShowWindow,
                HWND_TOP, SET_WINDOW_POS_FLAGS, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_HIDE,
                SW_SHOWNOACTIVATE, WINDOW_EX_STYLE, WS_CHILD, WS_CLIPCHILDREN,
            },
        },
    },
};

use crate::document::{build_body_document, build_plain_text_document, DocumentAppearance};
use crate::policy::{
    allow_frame_navigation, allow_main_navigation, allow_remote_image_request,
    allowed_external_url, document_headers, document_url, is_internal_reader_url, needs_new_host,
    open_link_message, parse_external_url, parse_thread_action, DocumentMode, ReaderLoadStatus,
    WebViewAction,
};

/// Physical pixel rectangle in parent client coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PixelRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl PixelRect {
    pub fn from_xywh(left: i32, top: i32, width: i32, height: i32) -> Self {
        Self {
            left,
            top,
            right: left.saturating_add(width.max(0)),
            bottom: top.saturating_add(height.max(0)),
        }
    }
}

impl From<RECT> for PixelRect {
    fn from(rect: RECT) -> Self {
        Self {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        }
    }
}

impl From<PixelRect> for RECT {
    fn from(rect: PixelRect) -> Self {
        Self {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        }
    }
}

/// Logical (CSS/DIP) rectangle relative to the parent client area.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LogicalRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Convert logical client coordinates to physical pixel coordinates using `scale_factor`.
pub fn logical_to_pixel_rect(rect: LogicalRect, scale_factor: f64) -> PixelRect {
    let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    let left = (rect.x * scale).round() as i32;
    let top = (rect.y * scale).round() as i32;
    let right = ((rect.x + rect.width.max(0.0)) * scale).round() as i32;
    let bottom = ((rect.y + rect.height.max(0.0)) * scale).round() as i32;
    PixelRect {
        left,
        top,
        right,
        bottom,
    }
}

/// Return the DPI scale factor (`dpi / 96.0`) for `hwnd`, falling back to `1.0`.
pub fn hwnd_scale_factor(hwnd: HWND) -> f64 {
    if hwnd.is_invalid() {
        return 1.0;
    }
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi == 0 {
        1.0
    } else {
        dpi as f64 / 96.0
    }
}

/// Intersect `content` with `viewport` and return `(clip_window_rect, controller_bounds)`.
pub fn clipped_rects(content: RECT, viewport: RECT) -> Option<(RECT, RECT)> {
    let clip = RECT {
        left: content.left.max(viewport.left),
        top: content.top.max(viewport.top),
        right: content.right.min(viewport.right),
        bottom: content.bottom.min(viewport.bottom),
    };
    if clip.right <= clip.left
        || clip.bottom <= clip.top
        || content.right <= content.left
        || content.bottom <= content.top
    {
        return None;
    }
    let bounds = RECT {
        left: content.left - clip.left,
        top: content.top - clip.top,
        right: content.right - clip.left,
        bottom: content.bottom - clip.top,
    };
    Some((clip, bounds))
}

/// Typed events emitted by [`WebView2Host`] to the embedding UI host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReaderEvent {
    StatusChanged {
        version: u64,
        status: ReaderLoadStatus,
    },
    ExternalLink {
        version: u64,
        url: String,
    },
    ThreadAction {
        version: u64,
        action: WebViewAction,
    },
    ScrollAnchor {
        version: u64,
        msg_id: u32,
        offset_px: i32,
    },
    LayoutApplied {
        version: u64,
        visible: bool,
        controller_ready: bool,
    },
    Error {
        version: u64,
        stage: &'static str,
        detail: String,
    },
    ProcessFailed {
        version: u64,
    },
    StateChanged,
}

type EventCallback = Rc<RefCell<dyn FnMut(ReaderEvent)>>;

struct State {
    controller: Option<ICoreWebView2Controller>,
    clip_window: HWND,
    document: Vec<u8>,
    allow_remote_images: bool,
    version: u64,
    navigation_id: Option<u64>,
    loaded: bool,
    layout_applied: bool,
    last_error: Option<String>,
    links: Vec<(u64, String)>,
    actions: Vec<(u64, WebViewAction)>,
    last_scroll_anchor: Option<(u64, u32, i32)>,
    bridge_script_version: u64,
    callback: Option<EventCallback>,
    pending_events: Vec<ReaderEvent>,
    mode: DocumentMode,
    clip_rect: RECT,
    bounds: RECT,
    zoom_factor: f64,
    visible: bool,
    has_area: bool,
    window_shown: bool,
    failed: bool,
    process_failed: bool,
    closed: bool,
    suspended: bool,
    suspending: bool,
    suspend_after_load: bool,
    suspend_inhibited: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            controller: None,
            clip_window: HWND::default(),
            document: Vec::new(),
            allow_remote_images: false,
            version: 0,
            navigation_id: None,
            loaded: false,
            layout_applied: false,
            last_error: None,
            links: Vec::new(),
            actions: Vec::new(),
            last_scroll_anchor: None,
            bridge_script_version: 0,
            callback: None,
            pending_events: Vec::new(),
            mode: DocumentMode::default(),
            clip_rect: RECT::default(),
            bounds: RECT::default(),
            zoom_factor: 1.0,
            visible: false,
            has_area: false,
            window_shown: false,
            failed: false,
            process_failed: false,
            closed: false,
            suspended: false,
            suspending: false,
            suspend_after_load: false,
            suspend_inhibited: false,
        }
    }
}

impl State {
    fn queue_event(&mut self, event: ReaderEvent) {
        if self.callback.is_some() {
            self.pending_events.push(event);
        }
    }

    fn current_status(&self) -> ReaderLoadStatus {
        if self.process_failed {
            ReaderLoadStatus::ProcessFailed
        } else if self.failed || self.closed {
            ReaderLoadStatus::Unavailable
        } else if self.loaded {
            ReaderLoadStatus::Ready
        } else {
            ReaderLoadStatus::Loading
        }
    }
}

fn record_stage_error(state: &Rc<RefCell<State>>, stage: &'static str, detail: String) {
    eprintln!("[windows-mail-reader] {stage}: {detail}");
    {
        let mut inner = state.borrow_mut();
        if inner.closed {
            return;
        }
        inner.failed = true;
        inner.layout_applied = false;
        inner.last_error = Some(format!("{stage}: {detail}"));
        let version = inner.version;
        inner.queue_event(ReaderEvent::Error {
            version,
            stage,
            detail,
        });
        inner.queue_event(ReaderEvent::StatusChanged {
            version,
            status: ReaderLoadStatus::Unavailable,
        });
    }
    flush_events(state);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ClipWindowPosPlan {
    pub insert_after_top: bool,
    pub flags: SET_WINDOW_POS_FLAGS,
}

/// Compute Win32 `SetWindowPos` plan for the reader `WS_CHILD` clip window.
/// Always lifts to `HWND_TOP` (never `SWP_NOZORDER`) and preserves `SWP_NOACTIVATE`
/// when positioning or showing so Wry's sibling child window cannot occlude the reader.
pub(crate) fn plan_clip_window_pos(clip_changed: bool, visible: bool) -> Option<ClipWindowPosPlan> {
    if clip_changed {
        Some(ClipWindowPosPlan {
            insert_after_top: true,
            flags: SWP_NOACTIVATE,
        })
    } else if visible {
        Some(ClipWindowPosPlan {
            insert_after_top: true,
            flags: SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
        })
    } else {
        None
    }
}

unsafe fn apply_clip_window_pos_and_show(
    clip_window: HWND,
    clip: RECT,
    clip_changed: bool,
    visible: bool,
    visibility_changed: bool,
) -> Result<(), (&'static str, String)> {
    if clip_window.is_invalid() {
        return Err(("clip_window", "阅读器宿主子窗口句柄无效".to_string()));
    }
    if let Some(plan) = plan_clip_window_pos(clip_changed, visible) {
        let insert_after = plan.insert_after_top.then_some(HWND_TOP);
        SetWindowPos(
            clip_window,
            insert_after,
            clip.left,
            clip.top,
            clip.right - clip.left,
            clip.bottom - clip.top,
            plan.flags,
        )
        .map_err(|e| ("set_window_pos", format!("SetWindowPos 失败: {e}")))?;
    }
    if visibility_changed {
        let _ = ShowWindow(
            clip_window,
            if visible { SW_SHOWNOACTIVATE } else { SW_HIDE },
        );
    }
    Ok(())
}

fn flush_events(state: &Rc<RefCell<State>>) {
    let (cb, events) = {
        let mut inner = state.borrow_mut();
        let events = std::mem::take(&mut inner.pending_events);
        let cb = inner.callback.clone();
        (cb, events)
    };
    if let Some(cb) = cb {
        if let Ok(mut guard) = cb.try_borrow_mut() {
            for event in events {
                guard(event);
            }
        }
    }
}

fn record_external_link(state: &mut State, url: &str) -> bool {
    if state.closed || !state.loaded || !allowed_external_url(url) {
        return false;
    }
    let version = state.version;
    if !state
        .links
        .last()
        .is_some_and(|(v, last)| *v == version && last == url)
    {
        let owned = url.to_string();
        state.links.push((version, owned.clone()));
        state.queue_event(ReaderEvent::ExternalLink {
            version,
            url: owned,
        });
    }
    true
}

fn record_script_link(state: &Rc<RefCell<State>>, source: &str, url: &str) -> bool {
    let recorded = {
        let mut inner = state.borrow_mut();
        if !allow_main_navigation(source, inner.version) {
            false
        } else {
            let is_mailto =
                parse_external_url(url).is_some_and(|parsed| parsed.scheme() == "mailto");
            is_mailto && record_external_link(&mut inner, url)
        }
    };
    if recorded {
        flush_events(state);
    }
    recorded
}

fn apply_dom_content_loaded(state: &mut State, navigation_id: u64) -> bool {
    if state.navigation_id == Some(navigation_id) && !state.closed {
        state.loaded = true;
        state.failed = false;
        state.process_failed = false;
        let version = state.version;
        state.queue_event(ReaderEvent::StatusChanged {
            version,
            status: ReaderLoadStatus::Ready,
        });
        true
    } else {
        false
    }
}

fn should_allow_main_navigation(state: &State, url: &str) -> bool {
    !state.loaded && allow_main_navigation(url, state.version)
}

fn apply_navigation_completed(state: &mut State, navigation_id: u64, success: bool) -> bool {
    if state.closed {
        return false;
    }
    if state.navigation_id == Some(navigation_id) {
        state.loaded = success;
        state.failed = !success;
        if success {
            state.process_failed = false;
            state.document.clear();
            state.document.shrink_to_fit();
        }
        let version = state.version;
        let status = if success {
            ReaderLoadStatus::Ready
        } else {
            ReaderLoadStatus::Unavailable
        };
        state.queue_event(ReaderEvent::StatusChanged { version, status });
        success
    } else {
        false
    }
}

fn maybe_install_bridge_script(state: &Rc<RefCell<State>>, view: Option<&ICoreWebView2>) {
    let mode = {
        let mut inner = state.borrow_mut();
        let version = inner.version;
        if inner.loaded && inner.bridge_script_version != version {
            inner.bridge_script_version = version;
            Some(inner.mode)
        } else {
            None
        }
    };
    if let (Some(view), Some(mode)) = (view, mode) {
        // Run the trusted listener in the wrapper. The mail frame stays scriptless.
        let script = match mode {
            DocumentMode::SingleBody => include_str!("scripts/thread_scroll.js"),
            DocumentMode::ThreadContainer => include_str!("scripts/reader_thread.js"),
        };
        let script: Vec<u16> = script.encode_utf16().chain(Some(0)).collect();
        unsafe {
            let _ = view.ExecuteScript(
                windows::core::PCWSTR(script.as_ptr()),
                None::<&ICoreWebView2ExecuteScriptCompletedHandler>,
            );
        }
    }
}

#[derive(Default)]
struct SharedEnvironment {
    environment: Option<ICoreWebView2Environment>,
    creating: bool,
    waiters: Vec<(HWND, Weak<RefCell<State>>)>,
}

thread_local! {
    static ENVIRONMENTS: RefCell<HashMap<PathBuf, SharedEnvironment>> = RefCell::new(HashMap::new());
}

fn create_controller(
    environment: &ICoreWebView2Environment,
    parent: HWND,
    weak: Weak<RefCell<State>>,
) {
    let Some(state) = weak.upgrade() else {
        return;
    };
    if state.borrow().closed {
        return;
    }
    let env = environment.clone();
    let handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
        move |result, controller| {
            let Some(state) = weak.upgrade() else {
                if let Some(controller) = controller {
                    unsafe {
                        let _ = controller.Close();
                    }
                }
                return Ok(());
            };
            let controller = match (result, controller) {
                (Ok(()), Some(controller)) => controller,
                (Err(err), _) => {
                    record_stage_error(
                        &state,
                        "create_controller",
                        format!("WebView2 控制器创建失败: {err}"),
                    );
                    return Ok(());
                }
                (Ok(()), None) => {
                    record_stage_error(
                        &state,
                        "create_controller",
                        "WebView2 控制器回调未返回实例".to_string(),
                    );
                    return Ok(());
                }
            };
            if state.borrow().closed {
                unsafe {
                    let _ = controller.Close();
                }
                return Ok(());
            }
            if let Err(err) = unsafe { configure(&env, &controller, Rc::downgrade(&state)) } {
                unsafe {
                    let _ = controller.Close();
                }
                record_stage_error(
                    &state,
                    "configure_webview2",
                    format!("WebView2 配置与事件绑定失败: {err}"),
                );
                return Ok(());
            }
            let (
                clip_window,
                clip_rect,
                bounds,
                visible,
                has_area,
                navigate,
                url,
                zoom_factor,
                version,
            ) = {
                let mut inner = state.borrow_mut();
                let clip_window = inner.clip_window;
                let clip_rect = inner.clip_rect;
                let bounds = inner.bounds;
                let has_area = inner.has_area;
                let visible = inner.visible && has_area;
                let navigate = !inner.document.is_empty();
                let url = document_url(inner.version);
                let zoom_factor = inner.zoom_factor;
                let version = inner.version;
                inner.controller = Some(controller.clone());
                (
                    clip_window,
                    clip_rect,
                    bounds,
                    visible,
                    has_area,
                    navigate,
                    url,
                    zoom_factor,
                    version,
                )
            };
            if visible {
                if let Err((stage, detail)) = unsafe {
                    apply_clip_window_pos_and_show(clip_window, clip_rect, false, true, true)
                } {
                    record_stage_error(&state, stage, detail);
                    return Ok(());
                }
            }
            let apply_res: windows::core::Result<()> = (|| unsafe {
                controller.SetBounds(bounds)?;
                controller.SetIsVisible(visible)?;
                if (zoom_factor - 1.0).abs() > f64::EPSILON {
                    controller.SetZoomFactor(zoom_factor)?;
                }
                Ok(())
            })();
            if let Err(err) = apply_res {
                record_stage_error(
                    &state,
                    "controller_initial_layout",
                    format!("WebView2 控制器初始化布局失败: {err}"),
                );
                return Ok(());
            }
            if has_area {
                {
                    let mut inner = state.borrow_mut();
                    inner.layout_applied = true;
                    inner.queue_event(ReaderEvent::LayoutApplied {
                        version,
                        visible,
                        controller_ready: true,
                    });
                }
                flush_events(&state);
            }
            if let Err(err) = if navigate {
                unsafe { navigate_to(&controller, &url) }
            } else {
                Ok(())
            } {
                record_stage_error(&state, "navigate", format!("WebView2 导航启动失败: {err}"));
            }
            Ok(())
        },
    ));
    if let Err(err) = unsafe { create_controller_with_inprivate(environment, parent, &handler) } {
        record_stage_error(
            &state,
            "create_controller_call",
            format!("调用 CreateCoreWebView2Controller 失败: {err}"),
        );
    }
}

unsafe fn create_controller_with_inprivate(
    environment: &ICoreWebView2Environment,
    parent: HWND,
    handler: &ICoreWebView2CreateCoreWebView2ControllerCompletedHandler,
) -> windows::core::Result<()> {
    if let Ok(env10) = environment.cast::<ICoreWebView2Environment10>() {
        if let Ok(options) = env10.CreateCoreWebView2ControllerOptions() {
            if options.SetIsInPrivateModeEnabled(true).is_ok() {
                return env10.CreateCoreWebView2ControllerWithOptions(parent, &options, handler);
            }
        }
    }
    environment.CreateCoreWebView2Controller(parent, handler)
}

fn clear_webview_cookies(controller: &ICoreWebView2Controller) {
    if let Ok(view) = unsafe { controller.CoreWebView2() } {
        if let Ok(view2) = view.cast::<ICoreWebView2_2>() {
            if let Ok(manager) = unsafe { view2.CookieManager() } {
                unsafe {
                    let _ = manager.DeleteAllCookies();
                }
            }
        }
    }
}

fn restore_parent_focus_if_inside(clip_window: HWND) {
    if clip_window.is_invalid() {
        return;
    }
    unsafe {
        let focused = GetFocus();
        if !focused.is_invalid()
            && (focused == clip_window || IsChild(clip_window, focused).as_bool())
        {
            if let Ok(parent) = GetParent(clip_window) {
                if !parent.is_invalid() {
                    let _ = SetFocus(Some(parent));
                }
            }
        }
    }
}

/// STA-only native WebView2 host attached to a parent `HWND` with an isolated
/// `user_data_dir` and clipping child window.
pub struct WebView2Host(Rc<RefCell<State>>);

impl WebView2Host {
    /// Create a WebView2 host as a child of `parent`, isolating browser state under `user_data_dir`.
    pub fn create(parent: HWND, user_data_dir: PathBuf) -> windows::core::Result<Self> {
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
                Some(parent),
                None,
                None,
                None,
            )?
        };
        let state = Rc::new(RefCell::new(State {
            clip_window,
            ..State::default()
        }));
        let weak = Rc::downgrade(&state);
        let env_key = user_data_dir.clone();
        let (environment, start) = ENVIRONMENTS.with(|envs| {
            let mut envs = envs.borrow_mut();
            let shared = envs.entry(env_key.clone()).or_default();
            if let Some(environment) = &shared.environment {
                return (Some(environment.clone()), false);
            }
            shared.waiters.push((clip_window, weak.clone()));
            let start = !shared.creating;
            shared.creating = true;
            (None, start)
        });
        if let Some(environment) = environment {
            create_controller(&environment, clip_window, weak);
            return Ok(Self(state));
        }
        if !start {
            return Ok(Self(state));
        }
        let _ = std::fs::create_dir_all(&user_data_dir);
        let callback_key = env_key.clone();
        let callback = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
            move |result, environment| {
                let env_error = result.as_ref().err().map(|e| format!("{e}"));
                let environment = environment.filter(|_| result.is_ok());
                let waiters = ENVIRONMENTS.with(|envs| {
                    let mut envs = envs.borrow_mut();
                    let shared = envs.entry(callback_key.clone()).or_default();
                    shared.creating = false;
                    shared.environment = environment.clone();
                    std::mem::take(&mut shared.waiters)
                });
                for (parent, weak) in waiters {
                    if let Some(environment) = &environment {
                        create_controller(environment, parent, weak);
                    } else if let Some(state) = weak.upgrade() {
                        let detail = match &env_error {
                            Some(err) => format!("创建 WebView2 环境失败: {err}"),
                            None => "创建 WebView2 环境失败: 未返回环境实例".to_string(),
                        };
                        record_stage_error(&state, "create_environment_completed", detail);
                    }
                }
                Ok(())
            },
        ));
        let data_dir: Vec<u16> = user_data_dir
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        let data_dir = if data_dir.is_empty() {
            windows::core::PCWSTR::null()
        } else {
            windows::core::PCWSTR(data_dir.as_ptr())
        };
        let options = webview2_com::CoreWebView2EnvironmentOptions::default();
        // Opt out of Windows sending WebView crash telemetry to Microsoft.
        unsafe { options.set_is_custom_crash_reporting_enabled(true) };
        let options = ICoreWebView2EnvironmentOptions::from(options);
        let result = unsafe {
            CreateCoreWebView2EnvironmentWithOptions(
                windows::core::PCWSTR::null(),
                data_dir,
                &options,
                &callback,
            )
        };
        if let Err(error) = result {
            let waiters = ENVIRONMENTS.with(|envs| {
                let mut envs = envs.borrow_mut();
                let shared = envs.entry(env_key).or_default();
                shared.creating = false;
                std::mem::take(&mut shared.waiters)
            });
            for (_, weak) in waiters {
                if let Some(state) = weak.upgrade() {
                    record_stage_error(
                        &state,
                        "create_environment",
                        format!("调用 CreateCoreWebView2EnvironmentWithOptions 失败: {error}"),
                    );
                }
            }
            unsafe {
                let _ = DestroyWindow(clip_window);
            }
            return Err(error);
        }
        Ok(Self(state))
    }

    /// Create a WebView2 host from a raw Win32 `HWND` integer handle (`isize`).
    pub fn from_raw_hwnd(
        parent_hwnd: isize,
        user_data_dir: PathBuf,
    ) -> windows::core::Result<Self> {
        Self::create(HWND(parent_hwnd as *mut _), user_data_dir)
    }

    /// Register a callback for typed status, link, action, scroll, and crash events.
    pub fn set_event_callback<F>(&self, callback: F)
    where
        F: FnMut(ReaderEvent) + 'static,
    {
        self.0.borrow_mut().callback = Some(Rc::new(RefCell::new(callback)));
    }

    /// Notify any registered callback with [`ReaderEvent::StateChanged`].
    pub fn notify_state_changed(&self) {
        self.0.borrow_mut().queue_event(ReaderEvent::StateChanged);
        flush_events(&self.0);
    }

    /// Prevent or allow `try_suspend()` while a host overlay/snapshot is active.
    pub fn set_suspend_inhibited(&self, inhibited: bool) {
        self.0.borrow_mut().suspend_inhibited = inhibited;
    }

    /// Return the current `(version, ReaderLoadStatus)` pair.
    pub fn status(&self) -> (u64, ReaderLoadStatus) {
        let state = self.0.borrow();
        (state.version, state.current_status())
    }

    pub fn version(&self) -> u64 {
        self.0.borrow().version
    }

    pub fn clip_hwnd(&self) -> HWND {
        self.0.borrow().clip_window
    }

    pub fn bounds_rect(&self) -> RECT {
        self.0.borrow().bounds
    }

    pub fn is_loaded(&self) -> bool {
        self.0.borrow().loaded
    }

    pub fn is_window_shown(&self) -> bool {
        self.0.borrow().window_shown
    }

    pub fn is_closed(&self) -> bool {
        self.0.borrow().closed
    }

    pub fn core_webview2(&self) -> Option<ICoreWebView2> {
        let controller = self.0.borrow().controller.clone()?;
        unsafe { controller.CoreWebView2().ok() }
    }

    pub fn take_links(&self) -> Vec<String> {
        let mut state = self.0.borrow_mut();
        let version = state.version;
        std::mem::take(&mut state.links)
            .into_iter()
            .filter_map(|(v, url)| (v == version).then_some(url))
            .collect()
    }

    pub fn take_actions(&self) -> Vec<WebViewAction> {
        let mut state = self.0.borrow_mut();
        let version = state.version;
        std::mem::take(&mut state.actions)
            .into_iter()
            .filter_map(|(v, action)| (v == version).then_some(action))
            .collect()
    }

    pub fn last_scroll_anchor(&self) -> Option<(u32, i32)> {
        let state = self.0.borrow();
        state
            .last_scroll_anchor
            .filter(|(v, _, _)| *v == state.version)
            .map(|(_, id, offset)| (id, offset))
    }

    pub fn execute_script(&self, script: &str) {
        let controller = {
            let state = self.0.borrow();
            if state.closed || !state.loaded {
                return;
            }
            state.controller.clone()
        };
        if let Some(controller) = controller {
            if let Ok(view) = unsafe { controller.CoreWebView2() } {
                let wide: Vec<u16> = script.encode_utf16().chain(Some(0)).collect();
                unsafe {
                    let _ = view.ExecuteScript(
                        windows::core::PCWSTR(wide.as_ptr()),
                        None::<&ICoreWebView2ExecuteScriptCompletedHandler>,
                    );
                }
            }
        }
    }

    pub fn try_suspend(&self) {
        try_suspend_state(&self.0);
    }

    fn resume_if_needed(&self, controller: &ICoreWebView2Controller) {
        resume_controller(&self.0, controller);
    }

    /// Update physical content rectangle, clip rectangle, and visibility in one step.
    pub fn set_bounds(&self, content_rect: PixelRect, clip_rect: PixelRect, visible: bool) {
        let _ = self.try_set_bounds(content_rect, clip_rect, visible);
    }

    /// Update physical content rectangle, clip rectangle, and visibility, reporting Win32/WebView2 errors.
    pub fn try_set_bounds(
        &self,
        content_rect: PixelRect,
        clip_rect: PixelRect,
        visible: bool,
    ) -> Result<(), String> {
        {
            self.0.borrow_mut().visible = visible;
        }
        self.apply_clipped_bounds(content_rect.into(), clip_rect.into())
    }

    /// Update both content and viewport rectangles to `bounds` without changing visibility.
    pub fn set_rect(&self, bounds: RECT) {
        self.set_clipped_bounds(bounds, bounds);
    }

    /// Update content and clip rectangles in physical pixels without changing visibility.
    pub fn set_clipped_bounds(&self, content: RECT, viewport: RECT) {
        let _ = self.apply_clipped_bounds(content, viewport);
    }

    fn apply_clipped_bounds(&self, content: RECT, viewport: RECT) -> Result<(), String> {
        let result = clipped_rects(content, viewport);
        let has_area = result.is_some();
        let (clip, bounds) = result.unwrap_or_default();
        let (
            controller,
            clip_window,
            visible,
            clip_changed,
            bounds_changed,
            visibility_changed,
            version,
        ) = {
            let mut state = self.0.borrow_mut();
            if state.closed {
                return Err("阅读器宿主已关闭".to_string());
            }
            let prev_has_area = state.has_area;
            let clip_changed = has_area && (!prev_has_area || state.clip_rect != clip);
            let bounds_changed = state.bounds != bounds || (has_area && !prev_has_area);
            state.clip_rect = clip;
            state.bounds = bounds;
            state.has_area = has_area;
            let visible = state.visible && has_area;
            let visibility_changed = state.window_shown != visible;
            state.window_shown = visible;
            (
                state.controller.clone(),
                state.clip_window,
                visible,
                clip_changed,
                bounds_changed,
                visibility_changed,
                state.version,
            )
        };
        if visibility_changed && visible {
            if let Some(controller) = &controller {
                self.resume_if_needed(controller);
            }
        }
        if visibility_changed && !visible {
            restore_parent_focus_if_inside(clip_window);
        }
        if let Err((stage, detail)) = unsafe {
            apply_clip_window_pos_and_show(
                clip_window,
                clip,
                clip_changed,
                visible,
                visibility_changed,
            )
        } {
            record_stage_error(&self.0, stage, detail.clone());
            return Err(detail);
        }
        let controller_ready = if let Some(controller) = controller {
            let res: Result<(), (&'static str, String)> = (|| unsafe {
                if bounds_changed {
                    controller
                        .SetBounds(bounds)
                        .map_err(|e| ("controller_set_bounds", format!("SetBounds 失败: {e}")))?;
                }
                if visibility_changed {
                    controller.SetIsVisible(visible).map_err(|e| {
                        ("controller_set_visible", format!("SetIsVisible 失败: {e}"))
                    })?;
                }
                Ok(())
            })();
            if let Err((stage, detail)) = res {
                record_stage_error(&self.0, stage, detail.clone());
                return Err(detail);
            }
            true
        } else {
            false
        };
        {
            let mut state = self.0.borrow_mut();
            if state.version == version && !state.closed {
                state.layout_applied = controller_ready;
                state.queue_event(ReaderEvent::LayoutApplied {
                    version,
                    visible,
                    controller_ready,
                });
            }
        }
        flush_events(&self.0);
        Ok(())
    }

    /// Show or hide the WebView2 clip window and controller, restoring focus if needed.
    pub fn set_visible(&self, visible: bool) {
        let _ = self.try_set_visible(visible);
    }

    /// Show or hide the WebView2 clip window and controller, reporting Win32/WebView2 errors.
    pub fn try_set_visible(&self, visible: bool) -> Result<(), String> {
        let (controller, clip_window, visible, visibility_changed, version) = {
            let mut state = self.0.borrow_mut();
            if state.closed {
                return Err("阅读器宿主已关闭".to_string());
            }
            state.visible = visible;
            let visible = visible && state.has_area;
            let visibility_changed = state.window_shown != visible;
            state.window_shown = visible;
            (
                state.controller.clone(),
                state.clip_window,
                visible,
                visibility_changed,
                state.version,
            )
        };
        if !visibility_changed && !visible {
            return Ok(());
        }
        if visibility_changed {
            if visible {
                if let Some(controller) = &controller {
                    self.resume_if_needed(controller);
                }
            } else {
                restore_parent_focus_if_inside(clip_window);
            }
        }
        if let Err((stage, detail)) = unsafe {
            apply_clip_window_pos_and_show(
                clip_window,
                RECT::default(),
                false,
                visible,
                visibility_changed,
            )
        } {
            record_stage_error(&self.0, stage, detail.clone());
            return Err(detail);
        }
        let controller_ready = if let Some(controller) = controller {
            if visibility_changed {
                if let Err(err) = unsafe { controller.SetIsVisible(visible) } {
                    let detail = format!("SetIsVisible 失败: {err}");
                    record_stage_error(&self.0, "controller_set_visible", detail.clone());
                    return Err(detail);
                }
            }
            true
        } else {
            false
        };
        if visibility_changed {
            let mut state = self.0.borrow_mut();
            if state.version == version && !state.closed {
                state.layout_applied = controller_ready;
                state.queue_event(ReaderEvent::LayoutApplied {
                    version,
                    visible,
                    controller_ready,
                });
            }
            drop(state);
            flush_events(&self.0);
        }
        Ok(())
    }

    /// Update the WebView2 controller zoom factor (clamped to `0.25..=5.0`).
    pub fn set_zoom(&self, zoom_factor: f64) {
        let clamped = if zoom_factor.is_finite() {
            zoom_factor.clamp(0.25, 5.0)
        } else {
            1.0
        };
        let controller = {
            let mut state = self.0.borrow_mut();
            if state.closed {
                return;
            }
            if (state.zoom_factor - clamped).abs() < f64::EPSILON {
                return;
            }
            state.zoom_factor = clamped;
            state.controller.clone()
        };
        if let Some(controller) = controller {
            unsafe {
                let _ = controller.SetZoomFactor(clamped);
            }
        }
    }

    /// Retire this host if switching remote-image permission requires an isolated controller.
    pub fn retire_for_remote_reload(&self, next_allowed: bool) -> bool {
        let mut state = self.0.borrow_mut();
        if !needs_new_host(state.version != 0, state.allow_remote_images, next_allowed) {
            return false;
        }
        state.allow_remote_images = false;
        state.closed = true;
        let controller = state.controller.take();
        drop(state);
        if let Some(controller) = controller {
            clear_webview_cookies(&controller);
            unsafe {
                let _ = controller.Close();
            }
        }
        true
    }

    /// Load an HTML email body using [`DocumentAppearance`] and return the document version.
    pub fn load_html(
        &self,
        html: &str,
        appearance: &DocumentAppearance,
    ) -> windows::core::Result<u64> {
        let document = build_body_document(html, appearance);
        self.load_document(
            document,
            !appearance.block_remote_images,
            DocumentMode::SingleBody,
        )
    }

    /// Load a plain-text email body using [`DocumentAppearance`] and return the document version.
    pub fn load_plain_text(
        &self,
        text: &str,
        appearance: &DocumentAppearance,
    ) -> windows::core::Result<u64> {
        let document = build_plain_text_document(text, appearance);
        self.load_document(document, false, DocumentMode::SingleBody)
    }

    pub fn load(&self, document: Vec<u8>) -> windows::core::Result<u64> {
        self.load_with_remote(document, false)
    }

    pub fn load_with_remote(
        &self,
        document: Vec<u8>,
        allow_remote_images: bool,
    ) -> windows::core::Result<u64> {
        self.load_document(document, allow_remote_images, DocumentMode::SingleBody)
    }

    pub fn unload_and_suspend(&self) {
        let _ = self.load_document_with_suspend(
            b"<!doctype html>".to_vec(),
            false,
            DocumentMode::SingleBody,
            true,
        );
    }

    pub fn load_document(
        &self,
        document: Vec<u8>,
        allow_remote_images: bool,
        mode: DocumentMode,
    ) -> windows::core::Result<u64> {
        self.load_document_with_suspend(document, allow_remote_images, mode, false)
    }

    pub fn load_document_with_suspend(
        &self,
        document: Vec<u8>,
        allow_remote_images: bool,
        mode: DocumentMode,
        suspend_after_load: bool,
    ) -> windows::core::Result<u64> {
        {
            let mut state = self.0.borrow_mut();
            if state.closed {
                state.failed = true;
                return Ok(state.version);
            }
        }
        self.set_visible(false);
        let (prev_allowed, version, url, controller) = {
            let mut state = self.0.borrow_mut();
            let prev_allowed = state.allow_remote_images;
            state.document = document;
            state.allow_remote_images = false;
            state.mode = mode;
            state.suspend_after_load = suspend_after_load;
            state.version = state.version.wrapping_add(1);
            state.loaded = false;
            state.layout_applied = false;
            state.last_error = None;
            state.process_failed = false;
            state.navigation_id = None;
            state.links.clear();
            state.actions.clear();
            state.last_scroll_anchor = None;
            let version = state.version;
            let url = document_url(version);
            let controller = state.controller.clone();
            if controller.is_some() {
                state.failed = false;
            }
            state.queue_event(ReaderEvent::StatusChanged {
                version,
                status: ReaderLoadStatus::Loading,
            });
            (prev_allowed, version, url, controller)
        };
        flush_events(&self.0);

        if let Some(controller) = &controller {
            self.resume_if_needed(controller);
            if let Ok(view) = unsafe { controller.CoreWebView2() } {
                let _ = unsafe { view.Stop() };
            }
            if prev_allowed || allow_remote_images {
                clear_webview_cookies(controller);
            }
        }
        self.0.borrow_mut().allow_remote_images = allow_remote_images;
        if let Some(controller) = controller {
            if let Err(error) = unsafe { navigate_to(&controller, &url) } {
                {
                    let mut state = self.0.borrow_mut();
                    state.navigation_id = None;
                    state.suspend_after_load = false;
                }
                record_stage_error(&self.0, "navigate", format!("WebView2 导航失败: {error}"));
                return Err(error);
            }
        }
        Ok(version)
    }

    /// Close the WebView2 controller, clear cookies/buffers, and destroy the clip child window.
    pub fn close(&self) {
        let (controller, clip_window) = {
            let mut state = self.0.borrow_mut();
            state.closed = true;
            state.visible = false;
            state.window_shown = false;
            state.document.clear();
            state.document.shrink_to_fit();
            state.callback = None;
            state.pending_events.clear();
            let controller = state.controller.take();
            let clip_window = std::mem::take(&mut state.clip_window);
            (controller, clip_window)
        };
        restore_parent_focus_if_inside(clip_window);
        if let Some(controller) = controller {
            clear_webview_cookies(&controller);
            unsafe {
                let _ = controller.Close();
            }
        }
        if !clip_window.is_invalid() {
            unsafe {
                let _ = DestroyWindow(clip_window);
            }
        }
    }
}

impl Drop for WebView2Host {
    fn drop(&mut self) {
        self.close();
    }
}

fn retired_resource_handler(state: &Weak<RefCell<State>>) -> bool {
    state.upgrade().is_none_or(|state| state.borrow().closed)
}

unsafe fn navigate_to(
    controller: &ICoreWebView2Controller,
    url: &str,
) -> windows::core::Result<()> {
    let url: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
    controller
        .CoreWebView2()?
        .Navigate(windows::core::PCWSTR(url.as_ptr()))
}

fn resume_controller(state: &Rc<RefCell<State>>, controller: &ICoreWebView2Controller) {
    {
        let mut inner = state.borrow_mut();
        inner.suspended = false;
        inner.suspending = false;
    }
    if let Ok(view) = unsafe { controller.CoreWebView2() } {
        if let Ok(view3) = view.cast::<ICoreWebView2_3>() {
            let mut is_suspended = windows::core::BOOL::default();
            if unsafe { view3.IsSuspended(&mut is_suspended) }.is_ok() && is_suspended.as_bool() {
                unsafe {
                    let _ = view3.Resume();
                }
            }
        }
    }
}

fn try_suspend_state(state: &Rc<RefCell<State>>) {
    let (controller, version) = {
        let mut inner = state.borrow_mut();
        if inner.closed
            || inner.visible
            || inner.suspend_inhibited
            || inner.suspended
            || inner.suspending
        {
            return;
        }
        let Some(controller) = inner.controller.clone() else {
            return;
        };
        inner.suspending = true;
        (controller, inner.version)
    };
    let Ok(view) = (unsafe { controller.CoreWebView2() }) else {
        state.borrow_mut().suspending = false;
        return;
    };
    let Ok(view3) = view.cast::<ICoreWebView2_3>() else {
        state.borrow_mut().suspending = false;
        return;
    };
    let weak = Rc::downgrade(state);
    let handler = TrySuspendCompletedHandler::create(Box::new(move |result, succeeded| {
        if let Some(state) = weak.upgrade() {
            let did_suspend = result.is_ok() && succeeded;
            let resume_ctrl = {
                let mut inner = state.borrow_mut();
                inner.suspending = false;
                if inner.closed {
                    None
                } else if inner.version == version && !inner.visible && !inner.suspend_inhibited {
                    inner.suspended = did_suspend;
                    None
                } else {
                    inner.suspended = false;
                    did_suspend.then(|| inner.controller.clone()).flatten()
                }
            };
            if let Some(ctrl) = resume_ctrl {
                resume_controller(&state, &ctrl);
            }
        }
        Ok(())
    }));
    if unsafe { view3.TrySuspend(&handler) }.is_err() {
        state.borrow_mut().suspending = false;
    }
}

fn clear_private_image_headers(
    request: &ICoreWebView2WebResourceRequest,
) -> windows::core::Result<()> {
    let headers = unsafe { request.Headers()? };
    for name in [w!("Referer"), w!("Cookie"), w!("Authorization")] {
        let mut present = Default::default();
        unsafe {
            headers.Contains(name, &mut present)?;
        }
        if present.as_bool() {
            unsafe {
                headers.RemoveHeader(name)?;
            }
        }
    }
    Ok(())
}

fn record_thread_action(state: &Rc<RefCell<State>>, source: &str, message: &str) -> bool {
    let Some(action) = parse_thread_action(message) else {
        return false;
    };
    let should_flush = {
        let mut inner = state.borrow_mut();
        let allow_mode = inner.mode == DocumentMode::ThreadContainer
            || matches!(action, WebViewAction::FindCount { .. });
        if inner.closed
            || !inner.loaded
            || !allow_mode
            || !allow_main_navigation(source, inner.version)
        {
            return false;
        }
        let version = inner.version;
        match action {
            WebViewAction::ScrollAnchor { msg_id, offset_px } => {
                inner.last_scroll_anchor = Some((version, msg_id, offset_px));
                inner.queue_event(ReaderEvent::ScrollAnchor {
                    version,
                    msg_id,
                    offset_px,
                });
                false
            }
            other => {
                inner.actions.push((version, other.clone()));
                inner.queue_event(ReaderEvent::ThreadAction {
                    version,
                    action: other,
                });
                true
            }
        }
    };
    if should_flush {
        flush_events(state);
    } else {
        // Still deliver ScrollAnchor to any typed callback without marking GPUI dirty if unneeded.
        flush_events(state);
    }
    true
}

/// Read all bytes from an `IStream` from its beginning.
pub fn read_stream(stream: &IStream) -> Option<Vec<u8>> {
    use windows::Win32::System::Com::STREAM_SEEK_SET;

    unsafe { stream.Seek(0, STREAM_SEEK_SET, None).ok()? }
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let mut read = 0u32;
        let _ = unsafe {
            stream.Read(
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                Some(&mut read),
            )
        };
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read as usize]);
    }
    (!bytes.is_empty()).then_some(bytes)
}

unsafe fn configure(
    env: &ICoreWebView2Environment,
    controller: &ICoreWebView2Controller,
    state: Weak<RefCell<State>>,
) -> windows::core::Result<()> {
    let view = controller.CoreWebView2()?;
    if let Ok(settings) = unsafe { view.Settings() } {
        unsafe {
            let _ = settings.SetAreDevToolsEnabled(false);
            let _ = settings.SetIsStatusBarEnabled(false);
        }
    }
    let mut token = 0;
    let nav_state = state.clone();
    let nav = NavigationStartingEventHandler::create(Box::new(move |_, args| {
        if let Some(args) = args {
            let mut uri = windows::core::PWSTR::null();
            unsafe {
                args.Uri(&mut uri)?;
            }
            let url = webview2_com::take_pwstr(uri);
            let allowed = nav_state
                .upgrade()
                .is_some_and(|state| should_allow_main_navigation(&state.borrow(), &url));
            if allowed {
                let mut id = 0;
                unsafe {
                    args.NavigationId(&mut id)?;
                }
                if let Some(state) = nav_state.upgrade() {
                    state.borrow_mut().navigation_id = Some(id);
                }
            }
            unsafe {
                args.SetCancel(!allowed)?;
            }
        }
        Ok(())
    }));
    view.add_NavigationStarting(&nav, &mut token)?;
    if let Ok(view2) = view.cast::<ICoreWebView2_2>() {
        let dom_state = state.clone();
        let dom_loaded = DOMContentLoadedEventHandler::create(Box::new(move |view, args| {
            if let (Some(state), Some(args)) = (dom_state.upgrade(), args) {
                let mut id = 0;
                unsafe {
                    args.NavigationId(&mut id)?;
                }
                let matched = apply_dom_content_loaded(&mut state.borrow_mut(), id);
                if matched {
                    flush_events(&state);
                    maybe_install_bridge_script(&state, view.as_ref());
                }
            }
            Ok(())
        }));
        let _ = view2.add_DOMContentLoaded(&dom_loaded, &mut token);
    }
    let complete_state = state.clone();
    let complete = NavigationCompletedEventHandler::create(Box::new(move |view, args| {
        if let (Some(state), Some(args)) = (complete_state.upgrade(), args) {
            let mut id = 0;
            let mut success = windows::core::BOOL::default();
            unsafe {
                args.NavigationId(&mut id)?;
                args.IsSuccess(&mut success)?;
            }
            let matched =
                apply_navigation_completed(&mut state.borrow_mut(), id, success.as_bool());
            flush_events(&state);
            if matched {
                maybe_install_bridge_script(&state, view.as_ref());
                let should_suspend = {
                    let mut inner = state.borrow_mut();
                    let flag = inner.suspend_after_load
                        && !inner.visible
                        && !inner.suspend_inhibited
                        && !inner.closed;
                    inner.suspend_after_load = false;
                    flag
                };
                if should_suspend {
                    try_suspend_state(&state);
                }
            }
        }
        Ok(())
    }));
    view.add_NavigationCompleted(&complete, &mut token)?;
    let message_state = state.clone();
    let web_message = WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
        if let (Some(state), Some(args)) = (message_state.upgrade(), args) {
            let mut message = windows::core::PWSTR::null();
            if unsafe { args.TryGetWebMessageAsString(&mut message) }.is_ok() {
                let message = webview2_com::take_pwstr(message);
                let mut source = windows::core::PWSTR::null();
                unsafe { args.Source(&mut source)? };
                let source = webview2_com::take_pwstr(source);
                if let Some(url) = open_link_message(&message) {
                    record_script_link(&state, &source, url);
                } else {
                    record_thread_action(&state, &source, &message);
                }
            }
        }
        Ok(())
    }));
    view.add_WebMessageReceived(&web_message, &mut token)?;
    let failure_state = state.clone();
    let failure = ProcessFailedEventHandler::create(Box::new(move |_, _| {
        if let Some(state) = failure_state.upgrade() {
            {
                let mut inner = state.borrow_mut();
                inner.loaded = false;
                inner.failed = true;
                inner.process_failed = true;
                let version = inner.version;
                inner.queue_event(ReaderEvent::ProcessFailed { version });
                inner.queue_event(ReaderEvent::StatusChanged {
                    version,
                    status: ReaderLoadStatus::ProcessFailed,
                });
            }
            flush_events(&state);
        }
        Ok(())
    }));
    view.add_ProcessFailed(&failure, &mut token)?;
    let frame_state = state.clone();
    let frame = NavigationStartingEventHandler::create(Box::new(move |_, args| {
        if let Some(args) = args {
            let mut uri = windows::core::PWSTR::null();
            unsafe {
                args.Uri(&mut uri)?;
            }
            let url = webview2_com::take_pwstr(uri);
            let mut initiated = windows::core::BOOL::default();
            unsafe {
                args.IsUserInitiated(&mut initiated)?;
            }
            if initiated.as_bool() && allowed_external_url(&url) {
                if let Some(state) = frame_state.upgrade() {
                    let changed = record_external_link(&mut state.borrow_mut(), &url);
                    if changed {
                        flush_events(&state);
                    }
                }
            }
            unsafe {
                args.SetCancel(!allow_frame_navigation(&url))?;
            }
        }
        Ok(())
    }));
    view.add_FrameNavigationStarting(&frame, &mut token)?;
    let new_window_state = state.clone();
    let new_window = NewWindowRequestedEventHandler::create(Box::new(move |_, args| {
        if let Some(args) = args {
            let mut initiated = windows::core::BOOL::default();
            let mut uri = windows::core::PWSTR::null();
            unsafe {
                args.IsUserInitiated(&mut initiated)?;
                args.Uri(&mut uri)?;
            }
            let url = webview2_com::take_pwstr(uri);
            if initiated.as_bool() && allowed_external_url(&url) {
                if let Some(state) = new_window_state.upgrade() {
                    let changed = record_external_link(&mut state.borrow_mut(), &url);
                    if changed {
                        flush_events(&state);
                    }
                }
            }
            unsafe {
                args.SetHandled(true)?;
            }
        }
        Ok(())
    }));
    view.add_NewWindowRequested(&new_window, &mut token)?;
    if let Ok(view18) = view.cast::<ICoreWebView2_18>() {
        let external_state = state.clone();
        let external = LaunchingExternalUriSchemeEventHandler::create(Box::new(move |_, args| {
            if let Some(args) = args {
                unsafe {
                    args.SetCancel(true)?;
                }
                let mut initiated = windows::core::BOOL::default();
                let mut uri = windows::core::PWSTR::null();
                unsafe {
                    args.IsUserInitiated(&mut initiated)?;
                    args.Uri(&mut uri)?;
                }
                let url = webview2_com::take_pwstr(uri);
                if initiated.as_bool() && allowed_external_url(&url) {
                    if let Some(state) = external_state.upgrade() {
                        let changed = record_external_link(&mut state.borrow_mut(), &url);
                        if changed {
                            flush_events(&state);
                        }
                    }
                }
            }
            Ok(())
        }));
        let _ = view18.add_LaunchingExternalUriScheme(&external, &mut token);
    }
    let env = env.clone();
    let resource = WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
        if retired_resource_handler(&state) {
            if let Some(args) = args {
                if let Some(stream) = unsafe { SHCreateMemStream(Some(&[])) } {
                    let response = unsafe {
                        env.CreateWebResourceResponse(
                            &stream,
                            403,
                            w!("Forbidden"),
                            w!("Cache-Control: no-store"),
                        )?
                    };
                    unsafe {
                        args.SetResponse(&response)?;
                    }
                }
            }
            return Ok(());
        }
        if let (Some(state), Some(args)) = (state.upgrade(), args) {
            let mut uri = windows::core::PWSTR::null();
            let request = unsafe { args.Request() }.ok();
            if let Some(request) = &request {
                unsafe {
                    let _ = request.Uri(&mut uri);
                }
            }
            let url = webview2_com::take_pwstr(uri);
            let mut context = COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL;
            unsafe {
                let _ = args.ResourceContext(&mut context);
            }
            let mut referer = windows::core::PWSTR::null();
            let referer = if request.as_ref().is_some_and(|request| unsafe {
                request
                    .Headers()
                    .and_then(|headers| headers.GetHeader(w!("Referer"), &mut referer))
                    .is_ok()
            }) {
                webview2_com::take_pwstr(referer)
            } else {
                String::new()
            };
            let state = state.borrow();
            let current = allow_main_navigation(&url, state.version);
            let internal = is_internal_reader_url(&url);
            let allow_external = !internal
                && allow_remote_image_request(
                    &url,
                    context == COREWEBVIEW2_WEB_RESOURCE_CONTEXT_IMAGE,
                    state.allow_remote_images,
                    &referer,
                    state.version,
                );
            let allow_remote_images = current && state.allow_remote_images;
            if allow_external
                && request
                    .as_ref()
                    .is_some_and(|request| clear_private_image_headers(request).is_ok())
            {
                return Ok(());
            }
            let stream =
                unsafe { SHCreateMemStream(Some(if current { &state.document } else { &[] })) };
            drop(state);
            if let Some(stream) = stream {
                let headers: Vec<u16> = document_headers(allow_remote_images)
                    .encode_utf16()
                    .chain(Some(0))
                    .collect();
                let response = unsafe {
                    env.CreateWebResourceResponse(
                        &stream,
                        if current {
                            200
                        } else if internal {
                            404
                        } else {
                            403
                        },
                        if current {
                            w!("OK")
                        } else if internal {
                            w!("Not Found")
                        } else {
                            w!("Forbidden")
                        },
                        windows::core::PCWSTR(headers.as_ptr()),
                    )?
                };
                unsafe {
                    args.SetResponse(&response)?;
                }
            }
        }
        Ok(())
    }));
    view.AddWebResourceRequestedFilter(w!("https://*"), COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL)?;
    view.AddWebResourceRequestedFilter(w!("http://*"), COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL)?;
    view.add_WebResourceRequested(&resource, &mut token)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    #[ignore = "Windows WebView2 loopback smoke; launches an isolated subprocess"]
    fn isolated_remote_image_smoke() {
        use std::io::{Read, Write};
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        use std::time::{Duration, Instant};
        if std::env::var_os("KITEMAIL_WEBVIEW_SMOKE_CHILD").is_none() {
            use std::hash::{BuildHasher, Hasher};
            let mut random = std::collections::hash_map::RandomState::new().build_hasher();
            random.write_u32(std::process::id());
            let dir = std::env::temp_dir().join(format!(
                "kitemail-webview-smoke-{}-{:016x}",
                std::process::id(),
                random.finish()
            ));
            std::fs::create_dir(&dir).unwrap();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--ignored",
                    "--exact",
                    "webview2::tests::isolated_remote_image_smoke",
                    "--nocapture",
                ])
                .env("KITEMAIL_WEBVIEW_SMOKE_CHILD", "1")
                .env("KITEMAIL_DATA_DIR", &dir)
                .status()
                .unwrap();
            assert!(status.success());
            let _ = std::fs::remove_dir_all(dir);
            return;
        }
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
        use windows::Win32::UI::WindowsAndMessaging::{
            DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE, WS_OVERLAPPEDWINDOW,
        };
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
        }
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                w!("STATIC"),
                w!(""),
                WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
                0,
                0,
                200,
                200,
                None,
                None,
                None,
                None,
            )
            .unwrap()
        };
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let old_count = Arc::new(AtomicUsize::new(0));
        let new_count = Arc::new(AtomicUsize::new(0));
        let second_count = Arc::new(AtomicUsize::new(0));
        let private_headers = Arc::new(AtomicUsize::new(0));
        let old_cl = old_count.clone();
        let new_cl = new_count.clone();
        let second_cl = second_count.clone();
        let private_cl = private_headers.clone();
        std::thread::spawn(move || {
            let end = Instant::now() + Duration::from_secs(25);
            while Instant::now() < end {
                if let Ok((mut stream, _)) = listener.accept() {
                    let mut buf = [0; 4096];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    let request = String::from_utf8_lossy(&buf[..n]);
                    let headers = request.to_ascii_lowercase();
                    if headers.contains("\r\nreferer:")
                        || headers.contains("\r\ncookie:")
                        || headers.contains("\r\nauthorization:")
                    {
                        private_cl.fetch_add(1, Ordering::SeqCst);
                    }
                    if request.starts_with("GET /old") {
                        old_cl.fetch_add(1, Ordering::SeqCst);
                    }
                    if request.starts_with("GET /new") {
                        new_cl.fetch_add(1, Ordering::SeqCst);
                    }
                    if request.starts_with("GET /second") {
                        second_cl.fetch_add(1, Ordering::SeqCst);
                    }
                    let resp = if request.starts_with("GET /new") {
                        "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nSet-Cookie: kitemail_track=1; Path=/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    } else {
                        "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    };
                    let _ = stream.write_all(resp.as_bytes());
                } else {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        });
        let user_data = kitemail_mail_core::config::data_dir().join("WebView2");
        let old = WebView2Host::create(hwnd, user_data.clone()).unwrap();
        old.load_with_remote(format!("<img src='http://{addr}/old'>").into_bytes(), false)
            .unwrap();
        let pump = |duration: Duration| {
            let end = Instant::now() + duration;
            while Instant::now() < end {
                let mut msg = MSG::default();
                unsafe {
                    while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        pump(Duration::from_secs(3));
        assert_eq!(old_count.load(Ordering::SeqCst), 0);
        assert!(old.retire_for_remote_reload(true));
        let new = WebView2Host::create(hwnd, user_data.clone()).unwrap();
        new.load_with_remote(format!("<img src='http://{addr}/new'>").into_bytes(), true)
            .unwrap();
        pump(Duration::from_secs(5));
        assert_eq!(old_count.load(Ordering::SeqCst), 0);
        assert!(new_count.load(Ordering::SeqCst) > 0);
        assert!(new.retire_for_remote_reload(true));
        let second = WebView2Host::create(hwnd, user_data).unwrap();
        second
            .load_with_remote(
                format!("<img src='http://{addr}/second'>").into_bytes(),
                true,
            )
            .unwrap();
        pump(Duration::from_secs(5));
        assert!(second_count.load(Ordering::SeqCst) > 0);
        assert_eq!(private_headers.load(Ordering::SeqCst), 0);
        drop(second);
        drop(new);
        drop(old);
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }

    #[test]
    fn retired_resource_handler_checks_closed_or_dropped_state() {
        let old = Rc::new(RefCell::new(State::default()));
        let callback = Rc::downgrade(&old);
        assert!(!retired_resource_handler(&callback));
        old.borrow_mut().closed = true;
        assert!(retired_resource_handler(&callback));
        drop(old);
        assert!(retired_resource_handler(&callback));
    }

    #[test]
    fn thread_actions_validate_source_mode_and_scroll_anchors() {
        let (tx, rx) = mpsc::channel::<ReaderEvent>();
        let state = Rc::new(RefCell::new(State {
            version: 2,
            loaded: true,
            mode: DocumentMode::ThreadContainer,
            callback: Some(Rc::new(RefCell::new(move |ev| {
                let _ = tx.send(ev);
            }))),
            ..State::default()
        }));

        let host = std::mem::ManuallyDrop::new(WebView2Host(state.clone()));

        // Scroll anchor updates silently without entering action queue.
        assert!(record_thread_action(
            &state,
            &document_url(2),
            "scroll-anchor:9:48"
        ));
        assert_eq!(
            rx.try_recv().ok(),
            Some(ReaderEvent::ScrollAnchor {
                version: 2,
                msg_id: 9,
                offset_px: 48
            })
        );
        assert_eq!(host.last_scroll_anchor(), Some((9, 48)));
        assert!(host.take_actions().is_empty());

        // Action message queues and wakes callback; wrong version or SingleBody mode rejects.
        assert!(record_thread_action(
            &state,
            &document_url(2),
            "card-reply:9"
        ));
        assert_eq!(
            rx.try_recv().ok(),
            Some(ReaderEvent::ThreadAction {
                version: 2,
                action: WebViewAction::CardReply(9)
            })
        );
        assert!(!record_thread_action(
            &state,
            &document_url(1),
            "card-reply:9"
        ));

        state.borrow_mut().mode = DocumentMode::SingleBody;
        assert!(!record_thread_action(
            &state,
            &document_url(2),
            "card-reply:9"
        ));

        state.borrow_mut().mode = DocumentMode::ThreadContainer;
        assert_eq!(host.take_actions(), vec![WebViewAction::CardReply(9)]);
    }

    #[test]
    fn clipping_keeps_the_document_size_and_offsets_the_view() {
        let content = RECT {
            left: 10,
            top: 20,
            right: 210,
            bottom: 320,
        };
        let viewport = RECT {
            left: 30,
            top: 50,
            right: 180,
            bottom: 200,
        };
        let (clip, bounds) = clipped_rects(content, viewport).unwrap();
        assert_eq!(
            (clip.left, clip.top, clip.right, clip.bottom),
            (30, 50, 180, 200)
        );
        assert_eq!(
            (bounds.left, bounds.top, bounds.right, bounds.bottom),
            (-20, -30, 180, 270)
        );
        assert!(clipped_rects(
            content,
            RECT {
                left: 300,
                top: 0,
                right: 400,
                bottom: 100
            }
        )
        .is_none());

        let px = logical_to_pixel_rect(
            LogicalRect {
                x: 10.0,
                y: 20.0,
                width: 100.0,
                height: 200.0,
            },
            1.5,
        );
        assert_eq!(
            px,
            PixelRect {
                left: 15,
                top: 30,
                right: 165,
                bottom: 330
            }
        );
    }

    #[test]
    fn script_mailto_links_validate_source_state_scheme_and_dedup() {
        let (tx, rx) = mpsc::channel::<ReaderEvent>();
        let state = Rc::new(RefCell::new(State {
            version: 5,
            loaded: true,
            callback: Some(Rc::new(RefCell::new(move |ev| {
                let _ = tx.send(ev);
            }))),
            ..State::default()
        }));

        // Valid mailto from current document version queues link and notifies callback.
        assert!(record_script_link(
            &state,
            &document_url(5),
            "mailto:team@makenotion.com"
        ));
        assert_eq!(
            rx.try_recv().ok(),
            Some(ReaderEvent::ExternalLink {
                version: 5,
                url: "mailto:team@makenotion.com".to_string()
            })
        );

        // Adjacent duplicate in the same version is deduplicated.
        assert!(record_script_link(
            &state,
            &document_url(5),
            "mailto:team@makenotion.com"
        ));
        assert!(rx.try_recv().is_err());

        // Uppercase MAILTO is accepted.
        assert!(record_script_link(
            &state,
            &document_url(5),
            "MAILTO:other@example.com"
        ));
        assert_eq!(
            rx.try_recv().ok(),
            Some(ReaderEvent::ExternalLink {
                version: 5,
                url: "MAILTO:other@example.com".to_string()
            })
        );

        // Stale/wrong source, non-mailto schemes, unloaded or closed states are rejected.
        assert!(!record_script_link(
            &state,
            &document_url(4),
            "mailto:x@example.com"
        ));
        assert!(!record_script_link(
            &state,
            "https://example.com/",
            "mailto:x@example.com"
        ));
        assert!(!record_script_link(
            &state,
            &document_url(5),
            "https://example.com/"
        ));
        assert!(!record_script_link(
            &state,
            &document_url(5),
            "file:///C:/test"
        ));
        assert!(!record_script_link(
            &state,
            &document_url(5),
            "javascript:alert(1)"
        ));
        assert!(!record_script_link(&state, &document_url(5), "mailto:"));

        state.borrow_mut().loaded = false;
        assert!(!record_script_link(
            &state,
            &document_url(5),
            "mailto:unloaded@example.com"
        ));
        state.borrow_mut().loaded = true;
        state.borrow_mut().closed = true;
        assert!(!record_script_link(
            &state,
            &document_url(5),
            "mailto:closed@example.com"
        ));

        let host = std::mem::ManuallyDrop::new(WebView2Host(state));
        let links = host.take_links();
        assert_eq!(
            links,
            vec![
                "mailto:team@makenotion.com".to_string(),
                "MAILTO:other@example.com".to_string()
            ]
        );
    }

    #[test]
    fn dom_content_loaded_marks_ready_and_wakes_notifier_before_navigation_completed() {
        let (tx, rx) = mpsc::channel::<ReaderEvent>();
        let state = Rc::new(RefCell::new(State {
            version: 3,
            navigation_id: Some(42),
            callback: Some(Rc::new(RefCell::new(move |ev| {
                let _ = tx.send(ev);
            }))),
            ..State::default()
        }));

        // Stale navigation ID is ignored.
        assert!(!apply_dom_content_loaded(&mut state.borrow_mut(), 7));
        flush_events(&state);
        assert!(!state.borrow().loaded);
        assert!(rx.try_recv().is_err());

        // Matching DOMContentLoaded wakes callback immediately while remote images may still load.
        assert!(apply_dom_content_loaded(&mut state.borrow_mut(), 42));
        flush_events(&state);
        assert!(state.borrow().loaded);
        assert!(!state.borrow().failed);
        assert_eq!(
            rx.try_recv().ok(),
            Some(ReaderEvent::StatusChanged {
                version: 3,
                status: ReaderLoadStatus::Ready
            })
        );

        // Explicit main-document navigation failure resets loaded back to false.
        assert!(!apply_navigation_completed(
            &mut state.borrow_mut(),
            42,
            false
        ));
        flush_events(&state);
        assert!(!state.borrow().loaded);
        assert!(state.borrow().failed);
        assert_eq!(
            rx.try_recv().ok(),
            Some(ReaderEvent::StatusChanged {
                version: 3,
                status: ReaderLoadStatus::Unavailable
            })
        );
    }

    #[test]
    fn navigation_completed_frees_document_and_blocks_reloads() {
        let (tx, rx) = mpsc::channel::<ReaderEvent>();
        let state = Rc::new(RefCell::new(State {
            version: 2,
            navigation_id: Some(11),
            document: b"<html><body>big email</body></html>".to_vec(),
            callback: Some(Rc::new(RefCell::new(move |ev| {
                let _ = tx.send(ev);
            }))),
            ..State::default()
        }));

        assert!(should_allow_main_navigation(
            &state.borrow(),
            &document_url(2)
        ));

        assert!(apply_navigation_completed(
            &mut state.borrow_mut(),
            11,
            true
        ));
        flush_events(&state);
        assert!(state.borrow().loaded);
        assert!(state.borrow().document.is_empty());
        assert_eq!(
            rx.try_recv().ok(),
            Some(ReaderEvent::StatusChanged {
                version: 2,
                status: ReaderLoadStatus::Ready
            })
        );
        assert!(!should_allow_main_navigation(
            &state.borrow(),
            &document_url(2)
        ));
    }

    #[test]
    fn try_suspend_state_and_closed_host_preserve_status() {
        let rc = Rc::new(RefCell::new(State {
            version: 3,
            navigation_id: Some(7),
            suspend_after_load: true,
            ..State::default()
        }));
        try_suspend_state(&rc);
        assert!(!rc.borrow().closed);
        assert!(!rc.borrow().suspending);

        let host = std::mem::ManuallyDrop::new(WebView2Host(Rc::clone(&rc)));
        assert_eq!(host.status(), (3, ReaderLoadStatus::Loading));

        rc.borrow_mut().closed = true;
        assert!(!apply_navigation_completed(&mut rc.borrow_mut(), 7, true));
        assert_eq!(host.status(), (3, ReaderLoadStatus::Unavailable));

        let res = host.load_document_with_suspend(
            b"<p>x</p>".to_vec(),
            false,
            DocumentMode::SingleBody,
            true,
        );
        assert!(res.is_ok());
        assert!(rc.borrow().document.is_empty());
        assert_eq!(host.status(), (3, ReaderLoadStatus::Unavailable));
    }

    #[test]
    fn snapshot_stream_is_read_from_the_start_and_stops_at_the_end() {
        let png = b"\x89PNG\r\n\x1a\nnot-really-a-png";
        let stream = unsafe { SHCreateMemStream(Some(png)) }.unwrap();
        assert_eq!(read_stream(&stream).as_deref(), Some(&png[..]));
        assert_eq!(read_stream(&stream).as_deref(), Some(&png[..]));
        let empty = unsafe { SHCreateMemStream(None) }.unwrap();
        assert!(read_stream(&empty).is_none());
    }

    #[test]
    fn clip_window_pos_plan_lifts_to_hwnd_top_without_swp_nozorder() {
        use windows::Win32::UI::WindowsAndMessaging::SWP_NOZORDER;

        let moved = plan_clip_window_pos(true, true).expect("clip_changed plans SetWindowPos");
        assert!(moved.insert_after_top);
        assert!(moved.flags.contains(SWP_NOACTIVATE));
        assert!(!moved.flags.contains(SWP_NOZORDER));

        let shown = plan_clip_window_pos(false, true).expect("visible plans z-order lift");
        assert!(shown.insert_after_top);
        assert!(shown.flags.contains(SWP_NOACTIVATE));
        assert!(shown.flags.contains(SWP_NOMOVE));
        assert!(shown.flags.contains(SWP_NOSIZE));
        assert!(!shown.flags.contains(SWP_NOZORDER));

        assert!(plan_clip_window_pos(false, false).is_none());
    }

    #[test]
    fn dom_ready_and_stage_error_distinguish_navigation_from_layout() {
        let (tx, rx) = mpsc::channel::<ReaderEvent>();
        let state = Rc::new(RefCell::new(State {
            version: 4,
            navigation_id: Some(9),
            callback: Some(Rc::new(RefCell::new(move |ev| {
                let _ = tx.send(ev);
            }))),
            ..State::default()
        }));

        assert!(apply_dom_content_loaded(&mut state.borrow_mut(), 9));
        flush_events(&state);
        assert!(state.borrow().loaded);
        assert!(!state.borrow().layout_applied);
        assert_eq!(
            rx.try_recv().ok(),
            Some(ReaderEvent::StatusChanged {
                version: 4,
                status: ReaderLoadStatus::Ready
            })
        );

        record_stage_error(&state, "set_window_pos", "mock failure".to_string());
        assert!(state.borrow().failed);
        assert!(!state.borrow().layout_applied);
        assert_eq!(
            rx.try_recv().ok(),
            Some(ReaderEvent::Error {
                version: 4,
                stage: "set_window_pos",
                detail: "mock failure".to_string()
            })
        );
        assert_eq!(
            rx.try_recv().ok(),
            Some(ReaderEvent::StatusChanged {
                version: 4,
                status: ReaderLoadStatus::Unavailable
            })
        );
    }
}
