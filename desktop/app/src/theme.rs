//! Hylki 视觉主题系统。
//!
//! 定义《Hylki Windows 全页面UI实施方案》第二节中的语义色板
//! [`HylkiPalette`]（亮/暗两套），并从 Linux 版
//! `src/desktop/theme_palettes.rs` 复刻其余全部色板（rose / forest /
//! tidal / earth / midnight）。
//!
//! 对接 gpui-kit 主题机制：[`apply`] 先用 `Theme::change` 切换亮/暗模式
//!（gpui-kit 会重载该模式的注册主题），再用 `Theme::update` 用
//! [`theme_color_for`] 生成的 `ThemeColor` 覆盖组件层颜色，使 Button、
//! Input 等 gpui-kit 组件与 Hylki 语义色板保持一致，最后把当前
//! [`HylkiPalette`] 存入全局 [`HylkiThemeState`] 供三栏渲染读取。
//!
//! 避坑（文档第六节第 4 条）：`cx.theme()` / 全局读取均为不可变借用，
//! render 函数开头应先通过 [`current`] 把需要的 `Hsla`（`Copy`）提取为
//! 局部变量，不得跨 `cx.listener(...)` 或 `&mut Context` 使用。

use gpui_kit::component::{Theme, ThemeColor, ThemeMode};
use gpui_kit::{black, rgb, white, App, Global, Hsla};

/// 内置主题列表（id, 显示名）；偏好设置页（preferences.rs）读取该列表
/// 渲染色板选择器，持久化的 theme_id 经 [`static_theme_id`] 归一化。
pub const THEME_LIST: &[(&str, &str)] = &[
    ("default", "经典蓝"),
    ("rose", "玫瑰"),
    ("forest", "森林"),
    ("tidal", "海潮"),
    ("earth", "大地"),
    ("midnight", "子夜"),
];

/// Hylki 语义色板（`Copy`，可在 render 开头整体复制为局部变量）。
#[derive(Debug, Clone, Copy)]
pub struct HylkiPalette {
    /// 窗口底色、右栏阅读区背景。
    pub canvas: Hsla,
    /// 左栏背景。
    pub sidebar: Hsla,
    /// 中栏列表背景。
    pub surface: Hsla,
    /// 右栏邮件正文卡片、弹窗卡片。
    pub surface_raised: Hsla,
    /// 「写邮件」主按钮、选中邮件卡片背景、未读角标。
    pub accent: Hsla,
    /// `accent` 上的文字与图标。
    pub accent_fg: Hsla,
    /// 主正文与标题。
    pub text: Hsla,
    /// 次要信息、时间、摘要。
    pub text_muted: Hsla,
    /// 三栏分割线、卡片细边框。
    pub border: Hsla,
    /// 删除、认证错误、Outbox 待确认告警。
    pub danger: Hsla,
}

impl HylkiPalette {
    /// `accent` 的柔和底色（选中态悬停、胶囊标签底、列表行悬停）。
    pub fn accent_soft(&self) -> Hsla {
        self.accent.opacity(0.12)
    }
}

/// 星标琥珀金十六进制值（与亮暗主题无关，对齐 Linux 原版 `#e5a50a`）。
pub(crate) const STAR_GOLD: u32 = 0xe5_a5_0a;

/// 星标琥珀金 [`Hsla`]（`rgb()` 转换非常量求值，故以函数提供便捷访问）。
pub(crate) fn star_gold() -> Hsla {
    Hsla::from(rgb(STAR_GOLD))
}

/// 每套色板的十六进制语义值（亮 / 暗）。
#[derive(Clone, Copy)]
struct PaletteHex {
    canvas: &'static str,
    sidebar: &'static str,
    surface: &'static str,
    surface_raised: &'static str,
    accent: &'static str,
    accent_fg: &'static str,
    text: &'static str,
    text_muted: &'static str,
    border: &'static str,
    danger: &'static str,
}

impl PaletteHex {
    fn to_palette(self) -> HylkiPalette {
        HylkiPalette {
            canvas: hx(self.canvas),
            sidebar: hx(self.sidebar),
            surface: hx(self.surface),
            surface_raised: hx(self.surface_raised),
            accent: hx(self.accent),
            accent_fg: hx(self.accent_fg),
            text: hx(self.text),
            text_muted: hx(self.text_muted),
            border: hx(self.border),
            danger: hx(self.danger),
        }
    }
}

/// `#rrggbb` → `Hsla`。
fn hx(hex: &str) -> Hsla {
    let value = u32::from_str_radix(hex.trim_start_matches('#'), 16)
        .expect("theme palette hex must be valid");
    Hsla::from(rgb(value))
}

/// 经典蓝（方案文档第二节参考值）。
const DEFAULT_LIGHT: PaletteHex = PaletteHex {
    canvas: "#f5f6f8",
    sidebar: "#eef0f4",
    surface: "#ffffff",
    surface_raised: "#ffffff",
    accent: "#3584e4",
    accent_fg: "#ffffff",
    text: "#1e232d",
    text_muted: "#667085",
    border: "#e2e5ec",
    danger: "#e01b24",
};
const DEFAULT_DARK: PaletteHex = PaletteHex {
    canvas: "#1c1e24",
    sidebar: "#23262e",
    surface: "#282b34",
    surface_raised: "#2e323c",
    accent: "#3584e4",
    accent_fg: "#ffffff",
    text: "#eef2f8",
    text_muted: "#9aa4b8",
    border: "#363b47",
    danger: "#f66151",
};

/// 玫瑰（复刻自 `src/desktop/theme_palettes.rs` 的 ROSE）。
const ROSE_LIGHT: PaletteHex = PaletteHex {
    canvas: "#fdf7fd",
    sidebar: "#f2e1f4",
    surface: "#faf3fb",
    surface_raised: "#fdfafd",
    accent: "#b75077",
    accent_fg: "#ffffff",
    text: "#501854",
    text_muted: "#ac1668",
    border: "#eee1ed",
    danger: "#f7086c",
};
const ROSE_DARK: PaletteHex = PaletteHex {
    canvas: "#291921",
    sidebar: "#39202c",
    surface: "#291921",
    surface_raised: "#43343c",
    accent: "#a63962",
    accent_fg: "#fbd0e8",
    text: "#f9f8fb",
    text_muted: "#e7d0dd",
    border: "#654153",
    danger: "#9d174d",
};

/// 森林（复刻自 `src/desktop/theme_palettes.rs` 的 FOREST）。
const FOREST_LIGHT: PaletteHex = PaletteHex {
    canvas: "#f3f7f4",
    sidebar: "#e2ede7",
    surface: "#f3f7f4",
    surface_raised: "#ecefed",
    accent: "#1b7d50",
    accent_fg: "#fffaff",
    text: "#241523",
    text_muted: "#746c73",
    border: "#cbd5d1",
    danger: "#fb2c36",
};
const FOREST_DARK: PaletteHex = PaletteHex {
    canvas: "#1b2821",
    sidebar: "#21362b",
    surface: "#1b2821",
    surface_raised: "#36413c",
    accent: "#3eb272",
    accent_fg: "#241523",
    text: "#fffaff",
    text_muted: "#919595",
    border: "#415f4f",
    danger: "#fb414a",
};

/// 海潮（复刻自 `src/desktop/theme_palettes.rs` 的 TIDAL）。
const TIDAL_LIGHT: PaletteHex = PaletteHex {
    canvas: "#f5f7f8",
    sidebar: "#e4ecf2",
    surface: "#f5f7f8",
    surface_raised: "#edeff1",
    accent: "#2672af",
    accent_fg: "#fffaff",
    text: "#241523",
    text_muted: "#746c75",
    border: "#cdd4dc",
    danger: "#fb2c36",
};
const TIDAL_DARK: PaletteHex = PaletteHex {
    canvas: "#17212b",
    sidebar: "#1e2d3b",
    surface: "#17212b",
    surface_raised: "#333b45",
    accent: "#509ed8",
    accent_fg: "#241523",
    text: "#fffaff",
    text_muted: "#8d8f97",
    border: "#405567",
    danger: "#fb414a",
};

/// 大地（复刻自 `src/desktop/theme_palettes.rs` 的 EARTH）。
const EARTH_LIGHT: PaletteHex = PaletteHex {
    canvas: "#f9f7f5",
    sidebar: "#f3eae5",
    surface: "#f9f7f5",
    surface_raised: "#f1efee",
    accent: "#ae552a",
    accent_fg: "#fffaff",
    text: "#241523",
    text_muted: "#766c74",
    border: "#ddd2ce",
    danger: "#fb2c36",
};
const EARTH_DARK: PaletteHex = PaletteHex {
    canvas: "#291e1a",
    sidebar: "#39281f",
    surface: "#291e1a",
    surface_raised: "#433835",
    accent: "#ea8946",
    accent_fg: "#241523",
    text: "#fffaff",
    text_muted: "#968e8f",
    border: "#664c3f",
    danger: "#fb414a",
};

/// 子夜（复刻自 `src/desktop/theme_palettes.rs` 的 MIDNIGHT）。
const MIDNIGHT_LIGHT: PaletteHex = PaletteHex {
    canvas: "#f8f7f9",
    sidebar: "#edeaf4",
    surface: "#f8f7f9",
    surface_raised: "#f0eff2",
    accent: "#7253b9",
    accent_fg: "#fffaff",
    text: "#241523",
    text_muted: "#766c76",
    border: "#d6d1de",
    danger: "#fb2c36",
};
const MIDNIGHT_DARK: PaletteHex = PaletteHex {
    canvas: "#1d1929",
    sidebar: "#272139",
    surface: "#1d1929",
    surface_raised: "#383443",
    accent: "#5d41a6",
    accent_fg: "#ffffff",
    text: "#fffaff",
    text_muted: "#8e8a95",
    border: "#4d4366",
    danger: "#fb414a",
};

/// 全部内置色板（id, 亮, 暗），顺序与 [`THEME_LIST`] 一致。
const PALETTES: &[(&str, PaletteHex, PaletteHex)] = &[
    ("default", DEFAULT_LIGHT, DEFAULT_DARK),
    ("rose", ROSE_LIGHT, ROSE_DARK),
    ("forest", FOREST_LIGHT, FOREST_DARK),
    ("tidal", TIDAL_LIGHT, TIDAL_DARK),
    ("earth", EARTH_LIGHT, EARTH_DARK),
    ("midnight", MIDNIGHT_LIGHT, MIDNIGHT_DARK),
];

/// 按 id 与亮暗取语义色板；未知 id 回落到经典蓝。
pub fn palette_for(theme_id: &str, dark: bool) -> HylkiPalette {
    let (_, light, dark_hex) = PALETTES
        .iter()
        .find(|(id, _, _)| *id == theme_id)
        .copied()
        .unwrap_or(PALETTES[0]);
    let hex = if dark { dark_hex } else { light };
    hex.to_palette()
}

/// 当前会话的主题状态（全局）。
pub struct HylkiThemeState {
    pub palette: HylkiPalette,
}

/// 把持久化的 theme_id 归一化为内置列表里的 `&'static str`（未知回落经典蓝）。
pub fn static_theme_id(id: &str) -> &'static str {
    THEME_LIST
        .iter()
        .find(|(known, _)| *known == id)
        .map(|(known, _)| *known)
        .unwrap_or("default")
}

impl Global for HylkiThemeState {}

/// 读取当前语义色板（`Copy` 出来，避免跨借用边界持有引用）。
pub fn current(cx: &App) -> HylkiPalette {
    match cx.try_global::<HylkiThemeState>() {
        Some(state) => state.palette,
        None => palette_for("default", false),
    }
}

/// 安装主题：切换 gpui-kit 亮/暗模式后覆盖组件层颜色，并记录全局状态。
///
/// 注意：`Theme::change` 会重载该模式的注册主题，因此颜色必须在
/// `Theme::change` 之后的第二次 `Theme::update` 中覆盖（见 gpui-kit
/// `Theme::update` 文档说明）。
pub fn apply(theme_id: &'static str, dark: bool, cx: &mut App) {
    let palette = palette_for(theme_id, dark);
    let mode = if dark {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    };
    Theme::change(mode, None, cx);
    let colors = theme_color_for(palette, dark);
    Theme::update(cx, |theme| theme.colors = colors);
    cx.set_global(HylkiThemeState { palette });
}

/// 由语义色板构造 gpui-kit `ThemeColor`，使 Button / Input / Popover 等
/// 组件与 Hylki 色板一致。
fn theme_color_for(p: HylkiPalette, dark: bool) -> ThemeColor {
    let mut c = (*if dark {
        ThemeColor::dark()
    } else {
        ThemeColor::light()
    })
    .clone();

    c.background = p.canvas;
    c.foreground = p.text;
    c.border = p.border;
    c.input = p.border;
    c.ring = p.accent;

    // 主色（写邮件按钮、选中邮件卡片、未读角标）。
    c.primary = p.accent;
    c.primary_foreground = p.accent_fg;
    c.primary_hover = p.accent.blend(if dark {
        white().opacity(0.14)
    } else {
        black().opacity(0.12)
    });
    c.primary_active = p.accent.blend(if dark {
        white().opacity(0.24)
    } else {
        black().opacity(0.22)
    });
    c.button_primary = p.accent;
    c.button_primary_foreground = p.accent_fg;
    c.button_primary_hover = c.primary_hover;
    c.button_primary_active = c.primary_active;

    // 次级表面与文字。
    c.secondary = p.surface;
    c.secondary_hover = p.accent_soft();
    c.secondary_active = p.accent_soft();
    c.secondary_foreground = p.text;
    c.muted = p.surface;
    c.muted_foreground = p.text_muted;
    c.accent = p.accent_soft();
    c.accent_foreground = p.text;

    // 危险色。
    c.danger = p.danger;
    c.danger_foreground = p.accent_fg;
    c.danger_hover = p.danger.opacity(0.85);
    c.danger_active = p.danger.opacity(0.7);
    c.button_danger = p.danger;
    c.button_danger_foreground = p.accent_fg;
    c.button_danger_hover = c.danger_hover;
    c.button_danger_active = c.danger_active;

    // 三栏与卡片。
    c.sidebar = p.sidebar;
    c.sidebar_foreground = p.text;
    c.sidebar_border = p.border;
    c.sidebar_accent = p.accent_soft();
    c.sidebar_accent_foreground = p.text;
    c.popover = p.surface_raised;
    c.popover_foreground = p.text;
    c.title_bar = p.sidebar;
    c.title_bar_border = p.border;
    c.status_bar = p.sidebar;
    c.status_bar_border = p.border;
    c.list = p.surface;
    c.list_hover = p.accent_soft();
    c.list_active = p.accent_soft();
    c.list_active_border = p.accent;

    c.selection = p.accent.opacity(0.25);
    c.link = p.accent;
    c.link_hover = c.primary_hover;
    c.window_border = p.border;

    c
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Hsla` 未实现 `PartialEq`（f32 字段），用近似比较。
    fn approx(a: Hsla, b: Hsla) -> bool {
        (a.h - b.h).abs() < 1e-4
            && (a.s - b.s).abs() < 1e-4
            && (a.l - b.l).abs() < 1e-4
            && (a.a - b.a).abs() < 1e-4
    }

    #[test]
    fn palettes_parse_and_cover_all_ids() {
        assert_eq!(THEME_LIST.len(), PALETTES.len());
        for ((id, _), (pid, _, _)) in THEME_LIST.iter().zip(PALETTES.iter()) {
            assert_eq!(id, pid);
        }
        for (id, light, dark) in PALETTES.iter().copied() {
            for hex in [light.canvas, light.accent, dark.border, dark.danger] {
                assert_eq!(hex.len(), 7, "hex {hex} of theme {id} must be #rrggbb");
                assert!(u32::from_str_radix(&hex[1..], 16).is_ok());
            }
        }
    }

    #[test]
    fn unknown_theme_id_falls_back_to_default() {
        let fallback = palette_for("default", false);
        assert!(approx(
            palette_for("missing", false).accent,
            fallback.accent
        ));
        assert!(approx(
            palette_for("missing", true).canvas,
            DEFAULT_DARK.to_palette().canvas
        ));
    }

    #[test]
    fn default_matches_spec_values() {
        let light = palette_for("default", false);
        assert!(approx(light.accent, hx("#3584e4")));
        assert!(approx(light.canvas, hx("#f5f6f8")));
        assert!(approx(light.sidebar, hx("#eef0f4")));
        assert!(approx(light.text_muted, hx("#667085")));
        let dark = palette_for("default", true);
        assert!(approx(dark.canvas, hx("#1c1e24")));
        assert!(approx(dark.surface_raised, hx("#2e323c")));
        assert!(approx(dark.text, hx("#eef2f8")));
        assert!(approx(dark.danger, hx("#f66151")));
    }

    #[test]
    fn accent_soft_keeps_alpha() {
        let p = palette_for("default", false);
        assert!(p.accent_soft().a < p.accent.a);
    }

    #[test]
    fn star_gold_is_theme_independent_amber() {
        // #e5a50a → Rgba(229/255, 165/255, 10/255)。
        let gold = star_gold();
        assert!(approx(Hsla::from(rgb(0xe5a50a)), gold));
        assert!(gold.a == 1.0);
    }
}
