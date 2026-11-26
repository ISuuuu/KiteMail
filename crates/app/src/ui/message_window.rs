//! 独立邮件阅读窗口（Pop-out Window，WP-09，对齐 `src/ui/message_window.rs`）：
//! - 在独立 GPUI 窗口中查看单封邮件或会话；
//! - 复用 `cid:` 内联图、引用折叠（`⋯ 显示引用内容 (N 行)`）、`.link-preview` 悬停预览牌、
//!   页内查找（`Ctrl+F`）以及附件列表；
//! - 回复 / 回复全部 / 转发 / 星标操作通过 `WeakEntity<HylkiApp>` 回传主窗口执行。

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::text::{RangeHighlight, TextView, TextViewState};
use gpui_kit::component::Icon;
use gpui_kit::component::{h_flex, v_flex, Disableable, Selectable, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    black, div, hsla, px, AppContext, Context, Entity, Focusable, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, Styled, WeakEntity, Window,
};
use kitemail_mail_core::config::{self, AppPreferences};
use kitemail_mail_core::imap_client::{plain_text_to_html, sanitize_html_for_display_with_options};
use kitemail_mail_core::mime::{split_quoted_html, split_quoted_plain};
use kitemail_mail_core::{format_message_datetime, MessageDetail};

use crate::app::{split_sender, HylkiApp, PopoutReplyKind};
use crate::theme;
use crate::ui::toast::{self, ToastAction, ToastActionKind, ToastKind, WindowToastQueue};
use crate::ui::{avatar, print_preview, reader, window_controls};
use crate::webview_host::{
    build_body_document, supports_document, BodyView, DocumentAppearance, WebViewHost,
};
use kitemail_mail_core::verify::{check_sender_summary, SenderTrust};

pub(crate) struct MessageWindow {
    detail: MessageDetail,
    app_entity: WeakEntity<HylkiApp>,
    prefs: AppPreferences,
    prefer_plain: bool,
    expanded_quote_ids: HashSet<u32>,
    find_open: bool,
    find_input: Entity<InputState>,
    find_query: String,
    find_active_match: usize,
    find_total_matches: usize,
    text_states: HashMap<u32, (String, Entity<TextViewState>)>,
    cached_body: Option<crate::app::CachedReaderBody>,
    print_preview: Option<print_preview::PrintPreviewState>,
    webview: Option<Rc<WebViewHost>>,
    webview_appearance: Option<(DocumentAppearance, bool)>,
    toasts: WindowToastQueue,
    last_rem_scale: u32,
}

impl MessageWindow {
    fn observe_webview(&mut self, host: &Rc<WebViewHost>, cx: &mut Context<Self>) {
        let (tx, mut rx) = futures::channel::mpsc::channel::<()>(16);
        host.set_notifier(tx);
        cx.spawn(async move |this, cx| {
            use futures::StreamExt as _;
            let mut previous = None;
            while rx.next().await.is_some() {
                if this
                    .update(cx, |this, cx| {
                        let Some(host) = this.webview.as_ref() else {
                            return;
                        };
                        let status = Some(host.load_status());
                        if status != previous {
                            previous = status;
                            cx.notify();
                        }
                        let links = host.take_links();
                        if !this.prefer_plain && !this.find_open && this.print_preview.is_none() {
                            for link in links {
                                let _ = this
                                    .app_entity
                                    .update(cx, |app, cx| app.open_mail_link(&link, cx));
                            }
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    pub(crate) fn new(
        detail: MessageDetail,
        app_entity: WeakEntity<HylkiApp>,
        prefs: AppPreferences,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let initial_scale = config::clamp_ui_text_scale(prefs.ui_text_scale_percent);
        window.set_rem_size(px(16.0 * initial_scale as f32 / 100.0));

        let msg_id = detail.id;
        let app_for_close = app_entity.clone();
        let this_handle = window.window_handle();
        window.on_window_should_close(cx, move |_, cx| {
            let _ = app_for_close.update(cx, |app, _| {
                app.unregister_popout_message_window(msg_id, Some(this_handle));
            });
            true
        });

        let find_input = cx.new(|cx| InputState::new(window, cx).placeholder("在邮件中查找..."));
        cx.subscribe(
            &find_input,
            |this, input, event: &InputEvent, cx| match event {
                InputEvent::Change => {
                    this.find_query = input.read(cx).value().to_string();
                    this.find_active_match = 0;
                    this.refresh_find_highlights(cx);
                }
                InputEvent::PressEnter { secondary, shift } => {
                    this.step_find_match(if *secondary || *shift { -1 } else { 1 }, cx);
                }
                _ => {}
            },
        )
        .detach();

        cx.observe_global::<theme::HylkiThemeState>(|_, cx| cx.notify())
            .detach();
        if let Some(app) = app_entity.upgrade() {
            cx.observe(&app, |this, app, cx| {
                let app_ref = app.read(cx);
                let remote_changed = this.webview_appearance.is_some_and(|(_, allowed)| {
                    app_ref.remote_images_allowed(this.detail.id) != allowed
                });
                let next_prefs = &app_ref.prefs;
                let prefs_changed = this.prefs.reader_zoom_percent
                    != next_prefs.reader_zoom_percent
                    || this.prefs.message_theme != next_prefs.message_theme
                    || this.prefs.date_style != next_prefs.date_style
                    || this.prefs.clock_style != next_prefs.clock_style
                    || this.prefs.show_remote_banner != next_prefs.show_remote_banner
                    || this.prefs.clean_links != next_prefs.clean_links
                    || this.prefs.block_remote_images != next_prefs.block_remote_images
                    || this.prefs.ui_text_scale_percent != next_prefs.ui_text_scale_percent;
                let dark_changed = this.webview_appearance.is_some_and(|(appearance, _)| {
                    let forced_dark = match next_prefs.message_theme.as_str() {
                        "light" => Some(false),
                        "dark" => Some(true),
                        _ => None,
                    };
                    forced_dark.unwrap_or(app_ref.dark_mode) != appearance.dark
                });
                if prefs_changed {
                    this.prefs = next_prefs.clone();
                }
                if remote_changed || prefs_changed || dark_changed {
                    cx.notify();
                }
            })
            .detach();
        }

        Self {
            detail,
            app_entity,
            prefs,
            prefer_plain: false,
            expanded_quote_ids: HashSet::new(),
            find_open: false,
            find_input,
            find_query: String::new(),
            find_active_match: 0,
            find_total_matches: 0,
            text_states: HashMap::new(),
            cached_body: None,
            print_preview: None,
            webview: None,
            webview_appearance: None,
            toasts: WindowToastQueue::default(),
            last_rem_scale: initial_scale,
        }
    }

    fn push_toast(
        &mut self,
        key: impl Into<String>,
        kind: ToastKind,
        message: impl Into<SharedString>,
        action: Option<ToastAction>,
        cx: &mut Context<Self>,
    ) {
        self.toasts.push_transient(key, kind, message, action);
        if let Some(deadline) = self.toasts.next_expiry() {
            let wait = deadline.saturating_duration_since(std::time::Instant::now());
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(wait).await;
                let _ = this.update(cx, |win, cx| {
                    if win.toasts.prune_expired(std::time::Instant::now()) {
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        cx.notify();
    }

    fn toggle_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find_open = !self.find_open;
        if self.find_open {
            self.find_input
                .update(cx, |s, cx| s.focus_handle(cx).focus(window, cx));
            self.refresh_find_highlights(cx);
        } else {
            self.find_query.clear();
            self.find_total_matches = 0;
            self.find_active_match = 0;
            for (_, state) in self.text_states.values() {
                state.update(cx, |s, cx| s.clear_range_highlights(cx));
            }
        }
        cx.notify();
    }

    fn step_find_match(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.find_total_matches == 0 {
            return;
        }
        let len = self.find_total_matches as isize;
        self.find_active_match =
            ((self.find_active_match as isize + delta).rem_euclid(len)) as usize;
        self.refresh_find_highlights(cx);
    }

    fn refresh_find_highlights(&mut self, cx: &mut Context<Self>) {
        let query = self.find_query.trim().to_ascii_lowercase();
        if query.is_empty() {
            self.find_total_matches = 0;
            self.find_active_match = 0;
            for (_, state) in self.text_states.values() {
                state.update(cx, |s, cx| s.clear_range_highlights(cx));
            }
            cx.notify();
            return;
        }

        let mut all_matches: Vec<(Entity<TextViewState>, Vec<Range<usize>>)> = Vec::new();
        let mut total = 0usize;
        for (_, state) in self.text_states.values() {
            let rendered = state.read(cx).rendered_text();
            let ranges = find_case_insensitive_ranges(rendered.as_str(), &query);
            total += ranges.len();
            all_matches.push((state.clone(), ranges));
        }
        self.find_total_matches = total;
        if total == 0 {
            self.find_active_match = 0;
        } else if self.find_active_match >= total {
            self.find_active_match = 0;
        }

        let normal_bg = hsla(48.0 / 360.0, 0.96, 0.62, 0.45);
        let active_bg = hsla(28.0 / 360.0, 0.96, 0.56, 0.78);
        let mut cursor = 0usize;

        for (state, ranges) in all_matches {
            let mut highlights = Vec::with_capacity(ranges.len());
            let mut reveal: Option<Range<usize>> = None;
            for r in ranges {
                let is_active = total > 0 && cursor == self.find_active_match;
                if is_active {
                    reveal = Some(r.clone());
                }
                highlights.push(RangeHighlight::new(
                    r,
                    if is_active { active_bg } else { normal_bg },
                ));
                cursor += 1;
            }
            state.update(cx, |s, cx| {
                let _ = s.set_range_highlights(highlights, cx);
                if let Some(r) = reveal {
                    let _ = s.reveal_range(r, cx);
                }
            });
        }
        cx.notify();
    }

    fn ensure_text_state(
        &mut self,
        msg_id: u32,
        html: &str,
        cx: &mut Context<Self>,
    ) -> Entity<TextViewState> {
        if let Some((cached_html, state)) = self.text_states.get(&msg_id) {
            if cached_html == html {
                return state.clone();
            }
        }
        let state = cx.new(|cx| TextViewState::html(html, cx).selectable(true));
        self.text_states
            .insert(msg_id, (html.to_string(), state.clone()));
        state
    }
}

/// 在 UTF-8 字符串中按小写子串查找字节范围（大小写不敏感）。
pub(crate) fn find_case_insensitive_ranges(
    haystack: &str,
    needle_lower: &str,
) -> Vec<Range<usize>> {
    if needle_lower.is_empty() || haystack.is_empty() {
        return Vec::new();
    }
    let hay_lower = haystack.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut start = 0usize;
    while let Some(pos) = hay_lower[start..].find(needle_lower) {
        let abs = start + pos;
        let end = abs + needle_lower.len();
        if haystack.is_char_boundary(abs) && haystack.is_char_boundary(end) {
            out.push(abs..end);
        }
        start = end.max(start + 1);
    }
    out
}

impl Render for MessageWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let scale_percent = config::clamp_ui_text_scale(self.prefs.ui_text_scale_percent);
        if self.last_rem_scale != scale_percent {
            window.set_rem_size(px(16.0 * scale_percent as f32 / 100.0));
            self.last_rem_scale = scale_percent;
        }
        let p = theme::current(cx);
        let (canvas, border, muted_fg) = (p.canvas, p.border, p.text_muted);
        let d = self.detail.clone();
        let formatted_date = format_message_datetime(
            d.date_ts,
            &d.date_display,
            &self.prefs.date_style,
            &self.prefs.clock_style,
        );
        let (sender_name, sender_email) = split_sender(&d.sender);
        let display_name = if sender_name.is_empty() {
            d.sender.clone()
        } else {
            sender_name
        };

        let allow_remote = self
            .app_entity
            .upgrade()
            .is_some_and(|app| app.read(cx).remote_images_allowed(d.id));
        let app_for_link = self.app_entity.clone();
        let app_for_quote_link = self.app_entity.clone();
        let app_dark = self
            .app_entity
            .upgrade()
            .is_some_and(|app| app.read(cx).dark_mode);
        let forced_dark = match self.prefs.message_theme.as_str() {
            "light" => Some(false),
            "dark" => Some(true),
            _ => None,
        };
        let dark = forced_dark.unwrap_or(app_dark);
        let (background, foreground) = theme::reader_body_colors(dark);
        let body_muted = foreground.opacity(0.75);
        let appearance = DocumentAppearance {
            dark,
            zoom_percent: self.prefs.reader_zoom_percent,
            background,
            foreground,
        };
        let settings = Some((appearance, allow_remote));
        let mut load_scheduled = false;
        if self.webview_appearance != settings {
            self.webview_appearance = settings;
            if let Some(html) = d
                .body_html
                .as_deref()
                .filter(|html| supports_document(html))
            {
                let document = build_body_document(html, appearance);
                load_scheduled = true;
                cx.defer_in(window, move |this, window, cx| {
                    if this.webview_appearance != settings {
                        return;
                    }
                    if this
                        .webview
                        .as_ref()
                        .is_some_and(|host| host.retire_for_remote_reload(allow_remote))
                    {
                        this.webview = None;
                    }
                    if let Some(host) = &this.webview {
                        let _ = host.load_with_remote(document, allow_remote);
                    } else if let Some(host) = WebViewHost::for_window(window, None) {
                        let _ = host.load_with_remote(document, allow_remote);
                        let host = Rc::new(host);
                        this.observe_webview(&host, cx);
                        this.webview = Some(host);
                    }
                    cx.notify();
                });
            }
        }
        let body_view = BodyView::for_document(
            !self.prefer_plain && !self.find_open && self.print_preview.is_none(),
            load_scheduled,
            self.webview.as_deref(),
        );
        if let Some(host) = self.webview.clone().filter(|_| body_view != BodyView::Html) {
            cx.defer_in(window, move |_, _, _| host.set_visible(false));
        }

        let body_view_is_native = body_view == BodyView::Native;
        let need_blocked_images =
            body_view_is_native || (self.prefs.show_remote_banner && self.print_preview.is_none());
        let block_remote_images = !allow_remote;
        let clean_links = self.prefs.clean_links;
        let (main_html, quoted_opt, blocked_images): (String, Option<(String, usize)>, usize) =
            if !need_blocked_images {
                (String::new(), None, 0)
            } else if let Some(cached) = self.cached_body.as_ref().filter(|c| {
                c.body_view_is_native == body_view_is_native
                    && !c.show_raw_source
                    && c.prefer_plain == self.prefer_plain
                    && c.block_remote_images == block_remote_images
                    && c.clean_links == clean_links
                    && c.raw_html == d.body_html
                    && c.body_text == d.body_text
            }) {
                (
                    cached.main_html.clone(),
                    cached.quoted_part.clone(),
                    cached.blocked_images,
                )
            } else {
                let computed = if !self.prefer_plain {
                    if let Some(raw_html) = d.body_html.as_deref().filter(|s| !s.trim().is_empty())
                    {
                        let (clean, blocked) = sanitize_html_for_display_with_options(
                            raw_html,
                            block_remote_images,
                            clean_links,
                        );
                        if body_view_is_native {
                            let split = split_quoted_html(&clean);
                            (split.main, split.quoted, blocked)
                        } else {
                            (String::new(), None, blocked)
                        }
                    } else if body_view_is_native {
                        let split = split_quoted_plain(&d.body_text);
                        (
                            plain_text_to_html(&split.main),
                            split
                                .quoted
                                .map(|(q, lines)| (plain_text_to_html(&q), lines)),
                            0,
                        )
                    } else {
                        (String::new(), None, 0)
                    }
                } else if body_view_is_native {
                    let split = split_quoted_plain(&d.body_text);
                    (
                        plain_text_to_html(&split.main),
                        split
                            .quoted
                            .map(|(q, lines)| (plain_text_to_html(&q), lines)),
                        0,
                    )
                } else {
                    (String::new(), None, 0)
                };
                self.cached_body = Some(crate::app::CachedReaderBody {
                    body_view_is_native,
                    show_raw_source: false,
                    prefer_plain: self.prefer_plain,
                    block_remote_images,
                    clean_links,
                    raw_html: d.body_html.clone(),
                    body_text: d.body_text.clone(),
                    main_html: computed.0.clone(),
                    quoted_part: computed.1.clone(),
                    blocked_images: computed.2,
                });
                computed
            };

        let quote_expanded = self.expanded_quote_ids.contains(&d.id);
        let text_state = body_view_is_native.then(|| self.ensure_text_state(d.id, &main_html, cx));
        let win_controls =
            window_controls::render_standalone_window_controls("popout-wc", window, cx);
        let d_reply = d.clone();
        let d_reply_all = d.clone();
        let d_fwd = d.clone();
        let d_star = d.clone();
        let d_print = d.clone();

        v_flex()
            .size_full()
            .bg(canvas)
            .text_color(p.text)
            .on_key_down(
                cx.listener(|this, event: &gpui_kit::KeyDownEvent, window, cx| {
                    if event.keystroke.modifiers.control
                        && event.keystroke.key.eq_ignore_ascii_case("f")
                    {
                        this.toggle_find(window, cx);
                    } else if event.keystroke.key == "escape" && this.find_open {
                        this.toggle_find(window, cx);
                    } else if event.keystroke.key == "escape" && this.print_preview.is_some() {
                        this.print_preview = None;
                        cx.notify();
                    }
                }),
            )
            // 顶部工具栏。
            .child(
                h_flex()
                    .h(px(48.0))
                    .flex_shrink_0()
                    .pl_3()
                    .gap_1()
                    .items_center()
                    .border_b_1()
                    .border_color(border)
                    .child(
                        Button::new("popout-reply")
                            .ghost()
                            .compact()
                            .icon(IconName::Reply)
                            .tooltip("回复")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let d = d_reply.clone();
                                let started = this
                                    .app_entity
                                    .update(cx, |app, cx| {
                                        app.start_popout_compose_from_message(
                                            d,
                                            PopoutReplyKind::Reply,
                                            cx,
                                        )
                                    })
                                    .unwrap_or(false);
                                if started {
                                    window.remove_window();
                                } else {
                                    this.push_toast(
                                        "popout-compose-busy",
                                        ToastKind::Warning,
                                        "当前已有正在编辑的邮件，请先完成或保存后再回复",
                                        Some(ToastAction {
                                            label: "回到当前草稿".into(),
                                            primary: true,
                                            kind: ToastActionKind::FocusActiveCompose,
                                        }),
                                        cx,
                                    );
                                }
                            })),
                    )
                    .child(
                        Button::new("popout-reply-all")
                            .ghost()
                            .compact()
                            .icon(IconName::ReplyAll)
                            .tooltip("回复全部")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let d = d_reply_all.clone();
                                let started = this
                                    .app_entity
                                    .update(cx, |app, cx| {
                                        app.start_popout_compose_from_message(
                                            d,
                                            PopoutReplyKind::ReplyAll,
                                            cx,
                                        )
                                    })
                                    .unwrap_or(false);
                                if started {
                                    window.remove_window();
                                } else {
                                    this.push_toast(
                                        "popout-compose-busy",
                                        ToastKind::Warning,
                                        "当前已有正在编辑的邮件，请先完成或保存后再回复",
                                        Some(ToastAction {
                                            label: "回到当前草稿".into(),
                                            primary: true,
                                            kind: ToastActionKind::FocusActiveCompose,
                                        }),
                                        cx,
                                    );
                                }
                            })),
                    )
                    .child(
                        Button::new("popout-forward")
                            .ghost()
                            .compact()
                            .icon(IconName::Forward)
                            .tooltip("转发")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let d = d_fwd.clone();
                                let started = this
                                    .app_entity
                                    .update(cx, |app, cx| {
                                        app.start_popout_compose_from_message(
                                            d,
                                            PopoutReplyKind::Forward,
                                            cx,
                                        )
                                    })
                                    .unwrap_or(false);
                                if started {
                                    window.remove_window();
                                } else {
                                    this.push_toast(
                                        "popout-compose-busy",
                                        ToastKind::Warning,
                                        "当前已有正在编辑的邮件，请先完成或保存后再回复",
                                        Some(ToastAction {
                                            label: "回到当前草稿".into(),
                                            primary: true,
                                            kind: ToastActionKind::FocusActiveCompose,
                                        }),
                                        cx,
                                    );
                                }
                            })),
                    )
                    .child({
                        let app_w = self.app_entity.clone();
                        let flagged = d.flagged;
                        Button::new("popout-star")
                            .ghost()
                            .compact()
                            .icon(if flagged {
                                IconName::StarFill
                            } else {
                                IconName::Star
                            })
                            .when(flagged, |b| b.text_color(theme::star_gold()))
                            .tooltip("星标")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.detail.flagged = !this.detail.flagged;
                                let d = d_star.clone();
                                let _ = app_w.update(cx, |app, cx| {
                                    app.toggle_flagged(&d, cx);
                                });
                                cx.notify();
                            }))
                    })
                    .child(window_controls::drag_spacer())
                    .child(
                        Button::new("popout-find")
                            .ghost()
                            .compact()
                            .icon(IconName::Search)
                            .selected(self.find_open)
                            .tooltip("页内查找 (Ctrl+F)")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_find(window, cx);
                            })),
                    )
                    .child(
                        Button::new("popout-print")
                            .ghost()
                            .compact()
                            .icon(IconName::FileText)
                            .tooltip("打印 / 导出 PDF")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let pp = print_preview::PrintPreviewState::new(d_print.clone());
                                let date_style = this.prefs.date_style.clone();
                                let clock_style = this.prefs.clock_style.clone();
                                cx.background_executor()
                                    .spawn(async move {
                                        let _ = print_preview::launch_system_print(
                                            &pp,
                                            &date_style,
                                            &clock_style,
                                        );
                                    })
                                    .detach();
                            })),
                    )
                    .child(
                        Button::new("popout-plain")
                            .ghost()
                            .compact()
                            .icon(IconName::BookText)
                            .selected(self.prefer_plain)
                            .tooltip("阅读视图")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.prefer_plain = !this.prefer_plain;
                                cx.notify();
                            })),
                    )
                    .child(win_controls),
            )
            // 页内查找条。
            .when(self.find_open, |root| {
                let match_text = if self.find_query.trim().is_empty() {
                    String::new()
                } else if self.find_total_matches == 0 {
                    "无匹配".to_string()
                } else {
                    format!(
                        "{} / {}",
                        self.find_active_match + 1,
                        self.find_total_matches
                    )
                };
                root.child(
                    h_flex()
                        .px_4()
                        .py_1p5()
                        .gap_2()
                        .items_center()
                        .bg(p.surface_raised)
                        .border_b_1()
                        .border_color(border)
                        .child(
                            div()
                                .w(px(260.0))
                                .child(Input::new(&self.find_input).small().cleanable(true)),
                        )
                        .when(!match_text.is_empty(), |bar| {
                            bar.child(
                                div()
                                    .text_xs()
                                    .text_color(muted_fg)
                                    .child(SharedString::from(match_text)),
                            )
                        })
                        .child(
                            Button::new("popout-find-prev")
                                .ghost()
                                .compact()
                                .icon(IconName::ChevronUp)
                                .disabled(self.find_total_matches == 0)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.step_find_match(-1, cx);
                                })),
                        )
                        .child(
                            Button::new("popout-find-next")
                                .ghost()
                                .compact()
                                .icon(IconName::ChevronDown)
                                .disabled(self.find_total_matches == 0)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.step_find_match(1, cx);
                                })),
                        )
                        .child(
                            Button::new("popout-find-close")
                                .ghost()
                                .compact()
                                .icon(IconName::X)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.toggle_find(window, cx);
                                })),
                        ),
                )
            })
            .when(blocked_images > 0 && self.prefs.show_remote_banner && self.print_preview.is_none(), |root| {
                let id = d.id;
                root.child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("popout-allow-remote")
                                .outline()
                                .compact()
                                .label("显示本次图片")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let _ = this.app_entity.update(cx, |app, cx| app.allow_remote_images_for(id, cx));
                                    cx.notify();
                                })),
                        ),
                )
            })
            // 邮件头部信息区：主题、发件人头像/姓名/邮箱/认证状态/时间、收件人/抄送、附件列表。
            .child({
                let sender_check = check_sender_summary(&d.sender, d.auth_results.as_deref());
                let trust_badge = match sender_check.trust {
                    SenderTrust::Pass => Some(("已验证", hsla(0.38, 0.65, 0.36, 1.0))),
                    SenderTrust::Suspicious => Some(("疑似仿冒", hsla(0.10, 0.85, 0.45, 1.0))),
                    SenderTrust::Fail => Some(("校验失败", p.danger)),
                    SenderTrust::Unverified => None,
                };
                let app_for_att = self.app_entity.clone();
                let has_recipients = !d.recipients.trim().is_empty();
                let has_cc = !d.cc.trim().is_empty();
                let has_att = !d.attachments.is_empty();

                v_flex()
                    .flex_shrink_0()
                    .px_6()
                    .pt_4()
                    .pb_3()
                    .gap_3()
                    .bg(p.surface_raised)
                    .border_b_1()
                    .border_color(border)
                    .child(
                        div()
                            .text_size(px(20.0))
                            .font_weight(gpui_kit::FontWeight::BOLD)
                            .text_color(p.text)
                            .child(SharedString::from(if d.subject.trim().is_empty() {
                                "(无主题)".to_string()
                            } else {
                                d.subject.clone()
                            })),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .gap_3()
                            .child(avatar::avatar(&display_name, &sender_email, 34.0))
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        h_flex()
                                            .min_w_0()
                                            .gap_2()
                                            .items_center()
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .font_weight(gpui_kit::FontWeight::BOLD)
                                                    .text_color(p.text)
                                                    .truncate()
                                                    .child(SharedString::from(display_name)),
                                            )
                                            .when(!sender_email.is_empty(), |el| {
                                                el.child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(muted_fg)
                                                        .truncate()
                                                        .child(SharedString::from(format!(
                                                            "<{sender_email}>"
                                                        ))),
                                                )
                                            })
                                            .when_some(trust_badge, |el, (label, color)| {
                                                el.child(
                                                    div()
                                                        .flex_shrink_0()
                                                        .px_2()
                                                        .py_0p5()
                                                        .rounded_full()
                                                        .bg(canvas)
                                                        .border_1()
                                                        .border_color(color.opacity(0.45))
                                                        .text_color(color)
                                                        .text_xs()
                                                        .child(SharedString::from(label)),
                                                )
                                            }),
                                    )
                                    .when(has_recipients || has_cc, |el| {
                                        let d_recip = d.recipients.trim().to_string();
                                        let d_cc = d.cc.trim().to_string();
                                        el.child(
                                            v_flex()
                                                .gap_0p5()
                                                .text_xs()
                                                .text_color(muted_fg)
                                                .when(has_recipients, |row| {
                                                    row.child(div().truncate().child(SharedString::from(format!(
                                                        "收件人：{d_recip}"
                                                    ))))
                                                })
                                                .when(has_cc, |row| {
                                                    row.child(div().truncate().child(SharedString::from(format!(
                                                        "抄送：{d_cc}"
                                                    ))))
                                                }),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_xs()
                                    .text_color(muted_fg)
                                    .child(SharedString::from(formatted_date)),
                            ),
                    )
                    .when(has_att, |el| {
                        el.child(
                            div()
                                .id("popout-att-list")
                                .max_h(px(120.0))
                                .overflow_y_scrollbar()
                                .child(
                                    h_flex()
                                        .flex_wrap()
                                        .gap_2()
                                        .children(d.attachments.iter().map(|att| {
                                            let att_id = att.id;
                                            let filename = att.filename.clone();
                                            let size_text = kitemail_mail_core::config::format_bytes(
                                                att.size_bytes as u64,
                                            );
                                            let size_label = if att.downloaded {
                                                size_text
                                            } else {
                                                format!("约 {size_text} · 未下载")
                                            };
                                            let save_tooltip = if att.downloaded {
                                                "保存附件"
                                            } else {
                                                "下载并保存附件"
                                            };
                                            let app_w = app_for_att.clone();
                                            h_flex()
                                                .items_center()
                                                .gap_2()
                                                .px_2()
                                                .py_1()
                                                .rounded(px(8.0))
                                                .border_1()
                                                .border_color(border)
                                                .bg(canvas)
                                                .child(
                                                    Icon::new(IconName::Paperclip)
                                                        .with_size(px(14.0))
                                                        .text_color(muted_fg),
                                                )
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .max_w(px(200.0))
                                                        .truncate()
                                                        .child(SharedString::from(filename.clone())),
                                                )
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(muted_fg)
                                                        .child(SharedString::from(size_label)),
                                                )
                                                .child(
                                                    Button::new(("popout-save-att", att_id))
                                                        .ghost()
                                                        .compact()
                                                        .icon(IconName::Download)
                                                        .tooltip(save_tooltip)
                                                        .on_click(move |_, _, cx| {
                                                            let fn_clone = filename.clone();
                                                            let _ = app_w.update(cx, |app, cx| {
                                                                app.save_attachment(att_id, &fn_clone, cx);
                                                            });
                                                        }),
                                                )
                                        })),
                                ),
                        )
                    })
            })
            // 邮件正文滚动区。
            .when(body_view == BodyView::Loading, |root| {
                root.child(reader::body_loading(background, body_muted, false).flex_1().min_h_0())
            })
            .when(body_view == BodyView::Html, |root| {
                let host = self.webview.as_ref().unwrap().clone();
                let (version, _) = host.load_status();
                root.child(
                    gpui_kit::canvas(
                        move |bounds, window, cx| {
                            let scale = window.scale_factor();
                            let rect = windows::Win32::Foundation::RECT {
                                left: (bounds.origin.x.as_f32() * scale).round() as i32,
                                top: (bounds.origin.y.as_f32() * scale).round() as i32,
                                right: ((bounds.origin.x + bounds.size.width).as_f32() * scale)
                                    .round() as i32,
                                bottom: ((bounds.origin.y + bounds.size.height).as_f32() * scale)
                                    .round() as i32,
                            };
                            cx.defer(move |_| {
                                if host.load_status() == (version, BodyView::Html) {
                                    host.set_bounds(rect);
                                    host.set_visible(true);
                                }
                            });
                        },
                        |_, _, _, _| {},
                    )
                    .flex_1()
                    .min_h_0()
                    .w_full(),
                )
            })
            .when_some(text_state, |root, text_state| {
                root.child(
                    v_flex()
                        .id("popout-scroll")
                        .flex_1()
                        .min_h_0()
                        .bg(background)
                        .text_color(foreground)
                        .overflow_y_scrollbar()
                        .p_6()
                        .gap_4()
                        .child(
                            v_flex()
                                .p_5()
                                .gap_3()
                                .rounded(px(12.0))
                                .border_1()
                                .border_color(border)
                                .child(div().text_size(px(14.0)).child(
                                    TextView::new(&text_state)
                                        .image_source(move |uri| crate::webview_host::mail_image_source(uri, allow_remote))
                                        .on_link_click(move |url, event, _, cx| HylkiApp::open_clicked_mail_link(&app_for_link, url, event, cx))
                                        .link_tooltip(|url, _, cx| {
                                        let url = url.clone();
                                        cx.new(move |_| LinkPreviewTooltip { url: url.clone() })
                                            .into()
                                    }),
                                ))
                                .when_some(quoted_opt, |card, (q_html, lines)| {
                                    let msg_id = d.id;
                                    card.child(
                                        v_flex()
                                            .gap_2()
                                            .child(
                                                Button::new("popout-quote-toggle")
                                                    .outline()
                                                    .compact()
                                                    .label(SharedString::from(if quote_expanded {
                                                        format!("▾ 隐藏引用内容 ({lines} 行)")
                                                    } else {
                                                        format!("⋯ 显示引用内容 ({lines} 行)")
                                                    }))
                                                    .on_click(cx.listener(
                                                        move |this, _, _, cx| {
                                                            if !this
                                                                .expanded_quote_ids
                                                                .remove(&msg_id)
                                                            {
                                                                this.expanded_quote_ids
                                                                    .insert(msg_id);
                                                            }
                                                            cx.notify();
                                                        },
                                                    )),
                                            )
                                            .when(quote_expanded, |c| {
                                                c.child(
                                                    div()
                                                        .pl_3()
                                                        .border_l_2()
                                                        .border_color(p.accent.opacity(0.5))
                                                        .text_color(body_muted)
                                                        .text_size(px(13.0))
                                                        .child(
                                                            TextView::html(
                                                                SharedString::from(format!(
                                                                    "popout-quote-{msg_id}"
                                                                )),
                                                                q_html,
                                                            )
                                                            .selectable(true)
                                                            .image_source(move |uri| crate::webview_host::mail_image_source(uri, allow_remote))
                                                            .on_link_click(move |url, event, _, cx| HylkiApp::open_clicked_mail_link(&app_for_quote_link, url, event, cx)),
                                                        ),
                                                )
                                            }),
                                    )
                                }),
                        ),
                )
            })
            .child({
                // 固定底部 GPUI 反馈区：确保 WebView2 视口始终止于此区上方，Toast 可见可点击且不改变正文高度。
                let strip_h = toast::feedback_strip_height(self.prefs.ui_text_scale_percent);
                let visible = self.toasts.visible();
                let entity_act = cx.entity();
                let entity_dis = cx.entity();
                let app_act = self.app_entity.clone();
                let cards = toast::render_toast_stack(
                    &visible,
                    false,
                    &p,
                    move |kind, key, window, cx| {
                        let _ = entity_act.update(cx, |win, cx| {
                            win.toasts.dismiss_by_key(&key);
                            if matches!(kind, ToastActionKind::FocusActiveCompose) {
                                let _ = app_act.update(cx, |app, cx| {
                                    app.focus_active_compose_window(window, cx);
                                });
                            }
                            cx.notify();
                        });
                    },
                    move |key, _, cx| {
                        let _ = entity_dis.update(cx, |win, cx| {
                            win.toasts.dismiss_by_key(&key);
                            cx.notify();
                        });
                    },
                );
                h_flex()
                    .h(px(strip_h))
                    .flex_shrink_0()
                    .w_full()
                    .px_4()
                    .items_center()
                    .justify_center()
                    .bg(canvas)
                    .when(!cards.is_empty(), |strip| {
                        strip.child(v_flex().gap_1().items_center().children(cards))
                    })
            })
    }
}

/// `.link-preview` 链接悬停预览牌视图（对齐 `src/styles.css:1747-1754`）。
pub(crate) struct LinkPreviewTooltip {
    pub(crate) url: SharedString,
}

impl Render for LinkPreviewTooltip {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .bg(black().opacity(0.84))
            .text_color(hsla(0.0, 0.0, 1.0, 1.0))
            .px_2p5()
            .py_1()
            .rounded(px(6.0))
            .text_xs()
            .max_w(px(480.0))
            .truncate()
            .child(self.url.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_case_insensitive_ranges_finds_all_occurrences() {
        let ranges = find_case_insensitive_ranges("Hello World, hello hylki!", "hello");
        assert_eq!(ranges, vec![0..5, 13..18]);
        assert!(find_case_insensitive_ranges("abc", "").is_empty());
    }
}
