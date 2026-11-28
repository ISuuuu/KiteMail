//! Tauri main-thread native reader lifecycle, layout/visibility coordinator,
//! and shared `kitemail-windows-mail-reader` WebView2 host adapter.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};

use kitemail_mail_core::config::normalize_sender_email;
use kitemail_mail_core::{MessageDetail, ThreadMessage};
use kitemail_windows_mail_reader::{
    build_body_document, build_thread_document_with_extra,
    supports_document as reader_supports_document, DocumentAppearance, DocumentMode,
    ThreadCardRenderInput, ThreadDocContext, ThreadPalette, ThreadRenderExtra, WebViewAction,
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
    pub reading_view: bool,
    pub per_message_dark: HashMap<u32, bool>,
    pub card_tags: HashMap<u32, Vec<(String, String)>>,
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
    if doc.thread.is_some() {
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
        let palette = ThreadPalette::from_appearance(&doc_appearance);
        let fallback_single: Vec<ThreadMessage>;
        let thread_msgs: &[ThreadMessage] = if thread.detail.thread_messages.is_empty() {
            fallback_single = vec![ThreadMessage {
                id: thread.detail.id,
                rfc_message_id: thread.detail.rfc_message_id.clone(),
                subject: thread.detail.subject.clone(),
                sender: thread.detail.sender.clone(),
                recipients: thread.detail.recipients.clone(),
                cc: thread.detail.cc.clone(),
                date_ts: thread.detail.date_ts,
                date_display: thread.detail.date_display.clone(),
                body_text: thread.detail.body_text.clone(),
                body_html: thread.detail.body_html.clone(),
                body_loaded: true,
                unread: false,
                flagged: thread.detail.flagged,
                attachments: thread.detail.attachments.clone(),
                uid: thread.detail.uid,
                folder_path: thread.detail.folder_path.clone(),
            }];
            &fallback_single
        } else {
            &thread.detail.thread_messages
        };
        let cards: Vec<ThreadCardRenderInput<'_>> = thread_msgs
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
                let card_dark = thread
                    .per_message_dark
                    .get(&tm.id)
                    .copied()
                    .unwrap_or(appearance.dark);
                ThreadCardRenderInput {
                    message: tm,
                    is_current,
                    is_collapsed,
                    show_raw_source,
                    block_remote,
                    card_dark,
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
        let card_tags_vec: Vec<(u32, Vec<(String, String)>)> = thread
            .card_tags
            .iter()
            .map(|(k, v)| (*k, v.clone()))
            .collect();
        let extra = ThreadRenderExtra {
            reading_view: thread.reading_view,
            card_tags: &card_tags_vec,
        };
        let (bytes, any_remote_allowed) = build_thread_document_with_extra(&ctx, &extra);
        return (bytes, any_remote_allowed, DocumentMode::ThreadContainer);
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
    Ignored {
        reason: String,
    },
    Applied {
        clip: PhysicalRect,
        bounds: PhysicalRect,
        visible: bool,
        account_id: u32,
        message_id: u32,
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
    fn apply_layout(&self, clip: PhysicalRect, bounds: PhysicalRect, visible: bool)
        -> Result<(), String>;
    fn set_visible(&self, visible: bool);
    fn set_zoom(&self, zoom_percent: u32);
    fn execute_script(&self, _script: String) {}
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

    fn apply_layout(
        &self,
        _clip: PhysicalRect,
        _bounds: PhysicalRect,
        _visible: bool,
    ) -> Result<(), String> {
        Ok(())
    }
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

    pub fn find_in_reader(
        &self,
        reader_session_id: &str,
        query: &str,
        step: Option<i32>,
    ) -> Result<bool, IpcError> {
        let (matches_active, enabled, locked) = {
            let guard = self.state.lock().expect("reader mutex poisoned");
            (
                guard.active_session_id.as_deref() == Some(reader_session_id),
                !guard.reader_hidden_pref,
                guard.locked,
            )
        };
        if locked || !enabled || !matches_active || !self.sink.is_native_supported() {
            return Ok(false);
        }
        let trimmed = query.trim();
        if trimmed.is_empty() {
            self.sink
                .execute_script("window.kmFindClear && window.kmFindClear();".to_string());
            return Ok(true);
        }
        if let Some(delta) = step.filter(|&d| d != 0) {
            let dir = if delta < 0 { -1 } else { 1 };
            self.sink
                .execute_script(format!("window.kmFindStep && window.kmFindStep({dir});"));
            return Ok(true);
        }
        let escaped = serde_json::to_string(trimmed).unwrap_or_else(|_| "\"\"".to_string());
        self.sink
            .execute_script(format!("window.kmFind && window.kmFind({escaped});"));
        Ok(true)
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
                layout_applied: None,
                layout_visible: None,
                stage: Some("readerDisabled".to_string()),
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
                layout_applied: Some(false),
                layout_visible: Some(false),
                stage: Some("loading".to_string()),
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
                layout_applied: None,
                layout_visible: None,
                stage: Some("plainTextReady".to_string()),
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
            if guard.locked {
                LayoutApplyOutcome::Ignored {
                    reason: "locked".to_string(),
                }
            } else if guard.reader_hidden_pref {
                LayoutApplyOutcome::Ignored {
                    reason: "readerDisabled".to_string(),
                }
            } else if guard.active_session_id.as_deref() != Some(input.reader_session_id.as_str()) {
                LayoutApplyOutcome::Ignored {
                    reason: "staleSession".to_string(),
                }
            } else if guard
                .last_layout_revision
                .is_some_and(|prev| input.layout_revision <= prev)
            {
                LayoutApplyOutcome::Ignored {
                    reason: "staleRevision".to_string(),
                }
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
                let (account_id, message_id) = guard
                    .active_document
                    .as_ref()
                    .map_or((0, 0), |d| (d.account_id, d.message_id));
                LayoutApplyOutcome::Applied {
                    clip,
                    bounds,
                    visible,
                    account_id,
                    message_id,
                }
            }
        };

        match decision {
            LayoutApplyOutcome::Ignored { reason } => Ok(SetReaderLayoutOutput {
                applied: false,
                reader_session_id: input.reader_session_id.clone(),
                layout_revision: input.layout_revision,
                visible: None,
                reason: Some(reason),
            }),
            LayoutApplyOutcome::Applied {
                clip,
                bounds,
                visible,
                account_id,
                message_id,
            } => {
                if let Err(err) = self.sink.apply_layout(clip, bounds, visible) {
                    self.sink.emit_status(ReaderStatusEvent {
                        reader_session_id: input.reader_session_id.clone(),
                        account_id,
                        message_id,
                        body_status: BodyStatusDto::Unavailable,
                        detail: Some(err.clone()),
                        message: Some(err.clone()),
                        retryable: Some(true),
                        plain_text: None,
                        has_html: None,
                        html_body: None,
                        layout_applied: Some(false),
                        layout_visible: Some(false),
                        stage: Some("applyLayout".to_string()),
                    });
                    return Err(IpcError::render_failed(err, None));
                }
                Ok(SetReaderLayoutOutput {
                    applied: true,
                    reader_session_id: input.reader_session_id.clone(),
                    layout_revision: input.layout_revision,
                    visible: Some(visible),
                    reason: None,
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
                    let _ = self
                        .sink
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
                let _ = self
                    .sink
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
        navigation_ready: bool,
        layout_applied: bool,
        layout_visible: bool,
        last_error: Option<String>,
    }

    struct HostSlot {
        host: WebView2Host,
        meta: Rc<RefCell<ActiveSessionMeta>>,
    }

    thread_local! {
        static MAIN_THREAD_HOST: RefCell<Option<HostSlot>> = const { RefCell::new(None) };
    }

    fn emit_reader_status(app_handle: &tauri::AppHandle, event: ReaderStatusEvent) {
        let _ = app_handle.emit_to(MAIN_WINDOW_LABEL, EVENT_READER_STATUS, event);
    }

    fn emit_unavailable_status(
        app_handle: &tauri::AppHandle,
        session_id: String,
        account_id: u32,
        message_id: u32,
        stage: &str,
        detail: String,
    ) {
        if session_id.is_empty() {
            return;
        }
        emit_reader_status(
            app_handle,
            ReaderStatusEvent {
                reader_session_id: session_id,
                account_id,
                message_id,
                body_status: BodyStatusDto::Unavailable,
                detail: Some(detail.clone()),
                message: Some(detail),
                retryable: Some(true),
                plain_text: None,
                has_html: None,
                html_body: None,
                layout_applied: Some(false),
                layout_visible: Some(false),
                stage: Some(stage.to_string()),
            },
        );
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
    ) -> Result<HostSlot, String> {
        let win = tauri::Manager::get_webview_window(app_handle, MAIN_WINDOW_LABEL)
            .ok_or_else(|| "未找到主窗口，无法创建原生阅读器宿主".to_string())?;
        let raw = HasWindowHandle::window_handle(&win)
            .map_err(|e| format!("获取主窗口原生句柄失败: {e}"))?;
        let RawWindowHandle::Win32(win32) = raw.as_raw() else {
            return Err("主窗口不是有效的 Win32 HWND".to_string());
        };
        let host = WebView2Host::from_raw_hwnd(win32.hwnd.get(), user_data_dir.clone())
            .map_err(|e| format!("创建 WebView2 阅读器宿主失败: {e}"))?;
        let meta = Rc::new(RefCell::new(ActiveSessionMeta::default()));
        let cb_meta = meta.clone();
        let cb_handle = app_handle.clone();
        let cb_action_handler = Arc::clone(action_handler);
        host.set_event_callback(move |event| match event {
            ReaderEvent::StatusChanged { version, status } => {
                let (
                    session_id,
                    account_id,
                    message_id,
                    layout_applied,
                    layout_visible,
                    last_error,
                ) = {
                    let mut active = cb_meta.borrow_mut();
                    if version != active.expected_version || active.session_id.is_empty() {
                        return;
                    }
                    match status {
                        ReaderLoadStatus::Loading => {
                            active.navigation_ready = false;
                            active.layout_applied = false;
                            active.layout_visible = false;
                            active.last_error = None;
                        }
                        ReaderLoadStatus::Ready => {
                            active.navigation_ready = true;
                            active.last_error = None;
                        }
                        ReaderLoadStatus::Unavailable | ReaderLoadStatus::ProcessFailed => {
                            active.navigation_ready = false;
                            active.layout_applied = false;
                            active.layout_visible = false;
                        }
                    }
                    (
                        active.session_id.clone(),
                        active.account_id,
                        active.message_id,
                        active.layout_applied,
                        active.layout_visible,
                        active.last_error.clone(),
                    )
                };
                let (body_status, detail, retryable, stage) = match status {
                    ReaderLoadStatus::Loading => (
                        BodyStatusDto::Loading,
                        None,
                        None,
                        Some("navigationLoading".to_string()),
                    ),
                    ReaderLoadStatus::Ready => (
                        BodyStatusDto::Ready,
                        None,
                        None,
                        Some("navigationReady".to_string()),
                    ),
                    ReaderLoadStatus::Unavailable => (
                        BodyStatusDto::Unavailable,
                        Some(last_error.unwrap_or_else(|| "邮件正文加载失败".to_string())),
                        Some(true),
                        Some("navigationUnavailable".to_string()),
                    ),
                    ReaderLoadStatus::ProcessFailed => (
                        BodyStatusDto::Unavailable,
                        Some("WebView2 阅读进程异常退出".to_string()),
                        Some(true),
                        Some("processFailed".to_string()),
                    ),
                };
                emit_reader_status(
                    &cb_handle,
                    ReaderStatusEvent {
                        reader_session_id: session_id,
                        account_id,
                        message_id,
                        body_status,
                        detail: detail.clone(),
                        message: detail,
                        retryable,
                        plain_text: None,
                        has_html: None,
                        html_body: None,
                        layout_applied: Some(layout_applied),
                        layout_visible: Some(layout_visible),
                        stage,
                    },
                );
            }
            ReaderEvent::LayoutApplied {
                version,
                visible,
                controller_ready,
            } => {
                let payload = {
                    let mut active = cb_meta.borrow_mut();
                    if version != active.expected_version || active.session_id.is_empty() {
                        None
                    } else {
                        active.layout_applied = controller_ready;
                        active.layout_visible = visible;
                        let body_status = if active.navigation_ready {
                            BodyStatusDto::Ready
                        } else {
                            BodyStatusDto::Loading
                        };
                        let stage = if controller_ready && visible {
                            "layoutVisible"
                        } else if controller_ready {
                            "layoutHidden"
                        } else {
                            "layoutPendingController"
                        };
                        Some(ReaderStatusEvent {
                            reader_session_id: active.session_id.clone(),
                            account_id: active.account_id,
                            message_id: active.message_id,
                            body_status,
                            detail: None,
                            message: None,
                            retryable: None,
                            plain_text: None,
                            has_html: None,
                            html_body: None,
                            layout_applied: Some(controller_ready),
                            layout_visible: Some(visible),
                            stage: Some(stage.to_string()),
                        })
                    }
                };
                if let Some(ev) = payload {
                    emit_reader_status(&cb_handle, ev);
                }
            }
            ReaderEvent::Error {
                version,
                stage,
                detail,
            } => {
                let target = {
                    let mut active = cb_meta.borrow_mut();
                    if version != active.expected_version || active.session_id.is_empty() {
                        None
                    } else {
                        let msg = format!("{stage}: {detail}");
                        active.last_error = Some(msg.clone());
                        active.layout_applied = false;
                        active.layout_visible = false;
                        Some((active.session_id.clone(), active.account_id, active.message_id, msg))
                    }
                };
                if let Some((session_id, account_id, message_id, msg)) = target {
                    emit_unavailable_status(
                        &cb_handle,
                        session_id,
                        account_id,
                        message_id,
                        stage,
                        msg,
                    );
                }
            }
            ReaderEvent::ProcessFailed { version } => {
                let active = cb_meta.borrow();
                if version != active.expected_version || active.session_id.is_empty() {
                    return;
                }
                let detail = Some("WebView2 阅读进程异常退出".to_string());
                emit_reader_status(
                    &cb_handle,
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
                        layout_applied: Some(false),
                        layout_visible: Some(false),
                        stage: Some("processFailed".to_string()),
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
        Ok(HostSlot { host, meta })
    }

    pub struct TauriWindowsReaderSink {
        app_handle: tauri::AppHandle,
        user_data_dir: PathBuf,
        action_handler: SharedActionHandler,
        active_meta: Arc<Mutex<(String, u32, u32)>>,
    }

    impl TauriWindowsReaderSink {
        pub fn new(app_handle: tauri::AppHandle, user_data_dir: PathBuf) -> Self {
            Self {
                app_handle,
                user_data_dir,
                action_handler: Arc::new(Mutex::new(None)),
                active_meta: Arc::new(Mutex::new((String::new(), 0, 0))),
            }
        }

        fn with_host_on_main_thread<F>(&self, f: F) -> Result<(), String>
        where
            F: FnOnce(
                    &tauri::AppHandle,
                    &PathBuf,
                    &SharedActionHandler,
                    &mut Option<HostSlot>,
                    Option<String>,
                ) + Send
                + 'static,
        {
            let handle = self.app_handle.clone();
            let user_data_dir = self.user_data_dir.clone();
            let action_handler = Arc::clone(&self.action_handler);
            self.app_handle
                .run_on_main_thread(move || {
                    MAIN_THREAD_HOST.with(|slot| {
                        let mut borrow = slot.borrow_mut();
                        let create_err = if borrow.is_none() {
                            match create_host_slot(&handle, &user_data_dir, &action_handler) {
                                Ok(new_slot) => {
                                    *borrow = Some(new_slot);
                                    None
                                }
                                Err(err) => {
                                    eprintln!("[reader] {err}");
                                    Some(err)
                                }
                            }
                        } else {
                            None
                        };
                        f(&handle, &user_data_dir, &action_handler, &mut borrow, create_err);
                    });
                })
                .map_err(|e| format!("投递主线程阅读器任务失败: {e}"))
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
            if let Ok(mut meta) = self.active_meta.lock() {
                *meta = (session_id.clone(), account_id, message_id);
            }
            let session_for_err = session_id.clone();
            if let Err(err) = self.with_host_on_main_thread(
                move |handle, user_data_dir, action_handler, slot, create_err| {
                    if let Some(current) = slot.as_ref() {
                        if current.host.retire_for_remote_reload(allow_remote_images) {
                            current.host.close();
                            match create_host_slot(handle, user_data_dir, action_handler) {
                                Ok(new_slot) => *slot = Some(new_slot),
                                Err(err) => {
                                    eprintln!("[reader] recreate host failed: {err}");
                                    *slot = None;
                                    emit_unavailable_status(
                                        handle,
                                        session_id,
                                        account_id,
                                        message_id,
                                        "recreateHost",
                                        err,
                                    );
                                    return;
                                }
                            }
                        }
                    }
                    let Some(current) = slot.as_ref() else {
                        emit_unavailable_status(
                            handle,
                            session_id,
                            account_id,
                            message_id,
                            "createHost",
                            create_err.unwrap_or_else(|| "原生阅读器宿主不可用".to_string()),
                        );
                        return;
                    };
                    {
                        let mut meta = current.meta.borrow_mut();
                        meta.expected_version = current.host.version().wrapping_add(1);
                        meta.session_id = session_id.clone();
                        meta.account_id = account_id;
                        meta.message_id = message_id;
                        meta.navigation_ready = false;
                        meta.layout_applied = false;
                        meta.layout_visible = false;
                        meta.last_error = None;
                    }
                    match current
                        .host
                        .load_document(html_bytes, allow_remote_images, mode)
                    {
                        Ok(version) => {
                            current.meta.borrow_mut().expected_version = version;
                        }
                        Err(err) => {
                            let detail = format!("加载邮件文档到 WebView2 失败: {err}");
                            eprintln!("[reader] {detail}");
                            emit_unavailable_status(
                                handle,
                                session_id,
                                account_id,
                                message_id,
                                "loadDocument",
                                detail,
                            );
                        }
                    }
                },
            ) {
                eprintln!("[reader] {err}");
                emit_unavailable_status(
                    &self.app_handle,
                    session_for_err,
                    account_id,
                    message_id,
                    "dispatchLoadHtml",
                    err,
                );
            }
        }

        fn apply_layout(
            &self,
            clip: PhysicalRect,
            bounds: PhysicalRect,
            visible: bool,
        ) -> Result<(), String> {
            let active_meta = Arc::clone(&self.active_meta);
            self.with_host_on_main_thread(move |handle, _, _, slot, create_err| {
                let Some(current) = slot.as_ref() else {
                    if let Ok((sid, acc, msg)) = active_meta.lock().map(|g| g.clone()) {
                        emit_unavailable_status(
                            handle,
                            sid,
                            acc,
                            msg,
                            "createHost",
                            create_err.unwrap_or_else(|| "原生阅读器宿主不可用".to_string()),
                        );
                    }
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
                if let Err(err) = current.host.try_set_bounds(content_px, clip_px, visible) {
                    eprintln!("[reader] apply_layout failed: {err}");
                }
            })
        }

        fn set_visible(&self, visible: bool) {
            let _ = self.with_host_on_main_thread(move |_, _, _, slot, _| {
                if let Some(current) = slot.as_ref() {
                    if let Err(err) = current.host.try_set_visible(visible) {
                        eprintln!("[reader] set_visible failed: {err}");
                    }
                }
            });
        }

        fn set_zoom(&self, zoom_percent: u32) {
            let factor = f64::from(zoom_percent.clamp(50, 250)) / 100.0;
            let _ = self.with_host_on_main_thread(move |_, _, _, slot, _| {
                if let Some(current) = slot.as_ref() {
                    current.host.set_zoom(factor);
                }
            });
        }

        fn execute_script(&self, script: String) {
            let _ = self.with_host_on_main_thread(move |_, _, _, slot, _| {
                if let Some(current) = slot.as_ref() {
                    current.host.execute_script(&script);
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
        apply_layout_error: Mutex<Option<String>>,
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

        fn apply_layout(
            &self,
            clip: PhysicalRect,
            bounds: PhysicalRect,
            visible: bool,
        ) -> Result<(), String> {
            if let Some(err) = self.apply_layout_error.lock().unwrap().clone() {
                return Err(err);
            }
            self.layouts.lock().unwrap().push((clip, bounds, visible));
            Ok(())
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
                    reading_view: false,
                    per_message_dark: HashMap::new(),
                    card_tags: HashMap::new(),
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

    #[test]
    fn apply_layout_failure_emits_unavailable_status_and_returns_error() {
        let sink = Arc::new(RecordingSink::default());
        *sink.apply_layout_error.lock().unwrap() = Some("SetWindowPos 失败".to_string());
        let mgr = ReaderManager::new(sink.clone());
        let appearance = ReaderAppearance {
            theme_id: "system".to_string(),
            dark: false,
            zoom_percent: 100,
            block_remote_images: true,
        };
        mgr.begin_session(
            ReaderDocumentSpec {
                session_id: "sess-err".to_string(),
                account_id: 7,
                message_id: 42,
                subject: "HTML".to_string(),
                sender: "a@example.com".to_string(),
                plain_text: "body".to_string(),
                html_body: Some("<p>Hi</p>".to_string()),
                has_thread: false,
                thread: None,
            },
            &appearance,
            true,
        );

        let err = mgr
            .apply_layout(
                &SetReaderLayoutInput {
                    reader_session_id: "sess-err".to_string(),
                    layout_revision: 1,
                    content_rect: rect(400.0, 48.0, 600.0, 600.0),
                    clip_rect: rect(400.0, 48.0, 600.0, 600.0),
                    visible: true,
                    device_pixel_ratio: Some(1.0),
                },
                1.0,
            )
            .unwrap_err();
        assert_eq!(err.code, "RENDER_FAILED");
        assert!(err.message.contains("SetWindowPos"));

        let events = sink.events.lock().unwrap();
        let last = events.last().expect("unavailable event emitted");
        assert_eq!(last.body_status, BodyStatusDto::Unavailable);
        assert_eq!(last.account_id, 7);
        assert_eq!(last.message_id, 42);
        assert_eq!(last.layout_applied, Some(false));
        assert_eq!(last.stage.as_deref(), Some("applyLayout"));
    }
}
