//! 独立偏好设置窗口可复用控件（libadwaita 分组卡片、行高亮、下拉选择、3 列双态主题画廊）。

use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{h_flex, v_flex, Disableable, Icon, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    div, px, AnyElement, App, Context, ElementId, Entity, FontWeight, InteractiveElement,
    IntoElement, ParentElement, SharedString, StatefulInteractiveElement, Styled, Window,
};

use crate::app::HylkiApp;
use crate::theme::{self, HylkiPalette, THEME_LIST};
use crate::ui::preferences::SettingsWindow;

/// 窄窗自动折叠为单栏导航的宽度阈值（对齐 Linux 版 640px）。
pub(crate) const NARROW_BREAKPOINT: f32 = 640.0;
/// 通用设置卡片组最大宽度（对齐 Linux 版 PreferencesGroup 600px）。
pub(crate) const CARD_MAX_WIDTH: f32 = 600.0;
/// 宽卡片页面最大宽度（发件人白名单 / 账号卡片，对齐 Linux 版 640px）。
pub(crate) const CARDS_WIDE_MAX_WIDTH: f32 = 640.0;
/// 搜索结果定位后行背景高亮持续时间（对齐 Linux 版 1.6s）。
pub(crate) const SEARCH_HIGHLIGHT_MS: u64 = 1600;

/// Libadwaita 风格分组卡片容器：分组标题 + 可选说明位于卡片外侧，`12px` 圆角白底卡片内用 `1px` 细线分隔行。
pub(crate) fn setting_group(
    title: impl Into<SharedString>,
    description: Option<&str>,
    max_w: f32,
    rows: Vec<AnyElement>,
    p: &HylkiPalette,
) -> AnyElement {
    let mut card = v_flex()
        .w_full()
        .bg(p.surface_raised)
        .border_1()
        .border_color(p.border)
        .rounded(px(12.0))
        .overflow_hidden();

    let total = rows.len();
    for (idx, row) in rows.into_iter().enumerate() {
        card = card.child(row);
        if idx + 1 < total {
            card = card.child(div().w_full().h(px(1.0)).bg(p.border));
        }
    }

    v_flex()
        .w_full()
        .max_w(px(max_w))
        .gap_1p5()
        .child(
            div()
                .px_1()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.text)
                .child(title.into()),
        )
        .when_some(description, |el, desc| {
            el.child(
                div()
                    .px_1()
                    .text_xs()
                    .text_color(p.text_muted)
                    .child(SharedString::from(desc.to_string())),
            )
        })
        .child(card)
        .into_any_element()
}

/// 分组卡片内的标准设置行：左侧标题 + 可选副标题，右侧控件，支持搜索命中 `1.6s` 柔和强调底色。
pub(crate) fn setting_row(
    row_id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    description: Option<&str>,
    right_control: AnyElement,
    disabled: bool,
    highlighted: bool,
    p: &HylkiPalette,
) -> AnyElement {
    let highlight_bg = p.accent.opacity(0.18);
    let hover_bg = p.surface.opacity(0.65);
    let label_color = if disabled { p.text_muted } else { p.text };

    h_flex()
        .id(row_id)
        .w_full()
        .min_h(px(52.0))
        .px_4()
        .py_2p5()
        .gap_3()
        .items_center()
        .justify_between()
        .when(highlighted, |el| el.bg(highlight_bg))
        .when(!highlighted && !disabled, |el| {
            el.hover(move |s| s.bg(hover_bg))
        })
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(label_color)
                        .child(label.into()),
                )
                .when_some(description, |el, desc| {
                    el.child(
                        div()
                            .text_xs()
                            .text_color(p.text_muted)
                            .child(SharedString::from(desc.to_string())),
                    )
                }),
        )
        .child(div().flex_shrink_0().child(right_control))
        .into_any_element()
}

/// 分组卡片内的只读键值信息行（如数据目录、版本信息）。
pub(crate) fn info_row(
    row_id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    highlighted: bool,
    p: &HylkiPalette,
) -> AnyElement {
    let value_widget = div()
        .text_xs()
        .text_color(p.text_muted)
        .child(value.into())
        .into_any_element();
    setting_row(row_id, label, None, value_widget, false, highlighted, p)
}

/// 标准布尔开关控件，回调时保证写盘并刷新 UI。
pub(crate) fn bool_switch(
    id: &'static str,
    checked: bool,
    disabled: bool,
    on_toggle: impl Fn(bool, &mut Window, &mut App) + 'static,
) -> AnyElement {
    if disabled {
        return div()
            .opacity(0.45)
            .child(Switch::new(id).checked(checked))
            .into_any_element();
    }
    Switch::new(id)
        .checked(checked)
        .on_click(move |next: &bool, window, cx| {
            on_toggle(*next, window, cx);
        })
        .into_any_element()
}

/// 静态选项下拉按钮（对齐 Linux 版 ComboRow）。
pub(crate) fn combo_button<T: Copy + PartialEq + 'static>(
    id: &'static str,
    current_label: impl Into<SharedString>,
    options: &'static [(&'static str, T)],
    current: T,
    disabled: bool,
    on_select: impl Fn(T, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let handler = Arc::new(on_select);
    Button::new(id)
        .outline()
        .compact()
        .disabled(disabled)
        .label(current_label.into())
        .icon(IconName::ChevronDown)
        .dropdown_menu(move |mut menu, _, _| {
            for (label, value) in options {
                let value = *value;
                let h = Arc::clone(&handler);
                menu = menu.item(
                    PopupMenuItem::new(*label)
                        .checked(value == current)
                        .on_click(move |_, window, cx| {
                            h(value, window, cx);
                        }),
                );
            }
            menu
        })
        .into_any_element()
}

/// 动态选项下拉按钮（例如默认发件人列表）。
pub(crate) fn combo_button_dynamic(
    id: &'static str,
    current_label: impl Into<SharedString>,
    options: Vec<(SharedString, String)>,
    current: String,
    disabled: bool,
    on_select: impl Fn(String, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let handler = Arc::new(on_select);
    Button::new(id)
        .outline()
        .compact()
        .disabled(disabled)
        .label(current_label.into())
        .icon(IconName::ChevronDown)
        .dropdown_menu(move |mut menu, _, _| {
            for (label, value) in &options {
                let checked = *value == current;
                let value_cloned = value.clone();
                let h = Arc::clone(&handler);
                menu = menu.item(PopupMenuItem::new(label.clone()).checked(checked).on_click(
                    move |_, window, cx| {
                        h(value_cloned.clone(), window, cx);
                    },
                ));
            }
            menu
        })
        .into_any_element()
}

/// 单个 `70×50` 双态窗口缩略图：严格复刻 Linux 版 `theme_picker.rs`（`11px` 顶栏 + `24px` 侧栏 + 内容卡 + 强调色按钮）。
fn theme_mini_window(pal: &HylkiPalette) -> AnyElement {
    let sidebar_w = 22.0;
    v_flex()
        .w(px(70.0))
        .h(px(50.0))
        .rounded(px(6.0))
        .border_1()
        .border_color(pal.border)
        .bg(pal.canvas)
        .overflow_hidden()
        .child(
            h_flex()
                .h(px(11.0))
                .px(px(5.0))
                .bg(pal.sidebar)
                .border_b_1()
                .border_color(pal.border)
                .items_center()
                .justify_end()
                .gap(px(2.5))
                .child(
                    div()
                        .w(px(3.0))
                        .h(px(3.0))
                        .rounded_full()
                        .bg(pal.text_muted.opacity(0.45)),
                )
                .child(
                    div()
                        .w(px(3.0))
                        .h(px(3.0))
                        .rounded_full()
                        .bg(pal.text_muted.opacity(0.45)),
                )
                .child(
                    div()
                        .w(px(3.0))
                        .h(px(3.0))
                        .rounded_full()
                        .bg(pal.text_muted.opacity(0.65)),
                ),
        )
        .child(
            h_flex()
                .flex_1()
                .min_h_0()
                .p(px(4.0))
                .gap(px(4.0))
                .child(
                    v_flex()
                        .w(px(sidebar_w))
                        .h_full()
                        .gap(px(3.0))
                        .child(
                            div()
                                .w_full()
                                .h(px(7.0))
                                .rounded(px(3.0))
                                .bg(pal.accent_soft()),
                        )
                        .child(
                            div()
                                .w(px(14.0))
                                .h(px(3.0))
                                .rounded(px(1.5))
                                .bg(pal.text_muted.opacity(0.35)),
                        )
                        .child(
                            div()
                                .w(px(16.0))
                                .h(px(3.0))
                                .rounded(px(1.5))
                                .bg(pal.text_muted.opacity(0.25)),
                        ),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .h_full()
                        .rounded(px(4.0))
                        .bg(pal.surface_raised)
                        .border_1()
                        .border_color(pal.border)
                        .p(px(4.0))
                        .justify_between()
                        .child(
                            v_flex()
                                .gap(px(2.5))
                                .child(
                                    div()
                                        .w(px(22.0))
                                        .h(px(3.0))
                                        .rounded(px(1.5))
                                        .bg(pal.text.opacity(0.70)),
                                )
                                .child(
                                    div()
                                        .w(px(16.0))
                                        .h(px(3.0))
                                        .rounded(px(1.5))
                                        .bg(pal.text_muted.opacity(0.40)),
                                ),
                        )
                        .child(
                            h_flex().justify_end().child(
                                div().w(px(12.0)).h(px(5.0)).rounded(px(2.5)).bg(pal.accent),
                            ),
                        ),
                ),
        )
        .into_any_element()
}

/// 3 列双态（浅色 + 深色预览窗）主题画廊（对齐 Linux 顺序：`经典蓝 / 子夜 / 海潮 / 玫瑰 / 大地 / 森林`）。
pub(crate) fn theme_gallery(
    active_theme_id: &str,
    highlighted: bool,
    app_entity: Entity<HylkiApp>,
    p: &HylkiPalette,
    _cx: &mut Context<SettingsWindow>,
) -> AnyElement {
    let mut rows = v_flex().gap_2p5().w_full();
    for chunk in THEME_LIST.chunks(3) {
        let mut row = h_flex().gap_2p5().w_full();
        for (id, label) in chunk {
            let theme_id = *id;
            let selected = active_theme_id == theme_id;
            let light_pal = theme::palette_for(theme_id, false);
            let dark_pal = theme::palette_for(theme_id, true);
            let app_for_click = app_entity.clone();
            let card_bg = if selected {
                p.accent.opacity(0.10)
            } else {
                p.surface
            };
            let border_color = if selected { p.accent } else { p.border };
            let hover_bg = p.surface.opacity(0.75);

            row = row.child(
                v_flex()
                    .id(SharedString::from(format!("theme-card-{theme_id}")))
                    .flex_1()
                    .min_w_0()
                    .p_2p5()
                    .gap_2()
                    .rounded(px(10.0))
                    .bg(card_bg)
                    .when(selected, |el| el.border_2().border_color(border_color))
                    .when(!selected, |el| el.border_1().border_color(border_color))
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover_bg))
                    .on_click(move |_, _, cx| {
                        app_for_click.update(cx, |app, cx| {
                            app.mutate_preferences(cx, |prefs| {
                                prefs.theme_id = theme_id.to_string();
                            });
                        });
                    })
                    .child(
                        h_flex()
                            .justify_center()
                            .items_center()
                            .gap_1p5()
                            .child(theme_mini_window(&light_pal))
                            .child(theme_mini_window(&dark_pal)),
                    )
                    .child(
                        h_flex()
                            .justify_center()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(if selected {
                                        FontWeight::SEMIBOLD
                                    } else {
                                        FontWeight::MEDIUM
                                    })
                                    .text_color(if selected { p.accent } else { p.text })
                                    .child(SharedString::from(*label)),
                            )
                            .when(selected, |el| {
                                el.child(Icon::new(IconName::Check).xsmall().text_color(p.accent))
                            }),
                    ),
            );
        }
        rows = rows.child(row);
    }

    v_flex()
        .id("pref-theme-gallery")
        .w_full()
        .max_w(px(CARDS_WIDE_MAX_WIDTH))
        .gap_1p5()
        .when(highlighted, |el| {
            el.rounded(px(12.0)).bg(p.accent.opacity(0.14)).p_2()
        })
        .child(
            div()
                .px_1()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.text)
                .child("主题配色"),
        )
        .child(
            div()
                .px_1()
                .text_xs()
                .text_color(p.text_muted)
                .child("点击卡片即时切换主窗口与设置窗口的界面与按钮强调色"),
        )
        .child(
            div()
                .w_full()
                .p_3()
                .rounded(px(12.0))
                .bg(p.surface_raised)
                .border_1()
                .border_color(p.border)
                .child(rows),
        )
        .into_any_element()
}
