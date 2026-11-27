//! Tauri main-thread native reader lifecycle, layout/visibility coordinator,
//! and shared `kitemail-windows-mail-reader` WebView2 host adapter.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};

use kitemail_mail_core::config::normalize_sender_email;
use kitemail_mail_core::MessageDetail;
use kitemail_windows_mail_reader::{
    build_body_document, build_thread_document, supports_document as reader_supports_document,
    DocumentAppearance, DocumentMode, ThreadCardRenderInput, ThreadDocContext, ThreadPalette,
    WebViewAction,
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
pub struct ThreadRenderOptions {
    pub detail: MessageDetail,
    pub collapsed_cards: HashSet<u32>,
    pub raw_cards: HashSet<u32>,
    pub allowed_remote_cards: HashSet<u32>,
    pub allowed_remote_senders: Vec<String>,
    pub ui_text_scale_percent: u32,
    pub date_style: String,
    pub clock_style: String,
    pub always_show_recipients: bool,
    pub demo_mode: bool,
    pub read_receipt_policy: String,
    pub initial_anchor: Option<(u32, i32)>,
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
    pub thread: Option<ThreadRenderOptions>,
}

/// Callback handler invoked when the native thread reader emits a `WebViewAction`.
pub trait ThreadActionHandler: Send + Sync {
    fn handle_thread_action(&self, account_id: u32, message_id: u32, action: WebViewAction);
}

fn sender_in_allowlist(sender: &str, allowed_senders: &[String]) -> bool {
    let email = normalize_sender_email(sender);
    if email.is_empty() {
        return false;
    }
    allowed_senders
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(&email))
}

fn should_load_native_document(doc: &ReaderDocumentSpec) -> bool {
    if doc
        .thread
        .as_ref()
        .is_some_and(|t| t.detail.thread_messages.len() > 1)
    {
        return true;
    }
    doc.html_body.as_deref().is_some_and(supports_document)
}

/// Build the HTML bytes, remote-image flag, and WebView2 `DocumentMode` for a `ReaderDocumentSpec`.
pub fn build_reader_document_bytes(
    doc: &ReaderDocumentSpec,
    appearance: &ReaderAppearance,
) -> (Vec<u8>, bool, DocumentMode) {
    let doc_appearance = appearance.to_document_appearance();
    if let Some(ref thread) = doc.thread {
        if thread.detail.thread_messages.len() > 1 {
            let palette = ThreadPalette::from_appearance(&doc_appearance);
            let cards: Vec<ThreadCardRenderInput<'_>> = thread
                .detail
                .thread_messages
                .iter()
                .map(|tm| {
                    let is_current = tm.id == thread.detail.id;
                    let is_collapsed = thread.collapsed_cards.contains(&tm.id);
                    let show_raw_source = thread.raw_cards.contains(&tm.id);
                    let sender_allowed =
                        sender_in_allowlist(&tm.sender, &thread.allowed_remote_senders);
                    let block_remote = appearance.block_remote_images
                        && !sender_allowed
                        && !thread.allowed_remote_cards.contains(&tm.id);
                    ThreadCardRenderInput {
                        message: tm,
                        is_current,
                        is_collapsed,
                        show_raw_source,
                        block_remote,
                        card_dark: appearance.dark,
                        card_pending: false,
                        eml_exporting: false,
                        auth_results: is_current
                            .then_some(thread.detail.auth_results.as_deref())
                            .flatten(),
                        unsubscribe_url: is_current
                            .then_some(thread.detail.unsubscribe_url.as_deref())
                            .flatten(),
                    }
                })
                .collect();
            let ctx = ThreadDocContext {
                detail: &thread.detail,
                cards: &cards,
                palette: &palette,
                app_dark_mode: appearance.dark,
                zoom_percent: appearance.zoom_percent,
                ui_text_scale_percent: thread.ui_text_scale_percent,
                date_style: &thread.date_style,
                clock_style: &thread.clock_style,
                show_remote_banner: true,
                always_show_recipients: thread.always_show_recipients,
                card_attachments: true,
                demo_mode: thread.demo_mode,
                read_receipt_policy: &thread.read_receipt_policy,
                initial_anchor: thread.initial_anchor,
            };
            let (bytes, any_remote_allowed) = build_thread_document(&ctx);
            return (bytes, any_remote_allowed, DocumentMode::ThreadContainer);
        }
    }

    let html = doc.html_body.as_deref().unwrap_or("");
    let bytes = build_body_document(html, &doc_appearance);
    (
        bytes,
        !appearance.block_remote_images,
        DocumentMode::SingleBody,
    )
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
        mode: DocumentMode,
    );
    fn apply_layout(&self, clip: PhysicalRect, bounds: PhysicalRect, visible: bool);
    fn set_visible(&self, visible: bool);
    fn set_zoom(&self, zoom_percent: u32);
    fn close(&self);
    fn emit_status(&self, event: ReaderStatusEvent);
    fn set_action_handler(&self, _handler: Weak<dyn ThreadActionHandler>) {}
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
        _mode: DocumentMode,
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

    pub fn set_action_handler(&self, handler: Weak<dyn ThreadActionHandler>) {
        self.sink.set_action_handler(handler);
    }

    /// Start a new reader session for a selected message, resetting the layout revision guard.
    pub fn begin_session(
        &self,
        doc: ReaderDocumentSpec,
        appearance: &ReaderAppearance,
        reader_enabled: bool,
    ) -> BodyStatusDto {
        let should_load_native = should_load_native_document(&doc);
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
                has_html: Some(doc.html_body.is_some() || should_load_native),
                html_body: doc.html_body.clone(),
            });
            return status;
        }

        if should_load_native && self.sink.is_native_supported() {
            let (document_bytes, allow_remote, mode) =
                build_reader_document_bytes(&doc, appearance);
            self.sink.load_html(
                session_id.clone(),
                account_id,
                message_id,
                document_bytes,
                allow_remote,
                mode,
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
        let (doc, reader_enabled, locked, last_layout) = {
            let guard = self.state.lock().expect("reader mutex poisoned");
            (
                guard.active_document.clone(),
                !guard.reader_hidden_pref,
                guard.locked,
                guard.last_layout.clone(),
            )
        };
        if locked || !reader_enabled {
            return;
        }
        if let Some(doc) = doc {
            if should_load_native_document(&doc) && self.sink.is_native_supported() {
                let (bytes, allow_remote, mode) = build_reader_document_bytes(&doc, appearance);
                self.sink.load_html(
                    doc.session_id,
                    doc.account_id,
                    doc.message_id,
                    bytes,
                    allow_remote,
                    mode,
                );
                self.sink.set_zoom(appearance.zoom_percent);
                if let Some(layout) = last_layout {
                    self.sink
                        .apply_layout(layout.clip, layout.bounds, layout.visible);
                }
            }
        }
    }

    /// Mutate the active document (if it matches `account_id` and `message_id` or its thread)
    /// and trigger a WebView2 re-render while preserving the current layout.
    pub fn rerender_matching_document<F>(
        &self,
        account_id: u32,
        message_id: u32,
        appearance: &ReaderAppearance,
        mutator: F,
    ) -> bool
    where
        F: FnOnce(&mut ReaderDocumentSpec),
    {
        let (doc, last_layout, reader_enabled, locked) = {
            let mut guard = self.state.lock().expect("reader mutex poisoned");
            let reader_enabled = !guard.reader_hidden_pref;
            let locked = guard.locked;
            let last_layout = guard.last_layout.clone();
            let doc = if !locked && reader_enabled {
                if let Some(active) = guard.active_document.as_mut() {
                    let matches = active.account_id == account_id
                        && (active.message_id == message_id
                            || active.thread.as_ref().is_some_and(|t| {
                                t.detail.thread_messages.iter().any(|m| m.id == message_id)
                            }));
                    if matches {
                        mutator(active);
                        Some(active.clone())
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };
            (doc, last_layout, reader_enabled, locked)
        };

        if locked || !reader_enabled {
            return false;
        }
        let Some(doc) = doc else {
            return false;
        };
        if should_load_native_document(&doc) && self.sink.is_native_supported() {
            let (bytes, allow_remote, mode) = build_reader_document_bytes(&doc, appearance);
            self.sink.load_html(
                doc.session_id,
                doc.account_id,
                doc.message_id,
                bytes,
                allow_remote,
                mode,
            );
            self.sink.set_zoom(appearance.zoom_percent);
            if let Some(layout) = last_layout {
                self.sink
                    .apply_layout(layout.clip, layout.bounds, layout.visible);
            }
            true
        } else {
            false
        }
    }

    pub fn update_scroll_anchor(
        &self,
        account_id: u32,
        message_id: u32,
        msg_id: u32,
        offset_px: i32,
    ) {
        let mut guard = self.state.lock().expect("reader mutex poisoned");
        if let Some(active) = guard.active_document.as_mut() {
            if active.account_id == account_id && active.message_id == message_id {
                if let Some(ref mut thread) = active.thread {
                    thread.initial_anchor = Some((msg_id, offset_px));
                }
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

    type SharedActionHandler = Arc<Mutex<Option<Weak<dyn ThreadActionHandler>>>>;

    fn create_host_slot(
        app_handle: &tauri::AppHandle,
        user_data_dir: &PathBuf,
        action_handler: &SharedActionHandler,
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
        let cb_action_handler = Arc::clone(action_handler);
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
            ReaderEvent::ThreadAction { version, action } => {
                let target = {
                    let active = cb_meta.borrow();
                    if version != active.expected_version || active.session_id.is_empty() {
                        None
                    } else {
                        Some((active.account_id, active.message_id))
                    }
                };
                if let Some((account_id, message_id)) = target {
                    let handler = cb_action_handler
                        .lock()
                        .ok()
                        .and_then(|g| g.as_ref().and_then(Weak::upgrade));
                    if let Some(h) = handler {
                        h.handle_thread_action(account_id, message_id, action);
                    }
                }
            }
            ReaderEvent::ScrollAnchor {
                version,
                msg_id,
                offset_px,
            } => {
                let target = {
                    let active = cb_meta.borrow();
                    if version != active.expected_version || active.session_id.is_empty() {
                        None
                    } else {
                        Some((active.account_id, active.message_id))
                    }
                };
                if let Some((account_id, message_id)) = target {
                    let handler = cb_action_handler
                        .lock()
                        .ok()
                        .and_then(|g| g.as_ref().and_then(Weak::upgrade));
                    if let Some(h) = handler {
                        h.handle_thread_action(
                            account_id,
                            message_id,
                            WebViewAction::ScrollAnchor { msg_id, offset_px },
                        );
                    }
                }
            }
            ReaderEvent::StateChanged => {}
        });
        Some(HostSlot { host, meta })
    }

    pub struct TauriWindowsReaderSink {
        app_handle: tauri::AppHandle,
        user_data_dir: PathBuf,
        action_handler: SharedActionHandler,
    }

    impl TauriWindowsReaderSink {
        pub fn new(app_handle: tauri::AppHandle, user_data_dir: PathBuf) -> Self {
            Self {
                app_handle,
                user_data_dir,
                action_handler: Arc::new(Mutex::new(None)),
            }
        }

        fn with_host_on_main_thread<F>(&self, f: F)
        where
            F: FnOnce(&tauri::AppHandle, &PathBuf, &SharedActionHandler, &mut Option<HostSlot>)
                + Send
                + 'static,
        {
            let handle = self.app_handle.clone();
            let user_data_dir = self.user_data_dir.clone();
            let action_handler = Arc::clone(&self.action_handler);
            let _ = self.app_handle.run_on_main_thread(move || {
                MAIN_THREAD_HOST.with(|slot| {
                    let mut borrow = slot.borrow_mut();
                    if borrow.is_none() {
                        *borrow = create_host_slot(&handle, &user_data_dir, &action_handler);
                    }
                    f(&handle, &user_data_dir, &action_handler, &mut borrow);
                });
            });
        }
    }

    impl ReaderPlatformSink for TauriWindowsReaderSink {
        fn is_native_supported(&self) -> bool {
            true
        }

        fn set_action_handler(&self, handler: Weak<dyn ThreadActionHandler>) {
            *self
                .action_handler
                .lock()
                .expect("action_handler mutex poisoned") = Some(handler);
        }

        fn load_html(
            &self,
            session_id: String,
            account_id: u32,
            message_id: u32,
            html_bytes: Vec<u8>,
            allow_remote_images: bool,
            mode: DocumentMode,
        ) {
            self.with_host_on_main_thread(move |handle, user_data_dir, action_handler, slot| {
                if let Some(current) = slot.as_ref() {
                    if current.host.retire_for_remote_reload(allow_remote_images) {
                        current.host.close();
                        *slot = create_host_slot(handle, user_data_dir, action_handler);
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
                if let Ok(version) =
                    current
                        .host
                        .load_document(html_bytes, allow_remote_images, mode)
                {
                    current.meta.borrow_mut().expected_version = version;
                }
            });
        }

        fn apply_layout(&self, clip: PhysicalRect, bounds: PhysicalRect, visible: bool) {
            self.with_host_on_main_thread(move |_, _, _, slot| {
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
            self.with_host_on_main_thread(move |_, _, _, slot| {
                if let Some(current) = slot.as_ref() {
                    current.host.set_visible(visible);
                }
            });
        }

        fn set_zoom(&self, zoom_percent: u32) {
            let factor = f64::from(zoom_percent.clamp(50, 250)) / 100.0;
            self.with_host_on_main_thread(move |_, _, _, slot| {
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
        loads: Mutex<Vec<(Vec<u8>, bool, DocumentMode)>>,
    }

    impl ReaderPlatformSink for RecordingSink {
        fn is_native_supported(&self) -> bool {
            true
        }

        fn load_html(
            &self,
            _: String,
            _: u32,
            _: u32,
            bytes: Vec<u8>,
            allow_remote: bool,
            mode: DocumentMode,
        ) {
            self.loads.lock().unwrap().push((bytes, allow_remote, mode));
        }

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
                thread: None,
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
                thread: None,
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
                thread: None,
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

    #[test]
    fn thread_document_renders_with_thread_container_mode_and_supports_rerender() {
        use kitemail_mail_core::ThreadMessage;

        let sink = Arc::new(RecordingSink::default());
        let mgr = ReaderManager::new(sink.clone());
        let appearance = ReaderAppearance {
            theme_id: "system".to_string(),
            dark: false,
            zoom_percent: 100,
            block_remote_images: true,
        };
        let detail = MessageDetail {
            id: 1,
            account_id: 1,
            folder_id: 1,
            folder_path: "INBOX".to_string(),
            uid: 1,
            rfc_message_id: "<m1@example.com>".to_string(),
            sender: "Alice <alice@example.com>".to_string(),
            recipients: "bob@example.com".to_string(),
            cc: String::new(),
            subject: "Re: Thread".to_string(),
            date_ts: 1_700_000_100,
            date_display: "10:05".to_string(),
            body_text: "Head".to_string(),
            body_html: Some("<p>Head <img src=\"https://tracker.example/p.png\"></p>".to_string()),
            flagged: false,
            attachments: Vec::new(),
            thread_messages: vec![
                ThreadMessage {
                    id: 1,
                    rfc_message_id: "<m1@example.com>".to_string(),
                    subject: "Re: Thread".to_string(),
                    sender: "Alice <alice@example.com>".to_string(),
                    recipients: "bob@example.com".to_string(),
                    cc: String::new(),
                    date_ts: 1_700_000_100,
                    date_display: "10:05".to_string(),
                    body_text: "Head".to_string(),
                    body_html: Some(
                        "<p>Head <img src=\"https://tracker.example/p.png\"></p>".to_string(),
                    ),
                    body_loaded: true,
                    unread: false,
                    flagged: false,
                    attachments: Vec::new(),
                    uid: 1,
                    folder_path: "INBOX".to_string(),
                },
                ThreadMessage {
                    id: 2,
                    rfc_message_id: "<m2@example.com>".to_string(),
                    subject: "Thread".to_string(),
                    sender: "Bob <bob@example.com>".to_string(),
                    recipients: "alice@example.com".to_string(),
                    cc: String::new(),
                    date_ts: 1_700_000_000,
                    date_display: "09:00".to_string(),
                    body_text: "Older".to_string(),
                    body_html: Some("<p>Older</p>".to_string()),
                    body_loaded: true,
                    unread: false,
                    flagged: false,
                    attachments: Vec::new(),
                    uid: 2,
                    folder_path: "INBOX".to_string(),
                },
            ],
            unsubscribe_url: None,
            unsubscribe_one_click: false,
            auth_results: None,
        };

        let status = mgr.begin_session(
            ReaderDocumentSpec {
                session_id: "sess-thread".to_string(),
                account_id: 1,
                message_id: 1,
                subject: detail.subject.clone(),
                sender: detail.sender.clone(),
                plain_text: detail.body_text.clone(),
                html_body: detail.body_html.clone(),
                has_thread: true,
                thread: Some(ThreadRenderOptions {
                    detail,
                    collapsed_cards: HashSet::from([2]),
                    raw_cards: HashSet::new(),
                    allowed_remote_cards: HashSet::new(),
                    allowed_remote_senders: Vec::new(),
                    ui_text_scale_percent: 100,
                    date_style: "system".to_string(),
                    clock_style: "system".to_string(),
                    always_show_recipients: false,
                    demo_mode: true,
                    read_receipt_policy: "ask".to_string(),
                    initial_anchor: None,
                }),
            },
            &appearance,
            true,
        );
        assert_eq!(status, BodyStatusDto::Loading);

        {
            let loads = sink.loads.lock().unwrap();
            assert_eq!(loads.len(), 1);
            assert_eq!(loads[0].2, DocumentMode::ThreadContainer);
            assert!(!loads[0].1);
            let html = String::from_utf8_lossy(&loads[0].0);
            assert!(html.contains("<section class=\"km-card km-current\" data-msg-id=\"1\""));
            assert!(html.contains("<section class=\"km-card km-folded\" data-msg-id=\"2\""));
            assert!(html.contains("已拦截 1 张远程图片以保护隐私"));
        }

        // Apply layout, then unfold card #2 and allow remote images on card #1.
        mgr.apply_layout(
            &SetReaderLayoutInput {
                reader_session_id: "sess-thread".to_string(),
                layout_revision: 1,
                content_rect: rect(400.0, 48.0, 600.0, 600.0),
                clip_rect: rect(400.0, 48.0, 600.0, 600.0),
                visible: true,
                device_pixel_ratio: Some(1.0),
            },
            1.0,
        )
        .unwrap();

        let rerendered = mgr.rerender_matching_document(1, 1, &appearance, |spec| {
            if let Some(ref mut thread) = spec.thread {
                thread.collapsed_cards.remove(&2);
                thread.allowed_remote_cards.insert(1);
            }
        });
        assert!(rerendered);

        let loads = sink.loads.lock().unwrap();
        assert_eq!(loads.len(), 2);
        assert_eq!(loads[1].2, DocumentMode::ThreadContainer);
        assert!(loads[1].1);
        let html2 = String::from_utf8_lossy(&loads[1].0);
        assert!(html2.contains("<section class=\"km-card\" data-msg-id=\"2\""));
        assert!(!html2.contains("<section class=\"km-card km-folded\""));
        assert!(html2.contains("https://tracker.example/p.png"));
        // Re-render also re-applied last layout so a recreated host stays visible.
        assert_eq!(sink.layouts.lock().unwrap().len(), 2);
    }
}
