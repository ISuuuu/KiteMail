//! 异步 WebView2 宿主。调用方必须在创建父窗口的 STA 线程使用。
#![allow(dead_code)]

use futures::channel::mpsc as futures_mpsc;
use gpui_kit::{
    point, px, AsyncWindowContext, Hsla, PlatformInput, Rgba, ScrollDelta, ScrollEvent, TouchPhase,
    Window,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{
    cell::RefCell,
    os::windows::ffi::OsStrExt,
    rc::{Rc, Weak},
};
use webview2_com::{
    CapturePreviewCompletedHandler, CreateCoreWebView2ControllerCompletedHandler,
    CreateCoreWebView2EnvironmentCompletedHandler, DOMContentLoadedEventHandler,
    LaunchingExternalUriSchemeEventHandler, Microsoft::Web::WebView2::Win32::*,
    NavigationCompletedEventHandler, NavigationStartingEventHandler,
    NewWindowRequestedEventHandler, ProcessFailedEventHandler, WebMessageReceivedEventHandler,
    WebResourceRequestedEventHandler,
};
use windows::{
    core::{w, Interface},
    Win32::{
        Foundation::{HWND, POINT, RECT},
        Graphics::Gdi::{
            BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC,
            GetMonitorInfoW, MonitorFromRect, ReleaseDC, SelectObject, BITMAPINFO,
            BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, MONITORINFO, MONITOR_DEFAULTTONULL, SRCCOPY,
        },
        System::Com::{IStream, STREAM_SEEK_SET},
        UI::{
            Shell::SHCreateMemStream,
            WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, GetAncestor, GetCursorPos, GetForegroundWindow,
                GetParent, GetWindowInfo, GetWindowRect, SetWindowPos, ShowWindow, GA_ROOT,
                SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE, SW_SHOWNOACTIVATE, WINDOWINFO,
                WINDOW_EX_STYLE, WS_CHILD, WS_CLIPCHILDREN,
            },
        },
    },
};

const DOCUMENT_URL: &str = "https://kitemail.invalid/reader";

#[path = "webview_dark.rs"]
mod dark;

fn document_url(version: u64) -> String {
    format!("{DOCUMENT_URL}/{version}")
}

fn allow_main_navigation(url: &str, version: u64) -> bool {
    url == document_url(version)
}

fn allow_frame_navigation(url: &str) -> bool {
    matches!(url, "about:srcdoc" | "about:blank")
}

fn parse_external_url(value: &str) -> Option<url::Url> {
    let url = url::Url::parse(value).ok()?;
    let allowed = match url.scheme() {
        "http" | "https" => {
            url.host_str().is_some() && url.username().is_empty() && url.password().is_none()
        }
        "mailto" => !url.path().is_empty(),
        _ => false,
    };
    allowed.then_some(url)
}

pub(crate) fn allowed_external_url(value: &str) -> bool {
    parse_external_url(value).is_some()
}

fn allowed_mail_image_url(value: &str, allow_remote: bool) -> bool {
    allow_remote
        && url::Url::parse(value).is_ok_and(|url| {
            matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
        })
}

/// Apply the same URL policy to native mail images after HTML parsing.
pub(crate) fn mail_image_source(
    uri: &gpui_kit::SharedUri,
    allow_remote: bool,
) -> gpui_kit::ImageSource {
    if let Some((format, bytes)) = decode_inline_mail_image(uri.as_ref()) {
        return gpui_kit::ImageSource::Image(std::sync::Arc::new(gpui_kit::Image::from_bytes(
            format, bytes,
        )));
    }
    if allowed_mail_image_url(uri.as_ref(), allow_remote) {
        uri.clone().into()
    } else {
        gpui_kit::ImageSource::Custom(std::sync::Arc::new(|_, _| None))
    }
}

pub(crate) fn decode_inline_mail_image(value: &str) -> Option<(gpui_kit::ImageFormat, Vec<u8>)> {
    use gpui_kit::ImageFormat;

    let data = data_url::DataUrl::process(value).ok()?;
    let mime = data.mime_type();
    if mime.type_ != "image" {
        return None;
    }
    let format = match mime.subtype.as_str() {
        "png" => ImageFormat::Png,
        "jpeg" => ImageFormat::Jpeg,
        "gif" => ImageFormat::Gif,
        "webp" => ImageFormat::Webp,
        "bmp" => ImageFormat::Bmp,
        _ => return None,
    };
    let (bytes, _) = data.decode_to_vec().ok()?;
    Some((format, bytes))
}

pub(crate) fn prepared_mail_link(value: &str, clean: bool) -> Option<String> {
    let url = parse_external_url(value)?;
    let url = if clean {
        parse_external_url(&kitemail_mail_core::link_clean::clean_link(url.as_str()))?
    } else {
        url
    };
    Some(url.into())
}

fn document_headers(allow_remote_images: bool) -> String {
    let image_sources = if allow_remote_images {
        "data: http: https:"
    } else {
        "data:"
    };
    format!("Content-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; img-src {image_sources}; style-src 'unsafe-inline'; frame-src 'self'; base-uri 'none'; form-action 'none'\r\n")
}

fn allow_remote_request(
    url: &str,
    context: COREWEBVIEW2_WEB_RESOURCE_CONTEXT,
    allowed: bool,
    referer: &str,
    version: u64,
) -> bool {
    context == COREWEBVIEW2_WEB_RESOURCE_CONTEXT_IMAGE
        && (referer.is_empty() || referer == document_url(version))
        && allowed_mail_image_url(url, allowed)
}

fn clipped_rects(content: RECT, viewport: RECT) -> Option<(RECT, RECT)> {
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

pub(crate) fn supports_document(html: &str) -> bool {
    !html.trim().is_empty()
}

fn find_html_tag(lower: &str, tag: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(at) = lower[from..].find(tag).map(|i| from + i) {
        let next = lower
            .as_bytes()
            .get(at + tag.len())
            .copied()
            .unwrap_or(b'>');
        if next == b'>' || next == b'/' || next.is_ascii_whitespace() {
            return Some(at);
        }
        from = at + tag.len();
    }
    None
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct DocumentAppearance {
    pub dark: bool,
    pub zoom_percent: u32,
    pub background: Hsla,
    pub foreground: Hsla,
}

pub(crate) fn build_body_document(html: &str, appearance: DocumentAppearance) -> Vec<u8> {
    make_document(html, None, appearance, false)
}

pub(crate) fn build_thread_body_document(html: &str, appearance: DocumentAppearance) -> Vec<u8> {
    make_document(html, None, appearance, true)
}

pub(crate) fn build_document(
    html: &str,
    subject: &str,
    sender: &str,
    appearance: DocumentAppearance,
) -> Vec<u8> {
    make_document(html, Some((subject, sender)), appearance, false)
}

fn make_document(
    html: &str,
    header: Option<(&str, &str)>,
    appearance: DocumentAppearance,
    in_thread: bool,
) -> Vec<u8> {
    let escape = |value: &str| {
        value
            .replace('&', "&amp;")
            .replace('"', "&quot;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    let html = if appearance.dark {
        dark::adapt_colors_for_dark(html)
    } else {
        html.to_string()
    };
    let html = dark::pin_color_scheme(&html, appearance.dark);
    let css_color = |color: Hsla| {
        let rgb = Rgba::from(color);
        let [r, g, b] = [rgb.r, rgb.g, rgb.b].map(|channel| (channel * 255.0).round() as u8);
        format!("#{r:02x}{g:02x}{b:02x}")
    };
    let bg = css_color(appearance.background);
    let fg = css_color(appearance.foreground);
    let scheme = if appearance.dark { "dark" } else { "light" };
    let thread_css = if in_thread {
        "html{display:flow-root!important;height:auto!important;min-height:0!important}body{height:auto!important;min-height:0!important}"
    } else {
        ""
    };
    let style = format!(
        "<style>:root{{color-scheme:{scheme};zoom:{}%}}{thread_css}:where(body){{background-color:{bg};color:{fg}}}</style>",
        appearance.zoom_percent.clamp(50, 250)
    );
    let lower = html.to_ascii_lowercase();
    let insertion = find_html_tag(&lower, "<head")
        .or_else(|| lower.find("<!doctype"))
        .and_then(|start| lower[start..].find('>').map(|end| start + end + 1))
        .unwrap_or(0);
    let html = format!("{}{}{}", &html[..insertion], style, &html[insertion..]);
    let escaped = escape(&html);
    let header = header
        .map(|(subject, sender)| {
            format!(
                "<header><h1>{}</h1><div>{}</div></header>",
                escape(subject),
                escape(sender)
            )
        })
        .unwrap_or_default();
    format!("<!doctype html><meta charset=utf-8><style>:root{{color-scheme:{scheme}}}html,body{{height:100%;margin:0}}body{{display:flex;flex-direction:column;font:14px system-ui;background:{bg};color:{fg}}}header{{padding:24px 24px 12px}}h1{{font-size:20px;margin:0 0 12px}}iframe{{flex:1;width:100%;border:0;min-height:0}}</style>{header}<iframe sandbox=\"allow-same-origin allow-popups\" srcdoc=\"{escaped}\"></iframe>").into_bytes()
}

#[derive(Clone)]
pub(crate) struct PopupSnapshot {
    pub(crate) image: std::sync::Arc<gpui_kit::RenderImage>,
    /// 非 `None` 时表示仅截取了裁剪后的 `clip_window` 区域，记录其在内容区内的物理偏移与尺寸
    /// `(offset_x, offset_y, width, height)`；`None` 表示与内容区域 1:1 铺满。
    pub(crate) clipped_rect_px: Option<(i32, i32, i32, i32)>,
}

#[derive(Default)]
struct State {
    controller: Option<ICoreWebView2Controller>,
    clip_window: HWND,
    document: Vec<u8>,
    allow_remote_images: bool,
    version: u64,
    navigation_id: Option<u64>,
    loaded: bool,
    links: Vec<(u64, String)>,
    scroll_window: Option<AsyncWindowContext>,
    svg_renderer: Option<gpui_kit::SvgRenderer>,
    bridge_script_version: u64,
    notifier: Option<futures_mpsc::Sender<()>>,
    in_thread: bool,
    clip_rect: RECT,
    bounds: RECT,
    visible: bool,
    has_area: bool,
    window_shown: bool,
    failed: bool,
    closed: bool,
    popup_open: bool,
    capturing: bool,
    capture_seq: u64,
    snapshot: Option<(u64, PopupSnapshot)>,
    content_height: Option<f32>,
    height_updates: u8,
}

/// 将 top-down 32 位 BGRA 像素打包成 54 字节未压缩 BMP，供同步生成 `RenderImage`。
fn bmp_from_top_down_bgra(width: i32, height: i32, bgra: &[u8]) -> Option<Vec<u8>> {
    if width <= 0 || height <= 0 {
        return None;
    }
    let expected = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)?;
    if bgra.len() != expected {
        return None;
    }
    let total_size = 54u32.checked_add(u32::try_from(expected).ok()?)?;
    let mut bmp = Vec::with_capacity(total_size as usize);
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&total_size.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&54u32.to_le_bytes());
    bmp.extend_from_slice(&40u32.to_le_bytes());
    bmp.extend_from_slice(&width.to_le_bytes());
    bmp.extend_from_slice(&(-height).to_le_bytes());
    bmp.extend_from_slice(&1u16.to_le_bytes());
    bmp.extend_from_slice(&32u16.to_le_bytes());
    bmp.extend_from_slice(&0u32.to_le_bytes());
    bmp.extend_from_slice(&(expected as u32).to_le_bytes());
    bmp.extend_from_slice(&[0u8; 16]);
    bmp.extend_from_slice(bgra);
    Some(bmp)
}

/// 读取内存流的全部字节。
fn read_stream(stream: &IStream) -> Option<Vec<u8>> {
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

/// 快照拍摄结束（成功或失败）：记下已解码的 RenderImage 并唤醒 UI 换帧。
fn finish_capture(
    state: &Rc<RefCell<State>>,
    version: u64,
    seq: u64,
    image: Option<std::sync::Arc<gpui_kit::RenderImage>>,
) {
    let mut inner = state.borrow_mut();
    if inner.capture_seq != seq {
        return;
    }
    inner.capturing = false;
    if let Some(image) =
        image.filter(|_| inner.popup_open && inner.version == version && !inner.closed)
    {
        inner.snapshot = Some((
            version,
            PopupSnapshot {
                image,
                clipped_rect_px: None,
            },
        ));
    }
    inner.notify_ui();
}

impl State {
    fn notify_ui(&mut self) {
        if let Some(tx) = &mut self.notifier {
            let _ = tx.try_send(());
        }
    }
}

fn open_link_message(message: &str) -> Option<&str> {
    let url = message.strip_prefix("open-link:")?;
    let parsed = parse_external_url(url)?;
    (parsed.scheme() == "mailto").then_some(url)
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
        state.links.push((version, url.to_string()));
        state.notify_ui();
    }
    true
}

fn record_script_link(state: &Rc<RefCell<State>>, source: &str, url: &str) -> bool {
    let mut inner = state.borrow_mut();
    if !allow_main_navigation(source, inner.version) {
        return false;
    }
    let is_mailto = parse_external_url(url).is_some_and(|parsed| parsed.scheme() == "mailto");
    is_mailto && record_external_link(&mut inner, url)
}

fn apply_dom_content_loaded(state: &mut State, navigation_id: u64) -> bool {
    if state.navigation_id == Some(navigation_id) && !state.closed {
        state.loaded = true;
        state.failed = false;
        state.notify_ui();
        true
    } else {
        false
    }
}

fn apply_navigation_completed(state: &mut State, navigation_id: u64, success: bool) -> bool {
    if state.navigation_id == Some(navigation_id) {
        state.loaded = success;
        state.failed = !success;
        state.notify_ui();
        success
    } else {
        false
    }
}

fn maybe_install_bridge_script(state: &Rc<RefCell<State>>, view: Option<&ICoreWebView2>) {
    let forward_wheel = {
        let mut inner = state.borrow_mut();
        let version = inner.version;
        if inner.loaded && inner.bridge_script_version != version {
            inner.bridge_script_version = version;
            Some(inner.in_thread && inner.scroll_window.is_some())
        } else {
            None
        }
    };
    if let (Some(view), Some(forward_wheel)) = (view, forward_wheel) {
        // Run the trusted listener in the wrapper. The mail frame stays scriptless.
        let script = format!(
            "window.__kitemailForwardWheel = {forward_wheel};\n{}",
            include_str!("thread_scroll.js")
        );
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
    static ENVIRONMENT: RefCell<SharedEnvironment> = RefCell::new(SharedEnvironment::default());
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
            let Some(controller) = controller.filter(|_| result.is_ok()) else {
                let mut inner = state.borrow_mut();
                inner.failed = true;
                inner.notify_ui();
                return Ok(());
            };
            if state.borrow().closed {
                unsafe {
                    let _ = controller.Close();
                }
                return Ok(());
            }
            if unsafe { configure(&env, &controller, Rc::downgrade(&state)) }.is_err() {
                unsafe {
                    let _ = controller.Close();
                }
                let mut inner = state.borrow_mut();
                inner.failed = true;
                inner.notify_ui();
                return Ok(());
            }
            let mut inner = state.borrow_mut();
            let bounds = inner.bounds;
            let visible = inner.visible && inner.has_area;
            let navigate = !inner.document.is_empty();
            let url = document_url(inner.version);
            inner.controller = Some(controller.clone());
            drop(inner);
            unsafe {
                let _ = controller.SetBounds(bounds);
                let _ = controller.SetIsVisible(visible);
            }
            if navigate && unsafe { navigate_to(&controller, &url) }.is_err() {
                let mut inner = state.borrow_mut();
                inner.failed = true;
                inner.notify_ui();
            }
            Ok(())
        },
    ));
    if unsafe { environment.CreateCoreWebView2Controller(parent, &handler) }.is_err() {
        let mut inner = state.borrow_mut();
        inner.failed = true;
        inner.notify_ui();
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum BodyView {
    #[default]
    Native,
    Loading,
    Html,
}

impl BodyView {
    pub(crate) fn for_document(
        enabled: bool,
        load_scheduled: bool,
        host: Option<&WebViewHost>,
    ) -> Self {
        match (enabled, load_scheduled) {
            (false, _) => Self::Native,
            (true, true) => Self::Loading,
            (true, false) => host.map_or(Self::Native, |host| host.load_status().1),
        }
    }
}

/// 创建不会阻塞或泵消息；异步完成后可通过 `load_status` 检查。
pub(crate) struct WebViewHost(Rc<RefCell<State>>);

impl WebViewHost {
    pub(crate) fn for_window(
        window: &Window,
        scroll_window: Option<AsyncWindowContext>,
    ) -> Option<Self> {
        let RawWindowHandle::Win32(handle) = HasWindowHandle::window_handle(window).ok()?.as_raw()
        else {
            return None;
        };
        let hwnd = HWND(handle.hwnd.get() as *mut _);
        Self::create(hwnd, scroll_window).ok()
    }

    fn create(
        parent: HWND,
        scroll_window: Option<AsyncWindowContext>,
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
                Some(parent),
                None,
                None,
                None,
            )?
        };
        let state = Rc::new(RefCell::new(State {
            clip_window,
            scroll_window,
            ..State::default()
        }));
        let weak = Rc::downgrade(&state);
        let (environment, start) = ENVIRONMENT.with(|shared| {
            let mut shared = shared.borrow_mut();
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
        let callback = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
            move |result, environment| {
                let environment = environment.filter(|_| result.is_ok());
                let waiters = ENVIRONMENT.with(|shared| {
                    let mut shared = shared.borrow_mut();
                    shared.creating = false;
                    shared.environment = environment.clone();
                    std::mem::take(&mut shared.waiters)
                });
                for (parent, weak) in waiters {
                    if let Some(environment) = &environment {
                        create_controller(environment, parent, weak);
                    } else if let Some(state) = weak.upgrade() {
                        let mut inner = state.borrow_mut();
                        inner.failed = true;
                        inner.notify_ui();
                    }
                }
                Ok(())
            },
        ));
        let data_dir = Some(kitemail_mail_core::config::data_dir().join("WebView2"));
        let data_dir: Vec<u16> = data_dir
            .as_deref()
            .map(|path| path.as_os_str().encode_wide().chain(Some(0)).collect())
            .unwrap_or_default();
        let data_dir = if data_dir.is_empty() {
            windows::core::PCWSTR::null()
        } else {
            windows::core::PCWSTR(data_dir.as_ptr())
        };
        let result = unsafe {
            CreateCoreWebView2EnvironmentWithOptions(
                windows::core::PCWSTR::null(),
                data_dir,
                None::<&ICoreWebView2EnvironmentOptions>,
                &callback,
            )
        };
        if let Err(error) = result {
            let waiters = ENVIRONMENT.with(|shared| {
                let mut shared = shared.borrow_mut();
                shared.creating = false;
                std::mem::take(&mut shared.waiters)
            });
            for (_, weak) in waiters {
                if let Some(state) = weak.upgrade() {
                    let mut inner = state.borrow_mut();
                    inner.failed = true;
                    inner.notify_ui();
                }
            }
            unsafe {
                let _ = DestroyWindow(clip_window);
            }
            return Err(error);
        }
        Ok(Self(state))
    }

    pub(crate) fn set_notifier(&self, tx: futures_mpsc::Sender<()>) {
        self.0.borrow_mut().notifier = Some(tx);
    }

    pub(crate) fn set_svg_renderer(&self, renderer: gpui_kit::SvgRenderer) {
        self.0.borrow_mut().svg_renderer = Some(renderer);
    }

    pub(crate) fn load_status(&self) -> (u64, BodyView) {
        let state = self.0.borrow();
        let view = if state.failed {
            BodyView::Native
        } else if state.loaded {
            BodyView::Html
        } else {
            BodyView::Loading
        };
        (state.version, view)
    }
    pub(crate) fn content_height(&self) -> Option<f32> {
        let state = self.0.borrow();
        state.in_thread.then_some(state.content_height).flatten()
    }
    pub(crate) fn take_links(&self) -> Vec<String> {
        let mut state = self.0.borrow_mut();
        let version = state.version;
        std::mem::take(&mut state.links)
            .into_iter()
            .filter_map(|(v, url)| (v == version).then_some(url))
            .collect()
    }
    pub(crate) fn set_bounds(&self, bounds: RECT) {
        self.set_clipped_bounds(bounds, bounds);
    }
    pub(crate) fn set_clipped_bounds(&self, content: RECT, viewport: RECT) {
        let result = clipped_rects(content, viewport);
        let has_area = result.is_some();
        let (clip, bounds) = result.unwrap_or_default();
        let mut state = self.0.borrow_mut();
        let prev_has_area = state.has_area;
        let clip_changed = has_area && (!prev_has_area || state.clip_rect != clip);
        let bounds_changed = state.bounds != bounds || (has_area && !prev_has_area);
        state.clip_rect = clip;
        state.bounds = bounds;
        state.has_area = has_area;
        let controller = state.controller.clone();
        let clip_window = state.clip_window;
        let visible = state.visible && has_area;
        let visibility_changed = state.window_shown != visible;
        state.window_shown = visible;
        drop(state);
        unsafe {
            if clip_changed {
                let _ = SetWindowPos(
                    clip_window,
                    None,
                    clip.left,
                    clip.top,
                    clip.right - clip.left,
                    clip.bottom - clip.top,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                );
            }
            if visibility_changed {
                let _ = ShowWindow(
                    clip_window,
                    if visible { SW_SHOWNOACTIVATE } else { SW_HIDE },
                );
            }
        }
        if let Some(controller) = controller {
            unsafe {
                if bounds_changed {
                    let _ = controller.SetBounds(bounds);
                }
                if visibility_changed {
                    let _ = controller.SetIsVisible(visible);
                }
            }
        }
    }
    pub(crate) fn set_visible(&self, visible: bool) {
        let mut state = self.0.borrow_mut();
        state.visible = visible;
        let controller = state.controller.clone();
        let clip_window = state.clip_window;
        let visible = visible && state.has_area;
        let visibility_changed = state.window_shown != visible;
        state.window_shown = visible;
        drop(state);
        if !visibility_changed {
            return;
        }
        unsafe {
            let _ = ShowWindow(
                clip_window,
                if visible { SW_SHOWNOACTIVATE } else { SW_HIDE },
            );
        }
        if let Some(controller) = controller {
            unsafe {
                let _ = controller.SetIsVisible(visible);
            }
        }
    }
    /// 窗口内浮层（菜单 / 弹层）开关。
    ///
    /// 浮层画在窗口自己的绘制层里，会被正文的 WebView2 子窗口盖住。打开时优先在点击
    /// 当下同步截取 `clip_window` 屏幕像素，在第一帧渲染前就就绪快照，避免异步
    /// `CapturePreview` 等待期内浮层被 WebView2 遮挡以及随后的切图闪烁；不满足
    /// 同步截屏条件时再回退到异步 `CapturePreview`。关闭时按需立即放回子窗口。
    pub(crate) fn set_popup_open(&self, open: bool, restore_visible: bool) {
        {
            let mut state = self.0.borrow_mut();
            state.popup_open = open;
            state.capture_seq = state.capture_seq.wrapping_add(1);
            state.capturing = false;
            // 旧快照一律作废：再次打开要重新拍（正文可能滚动过）。
            state.snapshot = None;
        }
        if open {
            if let Some((version, snap)) = self.capture_clip_window_sync() {
                let mut state = self.0.borrow_mut();
                if state.popup_open && state.version == version && !state.closed {
                    state.snapshot = Some((version, snap));
                }
            } else {
                self.capture_preview();
            }
        } else if restore_visible && self.0.borrow().loaded {
            self.set_visible(true);
        }
    }
    /// 浮层打开且快照已拍完（或没得拍）：子窗口该让位给快照了。
    /// 拍摄及解码期间子窗口必须保持可见，否则会拍到空白或出现白屏闪烁。
    pub(crate) fn popup_settled(&self) -> bool {
        let state = self.0.borrow();
        state.popup_open && !state.capturing
    }
    /// 当前文档的浮层快照（没有或已过期返回 None）。
    pub(crate) fn popup_snapshot(&self) -> Option<PopupSnapshot> {
        let state = self.0.borrow();
        if !state.popup_open {
            return None;
        }
        state
            .snapshot
            .as_ref()
            .filter(|(version, _)| *version == state.version)
            .map(|(_, snap)| snap.clone())
    }
    fn capture_clip_window_sync(&self) -> Option<(u64, PopupSnapshot)> {
        let (clip_window, version, bounds, renderer) = {
            let state = self.0.borrow();
            if !state.loaded || !state.window_shown || state.closed {
                return None;
            }
            (
                state.clip_window,
                state.version,
                state.bounds,
                state.svg_renderer.clone()?,
            )
        };
        let mut clip_rect = RECT::default();
        if unsafe { GetWindowRect(clip_window, &mut clip_rect) }.is_err() {
            return None;
        }
        let width = clip_rect.right - clip_rect.left;
        let height = clip_rect.bottom - clip_rect.top;
        if width <= 0 || height <= 0 {
            return None;
        }
        // 仅当宿主窗口处于前台且完整落在显示器内时，桌面 DC 才准确对应 WebView2 画面。
        let root = unsafe { GetAncestor(clip_window, GA_ROOT) };
        if root.is_invalid() || root != unsafe { GetForegroundWindow() } {
            return None;
        }
        let monitor = unsafe { MonitorFromRect(&clip_rect, MONITOR_DEFAULTTONULL) };
        if monitor.is_invalid() {
            return None;
        }
        let mut monitor_info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !unsafe { GetMonitorInfoW(monitor, &mut monitor_info) }.as_bool() {
            return None;
        }
        let mon = monitor_info.rcMonitor;
        if clip_rect.left < mon.left
            || clip_rect.top < mon.top
            || clip_rect.right > mon.right
            || clip_rect.bottom > mon.bottom
        {
            return None;
        }

        let screen_dc = unsafe { GetDC(None) };
        if screen_dc.is_invalid() {
            return None;
        }
        let mem_dc = unsafe { CreateCompatibleDC(Some(screen_dc)) };
        if mem_dc.is_invalid() {
            unsafe {
                let _ = ReleaseDC(None, screen_dc);
            }
            return None;
        }
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let Ok(hbitmap) = (unsafe {
            CreateDIBSection(Some(screen_dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
        }) else {
            unsafe {
                let _ = DeleteDC(mem_dc);
                let _ = ReleaseDC(None, screen_dc);
            }
            return None;
        };
        let old = unsafe { SelectObject(mem_dc, hbitmap.into()) };
        let blt_ok = unsafe {
            BitBlt(
                mem_dc,
                0,
                0,
                width,
                height,
                Some(screen_dc),
                clip_rect.left,
                clip_rect.top,
                SRCCOPY,
            )
        }
        .is_ok();
        let byte_len = (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(4)?;
        let bgra = if blt_ok && !bits.is_null() {
            Some(unsafe { std::slice::from_raw_parts(bits as *const u8, byte_len) }.to_vec())
        } else {
            None
        };
        unsafe {
            SelectObject(mem_dc, old);
            let _ = DeleteObject(hbitmap.into());
            let _ = DeleteDC(mem_dc);
            let _ = ReleaseDC(None, screen_dc);
        }
        let bmp = bmp_from_top_down_bgra(width, height, &bgra?)?;
        let image = gpui_kit::Image::from_bytes(gpui_kit::ImageFormat::Bmp, bmp)
            .to_image_data(renderer)
            .ok()?;
        let unclipped =
            bounds.left == 0 && bounds.top == 0 && bounds.right == width && bounds.bottom == height;
        let clipped_rect_px = (!unclipped).then_some((-bounds.left, -bounds.top, width, height));
        Some((
            version,
            PopupSnapshot {
                image,
                clipped_rect_px,
            },
        ))
    }
    fn capture_preview(&self) {
        let (controller, version, seq) = {
            let mut state = self.0.borrow_mut();
            // 子窗口没显示出来时 CapturePreview 拍到的是空白，直接跳过。
            if !state.loaded || !state.window_shown {
                return;
            }
            if state.capturing {
                return;
            }
            state.capturing = true;
            (state.controller.clone(), state.version, state.capture_seq)
        };
        let weak = Rc::downgrade(&self.0);
        let fail = move || {
            if let Some(state) = weak.upgrade() {
                finish_capture(&state, version, seq, None);
            }
        };
        let Some(stream) = (unsafe { SHCreateMemStream(None) }) else {
            fail();
            return;
        };
        let capture_state = Rc::downgrade(&self.0);
        let capture_stream = stream.clone();
        let handler = CapturePreviewCompletedHandler::create(Box::new(
            move |result: windows::core::Result<()>| {
                let bytes = result.ok().and_then(|_| read_stream(&capture_stream));
                let Some(state) = capture_state.upgrade() else {
                    return Ok(());
                };
                let (active, window, renderer) = {
                    let inner = state.borrow();
                    let active = inner.popup_open
                        && inner.version == version
                        && inner.capture_seq == seq
                        && !inner.closed;
                    (
                        active,
                        inner.scroll_window.clone(),
                        inner.svg_renderer.clone(),
                    )
                };
                let (Some(bytes), true, Some(renderer)) = (bytes, active, renderer) else {
                    finish_capture(&state, version, seq, None);
                    return Ok(());
                };
                if let Some(window) = window {
                    let decode_state = Rc::downgrade(&state);
                    window
                        .spawn(async move |cx| {
                            let image = cx
                                .background_executor()
                                .spawn(async move {
                                    gpui_kit::Image::from_bytes(gpui_kit::ImageFormat::Png, bytes)
                                        .to_image_data(renderer)
                                        .ok()
                                })
                                .await;
                            if let Some(state) = decode_state.upgrade() {
                                finish_capture(&state, version, seq, image);
                            }
                        })
                        .detach();
                } else {
                    let image = gpui_kit::Image::from_bytes(gpui_kit::ImageFormat::Png, bytes)
                        .to_image_data(renderer)
                        .ok();
                    finish_capture(&state, version, seq, image);
                }
                Ok(())
            },
        ));
        let Some(controller) = controller else {
            fail();
            return;
        };
        let captured = unsafe {
            controller.CoreWebView2().and_then(|view| {
                view.CapturePreview(
                    COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
                    &stream,
                    &handler,
                )
            })
        };
        if captured.is_err() {
            fail();
        }
    }
    pub(crate) fn load(&self, document: Vec<u8>) -> windows::core::Result<()> {
        self.load_with_remote(document, false)
    }
    pub(crate) fn load_with_remote(
        &self,
        document: Vec<u8>,
        allow_remote_images: bool,
    ) -> windows::core::Result<()> {
        self.load_document(document, allow_remote_images, false)
    }
    pub(crate) fn load_document(
        &self,
        document: Vec<u8>,
        allow_remote_images: bool,
        in_thread: bool,
    ) -> windows::core::Result<()> {
        self.set_visible(false);
        let mut state = self.0.borrow_mut();
        state.document = document;
        state.allow_remote_images = false;
        state.in_thread = in_thread;
        state.version = state.version.wrapping_add(1);
        state.loaded = false;
        state.navigation_id = None;
        state.snapshot = None;
        state.content_height = None;
        state.height_updates = 0;
        let url = document_url(state.version);
        let controller = state.controller.clone();
        if controller.is_some() {
            state.failed = false;
        }
        drop(state);
        if let Some(controller) = &controller {
            if let Ok(view) = unsafe { controller.CoreWebView2() } {
                let _ = unsafe { view.Stop() };
            }
        }
        self.0.borrow_mut().allow_remote_images = allow_remote_images;
        if let Some(controller) = controller {
            if let Err(error) = unsafe { navigate_to(&controller, &url) } {
                let mut state = self.0.borrow_mut();
                state.navigation_id = None;
                state.failed = true;
                return Err(error);
            }
        }
        Ok(())
    }
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

impl Drop for WebViewHost {
    fn drop(&mut self) {
        let mut state = self.0.borrow_mut();
        state.closed = true;
        state.document.clear();
        let controller = state.controller.take();
        let clip_window = state.clip_window;
        drop(state);
        if let Some(controller) = controller {
            unsafe {
                let _ = controller.Close();
            }
        }
        unsafe {
            let _ = DestroyWindow(clip_window);
        }
    }
}

fn content_height_message(message: &str) -> Option<f32> {
    let height: f32 = message.strip_prefix("content-height:")?.parse().ok()?;
    (height.is_finite() && height > 0.0).then(|| height.clamp(40.0, 4800.0))
}

fn record_content_height(state: &Rc<RefCell<State>>, source: &str, height: f32) -> bool {
    let mut inner = state.borrow_mut();
    if inner.closed
        || !inner.loaded
        || !inner.in_thread
        || !allow_main_navigation(source, inner.version)
        || inner.height_updates >= 24
    {
        return false;
    }
    if inner
        .content_height
        .is_some_and(|prev| (prev - height).abs() < 1.0)
    {
        return false;
    }
    inner.content_height = Some(height);
    inner.height_updates = inner.height_updates.saturating_add(1);
    inner.notify_ui();
    true
}

fn thread_wheel_delta(message: &str) -> Option<f32> {
    let delta: f32 = message.strip_prefix("thread-wheel:")?.parse().ok()?;
    (delta.is_finite() && delta != 0.0).then_some(delta)
}

fn forward_thread_wheel(
    state: Rc<RefCell<State>>,
    source: String,
    delta: f32,
) -> windows::core::Result<()> {
    let (window, clip_window) = {
        let state = state.borrow();
        let Some(window) = state.scroll_window.clone() else {
            return Ok(());
        };
        (window, state.clip_window)
    };
    let mut cursor = POINT::default();
    unsafe { GetCursorPos(&mut cursor)? };

    // Schedule GPUI input after the WebView2 callback returns.
    window
        .spawn(async move |cx| {
            {
                let state = state.borrow();
                if state.closed
                    || !state.loaded
                    || !state.window_shown
                    || !state.in_thread
                    || !allow_main_navigation(&source, state.version)
                {
                    return;
                }
            }
            let mut clip = RECT::default();
            let mut info = WINDOWINFO {
                cbSize: std::mem::size_of::<WINDOWINFO>() as u32,
                ..Default::default()
            };
            let position = unsafe {
                GetWindowRect(clip_window, &mut clip)
                    .and_then(|_| GetWindowInfo(GetParent(clip_window)?, &mut info))
            };
            if position.is_err()
                || cursor.x < clip.left
                || cursor.x >= clip.right
                || cursor.y < clip.top
                || cursor.y >= clip.bottom
            {
                return;
            }
            let _ = cx.update(|window, cx| {
                let scale = window.scale_factor();
                window.dispatch_event(
                    PlatformInput::Scroll(ScrollEvent {
                        position: point(
                            px((cursor.x - info.rcClient.left) as f32 / scale),
                            px((cursor.y - info.rcClient.top) as f32 / scale),
                        ),
                        delta: ScrollDelta::Pixels(point(px(0.), px(-delta))),
                        touch_phase: TouchPhase::Moved,
                        ..Default::default()
                    }),
                    cx,
                );
            });
        })
        .detach();
    Ok(())
}

unsafe fn configure(
    env: &ICoreWebView2Environment,
    controller: &ICoreWebView2Controller,
    state: Weak<RefCell<State>>,
) -> windows::core::Result<()> {
    let view = controller.CoreWebView2()?;
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
                .is_some_and(|state| allow_main_navigation(&url, state.borrow().version));
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
            if matched {
                maybe_install_bridge_script(&state, view.as_ref());
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
                if let Some(delta) = thread_wheel_delta(&message) {
                    forward_thread_wheel(state, source, delta)?;
                } else if let Some(height) = content_height_message(&message) {
                    record_content_height(&state, &source, height);
                } else if let Some(url) = open_link_message(&message) {
                    record_script_link(&state, &source, url);
                }
            }
        }
        Ok(())
    }));
    view.add_WebMessageReceived(&web_message, &mut token)?;
    let failure_state = state.clone();
    let failure = ProcessFailedEventHandler::create(Box::new(move |_, _| {
        if let Some(state) = failure_state.upgrade() {
            let mut state = state.borrow_mut();
            state.loaded = false;
            state.failed = true;
            state.notify_ui();
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
                    record_external_link(&mut state.borrow_mut(), &url);
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
                    record_external_link(&mut state.borrow_mut(), &url);
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
                        record_external_link(&mut state.borrow_mut(), &url);
                    }
                }
            }
            Ok(())
        }));
        let _ = view18.add_LaunchingExternalUriScheme(&external, &mut token);
    }
    let env = env.clone();
    let resource = WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
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
            let internal = url::Url::parse(&url).ok().is_some_and(|parsed| {
                parsed.scheme() == "https" && parsed.host_str() == Some("kitemail.invalid")
            });
            let allow_external = !internal
                && allow_remote_request(
                    &url,
                    context,
                    state.allow_remote_images,
                    &referer,
                    state.version,
                );
            let allow_remote_images = current && state.allow_remote_images;
            if allow_external {
                if let Some(request) = &request {
                    if let Ok(headers) = unsafe { request.Headers() } {
                        let _ = unsafe { headers.RemoveHeader(w!("Referer")) };
                    }
                }
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

    #[test]
    fn a_scheduled_html_document_shows_loading_before_the_host_exists() {
        assert_eq!(BodyView::for_document(true, true, None), BodyView::Loading);
        assert_eq!(BodyView::for_document(true, false, None), BodyView::Native);
        assert_eq!(BodyView::for_document(false, true, None), BodyView::Native);
    }

    #[test]
    fn snapshot_stream_is_read_from_the_start_and_stops_at_the_end() {
        let png = b"\x89PNG\r\n\x1a\nnot-really-a-png";
        let stream = unsafe { SHCreateMemStream(Some(png)) }.unwrap();
        // 第二次读同样从头开始，对应「拍完再读」的用法。
        assert_eq!(read_stream(&stream).as_deref(), Some(&png[..]));
        assert_eq!(read_stream(&stream).as_deref(), Some(&png[..]));
        let empty = unsafe { SHCreateMemStream(None) }.unwrap();
        assert!(read_stream(&empty).is_none());
    }

    #[test]
    fn bmp_from_top_down_bgra_produces_decodable_bmp() {
        let bgra = [0x10u8, 0x20, 0x30, 0x00, 0x40, 0x50, 0x60, 0x00];
        let bmp = bmp_from_top_down_bgra(2, 1, &bgra).unwrap();
        assert_eq!(bmp.len(), 54 + bgra.len());
        assert_eq!(&bmp[..2], b"BM");
        assert!(bmp_from_top_down_bgra(0, 1, &[]).is_none());
        assert!(bmp_from_top_down_bgra(2, 1, &bgra[..4]).is_none());
    }

    #[test]
    fn finish_capture_respects_capture_seq_popup_open_and_version() {
        let state = Rc::new(RefCell::new(State {
            version: 3,
            popup_open: true,
            capturing: true,
            capture_seq: 2,
            ..State::default()
        }));
        let host = WebViewHost(state.clone());
        let img = std::sync::Arc::new(gpui_kit::RenderImage::new(Vec::new()));

        // 拍摄尚未完成前不可让位，也没有快照。
        assert!(!host.popup_settled());
        assert!(host.popup_snapshot().is_none());

        // 过期序号（seq=1）不能打断当前拍摄（seq=2）。
        finish_capture(&state, 3, 1, Some(img.clone()));
        assert!(!host.popup_settled());
        assert!(host.popup_snapshot().is_none());

        // 过期版本（version=2）结束拍摄但不能写入快照。
        finish_capture(&state, 2, 2, Some(img.clone()));
        assert!(host.popup_settled());
        assert!(host.popup_snapshot().is_none());

        // 匹配当前版本和序号时保存快照并允许让位。
        state.borrow_mut().capturing = true;
        finish_capture(&state, 3, 2, Some(img));
        assert!(host.popup_settled());
        assert!(host.popup_snapshot().is_some());

        // 浮层已关闭时不保留新落地的快照。
        state.borrow_mut().popup_open = false;
        state.borrow_mut().snapshot = None;
        state.borrow_mut().capturing = true;
        let img2 = std::sync::Arc::new(gpui_kit::RenderImage::new(Vec::new()));
        finish_capture(&state, 3, 2, Some(img2));
        assert!(!host.popup_settled());
        assert!(host.popup_snapshot().is_none());
    }

    #[test]
    fn thread_wheel_messages_require_a_finite_nonzero_delta() {
        assert_eq!(thread_wheel_delta("thread-wheel:42.5"), Some(42.5));
        assert_eq!(thread_wheel_delta("thread-wheel:-120"), Some(-120.0));
        for message in [
            "42",
            "thread-wheel:",
            "thread-wheel:0",
            "thread-wheel:NaN",
            "thread-wheel:inf",
        ] {
            assert_eq!(thread_wheel_delta(message), None);
        }
    }

    #[test]
    fn native_images_follow_the_message_permission() {
        use gpui_kit::{ImageSource, SharedUri};

        let remote = SharedUri::from("https://example.com/image.png");
        let inline = SharedUri::from("data:image/png;base64,iVBORw0KGgo=");
        assert!(matches!(
            mail_image_source(&remote, false),
            ImageSource::Custom(_)
        ));
        assert!(matches!(
            mail_image_source(&remote, true),
            ImageSource::Resource(_)
        ));
        assert!(matches!(
            mail_image_source(&inline, false),
            ImageSource::Image(_)
        ));
        for path in [
            "file:///C:/image.png",
            "/image.png",
            "data:text/plain,hello",
        ] {
            assert!(matches!(
                mail_image_source(&SharedUri::from(path), true),
                ImageSource::Custom(_)
            ));
        }
    }

    #[test]
    fn prepared_links_are_checked_and_cleaned_for_dispatch() {
        let tracked = "https://example.com/path?id=42&utm_source=mail";
        assert_eq!(
            prepared_mail_link(tracked, true).as_deref(),
            Some("https://example.com/path?id=42")
        );
        assert_eq!(prepared_mail_link(tracked, false).as_deref(), Some(tracked));
        assert_eq!(
            prepared_mail_link("MAILTO:user@example.com", false).as_deref(),
            Some("mailto:user@example.com")
        );
        assert!(prepared_mail_link("file:///C:/test", true).is_none());
    }

    #[test]
    fn navigation_is_limited_to_the_document_and_inert_frames() {
        assert!(allow_main_navigation(&document_url(2), 2));
        assert!(!allow_main_navigation(&document_url(1), 2));
        assert!(!allow_main_navigation(
            "https://kitemail.invalid/reader?other",
            2
        ));
        assert!(!allow_main_navigation("https://example.com/", 2));
        assert!(allow_frame_navigation("about:srcdoc"));
        assert!(allow_frame_navigation("about:blank"));
        assert!(!allow_frame_navigation(DOCUMENT_URL));
        assert!(!allow_frame_navigation("file:///C:/test"));
        assert!(!allow_frame_navigation("https://example.com/"));
        assert!(allowed_external_url("https://example.com/path"));
        assert!(allowed_external_url("mailto:user@example.com"));
        assert!(!allowed_external_url("file:///C:/test"));
        assert!(!allowed_external_url("https://user:pass@example.com"));
        assert!(!allowed_external_url("javascript:alert(1)"));
        assert!(document_headers(false).contains("Referrer-Policy: no-referrer"));
        assert!(document_headers(true).contains("Referrer-Policy: no-referrer"));
        assert!(document_headers(false).contains("img-src data:;"));
        assert!(document_headers(true).contains("img-src data: http: https:;"));
        assert!(document_headers(true).contains("default-src 'none'"));
        let image = COREWEBVIEW2_WEB_RESOURCE_CONTEXT_IMAGE;
        assert!(allow_remote_request(
            "https://example.com/a.png",
            image,
            true,
            &document_url(4),
            4
        ));
        assert!(!allow_remote_request(
            "https://example.com/a.png",
            image,
            false,
            &document_url(4),
            4
        ));
        assert!(!allow_remote_request(
            "https://example.com/a.png",
            image,
            true,
            &document_url(3),
            4
        ));
        assert!(allow_remote_request(
            "https://example.com/a.png",
            image,
            true,
            "",
            4
        ));
        assert!(!allow_remote_request(
            "https://example.com/a.png",
            image,
            false,
            "",
            4
        ));
        assert!(!allow_remote_request(
            "https://example.com/a.png",
            COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
            true,
            &document_url(4),
            4
        ));
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
    }

    #[test]
    fn document_preserves_markup_inside_an_escaped_sandboxed_frame() {
        let (background, foreground) = crate::theme::reader_body_colors(false);
        let light = DocumentAppearance {
            dark: false,
            zoom_percent: 100,
            background,
            foreground,
        };
        let document = String::from_utf8(build_document(
            "<style>p{color:red}</style><p title=\"x\">A & B</p>",
            "<subject>",
            "sender & co",
            light,
        ))
        .unwrap();
        assert!(document.contains("<iframe sandbox=\"allow-same-origin allow-popups\" srcdoc=\""));
        assert!(document.contains("&lt;style&gt;p{color:red}&lt;/style&gt;"));
        assert!(document.contains("title=&quot;x&quot;"));
        assert!(document.contains("&lt;subject&gt;"));
        assert!(document.contains("sender &amp; co"));
        assert!(document.contains("background:#ffffff;color:#202124"));
        assert!(document.contains(":where(body){background-color:#ffffff;color:#202124}"));
        assert!(supports_document(
            "<p><a href='https://example.com'>link</a></p>"
        ));
        assert!(supports_document(
            "<a target='_blank' href='https://example.com'>link</a>"
        ));
        assert!(supports_document("<blockquote>quote</blockquote>"));
        assert!(supports_document("<div class='gmail_quote'>quote</div>"));
        assert!(supports_document("<a href='mailto:a@example.com'>mail</a>"));
        assert!(!supports_document(""));
        assert!(!supports_document("   \n\t "));
        let (background, foreground) = crate::theme::reader_body_colors(true);
        let dark = String::from_utf8(build_body_document(
            "<!doctype html><html><head><style>p{color:#000}</style></head><body><p>x</p></body></html>",
            DocumentAppearance {
                dark: true,
                zoom_percent: 150,
                background,
                foreground,
            },
        )).unwrap();
        assert!(dark.contains("color-scheme:dark;zoom:150%"));
        assert!(dark.contains(":where(body){background-color:#202124;color:#e8eaed}"));
        assert!(dark.contains("&lt;!doctype html&gt;&lt;html&gt;&lt;head&gt;&lt;style&gt;"));
        let clamped = String::from_utf8(build_body_document(
            "<p>x</p>",
            DocumentAppearance {
                zoom_percent: 999,
                ..light
            },
        ))
        .unwrap();
        assert!(clamped.contains("zoom:250%"));
    }

    #[test]
    fn make_document_does_not_insert_style_inside_header_element() {
        let (background, foreground) = crate::theme::reader_body_colors(false);
        let light = DocumentAppearance {
            dark: false,
            zoom_percent: 100,
            background,
            foreground,
        };
        let document = String::from_utf8(build_body_document(
            "<header><h1>Title</h1></header><p>Body</p>",
            light,
        ))
        .unwrap();
        assert!(document.contains("srcdoc=\"&lt;style&gt;:root{color-scheme:light;zoom:100%}"));
        assert!(document
            .contains("&lt;/style&gt;&lt;header&gt;&lt;h1&gt;Title&lt;/h1&gt;&lt;/header&gt;"));
    }

    #[test]
    fn script_mailto_links_validate_source_state_scheme_and_dedup() {
        assert_eq!(
            open_link_message("open-link:mailto:team@makenotion.com"),
            Some("mailto:team@makenotion.com")
        );
        assert_eq!(
            open_link_message("open-link:MAILTO:user@example.com?subject=hi"),
            Some("MAILTO:user@example.com?subject=hi")
        );
        for invalid in [
            "thread-wheel:10",
            "open-link:",
            "open-link:mailto:",
            "open-link:mailto:?subject=only",
            "open-link:https://example.com",
            "open-link:file:///C:/test",
            "open-link:javascript:alert(1)",
        ] {
            assert_eq!(open_link_message(invalid), None);
        }

        let (tx, mut rx) = futures_mpsc::channel::<()>(8);
        let state = Rc::new(RefCell::new(State::default()));
        {
            let mut inner = state.borrow_mut();
            inner.version = 5;
            inner.loaded = true;
            inner.notifier = Some(tx);
        }

        // Valid mailto from current document version queues link and wakes UI.
        assert!(record_script_link(
            &state,
            &document_url(5),
            "mailto:team@makenotion.com"
        ));
        assert_eq!(rx.try_recv().ok(), Some(()));

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
        assert_eq!(rx.try_recv().ok(), Some(()));

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

        let links = WebViewHost(state).take_links();
        assert_eq!(
            links,
            vec![
                "mailto:team@makenotion.com".to_string(),
                "MAILTO:other@example.com".to_string()
            ]
        );
    }

    #[test]
    fn build_body_document_omits_header() {
        let (background, foreground) = crate::theme::reader_body_colors(false);
        let light = DocumentAppearance {
            dark: false,
            zoom_percent: 100,
            background,
            foreground,
        };
        let document = String::from_utf8(build_body_document("<p>正文内容</p>", light)).unwrap();
        assert!(!document.contains("<header>"));
        assert!(document.contains("<iframe"));
    }

    #[test]
    fn dom_content_loaded_marks_ready_and_wakes_notifier_before_navigation_completed() {
        let (tx, mut rx) = futures_mpsc::channel::<()>(4);
        let mut state = State::default();
        state.version = 3;
        state.navigation_id = Some(42);
        state.notifier = Some(tx);

        // Stale navigation ID is ignored.
        assert!(!apply_dom_content_loaded(&mut state, 7));
        assert!(!state.loaded);
        assert!(rx.try_recv().is_err());

        // Matching DOMContentLoaded wakes UI immediately while remote images may still load.
        assert!(apply_dom_content_loaded(&mut state, 42));
        assert!(state.loaded);
        assert!(!state.failed);
        assert_eq!(rx.try_recv().ok(), Some(()));

        // Explicit main-document navigation failure resets loaded back to false.
        assert!(!apply_navigation_completed(&mut state, 42, false));
        assert!(!state.loaded);
        assert!(state.failed);
        assert_eq!(rx.try_recv().ok(), Some(()));
    }

    #[test]
    fn thread_content_height_is_validated_clamped_and_deduped() {
        assert_eq!(content_height_message("content-height:642.5"), Some(642.5));
        assert_eq!(content_height_message("content-height:12"), Some(40.0));
        assert_eq!(content_height_message("content-height:99999"), Some(4800.0));
        for invalid in [
            "content-height:",
            "content-height:0",
            "content-height:-10",
            "content-height:NaN",
            "content-height:inf",
            "thread-wheel:10",
        ] {
            assert_eq!(content_height_message(invalid), None);
        }

        let (tx, mut rx) = futures_mpsc::channel::<()>(8);
        let state = Rc::new(RefCell::new(State {
            version: 2,
            loaded: true,
            in_thread: true,
            notifier: Some(tx),
            ..State::default()
        }));
        assert!(record_content_height(&state, &document_url(2), 512.0));
        assert_eq!(WebViewHost(state.clone()).content_height(), Some(512.0));
        assert_eq!(rx.try_recv().ok(), Some(()));

        // Sub-pixel jitter and wrong version/source are ignored.
        assert!(!record_content_height(&state, &document_url(2), 512.4));
        assert!(!record_content_height(&state, &document_url(1), 700.0));
        assert!(rx.try_recv().is_err());

        let (background, foreground) = crate::theme::reader_body_colors(false);
        let light = DocumentAppearance {
            dark: false,
            zoom_percent: 100,
            background,
            foreground,
        };
        let doc = String::from_utf8(build_thread_body_document("<p>Hi</p>", light)).unwrap();
        assert!(doc.contains("display:flow-root!important"));
        assert!(doc.contains("height:auto!important"));
    }
}
