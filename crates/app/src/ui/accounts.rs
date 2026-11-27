//! 偏好设置 -> 邮箱账号页（对齐 Linux 版双层下钻结构与分组卡片设计）：
//! - 第一层（列表页）：单张 `boxed-list` 账号卡片，支持拖拽排序把手、30px 彩色圆盘头像、
//!   品牌图标 / 蓝底 `IMAP` 胶囊、`Hylki` 来源标记、启用/停用 `Switch`、右箭头进入编辑；
//!   右上方提供「添加账号…」入口。
//! - 第二层（编辑 / 添加页）：由 `SettingsWindow` 顶栏接管返回 / 移除 / 保存，页面内展示
//!   56px 顶部品牌徽章、「邮箱账号」分组（服务商下拉 + 动态 IMAP/SMTP 或 Microsoft OAuth2）、
//!   「外观」分组（72px 实时预览 + 10 色板 + 首字母/Emoji/自定义图片）、
//!   「签名」分组与「同步」分组（逐账号推送覆盖与手动同步）。

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::text::TextView;
use gpui_kit::component::{h_flex, v_flex, Disableable, Icon, Selectable, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    div, img, px, rgb, white, AnyElement, AppContext, Context, Entity, FontWeight, Hsla,
    InteractiveElement, IntoElement, MouseButton, ObjectFit, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, StyledImage, Window,
};
use kitemail_mail_core::config::{
    self, AccountConfig, AuthBackend, NamedSignature, SecurityMode, SendAsAlias,
    SpecialFoldersConfig,
};
use kitemail_mail_core::imap_client::sanitize_html_for_display_with_options;
use kitemail_mail_core::{markup, signature, MailRequest};

use crate::app::HylkiApp;
use crate::theme::{self, HylkiPalette};
use crate::ui::avatar::{self, ACCOUNT_ACCENT_COLORS};
use crate::ui::preferences::widgets::{
    combo_button, combo_button_dynamic, setting_group, CARD_MAX_WIDTH,
};
use crate::ui::preferences::SettingsWindow;
use crate::ui::toast::ToastKind;

/// 常用快捷 Emoji 预设列表。
const QUICK_EMOJIS: &[&str] = &["💼", "🏠", "📧", "⭐", "🚀", "🎨", "🌿", "📮"];

/// IMAP / SMTP 加密方式下拉选项。
const SECURITY_OPTIONS: &[(&str, bool)] = &[("SSL/TLS", true), ("STARTTLS", false)];

/// 逐账号实时推送覆盖下拉选项。
const PUSH_OVERRIDE_OPTIONS: &[(&str, Option<bool>)] = &[
    ("跟随全局设置", None),
    ("为此账号开启", Some(true)),
    ("为此账号关闭", Some(false)),
];

/// WP-06：账号级轮询周期覆盖下拉选项。
const ACCOUNT_SYNC_INTERVAL_OPTIONS: &[(&str, Option<u32>)] = &[
    ("跟随全局设置", None),
    ("仅手动同步", Some(0)),
    ("每 1 分钟", Some(1)),
    ("每 5 分钟", Some(5)),
    ("每 15 分钟", Some(15)),
    ("每 30 分钟", Some(30)),
];

/// WP-06：下载历史邮件时间窗口。
const DOWNLOAD_WINDOW_OPTIONS: &[(&str, u32)] = &[
    ("全部历史", 0),
    ("最近 30 天", 30),
    ("最近 90 天", 90),
    ("最近 1 年", 365),
];

/// WP-06：自动清空废纸篓 / 垃圾邮件天数。
const AUTO_EMPTY_DAYS_OPTIONS: &[(&str, u32)] = &[
    ("从不自动清空", 0),
    ("超过 7 天", 7),
    ("超过 14 天", 14),
    ("超过 30 天", 30),
];

/// 服务商接入类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProviderKind {
    Manual,
    Microsoft,
    Preset,
}

/// 服务商预设定义（保留原版 Linux 全部可用服务商，并补充国内常用邮箱与 Gmail 预设）。
#[derive(Clone, Copy, Debug)]
pub(crate) struct AccountProvider {
    pub id: &'static str,
    pub label: &'static str,
    pub kind: ProviderKind,
    pub brand: Option<&'static str>,
    pub featured_on_welcome: bool,
    pub imap_host: &'static str,
    pub imap_port: u16,
    pub imap_tls: bool,
    pub smtp_host: &'static str,
    pub smtp_port: u16,
    pub smtp_tls: bool,
    pub hint: &'static str,
}

/// 服务商下拉菜单全部条目（共 17 项：手动 + Microsoft OAuth2 + 国内常用/主流 IMAP 预设）。
pub(crate) const ACCOUNT_PROVIDERS: &[AccountProvider] = &[
    AccountProvider {
        id: "manual",
        label: "手动设置（IMAP / SMTP）",
        kind: ProviderKind::Manual,
        brand: None,
        featured_on_welcome: false,
        imap_host: "",
        imap_port: 993,
        imap_tls: true,
        smtp_host: "",
        smtp_port: 465,
        smtp_tls: true,
        hint: "手动填写邮件服务商提供的 IMAP 与 SMTP 服务器地址和端口。",
    },
    AccountProvider {
        id: "microsoft",
        label: "Microsoft 365 / Outlook",
        kind: ProviderKind::Microsoft,
        brand: Some("outlook"),
        featured_on_welcome: false,
        imap_host: "",
        imap_port: 993,
        imap_tls: true,
        smtp_host: "",
        smtp_port: 587,
        smtp_tls: false,
        hint: "通过系统浏览器完成 Microsoft OAuth2 登录授权（支持 Outlook.com / Hotmail / Live / Microsoft 365）。",
    },
    AccountProvider {
        id: "qq",
        label: "QQ 邮箱",
        kind: ProviderKind::Preset,
        brand: Some("qq"),
        featured_on_welcome: true,
        imap_host: "imap.qq.com",
        imap_port: 993,
        imap_tls: true,
        smtp_host: "smtp.qq.com",
        smtp_port: 465,
        smtp_tls: true,
        hint: "QQ 邮箱需在网页版「设置 → 账号」开启 IMAP/SMTP 服务，并填写生成的 16 位授权码（非 QQ 登录密码）。",
    },
    AccountProvider {
        id: "netease163",
        label: "网易 163",
        kind: ProviderKind::Preset,
        brand: Some("netease163"),
        featured_on_welcome: true,
        imap_host: "imap.163.com",
        imap_port: 993,
        imap_tls: true,
        smtp_host: "smtp.163.com",
        smtp_port: 465,
        smtp_tls: true,
        hint: "网易 163 需在网页版「设置 → POP3/SMTP/IMAP」开启 IMAP/SMTP 服务，并填写客户端授权密码。",
    },
    AccountProvider {
        id: "netease126",
        label: "网易 126",
        kind: ProviderKind::Preset,
        brand: Some("netease126"),
        featured_on_welcome: true,
        imap_host: "imap.126.com",
        imap_port: 993,
        imap_tls: true,
        smtp_host: "smtp.126.com",
        smtp_port: 465,
        smtp_tls: true,
        hint: "网易 126 需在网页版「设置 → POP3/SMTP/IMAP」开启 IMAP/SMTP 服务，并填写客户端授权密码。",
    },
    AccountProvider {
        id: "exmail",
        label: "腾讯企业邮",
        kind: ProviderKind::Preset,
        brand: Some("exmail"),
        featured_on_welcome: true,
        imap_host: "imap.exmail.qq.com",
        imap_port: 993,
        imap_tls: true,
        smtp_host: "smtp.exmail.qq.com",
        smtp_port: 465,
        smtp_tls: true,
        hint: "若管理员开启了安全登录或微信绑定，请在腾讯企业邮网页端「设置 → 账号安全」生成客户端专用密码。",
    },
    AccountProvider {
        id: "aliyun_qiye",
        label: "阿里企业邮",
        kind: ProviderKind::Preset,
        brand: Some("aliyun_qiye"),
        featured_on_welcome: true,
        imap_host: "imap.qiye.aliyun.com",
        imap_port: 993,
        imap_tls: true,
        smtp_host: "smtp.qiye.aliyun.com",
        smtp_port: 465,
        smtp_tls: true,
        hint: "请使用完整企业邮箱地址作为用户名；如开启三方客户端安全密码，请在阿里企业邮网页端生成专用密码。",
    },
    AccountProvider {
        id: "gmail",
        label: "Gmail",
        kind: ProviderKind::Preset,
        brand: Some("gmail"),
        featured_on_welcome: true,
        imap_host: "imap.gmail.com",
        imap_port: 993,
        imap_tls: true,
        smtp_host: "smtp.gmail.com",
        smtp_port: 465,
        smtp_tls: true,
        hint: "Gmail 需在 Google 账号开启「两步验证」，并在安全性设置中生成 16 位「应用专用密码 (App Password)」。",
    },
    AccountProvider {
        id: "icloud",
        label: "iCloud",
        kind: ProviderKind::Preset,
        brand: Some("icloud"),
        featured_on_welcome: true,
        imap_host: "imap.mail.me.com",
        imap_port: 993,
        imap_tls: true,
        smtp_host: "smtp.mail.me.com",
        smtp_port: 587,
        smtp_tls: false,
        hint: "iCloud 需在 account.apple.com 生成「App 专用密码」后填写，不支持直接使用 Apple ID 登录密码。",
    },
    AccountProvider {
        id: "yahoo",
        label: "Yahoo Mail",
        kind: ProviderKind::Preset,
        brand: Some("yahoo"),
        featured_on_welcome: false,
        imap_host: "imap.mail.yahoo.com",
        imap_port: 993,
        imap_tls: true,
        smtp_host: "smtp.mail.yahoo.com",
        smtp_port: 465,
        smtp_tls: true,
        hint: "Yahoo 需在账号安全设置中开启「生成应用密码」并使用应用专用密码登录。",
    },
    AccountProvider {
        id: "proton",
        label: "Proton Mail (Bridge)",
        kind: ProviderKind::Preset,
        brand: Some("proton"),
        featured_on_welcome: false,
        imap_host: "127.0.0.1",
        imap_port: 1143,
        imap_tls: false,
        smtp_host: "127.0.0.1",
        smtp_port: 1025,
        smtp_tls: false,
        hint: "需先在本机运行 Proton Mail Bridge，并填写 Bridge 面板显示的本地 IMAP/SMTP 密码与端口。",
    },
    AccountProvider {
        id: "fastmail",
        label: "Fastmail",
        kind: ProviderKind::Preset,
        brand: Some("fastmail"),
        featured_on_welcome: true,
        imap_host: "imap.fastmail.com",
        imap_port: 993,
        imap_tls: true,
        smtp_host: "smtp.fastmail.com",
        smtp_port: 465,
        smtp_tls: true,
        hint: "Fastmail 需在「设置 → 隐私与安全 → 第三方应用密码」生成专用密码。",
    },
    AccountProvider {
        id: "aol",
        label: "AOL Mail",
        kind: ProviderKind::Preset,
        brand: Some("aol"),
        featured_on_welcome: false,
        imap_host: "imap.aol.com",
        imap_port: 993,
        imap_tls: true,
        smtp_host: "smtp.aol.com",
        smtp_port: 465,
        smtp_tls: true,
        hint: "AOL 需在账号安全设置中生成「应用密码」后用于 IMAP/SMTP 登录。",
    },
    AccountProvider {
        id: "zoho",
        label: "Zoho Mail",
        kind: ProviderKind::Preset,
        brand: Some("zoho"),
        featured_on_welcome: false,
        imap_host: "imap.zoho.com",
        imap_port: 993,
        imap_tls: true,
        smtp_host: "smtp.zoho.com",
        smtp_port: 465,
        smtp_tls: true,
        hint: "Zoho 建议启用双因素认证并使用应用专用密码；部分地区域名对应 imap.zoho.com.cn / imap.zoho.eu。",
    },
    AccountProvider {
        id: "gmx",
        label: "GMX",
        kind: ProviderKind::Preset,
        brand: Some("gmx"),
        featured_on_welcome: false,
        imap_host: "imap.gmx.com",
        imap_port: 993,
        imap_tls: true,
        smtp_host: "mail.gmx.com",
        smtp_port: 587,
        smtp_tls: false,
        hint: "请先在 GMX 网页版设置中勾选允许通过 POP3/IMAP 收发邮件。",
    },
    AccountProvider {
        id: "yandex",
        label: "Yandex Mail",
        kind: ProviderKind::Preset,
        brand: Some("yandex"),
        featured_on_welcome: false,
        imap_host: "imap.yandex.com",
        imap_port: 993,
        imap_tls: true,
        smtp_host: "smtp.yandex.com",
        smtp_port: 465,
        smtp_tls: true,
        hint: "Yandex 需在安全设置中开启「邮件客户端应用密码」后使用专用密码登录。",
    },
    AccountProvider {
        id: "mailcom",
        label: "Mail.com",
        kind: ProviderKind::Preset,
        brand: Some("mailcom"),
        featured_on_welcome: false,
        imap_host: "imap.mail.com",
        imap_port: 993,
        imap_tls: true,
        smtp_host: "smtp.mail.com",
        smtp_port: 587,
        smtp_tls: false,
        hint: "请先在 Mail.com 网页端邮箱设置中开启 POP3/IMAP 客户端访问权限。",
    },
];

/// 按账号配置与邮箱域名推断品牌图标 ID（对应 `brands/<id>.png`）。
pub(crate) fn brand_for_account(acc: &AccountConfig) -> Option<&'static str> {
    brand_for_account_fields(acc.auth_backend, &acc.imap_host, acc.imap_port, &acc.email)
}

/// 按认证方式、IMAP 主机/端口与邮箱域名推断品牌图标 ID（对应 `brands/<id>.png`）。
pub(crate) fn brand_for_account_fields(
    auth_backend: AuthBackend,
    imap_host: &str,
    imap_port: u16,
    email: &str,
) -> Option<&'static str> {
    if auth_backend == AuthBackend::MicrosoftGraphOAuth2 {
        return Some("outlook");
    }
    let host = imap_host.trim().to_ascii_lowercase();
    let domain = email
        .trim()
        .rsplit_once('@')
        .map(|(_, d)| d.to_ascii_lowercase())
        .unwrap_or_default();

    if host.contains("exmail.qq.com") {
        Some("exmail")
    } else if host.contains("imap.qq.com")
        || matches!(domain.as_str(), "qq.com" | "foxmail.com" | "vip.qq.com")
    {
        Some("qq")
    } else if host.contains("163.com") || matches!(domain.as_str(), "163.com" | "vip.163.com") {
        Some("netease163")
    } else if host.contains("126.com")
        || matches!(domain.as_str(), "126.com" | "yeah.net" | "vip.126.com")
    {
        Some("netease126")
    } else if host.contains("qiye.aliyun.com") || host.contains("mxhichina.com") {
        Some("aliyun_qiye")
    } else if host.contains("outlook.")
        || host.contains("office365.")
        || host.contains("hotmail.")
        || matches!(
            domain.as_str(),
            "outlook.com" | "hotmail.com" | "live.com" | "msn.com"
        )
        || domain.starts_with("outlook.")
        || domain.starts_with("hotmail.")
        || domain.starts_with("live.")
    {
        Some("outlook")
    } else if host.contains("gmail.com")
        || host.contains("googlemail.com")
        || matches!(domain.as_str(), "gmail.com" | "googlemail.com")
    {
        Some("gmail")
    } else if host.contains("mail.me.com")
        || matches!(domain.as_str(), "icloud.com" | "me.com" | "mac.com")
    {
        Some("icloud")
    } else if host.contains("yahoo.") || domain.starts_with("yahoo.") {
        Some("yahoo")
    } else if (host == "127.0.0.1" && imap_port == 1143)
        || host.contains("proton")
        || domain.starts_with("proton.")
        || domain == "protonmail.com"
        || domain == "pm.me"
    {
        Some("proton")
    } else if host.contains("fastmail.") || domain.starts_with("fastmail.") {
        Some("fastmail")
    } else if host.contains("aol.") || domain == "aol.com" {
        Some("aol")
    } else if host.contains("zoho.") || domain.starts_with("zoho.") {
        Some("zoho")
    } else if host.contains("gmx.") || domain.starts_with("gmx.") {
        Some("gmx")
    } else if host.contains("yandex.") || domain.starts_with("yandex.") || domain == "ya.ru" {
        Some("yandex")
    } else if host.contains("imap.mail.com") || domain == "mail.com" {
        Some("mailcom")
    } else if host.contains("stalwart") || domain.contains("stalwart") {
        Some("stalwart")
    } else {
        None
    }
}

/// 根据已有账号配置匹配 `ACCOUNT_PROVIDERS` 索引，未命中回退 `0`（手动设置）。
pub(crate) fn provider_index_for_account(acc: &AccountConfig) -> usize {
    if acc.auth_backend == AuthBackend::MicrosoftGraphOAuth2 {
        return ACCOUNT_PROVIDERS
            .iter()
            .position(|p| p.kind == ProviderKind::Microsoft)
            .unwrap_or(1);
    }
    let host = acc.imap_host.trim().to_ascii_lowercase();
    if host == "127.0.0.1" && acc.imap_port == 1143 {
        if let Some(idx) = ACCOUNT_PROVIDERS.iter().position(|p| p.id == "proton") {
            return idx;
        }
    }
    ACCOUNT_PROVIDERS
        .iter()
        .position(|p| p.kind == ProviderKind::Preset && host.eq_ignore_ascii_case(p.imap_host))
        .unwrap_or(0)
}

/// 渲染品牌 PNG 或通用蓝底白字 `IMAP` 协议徽章（列表 24px / 编辑页顶部 56px）。
fn render_brand_mark(brand: Option<&'static str>, size_px: f32) -> AnyElement {
    if let Some(b) = brand {
        return img(SharedString::from(format!("brands/{b}.png")))
            .size(px(size_px))
            .object_fit(ObjectFit::Contain)
            .into_any_element();
    }
    let pill_blue = Hsla::from(rgb(0x005bff));
    if size_px >= 40.0 {
        div()
            .size(px(size_px))
            .rounded(px(14.0))
            .bg(pill_blue)
            .flex()
            .items_center()
            .justify_center()
            .text_color(white())
            .text_size(px(size_px * 0.25))
            .font_weight(FontWeight::BOLD)
            .child("IMAP")
            .into_any_element()
    } else {
        div()
            .w(px(38.0))
            .h(px(22.0))
            .rounded(px(6.0))
            .bg(pill_blue)
            .flex()
            .items_center()
            .justify_center()
            .text_color(white())
            .text_size(px(10.0))
            .font_weight(FontWeight::BOLD)
            .child("IMAP")
            .into_any_element()
    }
}

/// 列表行拖拽排序载荷。
#[derive(Clone)]
struct AccountRowDrag {
    index: usize,
    label: SharedString,
}

impl Render for AccountRowDrag {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = theme::current(cx);
        div()
            .px_3()
            .py_1p5()
            .rounded(px(8.0))
            .bg(p.surface_raised)
            .border_1()
            .border_color(p.accent)
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .text_color(p.text)
            .child(self.label.clone())
    }
}

/// 账号头像显示模式。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AvatarDisplayMode {
    /// 默认：优先使用品牌图标，未匹配品牌时回退首字母。
    BrandDefault,
    /// 原有方式：自定义 Emoji（留空则显示首字母，不使用品牌图标）。
    InitialsOrEmoji,
    /// 自定义上传图片。
    Picture,
}

/// 账号表单输入与编辑器状态（在设置窗口自己的 Window 上创建，避免跨窗口持有 InputState）。
pub(crate) struct AccountFormState {
    pub(crate) editing_account_id: Option<u32>,
    pub(crate) session_generation: u64,
    pub(crate) provider_idx: usize,
    pub(crate) original_enabled: bool,
    pub(crate) original_auth_backend: AuthBackend,
    pub(crate) label: Entity<InputState>,
    pub(crate) label_synced: bool,
    pub(crate) display_name: Entity<InputState>,
    pub(crate) email: Entity<InputState>,
    pub(crate) imap_host: Entity<InputState>,
    pub(crate) imap_port: Entity<InputState>,
    pub(crate) imap_tls: bool,
    pub(crate) imap_user: Entity<InputState>,
    pub(crate) imap_pass: Entity<InputState>,
    pub(crate) smtp_host: Entity<InputState>,
    pub(crate) smtp_port: Entity<InputState>,
    pub(crate) smtp_tls: bool,
    pub(crate) smtp_separate: bool,
    pub(crate) smtp_user: Entity<InputState>,
    pub(crate) smtp_pass: Entity<InputState>,
    pub(crate) oauth_client_id: Entity<InputState>,
    pub(crate) ms_advanced_expanded: bool,
    pub(crate) in_unified: bool,
    pub(crate) push_override: Option<bool>,
    pub(crate) color: Option<String>,
    pub(crate) avatar_mode: AvatarDisplayMode,
    pub(crate) emoji: Option<String>,
    pub(crate) emoji_input: Entity<InputState>,
    pub(crate) avatar: Option<String>,
    pub(crate) signature: Entity<TextareaState>,
    /// 签名编辑器是否显示渲染预览（仅 HTML 模式有效）。
    pub(crate) sig_preview: bool,
    // WP-06：高阶账号配置字段
    pub(crate) quota_used_bytes: Option<u64>,
    pub(crate) quota_total_bytes: Option<u64>,
    pub(crate) aliases: Vec<SendAsAlias>,
    pub(crate) alias_name_input: Entity<InputState>,
    pub(crate) alias_email_input: Entity<InputState>,
    pub(crate) alias_reply_to_input: Entity<InputState>,
    pub(crate) signatures: Vec<NamedSignature>,
    pub(crate) active_sig_idx: usize,
    pub(crate) sig_name_input: Entity<InputState>,
    pub(crate) default_new_signature: Option<usize>,
    pub(crate) default_reply_signature: Option<usize>,
    pub(crate) special_folders: SpecialFoldersConfig,
    pub(crate) sync_interval_minutes: Option<u32>,
    pub(crate) download_window_days: u32,
    pub(crate) auto_empty_trash_days: u32,
    pub(crate) auto_empty_junk_days: u32,
    pub(crate) hidden_folders: Vec<String>,
    pub(crate) validation_error: Option<SharedString>,
    initial_fingerprint: String,
}

fn normalize_initial_label(label: Option<&str>, email: &str) -> (String, bool) {
    let trimmed_email = email.trim();
    let trimmed_label = label.map(str::trim).unwrap_or("");
    let synced = trimmed_label.is_empty() || trimmed_label == trimmed_email;
    let initial = if trimmed_label.is_empty() {
        trimmed_email.to_string()
    } else {
        trimmed_label.to_string()
    };
    (initial, synced)
}

fn normalize_emoji_opt(emoji: Option<&str>) -> Option<String> {
    emoji
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(4).collect())
}

/// 签名源码渲染成预览 HTML：先按签名白名单清洗（去脚本、事件属性、危险协议），
/// 再按阅读器的规则整形，`TextView::html` 才排得开表格。
fn signature_preview_html(body: &str, is_html: bool) -> String {
    if !is_html || body.trim().is_empty() {
        return String::new();
    }
    let html = if markup::looks_like_html(body) {
        body.to_string()
    } else {
        signature::signature_to_html(body, false)
    };
    let html = signature::sanitize_signature_html(&html);
    sanitize_html_for_display_with_options(&html, false, false).0
}

/// 删除第 `rm_idx` 个签名后同步更新默认签名索引：命中被删项时退回「不使用签名」(`None`)，
/// 其余排在其后的索引前移一位。
fn adjust_default_sig_after_remove(current: &mut Option<usize>, rm_idx: usize) {
    match *current {
        Some(i) if i == rm_idx => *current = None,
        Some(i) if i > rm_idx => *current = Some(i - 1),
        _ => {}
    }
}

/// 签名落盘前规整（对齐原版把源码粘进签名后立即清洗的做法）：
/// 看起来就是 HTML 源码的按 HTML 处理，并过一遍白名单清洗；
/// HTML 模式下的纯文本转义成等价 HTML，避免标签被当成文字发出去。
fn normalize_signatures(signatures: &mut [NamedSignature]) {
    for sig in signatures.iter_mut() {
        let body = sig.body.trim();
        if body.is_empty() {
            continue;
        }
        let html_source =
            markup::looks_like_html(body) || (body.contains('<') && body.contains('>'));
        if sig.is_html || markup::looks_like_html(body) {
            sig.is_html = true;
            sig.body = if html_source {
                signature::sanitize_signature_html(body)
            } else {
                signature::signature_to_html(body, false)
            };
        }
    }
}

impl SettingsWindow {
    /// 「导入 HTML…」：选一个 HTML / 文本文件，清洗后放进当前签名。
    /// 本地图片按文件所在目录内联（对齐原版 Import File…）。
    pub(crate) fn import_signature_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.file_dialog_open {
            return;
        }
        self.file_dialog_open = true;
        let generation = self.account_form.session_generation;
        cx.spawn_in(window, async move |this, cx| {
            let path = cx
                .background_executor()
                .spawn(async move {
                    rfd::FileDialog::new()
                        .set_title("选择签名文件")
                        .add_filter("HTML 与文本文件", &["html", "htm", "txt"])
                        .pick_file()
                })
                .await;
            let Some(path) = path else {
                let _ = this.update(cx, |this, _| this.file_dialog_open = false);
                return;
            };
            let read = std::fs::read_to_string(&path);
            let _ = cx.update(|window, cx| {
                this.update(cx, |this, cx| {
                    this.file_dialog_open = false;
                    if !this.account_editor_open
                        || this.account_form.session_generation != generation
                    {
                        return;
                    }
                    match read {
                        Ok(text) => {
                            let body = signature::signature_from_source(&text, path.parent());
                            let is_html = markup::looks_like_html(&text);
                            this.apply_imported_signature(body, is_html, window, cx);
                        }
                        Err(err) => {
                            let message = format!("读取签名文件失败：{err}");
                            this.app.update(cx, |app, cx| {
                                app.push_toast(
                                    "sig-import-error",
                                    ToastKind::Error,
                                    message,
                                    None,
                                    cx,
                                );
                            });
                        }
                    }
                })
            });
        })
        .detach();
    }

    /// 把导入进来的签名正文放进当前编辑的签名。
    fn apply_imported_signature(
        &mut self,
        body: String,
        is_html: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let idx = self.account_form.active_sig_idx;
        if let Some(sig) = self.account_form.signatures.get_mut(idx) {
            sig.is_html = is_html;
            sig.body = body.clone();
        }
        self.account_form.sig_preview &= is_html;
        self.account_form
            .signature
            .update(cx, |state, cx| state.set_value(body, window, cx));
        cx.notify();
    }
}

impl AccountFormState {
    pub(crate) fn new(
        initial: Option<&AccountConfig>,
        window: &mut Window,
        cx: &mut Context<SettingsWindow>,
    ) -> Self {
        let cfg = initial.cloned().unwrap_or_default();
        let editing_account_id = initial.map(|c| c.id);
        let provider_idx = initial.map(provider_index_for_account).unwrap_or(0);
        let (initial_label, label_synced) =
            normalize_initial_label(cfg.label.as_deref(), &cfg.email);
        let normalized_emoji = normalize_emoji_opt(cfg.emoji.as_deref());

        let label = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("侧栏显示名称（默认同邮箱地址）")
                .default_value(initial_label)
        });
        let display_name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("寄件人署名（如：张三）")
                .default_value(cfg.display_name.clone())
        });
        let email = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("邮箱地址（如：user@example.com）")
                .default_value(cfg.email.clone())
        });
        let imap_host = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("IMAP 服务器（如：imap.example.com）")
                .default_value(cfg.imap_host.clone())
        });
        let imap_port = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("993")
                .default_value(cfg.imap_port.to_string())
        });
        let imap_user = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("留空默认使用邮箱地址")
                .default_value(cfg.imap_username.clone())
        });
        let imap_pass = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("密码或应用专用密码（存入 Windows 凭据管理器）")
        });
        let smtp_host = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("SMTP 服务器（如：smtp.example.com）")
                .default_value(cfg.smtp_host.clone())
        });
        let smtp_port = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("465")
                .default_value(cfg.smtp_port.to_string())
        });
        let smtp_user = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("SMTP 独立用户名（可选）")
                .default_value(cfg.smtp_username.clone())
        });
        let smtp_pass = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("SMTP 独立授权码（启用独立凭据时填写）")
        });
        let oauth_client_id = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("可选：自定义 Microsoft Entra Client ID（留空使用内置）")
                .default_value(cfg.oauth_client_id.clone())
        });
        let emoji_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("输入或选择单个 Emoji，如 💼")
                .default_value(normalized_emoji.clone().unwrap_or_default())
        });
        let initial_signatures = if !cfg.signatures.is_empty() {
            cfg.signatures.clone()
        } else {
            vec![NamedSignature {
                name: "默认签名".to_string(),
                body: cfg.signature.clone(),
                is_html: false,
            }]
        };
        let initial_sig_body = initial_signatures
            .first()
            .map(|s| s.body.clone())
            .unwrap_or_default();
        let initial_sig_name = initial_signatures
            .first()
            .map(|s| s.name.clone())
            .unwrap_or_else(|| "默认签名".to_string());
        let signature = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(3, 6)
                .placeholder("可选：邮件签名（可在下方指定新邮件与回复/转发默认签名）")
                .default_value(initial_sig_body)
        });
        let sig_name_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("签名名称（如：商务签名 / 简短回复）")
                .default_value(initial_sig_name)
        });
        let alias_name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("发件人显示名称（可选）"));
        let alias_email_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("别名邮箱地址（如：sales@example.com）")
        });
        let alias_reply_to_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Reply-To 回复地址（可选）"));

        // 实时同步邮箱 -> 默认昵称，并随用户输入刷新 72px 外观预览与清除过期错误。
        cx.subscribe_in(
            &email,
            window,
            |this, input, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change) {
                    let email_val = input.read(cx).value().to_string();
                    this.account_form.validation_error = None;
                    if this.account_form.label_synced {
                        this.account_form
                            .label
                            .update(cx, |s, cx| s.set_value(email_val, window, cx));
                    }
                    cx.notify();
                }
            },
        )
        .detach();

        cx.subscribe_in(&label, window, |this, input, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                let label_val = input.read(cx).value().trim().to_string();
                let email_val = this.account_form.email.read(cx).value().trim().to_string();
                this.account_form.label_synced = label_val.is_empty() || label_val == email_val;
                this.account_form.validation_error = None;
                cx.notify();
            }
        })
        .detach();

        cx.subscribe_in(
            &emoji_input,
            window,
            |this, input, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    let raw = input.read(cx).value().trim().to_string();
                    this.account_form.emoji = if raw.is_empty() {
                        None
                    } else {
                        Some(raw.chars().take(4).collect())
                    };
                    cx.notify();
                }
            },
        )
        .detach();

        for watched in [
            &display_name,
            &imap_host,
            &imap_port,
            &imap_pass,
            &smtp_host,
            &smtp_port,
        ] {
            cx.subscribe(watched, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.account_form.validation_error = None;
                    cx.notify();
                }
            })
            .detach();
        }

        let special_folders = if editing_account_id.is_none() && !cfg.email.is_empty() {
            SpecialFoldersConfig {
                server_saves_sent: config::default_server_saves_sent_for_email(&cfg.email),
                ..cfg.special_folders.clone()
            }
        } else {
            cfg.special_folders.clone()
        };
        let avatar_mode = if cfg.avatar.is_some() {
            AvatarDisplayMode::Picture
        } else if normalized_emoji.is_some() || !cfg.use_brand_avatar {
            AvatarDisplayMode::InitialsOrEmoji
        } else {
            AvatarDisplayMode::BrandDefault
        };
        let mut form = Self {
            editing_account_id,
            session_generation: 0,
            provider_idx,
            original_enabled: cfg.enabled,
            original_auth_backend: cfg.auth_backend,
            label,
            label_synced,
            display_name,
            email,
            imap_host,
            imap_port,
            imap_tls: cfg.imap_security == SecurityMode::Tls,
            imap_user,
            imap_pass,
            smtp_host,
            smtp_port,
            smtp_tls: cfg.smtp_security == SecurityMode::Tls,
            smtp_separate: cfg.smtp_separate_credentials,
            smtp_user,
            smtp_pass,
            oauth_client_id,
            ms_advanced_expanded: !cfg.oauth_client_id.trim().is_empty(),
            in_unified: cfg.in_unified,
            push_override: cfg.push_override,
            color: cfg.color,
            avatar_mode,
            emoji: normalized_emoji,
            emoji_input,
            avatar: cfg.avatar,
            signature,
            sig_preview: false,
            quota_used_bytes: cfg.quota_used_bytes,
            quota_total_bytes: cfg.quota_total_bytes,
            aliases: cfg.aliases,
            alias_name_input,
            alias_email_input,
            alias_reply_to_input,
            signatures: initial_signatures,
            active_sig_idx: 0,
            sig_name_input,
            default_new_signature: cfg.default_new_signature,
            default_reply_signature: cfg.default_reply_signature,
            special_folders,
            sync_interval_minutes: cfg.sync_interval_minutes,
            download_window_days: cfg.download_window_days,
            auto_empty_trash_days: cfg.auto_empty_trash_days,
            auto_empty_junk_days: cfg.auto_empty_junk_days,
            hidden_folders: cfg.hidden_folders,
            validation_error: None,
            initial_fingerprint: String::new(),
        };
        form.snapshot_clean(cx);
        form
    }

    pub(crate) fn load_account(
        &mut self,
        cfg: Option<&AccountConfig>,
        window: &mut Window,
        cx: &mut Context<SettingsWindow>,
    ) {
        self.session_generation = self.session_generation.wrapping_add(1);
        let c = cfg.cloned().unwrap_or_default();
        self.editing_account_id = cfg.map(|a| a.id);
        self.provider_idx = cfg.map(provider_index_for_account).unwrap_or(0);
        self.original_enabled = c.enabled;
        self.original_auth_backend = c.auth_backend;
        let (initial_label, label_synced) = normalize_initial_label(c.label.as_deref(), &c.email);
        let normalized_emoji = normalize_emoji_opt(c.emoji.as_deref());
        self.label_synced = label_synced;

        self.label
            .update(cx, |s, cx| s.set_value(initial_label, window, cx));
        self.display_name
            .update(cx, |s, cx| s.set_value(c.display_name, window, cx));
        self.email
            .update(cx, |s, cx| s.set_value(c.email, window, cx));
        self.imap_host
            .update(cx, |s, cx| s.set_value(c.imap_host, window, cx));
        self.imap_port
            .update(cx, |s, cx| s.set_value(c.imap_port.to_string(), window, cx));
        self.imap_tls = c.imap_security == SecurityMode::Tls;
        self.imap_user
            .update(cx, |s, cx| s.set_value(c.imap_username, window, cx));
        self.imap_pass
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.smtp_host
            .update(cx, |s, cx| s.set_value(c.smtp_host, window, cx));
        self.smtp_port
            .update(cx, |s, cx| s.set_value(c.smtp_port.to_string(), window, cx));
        self.smtp_tls = c.smtp_security == SecurityMode::Tls;
        self.smtp_separate = c.smtp_separate_credentials;
        self.smtp_user
            .update(cx, |s, cx| s.set_value(c.smtp_username, window, cx));
        self.smtp_pass
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.ms_advanced_expanded = !c.oauth_client_id.trim().is_empty();
        self.oauth_client_id
            .update(cx, |s, cx| s.set_value(c.oauth_client_id, window, cx));
        self.in_unified = c.in_unified;
        self.push_override = c.push_override;
        self.color = c.color;
        self.avatar_mode = if c.avatar.is_some() {
            AvatarDisplayMode::Picture
        } else if normalized_emoji.is_some() || !c.use_brand_avatar {
            AvatarDisplayMode::InitialsOrEmoji
        } else {
            AvatarDisplayMode::BrandDefault
        };
        self.emoji = normalized_emoji.clone();
        self.emoji_input.update(cx, |s, cx| {
            s.set_value(normalized_emoji.unwrap_or_default(), window, cx)
        });
        self.avatar = c.avatar;
        let loaded_sigs = if !c.signatures.is_empty() {
            c.signatures
        } else {
            vec![NamedSignature {
                name: "默认签名".to_string(),
                body: c.signature.clone(),
                is_html: false,
            }]
        };
        let first_body = loaded_sigs
            .first()
            .map(|s| s.body.clone())
            .unwrap_or_default();
        let first_name = loaded_sigs
            .first()
            .map(|s| s.name.clone())
            .unwrap_or_else(|| "默认签名".to_string());
        self.signatures = loaded_sigs;
        self.active_sig_idx = 0;
        self.sig_preview = false;
        self.signature
            .update(cx, |s, cx| s.set_value(first_body, window, cx));
        self.sig_name_input
            .update(cx, |s, cx| s.set_value(first_name, window, cx));
        self.quota_used_bytes = c.quota_used_bytes;
        self.quota_total_bytes = c.quota_total_bytes;
        self.aliases = c.aliases;
        self.alias_name_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.alias_email_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.alias_reply_to_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.default_new_signature = c.default_new_signature;
        self.default_reply_signature = c.default_reply_signature;
        self.special_folders = c.special_folders;
        self.sync_interval_minutes = c.sync_interval_minutes;
        self.download_window_days = c.download_window_days;
        self.auto_empty_trash_days = c.auto_empty_trash_days;
        self.auto_empty_junk_days = c.auto_empty_junk_days;
        self.hidden_folders = c.hidden_folders;
        self.validation_error = None;
        self.snapshot_clean(cx);
    }

    /// 载入空白账号并预选指定服务商预设（例如从欢迎页卡片跳入），视为干净初始状态。
    pub(crate) fn apply_provider_for_new(
        &mut self,
        provider_idx: usize,
        window: &mut Window,
        cx: &mut Context<SettingsWindow>,
    ) {
        self.load_account(None, window, cx);
        self.select_provider(provider_idx, window, cx);
        self.snapshot_clean(cx);
    }

    /// 在编辑器中切换服务商预设（不重置脏基线，以便触发未保存离开保护）。
    pub(crate) fn select_provider(
        &mut self,
        provider_idx: usize,
        window: &mut Window,
        cx: &mut Context<SettingsWindow>,
    ) {
        let Some(provider) = ACCOUNT_PROVIDERS.get(provider_idx) else {
            return;
        };
        self.provider_idx = provider_idx;
        self.validation_error = None;
        if provider.kind == ProviderKind::Preset {
            self.imap_host
                .update(cx, |s, cx| s.set_value(provider.imap_host, window, cx));
            self.imap_port.update(cx, |s, cx| {
                s.set_value(provider.imap_port.to_string(), window, cx)
            });
            self.imap_tls = provider.imap_tls;
            self.smtp_host
                .update(cx, |s, cx| s.set_value(provider.smtp_host, window, cx));
            self.smtp_port.update(cx, |s, cx| {
                s.set_value(provider.smtp_port.to_string(), window, cx)
            });
            self.smtp_tls = provider.smtp_tls;
        }
    }

    /// 将当前激活签名的名称与正文从输入框写回 `self.signatures` 列表。
    pub(crate) fn sync_active_signature_from_inputs(&mut self, cx: &Context<SettingsWindow>) {
        if let Some(sig) = self.signatures.get_mut(self.active_sig_idx) {
            let name = self.sig_name_input.read(cx).value().trim().to_string();
            sig.name = if name.is_empty() {
                format!("签名 {}", self.active_sig_idx + 1)
            } else {
                name
            };
            sig.body = self.signature.read(cx).value().to_string();
        }
    }

    /// 切换到指定索引的命名签名。
    pub(crate) fn select_signature(
        &mut self,
        idx: usize,
        window: &mut Window,
        cx: &mut Context<SettingsWindow>,
    ) {
        if idx >= self.signatures.len() {
            return;
        }
        self.sync_active_signature_from_inputs(cx);
        self.active_sig_idx = idx;
        self.sig_preview = false;
        let sig = self.signatures[idx].clone();
        self.sig_name_input
            .update(cx, |s, cx| s.set_value(sig.name, window, cx));
        self.signature
            .update(cx, |s, cx| s.set_value(sig.body, window, cx));
    }

    fn fingerprint(&self, cx: &Context<SettingsWindow>) -> String {
        format!(
            "{:?}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{}|{:?}|{:?}|{}|{:?}|{:?}|{}|{:?}|{:?}|{}|{:?}|{:?}|{:?}|{:?}|{}|{}|{}|{:?}",
            self.editing_account_id,
            self.provider_idx,
            self.original_enabled,
            self.label.read(cx).value().trim(),
            self.display_name.read(cx).value().trim(),
            self.email.read(cx).value().trim(),
            self.imap_host.read(cx).value().trim(),
            self.imap_port.read(cx).value().trim(),
            self.imap_tls,
            self.imap_user.read(cx).value().trim(),
            self.imap_pass.read(cx).value(),
            self.smtp_host.read(cx).value().trim(),
            self.smtp_port.read(cx).value().trim(),
            self.smtp_tls,
            self.smtp_separate,
            self.smtp_user.read(cx).value().trim(),
            self.smtp_pass.read(cx).value(),
            self.oauth_client_id.read(cx).value().trim(),
            self.in_unified,
            self.push_override.map(|v| if v { "1" } else { "0" }).unwrap_or("n"),
            self.color,
            match self.avatar_mode {
                AvatarDisplayMode::BrandDefault => 0u8,
                AvatarDisplayMode::InitialsOrEmoji => 1,
                AvatarDisplayMode::Picture => 2,
            },
            self.emoji.as_deref().unwrap_or(""),
            self.avatar,
            self.signature.read(cx).value(),
            self.emoji_input.read(cx).value().trim(),
            self.aliases,
            self.signatures,
            self.sig_name_input.read(cx).value().trim(),
            self.default_new_signature,
            self.default_reply_signature,
            self.special_folders,
            self.sync_interval_minutes,
            self.download_window_days,
            self.auto_empty_trash_days,
            self.auto_empty_junk_days,
            self.hidden_folders,
        )
    }

    pub(crate) fn snapshot_clean(&mut self, cx: &Context<SettingsWindow>) {
        self.initial_fingerprint = self.fingerprint(cx);
    }

    pub(crate) fn is_dirty(&self, cx: &Context<SettingsWindow>) -> bool {
        self.fingerprint(cx) != self.initial_fingerprint
    }

    fn current_provider(&self) -> &'static AccountProvider {
        ACCOUNT_PROVIDERS
            .get(self.provider_idx)
            .unwrap_or(&ACCOUNT_PROVIDERS[0])
    }

    /// 保存前字段校验；错误文案与 Linux 表单提示保持一致。
    pub(crate) fn validate_before_save(&self, cx: &Context<SettingsWindow>) -> Result<(), String> {
        let provider = self.current_provider();
        if provider.kind == ProviderKind::Microsoft && self.editing_account_id.is_none() {
            return Err("请先点击「使用 Microsoft 账号登录」在浏览器中完成授权".to_string());
        }
        if provider.kind == ProviderKind::Microsoft {
            return Ok(());
        }

        let email = self.email.read(cx).value().trim().to_string();
        if email.is_empty() || !email.contains('@') {
            return Err("请输入有效的邮箱地址".to_string());
        }
        if self.imap_host.read(cx).value().trim().is_empty() {
            return Err("请填写 IMAP 服务器地址".to_string());
        }
        if self.smtp_host.read(cx).value().trim().is_empty() {
            return Err("请填写 SMTP 服务器地址".to_string());
        }
        let imap_port = self.imap_port.read(cx).value().trim().parse::<u16>();
        if imap_port.map(|p| p == 0).unwrap_or(true) {
            return Err("IMAP 端口需为 1–65535 的有效整数".to_string());
        }
        let smtp_port = self.smtp_port.read(cx).value().trim().parse::<u16>();
        if smtp_port.map(|p| p == 0).unwrap_or(true) {
            return Err("SMTP 端口需为 1–65535 的有效整数".to_string());
        }
        if self.editing_account_id.is_none() && self.imap_pass.read(cx).value().is_empty() {
            return Err("新增 IMAP 账号时请填写密码或应用专用密码".to_string());
        }
        Ok(())
    }

    /// 采集表单生成 [`AccountConfig`] 及凭据；保留 OAuth2 账号协议而不误写为密码模式。
    pub(crate) fn collect(
        &self,
        existing_accounts: &[AccountConfig],
        cx: &Context<SettingsWindow>,
    ) -> (AccountConfig, String, Option<String>) {
        let email = self.email.read(cx).value().trim().to_string();
        let raw_label = self.label.read(cx).value().trim().to_string();
        let label = if raw_label.is_empty() || raw_label == email {
            None
        } else {
            Some(raw_label)
        };
        let id = self
            .editing_account_id
            .unwrap_or_else(|| config::next_account_id(existing_accounts));

        let color = self
            .color
            .as_deref()
            .filter(|c| avatar::parse_hex_color(c).is_some())
            .map(|c| c.trim().to_ascii_lowercase());
        let emoji = if self.avatar_mode == AvatarDisplayMode::InitialsOrEmoji {
            self.emoji
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.chars().take(4).collect())
        } else {
            None
        };
        let avatar_file = if self.avatar_mode == AvatarDisplayMode::Picture {
            self.avatar
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned)
        } else {
            None
        };
        let use_brand_avatar = self.avatar_mode == AvatarDisplayMode::BrandDefault;

        let existing_acc = self
            .editing_account_id
            .and_then(|aid| existing_accounts.iter().find(|a| a.id == aid));
        let is_oauth = self.editing_account_id.is_some()
            && self.original_auth_backend == AuthBackend::MicrosoftGraphOAuth2
            && self.current_provider().kind == ProviderKind::Microsoft;

        let mut collected_signatures = self.signatures.clone();
        if let Some(active) = collected_signatures.get_mut(self.active_sig_idx) {
            let name = self.sig_name_input.read(cx).value().trim().to_string();
            active.name = if name.is_empty() {
                format!("签名 {}", self.active_sig_idx + 1)
            } else {
                name
            };
            active.body = self.signature.read(cx).value().to_string();
        }
        normalize_signatures(&mut collected_signatures);
        let primary_sig = self
            .default_new_signature
            .and_then(|idx| collected_signatures.get(idx))
            .map(|s| s.body.clone())
            .unwrap_or_default();

        if is_oauth {
            let base = existing_acc.cloned().unwrap_or_default();
            let cfg = AccountConfig {
                id,
                auth_backend: AuthBackend::MicrosoftGraphOAuth2,
                oauth_client_id: self.oauth_client_id.read(cx).value().trim().to_string(),
                label,
                enabled: self.original_enabled,
                display_name: self.display_name.read(cx).value().trim().to_string(),
                email: if email.is_empty() { base.email } else { email },
                imap_host: base.imap_host,
                imap_port: base.imap_port,
                imap_security: base.imap_security,
                imap_username: base.imap_username,
                smtp_host: base.smtp_host,
                smtp_port: base.smtp_port,
                smtp_security: base.smtp_security,
                smtp_username: base.smtp_username,
                smtp_separate_credentials: base.smtp_separate_credentials,
                signature: primary_sig,
                color,
                emoji,
                avatar: avatar_file,
                use_brand_avatar,
                in_unified: self.in_unified,
                push_override: self.push_override,
                quota_used_bytes: self.quota_used_bytes,
                quota_total_bytes: self.quota_total_bytes,
                aliases: self.aliases.clone(),
                signatures: collected_signatures,
                default_new_signature: self.default_new_signature,
                default_reply_signature: self.default_reply_signature,
                special_folders: self.special_folders.clone(),
                sync_interval_minutes: self.sync_interval_minutes,
                download_window_days: self.download_window_days,
                auto_empty_trash_days: self.auto_empty_trash_days,
                auto_empty_junk_days: self.auto_empty_junk_days,
                hidden_folders: self.hidden_folders.clone(),
                storage_dir: base.storage_dir,
            };
            return (cfg, String::new(), None);
        }

        let imap_user = self.imap_user.read(cx).value().trim().to_string();
        let smtp_user = self.smtp_user.read(cx).value().trim().to_string();
        let imap_port = self
            .imap_port
            .read(cx)
            .value()
            .trim()
            .parse::<u16>()
            .unwrap_or(993);
        let smtp_port = self
            .smtp_port
            .read(cx)
            .value()
            .trim()
            .parse::<u16>()
            .unwrap_or(465);

        let cfg = AccountConfig {
            id,
            auth_backend: AuthBackend::Password,
            oauth_client_id: String::new(),
            label,
            enabled: self.original_enabled,
            display_name: self.display_name.read(cx).value().trim().to_string(),
            email: email.clone(),
            imap_host: self.imap_host.read(cx).value().trim().to_string(),
            imap_port,
            imap_security: if self.imap_tls {
                SecurityMode::Tls
            } else {
                SecurityMode::StartTls
            },
            imap_username: if imap_user.is_empty() {
                email
            } else {
                imap_user
            },
            smtp_host: self.smtp_host.read(cx).value().trim().to_string(),
            smtp_port,
            smtp_security: if self.smtp_tls {
                SecurityMode::Tls
            } else {
                SecurityMode::StartTls
            },
            smtp_username: smtp_user,
            smtp_separate_credentials: self.smtp_separate,
            signature: primary_sig,
            color,
            emoji,
            avatar: avatar_file,
            use_brand_avatar,
            in_unified: self.in_unified,
            push_override: self.push_override,
            quota_used_bytes: self.quota_used_bytes,
            quota_total_bytes: self.quota_total_bytes,
            aliases: self.aliases.clone(),
            signatures: collected_signatures,
            default_new_signature: self.default_new_signature,
            default_reply_signature: self.default_reply_signature,
            special_folders: self.special_folders.clone(),
            sync_interval_minutes: self.sync_interval_minutes,
            download_window_days: self.download_window_days,
            auto_empty_trash_days: self.auto_empty_trash_days,
            auto_empty_junk_days: self.auto_empty_junk_days,
            hidden_folders: self.hidden_folders.clone(),
            storage_dir: existing_acc.and_then(|a| a.storage_dir.clone()),
        };
        let imap_pwd = self.imap_pass.read(cx).value().to_string();
        let smtp_pwd = if self.smtp_separate {
            let v = self.smtp_pass.read(cx).value().to_string();
            if v.is_empty() {
                None
            } else {
                Some(v)
            }
        } else {
            None
        };
        (cfg, imap_pwd, smtp_pwd)
    }
}

/// 第一层：邮箱账号列表页（单张 `boxed-list` 卡片 + 拖拽排序 + 启用开关 + 添加按钮）。
pub(crate) fn render_list_page(
    win: &SettingsWindow,
    app_entity: Entity<HylkiApp>,
    p: &HylkiPalette,
    cx: &mut Context<SettingsWindow>,
) -> AnyElement {
    let (accounts_list, demo_mode, cfg_feedback) = {
        let app = app_entity.read(cx);
        (
            app.accounts.clone(),
            app.demo_mode,
            app.cfg_feedback.clone(),
        )
    };
    let highlighted = win.highlighted_row_id == Some("accounts-manage");
    let data_dir = config::data_dir();
    let accent = p.accent;
    let border = p.border;
    let text = p.text;
    let muted_fg = p.text_muted;
    let surface_raised = p.surface_raised;
    let surface = p.surface;
    let danger = p.danger;

    let mut card = v_flex()
        .w_full()
        .bg(if highlighted {
            accent.opacity(0.10)
        } else {
            surface_raised
        })
        .border_1()
        .border_color(if highlighted { accent } else { border })
        .rounded(px(12.0))
        .overflow_hidden();

    if accounts_list.is_empty() {
        card = card.child(
            v_flex()
                .w_full()
                .py_10()
                .px_4()
                .items_center()
                .justify_center()
                .gap_2()
                .child(
                    Icon::new(IconName::CircleUser)
                        .with_size(px(32.0))
                        .text_color(muted_fg),
                )
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(text)
                        .child("尚未添加任何邮箱账号"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted_fg)
                        .child("支持 Microsoft 365 / Outlook OAuth2 及主流 IMAP / SMTP 服务商"),
                )
                .when(!demo_mode, |el| {
                    el.child(
                        div().pt_1().child(
                            Button::new("acct-empty-add")
                                .primary()
                                .compact()
                                .icon(IconName::Plus)
                                .label("添加账号…")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_new_account_editor(window, cx);
                                })),
                        ),
                    )
                }),
        );
    } else {
        let total = accounts_list.len();
        for (idx, acc) in accounts_list.iter().enumerate() {
            let acc_id = acc.id;
            let display_label = acc.display_label();
            let secondary = if display_label == acc.email {
                acc.display_name.trim().to_string()
            } else {
                acc.email.clone()
            };
            let brand = brand_for_account(acc);
            let enabled = acc.enabled;
            let app_for_toggle = app_entity.clone();
            let app_for_drop = app_entity.clone();
            let circle = avatar::account_circle_visual(
                &display_label,
                &acc.email,
                acc.color.as_deref(),
                acc.emoji.as_deref(),
                acc.avatar.as_deref(),
                if acc.use_brand_avatar { brand } else { None },
                &data_dir,
                30.0,
                2.0,
                1.0,
            );
            let drag_payload = AccountRowDrag {
                index: idx,
                label: SharedString::from(display_label.clone()),
            };

            let mut row = h_flex()
                .id(("acct-row", acc_id))
                .w_full()
                .min_h(px(62.0))
                .px_3p5()
                .py_2p5()
                .gap_3()
                .items_center()
                .cursor_pointer()
                .hover(move |s| s.bg(surface.opacity(0.65)))
                .on_click(cx.listener(move |this, _, window, cx| {
                    // 任务二：列表行跳转编辑同样接入未保存确认（保存 / 放弃 / 继续编辑）。
                    this.request_edit_account_editor(acc_id, window, cx);
                }));

            if !demo_mode {
                row = row
                    .on_drag(drag_payload, |drag, _, _, cx| cx.new(|_| drag.clone()))
                    .drag_over::<AccountRowDrag>(move |style, dragged, _, _| {
                        if dragged.index > idx {
                            style.border_t_2().border_color(accent)
                        } else if dragged.index < idx {
                            style.border_b_2().border_color(accent)
                        } else {
                            style
                        }
                    })
                    .on_drop::<AccountRowDrag>(cx.listener(
                        move |_, dragged: &AccountRowDrag, _, cx| {
                            if dragged.index != idx {
                                app_for_drop.update(cx, |app, cx| {
                                    app.reorder_accounts(dragged.index, idx, cx);
                                });
                            }
                        },
                    ));
            }

            row = row
                .child(
                    Icon::new(IconName::GripVertical)
                        .small()
                        .text_color(muted_fg.opacity(if demo_mode { 0.3 } else { 0.75 })),
                )
                .child(div().when(!enabled, |el| el.opacity(0.55)).child(circle))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_0p5()
                        .when(!enabled, |el| el.opacity(0.6))
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(text)
                                .truncate()
                                .child(SharedString::from(display_label)),
                        )
                        .when(!secondary.is_empty(), |el| {
                            el.child(
                                div()
                                    .text_xs()
                                    .text_color(muted_fg)
                                    .truncate()
                                    .child(SharedString::from(secondary)),
                            )
                        }),
                )
                .child(
                    div()
                        .w(px(44.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .flex_shrink_0()
                        .child(render_brand_mark(brand, 24.0)),
                )
                .child(
                    div()
                        .px_2()
                        .py_0p5()
                        .rounded(px(6.0))
                        .bg(surface)
                        .border_1()
                        .border_color(border)
                        .text_xs()
                        .text_color(muted_fg)
                        .flex_shrink_0()
                        .child("KiteMail"),
                )
                .child(
                    div()
                        .id(("acct-toggle-wrap", acc_id))
                        .flex_shrink_0()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .when(demo_mode, |el| {
                            el.opacity(0.45).child(
                                Switch::new(("acct-enable-disabled", acc_id)).checked(enabled),
                            )
                        })
                        .when(!demo_mode, |el| {
                            el.child(
                                Switch::new(("acct-enable", acc_id))
                                    .checked(enabled)
                                    .on_click(move |next: &bool, _, cx| {
                                        let next_val = *next;
                                        app_for_toggle.update(cx, |app, cx| {
                                            app.set_account_enabled(acc_id, next_val, cx);
                                        });
                                    }),
                            )
                        }),
                )
                .child(
                    Icon::new(IconName::ChevronRight)
                        .xsmall()
                        .text_color(muted_fg),
                );

            card = card.child(row);
            if idx + 1 < total {
                card = card.child(div().w_full().h(px(1.0)).bg(border));
            }
        }
    }

    v_flex()
        .id("accounts-list-scroll")
        .flex_1()
        .min_h_0()
        .px_6()
        .py_5()
        .overflow_y_scrollbar()
        .child(
            v_flex()
                .w_full()
                .max_w(px(CARD_MAX_WIDTH))
                .mx_auto()
                .gap_4()
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_0p5()
                                .child(
                                    div()
                                        .text_lg()
                                        .font_weight(FontWeight::BOLD)
                                        .text_color(text)
                                        .child("邮箱账号"),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(muted_fg)
                                        .child("管理已连接的邮箱账号，拖拽调整侧栏显示顺序"),
                                ),
                        )
                        .when(!demo_mode, |el| {
                            el.child(
                                Button::new("acct-header-add")
                                    .primary()
                                    .compact()
                                    .icon(IconName::Plus)
                                    .label("添加账号…")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_new_account_editor(window, cx);
                                    })),
                            )
                        }),
                )
                .when(demo_mode, |el| {
                    el.child(
                        v_flex()
                            .w_full()
                            .p_3()
                            .gap_1()
                            .rounded(px(10.0))
                            .border_1()
                            .border_color(accent.opacity(0.45))
                            .bg(p.accent_soft())
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(text)
                                    .child("当前处于演示模式"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted_fg)
                                    .child("演示模式下使用内置模拟数据，账号配置只读。"),
                            ),
                    )
                })
                .when_some(cfg_feedback.as_ref(), |el, (ok, msg)| {
                    el.child(
                        div()
                            .px_3()
                            .py_2()
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(if *ok {
                                accent.opacity(0.45)
                            } else {
                                danger.opacity(0.45)
                            })
                            .bg(if *ok {
                                p.accent_soft()
                            } else {
                                danger.opacity(0.10)
                            })
                            .text_xs()
                            .text_color(if *ok { accent } else { danger })
                            .child(msg.clone()),
                    )
                })
                .child(card),
        )
        .into_any_element()
}

/// 表单内标准输入行（左固定标签 + 右全宽输入区域）。
fn form_entry_row(label: &str, control: AnyElement, muted_fg: Hsla) -> AnyElement {
    h_flex()
        .w_full()
        .min_h(px(52.0))
        .px_4()
        .py_2()
        .gap_3()
        .items_center()
        .child(
            div()
                .w(px(128.0))
                .flex_shrink_0()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(muted_fg)
                .child(SharedString::from(label.to_string())),
        )
        .child(div().flex_1().min_w_0().child(control))
        .into_any_element()
}

/// 第二层：账号添加 / 编辑页（顶栏由 `SettingsWindow` 提供返回 / 移除 / 保存）。
pub(crate) fn render_editor_page(
    win: &SettingsWindow,
    app_entity: Entity<HylkiApp>,
    p: &HylkiPalette,
    cx: &mut Context<SettingsWindow>,
) -> AnyElement {
    let form = &win.account_form;
    let provider = form.current_provider();
    let is_microsoft = provider.kind == ProviderKind::Microsoft;
    let editing_id = form.editing_account_id;

    let (existing_accounts, demo_mode, cfg_feedback, folders_by_account) = {
        let app = app_entity.read(cx);
        (
            app.accounts.clone(),
            app.demo_mode,
            app.cfg_feedback.clone(),
            app.folders_by_account.clone(),
        )
    };
    let existing_account = editing_id.and_then(|id| existing_accounts.iter().find(|a| a.id == id));
    let existing_brand = existing_account.and_then(brand_for_account);
    let top_brand = provider.brand.or(existing_brand);
    let oauth_locked =
        editing_id.is_some() && form.original_auth_backend == AuthBackend::MicrosoftGraphOAuth2;

    let data_dir = config::data_dir();
    let (accent, danger, text, muted_fg, border, surface) = (
        p.accent,
        p.danger,
        p.text,
        p.text_muted,
        p.border,
        p.surface,
    );

    let sync_status_text = match editing_id.and_then(|id| folders_by_account.get(&id)) {
        Some(folders) if !folders.is_empty() => {
            let total: usize = folders.iter().map(|f| f.total_count).sum();
            let unread: u32 = folders.iter().map(|f| f.unread_count).sum();
            format!("已同步：{total} 封邮件 / {unread} 未读")
        }
        _ if editing_id.is_some() => "尚未完成首次同步".to_string(),
        _ => String::new(),
    };

    // ---- 分组 1：邮箱账号 ----
    let mut account_rows: Vec<AnyElement> = Vec::new();
    let win_entity = cx.entity();
    let provider_combo = Button::new("acct-provider-combo")
        .outline()
        .compact()
        .disabled(oauth_locked || demo_mode)
        .label(provider.label)
        .icon(IconName::ChevronDown)
        .dropdown_menu(move |mut menu, _, cx| {
            let cur = win_entity.read(cx).account_form.provider_idx;
            for (idx, item) in ACCOUNT_PROVIDERS.iter().enumerate() {
                let win_for_item = win_entity.clone();
                menu = menu.item(PopupMenuItem::new(item.label).checked(idx == cur).on_click(
                    move |_, window, cx| {
                        let _ = win_for_item.update(cx, |win, cx| {
                            win.account_form.select_provider(idx, window, cx);
                            cx.notify();
                        });
                    },
                ));
            }
            menu
        })
        .into_any_element();

    account_rows.push(
        h_flex()
            .w_full()
            .min_h(px(52.0))
            .px_4()
            .py_2p5()
            .gap_3()
            .items_center()
            .justify_between()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(text)
                            .child("服务商"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted_fg)
                            .child(SharedString::from(provider.hint)),
                    ),
            )
            .child(div().flex_shrink_0().child(provider_combo))
            .into_any_element(),
    );

    if is_microsoft {
        if let Some(acc) = existing_account {
            account_rows.push(form_entry_row(
                "账号邮箱",
                div()
                    .text_sm()
                    .text_color(text)
                    .child(SharedString::from(acc.email.clone()))
                    .into_any_element(),
                muted_fg,
            ));
            account_rows.push(form_entry_row(
                "昵称",
                Input::new(&form.label).into_any_element(),
                muted_fg,
            ));
            account_rows.push(form_entry_row(
                "寄件人名称",
                Input::new(&form.display_name).into_any_element(),
                muted_fg,
            ));
        }

        let app_for_oauth = app_entity.clone();
        let oauth_client_entity = form.oauth_client_id.clone();
        let oauth_email_entity = form.email.clone();
        account_rows.push(
            h_flex()
                .w_full()
                .min_h(px(54.0))
                .px_4()
                .py_2p5()
                .gap_3()
                .items_center()
                .justify_between()
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_0p5()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(text)
                                .child("Microsoft OAuth2 授权"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted_fg)
                                .child("无需输入密码，通过系统浏览器完成 Microsoft 账号登录"),
                        ),
                )
                .child(
                    Button::new("acct-ms-oauth-btn")
                        .primary()
                        .compact()
                        .disabled(demo_mode)
                        .label(if editing_id.is_some() {
                            "重新授权 Microsoft 账号"
                        } else {
                            "使用 Microsoft 账号登录"
                        })
                        .on_click(cx.listener(move |_, _, _, cx| {
                            let client_id = oauth_client_entity.read(cx).value().trim().to_string();
                            let login_hint = oauth_email_entity.read(cx).value().trim().to_string();
                            app_for_oauth.update(cx, |app, cx| {
                                app.start_outlook_oauth(client_id, login_hint, editing_id, cx);
                            });
                        })),
                )
                .into_any_element(),
        );

        let expanded = form.ms_advanced_expanded;
        account_rows.push(
            v_flex()
                .w_full()
                .child(
                    h_flex()
                        .id("acct-ms-adv-toggle")
                        .w_full()
                        .min_h(px(46.0))
                        .px_4()
                        .py_2()
                        .items_center()
                        .justify_between()
                        .cursor_pointer()
                        .hover(move |s| s.bg(surface.opacity(0.55)))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.account_form.ms_advanced_expanded =
                                !this.account_form.ms_advanced_expanded;
                            cx.notify();
                        }))
                        .child(
                            div()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(muted_fg)
                                .child("高级：自定义 Entra Client ID"),
                        )
                        .child(
                            Icon::new(if expanded {
                                IconName::ChevronUp
                            } else {
                                IconName::ChevronDown
                            })
                            .xsmall()
                            .text_color(muted_fg),
                        ),
                )
                .when(expanded, |el| {
                    el.child(div().px_4().pb_3().child(Input::new(&form.oauth_client_id)))
                })
                .into_any_element(),
        );
    } else {
        account_rows.push(form_entry_row(
            "昵称",
            Input::new(&form.label).into_any_element(),
            muted_fg,
        ));
        account_rows.push(form_entry_row(
            "寄件人名称",
            Input::new(&form.display_name).into_any_element(),
            muted_fg,
        ));
        account_rows.push(form_entry_row(
            "邮箱地址",
            Input::new(&form.email).into_any_element(),
            muted_fg,
        ));

        let win_for_imap_sec = cx.entity();
        account_rows.push(form_entry_row(
            "IMAP 服务器",
            h_flex()
                .w_full()
                .gap_2()
                .items_center()
                .child(div().flex_1().min_w_0().child(Input::new(&form.imap_host)))
                .child(
                    div()
                        .w(px(74.0))
                        .flex_shrink_0()
                        .child(Input::new(&form.imap_port)),
                )
                .child(combo_button(
                    "acct-imap-sec",
                    if form.imap_tls { "SSL/TLS" } else { "STARTTLS" },
                    SECURITY_OPTIONS,
                    form.imap_tls,
                    demo_mode,
                    move |tls, _, cx| {
                        let _ = win_for_imap_sec.update(cx, |win, cx| {
                            win.account_form.imap_tls = tls;
                            cx.notify();
                        });
                    },
                ))
                .into_any_element(),
            muted_fg,
        ));
        account_rows.push(form_entry_row(
            "IMAP 用户名",
            Input::new(&form.imap_user).into_any_element(),
            muted_fg,
        ));
        account_rows.push(form_entry_row(
            if editing_id.is_some() {
                "密码 / 授权码（留空保留）"
            } else {
                "密码 / 授权码"
            },
            Input::new(&form.imap_pass).mask_toggle().into_any_element(),
            muted_fg,
        ));

        let win_for_smtp_sec = cx.entity();
        account_rows.push(form_entry_row(
            "SMTP 服务器",
            h_flex()
                .w_full()
                .gap_2()
                .items_center()
                .child(div().flex_1().min_w_0().child(Input::new(&form.smtp_host)))
                .child(
                    div()
                        .w(px(74.0))
                        .flex_shrink_0()
                        .child(Input::new(&form.smtp_port)),
                )
                .child(combo_button(
                    "acct-smtp-sec",
                    if form.smtp_tls { "SSL/TLS" } else { "STARTTLS" },
                    SECURITY_OPTIONS,
                    form.smtp_tls,
                    demo_mode,
                    move |tls, _, cx| {
                        let _ = win_for_smtp_sec.update(cx, |win, cx| {
                            win.account_form.smtp_tls = tls;
                            cx.notify();
                        });
                    },
                ))
                .into_any_element(),
            muted_fg,
        ));

        account_rows.push(
            h_flex()
                .w_full()
                .min_h(px(48.0))
                .px_4()
                .py_2()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(text)
                        .child("SMTP 使用独立用户名与授权码"),
                )
                .child(
                    Switch::new("acct-smtp-separate")
                        .checked(form.smtp_separate)
                        .on_click(cx.listener(|this, next: &bool, _, cx| {
                            this.account_form.smtp_separate = *next;
                            cx.notify();
                        })),
                )
                .into_any_element(),
        );

        if form.smtp_separate {
            account_rows.push(form_entry_row(
                "SMTP 用户名",
                Input::new(&form.smtp_user).into_any_element(),
                muted_fg,
            ));
            account_rows.push(form_entry_row(
                "SMTP 授权码",
                Input::new(&form.smtp_pass).mask_toggle().into_any_element(),
                muted_fg,
            ));
        }

        let app_for_verify = app_entity.clone();
        account_rows.push(
            h_flex()
                .w_full()
                .min_h(px(50.0))
                .px_4()
                .py_2()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_xs()
                        .text_color(muted_fg)
                        .child("保存前可测试 IMAP / SMTP 服务器连通性与凭据"),
                )
                .child(
                    Button::new("acct-verify-btn")
                        .outline()
                        .compact()
                        .disabled(demo_mode)
                        .label("测试连接")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let existing = app_for_verify.read(cx).accounts.clone();
                            let (cfg, imap_pwd, smtp_pwd) =
                                this.account_form.collect(&existing, cx);
                            app_for_verify.update(cx, |app, cx| {
                                app.verify_account_config(cfg, imap_pwd, smtp_pwd, cx);
                            });
                        })),
                )
                .into_any_element(),
        );
    }

    account_rows.push(
        h_flex()
            .w_full()
            .min_h(px(52.0))
            .px_4()
            .py_2()
            .gap_3()
            .items_center()
            .justify_between()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(text)
                            .child("在「所有收件箱」中显示"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted_fg)
                            .child("关闭后仅在左侧对应账号文件夹中查看，不混入统一视图"),
                    ),
            )
            .child(
                Switch::new("acct-in-unified")
                    .checked(form.in_unified)
                    .on_click(cx.listener(|this, next: &bool, _, cx| {
                        this.account_form.in_unified = *next;
                        cx.notify();
                    })),
            )
            .into_any_element(),
    );

    // ---- 分组 2：外观（72px 实时预览 + 10 色板 + 首字母/Emoji/图片） ----
    let preview_email = form.email.read(cx).value().trim().to_string();
    let preview_label = {
        let lbl = form.label.read(cx).value().trim().to_string();
        if !lbl.is_empty() {
            lbl
        } else {
            let dn = form.display_name.read(cx).value().trim().to_string();
            if !dn.is_empty() {
                dn
            } else {
                preview_email.clone()
            }
        }
    };
    let preview_imap_host = form.imap_host.read(cx).value().trim().to_string();
    let preview_imap_port = form
        .imap_port
        .read(cx)
        .value()
        .trim()
        .parse::<u16>()
        .unwrap_or(993);
    let preview_auth_backend = if oauth_locked || provider.kind == ProviderKind::Microsoft {
        AuthBackend::MicrosoftGraphOAuth2
    } else {
        AuthBackend::Password
    };
    let preview_brand = provider
        .brand
        .or_else(|| {
            brand_for_account_fields(
                preview_auth_backend,
                &preview_imap_host,
                preview_imap_port,
                &preview_email,
            )
        })
        .or(existing_brand);
    let preview_disc = avatar::account_circle_visual(
        &preview_label,
        &preview_email,
        form.color.as_deref(),
        if form.avatar_mode == AvatarDisplayMode::InitialsOrEmoji {
            form.emoji.as_deref()
        } else {
            None
        },
        if form.avatar_mode == AvatarDisplayMode::Picture {
            form.avatar.as_deref()
        } else {
            None
        },
        if form.avatar_mode == AvatarDisplayMode::BrandDefault {
            preview_brand
        } else {
            None
        },
        &data_dir,
        72.0,
        3.0,
        2.0,
    );

    let effective_hex = avatar::account_accent_hex(&preview_email, form.color.as_deref());
    let swatches = ACCOUNT_ACCENT_COLORS
        .iter()
        .enumerate()
        .map(|(idx, (hex, _name))| {
            let selected = effective_hex.eq_ignore_ascii_case(hex);
            let swatch_color = avatar::parse_hex_color(hex).unwrap_or(accent);
            let hex_owned = (*hex).to_string();
            div()
                .id(("acct-swatch", idx))
                .size(px(26.0))
                .rounded_full()
                .p(px(2.0))
                .border(px(if selected { 2.0 } else { 1.0 }))
                .border_color(if selected { swatch_color } else { border })
                .cursor_pointer()
                .flex()
                .items_center()
                .justify_center()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.account_form.color = Some(hex_owned.clone());
                    cx.notify();
                }))
                .child(
                    div()
                        .size_full()
                        .rounded_full()
                        .bg(swatch_color)
                        .flex()
                        .items_center()
                        .justify_center()
                        .when(selected, |el| {
                            el.child(Icon::new(IconName::Check).xsmall().text_color(white()))
                        }),
                )
                .into_any_element()
        });

    let mut appearance_rows: Vec<AnyElement> = vec![
        div()
            .w_full()
            .py_4()
            .flex()
            .items_center()
            .justify_center()
            .child(preview_disc)
            .into_any_element(),
        h_flex()
            .w_full()
            .min_h(px(56.0))
            .px_4()
            .py_2p5()
            .gap_3()
            .items_center()
            .justify_between()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(text)
                    .child("强调色"),
            )
            .child(
                h_flex()
                    .gap_1p5()
                    .items_center()
                    .flex_wrap()
                    .children(swatches)
                    .child(
                        Button::new("acct-color-reset")
                            .ghost()
                            .compact()
                            .disabled(form.color.is_none())
                            .label("默认")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.account_form.color = None;
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element(),
        h_flex()
            .w_full()
            .min_h(px(52.0))
            .px_4()
            .py_2()
            .gap_3()
            .items_center()
            .justify_between()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(text)
                    .child("头像显示"),
            )
            .child(
                h_flex()
                    .gap_1()
                    .p_0p5()
                    .rounded(px(8.0))
                    .bg(surface)
                    .border_1()
                    .border_color(border)
                    .child(
                        Button::new("acct-mode-brand")
                            .ghost()
                            .compact()
                            .selected(form.avatar_mode == AvatarDisplayMode::BrandDefault)
                            .label("默认")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.account_form.avatar_mode = AvatarDisplayMode::BrandDefault;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("acct-mode-text")
                            .ghost()
                            .compact()
                            .selected(form.avatar_mode == AvatarDisplayMode::InitialsOrEmoji)
                            .label("首字母或 Emoji")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.account_form.avatar_mode = AvatarDisplayMode::InitialsOrEmoji;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("acct-mode-picture")
                            .ghost()
                            .compact()
                            .selected(form.avatar_mode == AvatarDisplayMode::Picture)
                            .label("图片")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.account_form.avatar_mode = AvatarDisplayMode::Picture;
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element(),
    ];

    match form.avatar_mode {
        AvatarDisplayMode::BrandDefault => {
            let brand_hint = if preview_brand.is_some() {
                "默认使用服务商品牌图标；如需文字或自定义图片，可切换上方选项"
            } else {
                "当前邮箱未匹配到品牌图标，将自动使用首字母头像"
            };
            appearance_rows.push(
                div()
                    .w_full()
                    .px_4()
                    .py_2p5()
                    .text_xs()
                    .text_color(muted_fg)
                    .child(brand_hint)
                    .into_any_element(),
            );
        }
        AvatarDisplayMode::InitialsOrEmoji => {
            let win_for_emoji = cx.entity();
            appearance_rows.push(
                h_flex()
                    .w_full()
                    .min_h(px(54.0))
                    .px_4()
                    .py_2()
                    .gap_3()
                    .items_center()
                    .justify_between()
                    .child(
                        v_flex()
                            .w(px(140.0))
                            .flex_shrink_0()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(text)
                                    .child("Emoji（可选）"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted_fg)
                                    .child("留空自动使用首字母"),
                            ),
                    )
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(Input::new(&form.emoji_input)),
                            )
                            .child(
                                Button::new("acct-emoji-picker")
                                    .outline()
                                    .compact()
                                    .label("预设")
                                    .icon(IconName::ChevronDown)
                                    .dropdown_menu(move |mut menu, _, _| {
                                        for emoji_choice in QUICK_EMOJIS {
                                            let win_ref = win_for_emoji.clone();
                                            let glyph = *emoji_choice;
                                            menu = menu.item(PopupMenuItem::new(glyph).on_click(
                                                move |_, window, cx| {
                                                    let _ = win_ref.update(cx, |win, cx| {
                                                        win.account_form.emoji =
                                                            Some(glyph.to_string());
                                                        win.account_form
                                                            .emoji_input
                                                            .update(cx, |s, cx| {
                                                                s.set_value(glyph, window, cx)
                                                            });
                                                        cx.notify();
                                                    });
                                                },
                                            ));
                                        }
                                        let win_clear = win_for_emoji.clone();
                                        menu.separator().item(
                                            PopupMenuItem::new("清除 Emoji").on_click(
                                                move |_, window, cx| {
                                                    let _ = win_clear.update(cx, |win, cx| {
                                                        win.account_form.emoji = None;
                                                        win.account_form
                                                            .emoji_input
                                                            .update(cx, |s, cx| {
                                                                s.set_value("", window, cx)
                                                            });
                                                        cx.notify();
                                                    });
                                                },
                                            ),
                                        )
                                    }),
                            ),
                    )
                    .into_any_element(),
            );
        }
        AvatarDisplayMode::Picture => {
            let has_avatar = form.avatar.is_some();
            let file_label = form
                .avatar
                .as_deref()
                .unwrap_or("尚未选择图片（支持 PNG / JPG / WebP / GIF）");
            appearance_rows.push(
                h_flex()
                    .w_full()
                    .min_h(px(56.0))
                    .px_4()
                    .py_2()
                    .gap_3()
                    .items_center()
                    .justify_between()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(text)
                                    .child("自定义头像图片"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted_fg)
                                    .truncate()
                                    .child(SharedString::from(file_label.to_string())),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_shrink_0()
                            .child(
                                Button::new("acct-pick-avatar")
                                    .outline()
                                    .compact()
                                    .icon(IconName::Image)
                                    .label("选择图片…")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if this.file_dialog_open {
                                            return;
                                        }
                                        this.file_dialog_open = true;
                                        let generation = this.account_form.session_generation;
                                        cx.spawn(async move |this, cx| {
                                            let src = cx
                                                .background_executor()
                                                .spawn(async move {
                                                    rfd::FileDialog::new()
                                                        .set_title("选择账号头像图片")
                                                        .add_filter(
                                                            "图片文件",
                                                            &["png", "jpg", "jpeg", "webp", "gif"],
                                                        )
                                                        .pick_file()
                                                })
                                                .await;
                                            let _ = this.update(cx, |this, _| {
                                                this.file_dialog_open = false
                                            });
                                            if let Some(src) = src {
                                                let _ = this.update(cx, |this, cx| {
                                                    if !this.account_editor_open
                                                        || this.account_form.session_generation
                                                            != generation
                                                    {
                                                        return;
                                                    }
                                                    match config::import_avatar_file_in(
                                                        &config::data_dir(),
                                                        &src,
                                                    ) {
                                                        Ok(filename) => {
                                                            this.account_form.avatar =
                                                                Some(filename);
                                                            this.account_form.avatar_mode =
                                                                AvatarDisplayMode::Picture;
                                                            this.account_form.validation_error =
                                                                None;
                                                        }
                                                        Err(err) => {
                                                            this.account_form.validation_error =
                                                                Some(SharedString::from(err));
                                                        }
                                                    }
                                                    cx.notify();
                                                });
                                            }
                                        })
                                        .detach();
                                    })),
                            )
                            .when(has_avatar, |el| {
                                el.child(
                                    Button::new("acct-clear-avatar")
                                        .ghost()
                                        .compact()
                                        .label("移除图片")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.account_form.avatar = None;
                                            cx.notify();
                                        })),
                                )
                            }),
                    )
                    .into_any_element(),
            );
        }
    }

    // ---- WP-06 分组 0：存储配额（Storage） ----
    let storage_group: Option<AnyElement> = if editing_id.is_some() {
        let (quota_subtitle, quota_ratio) = match (form.quota_used_bytes, form.quota_total_bytes) {
            (Some(used), Some(total)) if total > 0 => {
                let pct = ((used as f64 / total as f64) * 100.0).clamp(0.0, 100.0);
                (
                    format!(
                        "{} / {} ({:.0}%)",
                        config::format_bytes(used),
                        config::format_bytes(total),
                        pct
                    ),
                    Some((used as f32 / total as f32).clamp(0.0, 1.0)),
                )
            }
            _ => (
                "服务器未上报配额（IMAP GETQUOTAROOT 同步后自动更新）".to_string(),
                None,
            ),
        };
        let quota_bar: AnyElement = if let Some(ratio) = quota_ratio {
            div()
                .w(px(160.0))
                .h(px(8.0))
                .rounded(px(4.0))
                .bg(border)
                .overflow_hidden()
                .child(
                    div()
                        .h_full()
                        .w(px(160.0 * ratio))
                        .rounded(px(4.0))
                        .bg(if ratio > 0.9 { danger } else { accent }),
                )
                .into_any_element()
        } else {
            div()
                .text_xs()
                .text_color(muted_fg)
                .child("无配额上限")
                .into_any_element()
        };
        Some(setting_group(
            "存储空间",
            None,
            CARD_MAX_WIDTH,
            vec![h_flex()
                .w_full()
                .min_h(px(54.0))
                .px_4()
                .py_2()
                .gap_3()
                .items_center()
                .justify_between()
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_0p5()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(text)
                                .child("邮箱配额"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted_fg)
                                .child(SharedString::from(quota_subtitle)),
                        ),
                )
                .child(quota_bar)
                .into_any_element()],
            p,
        ))
    } else {
        None
    };

    // ---- WP-06 分组 3：发信别名（Send-as Aliases） ----
    let mut alias_rows: Vec<AnyElement> = Vec::new();
    for (idx, alias) in form.aliases.iter().enumerate() {
        let title = if alias.name.trim().is_empty() {
            alias.email.clone()
        } else {
            format!("{} <{}>", alias.name.trim(), alias.email.trim())
        };
        let subtitle = if alias.reply_to.trim().is_empty() {
            if alias.is_default {
                "默认发信身份".to_string()
            } else {
                "使用主账号 SMTP 发送".to_string()
            }
        } else {
            format!("Reply-To: {}", alias.reply_to.trim())
        };
        let is_def = alias.is_default;
        alias_rows.push(
            h_flex()
                .w_full()
                .min_h(px(52.0))
                .px_4()
                .py_2()
                .gap_2()
                .items_center()
                .justify_between()
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_0p5()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(text)
                                .truncate()
                                .child(SharedString::from(title)),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted_fg)
                                .truncate()
                                .child(SharedString::from(subtitle)),
                        ),
                )
                .child(
                    h_flex()
                        .gap_1p5()
                        .flex_shrink_0()
                        .child(
                            Button::new(("acct-alias-default", idx))
                                .outline()
                                .compact()
                                .disabled(demo_mode)
                                .label(if is_def { "默认" } else { "设为默认" })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    for (i, item) in
                                        this.account_form.aliases.iter_mut().enumerate()
                                    {
                                        item.is_default = i == idx;
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new(("acct-alias-del", idx))
                                .ghost()
                                .compact()
                                .disabled(demo_mode)
                                .icon(IconName::Trash)
                                .tooltip("移除此别名")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if idx < this.account_form.aliases.len() {
                                        this.account_form.aliases.remove(idx);
                                        cx.notify();
                                    }
                                })),
                        ),
                )
                .into_any_element(),
        );
    }
    alias_rows.push(
        v_flex()
            .w_full()
            .px_4()
            .py_3()
            .gap_2()
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&form.alias_name_input).cleanable(true)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&form.alias_email_input).cleanable(true)),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .child(Input::new(&form.alias_reply_to_input).cleanable(true)),
                    )
                    .child(
                        Button::new("acct-alias-add")
                            .outline()
                            .compact()
                            .disabled(demo_mode)
                            .icon(IconName::Plus)
                            .label("添加别名")
                            .on_click(cx.listener(|this, _, window, cx| {
                                let email = this
                                    .account_form
                                    .alias_email_input
                                    .read(cx)
                                    .value()
                                    .trim()
                                    .to_string();
                                if email.is_empty() || !email.contains('@') {
                                    this.account_form.validation_error =
                                        Some("请输入有效的别名邮箱地址".into());
                                    cx.notify();
                                    return;
                                }
                                let name = this
                                    .account_form
                                    .alias_name_input
                                    .read(cx)
                                    .value()
                                    .trim()
                                    .to_string();
                                let reply_to = this
                                    .account_form
                                    .alias_reply_to_input
                                    .read(cx)
                                    .value()
                                    .trim()
                                    .to_string();
                                let is_default = this.account_form.aliases.is_empty();
                                this.account_form.aliases.push(SendAsAlias {
                                    name,
                                    email,
                                    reply_to,
                                    is_default,
                                });
                                this.account_form
                                    .alias_name_input
                                    .update(cx, |s, cx| s.set_value("", window, cx));
                                this.account_form
                                    .alias_email_input
                                    .update(cx, |s, cx| s.set_value("", window, cx));
                                this.account_form
                                    .alias_reply_to_input
                                    .update(cx, |s, cx| s.set_value("", window, cx));
                                this.account_form.validation_error = None;
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element(),
    );
    let aliases_group = setting_group(
        "发信别名",
        Some("为一个账号配置多个发件身份（名称 + 邮箱 + Reply-To），写信时可在发件人中切换"),
        CARD_MAX_WIDTH,
        alias_rows,
        p,
    );

    // ---- 分组 4：多命名 HTML/文本签名 ----
    let sig_count = form.signatures.len();
    let active_sig_idx = form.active_sig_idx.min(sig_count.saturating_sub(1));
    let active_sig_is_html = form
        .signatures
        .get(active_sig_idx)
        .map(|s| s.is_html)
        .unwrap_or(false);
    let mut sig_tabs = h_flex().w_full().flex_wrap().gap_1p5().items_center();
    for (idx, sig) in form.signatures.iter().enumerate() {
        let selected = idx == active_sig_idx;
        let lbl = if sig.name.trim().is_empty() {
            format!("签名 {}", idx + 1)
        } else {
            sig.name.clone()
        };
        sig_tabs = sig_tabs.child(
            Button::new(("acct-sig-tab", idx))
                .compact()
                .outline()
                .selected(selected)
                .label(SharedString::from(lbl))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.account_form.select_signature(idx, window, cx);
                    cx.notify();
                })),
        );
    }
    sig_tabs = sig_tabs
        .child(
            Button::new("acct-sig-add")
                .ghost()
                .compact()
                .disabled(demo_mode)
                .icon(IconName::Plus)
                .label("新建签名")
                .on_click(cx.listener(|this, _, window, cx| {
                    this.account_form.sync_active_signature_from_inputs(cx);
                    let next_idx = this.account_form.signatures.len();
                    this.account_form.signatures.push(NamedSignature {
                        name: format!("签名 {}", next_idx + 1),
                        body: String::new(),
                        is_html: false,
                    });
                    this.account_form.select_signature(next_idx, window, cx);
                    cx.notify();
                })),
        )
        .when(sig_count > 1, |el| {
            el.child(
                Button::new("acct-sig-del")
                    .ghost()
                    .compact()
                    .disabled(demo_mode)
                    .icon(IconName::Trash)
                    .tooltip("删除当前签名")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if this.account_form.signatures.len() <= 1 {
                            return;
                        }
                        let rm_idx = this.account_form.active_sig_idx;
                        this.account_form.signatures.remove(rm_idx);
                        let next_idx = rm_idx.min(this.account_form.signatures.len() - 1);
                        this.account_form.active_sig_idx = next_idx;
                        this.account_form.sig_preview = false;
                        let sig = this.account_form.signatures[next_idx].clone();
                        this.account_form
                            .sig_name_input
                            .update(cx, |s, cx| s.set_value(sig.name, window, cx));
                        this.account_form
                            .signature
                            .update(cx, |s, cx| s.set_value(sig.body, window, cx));
                        adjust_default_sig_after_remove(
                            &mut this.account_form.default_new_signature,
                            rm_idx,
                        );
                        adjust_default_sig_after_remove(
                            &mut this.account_form.default_reply_signature,
                            rm_idx,
                        );
                        cx.notify();
                    })),
            )
        });

    let mut sig_choice_options: Vec<(SharedString, String)> =
        vec![(SharedString::from("不使用签名"), String::new())];
    for (idx, sig) in form.signatures.iter().enumerate() {
        let lbl = if idx == active_sig_idx {
            let live_name = form.sig_name_input.read(cx).value().trim().to_string();
            if live_name.is_empty() {
                format!("签名 {}", idx + 1)
            } else {
                live_name
            }
        } else if sig.name.trim().is_empty() {
            format!("签名 {}", idx + 1)
        } else {
            sig.name.clone()
        };
        sig_choice_options.push((SharedString::from(lbl), idx.to_string()));
    }
    let current_new_sig_val = form
        .default_new_signature
        .map(|i| i.to_string())
        .unwrap_or_default();
    let current_reply_sig_val = form
        .default_reply_signature
        .map(|i| i.to_string())
        .unwrap_or_default();
    let new_sig_label = sig_choice_options
        .iter()
        .find(|(_, v)| *v == current_new_sig_val)
        .map(|(l, _)| l.clone())
        .unwrap_or_else(|| SharedString::from("不使用签名"));
    let reply_sig_label = sig_choice_options
        .iter()
        .find(|(_, v)| *v == current_reply_sig_val)
        .map(|(l, _)| l.clone())
        .unwrap_or_else(|| SharedString::from("不使用签名"));
    let signature_preview = form.sig_preview && active_sig_is_html;
    let signature_body: AnyElement = if signature_preview {
        let html = signature_preview_html(form.signature.read(cx).value().as_ref(), true);
        div()
            .id("acct-sig-preview")
            .w_full()
            .h(px(150.0))
            .overflow_y_scroll()
            .child(
                TextView::html("acct-sig-preview-text", html)
                    .selectable(true)
                    .image_source(|uri| crate::webview_host::mail_image_source(uri, false)),
            )
            .into_any_element()
    } else {
        Textarea::new(&form.signature).into_any_element()
    };
    let win_for_sig_new = cx.entity();
    let win_for_sig_reply = cx.entity();

    let signature_group = setting_group(
        "签名",
        Some("支持多个命名 HTML/纯文本签名，并分别指定新邮件与回复/转发默认签名"),
        CARD_MAX_WIDTH,
        vec![
            v_flex()
                .w_full()
                .p_3()
                .gap_2p5()
                .child(sig_tabs)
                .child(
                    h_flex()
                        .w_full()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .child(Input::new(&form.sig_name_input).cleanable(true)),
                        )
                        .child(
                            Button::new("acct-sig-html-toggle")
                                .outline()
                                .compact()
                                .disabled(demo_mode)
                                .label(if active_sig_is_html {
                                    "HTML 模式"
                                } else {
                                    "纯文本模式"
                                })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(sig) =
                                        this.account_form.signatures.get_mut(active_sig_idx)
                                    {
                                        sig.is_html = !sig.is_html;
                                    }
                                    this.account_form.sig_preview = false;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("acct-sig-import")
                                .ghost()
                                .compact()
                                .disabled(demo_mode)
                                .label("导入 HTML…")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.import_signature_file(window, cx);
                                })),
                        )
                        .when(active_sig_is_html, |el| {
                            el.child(
                                Button::new("acct-sig-preview-btn")
                                    .ghost()
                                    .compact()
                                    .label(if form.sig_preview {
                                        "编辑源码"
                                    } else {
                                        "预览"
                                    })
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.account_form.sig_preview =
                                            !this.account_form.sig_preview;
                                        cx.notify();
                                    })),
                            )
                        }),
                )
                .child(
                    div()
                        .w_full()
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(border)
                        .bg(surface)
                        .p_1p5()
                        .child(signature_body),
                )
                .into_any_element(),
            h_flex()
                .w_full()
                .min_h(px(50.0))
                .px_4()
                .py_2()
                .gap_3()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(text)
                        .child("新邮件默认签名"),
                )
                .child(combo_button_dynamic(
                    "acct-sig-default-new",
                    new_sig_label,
                    sig_choice_options.clone(),
                    current_new_sig_val,
                    demo_mode,
                    move |val, _, cx| {
                        let _ = win_for_sig_new.update(cx, |win, cx| {
                            win.account_form.default_new_signature = val.parse::<usize>().ok();
                            cx.notify();
                        });
                    },
                ))
                .into_any_element(),
            h_flex()
                .w_full()
                .min_h(px(50.0))
                .px_4()
                .py_2()
                .gap_3()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(text)
                        .child("回复/转发默认签名"),
                )
                .child(combo_button_dynamic(
                    "acct-sig-default-reply",
                    reply_sig_label,
                    sig_choice_options,
                    current_reply_sig_val,
                    demo_mode,
                    move |val, _, cx| {
                        let _ = win_for_sig_reply.update(cx, |win, cx| {
                            win.account_form.default_reply_signature = val.parse::<usize>().ok();
                            cx.notify();
                        });
                    },
                ))
                .into_any_element(),
        ],
        p,
    );

    // ---- WP-06 分组 5：特殊文件夹映射（Special Folders） ----
    let account_folders = editing_id
        .and_then(|id| folders_by_account.get(&id))
        .cloned()
        .unwrap_or_default();
    let mut folder_choices: Vec<(SharedString, String)> =
        vec![(SharedString::from("自动检测"), String::new())];
    for f in &account_folders {
        folder_choices.push((
            SharedString::from(format!("{} ({})", f.name, f.path)),
            f.path.clone(),
        ));
    }
    let special_specs: [(&'static str, &'static str, &'static str, String); 5] = [
        (
            "sent",
            "acct-sf-sent",
            "已发送 (Sent)",
            form.special_folders.sent.clone(),
        ),
        (
            "drafts",
            "acct-sf-drafts",
            "草稿箱 (Drafts)",
            form.special_folders.drafts.clone(),
        ),
        (
            "trash",
            "acct-sf-trash",
            "废纸篓 (Trash)",
            form.special_folders.trash.clone(),
        ),
        (
            "junk",
            "acct-sf-junk",
            "垃圾邮件 (Junk)",
            form.special_folders.junk.clone(),
        ),
        (
            "archive",
            "acct-sf-archive",
            "归档 (Archive)",
            form.special_folders.archive.clone(),
        ),
    ];
    let mut special_rows: Vec<AnyElement> = Vec::new();
    for (role_key, btn_id, role_label, current_path) in special_specs {
        let mut opts = folder_choices.clone();
        if !current_path.is_empty() && !opts.iter().any(|(_, p)| p == &current_path) {
            opts.push((
                SharedString::from(current_path.clone()),
                current_path.clone(),
            ));
        }
        let current_label = opts
            .iter()
            .find(|(_, p)| p == &current_path)
            .map(|(l, _)| l.clone())
            .unwrap_or_else(|| SharedString::from("自动检测"));
        let win_for_sf = cx.entity();
        special_rows.push(
            h_flex()
                .w_full()
                .min_h(px(50.0))
                .px_4()
                .py_2()
                .gap_3()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(text)
                        .child(role_label),
                )
                .child(combo_button_dynamic(
                    btn_id,
                    current_label,
                    opts,
                    current_path,
                    demo_mode,
                    move |val, _, cx| {
                        let _ = win_for_sf.update(cx, |win, cx| {
                            match role_key {
                                "sent" => win.account_form.special_folders.sent = val,
                                "drafts" => win.account_form.special_folders.drafts = val,
                                "trash" => win.account_form.special_folders.trash = val,
                                "junk" => win.account_form.special_folders.junk = val,
                                "archive" => win.account_form.special_folders.archive = val,
                                _ => {}
                            }
                            cx.notify();
                        });
                    },
                ))
                .into_any_element(),
        );
    }
    let win_for_sss = cx.entity();
    special_rows.push(
        h_flex()
            .w_full()
            .min_h(px(54.0))
            .px_4()
            .py_2()
            .gap_3()
            .items_center()
            .justify_between()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(text)
                            .child("服务器自动保存已发送邮件"),
                    )
                    .child(
                        div().text_xs().text_color(muted_fg).child(
                            "Gmail / Outlook 等服务商默认开启，避免重复 APPEND 到已发送文件夹",
                        ),
                    ),
            )
            .child(
                Switch::new("acct-server-saves-sent")
                    .checked(form.special_folders.server_saves_sent)
                    .disabled(demo_mode)
                    .on_click(move |checked, _, cx| {
                        let _ = win_for_sss.update(cx, |win, cx| {
                            win.account_form.special_folders.server_saves_sent = *checked;
                            cx.notify();
                        });
                    }),
            )
            .into_any_element(),
    );
    let special_folders_group = setting_group(
        "特殊文件夹",
        Some("绑定各标准角色对应的远端 IMAP 文件夹"),
        CARD_MAX_WIDTH,
        special_rows,
        p,
    );

    // ---- 分组 6：同步与自动清理策略 ----
    let push_label = PUSH_OVERRIDE_OPTIONS
        .iter()
        .find(|(_, v)| *v == form.push_override)
        .map(|(l, _)| *l)
        .unwrap_or("跟随全局设置");
    let sync_interval_label = ACCOUNT_SYNC_INTERVAL_OPTIONS
        .iter()
        .find(|(_, v)| *v == form.sync_interval_minutes)
        .map(|(l, _)| *l)
        .unwrap_or("跟随全局设置");
    let download_window_label = DOWNLOAD_WINDOW_OPTIONS
        .iter()
        .find(|(_, v)| *v == form.download_window_days)
        .map(|(l, _)| *l)
        .unwrap_or("全部历史");
    let auto_trash_label = AUTO_EMPTY_DAYS_OPTIONS
        .iter()
        .find(|(_, v)| *v == form.auto_empty_trash_days)
        .map(|(l, _)| *l)
        .unwrap_or("从不自动清空");
    let auto_junk_label = AUTO_EMPTY_DAYS_OPTIONS
        .iter()
        .find(|(_, v)| *v == form.auto_empty_junk_days)
        .map(|(l, _)| *l)
        .unwrap_or("从不自动清空");

    let win_for_push = cx.entity();
    let win_for_sync_int = cx.entity();
    let win_for_dl_win = cx.entity();
    let win_for_trash_days = cx.entity();
    let win_for_junk_days = cx.entity();

    let mut sync_rows: Vec<AnyElement> = vec![
        h_flex()
            .w_full()
            .min_h(px(54.0))
            .px_4()
            .py_2()
            .gap_3()
            .items_center()
            .justify_between()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(text)
                            .child("实时推送（IMAP IDLE）"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted_fg)
                            .child("为此账号单独开启或关闭实时新邮件推送"),
                    ),
            )
            .child(combo_button(
                "acct-push-override",
                push_label,
                PUSH_OVERRIDE_OPTIONS,
                form.push_override,
                demo_mode,
                move |val, _, cx| {
                    let _ = win_for_push.update(cx, |win, cx| {
                        win.account_form.push_override = val;
                        cx.notify();
                    });
                },
            ))
            .into_any_element(),
        h_flex()
            .w_full()
            .min_h(px(54.0))
            .px_4()
            .py_2()
            .gap_3()
            .items_center()
            .justify_between()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(text)
                            .child("轮询周期覆盖"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted_fg)
                            .child("覆盖全局定时拉取间隔"),
                    ),
            )
            .child(combo_button(
                "acct-sync-interval",
                sync_interval_label,
                ACCOUNT_SYNC_INTERVAL_OPTIONS,
                form.sync_interval_minutes,
                demo_mode,
                move |val, _, cx| {
                    let _ = win_for_sync_int.update(cx, |win, cx| {
                        win.account_form.sync_interval_minutes = val;
                        cx.notify();
                    });
                },
            ))
            .into_any_element(),
        h_flex()
            .w_full()
            .min_h(px(54.0))
            .px_4()
            .py_2()
            .gap_3()
            .items_center()
            .justify_between()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(text)
                            .child("下载邮件时间范围"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted_fg)
                            .child("控制本地缓存同步的历史时间跨度"),
                    ),
            )
            .child(combo_button(
                "acct-download-window",
                download_window_label,
                DOWNLOAD_WINDOW_OPTIONS,
                form.download_window_days,
                demo_mode,
                move |val, _, cx| {
                    let _ = win_for_dl_win.update(cx, |win, cx| {
                        win.account_form.download_window_days = val;
                        cx.notify();
                    });
                },
            ))
            .into_any_element(),
        h_flex()
            .w_full()
            .min_h(px(52.0))
            .px_4()
            .py_2()
            .gap_3()
            .items_center()
            .justify_between()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(text)
                    .child("自动清空废纸篓"),
            )
            .child(combo_button(
                "acct-auto-empty-trash",
                auto_trash_label,
                AUTO_EMPTY_DAYS_OPTIONS,
                form.auto_empty_trash_days,
                demo_mode,
                move |val, _, cx| {
                    let _ = win_for_trash_days.update(cx, |win, cx| {
                        win.account_form.auto_empty_trash_days = val;
                        cx.notify();
                    });
                },
            ))
            .into_any_element(),
        h_flex()
            .w_full()
            .min_h(px(52.0))
            .px_4()
            .py_2()
            .gap_3()
            .items_center()
            .justify_between()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(text)
                    .child("自动清空垃圾邮件"),
            )
            .child(combo_button(
                "acct-auto-empty-junk",
                auto_junk_label,
                AUTO_EMPTY_DAYS_OPTIONS,
                form.auto_empty_junk_days,
                demo_mode,
                move |val, _, cx| {
                    let _ = win_for_junk_days.update(cx, |win, cx| {
                        win.account_form.auto_empty_junk_days = val;
                        cx.notify();
                    });
                },
            ))
            .into_any_element(),
    ];

    // ---- WP-06 分组 7：隐藏文件夹（Hidden Folders） ----
    let mut hidden_rows: Vec<AnyElement> = Vec::new();
    let mut all_folder_paths: Vec<(String, String)> = account_folders
        .iter()
        .map(|f| (f.name.clone(), f.path.clone()))
        .collect();
    for hp in &form.hidden_folders {
        if !all_folder_paths.iter().any(|(_, p)| p == hp) {
            all_folder_paths.push((hp.clone(), hp.clone()));
        }
    }
    if all_folder_paths.is_empty() {
        hidden_rows.push(
            div()
                .w_full()
                .px_4()
                .py_3()
                .text_xs()
                .text_color(muted_fg)
                .child(
                    "完成首次同步后可在此勾选需要从侧边栏隐藏的远端文件夹（如 [Gmail]/Important）",
                )
                .into_any_element(),
        );
    } else {
        for (idx, (fname, fpath)) in all_folder_paths.into_iter().enumerate() {
            let is_hidden = form.hidden_folders.iter().any(|p| p == &fpath);
            let win_for_hf = cx.entity();
            let path_for_toggle = fpath.clone();
            hidden_rows.push(
                h_flex()
                    .w_full()
                    .min_h(px(46.0))
                    .px_4()
                    .py_1p5()
                    .gap_3()
                    .items_center()
                    .justify_between()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(text)
                                    .truncate()
                                    .child(SharedString::from(fname)),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted_fg)
                                    .truncate()
                                    .child(SharedString::from(fpath)),
                            ),
                    )
                    .child(
                        Switch::new(("acct-hidden-folder", idx))
                            .checked(is_hidden)
                            .disabled(demo_mode)
                            .on_click(move |checked, _, cx| {
                                let _ = win_for_hf.update(cx, |win, cx| {
                                    if *checked {
                                        if !win
                                            .account_form
                                            .hidden_folders
                                            .iter()
                                            .any(|p| p == &path_for_toggle)
                                        {
                                            win.account_form
                                                .hidden_folders
                                                .push(path_for_toggle.clone());
                                        }
                                    } else {
                                        win.account_form
                                            .hidden_folders
                                            .retain(|p| p != &path_for_toggle);
                                    }
                                    cx.notify();
                                });
                            }),
                    )
                    .into_any_element(),
            );
        }
    }
    let hidden_folders_group = setting_group(
        "隐藏文件夹",
        Some("开启后该远端文件夹将不在主窗口侧边栏显示"),
        CARD_MAX_WIDTH,
        hidden_rows,
        p,
    );

    if let Some(acc_id) = editing_id {
        let app_for_sync = app_entity.clone();
        sync_rows.push(
            h_flex()
                .w_full()
                .min_h(px(52.0))
                .px_4()
                .py_2()
                .gap_3()
                .items_center()
                .justify_between()
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_0p5()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(text)
                                .child("同步状态"),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted_fg)
                                .child(SharedString::from(sync_status_text)),
                        ),
                )
                .child(
                    Button::new("acct-sync-now")
                        .outline()
                        .compact()
                        .disabled(demo_mode)
                        .icon(IconName::RefreshCw)
                        .label("立即同步")
                        .on_click(cx.listener(move |_, _, _, cx| {
                            app_for_sync.update(cx, |app, cx| {
                                if let Some(ref w) = app.worker {
                                    app.syncing_accounts.insert(acc_id);
                                    app.account_sync_errors.remove(&acc_id);
                                    w.send(MailRequest::SyncAccount {
                                        account_id: acc_id,
                                        user_initiated: true,
                                    });
                                    cx.notify();
                                }
                            });
                        })),
                )
                .into_any_element(),
        );
    }

    let app_for_delete = app_entity.clone();

    v_flex()
        .id("accounts-editor-scroll")
        .flex_1()
        .min_h_0()
        .px_6()
        .py_5()
        .overflow_y_scrollbar()
        .child(
            v_flex()
                .w_full()
                .max_w(px(CARD_MAX_WIDTH))
                .mx_auto()
                .gap_5()
                .child(
                    v_flex()
                        .w_full()
                        .pt_1()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .child(render_brand_mark(top_brand, 56.0))
                        .when_some(existing_account.map(|a| a.display_label()), |el, lbl| {
                            el.child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(text)
                                    .child(SharedString::from(lbl)),
                            )
                        }),
                )
                .when(win.confirm_remove_in_editor, |el| {
                    if let Some(acc_id) = editing_id {
                        let app_del = app_for_delete.clone();
                        el.child(
                            h_flex()
                                .w_full()
                                .p_3p5()
                                .gap_3()
                                .items_center()
                                .justify_between()
                                .rounded(px(12.0))
                                .bg(danger.opacity(0.10))
                                .border_1()
                                .border_color(danger.opacity(0.5))
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .gap_0p5()
                                        .child(
                                            div()
                                                .text_sm()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(danger)
                                                .child("确定移除此邮箱账号？"),
                                        )
                                        .child(div().text_xs().text_color(muted_fg).child(
                                            "将删除本地邮件缓存、配置与凭据管理器中的密钥。",
                                        )),
                                )
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .flex_shrink_0()
                                        .child(
                                            Button::new("acct-remove-cancel")
                                                .outline()
                                                .compact()
                                                .label("取消")
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.confirm_remove_in_editor = false;
                                                    cx.notify();
                                                })),
                                        )
                                        .child(
                                            Button::new("acct-remove-confirm")
                                                .danger()
                                                .compact()
                                                .icon(IconName::Trash)
                                                .label("确认移除")
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.confirm_remove_in_editor = false;
                                                    app_del.update(cx, |app, cx| {
                                                        app.delete_account(acc_id, cx);
                                                    });
                                                })),
                                        ),
                                ),
                        )
                    } else {
                        el
                    }
                })
                .when_some(form.validation_error.as_ref(), |el, err| {
                    el.child(
                        div()
                            .px_3()
                            .py_2()
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(danger.opacity(0.5))
                            .bg(danger.opacity(0.10))
                            .text_xs()
                            .text_color(danger)
                            .child(err.clone()),
                    )
                })
                .when_some(
                    cfg_feedback
                        .as_ref()
                        .filter(|_| form.validation_error.is_none()),
                    |el, (ok, msg)| {
                        el.child(
                            div()
                                .px_3()
                                .py_2()
                                .rounded(px(8.0))
                                .border_1()
                                .border_color(if *ok {
                                    accent.opacity(0.45)
                                } else {
                                    danger.opacity(0.45)
                                })
                                .bg(if *ok {
                                    p.accent_soft()
                                } else {
                                    danger.opacity(0.10)
                                })
                                .text_xs()
                                .text_color(if *ok { accent } else { danger })
                                .child(msg.clone()),
                        )
                    },
                )
                .when_some(storage_group, |el, sg| el.child(sg))
                .child(setting_group(
                    "邮箱账号",
                    None,
                    CARD_MAX_WIDTH,
                    account_rows,
                    p,
                ))
                .child(setting_group(
                    "外观",
                    None,
                    CARD_MAX_WIDTH,
                    appearance_rows,
                    p,
                ))
                .child(aliases_group)
                .child(signature_group)
                .child(special_folders_group)
                .child(setting_group("同步", None, CARD_MAX_WIDTH, sync_rows, p))
                .child(hidden_folders_group)
                .child(div().px_1().pb_4().text_xs().text_color(muted_fg).child(
                    "密码与 OAuth 令牌安全保存在 Windows 凭据管理器中，本地配置仅保存非敏感设置。",
                )),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn account_providers_has_seventeen_unique_entries() {
        assert_eq!(ACCOUNT_PROVIDERS.len(), 17);
        let ids: HashSet<&str> = ACCOUNT_PROVIDERS.iter().map(|p| p.id).collect();
        assert_eq!(ids.len(), 17);
        assert_eq!(ACCOUNT_PROVIDERS[0].kind, ProviderKind::Manual);
        assert_eq!(ACCOUNT_PROVIDERS[1].kind, ProviderKind::Microsoft);
        assert_eq!(
            ACCOUNT_PROVIDERS
                .iter()
                .filter(|p| p.kind == ProviderKind::Preset)
                .count(),
            15
        );
        for domestic in [
            "qq",
            "netease163",
            "netease126",
            "exmail",
            "aliyun_qiye",
            "gmail",
        ] {
            assert!(ids.contains(domestic), "missing preset {domestic}");
        }
    }

    /// 签名预览：纯文本按 HTML 转义，脚本与事件属性被丢；纯文本模式不预览。
    #[test]
    fn signature_preview_sanitizes_source() {
        let preview = signature_preview_html("Ada & Co\nCEO", true);
        assert!(preview.contains("Ada &amp; Co<br>CEO"), "{preview}");

        let preview =
            signature_preview_html(r#"<p onclick="x()">Ada</p><script>alert(1)</script>"#, true);
        assert!(preview.contains("Ada"), "{preview}");
        assert!(!preview.contains("script"), "{preview}");
        assert!(!preview.contains("onclick"), "{preview}");

        assert_eq!(signature_preview_html("<p>x</p>", false), "");
    }

    #[test]
    fn brand_for_account_detects_oauth_hosts_and_domains() {
        let oauth = AccountConfig {
            auth_backend: AuthBackend::MicrosoftGraphOAuth2,
            email: "user@example.com".to_string(),
            ..Default::default()
        };
        assert_eq!(brand_for_account(&oauth), Some("outlook"));

        let icloud = AccountConfig {
            imap_host: "imap.mail.me.com".to_string(),
            email: "me@icloud.com".to_string(),
            ..Default::default()
        };
        assert_eq!(brand_for_account(&icloud), Some("icloud"));

        let gmail = AccountConfig {
            imap_host: "imap.gmail.com".to_string(),
            email: "person@gmail.com".to_string(),
            ..Default::default()
        };
        assert_eq!(brand_for_account(&gmail), Some("gmail"));

        let proton = AccountConfig {
            imap_host: "127.0.0.1".to_string(),
            imap_port: 1143,
            email: "me@proton.me".to_string(),
            ..Default::default()
        };
        assert_eq!(brand_for_account(&proton), Some("proton"));

        let unknown = AccountConfig {
            imap_host: "mail.corp.internal".to_string(),
            email: "dev@corp.internal".to_string(),
            ..Default::default()
        };
        assert_eq!(brand_for_account(&unknown), None);
    }

    #[test]
    fn provider_index_for_account_roundtrips_all_providers() {
        for (expected_idx, provider) in ACCOUNT_PROVIDERS.iter().enumerate() {
            let acc = match provider.kind {
                ProviderKind::Manual => AccountConfig {
                    imap_host: "mail.custom.org".to_string(),
                    ..Default::default()
                },
                ProviderKind::Microsoft => AccountConfig {
                    auth_backend: AuthBackend::MicrosoftGraphOAuth2,
                    ..Default::default()
                },
                ProviderKind::Preset => AccountConfig {
                    imap_host: provider.imap_host.to_string(),
                    imap_port: provider.imap_port,
                    ..Default::default()
                },
            };
            assert_eq!(
                provider_index_for_account(&acc),
                expected_idx,
                "provider {} should roundtrip",
                provider.id
            );
        }
    }

    #[test]
    fn removing_signature_clears_matching_default_and_shifts_later_indices() {
        let mut selected_removed = Some(1);
        adjust_default_sig_after_remove(&mut selected_removed, 1);
        assert_eq!(selected_removed, None);

        let mut later_sig = Some(2);
        adjust_default_sig_after_remove(&mut later_sig, 0);
        assert_eq!(later_sig, Some(1));

        let mut earlier_sig = Some(0);
        adjust_default_sig_after_remove(&mut earlier_sig, 2);
        assert_eq!(earlier_sig, Some(0));

        let mut none_sig = None;
        adjust_default_sig_after_remove(&mut none_sig, 0);
        assert_eq!(none_sig, None);
    }
}
