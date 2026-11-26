//! 发件人双字母圆形彩色头像。
//!
//! 规则（方案文档第二节第 2 条，对齐 Linux 版 `src/ui/initials.rs`）：
//! - 显示名首字符：中文取 1 字（如「张三」→「张」），英文取前两个单词
//!   首字母（`Marcus Chen` → `MC`，与 Linux 版 `initials_of` 一样取首词
//!   与末词的首字母）。
//! - 对邮箱地址做 FNV-1a 哈希，从 10 个柔和高饱和预设色中稳定取背景色，
//!   文字固定白色。
//! - 圆形尺寸支持 36x36（列表）与 34x34（阅读卡片）等。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

use gpui_kit::{
    div, hsla, img, px, white, Div, FontWeight, Hsla, Image, ImageFormat, ObjectFit, ParentElement,
    SharedString, Styled, StyledImage,
};
use kitemail_mail_core::config::{self, AccountConfig};

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

/// 账号外观面板可选的 10 色强调色调色板（十六进制 + 中文名称，对齐 Linux 版）。
pub const ACCOUNT_ACCENT_COLORS: [(&str, &str); 10] = [
    ("#337fdc", "海洋蓝"),
    ("#0f9ac8", "青碧"),
    ("#29ae74", "翠绿"),
    ("#6ab85b", "草绿"),
    ("#d29d09", "琥珀"),
    ("#d68400", "暖橙"),
    ("#ed5b00", "橘红"),
    ("#e62d42", "绯红"),
    ("#e33b6a", "玫红"),
    ("#9945b5", "紫罗兰"),
];

/// 解析 `#rrggbb` 十六进制颜色为 [`Hsla`]。
pub fn parse_hex_color(hex: &str) -> Option<Hsla> {
    let trimmed = hex.trim();
    let body = trimmed.strip_prefix('#')?;
    if body.len() != 6 || !body.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let value = u32::from_str_radix(body, 16).ok()?;
    Some(rgb_to_hsla(value))
}

/// 返回账号最终强调色十六进制串：优先合法自定义色，否则按邮箱 FNV-1a 取默认色。
pub fn account_accent_hex(email: &str, custom_color: Option<&str>) -> String {
    if let Some(custom) = custom_color {
        let trimmed = custom.trim();
        if parse_hex_color(trimmed).is_some() {
            return trimmed.to_ascii_lowercase();
        }
    }
    AVATAR_COLORS[(fnv1a(email) % AVATAR_COLORS.len() as u32) as usize].to_string()
}

/// 返回账号最终强调色 [`Hsla`]。
pub fn account_accent_color(email: &str, custom_color: Option<&str>) -> Hsla {
    let hex = account_accent_hex(email, custom_color);
    parse_hex_color(&hex).unwrap_or_else(|| avatar_bg_color(email))
}

/// 对邮箱地址（无邮箱时用显示名）做 FNV-1a 哈希，稳定取背景色。
pub fn avatar_bg_color(key: &str) -> Hsla {
    let hex = AVATAR_COLORS[(fnv1a(key) % AVATAR_COLORS.len() as u32) as usize];
    let value = u32::from_str_radix(&hex[1..], 16).expect("avatar palette hex must be valid");
    rgb_to_hsla(value)
}

struct CachedAvatarEntry {
    checked_at: std::time::Instant,
    stamp: u64,
    image: Option<Arc<Image>>,
}

type AvatarImageCache = HashMap<PathBuf, CachedAvatarEntry>;

const AVATAR_STAT_TTL: std::time::Duration = std::time::Duration::from_secs(5);

fn avatar_image_cache() -> &'static Mutex<AvatarImageCache> {
    static CACHE: OnceLock<Mutex<AvatarImageCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

struct CachedSenderAvatar {
    checked_at: Instant,
    image: Option<Arc<Image>>,
}

const SENDER_AVATAR_CACHE_TTL: Duration = Duration::from_secs(10);
const SENDER_AVATAR_CACHE_MAX_ENTRIES: usize = 2048;

fn sender_avatar_cache() -> &'static Mutex<HashMap<(PathBuf, String), CachedSenderAvatar>> {
    static CACHE: OnceLock<Mutex<HashMap<(PathBuf, String), CachedSenderAvatar>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 解析发送者缓存头像（BIMI / Gravatar），使用短 TTL 内存缓存避免每次重绘做磁盘 stat / 读取和重复构建 Image。
fn cached_sender_avatar_image(email: &str) -> Option<Arc<Image>> {
    let data_dir = config::data_dir();
    let key = (data_dir.clone(), email.to_ascii_lowercase());
    let now = Instant::now();

    if let Ok(cache) = sender_avatar_cache().lock() {
        if let Some(entry) = cache.get(&key) {
            if now.duration_since(entry.checked_at) < SENDER_AVATAR_CACHE_TTL {
                return entry.image.clone();
            }
        }
    }

    let resolved =
        kitemail_mail_core::avatar::cached_avatar_path(&data_dir, email).and_then(|cached_path| {
            let format = image_format_for_path(&cached_path)?;
            let bytes = std::fs::read(&cached_path).ok()?;
            Some(Arc::new(Image::from_bytes(format, bytes)))
        });

    if let Ok(mut cache) = sender_avatar_cache().lock() {
        if cache.len() >= SENDER_AVATAR_CACHE_MAX_ENTRIES {
            cache.retain(|_, v| now.duration_since(v.checked_at) < SENDER_AVATAR_CACHE_TTL);
            if cache.len() >= SENDER_AVATAR_CACHE_MAX_ENTRIES {
                cache.clear();
            }
        }
        cache.insert(
            key,
            CachedSenderAvatar {
                checked_at: now,
                image: resolved.clone(),
            },
        );
    }

    resolved
}

fn image_format_for_path(path: &Path) -> Option<ImageFormat> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())?;
    match ext.as_str() {
        "png" => Some(ImageFormat::Png),
        "jpg" | "jpeg" => Some(ImageFormat::Jpeg),
        "webp" => Some(ImageFormat::Webp),
        "gif" => Some(ImageFormat::Gif),
        _ => None,
    }
}

/// 从本地 `avatars/` 目录安全加载用户自定义账号头像图片并做内存缓存。
pub fn load_local_avatar_image(data_dir: &Path, filename: &str) -> Option<Arc<Image>> {
    let path = config::avatar_path_in(data_dir, filename)?;
    let format = image_format_for_path(&path)?;
    let now = std::time::Instant::now();

    if let Ok(cache) = avatar_image_cache().lock() {
        if let Some(entry) = cache.get(&path) {
            if now.duration_since(entry.checked_at) < AVATAR_STAT_TTL {
                return entry.image.as_ref().map(Arc::clone);
            }
        }
    }

    let Ok(meta) = std::fs::metadata(&path) else {
        if let Ok(mut cache) = avatar_image_cache().lock() {
            cache.insert(
                path,
                CachedAvatarEntry {
                    checked_at: now,
                    stamp: 0,
                    image: None,
                },
            );
        }
        return None;
    };
    let modified_nanos = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let stamp = modified_nanos ^ meta.len().rotate_left(17);

    if let Ok(mut cache) = avatar_image_cache().lock() {
        if let Some(entry) = cache.get_mut(&path) {
            if entry.stamp == stamp && entry.image.is_some() {
                entry.checked_at = now;
                return entry.image.as_ref().map(Arc::clone);
            }
        }
    }

    let bytes = std::fs::read(&path).ok();
    let image = bytes.map(|b| Arc::new(Image::from_bytes(format, b)));
    if let Ok(mut cache) = avatar_image_cache().lock() {
        cache.insert(
            path,
            CachedAvatarEntry {
                checked_at: now,
                stamp,
                image: image.as_ref().map(Arc::clone),
            },
        );
    }
    image
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

/// 返回发件人头像的展示字符与背景十六进制颜色（与 [`avatar`] 规则一致，供 HTML 会话卡片复用）。
pub(crate) fn avatar_badge_spec(name: &str, email: &str) -> (String, &'static str) {
    let trimmed_email = email.trim();
    if !trimmed_email.is_empty() {
        if let Some(brand) = kitemail_mail_core::avatar::lookup_builtin_brand(trimmed_email) {
            return (brand.short.to_string(), brand.color);
        }
    }
    let initials = sender_initials(name);
    let color_key = if trimmed_email.is_empty() {
        name
    } else {
        trimmed_email
    };
    let bg = AVATAR_COLORS[(fnv1a(color_key) % AVATAR_COLORS.len() as u32) as usize];
    (initials, bg)
}

/// 圆形头像：`size` x `size`（如 36.0 列表 / 34.0 阅读卡片）。
///
/// 优先检查本地缓存的品牌 Logo / Gravatar 图片，其次匹配内置常见品牌图标，
/// 未命中则回退到发件人双字母彩色头像。
pub fn avatar(name: &str, email: &str, size: f32) -> Div {
    let trimmed_email = email.trim();
    if !trimmed_email.is_empty() {
        if let Some(image) = cached_sender_avatar_image(trimmed_email) {
            return div()
                .flex()
                .items_center()
                .justify_center()
                .flex_shrink_0()
                .rounded_full()
                .overflow_hidden()
                .size(px(size))
                .child(img(image).size_full().object_fit(ObjectFit::Cover));
        }
        if let Some(brand) = kitemail_mail_core::avatar::lookup_builtin_brand(trimmed_email) {
            let bg = parse_hex_color(brand.color).unwrap_or_else(|| avatar_bg_color(trimmed_email));
            return div()
                .flex()
                .items_center()
                .justify_center()
                .flex_shrink_0()
                .rounded_full()
                .size(px(size))
                .bg(bg)
                .text_color(white())
                .text_size(px(size * AVATAR_TEXT_SCALE))
                .font_weight(FontWeight::BOLD)
                .child(SharedString::from(brand.short));
        }
    }

    let initials = sender_initials(name);
    let color_key = if trimmed_email.is_empty() {
        name
    } else {
        trimmed_email
    };
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

/// 账号外观圆盘：外层强调色圆环 + `ring_gap_px` 留白 + 内层圆盘（图片 → Emoji → 首字母）。
#[allow(clippy::too_many_arguments)]
pub fn account_circle_visual(
    display_label: &str,
    email: &str,
    color_hex: Option<&str>,
    emoji: Option<&str>,
    avatar_file: Option<&str>,
    data_dir: &Path,
    size: f32,
    ring_px: f32,
    ring_gap_px: f32,
) -> Div {
    let accent = account_accent_color(email, color_hex);
    let mut inner = div()
        .size(px(size))
        .rounded_full()
        .overflow_hidden()
        .bg(accent)
        .flex()
        .items_center()
        .justify_center();

    if let Some(image) = avatar_file.and_then(|f| load_local_avatar_image(data_dir, f)) {
        inner = inner.child(img(image).size_full().object_fit(ObjectFit::Cover));
    } else if let Some(em) = emoji.map(str::trim).filter(|s| !s.is_empty()) {
        let glyph: String = em.chars().take(4).collect();
        inner = inner
            .text_color(white())
            .text_size(px(size * 0.52))
            .child(SharedString::from(glyph));
    } else {
        let label = if display_label.trim().is_empty() {
            email
        } else {
            display_label
        };
        let initials = sender_initials(label);
        inner = inner
            .text_color(white())
            .text_size(px(size * AVATAR_TEXT_SCALE))
            .font_weight(FontWeight::BOLD)
            .child(SharedString::from(initials));
    }

    div()
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .rounded_full()
        .size(px(size + 2.0 * (ring_px + ring_gap_px)))
        .border(px(ring_px))
        .border_color(accent)
        .p(px(ring_gap_px))
        .child(inner)
}

fn attach_unread_badge(base: Div, size: f32, unread: u32, accent: Hsla, border: Hsla) -> Div {
    if unread == 0 {
        return base;
    }
    let badge = size * 0.5;
    div().relative().flex_shrink_0().child(base).child(
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
    )
}

/// 账号头像（左栏账号分组），复用自定义外观圆盘并叠加右上角未读角标。
pub fn account_circle_with_badge(
    acc: &AccountConfig,
    data_dir: &Path,
    size: f32,
    unread: u32,
    badge_accent: Hsla,
    badge_border: Hsla,
) -> Div {
    let base = account_circle_visual(
        &acc.display_label(),
        &acc.email,
        acc.color.as_deref(),
        acc.emoji.as_deref(),
        acc.avatar.as_deref(),
        data_dir,
        size,
        2.0,
        1.0,
    );
    attach_unread_badge(base, size, unread, badge_accent, badge_border)
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

    #[test]
    fn parse_hex_color_and_accent_hex_handle_custom_and_default() {
        assert!(parse_hex_color("#337fdc").is_some());
        assert!(parse_hex_color("  #E62D42 ").is_some());
        assert!(parse_hex_color("337fdc").is_none());
        assert!(parse_hex_color("#12345").is_none());
        assert!(parse_hex_color("#zzzzzz").is_none());

        assert_eq!(
            account_accent_hex("alice@example.com", Some(" #E62D42 ")),
            "#e62d42"
        );
        let fallback = account_accent_hex("alice@example.com", Some("invalid"));
        assert!(AVATAR_COLORS.contains(&fallback.as_str()));
        assert_eq!(account_accent_hex("alice@example.com", None), fallback);

        let parsed = account_accent_color("alice@example.com", Some("#29ae74"));
        assert!(approx_hsla(parsed, parse_hex_color("#29ae74").unwrap()));
    }
}
