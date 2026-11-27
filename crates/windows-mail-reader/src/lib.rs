//! Shared Windows native mail reader (document builders, security policy,
//! dark-mode color adaptation, and WebView2 controller host).

pub mod dark;
pub mod document;
pub mod policy;
pub mod thread_document;
#[cfg(windows)]
pub mod webview2;

pub use dark::{adapt_colors_for_dark, parse_css_color, pin_color_scheme};
pub use document::{
    build_body_document, build_document, build_plain_text_document, escape_html, find_html_tag,
    reader_body_colors, supports_document, DocumentAppearance,
};
pub use policy::{
    allow_frame_navigation, allow_main_navigation, allow_remote_image_request,
    allowed_external_url, allowed_mail_image_url, decode_inline_mail_image, document_headers,
    document_url, is_internal_reader_url, needs_new_host, open_link_message, parse_external_url,
    parse_thread_action, prepare_external_url, DocumentMode, InlineImageFormat, ReaderLoadStatus,
    WebViewAction, DOCUMENT_URL,
};
pub use thread_document::{
    avatar_badge_spec, build_thread_document, first_snippet_line, format_raw_source,
    recipient_list, split_sender, ThreadCardRenderInput, ThreadDocContext, ThreadPalette,
    STAR_GOLD,
};
#[cfg(windows)]
pub use webview2::{
    clipped_rects, hwnd_scale_factor, logical_to_pixel_rect, LogicalRect, PixelRect, ReaderEvent,
    WebView2Host,
};
