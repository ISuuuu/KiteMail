//! 附件总览页（方案文档第三节页面 7）：
//! - 顶部栏：附件图标 + 标题 + 搜索框（即时过滤，变更发 [`GalleryEvent::QueryChanged`]）
//!   + 账号筛选下拉（变更发 [`GalleryEvent::AccountFilterChanged`]）。
//! - 下方紧凑表格：文件图标、文件名、大小（人类可读）、发件人、所属邮件主题、
//!   日期，以及「保存」「定位到原邮件」按钮（点击发对应事件）。
//!
//! 自包含组件：数据由集成者通过 [`GalleryState::set_items`] /
//! [`GalleryState::set_accounts`] 注入，交互通过 [`GalleryEvent`] 通知集成者，
//! 组件内部同时对当前数据做客户端过滤以便即时反馈。

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::Icon;
use gpui_kit::component::{h_flex, v_flex, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    div, px, AppContext, Context, EventEmitter, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, Stateful, StatefulInteractiveElement, Styled, WindowControlArea,
};
use hylki_mail_core::cache::CachedAttachment;

/// 顶部栏高度。
const HEADER_HEIGHT: f32 = 48.0;
/// 附件行高度。
const ROW_HEIGHT: f32 = 44.0;
/// 表格列宽（与表头、数据行共用）。
const COL_SENDER_W: f32 = 150.0;
const COL_SUBJECT_W: f32 = 200.0;
const COL_SIZE_W: f32 = 80.0;
const COL_DATE_W: f32 = 120.0;
const COL_ACTIONS_W: f32 = 210.0;

/// 附件总览事件：由集成者订阅并转换为缓存查询 / 文件保存 / 视图跳转。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GalleryEvent {
    /// 搜索框文本变化（完整当前值）。
    QueryChanged(String),
    /// 账号筛选变化（`None` 表示全部账号）。
    AccountFilterChanged(Option<u32>),
    /// 请求保存附件（参数为附件 id）。
    SaveAttachment(u32),
    /// 请求定位到附件所属原邮件（参数为邮件 id）。
    LocateMessage(u32),
}

/// 附件总览视图状态（独立 Entity，由 HylkiApp 挂载）。
pub struct GalleryState {
    search_input: gpui_kit::Entity<InputState>,
    /// 当前搜索词（客户端过滤 + 供集成者展示）。
    query: String,
    /// 当前账号筛选。
    account_filter: Option<u32>,
    /// 集成者注入的账号列表 `(account_id, 显示名)`。
    accounts: Vec<(u32, String)>,
    /// 集成者注入的附件列表。
    items: Vec<CachedAttachment>,
}

impl EventEmitter<GalleryEvent> for GalleryState {}

impl GalleryState {
    pub fn new(window: &mut gpui_kit::Window, cx: &mut Context<Self>) -> Self {
        let search_input = cx
            .new(|cx| InputState::new(window, cx).placeholder("搜索文件名 / 发件人 / 邮件主题..."));
        cx.subscribe(&search_input, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let query = input.read(cx).value().to_string();
                this.query = query.clone();
                cx.emit(GalleryEvent::QueryChanged(query));
                cx.notify();
            }
        })
        .detach();
        Self {
            search_input,
            query: String::new(),
            account_filter: None,
            accounts: Vec::new(),
            items: Vec::new(),
        }
    }

    /// 注入附件数据（集成者在收到缓存查询结果后调用）。
    pub fn set_items(&mut self, items: Vec<CachedAttachment>) {
        self.items = items;
    }

    /// 注入账号列表（`(account_id, 显示名)`，供筛选下拉展示）。
    pub fn set_accounts(&mut self, accounts: Vec<(u32, String)>) {
        self.accounts = accounts;
    }

    /// 客户端过滤：搜索词匹配文件名 / 发件人 / 主题，且命中账号筛选。
    fn visible_items(&self) -> Vec<&CachedAttachment> {
        let query = self.query.trim().to_lowercase();
        self.items
            .iter()
            .filter(|item| {
                self.account_filter.is_none_or(|id| item.account_id == id)
                    && (query.is_empty()
                        || item.filename.to_lowercase().contains(&query)
                        || item.sender.to_lowercase().contains(&query)
                        || item.subject.to_lowercase().contains(&query))
            })
            .collect()
    }
}

/// 按附件 MIME 类型挑选文件图标。
fn icon_for_mime(mime_type: &str) -> IconName {
    let mime = mime_type.to_lowercase();
    if mime.starts_with("image/") {
        IconName::Image
    } else if mime.contains("pdf") || mime.contains("text") {
        IconName::FileText
    } else if mime.contains("zip") || mime.contains("compressed") || mime.contains("tar") {
        IconName::FileArchive
    } else {
        IconName::File
    }
}

/// 字节数 → 人类可读大小（B / KB / MB，进率 1024）。
pub fn format_size(bytes: usize) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    let value = bytes as f64;
    if value < KB {
        format!("{bytes} B")
    } else if value < MB {
        format!("{:.1} KB", value / KB)
    } else {
        format!("{:.1} MB", value / MB)
    }
}

impl Render for GalleryState {
    fn render(
        &mut self,
        _window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // 避坑清单第 4 条：render 开头先把语义色 Copy 成局部变量。
        let p = crate::theme::current(cx);
        let (canvas, surface, border, text, muted_fg, accent_soft) = (
            p.canvas,
            p.surface,
            p.border,
            p.text,
            p.text_muted,
            p.accent_soft(),
        );

        let visible = self.visible_items();
        let total = self.items.len();
        let account_filter = self.account_filter;
        let accounts = self.accounts.clone();
        let entity = cx.entity();

        // 账号筛选下拉：全部账号 + 各账号。
        let filter_button = {
            let filter_label = match account_filter {
                None => "全部账号".to_string(),
                Some(id) => accounts
                    .iter()
                    .find(|(acc_id, _)| *acc_id == id)
                    .map(|(_, name)| name.clone())
                    .unwrap_or_else(|| format!("账号 #{id}")),
            };
            let accounts_for_menu = accounts.clone();
            let entity_for_menu = entity.clone();
            Button::new("gallery-account-filter")
                .ghost()
                .label(filter_label)
                .icon(IconName::ChevronDown)
                .dropdown_menu(move |menu, _, _| {
                    let entity_none = entity_for_menu.clone();
                    let mut menu = menu.item(
                        PopupMenuItem::new("全部账号")
                            .when(account_filter.is_none(), |item| {
                                item.icon(Icon::new(IconName::Check))
                            })
                            .on_click(move |_, _, cx| {
                                entity_none.update(cx, |this, cx| {
                                    this.account_filter = None;
                                    cx.emit(GalleryEvent::AccountFilterChanged(None));
                                    cx.notify();
                                });
                            }),
                    );
                    for (acc_id, acc_name) in accounts_for_menu.iter() {
                        let acc_id = *acc_id;
                        let entity_acc = entity_for_menu.clone();
                        let is_selected = account_filter == Some(acc_id);
                        menu = menu.item(
                            PopupMenuItem::new(acc_name.clone())
                                .when(is_selected, |item| item.icon(Icon::new(IconName::Check)))
                                .on_click(move |_, _, cx| {
                                    entity_acc.update(cx, |this, cx| {
                                        this.account_filter = Some(acc_id);
                                        cx.emit(GalleryEvent::AccountFilterChanged(Some(acc_id)));
                                        cx.notify();
                                    });
                                }),
                        );
                    }
                    menu
                })
        };

        let rows: Vec<Stateful<gpui_kit::Div>> = visible
            .iter()
            .map(|item| {
                let icon = icon_for_mime(&item.mime_type);
                let filename = item.filename.clone();
                let size_text = format_size(item.size_bytes);
                let sender = item.sender.clone();
                let subject = item.subject.clone();
                let date = item.date_display.clone();
                let attachment_id = item.id;
                let message_id = item.message_id;

                h_flex()
                    .id(("gallery-row", attachment_id))
                    .h(px(ROW_HEIGHT))
                    .mx_2()
                    .px_3()
                    .gap_2()
                    .items_center()
                    .rounded(px(8.0))
                    .cursor_pointer()
                    .hover(move |s| s.bg(accent_soft))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(GalleryEvent::LocateMessage(message_id));
                        cx.notify();
                    }))
                    // 文件图标。
                    .child(Icon::new(icon).with_size(px(18.0)).text_color(muted_fg))
                    // 文件名（弹性列）。
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .text_color(text)
                            .truncate()
                            .child(SharedString::from(filename)),
                    )
                    // 发件人。
                    .child(
                        div()
                            .w(px(COL_SENDER_W))
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(muted_fg)
                            .truncate()
                            .child(SharedString::from(sender)),
                    )
                    // 所属邮件主题。
                    .child(
                        div()
                            .w(px(COL_SUBJECT_W))
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(muted_fg)
                            .truncate()
                            .child(SharedString::from(subject)),
                    )
                    // 大小。
                    .child(
                        div()
                            .w(px(COL_SIZE_W))
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(muted_fg)
                            .child(SharedString::from(size_text)),
                    )
                    // 日期。
                    .child(
                        div()
                            .w(px(COL_DATE_W))
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(muted_fg)
                            .truncate()
                            .child(SharedString::from(date)),
                    )
                    // 操作按钮。
                    .child(
                        h_flex()
                            .w(px(COL_ACTIONS_W))
                            .flex_shrink_0()
                            .justify_end()
                            .gap_1()
                            .child(
                                Button::new(("gallery-save", attachment_id))
                                    .ghost()
                                    .compact()
                                    .label("保存")
                                    .icon(IconName::Download)
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.emit(GalleryEvent::SaveAttachment(attachment_id));
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new(("gallery-locate", attachment_id))
                                    .ghost()
                                    .compact()
                                    .label("定位到原邮件")
                                    .icon(IconName::ExternalLink)
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.emit(GalleryEvent::LocateMessage(message_id));
                                        cx.notify();
                                    })),
                            ),
                    )
            })
            .collect();

        v_flex()
            .size_full()
            .bg(canvas)
            // 顶部栏 48px：图标 + 标题 + 搜索框 + 账号筛选 + 计数。
            .child(
                h_flex()
                    .h(px(HEADER_HEIGHT))
                    .flex_shrink_0()
                    .px_3()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(border)
                    .bg(surface)
                    .child(
                        Icon::new(IconName::Paperclip)
                            .with_size(px(16.0))
                            .text_color(muted_fg),
                    )
                    .child(
                        div()
                            .text_base()
                            .font_weight(gpui_kit::FontWeight::BOLD)
                            .text_color(text)
                            .child("附件总览"),
                    )
                    .child(
                        div()
                            .w(px(280.0))
                            .flex_shrink_0()
                            .child(Input::new(&self.search_input)),
                    )
                    .child(filter_button)
                    .child(
                        h_flex()
                            .flex_1()
                            .h_full()
                            .pr(px(144.0))
                            .items_center()
                            .window_control_area(WindowControlArea::Drag)
                            .child(div().text_xs().text_color(muted_fg).child(
                                SharedString::from(format!(
                                    "共 {total} 个附件，当前显示 {} 个",
                                    visible.len()
                                )),
                            )),
                    ),
            )
            // 表头。
            .child(
                h_flex()
                    .h(px(30.0))
                    .flex_shrink_0()
                    .mx_2()
                    .px_3()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(border)
                    .text_xs()
                    .text_color(muted_fg)
                    .child(div().w(px(18.0)).flex_shrink_0())
                    .child(div().flex_1().min_w_0().child("文件名"))
                    .child(div().w(px(COL_SENDER_W)).flex_shrink_0().child("发件人"))
                    .child(div().w(px(COL_SUBJECT_W)).flex_shrink_0().child("邮件主题"))
                    .child(div().w(px(COL_SIZE_W)).flex_shrink_0().child("大小"))
                    .child(div().w(px(COL_DATE_W)).flex_shrink_0().child("日期"))
                    .child(div().w(px(COL_ACTIONS_W)).flex_shrink_0().child("操作")),
            )
            // 附件行（可滚动）。行点击即「定位到原邮件」。
            .child(
                v_flex()
                    .id("gallery-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .py_1()
                    .when(visible.is_empty(), |el| {
                        el.child(
                            v_flex()
                                .flex_1()
                                .items_center()
                                .justify_center()
                                .gap_2()
                                .child(
                                    Icon::new(IconName::Paperclip)
                                        .with_size(px(28.0))
                                        .text_color(muted_fg),
                                )
                                .child(div().text_sm().text_color(muted_fg).child(if total == 0 {
                                    "暂无附件"
                                } else {
                                    "无匹配的附件"
                                })),
                        )
                    })
                    .children(rows),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_size_uses_b_kb_mb() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
        assert_eq!(format_size(1024 * 1024 + 512 * 1024), "1.5 MB");
    }

    #[test]
    fn icon_for_mime_covers_common_types() {
        assert!(matches!(icon_for_mime("image/png"), IconName::Image));
        assert!(matches!(
            icon_for_mime("application/pdf"),
            IconName::FileText
        ));
        assert!(matches!(
            icon_for_mime("application/zip"),
            IconName::FileArchive
        ));
        assert!(matches!(
            icon_for_mime("application/octet-stream"),
            IconName::File
        ));
    }
}
