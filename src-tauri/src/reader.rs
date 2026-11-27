//! Tauri main-thread native reader lifecycle, layout/visibility coordinator,
//! and shared `kitemail-windows-mail-reader` WebView2 host adapter.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use kitemail_windows_mail_reader::{
    build_body_document, supports_document as reader_supports_document, DocumentAppearance,
};

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
    pub fn to_document_appearance(&self) -> DocumentAppearance {
        DocumentAppearance::for_dark(self.dark, self.zoom_percent, self.block_remote_images)
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
    fn load_html(
        &self,
        session_id: String,
        account_id: u32,
        message_id: u32,
        html_bytes: Vec<u8>,
        allow_remote_images: bool,
    );
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
        _allow_remote_images: bool,
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
                message: None,
                retryable: None,
                plain_text: Some(doc.plain_text.clone()),
                has_html: Some(doc.html_body.is_some()),
                html_body: doc.html_body.clone(),
            });
            return status;
        }

        if should_load_native && self.sink.is_native_supported() {
            let html = doc.html_body.as_deref().unwrap_or("");
            let document_bytes = build_body_document(html, &appearance.to_document_appearance());
            self.sink.load_html(
                session_id.clone(),
                account_id,
                message_id,
                document_bytes,
                !appearance.block_remote_images,
            );
            self.sink.set_zoom(appearance.zoom_percent);
            self.sink.emit_status(ReaderStatusEvent {
                reader_session_id: session_id,
                account_id,
                message_id,
                body_status: BodyStatusDto::Loading,
                detail: None,
                message: None,
                retryable: None,
                plain_text: Some(doc.plain_text.clone()),
                has_html: Some(true),
                html_body: doc.html_body.clone(),
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
                message: None,
                retryable: None,
                plain_text: Some(doc.plain_text.clone()),
                has_html: Some(doc.html_body.is_some()),
                html_body: doc.html_body.clone(),
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
                let bytes = build_body_document(html, &appearance.to_document_appearance());
                self.sink.load_html(
                    doc.session_id,
                    doc.account_id,
                    doc.message_id,
                    bytes,
                    !appearance.block_remote_images,
                );
                self.sink.set_zoom(appearance.zoom_percent);
            }
        }
    }

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
            self.sink.close();
        }
    }

    pub fn emit_reader_status(&self, event: ReaderStatusEvent) {
        self.sink.emit_status(event);
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
    reader_supports_document(html)
}

/// Windows WebView2 child-HWND sink delegating to shared `kitemail-windows-mail-reader`.
#[cfg(windows)]
pub mod platform_win {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    use kitemail_windows_mail_reader::{
        prepare_external_url, PixelRect, ReaderEvent, ReaderLoadStatus, WebView2Host,
    };
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use tauri::Emitter;
    use windows::{
        core::w,
        Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNOACTIVATE},
    };

    #[derive(Default)]
    struct ActiveSessionMeta {
        expected_version: u64,
        session_id: String,
        account_id: u32,
        message_id: u32,
    }

    struct HostSlot {
        host: WebView2Host,
        meta: Rc<RefCell<ActiveSessionMeta>>,
    }

    thread_local! {
        static MAIN_THREAD_HOST: RefCell<Option<HostSlot>> = const { RefCell::new(None) };
    }

    fn open_external_url(raw: &str) {
        let Some(clean) = prepare_external_url(raw, true) else {
            return;
        };
        let wide: Vec<u16> = clean.encode_utf16().chain(Some(0)).collect();
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

    fn create_host_slot(
        app_handle: &tauri::AppHandle,
        user_data_dir: &PathBuf,
    ) -> Option<HostSlot> {
        let win = tauri::Manager::get_webview_window(app_handle, MAIN_WINDOW_LABEL)?;
        let raw = HasWindowHandle::window_handle(&win).ok()?;
        let RawWindowHandle::Win32(win32) = raw.as_raw() else {
            return None;
        };
        let host = WebView2Host::from_raw_hwnd(win32.hwnd.get(), user_data_dir.clone()).ok()?;
        let meta = Rc::new(RefCell::new(ActiveSessionMeta::default()));
        let cb_meta = meta.clone();
        let cb_handle = app_handle.clone();
        host.set_event_callback(move |event| match event {
            ReaderEvent::StatusChanged { version, status } => {
                let active = cb_meta.borrow();
                if version != active.expected_version || active.session_id.is_empty() {
                    return;
                }
                let (body_status, detail, retryable) = match status {
                    ReaderLoadStatus::Loading => (BodyStatusDto::Loading, None, None),
                    ReaderLoadStatus::Ready => (BodyStatusDto::Ready, None, None),
                    ReaderLoadStatus::Unavailable => (
                        BodyStatusDto::Unavailable,
                        Some("邮件正文加载失败".to_string()),
                        Some(true),
                    ),
                    ReaderLoadStatus::ProcessFailed => (
                        BodyStatusDto::Unavailable,
                        Some("WebView2 阅读进程异常退出".to_string()),
                        Some(true),
                    ),
                };
                let _ = cb_handle.emit_to(
                    MAIN_WINDOW_LABEL,
                    EVENT_READER_STATUS,
                    ReaderStatusEvent {
                        reader_session_id: active.session_id.clone(),
                        account_id: active.account_id,
                        message_id: active.message_id,
                        body_status,
                        detail: detail.clone(),
                        message: detail,
                        retryable,
                        plain_text: None,
                        has_html: None,
                        html_body: None,
                    },
                );
            }
            ReaderEvent::ProcessFailed { version } => {
                let active = cb_meta.borrow();
                if version != active.expected_version || active.session_id.is_empty() {
                    return;
                }
                let detail = Some("WebView2 阅读进程异常退出".to_string());
                let _ = cb_handle.emit_to(
                    MAIN_WINDOW_LABEL,
                    EVENT_READER_STATUS,
                    ReaderStatusEvent {
                        reader_session_id: active.session_id.clone(),
                        account_id: active.account_id,
                        message_id: active.message_id,
                        body_status: BodyStatusDto::Unavailable,
                        detail: detail.clone(),
                        message: detail,
                        retryable: Some(true),
                        plain_text: None,
                        has_html: None,
                        html_body: None,
                    },
                );
            }
            ReaderEvent::ExternalLink { url, .. } => {
                open_external_url(&url);
            }
            ReaderEvent::ThreadAction { .. }
            | ReaderEvent::ScrollAnchor { .. }
            | ReaderEvent::StateChanged => {}
        });
        Some(HostSlot { host, meta })
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
            F: FnOnce(&tauri::AppHandle, &PathBuf, &mut Option<HostSlot>) + Send + 'static,
        {
            let handle = self.app_handle.clone();
            let user_data_dir = self.user_data_dir.clone();
            let _ = self.app_handle.run_on_main_thread(move || {
                MAIN_THREAD_HOST.with(|slot| {
                    let mut borrow = slot.borrow_mut();
                    if borrow.is_none() {
                        *borrow = create_host_slot(&handle, &user_data_dir);
                    }
                    f(&handle, &user_data_dir, &mut borrow);
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
            allow_remote_images: bool,
        ) {
            self.with_host_on_main_thread(move |handle, user_data_dir, slot| {
                if let Some(current) = slot.as_ref() {
                    if current.host.retire_for_remote_reload(allow_remote_images) {
                        current.host.close();
                        *slot = create_host_slot(handle, user_data_dir);
                    }
                }
                let Some(current) = slot.as_ref() else {
                    return;
                };
                {
                    let mut meta = current.meta.borrow_mut();
                    meta.expected_version = current.host.version().wrapping_add(1);
                    meta.session_id = session_id;
                    meta.account_id = account_id;
                    meta.message_id = message_id;
                }
                if let Ok(version) = current
                    .host
                    .load_with_remote(html_bytes, allow_remote_images)
                {
                    current.meta.borrow_mut().expected_version = version;
                }
            });
        }

        fn apply_layout(&self, clip: PhysicalRect, bounds: PhysicalRect, visible: bool) {
            self.with_host_on_main_thread(move |_, _, slot| {
                let Some(current) = slot.as_ref() else {
                    return;
                };
                let clip_px = PixelRect {
                    left: clip.left,
                    top: clip.top,
                    right: clip.right,
                    bottom: clip.bottom,
                };
                let content_px = PixelRect {
                    left: clip.left + bounds.left,
                    top: clip.top + bounds.top,
                    right: clip.left + bounds.right,
                    bottom: clip.top + bounds.bottom,
                };
                current.host.set_bounds(content_px, clip_px, visible);
            });
        }

        fn set_visible(&self, visible: bool) {
            self.with_host_on_main_thread(move |_, _, slot| {
                if let Some(current) = slot.as_ref() {
                    current.host.set_visible(visible);
                }
            });
        }

        fn set_zoom(&self, zoom_percent: u32) {
            let factor = f64::from(zoom_percent.clamp(50, 250)) / 100.0;
            self.with_host_on_main_thread(move |_, _, slot| {
                if let Some(current) = slot.as_ref() {
                    current.host.set_zoom(factor);
                }
            });
        }

        fn close(&self) {
            let _ = self.app_handle.run_on_main_thread(|| {
                MAIN_THREAD_HOST.with(|slot| {
                    if let Some(current) = slot.borrow_mut().take() {
                        current.host.close();
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

        fn load_html(&self, _: String, _: u32, _: u32, _: Vec<u8>, _: bool) {}

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
