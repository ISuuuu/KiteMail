//! 附件总览页（方案文档第三节页面 7）：
//! - 顶部栏：附件图标 + 标题 + 搜索框（即时过滤，变更发 [`GalleryEvent::QueryChanged`]）
//!   + 账号筛选下拉（变更发 [`GalleryEvent::AccountFilterChanged`]）
//!   + F-6「网格 / 列表」视图切换（会话级记忆）。
//! - 下方紧凑表格 / 卡片网格：
//!   - 列表：文件图标、文件名、大小（人类可读）、发件人、所属邮件主题、日期，
//!     以及「保存」「定位到原邮件」按钮；文件名 / 大小 / 日期表头点击排序
//!     （升 ↔ 降循环，箭头指示方向），与账号筛选叠加。
//!   - 网格：卡片（类型图标 + 文件名 + 大小）；图片附件点击预览发
//!     [`GalleryEvent::PreviewImage`]，其余定位到原邮件。
//! - F-6 图片预览浮层：集成者回填附件字节后显示大图（`img` + ObjectFit），
//!   无字节（demo / 非缓存）显示占位说明与「打开所在邮件」。
//!
//! 自包含组件：数据由集成者通过 [`GalleryState::set_items`] /
//! [`GalleryState::set_accounts`] 注入，交互通过 [`GalleryEvent`] 通知集成者，
//! 组件内部同时对当前数据做客户端过滤以便即时反馈。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::Icon;
use gpui_kit::component::{h_flex, v_flex, Selectable, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    black, div, img, px, AppContext, Context, EventEmitter, ImageFormat, InteractiveElement,
    IntoElement, MouseButton, ObjectFit, ParentElement, Render, SharedString, Stateful,
    StatefulInteractiveElement, Styled, StyledImage, WindowControlArea,
};
use kitemail_mail_core::cache::CachedAttachment;

/// 顶部栏高度。
const HEADER_HEIGHT: f32 = 48.0;
/// 底部 ActionBar 高度。
const FOOTER_HEIGHT: f32 = 40.0;
/// 附件行高度。
const ROW_HEIGHT: f32 = 44.0;
/// 表格列宽（与表头、数据行共用）。
const COL_SENDER_W: f32 = 150.0;
const COL_SUBJECT_W: f32 = 200.0;
const COL_SIZE_W: f32 = 80.0;
const COL_DATE_W: f32 = 120.0;
const COL_ACTIONS_W: f32 = 210.0;
/// F-6 网格卡片默认尺寸与缩略图滑块范围。
const THUMB_SIZE_DEFAULT: f32 = 150.0;
const THUMB_SIZE_MIN: f32 = 110.0;
const THUMB_SIZE_MAX: f32 = 230.0;
const THUMB_SIZE_STEP: f32 = 20.0;

/// WP-10 附件画廊 7 种文件类型筛选（对齐原版 `src/ui/attachments_gallery.rs:439-447`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GalleryTypeFilter {
    #[default]
    All,
    Images,
    Pdfs,
    Documents,
    Archives,
    AudioVideo,
    Other,
}

impl GalleryTypeFilter {
    pub const ALL: [Self; 7] = [
        Self::All,
        Self::Images,
        Self::Pdfs,
        Self::Documents,
        Self::Archives,
        Self::AudioVideo,
        Self::Other,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "全部类型",
            Self::Images => "图片",
            Self::Pdfs => "PDF",
            Self::Documents => "文档",
            Self::Archives => "压缩包",
            Self::AudioVideo => "音视频",
            Self::Other => "其他",
        }
    }

    /// 判断附件是否匹配当前类型筛选。
    pub fn matches(self, mime_type: &str, filename: &str) -> bool {
        if self == Self::All {
            return true;
        }
        let mime = mime_type.to_ascii_lowercase();
        let ext = filename
            .rsplit('.')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        let is_image = mime.starts_with("image/")
            || matches!(
                ext.as_str(),
                "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "svg"
            );
        let is_pdf = mime == "application/pdf" || ext == "pdf";
        let is_archive = mime.contains("zip")
            || mime.contains("compressed")
            || mime.contains("tar")
            || mime.contains("rar")
            || mime.contains("7z")
            || matches!(
                ext.as_str(),
                "zip" | "rar" | "7z" | "tar" | "gz" | "bz2" | "xz"
            );
        let is_av = mime.starts_with("audio/")
            || mime.starts_with("video/")
            || matches!(
                ext.as_str(),
                "mp3" | "wav" | "flac" | "ogg" | "mp4" | "mkv" | "mov" | "avi" | "webm"
            );
        let is_doc = !is_pdf
            && (mime.starts_with("text/")
                || mime.contains("word")
                || mime.contains("excel")
                || mime.contains("spreadsheet")
                || mime.contains("presentation")
                || mime.contains("opendocument")
                || matches!(
                    ext.as_str(),
                    "doc"
                        | "docx"
                        | "xls"
                        | "xlsx"
                        | "ppt"
                        | "pptx"
                        | "txt"
                        | "md"
                        | "csv"
                        | "rtf"
                        | "odt"
                        | "ods"
                        | "odp"
                ));

        match self {
            Self::All => true,
            Self::Images => is_image,
            Self::Pdfs => is_pdf,
            Self::Documents => is_doc,
            Self::Archives => is_archive,
            Self::AudioVideo => is_av,
            Self::Other => !is_image && !is_pdf && !is_doc && !is_archive && !is_av,
        }
    }
}

/// F-6 可排序的表头列。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GallerySortColumn {
    Filename,
    Size,
    Date,
}

impl GallerySortColumn {
    /// 表头展示名。
    fn label(self) -> &'static str {
        match self {
            Self::Filename => "文件名",
            Self::Size => "大小",
            Self::Date => "日期",
        }
    }
}

#[derive(Clone, Debug)]
enum GalleryPreviewStatus {
    Loading,
    Loaded(ImageFormat, Vec<u8>),
    Unsupported,
    Failed(SharedString),
}

/// F-6 图片预览浮层状态。
struct GalleryPreview {
    attachment_id: u32,
    filename: String,
    status: GalleryPreviewStatus,
}

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
    /// F-6 请求预览图片附件（参数为附件 id；字节由集成者回传）。
    PreviewImage(u32),
    /// WP-10 请求加载网格图片缩略图（参数为附件 id）。
    RequestThumbnail(u32),
}

/// 附件总览视图状态（独立 Entity，由 HylkiApp 挂载）。
pub struct GalleryState {
    search_input: gpui_kit::Entity<InputState>,
    /// 当前搜索词（客户端过滤 + 供集成者展示）。
    query: String,
    /// 当前账号筛选。
    account_filter: Option<u32>,
    /// WP-10 7 种文件类型筛选。
    type_filter: GalleryTypeFilter,
    /// WP-10 文件夹范围：包含归档 / 包含其他文件夹（收件箱始终包含）。
    include_archive: bool,
    include_other: bool,
    /// WP-10 文件夹范围 Popover 开关。
    folders_popover_open: bool,
    /// WP-10 网格缩略图尺寸（px）。
    thumb_size: f32,
    /// F-6 表头排序（`None` = 默认按日期倒序的服务端次序）。
    sort_column: Option<GallerySortColumn>,
    /// F-6 当前排序方向。
    sort_asc: bool,
    /// F-6 视图模式（`true` = 网格，`false` = 列表；会话级记忆）。
    view_grid: bool,
    /// F-6 当前打开的图片预览浮层。
    preview: Option<GalleryPreview>,
    /// WP-10 网格图片真实缩略图缓存。
    thumbnails: HashMap<u32, Arc<gpui_kit::Image>>,
    /// WP-10 已请求过缩略图的附件 ID（避免重复请求）。
    requested_thumbnails: HashSet<u32>,
    /// 集成者注入的账号列表 `(account_id, 显示名)`。
    accounts: Vec<(u32, String)>,
    /// 集成者注入的附件列表。
    items: Vec<CachedAttachment>,
    demo_mode: bool,
    saving_ids: HashSet<u32>,
    item_errors: HashMap<u32, SharedString>,
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
            type_filter: GalleryTypeFilter::All,
            include_archive: true,
            include_other: true,
            folders_popover_open: false,
            thumb_size: THUMB_SIZE_DEFAULT,
            sort_column: None,
            sort_asc: true,
            view_grid: false,
            preview: None,
            thumbnails: HashMap::new(),
            requested_thumbnails: HashSet::new(),
            accounts: Vec::new(),
            items: Vec::new(),
            demo_mode: false,
            saving_ids: HashSet::new(),
            item_errors: HashMap::new(),
        }
    }

    pub fn set_demo_mode(&mut self, demo: bool) {
        self.demo_mode = demo;
    }

    pub fn set_saving(&mut self, attachment_id: u32, saving: bool) {
        if saving {
            self.saving_ids.insert(attachment_id);
            self.item_errors.remove(&attachment_id);
        } else {
            self.saving_ids.remove(&attachment_id);
        }
    }

    pub fn set_item_error(&mut self, attachment_id: u32, error: impl Into<SharedString>) {
        self.saving_ids.remove(&attachment_id);
        self.item_errors.insert(attachment_id, error.into());
    }

    pub fn dismiss_overlay(&mut self) -> bool {
        if self.preview.take().is_some() || std::mem::take(&mut self.folders_popover_open) {
            return true;
        }
        false
    }

    /// 注入附件数据（集成者在收到缓存查询结果后调用）。
    pub fn set_items(&mut self, items: Vec<CachedAttachment>) {
        self.items = items;
    }

    /// 标记指定附件已完成按需下载，并更新展示大小。
    pub fn mark_downloaded(&mut self, attachment_id: u32, size_bytes: usize) {
        for item in &mut self.items {
            if item.id == attachment_id {
                item.downloaded = true;
                item.size_bytes = size_bytes;
            }
        }
    }

    /// 注入账号列表（`(account_id, 显示名)`，供筛选下拉展示）。
    pub fn set_accounts(&mut self, accounts: Vec<(u32, String)>) {
        self.accounts = accounts;
    }

    /// WP-10 回填网格图片缩略图。
    pub fn set_thumbnail(&mut self, attachment_id: u32, mime_type: &str, bytes: Vec<u8>) {
        if bytes.is_empty() {
            return;
        }
        if let Some(format) = image_format_for_mime(mime_type) {
            self.thumbnails.insert(
                attachment_id,
                Arc::new(gpui_kit::Image::from_bytes(format, bytes)),
            );
        }
    }

    /// F-6 回填图片预览字节：MIME 非图片或字节为空时标明对应状态。
    pub fn set_preview_image(&mut self, filename: &str, mime_type: &str, bytes: Vec<u8>) {
        let format = image_format_for_mime(mime_type);
        if let Some(preview) = self.preview.as_mut() {
            preview.filename = filename.to_string();
            preview.status = match format {
                None => GalleryPreviewStatus::Unsupported,
                Some(_) if bytes.is_empty() => {
                    GalleryPreviewStatus::Failed(SharedString::from("图片数据为空"))
                }
                Some(fmt) => {
                    self.thumbnails.insert(
                        preview.attachment_id,
                        Arc::new(gpui_kit::Image::from_bytes(fmt, bytes.clone())),
                    );
                    GalleryPreviewStatus::Loaded(fmt, bytes)
                }
            };
        }
    }

    /// F-6 预览字节不可用（demo / 抓取失败）：显示占位说明。
    pub fn set_preview_unavailable(&mut self) {
        if let Some(preview) = self.preview.as_mut() {
            preview.status = if self.demo_mode {
                GalleryPreviewStatus::Unsupported
            } else {
                GalleryPreviewStatus::Failed(SharedString::from(
                    "未能加载图片数据，请重试或打开所在邮件",
                ))
            };
        }
    }

    pub fn set_preview_failed(&mut self, attachment_id: u32, reason: impl Into<SharedString>) {
        if let Some(preview) = self
            .preview
            .as_mut()
            .filter(|p| p.attachment_id == attachment_id)
        {
            preview.status = GalleryPreviewStatus::Failed(reason.into());
        }
    }

    /// WP-10 为当前可见图片附件请求缩略图（避免重复请求）。
    fn request_visible_thumbnails(&mut self, cx: &mut Context<Self>) {
        if !self.view_grid {
            return;
        }
        let ids: Vec<u32> = self
            .visible_items()
            .into_iter()
            .filter(|item| {
                item.downloaded && item.mime_type.to_ascii_lowercase().starts_with("image/")
            })
            .map(|item| item.id)
            .filter(|id| {
                !self.thumbnails.contains_key(id) && !self.requested_thumbnails.contains(id)
            })
            .take(40)
            .collect();
        for id in ids {
            self.requested_thumbnails.insert(id);
            cx.emit(GalleryEvent::RequestThumbnail(id));
        }
    }

    /// F-6 表头点击循环排序：切列默认升序，再点同列反转方向。
    fn cycle_sort(&mut self, column: GallerySortColumn) {
        if self.sort_column == Some(column) {
            if self.sort_asc {
                self.sort_asc = false;
            } else {
                // 第三次点击恢复默认次序。
                self.sort_column = None;
                self.sort_asc = true;
            }
        } else {
            self.sort_column = Some(column);
            self.sort_asc = true;
        }
    }

    /// 客户端过滤 + F-6 表头排序：搜索词匹配文件名 / 发件人 / 主题，
    /// 命中账号筛选与 WP-10 7 种文件类型筛选；按所选列升 / 降序（默认服务端日期倒序）。
    fn visible_items(&self) -> Vec<&CachedAttachment> {
        let query = self.query.trim().to_lowercase();
        let mut items: Vec<&CachedAttachment> = self
            .items
            .iter()
            .filter(|item| {
                self.account_filter.is_none_or(|id| item.account_id == id)
                    && self.type_filter.matches(&item.mime_type, &item.filename)
                    && (query.is_empty()
                        || item.filename.to_lowercase().contains(&query)
                        || item.sender.to_lowercase().contains(&query)
                        || item.subject.to_lowercase().contains(&query))
            })
            .collect();
        match self.sort_column {
            Some(GallerySortColumn::Filename) => {
                items.sort_by(|a, b| {
                    let ord = a.filename.to_lowercase().cmp(&b.filename.to_lowercase());
                    if self.sort_asc {
                        ord
                    } else {
                        ord.reverse()
                    }
                });
            }
            Some(GallerySortColumn::Size) => {
                items.sort_by(|a, b| {
                    let ord = a.size_bytes.cmp(&b.size_bytes);
                    if self.sort_asc {
                        ord
                    } else {
                        ord.reverse()
                    }
                });
            }
            Some(GallerySortColumn::Date) => {
                items.sort_by(|a, b| {
                    let ord = a.date_ts.cmp(&b.date_ts).then_with(|| a.id.cmp(&b.id));
                    if self.sort_asc {
                        ord
                    } else {
                        ord.reverse()
                    }
                });
            }
            None => {}
        }
        items
    }
}

/// F-6 MIME 类型 → gpui 可渲染的图片格式（仅常见位图；SVG 不支持预览）。
fn image_format_for_mime(mime_type: &str) -> Option<ImageFormat> {
    let mime = mime_type.to_ascii_lowercase();
    if mime.starts_with("image/png") {
        Some(ImageFormat::Png)
    } else if mime.starts_with("image/jpeg") {
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

        // F-6：「网格 / 列表」视图切换（会话级记忆，默认列表）。
        let entity_for_views = entity.clone();
        let grid_btn = Button::new("gallery-view-grid")
            .ghost()
            .compact()
            .icon(IconName::LayoutGrid)
            .tooltip("网格视图")
            .selected(self.view_grid)
            .on_click(cx.listener(|this, _, _, cx| {
                this.view_grid = true;
                cx.notify();
            }));
        let list_btn = Button::new("gallery-view-list")
            .ghost()
            .compact()
            .icon(IconName::List)
            .tooltip("列表视图")
            .selected(!self.view_grid)
            .on_click(move |_, _, cx| {
                entity_for_views.update(cx, |this, cx| {
                    this.view_grid = false;
                    cx.notify();
                });
            });

        // F-6：表头单元格（文件名 / 大小 / 日期）点击循环排序，箭头指示方向。
        let sort_header = |column: GallerySortColumn| -> gpui_kit::Stateful<gpui_kit::Div> {
            let active = self.sort_column == Some(column);
            let (label, width) = match column {
                GallerySortColumn::Filename => (column.label(), 0.0),
                GallerySortColumn::Size => (column.label(), COL_SIZE_W),
                GallerySortColumn::Date => (column.label(), COL_DATE_W),
            };
            let arrow = if active {
                Some(if self.sort_asc {
                    IconName::SortAscending
                } else {
                    IconName::SortDescending
                })
            } else {
                None
            };
            let mut cell = h_flex()
                .id(("gallery-sort", column as u32))
                .items_center()
                .gap_0p5()
                .cursor_pointer();
            if width > 0.0 {
                cell = cell.w(px(width)).flex_shrink_0();
            } else {
                cell = cell.flex_1().min_w_0();
            }
            let entity_for_sort = entity.clone();
            cell.child(div().child(label))
                .when_some(arrow, |el, icon| {
                    el.child(Icon::new(icon).with_size(px(12.0)).text_color(if active {
                        text
                    } else {
                        muted_fg
                    }))
                })
                .when(active, |el| el.text_color(text))
                .on_click(move |_, _, cx| {
                    entity_for_sort.update(cx, |this, cx| {
                        this.cycle_sort(column);
                        cx.notify();
                    });
                })
        };

        let rows: Vec<Stateful<gpui_kit::Div>> = visible
            .iter()
            .map(|item| {
                let icon = icon_for_mime(&item.mime_type);
                let filename = item.filename.clone();
                let downloaded = item.downloaded;
                let size_text = if downloaded {
                    format_size(item.size_bytes)
                } else {
                    format!("约 {} · 未下载", format_size(item.size_bytes))
                };
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
                                    .tooltip(if downloaded {
                                        "保存附件"
                                    } else {
                                        "下载并保存附件"
                                    })
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

        // F-6：图片预览浮层（遮罩 + 居中卡片）。
        // 已回传图片字节时显示大图（ObjectFit::Contain 等比缩放），
        // 否则显示占位说明与「打开所在邮件」跳转。
        let preview_overlay = if let Some(preview) = self.preview.as_ref() {
            let message_id = self
                .items
                .iter()
                .find(|item| item.id == preview.attachment_id)
                .map(|item| item.message_id);
            let filename = preview.filename.clone();
            let attachment_id_for_retry = preview.attachment_id;
            let status = preview.status.clone();
            div()
                .absolute()
                .top(px(0.0))
                .left(px(0.0))
                .size_full()
                .bg(black().opacity(0.5))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.preview = None;
                        cx.notify();
                    }),
                )
                .child(
                    h_flex().size_full().items_center().justify_center().child(
                        v_flex()
                            .bg(p.surface)
                            .rounded(px(12.0))
                            .border_1()
                            .border_color(p.border)
                            .shadow_xl()
                            .p_3()
                            .gap_2()
                            .max_w(px(720.0))
                            .max_h(px(560.0))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                cx.stop_propagation();
                            })
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(gpui_kit::FontWeight::MEDIUM)
                                            .text_color(text)
                                            .truncate()
                                            .child(SharedString::from(filename.clone())),
                                    )
                                    .child(
                                        Button::new("gallery-preview-close")
                                            .ghost()
                                            .compact()
                                            .icon(IconName::Close)
                                            .tooltip("关闭预览")
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.preview = None;
                                                cx.notify();
                                            })),
                                    ),
                            )
                            .child(match status {
                                GalleryPreviewStatus::Loaded(format, bytes) => {
                                    let image =
                                        std::sync::Arc::new(gpui_kit::Image::from_bytes(format, bytes));
                                    img(image)
                                        .object_fit(ObjectFit::Contain)
                                        .flex_1()
                                        .min_h_0()
                                        .rounded(px(6.0))
                                        .into_any_element()
                                }
                                GalleryPreviewStatus::Loading => v_flex()
                                    .items_center()
                                    .justify_center()
                                    .gap_2()
                                    .py_8()
                                    .child(
                                        Icon::new(IconName::Image)
                                            .with_size(px(32.0))
                                            .text_color(muted_fg),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(muted_fg)
                                            .child("正在加载图片预览…"),
                                    )
                                    .into_any_element(),
                                GalleryPreviewStatus::Unsupported => v_flex()
                                    .items_center()
                                    .justify_center()
                                    .gap_2()
                                    .py_8()
                                    .child(
                                        Icon::new(IconName::Image)
                                            .with_size(px(32.0))
                                            .text_color(muted_fg),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(muted_fg)
                                            .child("当前格式或演示模式暂不支持内联预览"),
                                    )
                                    .when_some(message_id, |el, msg_id| {
                                        el.child(
                                            Button::new("gallery-preview-locate")
                                                .outline()
                                                .compact()
                                                .label("打开所在邮件")
                                                .icon(IconName::ExternalLink)
                                                .on_click(cx.listener(move |_, _, _, cx| {
                                                    cx.emit(GalleryEvent::LocateMessage(msg_id));
                                                    cx.notify();
                                                })),
                                        )
                                    })
                                    .into_any_element(),
                                GalleryPreviewStatus::Failed(reason) => v_flex()
                                    .items_center()
                                    .justify_center()
                                    .gap_2()
                                    .py_8()
                                    .child(
                                        Icon::new(IconName::TriangleAlert)
                                            .with_size(px(28.0))
                                            .text_color(p.danger),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(p.danger)
                                            .child(reason),
                                    )
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .when(!self.demo_mode, |row| {
                                                row.child(
                                                    Button::new("gallery-preview-retry")
                                                        .primary()
                                                        .compact()
                                                        .label("重试")
                                                        .on_click(cx.listener(move |this, _, _, cx| {
                                                            if let Some(ref mut pr) = this.preview {
                                                                pr.status = GalleryPreviewStatus::Loading;
                                                            }
                                                            cx.emit(GalleryEvent::PreviewImage(
                                                                attachment_id_for_retry,
                                                            ));
                                                            cx.notify();
                                                        })),
                                                )
                                            })
                                            .when_some(message_id, |row, msg_id| {
                                                row.child(
                                                    Button::new("gallery-preview-locate")
                                                        .outline()
                                                        .compact()
                                                        .label("打开所在邮件")
                                                        .icon(IconName::ExternalLink)
                                                        .on_click(cx.listener(move |_, _, _, cx| {
                                                            cx.emit(GalleryEvent::LocateMessage(
                                                                msg_id,
                                                            ));
                                                            cx.notify();
                                                        })),
                                                )
                                            }),
                                    )
                                    .into_any_element(),
                            }),
                    ),
                )
        } else {
            div()
        };

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
                    // F-6：网格 / 列表视图切换。
                    .child(grid_btn)
                    .child(list_btn)
                    .child(
                        h_flex()
                            .flex_1()
                            .h_full()
                            .pr(px(144.0))
                            .items_center()
                            .window_control_area(WindowControlArea::Drag)
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted_fg)
                                    .child(SharedString::from(format!(
                                        "共 {total} 个附件，当前显示 {} 个",
                                        visible.len()
                                    ))),
                            ),
                    ),
            )
            // 表头（列表视图）：文件名 / 大小 / 日期可点击排序。
            .when(!self.view_grid, |el| {
                el.child(
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
                        .child(sort_header(GallerySortColumn::Filename))
                        .child(div().w(px(COL_SENDER_W)).flex_shrink_0().child("发件人"))
                        .child(div().w(px(COL_SUBJECT_W)).flex_shrink_0().child("邮件主题"))
                        .child(sort_header(GallerySortColumn::Size))
                        .child(sort_header(GallerySortColumn::Date))
                        .child(div().w(px(COL_ACTIONS_W)).flex_shrink_0().child("操作")),
                )
            })
            // 附件行 / 卡片网格（可滚动）。列表行点击即「定位到原邮件」；
            // 网格卡片：图片附件点击预览，其余定位到原邮件。
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
                    .when(self.view_grid, |el| {
                        // F-6 + WP-10 网格视图：真实图片缩略图 + 尺寸滑块动态宽高。
                        let card_w = self.thumb_size;
                        let card_h = (self.thumb_size * 0.82).max(96.0);
                        let thumb_h = (card_h - 46.0).max(44.0);
                        let cards: Vec<gpui_kit::Stateful<gpui_kit::Div>> = visible
                            .iter()
                            .map(|item| {
                                let icon = icon_for_mime(&item.mime_type);
                                let is_image = item.mime_type.to_lowercase().starts_with("image/");
                                let filename = item.filename.clone();
                                let preview_filename = filename.clone();
                                let size_text = if item.downloaded {
                                    format_size(item.size_bytes)
                                } else {
                                    format!("约 {} · 未下载", format_size(item.size_bytes))
                                };
                                let attachment_id = item.id;
                                let message_id = item.message_id;
                                let thumb = self.thumbnails.get(&attachment_id).cloned();
                                h_flex()
                                    .id(("gallery-card", attachment_id))
                                    .flex_col()
                                    .w(px(card_w))
                                    .h(px(card_h))
                                    .p_2()
                                    .gap_1()
                                    .rounded(px(10.0))
                                    .border_1()
                                    .border_color(border)
                                    .bg(surface)
                                    .cursor_pointer()
                                    .hover(move |s| s.bg(accent_soft))
                                    .items_center()
                                    .justify_between()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if is_image {
                                            // F-6：先进入加载中，字节回传后显示大图或失败重试。
                                            this.preview = Some(GalleryPreview {
                                                attachment_id,
                                                filename: preview_filename.clone(),
                                                status: GalleryPreviewStatus::Loading,
                                            });
                                            cx.emit(GalleryEvent::PreviewImage(attachment_id));
                                        } else {
                                            cx.emit(GalleryEvent::LocateMessage(message_id));
                                        }
                                        cx.notify();
                                    }))
                                    .child(if let Some(img_arc) = thumb {
                                        div()
                                            .w_full()
                                            .h(px(thumb_h))
                                            .rounded(px(6.0))
                                            .overflow_hidden()
                                            .child(
                                                img(img_arc)
                                                    .size_full()
                                                    .object_fit(ObjectFit::Cover),
                                            )
                                            .into_any_element()
                                    } else {
                                        h_flex()
                                            .w_full()
                                            .h(px(thumb_h))
                                            .items_center()
                                            .justify_center()
                                            .child(
                                                Icon::new(icon)
                                                    .with_size(px(
                                                        (thumb_h * 0.55).clamp(24.0, 44.0)
                                                    ))
                                                    .text_color(muted_fg),
                                            )
                                            .into_any_element()
                                    })
                                    .child(
                                        v_flex()
                                            .w_full()
                                            .items_center()
                                            .child(
                                                div()
                                                    .w_full()
                                                    .text_xs()
                                                    .text_color(text)
                                                    .text_center()
                                                    .truncate()
                                                    .child(SharedString::from(filename)),
                                            )
                                            .child(
                                                div()
                                                    .text_xs()
                                                    .text_color(muted_fg)
                                                    .child(SharedString::from(size_text)),
                                            ),
                                    )
                            })
                            .collect();
                        el.child(
                            div()
                                .flex()
                                .flex_wrap()
                                .p_2()
                                .gap_2()
                                .content_start()
                                .children(cards),
                        )
                    })
                    .when(!self.view_grid, |el| el.children(rows)),
            )
            // WP-10：底部 ActionBar（对齐原版 `src/ui/attachments_gallery.rs:329-485`）：
            // 视图切换 + 账号筛选 + 文件夹范围 Popover + 7 种文件类型筛选 + 缩略图尺寸滑块 + 数量统计。
            .child(
                h_flex()
                    .h(px(FOOTER_HEIGHT))
                    .flex_shrink_0()
                    .px_3()
                    .gap_2()
                    .items_center()
                    .border_t_1()
                    .border_color(border)
                    .bg(surface)
                    .child(
                        Button::new("gallery-footer-grid")
                            .ghost()
                            .compact()
                            .icon(IconName::LayoutGrid)
                            .tooltip("缩略图网格")
                            .selected(self.view_grid)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.view_grid = true;
                                this.request_visible_thumbnails(cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("gallery-footer-list")
                            .ghost()
                            .compact()
                            .icon(IconName::List)
                            .tooltip("列表表格")
                            .selected(!self.view_grid)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.view_grid = false;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("gallery-folders-scope")
                            .outline()
                            .compact()
                            .icon(IconName::Folder)
                            .label(if self.include_archive && self.include_other {
                                "全部文件夹"
                            } else if self.include_archive {
                                "收件箱 + 归档"
                            } else {
                                "仅收件箱"
                            })
                            .selected(self.folders_popover_open)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.folders_popover_open = !this.folders_popover_open;
                                cx.notify();
                            })),
                    )
                    .child({
                        let current_type = self.type_filter;
                        let entity_type = entity.clone();
                        Button::new("gallery-type-filter")
                            .outline()
                            .compact()
                            .label(current_type.label())
                            .icon(IconName::ChevronDown)
                            .dropdown_menu(move |mut menu, _, _| {
                                for kind in GalleryTypeFilter::ALL {
                                    let entity_item = entity_type.clone();
                                    menu = menu.item(
                                        PopupMenuItem::new(kind.label())
                                            .when(current_type == kind, |item| {
                                                item.icon(Icon::new(IconName::Check))
                                            })
                                            .on_click(move |_, _, cx| {
                                                entity_item.update(cx, |this, cx| {
                                                    this.type_filter = kind;
                                                    cx.notify();
                                                });
                                            }),
                                    );
                                }
                                menu
                            })
                    })
                    .when(self.view_grid, |bar| {
                        let ratio = ((self.thumb_size - THUMB_SIZE_MIN)
                            / (THUMB_SIZE_MAX - THUMB_SIZE_MIN))
                            .clamp(0.0, 1.0);
                        bar.child(
                            h_flex()
                                .ml_2()
                                .gap_1()
                                .items_center()
                                .child(
                                    Button::new("gallery-thumb-smaller")
                                        .ghost()
                                        .compact()
                                        .label("－")
                                        .tooltip("缩小缩略图")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.thumb_size = (this.thumb_size - THUMB_SIZE_STEP)
                                                .clamp(THUMB_SIZE_MIN, THUMB_SIZE_MAX);
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    div()
                                        .w(px(72.0))
                                        .h(px(6.0))
                                        .rounded(px(9999.0))
                                        .bg(border)
                                        .overflow_hidden()
                                        .child(div().w(px(72.0 * ratio)).h_full().bg(p.accent)),
                                )
                                .child(
                                    Button::new("gallery-thumb-larger")
                                        .ghost()
                                        .compact()
                                        .label("＋")
                                        .tooltip("放大缩略图")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.thumb_size = (this.thumb_size + THUMB_SIZE_STEP)
                                                .clamp(THUMB_SIZE_MIN, THUMB_SIZE_MAX);
                                            cx.notify();
                                        })),
                                )
                                .child(div().text_xs().text_color(muted_fg).child(
                                    SharedString::from(format!("{:.0}px", self.thumb_size)),
                                )),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted_fg)
                            .child(SharedString::from(format!(
                                "显示 {} / 共 {} 个附件",
                                visible.len(),
                                total
                            ))),
                    ),
            )
            // WP-10 文件夹范围 Popover（位于左下角 ActionBar 上方）。
            .when(self.folders_popover_open, |root| {
                root.child(
                    v_flex()
                        .absolute()
                        .bottom(px(FOOTER_HEIGHT + 8.0))
                        .left(px(96.0))
                        .w(px(260.0))
                        .p_3()
                        .gap_2()
                        .rounded(px(10.0))
                        .border_1()
                        .border_color(border)
                        .bg(p.surface_raised)
                        .shadow_lg()
                        .child(
                            h_flex()
                                .justify_between()
                                .items_center()
                                .child(
                                    div()
                                        .text_xs()
                                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                        .text_color(text)
                                        .child("附件扫描文件夹范围"),
                                )
                                .child(
                                    Button::new("gallery-folders-close")
                                        .ghost()
                                        .compact()
                                        .icon(IconName::Close)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.folders_popover_open = false;
                                            cx.notify();
                                        })),
                                ),
                        )
                        .child(
                            Button::new("gallery-scope-archive")
                                .outline()
                                .compact()
                                .w_full()
                                .selected(self.include_archive)
                                .label(if self.include_archive {
                                    "✓ 包含「归档」文件夹"
                                } else {
                                    "包含「归档」文件夹"
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.include_archive = !this.include_archive;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("gallery-scope-other")
                                .outline()
                                .compact()
                                .w_full()
                                .selected(self.include_other)
                                .label(if self.include_other {
                                    "✓ 包含其他自定义文件夹"
                                } else {
                                    "包含其他自定义文件夹"
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.include_other = !this.include_other;
                                    cx.notify();
                                })),
                        ),
                )
            })
            // F-6 图片预览浮层（页内居中，遮罩点击关闭）。
            .relative()
            .when(self.preview.is_some(), |root| root.child(preview_overlay))
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

    #[test]
    fn image_format_for_mime_maps_common_bitmaps() {
        assert_eq!(image_format_for_mime("image/png"), Some(ImageFormat::Png));
        assert_eq!(image_format_for_mime("IMAGE/JPEG"), Some(ImageFormat::Jpeg));
        assert_eq!(image_format_for_mime("image/gif"), Some(ImageFormat::Gif));
        assert_eq!(image_format_for_mime("image/webp"), Some(ImageFormat::Webp));
        // SVG 不支持预览渲染，回退占位说明。
        assert_eq!(image_format_for_mime("image/svg+xml"), None);
        assert_eq!(image_format_for_mime("application/pdf"), None);
    }

    #[test]
    fn type_filter_categorizes_seven_types() {
        assert!(GalleryTypeFilter::Images.matches("image/png", "photo.png"));
        assert!(GalleryTypeFilter::Pdfs.matches("application/pdf", "doc.pdf"));
        assert!(GalleryTypeFilter::Documents.matches("text/plain", "notes.txt"));
        assert!(GalleryTypeFilter::Archives.matches("application/zip", "bundle.zip"));
        assert!(GalleryTypeFilter::AudioVideo.matches("video/mp4", "clip.mp4"));
        assert!(GalleryTypeFilter::Other.matches("application/octet-stream", "data.bin"));
        assert!(!GalleryTypeFilter::Images.matches("application/pdf", "doc.pdf"));
    }
}
