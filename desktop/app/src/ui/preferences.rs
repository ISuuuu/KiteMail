//! 偏好设置页（方案文档第三节页面 5 + 第五节 2）：
//! - 左分类 180px：常规 / 外观 / 阅读 / 数据与存储（可点击切换）。
//! - 右设置卡片 max-width 600px：
//!   - 常规：默认启动视图（unified / last）、后台自动刷新间隔（0 / 5 / 15 分钟）。
//!   - 外观：亮色 / 暗色 / 跟随系统（dark_mode）、主题色板（theme_id，全部
//!     [`crate::theme::THEME_LIST`]）、列表密度（compact_list）。
//!   - 阅读：每页加载数量（page_size）、远程图片默认关闭（block_remote_images）。
//!   - 数据与存储：数据目录路径、SQLite 缓存大小、草稿 / Outbox 数量、
//!     「清理正文缓存」与「关闭」按钮。
//!
//! 自包含组件：内部持有 [`AppPreferences`] 副本，任何改动都更新副本并发
//! [`PreferencesEvent::Changed`]（携带完整新值）；**不**直接修改全局主题，
//! 由集成者负责应用与持久化。

use std::path::PathBuf;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::Icon;
use gpui_kit::component::{h_flex, v_flex, Selectable, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    div, px, Context, EventEmitter, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled,
};
use hylki_mail_core::config::{self, AppPreferences};

use crate::theme;

/// 左分类栏宽度。
const CATEGORY_WIDTH: f32 = 180.0;
/// 分类行高度。
const CATEGORY_ROW_HEIGHT: f32 = 36.0;
/// 顶部栏高度。
const HEADER_HEIGHT: f32 = 48.0;
/// 设置卡片最大宽度。
const CARD_MAX_WIDTH: f32 = 600.0;

/// 设置分类（左栏可切换）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrefTab {
    General,
    Appearance,
    Reading,
    Storage,
}

impl PrefTab {
    const ALL: [PrefTab; 4] = [
        PrefTab::General,
        PrefTab::Appearance,
        PrefTab::Reading,
        PrefTab::Storage,
    ];

    fn label(self) -> &'static str {
        match self {
            PrefTab::General => "常规",
            PrefTab::Appearance => "外观",
            PrefTab::Reading => "阅读",
            PrefTab::Storage => "数据与存储",
        }
    }

    fn icon(self) -> IconName {
        match self {
            PrefTab::General => IconName::Settings,
            PrefTab::Appearance => IconName::Palette,
            PrefTab::Reading => IconName::BookOpen,
            PrefTab::Storage => IconName::HardDrive,
        }
    }
}

/// 偏好设置事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreferencesEvent {
    /// 任一设置改动（携带完整新值，集成者负责应用与持久化）。
    Changed(AppPreferences),
    /// 用户点击「关闭」。
    Closed,
    /// 用户点击「清理正文缓存」（保留账号与配置）。
    ClearBodyCache,
}

/// 偏好设置视图状态（独立 Entity，由 HylkiApp 挂载）。
pub struct PreferencesState {
    /// 内部持有的偏好副本（每次改动后整体发出）。
    prefs: AppPreferences,
    /// 当前展示的分类。
    active_tab: PrefTab,
    /// 草稿数量（由集成者通过 [`Self::set_storage_stats`] 填充）。
    drafts: usize,
    /// 发件箱待处理数量（同上）。
    outbox: usize,
    /// 数据目录（构造时确定）。
    data_dir: PathBuf,
}

impl EventEmitter<PreferencesEvent> for PreferencesState {}

impl PreferencesState {
    pub fn new(
        _window: &mut gpui_kit::Window,
        _cx: &mut Context<Self>,
        prefs: AppPreferences,
    ) -> Self {
        Self {
            prefs,
            active_tab: PrefTab::General,
            drafts: 0,
            outbox: 0,
            data_dir: config::data_dir(),
        }
    }

    /// 填充草稿 / 发件箱数量统计（数据与存储页展示）。
    pub fn set_storage_stats(&mut self, drafts: usize, outbox: usize) {
        self.drafts = drafts;
        self.outbox = outbox;
    }

    /// 当前偏好副本（供集成者在收到 Changed 前读取；当前集成链路由
    /// HylkiApp 持有偏好权威副本，此 getter 保留作组件语义）。
    #[allow(dead_code)]
    pub fn preferences(&self) -> &AppPreferences {
        &self.prefs
    }

    /// 更新副本并发 Changed(完整新值)。
    fn emit_changed(&mut self, cx: &mut Context<Self>) {
        let prefs = self.prefs.clone();
        cx.emit(PreferencesEvent::Changed(prefs));
        cx.notify();
    }

    fn set_default_view(&mut self, value: &str, cx: &mut Context<Self>) {
        if self.prefs.default_view != value {
            self.prefs.default_view = value.to_string();
            self.emit_changed(cx);
        }
    }

    fn set_auto_sync_minutes(&mut self, minutes: u32, cx: &mut Context<Self>) {
        if self.prefs.auto_sync_minutes != minutes {
            self.prefs.auto_sync_minutes = minutes;
            self.emit_changed(cx);
        }
    }

    fn set_dark_mode(&mut self, dark: Option<bool>, cx: &mut Context<Self>) {
        if self.prefs.dark_mode != dark {
            self.prefs.dark_mode = dark;
            self.emit_changed(cx);
        }
    }

    fn set_theme_id(&mut self, theme_id: &str, cx: &mut Context<Self>) {
        if self.prefs.theme_id != theme_id {
            self.prefs.theme_id = theme_id.to_string();
            self.emit_changed(cx);
        }
    }

    fn set_compact_list(&mut self, compact: bool, cx: &mut Context<Self>) {
        if self.prefs.compact_list != compact {
            self.prefs.compact_list = compact;
            self.emit_changed(cx);
        }
    }

    fn set_page_size(&mut self, size: usize, cx: &mut Context<Self>) {
        if self.prefs.page_size != size {
            self.prefs.page_size = size;
            self.emit_changed(cx);
        }
    }
}

/// 亮暗模式选项标签（渲染走 option_pills 内联文案；此助手仅单元测试引用）。
#[allow(dead_code)]
fn dark_mode_label(dark: Option<bool>) -> &'static str {
    match dark {
        Some(false) => "亮色",
        Some(true) => "暗色",
        None => "跟随系统",
    }
}

/// 启动视图选项标签（同上，仅单元测试引用）。
#[allow(dead_code)]
fn default_view_label(view: &str) -> &'static str {
    match view {
        "last" => "上次文件夹",
        _ => "统一收件箱",
    }
}

/// 自动刷新间隔选项标签（同上，仅单元测试引用）。
#[allow(dead_code)]
fn sync_interval_label(minutes: u32) -> &'static str {
    match minutes {
        5 => "5 分钟",
        15 => "15 分钟",
        _ => "手动",
    }
}

/// 设置卡片：标题 + 若干行。
fn setting_card(
    title: &str,
    p: &theme::HylkiPalette,
    rows: Vec<gpui_kit::AnyElement>,
) -> gpui_kit::AnyElement {
    let (surface_raised, text, border) = (p.surface_raised, p.text, p.border);
    v_flex()
        .w_full()
        .max_w(px(CARD_MAX_WIDTH))
        .gap_3()
        .px_3()
        .py_3()
        .bg(surface_raised)
        .rounded(px(12.0))
        .border_1()
        .border_color(border)
        .child(
            div()
                .text_sm()
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .text_color(text)
                .child(SharedString::from(title)),
        )
        .children(rows)
        .into_any_element()
}

/// 单行设置项：左侧标签（+ 可选说明）与右侧控件。
fn setting_row(
    label: &str,
    desc: Option<&str>,
    control: impl IntoElement,
    p: &theme::HylkiPalette,
) -> gpui_kit::AnyElement {
    let (text, muted_fg) = (p.text, p.text_muted);
    h_flex()
        .items_center()
        .justify_between()
        .gap_2()
        .child(
            v_flex()
                .min_w_0()
                .gap_0p5()
                .child(
                    div()
                        .text_sm()
                        .text_color(text)
                        .child(SharedString::from(label)),
                )
                .when_some(desc, |el, d| {
                    el.child(
                        div()
                            .text_xs()
                            .text_color(muted_fg)
                            .child(SharedString::from(d)),
                    )
                }),
        )
        .child(control)
        .into_any_element()
}

/// 选项胶囊组（单选）：当前值高亮。
fn option_pills<T: Copy + PartialEq + 'static>(
    id_prefix: &'static str,
    options: &[(T, &'static str)],
    current: T,
    cx: &mut Context<PreferencesState>,
    on_select: fn(&mut PreferencesState, T, &mut Context<PreferencesState>),
) -> gpui_kit::AnyElement {
    h_flex()
        .gap_1()
        .children(options.iter().enumerate().map(|(ix, (value, label))| {
            let value = *value;
            let is_selected = value == current;
            Button::new((id_prefix, ix))
                .ghost()
                .compact()
                .selected(is_selected)
                .label(*label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    on_select(this, value, cx);
                }))
        }))
        .into_any_element()
}

impl Render for PreferencesState {
    fn render(
        &mut self,
        _window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // 避坑清单第 4 条：render 开头先把语义色 Copy 成局部变量。
        let p = theme::current(cx);
        let (canvas, sidebar_bg, border, text, muted_fg, accent) = (
            p.canvas,
            p.sidebar,
            p.border,
            p.text,
            p.text_muted,
            p.accent,
        );

        let active_tab = self.active_tab;
        let prefs = self.prefs.clone();

        // 左分类栏。
        let categories = v_flex()
            .w(px(CATEGORY_WIDTH))
            .h_full()
            .flex_shrink_0()
            .bg(sidebar_bg)
            .border_r_1()
            .border_color(border)
            .child(
                h_flex()
                    .h(px(HEADER_HEIGHT))
                    .flex_shrink_0()
                    .px_3()
                    .items_center()
                    .border_b_1()
                    .border_color(border)
                    .child(
                        div()
                            .text_base()
                            .font_weight(gpui_kit::FontWeight::BOLD)
                            .text_color(text)
                            .child("偏好设置"),
                    ),
            )
            .child(
                v_flex()
                    .id("prefs-categories")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .px_2()
                    .py_2()
                    .gap_0p5()
                    .children(PrefTab::ALL.map(|tab| {
                        let is_active = tab == active_tab;
                        h_flex()
                            .id(SharedString::from(format!("prefs-tab-{}", tab.label())))
                            .h(px(CATEGORY_ROW_HEIGHT))
                            .px_2()
                            .gap_2()
                            .items_center()
                            .rounded(px(8.0))
                            .cursor_pointer()
                            .text_color(if is_active { accent } else { text })
                            .when(is_active, |row| {
                                row.bg(p.accent_soft())
                                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            })
                            .when(!is_active, |row| row.hover(|s| s.bg(p.accent_soft())))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.active_tab = tab;
                                cx.notify();
                            }))
                            .child(
                                Icon::new(tab.icon())
                                    .with_size(px(16.0))
                                    .text_color(if is_active { accent } else { muted_fg }),
                            )
                            .child(div().text_sm().child(SharedString::from(tab.label())))
                    })),
            );

        // 右侧按分类渲染设置卡片。
        let content: gpui_kit::AnyElement = match active_tab {
            PrefTab::General => {
                let default_view_pills = option_pills(
                    "pref-default-view",
                    &[("unified", "统一收件箱"), ("last", "上次文件夹")],
                    if prefs.default_view == "last" {
                        "last"
                    } else {
                        "unified"
                    },
                    cx,
                    PreferencesState::set_default_view,
                );
                let sync_pills = option_pills(
                    "pref-sync",
                    &[(0u32, "手动"), (5, "5 分钟"), (15, "15 分钟")],
                    prefs.auto_sync_minutes,
                    cx,
                    PreferencesState::set_auto_sync_minutes,
                );
                setting_card(
                    "常规",
                    &p,
                    vec![
                        setting_row(
                            "默认启动视图",
                            Some("启动 Hylki 时展示统一收件箱或上次浏览的文件夹"),
                            default_view_pills,
                            &p,
                        ),
                        setting_row(
                            "后台自动刷新间隔",
                            Some("按间隔在后台同步所有账号；「手动」仅手动刷新"),
                            sync_pills,
                            &p,
                        ),
                    ],
                )
            }
            PrefTab::Appearance => {
                // 主题色板选择：全部内置主题，圆点显示该主题 accent 色。
                let dark_for_swatch = prefs.dark_mode.unwrap_or(false);
                let swatches = h_flex().flex_wrap().gap_2().children(
                    theme::THEME_LIST
                        .iter()
                        .enumerate()
                        .map(|(ix, (theme_id, theme_label))| {
                            let theme_id = *theme_id;
                            let is_selected = prefs.theme_id == theme_id;
                            let swatch = theme::palette_for(theme_id, dark_for_swatch).accent;
                            h_flex()
                                .id(("pref-theme", ix))
                                .px_2()
                                .py_1()
                                .gap_1p5()
                                .items_center()
                                .rounded(px(8.0))
                                .cursor_pointer()
                                .border_1()
                                .border_color(if is_selected { accent } else { border })
                                .when(is_selected, |el| el.bg(p.accent_soft()))
                                .when(!is_selected, |el| el.hover(|s| s.bg(p.accent_soft())))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.set_theme_id(theme_id, cx);
                                }))
                                .child(
                                    div()
                                        .w(px(14.0))
                                        .h(px(14.0))
                                        .rounded_full()
                                        .bg(swatch)
                                        .border_1()
                                        .border_color(border),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(if is_selected { accent } else { text })
                                        .child(SharedString::from(*theme_label)),
                                )
                        }),
                );
                let mode_pills = option_pills(
                    "pref-dark-mode",
                    &[
                        (Some(false), "亮色"),
                        (Some(true), "暗色"),
                        (None, "跟随系统"),
                    ],
                    prefs.dark_mode,
                    cx,
                    PreferencesState::set_dark_mode,
                );
                let density_pills = option_pills(
                    "pref-density",
                    &[(false, "标准"), (true, "紧凑")],
                    prefs.compact_list,
                    cx,
                    PreferencesState::set_compact_list,
                );
                v_flex()
                    .w_full()
                    .max_w(px(CARD_MAX_WIDTH))
                    .gap_4()
                    .child(setting_card(
                        "外观",
                        &p,
                        vec![
                            setting_row(
                                "界面模式",
                                Some("亮色 / 暗色，或跟随系统"),
                                mode_pills,
                                &p,
                            ),
                            setting_row("主题色板", Some("选择内置主题配色"), swatches, &p),
                            setting_row(
                                "列表密度",
                                Some("紧凑模式显示更多邮件行"),
                                density_pills,
                                &p,
                            ),
                        ],
                    ))
                    .into_any_element()
            }
            PrefTab::Reading => {
                let page_size_pills = option_pills(
                    "pref-page-size",
                    &[(50usize, "50 条"), (100, "100 条"), (200, "200 条")],
                    prefs.page_size,
                    cx,
                    PreferencesState::set_page_size,
                );
                let entity = cx.entity();
                let remote_switch = Switch::new("pref-block-remote-images")
                    .checked(prefs.block_remote_images)
                    .on_click(move |checked, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.prefs.block_remote_images = *checked;
                            this.emit_changed(cx);
                        });
                    });
                setting_card(
                    "阅读",
                    &p,
                    vec![
                        setting_row(
                            "每页加载数量",
                            Some("邮件列表一次从缓存加载的邮件数"),
                            page_size_pills,
                            &p,
                        ),
                        setting_row(
                            "远程图片默认关闭",
                            Some("HTML 邮件默认不加载远程图片，保护隐私"),
                            remote_switch,
                            &p,
                        ),
                    ],
                )
            }
            PrefTab::Storage => {
                // 缓存数据库大小：不存在则显示「未创建」。
                let cache_size_text = match std::fs::metadata(config::cache_db_in(&self.data_dir)) {
                    Ok(meta) => format_size(meta.len() as usize),
                    Err(_) => "未创建".to_string(),
                };
                let info_row = |label: &str, value: String| {
                    h_flex()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .child(
                            div()
                                .text_sm()
                                .text_color(text)
                                .child(SharedString::from(label)),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted_fg)
                                .max_w(px(360.0))
                                .truncate()
                                .child(SharedString::from(value)),
                        )
                        .into_any_element()
                };
                setting_card(
                    "数据与存储",
                    &p,
                    vec![
                        info_row("数据目录", self.data_dir.display().to_string()),
                        info_row("SQLite 缓存文件大小", cache_size_text),
                        info_row("本地草稿数量", self.drafts.to_string()),
                        info_row("发件箱待处理数量", self.outbox.to_string()),
                        setting_row(
                            "正文缓存",
                            Some("清理已缓存的邮件正文，保留账号与配置"),
                            Button::new("pref-clear-body-cache")
                                .danger()
                                .label("清理正文缓存")
                                .on_click(cx.listener(|_, _, _, cx| {
                                    cx.emit(PreferencesEvent::ClearBodyCache);
                                    cx.notify();
                                })),
                            &p,
                        ),
                    ],
                )
            }
        };

        v_flex().size_full().bg(canvas).child(
            h_flex().flex_1().min_h_0().child(categories).child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    // 顶部 48px 栏：当前分类标题 + 全局关闭按钮（任意 Tab 均可直接关闭）。
                    .child(
                        h_flex()
                            .h(px(HEADER_HEIGHT))
                            .flex_shrink_0()
                            .px_6()
                            .items_center()
                            .justify_between()
                            .border_b_1()
                            .border_color(border)
                            .child(
                                div()
                                    .text_base()
                                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                    .text_color(text)
                                    .child(SharedString::from(active_tab.label())),
                            )
                            .child(
                                Button::new("pref-close")
                                    .ghost()
                                    .compact()
                                    .icon(IconName::X)
                                    .tooltip("关闭偏好设置 (Esc)")
                                    .on_click(cx.listener(|_, _, _, cx| {
                                        cx.emit(PreferencesEvent::Closed);
                                    })),
                            ),
                    )
                    .child(
                        v_flex()
                            .id("prefs-content")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scrollbar()
                            .px_6()
                            .py_4()
                            .gap_4()
                            .child(content),
                    ),
            ),
        )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mode_and_view_labels_are_visible() {
        assert_eq!(dark_mode_label(Some(false)), "亮色");
        assert_eq!(dark_mode_label(Some(true)), "暗色");
        assert_eq!(dark_mode_label(None), "跟随系统");
        assert_eq!(default_view_label("unified"), "统一收件箱");
        assert_eq!(default_view_label("last"), "上次文件夹");
        assert_eq!(default_view_label("unknown"), "统一收件箱");
        assert_eq!(sync_interval_label(0), "手动");
        assert_eq!(sync_interval_label(5), "5 分钟");
        assert_eq!(sync_interval_label(15), "15 分钟");
    }

    #[test]
    fn format_size_uses_b_kb_mb() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(2048), "2.0 KB");
        assert_eq!(format_size(3 * 1024 * 1024), "3.0 MB");
    }

    #[test]
    fn prefs_cover_all_theme_ids() {
        let prefs = AppPreferences::default();
        assert!(theme::THEME_LIST
            .iter()
            .any(|(id, _)| *id == prefs.theme_id));
    }
}
