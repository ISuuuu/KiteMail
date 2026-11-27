//! GPUI 异步 WebView2 宿主适配器，委托共享核心 `kitemail-windows-mail-reader`。
//! 仅保留 GPUI 专属的 `Hsla`、`AsyncWindowContext`、`SvgRenderer`、`ImageSource`、
//! 焦点回传和浮层快照逻辑。调用方必须在创建父窗口的 STA 线程使用。
#![allow(dead_code)]

use futures::channel::mpsc as futures_mpsc;
use gpui_kit::{AsyncWindowContext, Hsla, Window};
use kitemail_windows_mail_reader::{
    allowed_mail_image_url, decode_inline_mail_image as decode_shared_inline_image,
    prepare_external_url, InlineImageFormat, ReaderEvent, ReaderLoadStatus, WebView2Host,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{cell::RefCell, rc::Rc};
use webview2_com::{CapturePreviewCompletedHandler, Microsoft::Web::WebView2::Win32::*};
use windows::Win32::{
    Foundation::{HWND, RECT},
    Graphics::Gdi::{
        BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC,
        GetMonitorInfoW, MonitorFromRect, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER,
        BI_RGB, DIB_RGB_COLORS, MONITORINFO, MONITOR_DEFAULTTONULL, SRCCOPY,
    },
    UI::{
        Input::KeyboardAndMouse::SetFocus,
        Shell::SHCreateMemStream,
        WindowsAndMessaging::{GetAncestor, GetForegroundWindow, GetWindowRect, GA_ROOT},
    },
};

pub(crate) use kitemail_windows_mail_reader::{supports_document, DocumentMode, WebViewAction};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum BodyView {
    #[default]
    Native,
    Loading,
    Html,
}

impl From<ReaderLoadStatus> for BodyView {
    fn from(status: ReaderLoadStatus) -> Self {
        match status {
            ReaderLoadStatus::Loading => Self::Loading,
            ReaderLoadStatus::Ready => Self::Html,
            ReaderLoadStatus::Unavailable | ReaderLoadStatus::ProcessFailed => Self::Native,
        }
    }
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

    let (format, bytes) = decode_shared_inline_image(value)?;
    let format = match format {
        InlineImageFormat::Png => ImageFormat::Png,
        InlineImageFormat::Jpeg => ImageFormat::Jpeg,
        InlineImageFormat::Gif => ImageFormat::Gif,
        InlineImageFormat::Webp => ImageFormat::Webp,
        InlineImageFormat::Bmp => ImageFormat::Bmp,
    };
    Some((format, bytes))
}

pub(crate) fn prepared_mail_link(value: &str, clean: bool) -> Option<String> {
    prepare_external_url(value, clean)
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct DocumentAppearance {
    pub dark: bool,
    pub zoom_percent: u32,
    pub background: Hsla,
    pub foreground: Hsla,
}

impl DocumentAppearance {
    fn to_shared(self) -> kitemail_windows_mail_reader::DocumentAppearance {
        let mut shared = kitemail_windows_mail_reader::DocumentAppearance::for_dark(
            self.dark,
            self.zoom_percent,
            false,
        );
        shared.canvas_bg = crate::theme::css_hex_color(self.background);
        shared.text_color = crate::theme::css_hex_color(self.foreground);
        shared
    }
}

pub(crate) fn build_body_document(html: &str, appearance: DocumentAppearance) -> Vec<u8> {
    kitemail_windows_mail_reader::build_body_document(html, &appearance.to_shared())
}

pub(crate) fn build_document(
    html: &str,
    subject: &str,
    sender: &str,
    appearance: DocumentAppearance,
) -> Vec<u8> {
    kitemail_windows_mail_reader::build_document(html, subject, sender, &appearance.to_shared())
}

#[derive(Clone)]
pub(crate) struct PopupSnapshot {
    pub(crate) image: std::sync::Arc<gpui_kit::RenderImage>,
    /// 非 `None` 时表示仅截取了裁剪后的 `clip_window` 区域，记录其在内容区内的物理偏移与尺寸
    /// `(offset_x, offset_y, width, height)`；`None` 表示与内容区域 1:1 铺满。
    pub(crate) clipped_rect_px: Option<(i32, i32, i32, i32)>,
}

#[derive(Default)]
struct PopupState {
    scroll_window: Option<AsyncWindowContext>,
    svg_renderer: Option<gpui_kit::SvgRenderer>,
    notifier: Option<futures_mpsc::Sender<()>>,
    popup_open: bool,
    capturing: bool,
    capture_seq: u64,
    snapshot: Option<(u64, PopupSnapshot)>,
}

impl PopupState {
    fn notify_ui(&mut self) {
        if let Some(tx) = &mut self.notifier {
            let _ = tx.try_send(());
        }
    }
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

trait CaptureHostState {
    fn version(&self) -> u64;
    fn is_closed(&self) -> bool;
}

impl CaptureHostState for WebView2Host {
    fn version(&self) -> u64 {
        WebView2Host::version(self)
    }

    fn is_closed(&self) -> bool {
        WebView2Host::is_closed(self)
    }
}

/// 快照拍摄结束（成功或失败）：记下已解码的 RenderImage 并唤醒 UI 换帧。
fn finish_capture(
    inner: &dyn CaptureHostState,
    popup: &Rc<RefCell<PopupState>>,
    version: u64,
    seq: u64,
    image: Option<std::sync::Arc<gpui_kit::RenderImage>>,
) {
    let mut ps = popup.borrow_mut();
    if ps.capture_seq != seq {
        return;
    }
    ps.capturing = false;
    if let Some(image) =
        image.filter(|_| ps.popup_open && inner.version() == version && !inner.is_closed())
    {
        ps.snapshot = Some((
            version,
            PopupSnapshot {
                image,
                clipped_rect_px: None,
            },
        ));
    }
    ps.notify_ui();
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
pub(crate) struct WebViewHost {
    inner: Rc<WebView2Host>,
    popup: Rc<RefCell<PopupState>>,
}

impl WebViewHost {
    pub(crate) fn retire_for_remote_reload(&self, next_allowed: bool) -> bool {
        self.inner.retire_for_remote_reload(next_allowed)
    }

    pub(crate) fn focus_host_window(window: &Window) {
        let Ok(handle) = HasWindowHandle::window_handle(window) else {
            return;
        };
        if let RawWindowHandle::Win32(raw) = handle.as_raw() {
            let hwnd = HWND(raw.hwnd.get() as *mut _);
            unsafe {
                let _ = SetFocus(Some(hwnd));
            }
        }
    }

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
        let user_data_dir = kitemail_mail_core::config::data_dir().join("WebView2");
        let inner = Rc::new(WebView2Host::create(parent, user_data_dir)?);
        let popup = Rc::new(RefCell::new(PopupState {
            scroll_window,
            ..PopupState::default()
        }));
        Ok(Self { inner, popup })
    }

    pub(crate) fn set_notifier(&self, tx: futures_mpsc::Sender<()>) {
        self.popup.borrow_mut().notifier = Some(tx.clone());
        self.inner.set_event_callback(move |event| match event {
            ReaderEvent::ScrollAnchor { .. } => {}
            _ => {
                let mut sender = tx.clone();
                let _ = sender.try_send(());
            }
        });
    }

    pub(crate) fn set_svg_renderer(&self, renderer: gpui_kit::SvgRenderer) {
        self.popup.borrow_mut().svg_renderer = Some(renderer);
    }

    pub(crate) fn load_status(&self) -> (u64, BodyView) {
        let (version, status) = self.inner.status();
        (version, BodyView::from(status))
    }

    pub(crate) fn take_links(&self) -> Vec<String> {
        self.inner.take_links()
    }

    pub(crate) fn take_actions(&self) -> Vec<WebViewAction> {
        self.inner.take_actions()
    }

    pub(crate) fn last_scroll_anchor(&self) -> Option<(u32, i32)> {
        self.inner.last_scroll_anchor()
    }

    pub(crate) fn execute_script(&self, script: &str) {
        self.inner.execute_script(script);
    }

    pub(crate) fn try_suspend(&self) {
        self.inner.try_suspend();
    }

    pub(crate) fn set_bounds(&self, bounds: RECT) {
        self.inner.set_rect(bounds);
    }

    pub(crate) fn set_clipped_bounds(&self, content: RECT, viewport: RECT) {
        self.inner.set_clipped_bounds(content, viewport);
    }

    pub(crate) fn set_visible(&self, visible: bool) {
        self.inner.set_visible(visible);
    }

    /// 窗口内浮层（菜单 / 弹层）开关。
    ///
    /// 浮层画在窗口自己的绘制层里，会被正文的 WebView2 子窗口盖住。打开时优先在点击
    /// 当下同步截取 `clip_window` 屏幕像素，在第一帧渲染前就就绪快照，避免异步
    /// `CapturePreview` 等待期内浮层被 WebView2 遮挡以及随后的切图闪烁；不满足
    /// 同步截屏条件时再回退到异步 `CapturePreview`。关闭时按需立即放回子窗口。
    pub(crate) fn set_popup_open(&self, open: bool, restore_visible: bool) {
        self.inner.set_suspend_inhibited(open);
        {
            let mut ps = self.popup.borrow_mut();
            ps.popup_open = open;
            ps.capture_seq = ps.capture_seq.wrapping_add(1);
            ps.capturing = false;
            // 旧快照一律作废：再次打开要重新拍（正文可能滚动过）。
            ps.snapshot = None;
        }
        if open {
            if let Some((version, snap)) = self.capture_clip_window_sync() {
                let mut ps = self.popup.borrow_mut();
                if ps.popup_open && self.inner.version() == version && !self.inner.is_closed() {
                    ps.snapshot = Some((version, snap));
                }
            } else {
                self.capture_preview();
            }
        } else if restore_visible && self.inner.is_loaded() {
            self.inner.set_visible(true);
        }
    }

    /// 浮层打开且快照已拍完（或没得拍）：子窗口该让位给快照了。
    /// 拍摄及解码期间子窗口必须保持可见，否则会拍到空白或出现白屏闪烁。
    pub(crate) fn popup_settled(&self) -> bool {
        let ps = self.popup.borrow();
        ps.popup_open && !ps.capturing
    }

    /// 当前文档的浮层快照（没有或已过期返回 None）。
    pub(crate) fn popup_snapshot(&self) -> Option<PopupSnapshot> {
        let ps = self.popup.borrow();
        if !ps.popup_open {
            return None;
        }
        let version = self.inner.version();
        ps.snapshot
            .as_ref()
            .filter(|(v, _)| *v == version)
            .map(|(_, snap)| snap.clone())
    }

    fn capture_clip_window_sync(&self) -> Option<(u64, PopupSnapshot)> {
        if !self.inner.is_loaded() || !self.inner.is_window_shown() || self.inner.is_closed() {
            return None;
        }
        let clip_window = self.inner.clip_hwnd();
        let version = self.inner.version();
        let bounds = self.inner.bounds_rect();
        let renderer = self.popup.borrow().svg_renderer.clone()?;
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
        let (version, seq) = {
            if !self.inner.is_loaded() || !self.inner.is_window_shown() {
                return;
            }
            let mut ps = self.popup.borrow_mut();
            if ps.capturing {
                return;
            }
            ps.capturing = true;
            (self.inner.version(), ps.capture_seq)
        };
        let weak_inner = Rc::downgrade(&self.inner);
        let weak_popup = Rc::downgrade(&self.popup);
        let fail = move || {
            if let (Some(inner), Some(popup)) = (weak_inner.upgrade(), weak_popup.upgrade()) {
                finish_capture(inner.as_ref(), &popup, version, seq, None);
            }
        };
        let Some(stream) = (unsafe { SHCreateMemStream(None) }) else {
            fail();
            return;
        };
        let capture_inner = Rc::downgrade(&self.inner);
        let capture_popup = Rc::downgrade(&self.popup);
        let capture_stream = stream.clone();
        let handler = CapturePreviewCompletedHandler::create(Box::new(
            move |result: windows::core::Result<()>| {
                let bytes = result.ok().and_then(|_| {
                    kitemail_windows_mail_reader::webview2::read_stream(&capture_stream)
                });
                let (Some(inner), Some(popup)) = (capture_inner.upgrade(), capture_popup.upgrade())
                else {
                    return Ok(());
                };
                let (active, window, renderer) = {
                    let ps = popup.borrow();
                    let active = ps.popup_open
                        && inner.version() == version
                        && ps.capture_seq == seq
                        && !inner.is_closed();
                    (active, ps.scroll_window.clone(), ps.svg_renderer.clone())
                };
                let (Some(bytes), true, Some(renderer)) = (bytes, active, renderer) else {
                    finish_capture(inner.as_ref(), &popup, version, seq, None);
                    return Ok(());
                };
                if let Some(window) = window {
                    let decode_inner = Rc::downgrade(&inner);
                    let decode_popup = Rc::downgrade(&popup);
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
                            if let (Some(inner), Some(popup)) =
                                (decode_inner.upgrade(), decode_popup.upgrade())
                            {
                                finish_capture(inner.as_ref(), &popup, version, seq, image);
                            }
                        })
                        .detach();
                } else {
                    let image = gpui_kit::Image::from_bytes(gpui_kit::ImageFormat::Png, bytes)
                        .to_image_data(renderer)
                        .ok();
                    finish_capture(inner.as_ref(), &popup, version, seq, image);
                }
                Ok(())
            },
        ));
        let Some(view) = self.inner.core_webview2() else {
            fail();
            return;
        };
        let captured = unsafe {
            view.CapturePreview(
                COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
                &stream,
                &handler,
            )
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
        self.load_document(document, allow_remote_images, DocumentMode::SingleBody)
    }

    pub(crate) fn unload_and_suspend(&self) {
        self.popup.borrow_mut().snapshot = None;
        self.inner.unload_and_suspend();
    }

    pub(crate) fn load_document(
        &self,
        document: Vec<u8>,
        allow_remote_images: bool,
        mode: DocumentMode,
    ) -> windows::core::Result<()> {
        self.popup.borrow_mut().snapshot = None;
        self.inner
            .load_document(document, allow_remote_images, mode)
            .map(|_| ())
    }
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
    fn bmp_from_top_down_bgra_produces_decodable_bmp() {
        let bgra = [0x10u8, 0x20, 0x30, 0x00, 0x40, 0x50, 0x60, 0x00];
        let bmp = bmp_from_top_down_bgra(2, 1, &bgra).unwrap();
        assert_eq!(bmp.len(), 54 + bgra.len());
        assert_eq!(&bmp[..2], b"BM");
        assert!(bmp_from_top_down_bgra(0, 1, &[]).is_none());
        assert!(bmp_from_top_down_bgra(2, 1, &bgra[..4]).is_none());
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
    fn finish_capture_respects_capture_seq_popup_open_and_version() {
        struct MockHost {
            version: u64,
            closed: bool,
        }
        impl CaptureHostState for MockHost {
            fn version(&self) -> u64 {
                self.version
            }
            fn is_closed(&self) -> bool {
                self.closed
            }
        }

        let host = MockHost {
            version: 3,
            closed: false,
        };
        let popup = Rc::new(RefCell::new(PopupState {
            popup_open: true,
            capturing: true,
            capture_seq: 2,
            ..PopupState::default()
        }));
        let img = std::sync::Arc::new(gpui_kit::RenderImage::new(Vec::new()));

        let settled = |p: &Rc<RefCell<PopupState>>| {
            let ps = p.borrow();
            ps.popup_open && !ps.capturing
        };
        let snap = |p: &Rc<RefCell<PopupState>>| {
            let ps = p.borrow();
            if !ps.popup_open {
                return None;
            }
            ps.snapshot
                .as_ref()
                .filter(|(v, _)| *v == host.version)
                .map(|(_, s)| s.clone())
        };

        assert!(!settled(&popup));
        assert!(snap(&popup).is_none());

        // 过期序号（seq=1）不能打断当前拍摄（seq=2）。
        finish_capture(&host, &popup, 3, 1, Some(img.clone()));
        assert!(!settled(&popup));
        assert!(snap(&popup).is_none());

        // 过期版本（version=2）结束拍摄但不能写入快照。
        finish_capture(&host, &popup, 2, 2, Some(img.clone()));
        assert!(settled(&popup));
        assert!(snap(&popup).is_none());

        // 匹配当前版本和序号时保存快照并允许让位。
        popup.borrow_mut().capturing = true;
        finish_capture(&host, &popup, 3, 2, Some(img));
        assert!(settled(&popup));
        assert!(snap(&popup).is_some());

        // 浮层已关闭时不保留新落地的快照。
        popup.borrow_mut().popup_open = false;
        popup.borrow_mut().snapshot = None;
        popup.borrow_mut().capturing = true;
        let img2 = std::sync::Arc::new(gpui_kit::RenderImage::new(Vec::new()));
        finish_capture(&host, &popup, 3, 2, Some(img2));
        assert!(!settled(&popup));
        assert!(snap(&popup).is_none());
    }

    #[test]
    fn gpui_document_appearance_preserves_hsla_colors() {
        let (background, foreground) = crate::theme::reader_body_colors(false);
        let appearance = DocumentAppearance {
            dark: false,
            zoom_percent: 100,
            background,
            foreground,
        };
        let document = String::from_utf8(build_document(
            "<p>Hi</p>",
            "<subject>",
            "sender & co",
            appearance,
        ))
        .unwrap();
        assert!(document.contains("background:#ffffff;color:#202124"));
        assert!(document.contains(":where(body){background-color:#ffffff;color:#202124}"));
    }
}
