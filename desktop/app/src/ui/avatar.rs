//! 发件人双字母圆形彩色头像。
//!
//! 规则（方案文档第二节第 2 条，对齐 Linux 版 `src/ui/initials.rs`）：
//! - 显示名首字符：中文取 1 字（如「张三」→「张」），英文取前两个单词
//!   首字母（`Marcus Chen` → `MC`，与 Linux 版 `initials_of` 一样取首词
//!   与末词的首字母）。
//! - 对邮箱地址做 FNV-1a 哈希，从 10 个柔和高饱和预设色中稳定取背景色，
//!   文字固定白色。
//! - 圆形尺寸支持 36x36（列表）与 34x34（阅读卡片）等。

use gpui_kit::{div, hsla, px, white, Div, FontWeight, Hsla, ParentElement, SharedString, Styled};

/// 头像文字高度与圆盘尺寸的比例（双字母时，与 Linux 版 0.42 对齐）。
const AVATAR_TEXT_SCALE: f32 = 0.42;

/// FNV-1a（32 位）哈希。
fn fnv1a(text: &str) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in text.bytes() {
        hash ^= u32::from(byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// 头像背景预设色：10 个柔和高饱和色（取自 libadwaita 头像渐变的深端，
/// 与 Linux 版同一套观感），文字固定白色。
const AVATAR_COLORS: [&str; 10] = [
    "#337fdc", "#0f9ac8", "#29ae74", "#6ab85b", "#d29d09", "#d68400", "#ed5b00", "#e62d42",
    "#e33b6a", "#9945b5",
];

/// 对邮箱地址（无邮箱时用显示名）做 FNV-1a 哈希，稳定取背景色。
pub fn avatar_bg_color(key: &str) -> Hsla {
    let hex = AVATAR_COLORS[(fnv1a(key) % AVATAR_COLORS.len() as u32) as usize];
    let value = u32::from_str_radix(&hex[1..], 16).expect("avatar palette hex must be valid");
    rgb_to_hsla(value)
}

/// 简化 sRGB → HSL 转换（预设色不透明、无色相丢失场景）。
fn rgb_to_hsla(value: u32) -> Hsla {
    let r = ((value >> 16) & 0xff) as f32 / 255.0;
    let g = ((value >> 8) & 0xff) as f32 / 255.0;
    let b = (value & 0xff) as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if max == min {
        return hsla(0.0, 0.0, l, 1.0);
    }
    let d = max - min;
    let s = if l > 0.5 {
        d / (2.0 - max - min)
    } else {
        d / (max + min)
    };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    hsla(h / 6.0, s, l, 1.0)
}

/// 单词首字母列表：跳过括号 / 尖括号内容（`Name <a@b.c>` 只算 Name），
/// 取每个含字母数字单词的首个字母或数字。
fn word_initials(text: &str) -> Vec<char> {
    let outside = {
        let mut out = String::new();
        let mut depth = 0usize;
        for c in text.chars() {
            match c {
                '(' | '<' | '[' => depth += 1,
                ')' | '>' | ']' => {
                    if depth > 0 {
                        depth -= 1;
                    }
                }
                _ if depth > 0 => {}
                _ => out.push(c),
            }
        }
        if out.chars().any(|c| c.is_alphanumeric()) {
            out
        } else {
            text.to_string()
        }
    };
    outside
        .split_whitespace()
        .filter_map(|w| w.chars().find(|c| c.is_alphanumeric()))
        .collect()
}

/// 首字符为 CJK 表意文字（中文取 1 字规则）。
fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0xF900..=0xFAFF | 0x30000..=0x3134F)
}

/// 发件人显示名 → 头像首字符。
pub fn sender_initials(name: &str) -> String {
    let trimmed = name.trim();
    if let Some(first) = trimmed.chars().find(|c| c.is_alphanumeric()) {
        if is_cjk(first) {
            return first.to_string();
        }
    }
    let letters = word_initials(&trimmed.to_uppercase());
    let mut out = String::new();
    if let Some(&first) = letters.first() {
        out.push(first);
        // 避免中英文混排拼出 "M帐" 这类怪异组合：首字母为 ASCII 时，
        // 仅在后续词首同样为 ASCII 字母数字时取末词首字母。
        if first.is_ascii_alphanumeric() {
            if let Some(&last_ascii) = letters[1..]
                .iter()
                .rev()
                .find(|c| c.is_ascii_alphanumeric())
            {
                out.push(last_ascii);
            }
        } else if letters.len() > 1 {
            out.extend(letters.last());
        }
    }
    if out.is_empty() {
        "?".to_string()
    } else {
        out
    }
}

/// 圆形头像：`size` x `size`（如 36.0 列表 / 34.0 阅读卡片）。
///
/// `name` 优先于 `email`（显示名首字符）；取色键优先 `email`。
pub fn avatar(name: &str, email: &str, size: f32) -> Div {
    let initials = sender_initials(name);
    let color_key = if email.trim().is_empty() { name } else { email };
    let bg = avatar_bg_color(color_key);
    let font_size = size * AVATAR_TEXT_SCALE;
    div()
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .rounded_full()
        .size(px(size))
        .bg(bg)
        .text_color(white())
        .text_size(px(font_size))
        .font_weight(FontWeight::BOLD)
        .child(SharedString::from(initials))
}

/// 账号头像（左栏账号分组），圆盘 + 右上角未读小蓝圈。
pub fn account_avatar(
    display_name: &str,
    email: &str,
    size: f32,
    unread: u32,
    accent: Hsla,
    border: Hsla,
) -> Div {
    let base = avatar(display_name, email, size);
    if unread == 0 {
        return base;
    }
    let badge = size * 0.5;
    let mut wrapper = div().relative().flex_shrink_0();
    wrapper = wrapper.child(base);
    wrapper = wrapper.child(
        div()
            .absolute()
            .top(px(-badge * 0.3))
            .right(px(-badge * 0.3))
            .rounded_full()
            .min_w(px(badge))
            .min_h(px(badge))
            .bg(accent)
            .border_1()
            .border_color(border)
            .flex()
            .items_center()
            .justify_center()
            .px_0p5()
            .text_size(px(badge * 0.55))
            .text_color(white())
            .font_weight(FontWeight::BOLD)
            .child(SharedString::from(if unread > 99 {
                "99+".to_string()
            } else {
                unread.to_string()
            })),
    );
    wrapper
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initials_follow_the_avatar_rules() {
        assert_eq!(sender_initials("Marcus Chen"), "MC");
        assert_eq!(sender_initials("  sophie   turner "), "ST");
        assert_eq!(sender_initials("Apple"), "A");
        assert_eq!(sender_initials("张三"), "张");
        assert_eq!(sender_initials("  李雷  "), "李");
        assert_eq!(sender_initials("Microsoft 帐户团队"), "M");
        assert_eq!(sender_initials("Marcus Chen <marcus@studio.dev>"), "MC");
        assert_eq!(sender_initials("Marco Trevisan (3v1n0)"), "MT");
        assert_eq!(sender_initials("--- ..."), "?");
        assert_eq!(sender_initials(""), "?");
    }

    #[test]
    fn avatar_color_is_stable_per_key() {
        let a = avatar_bg_color("marcus@studio.dev");
        let b = avatar_bg_color("marcus@studio.dev");
        assert_eq!(a.h, b.h);
        assert_eq!(a.s, b.s);
        assert_eq!(a.l, b.l);
        // 任一取色都落在预设色表内。
        let c = avatar_bg_color("priya@studio.dev");
        let matched = AVATAR_COLORS.iter().any(|hex| {
            let value = u32::from_str_radix(&hex[1..], 16).unwrap();
            approx_hsla(rgb_to_hsla(value), c)
        });
        assert!(matched);
    }

    fn approx_hsla(a: Hsla, b: Hsla) -> bool {
        (a.h - b.h).abs() < 1e-4 && (a.s - b.s).abs() < 1e-4 && (a.l - b.l).abs() < 1e-4
    }
}
