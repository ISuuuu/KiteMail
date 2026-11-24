//! 阅读器底部可拖拽调高附件抽屉（WP-09，对齐 `src/ui/attachment_drawer.rs`）：
//! - 顶部可点击/可拖拽手柄（`.split-grab-pill`）：单击折叠/展开，上下拖拽调整抽屉高度（96..=420px）；
//! - 标题栏：`"N 个附件"` 折叠按钮 + 网格/列表视图切换 + 列表排序方向（A→Z / Z→A）+「全部保存」；
//! - 网格视图：图片附件显示真实缩略图预览（`gpui_kit::Image::from_bytes`），其余显示按类型着色的文件徽章；
//! - 列表视图：按字母顺序排列的紧凑行，含文件图标、大小、保存与「从服务器剥离」；
//! - 服务端剥离附件（`Strip from Message`）：带行内二次确认与「正在剥离…」淡红占位态。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    div, img, px, AppContext, Context, Hsla, Image, ImageFormat, InteractiveElement, IntoElement,
    ObjectFit, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled,
    StyledImage, Window,
};
use hylki_mail_core::AttachmentMeta;

use crate::app::HylkiApp;
use crate::theme;

/// 抽屉展开态最小/最大高度（px）。
pub(crate) const DRAWER_MIN_HEIGHT: f32 = 96.0;
pub(crate) const DRAWER_MAX_HEIGHT: f32 = 420.0;
pub(crate) const DRAWER_DEFAULT_HEIGHT: f32 = 176.0;

/// 抽屉顶缘拖拽手柄的拖拽载荷。
#[derive(Clone, Copy)]
pub(crate) struct AttachmentDrawerDrag;

impl Render for AttachmentDrawerDrag {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// 抽屉中展示的单条附件（关联所属邮件 ID 以便服务端剥离）。
#[derive(Clone, Debug)]
pub(crate) struct DrawerAttachment {
    pub(crate) account_id: u32,
    pub(crate) message_id: u32,
    pub(crate) meta: AttachmentMeta,
}

/// 格式化字节数为人类可读大小。
pub(crate) fn format_bytes(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// 映射图片 MIME 到 `gpui_kit::ImageFormat`（不支持矢量 SVG）。
pub(crate) fn image_format_for_mime(mime_type: &str) -> Option<ImageFormat> {
    let mime = mime_type.to_ascii_lowercase();
    if mime.starts_with("image/png") {
        Some(ImageFormat::Png)
    } else if mime.starts_with("image/jpeg") || mime.starts_with("image/jpg") {
        Some(ImageFormat::Jpeg)
    } else if mime.starts_with("image/webp") {
        Some(ImageFormat::Webp)
    } else if mime.starts_with("image/gif") {
        Some(ImageFormat::Gif)
    } else if mime.starts_with("image/bmp") {
        Some(ImageFormat::Bmp)
    } else {
        None
    }
}

/// 按 MIME 类型或扩展名返回短类型标签与着色。
fn file_badge(filename: &str, mime_type: &str, accent: Hsla) -> (&'static str, Hsla) {
    let lower_mime = mime_type.to_ascii_lowercase();
    let ext = filename
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if lower_mime.starts_with("image/") {
        ("IMG", gpui_kit::hsla(160.0 / 360.0, 0.55, 0.42, 1.0))
    } else if lower_mime == "application/pdf" || ext == "pdf" {
        ("PDF", gpui_kit::hsla(0.0, 0.68, 0.52, 1.0))
    } else if matches!(ext.as_str(), "zip" | "tar" | "gz" | "7z" | "rar") {
        ("ZIP", gpui_kit::hsla(36.0 / 360.0, 0.78, 0.48, 1.0))
    } else if matches!(ext.as_str(), "doc" | "docx" | "odt" | "rtf" | "txt" | "md") {
        ("DOC", accent)
    } else if matches!(ext.as_str(), "xls" | "xlsx" | "csv") {
        ("XLS", gpui_kit::hsla(142.0 / 360.0, 0.56, 0.42, 1.0))
    } else {
        ("FILE", gpui_kit::hsla(220.0 / 360.0, 0.14, 0.48, 1.0))
    }
}

/// 构建底部附件抽屉元素；无附件时返回 `None`。
#[allow(clippy::too_many_arguments)]
pub(crate) fn render_drawer(
    items: &[DrawerAttachment],
    collapsed: bool,
    height: f32,
    list_view: bool,
    sort_desc: bool,
    demo_mode: bool,
    thumbs: &HashMap<u32, Arc<Image>>,
    stripping_ids: &HashSet<u32>,
    pending_strip_id: Option<u32>,
    cx: &mut Context<HylkiApp>,
) -> Option<gpui_kit::AnyElement> {
    if items.is_empty() {
        return None;
    }

    let p = theme::current(cx);
    let (border, muted_fg, accent) = (p.border, p.text_muted, p.accent);
    let drawer_bg = p.surface_raised;
    let danger = gpui_kit::hsla(0.0, 0.72, 0.52, 1.0);

    let total_count = items.len();
    let all_ids: Vec<u32> = items.iter().map(|i| i.meta.id).collect();

    // 列表模式按文件名字母排序（支持 A→Z / Z→A）。
    let mut order: Vec<usize> = (0..items.len()).collect();
    if list_view {
        order.sort_by(|&a, &b| {
            let cmp = items[a]
                .meta
                .filename
                .to_lowercase()
                .cmp(&items[b].meta.filename.to_lowercase());
            if sort_desc {
                cmp.reverse()
            } else {
                cmp
            }
        });
    }

    let body_rows: Vec<gpui_kit::AnyElement> = order
        .into_iter()
        .map(|idx| {
            let item = &items[idx];
            let att_id = item.meta.id;
            let account_id = item.account_id;
            let message_id = item.message_id;
            let filename = item.meta.filename.clone();
            let mime_type = item.meta.mime_type.clone();
            let size_bytes = item.meta.size_bytes;
            let downloaded = item.meta.downloaded;
            let size_label = if downloaded {
                format_bytes(size_bytes)
            } else {
                format!("约 {} · 未下载", format_bytes(size_bytes))
            };
            let save_tooltip = if downloaded {
                "保存附件"
            } else {
                "下载并保存附件"
            };
            let is_stripping = stripping_ids.contains(&att_id);
            let is_confirming = pending_strip_id == Some(att_id);
            let (badge_label, badge_color) = file_badge(&filename, &mime_type, accent);
            let thumb = thumbs.get(&att_id).cloned();
            let is_image = image_format_for_mime(&mime_type).is_some();

            if list_view {
                let save_name = filename.clone();
                let strip_name = filename.clone();
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(if is_stripping {
                        danger.opacity(0.45)
                    } else {
                        border
                    })
                    .bg(if is_stripping {
                        danger.opacity(0.10)
                    } else {
                        p.canvas
                    })
                    .child(
                        div()
                            .px_1p5()
                            .py_0p5()
                            .rounded(px(4.0))
                            .bg(badge_color.opacity(0.16))
                            .text_color(badge_color)
                            .text_xs()
                            .font_weight(gpui_kit::FontWeight::BOLD)
                            .child(badge_label),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(p.text)
                            .truncate()
                            .child(SharedString::from(filename.clone())),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(muted_fg)
                            .child(SharedString::from(size_label)),
                    )
                    .when(is_stripping, |row| {
                        row.child(
                            div()
                                .text_xs()
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .text_color(danger)
                                .child("正在剥离…"),
                        )
                    })
                    .when(!is_stripping, |row| {
                        row.child(
                            Button::new(("drawer-list-save", att_id))
                                .ghost()
                                .compact()
                                .icon(IconName::Download)
                                .tooltip(save_tooltip)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.save_attachment(att_id, &save_name, cx);
                                })),
                        )
                        .when(!demo_mode && !is_confirming, |r| {
                            r.child(
                                Button::new(("drawer-list-strip", att_id))
                                    .ghost()
                                    .compact()
                                    .icon(IconName::Trash)
                                    .tooltip("从服务器剥离附件以释放配额")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.ask_strip_attachment(att_id, cx);
                                    })),
                            )
                        })
                        .when(!demo_mode && is_confirming, |r| {
                            let s_name = strip_name.clone();
                            r.child(
                                Button::new(("drawer-list-strip-confirm", att_id))
                                    .danger()
                                    .compact()
                                    .label("确认剥离")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.confirm_strip_attachment(
                                            account_id,
                                            message_id,
                                            att_id,
                                            s_name.clone(),
                                            size_bytes,
                                            cx,
                                        );
                                    })),
                            )
                            .child(
                                Button::new(("drawer-list-strip-cancel", att_id))
                                    .ghost()
                                    .compact()
                                    .label("取消")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.cancel_strip_attachment(cx);
                                    })),
                            )
                        })
                    })
                    .into_any_element()
            } else {
                let save_name = filename.clone();
                let strip_name = filename.clone();
                let preview_name = filename.clone();
                let preview_mime = mime_type.clone();
                v_flex()
                    .w(px(148.0))
                    .p_1p5()
                    .gap_1()
                    .rounded(px(10.0))
                    .border_1()
                    .border_color(if is_stripping {
                        danger.opacity(0.45)
                    } else {
                        border
                    })
                    .bg(if is_stripping {
                        danger.opacity(0.10)
                    } else {
                        p.canvas
                    })
                    .child(
                        div()
                            .id(("drawer-thumb", att_id))
                            .w_full()
                            .h(px(68.0))
                            .rounded(px(7.0))
                            .bg(p.surface)
                            .flex()
                            .items_center()
                            .justify_center()
                            .overflow_hidden()
                            .when(is_image, |box_el| {
                                box_el.cursor_pointer().on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.open_drawer_image_preview(
                                            att_id,
                                            preview_name.clone(),
                                            preview_mime.clone(),
                                            cx,
                                        );
                                    },
                                ))
                            })
                            .child(if let Some(image) = thumb {
                                img(image)
                                    .object_fit(ObjectFit::Contain)
                                    .w_full()
                                    .h_full()
                                    .into_any_element()
                            } else {
                                v_flex()
                                    .items_center()
                                    .justify_center()
                                    .gap_0p5()
                                    .child(
                                        div()
                                            .px_2()
                                            .py_0p5()
                                            .rounded(px(5.0))
                                            .bg(badge_color.opacity(0.16))
                                            .text_color(badge_color)
                                            .text_xs()
                                            .font_weight(gpui_kit::FontWeight::BOLD)
                                            .child(badge_label),
                                    )
                                    .into_any_element()
                            }),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(p.text)
                            .truncate()
                            .child(SharedString::from(filename)),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted_fg)
                                    .child(SharedString::from(size_label)),
                            )
                            .when(is_stripping, |row| {
                                row.child(div().text_xs().text_color(danger).child("剥离中…"))
                            })
                            .when(!is_stripping, |row| {
                                row.child(
                                    h_flex()
                                        .gap_0p5()
                                        .items_center()
                                        .child(
                                            Button::new(("drawer-grid-save", att_id))
                                                .ghost()
                                                .compact()
                                                .icon(IconName::Download)
                                                .tooltip(save_tooltip)
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.save_attachment(att_id, &save_name, cx);
                                                })),
                                        )
                                        .when(!demo_mode && !is_confirming, |r| {
                                            r.child(
                                                Button::new(("drawer-grid-strip", att_id))
                                                    .ghost()
                                                    .compact()
                                                    .icon(IconName::Trash)
                                                    .tooltip("从服务器剥离附件")
                                                    .on_click(cx.listener(
                                                        move |this, _, _, cx| {
                                                            this.ask_strip_attachment(att_id, cx);
                                                        },
                                                    )),
                                            )
                                        }),
                                )
                            }),
                    )
                    .when(!demo_mode && !is_stripping && is_confirming, |card| {
                        let s_name = strip_name;
                        card.child(
                            h_flex()
                                .gap_1()
                                .justify_end()
                                .child(
                                    Button::new(("drawer-grid-strip-cancel", att_id))
                                        .ghost()
                                        .compact()
                                        .label("取消")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.cancel_strip_attachment(cx);
                                        })),
                                )
                                .child(
                                    Button::new(("drawer-grid-strip-confirm", att_id))
                                        .danger()
                                        .compact()
                                        .label("剥离")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.confirm_strip_attachment(
                                                account_id,
                                                message_id,
                                                att_id,
                                                s_name.clone(),
                                                size_bytes,
                                                cx,
                                            );
                                        })),
                                ),
                        )
                    })
                    .into_any_element()
            }
        })
        .collect();

    Some(
        v_flex()
            .flex_shrink_0()
            .w_full()
            .bg(drawer_bg)
            .border_t_1()
            .border_color(border)
            // 顶部拖拽手柄 + 单击折叠/展开（.split-grab-pill）。
            .child(
                div()
                    .id("attachment-drawer-grab")
                    .w_full()
                    .h(px(12.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_ns_resize()
                    .on_drag(AttachmentDrawerDrag, |_, _, _, cx| {
                        cx.new(|_| AttachmentDrawerDrag)
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.toggle_attachment_drawer(cx);
                    }))
                    .child(
                        div()
                            .w(px(88.0))
                            .h(px(4.0))
                            .rounded_full()
                            .bg(muted_fg.opacity(0.45)),
                    ),
            )
            // 抽屉标题栏。
            .child(
                h_flex()
                    .px_4()
                    .pb_1p5()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        Button::new("attachment-drawer-count")
                            .ghost()
                            .compact()
                            .icon(if collapsed {
                                IconName::ChevronUp
                            } else {
                                IconName::ChevronDown
                            })
                            .label(SharedString::from(format!("{total_count} 个附件")))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_attachment_drawer(cx);
                            })),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .when(!collapsed && list_view, |bar| {
                                bar.child(
                                    Button::new("attachment-drawer-sort")
                                        .ghost()
                                        .compact()
                                        .icon(if sort_desc {
                                            IconName::ArrowDown
                                        } else {
                                            IconName::ArrowUp
                                        })
                                        .tooltip(if sort_desc {
                                            "当前 Z→A（点击切换 A→Z）"
                                        } else {
                                            "当前 A→Z（点击切换 Z→A）"
                                        })
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.toggle_attachment_drawer_sort(cx);
                                        })),
                                )
                            })
                            .when(!collapsed, |bar| {
                                bar.child(
                                    Button::new("attachment-drawer-view-mode")
                                        .ghost()
                                        .compact()
                                        .icon(if list_view {
                                            IconName::LayoutDashboard
                                        } else {
                                            IconName::Menu
                                        })
                                        .tooltip(if list_view {
                                            "切换为缩略图网格"
                                        } else {
                                            "切换为字母列表"
                                        })
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.toggle_attachment_drawer_view(cx);
                                        })),
                                )
                            })
                            .child(
                                Button::new("attachment-drawer-save-all")
                                    .outline()
                                    .compact()
                                    .icon(IconName::Download)
                                    .label("全部保存")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.save_all_attachments(all_ids.clone(), cx);
                                    })),
                            ),
                    ),
            )
            .when(!collapsed, |drawer| {
                drawer.child(
                    div()
                        .id("attachment-drawer-body")
                        .w_full()
                        .h(px(height.clamp(DRAWER_MIN_HEIGHT, DRAWER_MAX_HEIGHT)))
                        .overflow_y_scrollbar()
                        .px_4()
                        .pb_3()
                        .child(if list_view {
                            v_flex().gap_1().children(body_rows).into_any_element()
                        } else {
                            h_flex()
                                .flex_wrap()
                                .gap_2()
                                .items_start()
                                .children(body_rows)
                                .into_any_element()
                        }),
                )
            })
            .into_any_element(),
    )
}
