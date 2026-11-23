//! 异步 WebView2 宿主。调用方必须在创建父窗口的 STA 线程使用。
#![allow(dead_code)]

use std::{
    cell::RefCell,
    os::windows::ffi::OsStrExt,
    rc::{Rc, Weak},
};
use webview2_com::{
    CreateCoreWebView2ControllerCompletedHandler, CreateCoreWebView2EnvironmentCompletedHandler,
    Microsoft::Web::WebView2::Win32::*, NavigationCompletedEventHandler,
    NavigationStartingEventHandler, NewWindowRequestedEventHandler, ProcessFailedEventHandler,
    WebResourceRequestedEventHandler,
};
use windows::{
    core::w,
    Win32::{
        Foundation::{HWND, RECT},
        UI::{
            Shell::SHCreateMemStream,
            WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, SetWindowPos, ShowWindow, SWP_NOACTIVATE,
                SWP_NOZORDER, SW_HIDE, SW_SHOWNOACTIVATE, WINDOW_EX_STYLE, WS_CHILD,
                WS_CLIPCHILDREN,
            },
        },
    },
};

const DOCUMENT_URL: &str = "https://hylki.invalid/reader";

#[path = "../../../src/ui/message_view/dark.rs"]
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
        parse_external_url(&hylki_mail_core::link_clean::clean_link(url.as_str()))?
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
    format!("Content-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: unsafe-url\r\nContent-Security-Policy: default-src 'none'; img-src {image_sources}; style-src 'unsafe-inline'; frame-src 'self'; base-uri 'none'; form-action 'none'\r\n")
}

fn allow_remote_request(
    url: &str,
    context: COREWEBVIEW2_WEB_RESOURCE_CONTEXT,
    allowed: bool,
    referer: &str,
    version: u64,
) -> bool {
    context == COREWEBVIEW2_WEB_RESOURCE_CONTEXT_IMAGE
        && referer == document_url(version)
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
    let lower = html.to_ascii_lowercase();
    !html.trim().is_empty()
        && !lower.contains("<blockquote")
        && !lower.contains("gmail_quote")
        && !lower.contains("divrplyfwdmsg")
        && !lower.contains("appendonsend")
        && !lower.contains("mailto:")
}

pub(crate) fn build_body_document(html: &str, dark_mode: bool, zoom_percent: u32) -> Vec<u8> {
    make_document(html, None, dark_mode, zoom_percent)
}

pub(crate) fn build_document(
    html: &str,
    subject: &str,
    sender: &str,
    dark_mode: bool,
    zoom_percent: u32,
) -> Vec<u8> {
    make_document(html, Some((subject, sender)), dark_mode, zoom_percent)
}

fn make_document(
    html: &str,
    header: Option<(&str, &str)>,
    dark_mode: bool,
    zoom_percent: u32,
) -> Vec<u8> {
    let escape = |value: &str| {
        value
            .replace('&', "&amp;")
            .replace('"', "&quot;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    let html = dark::pin_color_scheme(html, dark_mode);
    let html = if dark_mode {
        dark::adapt_colors_for_dark(&html)
    } else {
        html
    };
    let (bg, fg, scheme) = if dark_mode {
        ("#202124", "#e8eaed", "dark")
    } else {
        ("#fff", "#202124", "light")
    };
    let style = format!(
        "<style>:root{{color-scheme:{scheme};zoom:{}%}}body{{background:{bg};color:{fg}}}</style>",
        zoom_percent.clamp(50, 250)
    );
    let lower = html.to_ascii_lowercase();
    let insertion = lower
        .find("<head")
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
    format!("<!doctype html><meta charset=utf-8><style>html,body{{height:100%;margin:0}}body{{display:flex;flex-direction:column;font:14px system-ui;background:{bg};color:{fg}}}header{{padding:24px 24px 12px}}h1{{font-size:20px;margin:0 0 12px}}iframe{{flex:1;width:100%;border:0;min-height:0}}</style>{header}<iframe sandbox=\"allow-same-origin allow-popups\" srcdoc=\"{escaped}\"></iframe>").into_bytes()
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
    bounds: RECT,
    visible: bool,
    has_area: bool,
    window_shown: bool,
    failed: bool,
    closed: bool,
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
                state.borrow_mut().failed = true;
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
                state.borrow_mut().failed = true;
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
                state.borrow_mut().failed = true;
            }
            Ok(())
        },
    ));
    if unsafe { environment.CreateCoreWebView2Controller(parent, &handler) }.is_err() {
        state.borrow_mut().failed = true;
    }
}

/// 创建不会阻塞或泵消息；异步完成后可通过 `ready` 检查。
pub(crate) struct WebViewHost(Rc<RefCell<State>>);

impl WebViewHost {
    pub(crate) fn create(parent: HWND) -> windows::core::Result<Self> {
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
                        state.borrow_mut().failed = true;
                    }
                }
                Ok(())
            },
        ));
        let data_dir = std::env::var_os("LOCALAPPDATA")
            .map(std::path::PathBuf::from)
            .map(|path| path.join("Hylki").join("WebView2"));
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
                    state.borrow_mut().failed = true;
                }
            }
            unsafe {
                let _ = DestroyWindow(clip_window);
            }
            return Err(error);
        }
        Ok(Self(state))
    }

    pub(crate) fn ready(&self) -> bool {
        let state = self.0.borrow();
        state.controller.is_some() && state.loaded && !state.failed
    }
    pub(crate) fn failed(&self) -> bool {
        self.0.borrow().failed
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
        state.bounds = bounds;
        state.has_area = has_area;
        let controller = state.controller.clone();
        let clip_window = state.clip_window;
        let visible = state.visible && has_area;
        let visibility_changed = state.window_shown != visible;
        state.window_shown = visible;
        drop(state);
        unsafe {
            if has_area {
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
                let _ = controller.SetBounds(bounds);
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
    pub(crate) fn load(&self, document: Vec<u8>) -> windows::core::Result<()> {
        self.load_with_remote(document, false)
    }
    pub(crate) fn load_with_remote(
        &self,
        document: Vec<u8>,
        allow_remote_images: bool,
    ) -> windows::core::Result<()> {
        let mut state = self.0.borrow_mut();
        state.document = document;
        state.allow_remote_images = false;
        state.version = state.version.wrapping_add(1);
        state.loaded = false;
        state.navigation_id = None;
        let url = document_url(state.version);
        let controller = state.controller.clone();
        drop(state);
        if let Some(controller) = &controller {
            unsafe {
                let _ = controller.CoreWebView2()?.Stop();
            }
        }
        self.0.borrow_mut().allow_remote_images = allow_remote_images;
        if let Some(controller) = controller {
            unsafe {
                navigate_to(&controller, &url)?;
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
    let complete_state = state.clone();
    let complete = NavigationCompletedEventHandler::create(Box::new(move |_, args| {
        if let (Some(state), Some(args)) = (complete_state.upgrade(), args) {
            let mut id = 0;
            let mut success = windows::core::BOOL::default();
            unsafe {
                args.NavigationId(&mut id)?;
                args.IsSuccess(&mut success)?;
            }
            let mut state = state.borrow_mut();
            if state.navigation_id == Some(id) {
                state.loaded = success.as_bool();
                state.failed = !state.loaded;
            }
        }
        Ok(())
    }));
    view.add_NavigationCompleted(&complete, &mut token)?;
    let failure_state = state.clone();
    let failure = ProcessFailedEventHandler::create(Box::new(move |_, _| {
        if let Some(state) = failure_state.upgrade() {
            let mut state = state.borrow_mut();
            state.loaded = false;
            state.failed = true;
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
                    let mut state = state.borrow_mut();
                    if state.loaded {
                        let version = state.version;
                        if !state
                            .links
                            .last()
                            .is_some_and(|(v, last)| *v == version && *last == url)
                        {
                            state.links.push((version, url.clone()));
                        }
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
                    let mut state = state.borrow_mut();
                    if state.loaded {
                        let version = state.version;
                        if !state
                            .links
                            .last()
                            .is_some_and(|(v, last)| *v == version && *last == url)
                        {
                            state.links.push((version, url));
                        }
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
                parsed.scheme() == "https" && parsed.host_str() == Some("hylki.invalid")
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
            "https://hylki.invalid/reader?other",
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
        assert!(!allow_remote_request(
            "https://example.com/a.png",
            image,
            true,
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
        let document = String::from_utf8(build_document(
            "<style>p{color:red}</style><p title=\"x\">A & B</p>",
            "<subject>",
            "sender & co",
            false,
            100,
        ))
        .unwrap();
        assert!(document.contains("<iframe sandbox=\"allow-same-origin allow-popups\" srcdoc=\""));
        assert!(document.contains("&lt;style&gt;p{color:red}&lt;/style&gt;"));
        assert!(document.contains("title=&quot;x&quot;"));
        assert!(document.contains("&lt;subject&gt;"));
        assert!(document.contains("sender &amp; co"));
        assert!(supports_document(
            "<p><a href='https://example.com'>link</a></p>"
        ));
        assert!(supports_document(
            "<a target='_blank' href='https://example.com'>link</a>"
        ));
        assert!(!supports_document("<blockquote>quote</blockquote>"));
        assert!(!supports_document(
            "<a href='mailto:a@example.com'>mail</a>"
        ));
        let dark = String::from_utf8(build_body_document("<!doctype html><html><head><style>p{color:#000}</style></head><body><p>x</p></body></html>", true, 150)).unwrap();
        assert!(dark.contains("color-scheme:dark;zoom:150%"));
        assert!(dark.contains("&lt;!doctype html&gt;&lt;html&gt;&lt;head&gt;&lt;style&gt;"));
        let clamped = String::from_utf8(build_body_document("<p>x</p>", false, 999)).unwrap();
        assert!(clamped.contains("zoom:250%"));
    }
}
