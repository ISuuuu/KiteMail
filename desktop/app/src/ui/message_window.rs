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
use gpui_kit::component::{h_flex, v_flex, Disableable, Selectable, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    black, div, hsla, px, AppContext, Context, Entity, Focusable, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, Styled, WeakEntity, Window,
};
use hylki_mail_core::config::AppPreferences;
use hylki_mail_core::imap_client::{plain_text_to_html, sanitize_html_for_display_with_options};
use hylki_mail_core::mime::{split_quoted_html, split_quoted_plain};
use hylki_mail_core::{format_message_datetime, MessageDetail};

use crate::app::{split_sender, HylkiApp};
use crate::theme;
use crate::ui::{avatar, print_preview, window_controls};
use crate::webview_host::{build_document, supports_document, WebViewHost};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

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
    print_preview: Option<print_preview::PrintPreviewState>,
    webview: Option<Rc<WebViewHost>>,
    webview_appearance: (bool, u32, bool),
}

impl MessageWindow {
    fn poll_webview(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let mut previous = (false, false);
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(100))
                    .await;
                if this
                    .update(cx, |this, cx| {
                        let Some(host) = this.webview.as_ref() else {
                            return;
                        };
                        let status = (host.ready(), host.failed());
                        if status != previous {
                            previous = status;
                            cx.notify();
                        }
                        let app_dark = this
                            .app_entity
                            .upgrade()
                            .is_some_and(|app| app.read(cx).dark_mode);
                        let dark = match this.prefs.message_theme.as_str() {
                            "light" => false,
                            "dark" => true,
                            _ => app_dark,
                        };
                        if dark != this.webview_appearance.0 {
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

        let eligible_html = detail
            .body_html
            .as_deref()
            .filter(|html| supports_document(html))
            .map(str::to_owned);
        let subject = detail.subject.clone();
        let sender = detail.sender.clone();
        let app_dark = app_entity
            .upgrade()
            .is_some_and(|app| app.read(cx).dark_mode);
        let dark_mode = match prefs.message_theme.as_str() {
            "light" => false,
            "dark" => true,
            _ => app_dark,
        };
        let zoom_percent = prefs.reader_zoom_percent;
        let allow_remote = app_entity.upgrade().is_some_and(|app| {
            cx.observe(&app, |this, app, cx| {
                if app.read(cx).remote_images_allowed(this.detail.id) != this.webview_appearance.2 {
                    cx.notify();
                }
            })
            .detach();
            app.read(cx).remote_images_allowed(detail.id)
        });
        if let Some(html) = eligible_html {
            cx.defer_in(window, move |this, window, cx| {
                if let Ok(handle) = window.window_handle() {
                    if let RawWindowHandle::Win32(handle) = handle.as_raw() {
                        let hwnd = windows::Win32::Foundation::HWND(handle.hwnd.get() as *mut _);
                        if let Ok(host) = WebViewHost::create(hwnd, None) {
                            let allow_remote = this.app_entity.upgrade().is_some_and(|app| {
                                app.read(cx).remote_images_allowed(this.detail.id)
                            });
                            if host
                                .load_with_remote(
                                    build_document(
                                        &html,
                                        &subject,
                                        &sender,
                                        dark_mode,
                                        zoom_percent,
                                    ),
                                    allow_remote,
                                )
                                .is_ok()
                            {
                                this.webview = Some(Rc::new(host));
                                this.poll_webview(cx);
                                cx.notify();
                            }
                        }
                    }
                }
            });
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
            print_preview: None,
            webview: None,
            webview_appearance: (dark_mode, zoom_percent, allow_remote),
        }
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
        let mut blocked_images = 0;
        let (main_html, quoted_opt) = if !self.prefer_plain {
            if let Some(ref raw_html) = d.body_html {
                if !raw_html.trim().is_empty() {
                    let (clean, blocked) = sanitize_html_for_display_with_options(
                        raw_html,
                        !allow_remote,
                        self.prefs.clean_links,
                    );
                    blocked_images = blocked;
                    let split = split_quoted_html(&clean);
                    (split.main, split.quoted.map(|(q, lines)| (q, lines)))
                } else {
                    let split = split_quoted_plain(&d.body_text);
                    (
                        plain_text_to_html(&split.main),
                        split
                            .quoted
                            .map(|(q, lines)| (plain_text_to_html(&q), lines)),
                    )
                }
            } else {
                let split = split_quoted_plain(&d.body_text);
                (
                    plain_text_to_html(&split.main),
                    split
                        .quoted
                        .map(|(q, lines)| (plain_text_to_html(&q), lines)),
                )
            }
        } else {
            let split = split_quoted_plain(&d.body_text);
            (
                plain_text_to_html(&split.main),
                split
                    .quoted
                    .map(|(q, lines)| (plain_text_to_html(&q), lines)),
            )
        };

        let text_state = self.ensure_text_state(d.id, &main_html, cx);
        let quote_expanded = self.expanded_quote_ids.contains(&d.id);
        let app_dark = self
            .app_entity
            .upgrade()
            .is_some_and(|app| app.read(cx).dark_mode);
        let dark_mode = match self.prefs.message_theme.as_str() {
            "light" => false,
            "dark" => true,
            _ => app_dark,
        };

        let appearance = (dark_mode, self.prefs.reader_zoom_percent, allow_remote);
        if self.webview_appearance != appearance {
            self.webview_appearance = appearance;
            if let (Some(host), Some(html)) = (self.webview.clone(), d.body_html.clone()) {
                let document =
                    build_document(&html, &d.subject, &d.sender, appearance.0, appearance.1);
                cx.defer_in(window, move |_, _, _| {
                    let _ = host.load_with_remote(document, appearance.2);
                });
            }
        }
        let show_webview = self.webview.as_ref().is_some_and(|host| host.ready())
            && !self.prefer_plain
            && !self.find_open
            && self.print_preview.is_none();
        if let Some(host) = self.webview.clone() {
            cx.defer_in(window, move |_, _, _| host.set_visible(show_webview));
        }
        let win_controls = h_flex()
            .flex_shrink_0()
            .h_full()
            .items_center()
            .child(
                Button::new("popout-win-min")
                    .ghost()
                    .compact()
                    .icon(IconName::Minus)
                    .on_click(|_, window, _| window.minimize_window()),
            )
            .child(
                Button::new("popout-win-max")
                    .ghost()
                    .compact()
                    .icon(IconName::Square)
                    .on_click(|_, window, _| window.zoom_window()),
            )
            .child(
                Button::new("popout-win-close")
                    .ghost()
                    .compact()
                    .icon(IconName::X)
                    .on_click(|_, window, _| window.remove_window()),
            );
        let app_weak = self.app_entity.clone();
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
                            .on_click(move |_, window, cx| {
                                let d = d_reply.clone();
                                let _ = app_weak.update(cx, |app, cx| {
                                    app.start_reply(&d, window, cx);
                                });
                            }),
                    )
                    .child({
                        let app_w = self.app_entity.clone();
                        Button::new("popout-reply-all")
                            .ghost()
                            .compact()
                            .icon(IconName::ReplyAll)
                            .tooltip("回复全部")
                            .on_click(move |_, window, cx| {
                                let d = d_reply_all.clone();
                                let _ = app_w.update(cx, |app, cx| {
                                    app.start_reply_all(&d, window, cx);
                                });
                            })
                    })
                    .child({
                        let app_w = self.app_entity.clone();
                        Button::new("popout-forward")
                            .ghost()
                            .compact()
                            .icon(IconName::Forward)
                            .tooltip("转发")
                            .on_click(move |_, window, cx| {
                                let d = d_fwd.clone();
                                let _ = app_w.update(cx, |app, cx| {
                                    app.start_forward(&d, window, cx);
                                });
                            })
                    })
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
                root.child(Button::new("popout-allow-remote").outline().compact().label("显示此邮件的远程图片")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let _ = this.app_entity.update(cx, |app, cx| app.allow_remote_images_for(id, cx));
                        cx.notify();
                    })))
            })
            // 邮件正文滚动区。
            .when(show_webview, |root| {
                let host = self.webview.as_ref().unwrap().clone();
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
                            cx.defer(move |_| host.set_bounds(rect));
                        },
                        |_, _, _, _| {},
                    )
                    .flex_1()
                    .min_h_0()
                    .w_full(),
                )
            })
            .when(!show_webview, |root| {
                root.child(
                    v_flex()
                        .id("popout-scroll")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scrollbar()
                        .p_6()
                        .gap_4()
                        .child(
                            div()
                                .text_size(px(20.0))
                                .font_weight(gpui_kit::FontWeight::BOLD)
                                .child(SharedString::from(d.subject.clone())),
                        )
                        .child(
                            v_flex()
                                .p_5()
                                .gap_3()
                                .rounded(px(12.0))
                                .bg(p.surface_raised)
                                .border_1()
                                .border_color(border)
                                .child(
                                    h_flex()
                                        .items_center()
                                        .gap_2()
                                        .child(avatar::avatar(&display_name, &sender_email, 34.0))
                                        .child(
                                            v_flex()
                                                .flex_1()
                                                .min_w_0()
                                                .child(
                                                    div()
                                                        .text_sm()
                                                        .font_weight(gpui_kit::FontWeight::BOLD)
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
                                                }),
                                        )
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(muted_fg)
                                                .child(SharedString::from(formatted_date)),
                                        ),
                                )
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
                                                        .text_color(muted_fg)
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
