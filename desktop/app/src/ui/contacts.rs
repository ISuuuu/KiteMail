//! 联系人页（方案文档第三节页面 7）：
//! - 左栏：搜索框（变更发 [`ContactsEvent::QueryChanged`]）+ 联系人列表
//!   （姓名 + 邮箱 + 往来邮件数胶囊，按邮件数降序展示）。
//! - 右栏：选中联系人卡片（头像、姓名、邮箱、往来邮件数、
//!   「给 TA 写信」按钮发 [`ContactsEvent::ComposeTo`]）。
//!
//! 自包含组件：数据由集成者通过 [`ContactsState::set_contacts`] 注入，
//! 组件内部按搜索词做客户端过滤以便即时反馈。

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::Icon;
use gpui_kit::component::{h_flex, v_flex, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    div, px, AppContext, Context, Div, EventEmitter, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, Stateful, StatefulInteractiveElement, Styled,
};
use hylki_mail_core::cache::KnownContact;

use crate::ui::avatar;

/// 左栏宽度。
const LEFT_WIDTH: f32 = 300.0;
/// 联系人行高度。
const ROW_HEIGHT: f32 = 56.0;

/// 联系人页事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContactsEvent {
    /// 搜索框文本变化（完整当前值）。
    QueryChanged(String),
    /// 请求给指定邮箱写信。
    ComposeTo(String),
}

/// 联系人视图状态（独立 Entity，由 HylkiApp 挂载）。
pub struct ContactsState {
    search_input: gpui_kit::Entity<InputState>,
    /// 当前搜索词。
    query: String,
    /// 集成者注入的联系人（set_contacts 时按邮件数降序排列）。
    contacts: Vec<KnownContact>,
    /// 当前选中的联系人下标（指向过滤后的列表）。
    selected: usize,
}

impl EventEmitter<ContactsEvent> for ContactsState {}

impl ContactsState {
    pub fn new(window: &mut gpui_kit::Window, cx: &mut Context<Self>) -> Self {
        let search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("搜索姓名或邮箱..."));
        cx.subscribe(&search_input, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let query = input.read(cx).value().to_string();
                this.query = query.clone();
                this.selected = 0;
                cx.emit(ContactsEvent::QueryChanged(query));
                cx.notify();
            }
        })
        .detach();
        Self {
            search_input,
            query: String::new(),
            contacts: Vec::new(),
            selected: 0,
        }
    }

    /// 注入联系人列表；内部按往来邮件数降序排序。
    pub fn set_contacts(&mut self, items: Vec<KnownContact>) {
        self.contacts = sort_contacts_desc(items);
        self.selected = 0;
    }

    /// 过滤后的联系人（搜索词匹配姓名或邮箱，忽略大小写）。
    fn visible_contacts(&self) -> Vec<&KnownContact> {
        self.contacts
            .iter()
            .filter(|c| contact_matches(c, &self.query))
            .collect()
    }
}

/// 按往来邮件数降序排序（同数按姓名升序稳定次序）。
fn sort_contacts_desc(mut items: Vec<KnownContact>) -> Vec<KnownContact> {
    items.sort_by(|a, b| {
        b.message_count
            .cmp(&a.message_count)
            .then_with(|| a.name.cmp(&b.name))
    });
    items
}

/// 搜索词匹配：空串匹配全部；否则忽略大小写的子串匹配姓名或邮箱。
fn contact_matches(contact: &KnownContact, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty()
        || contact.name.to_lowercase().contains(&q)
        || contact.email.to_lowercase().contains(&q)
}

/// 联系人行（头像 + 姓名/邮箱 + 邮件数胶囊）。
fn contact_row(
    contact: &KnownContact,
    is_selected: bool,
    p: &crate::theme::HylkiPalette,
) -> Stateful<Div> {
    let (text, muted_fg, accent, accent_fg, accent_soft, border) = (
        p.text,
        p.text_muted,
        p.accent,
        p.accent_fg,
        p.accent_soft(),
        p.border,
    );
    let count_text = contact.message_count.to_string();
    h_flex()
        .id(SharedString::from(format!("contact-{}", contact.email)))
        .h(px(ROW_HEIGHT))
        .mx_2()
        .px_2()
        .gap_2()
        .items_center()
        .rounded(px(8.0))
        .cursor_pointer()
        .hover(move |s| s.bg(accent_soft))
        .when(is_selected, |row| {
            row.bg(accent_soft).border_1().border_color(accent)
        })
        .when(!is_selected, |row| row.border_1().border_color(border))
        .child(avatar::avatar(&contact.name, &contact.email, 32.0))
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(
                    div()
                        .text_sm()
                        .font_weight(if is_selected {
                            gpui_kit::FontWeight::SEMIBOLD
                        } else {
                            gpui_kit::FontWeight::MEDIUM
                        })
                        .text_color(text)
                        .truncate()
                        .child(SharedString::from(contact.name.clone())),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted_fg)
                        .truncate()
                        .child(SharedString::from(contact.email.clone())),
                ),
        )
        .child(
            div()
                .rounded_full()
                .px_1p5()
                .h(px(18.0))
                .min_w(px(20.0))
                .flex_shrink_0()
                .flex()
                .items_center()
                .justify_center()
                .text_xs()
                .when(is_selected, |pill| pill.bg(accent).text_color(accent_fg))
                .when(!is_selected, |pill| {
                    pill.bg(accent_soft).text_color(muted_fg)
                })
                .child(SharedString::from(count_text)),
        )
}

impl Render for ContactsState {
    fn render(
        &mut self,
        _window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // 避坑清单第 4 条：render 开头先把语义色 Copy 成局部变量。
        let p = crate::theme::current(cx);
        let (canvas, surface, border, text, muted_fg) =
            (p.canvas, p.surface, p.border, p.text, p.text_muted);

        let visible = self.visible_contacts();
        let selected = if visible.is_empty() {
            0
        } else {
            self.selected.min(visible.len() - 1)
        };
        let selected_contact = visible.get(selected).cloned();
        let visible_len = visible.len();

        let rows: Vec<Stateful<Div>> = visible
            .iter()
            .enumerate()
            .map(|(ix, contact)| {
                contact_row(contact, ix == selected, &p).on_click(cx.listener(
                    move |this, _, _, cx| {
                        // 选中该联系人（下标指向当前过滤后的列表）。
                        this.selected = ix;
                        cx.notify();
                    },
                ))
            })
            .collect();

        let detail_card =
            match selected_contact {
                None => v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .child(
                        Icon::new(IconName::CircleUser)
                            .with_size(px(28.0))
                            .text_color(muted_fg),
                    )
                    .child(div().text_sm().text_color(muted_fg).child(
                        if self.contacts.is_empty() {
                            "暂无联系人（同步邮件后自动聚合）"
                        } else {
                            "无匹配的联系人"
                        },
                    )),
                Some(contact) => {
                    let name = contact.name.clone();
                    let email = contact.email.clone();
                    let count = contact.message_count;
                    let compose_email = email.clone();
                    v_flex()
                        .w(px(420.0))
                        .px_4()
                        .py_5()
                        .gap_3()
                        .bg(surface)
                        .rounded(px(12.0))
                        .border_1()
                        .border_color(border)
                        .items_center()
                        .child(avatar::avatar(&name, &email, 56.0))
                        .child(
                            div()
                                .text_base()
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .text_color(text)
                                .child(SharedString::from(name)),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(muted_fg)
                                .child(SharedString::from(email)),
                        )
                        .child(
                            h_flex()
                                .gap_1()
                                .items_center()
                                .child(
                                    Icon::new(IconName::Mail)
                                        .with_size(px(14.0))
                                        .text_color(muted_fg),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(muted_fg)
                                        .child(SharedString::from(format!("往来邮件 {count} 封"))),
                                ),
                        )
                        .child(
                            Button::new("contact-compose")
                                .primary()
                                .label("给 TA 写信")
                                .icon(IconName::Mail)
                                .rounded(px(8.0))
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(ContactsEvent::ComposeTo(compose_email.clone()));
                                    cx.notify();
                                })),
                        )
                }
            };

        v_flex().size_full().bg(canvas).child(
            h_flex()
                .flex_1()
                .min_h_0()
                // 左栏：搜索框 + 联系人列表。
                .child(
                    v_flex()
                        .w(px(LEFT_WIDTH))
                        .h_full()
                        .flex_shrink_0()
                        .bg(surface)
                        .border_r_1()
                        .border_color(border)
                        .child(
                            h_flex()
                                .h(px(48.0))
                                .flex_shrink_0()
                                .px_2()
                                .gap_2()
                                .items_center()
                                .border_b_1()
                                .border_color(border)
                                .child(
                                    Icon::new(IconName::Users)
                                        .with_size(px(16.0))
                                        .text_color(muted_fg),
                                )
                                .child(
                                    div()
                                        .text_base()
                                        .font_weight(gpui_kit::FontWeight::BOLD)
                                        .text_color(text)
                                        .child("联系人"),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .text_xs()
                                        .text_color(muted_fg)
                                        .child(SharedString::from(format!("{visible_len} 人"))),
                                ),
                        )
                        .child(
                            div()
                                .px_2()
                                .py_2()
                                .flex_shrink_0()
                                .child(Input::new(&self.search_input)),
                        )
                        .child(
                            v_flex()
                                .id("contacts-list")
                                .flex_1()
                                .min_h_0()
                                .overflow_y_scrollbar()
                                .py_1()
                                .when(visible.is_empty(), |el| {
                                    el.child(
                                        div().px_3().py_2().text_xs().text_color(muted_fg).child(
                                            if self.contacts.is_empty() {
                                                "暂无联系人"
                                            } else {
                                                "无匹配的联系人"
                                            },
                                        ),
                                    )
                                })
                                .children(rows),
                        ),
                )
                // 右栏：选中联系人卡片。
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .items_center()
                        .justify_center()
                        .child(detail_card),
                ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contact(name: &str, email: &str, count: usize) -> KnownContact {
        KnownContact {
            name: name.to_string(),
            email: email.to_string(),
            message_count: count,
        }
    }

    #[test]
    fn contact_matches_is_case_insensitive_substring() {
        let c = contact("Marcus Chen", "Marcus@Studio.dev", 3);
        assert!(contact_matches(&c, ""));
        assert!(contact_matches(&c, "marcus"));
        assert!(contact_matches(&c, "  CHEN "));
        assert!(contact_matches(&c, "studio.dev"));
        assert!(!contact_matches(&c, "zhang"));
    }

    #[test]
    fn sort_contacts_desc_orders_by_message_count() {
        let sorted = sort_contacts_desc(vec![
            contact("低", "low@example.com", 1),
            contact("高", "high@example.com", 9),
            contact("中", "mid@example.com", 4),
            contact("同数", "tie@example.com", 4),
        ]);
        let names: Vec<&str> = sorted.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["高", "中", "同数", "低"]);
    }
}
