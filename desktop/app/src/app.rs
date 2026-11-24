//! HylkiApp 全局状态机（从单文件原型 `main.rs` 迁入）。
//!
//! 职责：视图路由（[`RightPaneMode`]）、账号 / 文件夹 / 邮件状态、
//! WorkerEvent 轮询分发、写信与账号配置的动作逻辑，以及主窗口三栏
//! 组装（左栏 / 中栏由 `ui::` 模块渲染）。
//!
//! 本轮（迭代 1）只做「纯重构 + 视觉升级」，所有既有行为不变：
//! --demo、多账号同步、OAuth、搜索、分页、草稿、Outbox 重试、附件保存、
//! 标记已读 / 星标、账号增删配置全部保留。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::resizable::{resize_handle, HandleEdge};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::text::{RangeHighlight, TextViewState};
use gpui_kit::component::{h_flex, v_flex, Disableable, Icon};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    actions, black, div, img, point, px, size, AnyWindowHandle, AppContext, Axis, Bounds,
    ClipboardItem, Context, DragMoveEvent, Entity, Focusable, Hsla, Image, InteractiveElement,
    IntoElement, MouseButton, ObjectFit, ParentElement, Render, ScrollHandle, SharedString, Styled,
    StyledImage, Subscription, TitlebarOptions, Window, WindowAppearance, WindowBounds,
    WindowOptions,
};
use hylki_mail_core::cache::{sanitize_filename, CachedAttachment, KnownContact};
use hylki_mail_core::config::{self, AccountConfig, AppPreferences, SettingsBackupBundle};
use hylki_mail_core::demo::{
    DemoStore, DEMO_ARCHIVE_ID, DEMO_DRAFTS_ID, DEMO_INBOX_ID, DEMO_SENT_ID,
};
use hylki_mail_core::markup::ComposeFormat;
use hylki_mail_core::worker::MailWorkerHandle;
use hylki_mail_core::{
    contacts_io, detect_attachment_reminder, format_reply_header, ComposeDraft, FolderSummary,
    LocalDraft, MailPage, MailRequest, MailTemplate, MessageDetail, MessageFilter, MessageSort,
    MessageSummary, OutboxItem, QuickFilterSet, SearchScope, UnifiedView, UserContact, WorkerEvent,
};

use crate::compose_schedule::{self, LaterPreset};
use crate::theme;
use crate::tray::{self, TrayCommand, TrayHandle};
use crate::ui::attachment_drawer::{
    self, AttachmentDrawerDrag, DRAWER_DEFAULT_HEIGHT, DRAWER_MAX_HEIGHT, DRAWER_MIN_HEIGHT,
};
use crate::ui::command_palette::{CommandPaletteEvent, CommandPaletteState, PaletteCommand};
use crate::ui::compose_window::ComposeWindow;
use crate::ui::contacts::{ContactExportKind, ContactsEvent, ContactsState};
use crate::ui::gallery::{GalleryEvent, GalleryState};
use crate::ui::message_window::{self, MessageWindow};
use crate::ui::preferences::{PrefTab, SettingsWindow};
use crate::ui::print_preview::{self, PrintPreviewState};
use crate::ui::{compose, message_list, reader, sidebar, welcome, window_controls};

pub(crate) const UNIFIED_INBOX_FOLDER_ID: u32 = 0;

// ---- L-2：三栏宽度常量（对齐原版 app.rs L270-L341 与 list_header.rs）----

/// 左栏最小宽度（原版 OverlaySplitView `min_sidebar_width`）。
pub(crate) const SIDEBAR_MIN_WIDTH: f32 = 180.0;
/// 左栏最大宽度（原版 `max_sidebar_width`）。
pub(crate) const SIDEBAR_MAX_WIDTH: f32 = 280.0;
/// 中栏列表最小宽度（列表自身的下限）。
pub(crate) const LIST_MIN_WIDTH: f32 = 280.0;
/// 右栏（阅读器）最小宽度（原版 READER_MIN_WIDTH = 441：顶栏整排动作不被挤出窗口）。
pub(crate) const READER_MIN_WIDTH: f32 = 441.0;
/// 中栏 ≥ 该宽度自动切换单行表格模式（原版 LIST_WIDE_ON，含迟滞）。
pub(crate) const LIST_WIDE_ON: f32 = 600.0;
/// 单行表格模式退回卡片模式的迟滞下限（原版 LIST_WIDE_OFF，两侧留余量防抖动）。
pub(crate) const LIST_WIDE_OFF: f32 = 560.0;
/// 阅读器在下方时的最小高度。
pub(crate) const BELOW_PANE_MIN_HEIGHT: f32 = 200.0;
/// 下方模式列表最小高度。
pub(crate) const BELOW_LIST_MIN_HEIGHT: f32 = 240.0;
/// 下方模式列表默认高度（无持久化值时）。
pub(crate) const BELOW_LIST_DEFAULT_HEIGHT: f32 = 360.0;
/// WP-10-4：内联分屏写信面板的默认高度（贴顶）与最小高度。
pub(crate) const COMPOSE_SPLIT_DEFAULT_HEIGHT: f32 = 440.0;
pub(crate) const COMPOSE_SPLIT_MIN_HEIGHT: f32 = 220.0;
/// 内联分屏里下方阅读器的最小高度（再高就把写信面板压得只剩标题栏了）。
pub(crate) const COMPOSE_SPLIT_READER_MIN_HEIGHT: f32 = 160.0;
/// B-5：窗口宽度低于该值时阅读器顶栏低频动作收进「⋯」菜单（≈ 左栏最小 + 列表最小
/// + 阅读器最小 441 的下限组合，此时右栏已被压至最小宽度附近，平铺按钮放不下）。
pub(crate) const READER_OVERFLOW_BREAKPOINT: f32 = 880.0;
/// L-2：宽度拖拽停止后的落盘防抖时长（对齐原版 mail_split.rs SAVE_AFTER = 600ms）。
const LAYOUT_SAVE_DEBOUNCE: Duration = Duration::from_millis(600);
/// P3：半屏自动窄轨断点（对齐 Linux `adw::Breakpoint` MaxWidth 1094px，
/// src/app/window.rs）：窗口宽度 ≤ 该值（gpui 逻辑像素，与 DPI 缩放无关）时
/// 自动进入 80px 图标窄轨；扩回 > 该值时交还用户的手动折叠偏好。
pub(crate) const AUTO_RAIL_BREAKPOINT: f32 = 1094.0;

/// L-2：单行表格模式迟滞判定（对齐原版 list_header.rs：进入 ≥600、退出 <560，
/// 两侧留余量防拖拽在阈值附近来回抖动）。
fn list_wide_next(current_wide: bool, width: f32) -> bool {
    if current_wide {
        width >= LIST_WIDE_OFF
    } else {
        width >= LIST_WIDE_ON
    }
}

/// P3：半屏自动窄轨判定（对齐 Linux libadwaita MaxWidth 断点语义：宽度 ≤ 断点
/// 即进入窄轨，无迟滞；纯函数便于单测）。
fn auto_rail_next(width: f32) -> bool {
    width <= AUTO_RAIL_BREAKPOINT
}

/// P3：窄窗口浮层侧栏宽度（对齐 Linux peek 面板：沿用手动宽度偏好并夹取，
/// 上限再夹到窄轨右侧剩余空间，防止极窄窗口下面板溢出屏幕）。
fn peek_sidebar_width(pref_width: f32, viewport_w: f32) -> f32 {
    pref_width
        .clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH)
        .min((viewport_w - sidebar::RAIL_WIDTH).max(0.0))
}

fn is_selected_message_ready_for_popout(
    selected_id: Option<u32>,
    target_id: u32,
    pending_body_id: Option<u32>,
    is_placeholder: bool,
) -> bool {
    !is_placeholder && pending_body_id != Some(target_id) && selected_id == Some(target_id)
}

fn should_update_main_reader_on_body_loaded(
    detail_id: u32,
    pending_body_id: Option<u32>,
    popped_out: bool,
    selected_id: Option<u32>,
) -> bool {
    match pending_body_id {
        Some(id) => id == detail_id,
        None => !popped_out || selected_id == Some(detail_id),
    }
}

fn popout_cascade_offset(open_count: usize) -> f32 {
    (open_count % 8) as f32 * 28.0
}

fn compose_session_active(right_mode: RightPaneMode, compose_detached: bool) -> bool {
    right_mode == RightPaneMode::Compose || compose_detached
}

fn should_use_compose_split(split: bool, reply_inline: bool, has_selected_message: bool) -> bool {
    split && reply_inline && has_selected_message
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PopoutReplyKind {
    Reply,
    ReplyAll,
    Forward,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RightPaneMode {
    Reader,
    Compose,
    /// 首次启动欢迎向导（无账号且非 demo 时默认进入，方案文档五.1）。
    Welcome,
}

/// 左栏统一导航的当前视图（方案文档五.1）。
///
/// `AccountFolder` 表示正在浏览某个账号的具体文件夹（此时 `selected_folder_id`
/// 才参与列表请求）；其余视图为跨账号统一视图。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CurrentView {
    UnifiedInbox,
    Unread,
    Starred,
    Sent,
    Drafts,
    Archive,
    Junk,
    Trash,
    Snoozed,
    AccountFolder,
}

impl CurrentView {
    /// 对应的 mail-core 统一视图查询；`UnifiedInbox`/`Drafts` 走独立链路。
    pub(crate) fn unified(self) -> Option<UnifiedView> {
        match self {
            Self::Unread => Some(UnifiedView::Unread),
            Self::Starred => Some(UnifiedView::Starred),
            Self::Sent => Some(UnifiedView::Sent),
            Self::Archive => Some(UnifiedView::Archive),
            Self::Junk => Some(UnifiedView::Junk),
            Self::Trash => Some(UnifiedView::Trash),
            Self::Snoozed => Some(UnifiedView::Snoozed),
            Self::UnifiedInbox | Self::Drafts | Self::AccountFolder => None,
        }
    }

    /// 是否为跨账号统一视图（中栏多账号标签胶囊的显示条件）。
    pub(crate) fn is_unified(self) -> bool {
        !matches!(self, Self::AccountFolder)
    }
}

/// WP-07 侧边栏文件夹右键操作弹窗状态（新建子文件夹 / 重命名 / 删除确认）。
#[derive(Clone, Debug)]
pub(crate) enum FolderActionPrompt {
    CreateSubfolder {
        account_id: u32,
        parent_path: String,
        delimiter: char,
    },
    RenameFolder {
        account_id: u32,
        folder_id: u32,
        old_path: String,
        current_name: String,
    },
    DeleteFolder {
        account_id: u32,
        folder_id: u32,
        folder_path: String,
        folder_name: String,
    },
}

/// L-6：阅读器位置（对齐原版 config::ReadingPane #475）：列表右侧 / 列表下方 / 关闭。
///
/// 关闭时列表占满内容区，点击邮件仅选中并在状态栏提示（原版「不展示、不拉取正文、
/// 不标已读」中的前两条因写信 / 复制等动作依赖正文而放宽为仍拉取，但保持不标已读）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ReaderPosition {
    #[default]
    Right,
    Below,
    Hidden,
}

impl ReaderPosition {
    /// 持久化到 preferences.toml 的字符串键（对齐原版 ReadingPane::key）。
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Right => "right",
            Self::Below => "below",
            Self::Hidden => "off",
        }
    }

    /// 从偏好字符串解析（未知值回落 Right，兼容手改配置文件）。
    pub(crate) fn from_key(key: &str) -> Option<Self> {
        match key {
            "right" => Some(Self::Right),
            "below" => Some(Self::Below),
            "off" => Some(Self::Hidden),
            _ => None,
        }
    }
}

// L-2：三栏拖拽把手载荷。gpui 按载荷类型区分拖拽事件；渲染为空白 div（无拖拽预览）。
/// 左栏右缘把手载荷。
pub(crate) struct SidebarDrag;
/// 列表与右栏之间的竖直分隔把手载荷（Right 模式）。
pub(crate) struct ListDrag;
/// 列表与下方阅读器之间的水平分隔把手载荷（Below 模式）。
pub(crate) struct BelowDrag;
/// WP-10-4：内联分屏中「阅读器 | 写信器」之间的水平分隔把手载荷
/// （对齐原版 `.split-grab-pill`）。
pub(crate) struct ComposeSplitDrag;

/// WP-08：宽表列头分隔线拖拽载荷。
#[derive(Clone, Copy)]
pub(crate) struct ColumnResizeDrag {
    pub(crate) column: hylki_mail_core::config::ListColumn,
    pub(crate) start_x: f32,
    pub(crate) start_width: f32,
}

impl Render for ColumnResizeDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

impl Render for SidebarDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

impl Render for ListDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

impl Render for BelowDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

impl Render for ComposeSplitDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

// 全局快捷键动作（方案文档五.3）；键绑定注册在 main.rs，
// 动作监听挂在根节点 render 上。
//
// F-1 阶段三：`Reply*` / `ViewMessageSource` 为 Ctrl 组合键动作（不受单键偏好
// 控制）；`SingleKey*` 前缀为单键动作，on_action 守卫里统一检查
// `prefs.single_key_shortcuts`（偏好设置「常规」页可关闭）。
actions!(
    hylki,
    [
        NewCompose,
        OpenCommandPalette,
        OpenCommandPaletteActions,
        FocusSearch,
        RefreshAllAccounts,
        CloseOverlay,
        SendCompose,
        OpenPreferences,
        QuitApp,
        DeleteSelectedMessage,
        SelectNextMessage,
        SelectPrevMessage,
        UndoLastMove,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        ToggleFullscreen,
        ToggleAccountsList,
        OpenShortcutsWindow,
        // ---- F-1 阶段三：组合键（始终生效）----
        ReplyCurrent,
        ReplyAllCurrent,
        ViewMessageSource,
        // ---- F-1 阶段三：单键（受 single_key_shortcuts 偏好控制）----
        SingleKeyNextMessage,
        SingleKeyPrevMessage,
        SingleKeyReply,
        SingleKeyReplyAll,
        SingleKeyForward,
        SingleKeyArchive,
        SingleKeyDelete,
        SingleKeyMarkSpam,
        SingleKeyToggleStar,
        SingleKeyToggleRead,
        SingleKeyCompose,
        SingleKeyBackToList,
        SingleKeyNextCard,
        SingleKeyPrevCard,
        // L-6：阅读器位置（Ctrl+Alt+1/2/3）与专注模式（Ctrl+Shift+F）。
        SetReaderPositionRight,
        SetReaderPositionBelow,
        SetReaderPositionHidden,
        ToggleFocusMode,
        CloseSettingsWindow
    ]
);

/// 写信页当前激活联系人自动补全的地址输入行。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ComposeAddressField {
    To,
    Cc,
    Bcc,
}

impl ComposeAddressField {
    /// 胶囊「移动到下一个字段」的顺序：To -> Cc -> Bcc -> To
    /// （对齐 Linux `recipient_row` 的胶囊菜单行为）。
    pub(crate) fn next(self) -> Self {
        match self {
            Self::To => Self::Cc,
            Self::Cc => Self::Bcc,
            Self::Bcc => Self::To,
        }
    }

    /// 稳定的数字下标（用于生成唯一 `ElementId`）。
    pub(crate) fn index(self) -> usize {
        match self {
            Self::To => 0,
            Self::Cc => 1,
            Self::Bcc => 2,
        }
    }

    /// 缺失标签文案（无胶囊时显示在输入框前的占位提示）。
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::To => "收件人",
            Self::Cc => "抄送",
            Self::Bcc => "密送",
        }
    }
}

/// WP-10-3 富文本模式的格式化工具条按钮。
///
/// Windows 版正文是一个多行文本框，没有原版 GTK 富文本编辑器的选区命令，
/// 所以这些按钮做的是「往正文里插入对应语法」——写的是 Markdown 标记，
/// 切换格式 / 发信时按语法转换成 HTML。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ComposeMarkup {
    Bold,
    Italic,
    Strike,
    InlineCode,
    BulletList,
    NumberedList,
    Quote,
    CodeBlock,
    Link,
}

impl ComposeMarkup {
    /// 工具条从左到右的排列顺序。
    pub(crate) const ALL: [ComposeMarkup; 9] = [
        ComposeMarkup::Bold,
        ComposeMarkup::Italic,
        ComposeMarkup::Strike,
        ComposeMarkup::InlineCode,
        ComposeMarkup::BulletList,
        ComposeMarkup::NumberedList,
        ComposeMarkup::Quote,
        ComposeMarkup::CodeBlock,
        ComposeMarkup::Link,
    ];

    /// 按钮气泡提示。
    pub(crate) fn hint(self) -> &'static str {
        match self {
            Self::Bold => "加粗 (**文字**)",
            Self::Italic => "斜体 (*文字*)",
            Self::Strike => "删除线 (~~文字~~)",
            Self::InlineCode => "行内代码 (`代码`)",
            Self::BulletList => "无序列表 (- 条目)",
            Self::NumberedList => "有序列表 (1. 条目)",
            Self::Quote => "引用 (> 引用)",
            Self::CodeBlock => "代码块 (```)",
            Self::Link => "插入链接 ([文字](https://))",
        }
    }

    /// 生成要写进正文的片段：有选区时包裹选区，否则用占位文案。
    pub(crate) fn wrap(self, selected: &str) -> String {
        let text = if selected.is_empty() {
            match self {
                Self::Bold => "粗体",
                Self::Italic => "斜体",
                Self::Strike => "删除线",
                Self::InlineCode => "代码",
                Self::BulletList => "列表项",
                Self::NumberedList => "列表项",
                Self::Quote => "引用内容",
                Self::CodeBlock => "",
                Self::Link => "链接文字",
            }
        } else {
            selected
        };
        match self {
            Self::Bold => format!("**{text}**"),
            Self::Italic => format!("*{text}*"),
            Self::Strike => format!("~~{text}~~"),
            Self::InlineCode => format!("`{text}`"),
            Self::BulletList => prefix_lines(text, "- "),
            Self::NumberedList => number_lines(text),
            Self::Quote => prefix_lines(text, "> "),
            Self::CodeBlock => format!("```\n{text}\n```"),
            Self::Link => format!("[{text}](https://)"),
        }
    }
}

/// 给每一行加同样的前缀（空行不加），用于列表与引用。
fn prefix_lines(text: &str, prefix: &str) -> String {
    text.split('\n')
        .map(|line| {
            if line.trim().is_empty() {
                line.to_string()
            } else {
                format!("{prefix}{line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 给每一行加递增序号，用于有序列表。
fn number_lines(text: &str) -> String {
    let mut n = 0usize;
    text.split('\n')
        .map(|line| {
            if line.trim().is_empty() {
                line.to_string()
            } else {
                n += 1;
                format!("{n}. {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// WP-10-2 写信 From 下拉的一个候选身份（账号默认身份或某个发信别名）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ComposeFromChoice {
    /// 下拉项与 From 行显示的文本（`Name <addr>` 或纯地址）。
    pub(crate) label: String,
    /// 写入 `ComposeDraft::from_addr` 的值；空 = 账号默认身份。
    pub(crate) from_addr: String,
    /// 别名自带的 Reply-To（选中且用户未填 Reply-To 时预填）。
    pub(crate) reply_to: String,
}

/// 把显示名与地址拼成 `Name <addr>`；缺少显示名时只留地址。
pub(crate) fn format_identity(name: &str, email: &str) -> String {
    let name = name.trim();
    let email = email.trim();
    if email.is_empty() {
        return name.to_string();
    }
    if name.is_empty() {
        email.to_string()
    } else {
        format!("{name} <{email}>")
    }
}

/// 写信收件人胶囊（对齐 Linux `src/ui/recipient_row.rs` 的 `.recipient-pill`）：
/// 一个已确认的收件地址。输入框里按回车 / 逗号 / 空格 / 失焦后转成胶囊。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RecipientPill {
    /// 胶囊显示文本：有显示名时用显示名，否则用地址本身。
    pub(crate) label: String,
    /// 实际收件地址（发信与草稿落盘用它，`Name <addr>` 取尖括号内部分）。
    pub(crate) address: String,
    /// 地址格式是否合法；`false` 时渲染红色 `.invalid` 边框。
    pub(crate) valid: bool,
}

impl RecipientPill {
    /// 解析一段收件人文本；空白段返回 `None`。
    ///
    /// 支持 `addr`、`Name <addr>`、以及只写了显示名的半成品（后者
    /// `valid = false`，渲染为红胶囊提示用户补全地址）。
    pub(crate) fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        let (name, email) = split_sender(raw);
        // `split_sender` 对不含尖括号的输入把整段当成显示名返回；
        // 这类输入既可能是裸地址，也可能是纯显示名，统一当地址候选校验。
        let address = if email.is_empty() {
            name.clone()
        } else {
            email
        };
        let valid = looks_like_address(&address);
        let label = if name.is_empty() {
            address.clone()
        } else {
            name
        };
        Some(Self {
            label,
            address,
            valid,
        })
    }

    /// 送入发信 / 草稿的文本：有显示名时保留 `Name <addr>` 形式。
    pub(crate) fn to_field_text(&self) -> String {
        if self.label == self.address {
            self.address.clone()
        } else {
            format!("{} <{}>", self.label, self.address)
        }
    }
}

/// 轻量邮箱格式校验（本地段 + `@` + 含点的域名），用于胶囊 `.invalid` 标红。
pub(crate) fn looks_like_address(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() || s.chars().any(|c| c.is_whitespace()) {
        return false;
    }
    let Some((local, domain)) = s.rsplit_once('@') else {
        return false;
    };
    !local.is_empty()
        && !local.contains('@')
        && !domain.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
}

/// 按逗号 / 分号（含全角）切分收件人串，忽略 `<>` 内的分隔符
/// （显示名里可能含逗号，如 `Doe, John <j@ex.com>`）。
pub(crate) fn split_recipient_parts(text: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut buf = String::new();
    let mut depth = 0usize;
    for ch in text.chars() {
        match ch {
            '<' => {
                depth += 1;
                buf.push(ch);
            }
            '>' => {
                depth = depth.saturating_sub(1);
                buf.push(ch);
            }
            ',' | ';' | '，' | '；' | '\n' if depth == 0 => {
                parts.push(std::mem::take(&mut buf));
            }
            _ => buf.push(ch),
        }
    }
    parts.push(buf);
    parts
}

/// 撤销发送（Undo Send）缓冲中的待投递邮件。
/// 记录先持久化到发件箱（Queued + 到期时间），撤销 = 删除该记录，到期 / 立即发送 = 触发投递。
#[derive(Clone, Debug)]
pub(crate) struct PendingSend {
    pub(crate) token: u64,
    pub(crate) account_id: u32,
    pub(crate) draft: ComposeDraft,
    pub(crate) local_draft_id: Option<u32>,
    pub(crate) remaining_secs: u8,
    /// 发件箱记录 ID（worker 落盘回执 `OutboxEnqueued` 到达后回填）。
    pub(crate) outbox_id: Option<u32>,
    /// 用户已点击撤销但落盘回执未到：回执到达后删除记录并清理缓冲。
    pub(crate) undo_requested: bool,
    /// 已触发投递但落盘回执未到：回执到达后立即投递。
    pub(crate) commit_requested: bool,
    enqueue_failed: bool,
}

/// 主内容区路由（方案文档五.1）：`Mail` 默认三栏，`Gallery` 附件总览，
/// `Contacts` 联系人（后两者保留左栏，中栏 + 右栏整体替换）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MainContentView {
    Mail,
    Gallery,
    Contacts,
}

#[derive(Clone)]
enum SettingsOpenTarget {
    Tab(PrefTab),
    EditAccount(Option<AccountConfig>),
    PresetAccount(usize),
}

/// 检查更新流程状态（关于浮层里展示；启动检查只写状态栏）。
#[derive(Default)]
pub(crate) struct UpdateState {
    pub(crate) checking: bool,
    pub(crate) downloading: bool,
    pending_install: Option<(hylki_mail_core::update::UpdateInfo, Vec<u8>)>,
    pub(crate) info: Option<hylki_mail_core::update::UpdateInfo>,
    pub(crate) error: Option<SharedString>,
    /// 最近一次检查的结论（已是最新 / 正在检查）。
    pub(crate) status: Option<SharedString>,
}

impl UpdateState {
    fn cancel_install(&mut self) {
        if self.pending_install.take().is_some() {
            self.downloading = false;
            self.status = None;
        }
    }
}

/// 构建版本：发布流水线注入 `HYLKI_VERSION`（标签去掉 v），本地构建回落到 Cargo 版本。
pub(crate) fn app_version() -> &'static str {
    option_env!("HYLKI_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
}

/// 当前全局浮层（命令面板、快捷键、关于）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ActiveModal {
    None,
    CommandPalette,
    /// 键盘快捷键速查（F1 / ?，对齐原版 win.shortcuts）。
    Shortcuts,
    /// 关于 Hylki（对齐原版 win.about）。
    About,
}

/// F-3：最近一次可撤销的移动 / 删除 / 归档记录（状态栏轻量撤销条，15 秒后自动消失）。
#[derive(Clone, Debug)]
pub(crate) struct UndoRecord {
    pub(crate) account_id: u32,
    pub(crate) message_id: u32,
    /// RFC Message-ID（撤销时用于在服务端当前文件夹定位邮件）。
    pub(crate) rfc_message_id: String,
    /// 撤销时邮件所在的文件夹（回收站 / 归档 / 垃圾 / 移动目标）。
    pub(crate) current_folder_path: String,
    /// 移动 / 删除 / 归档前邮件所在的文件夹路径（撤销时的恢复目标）。
    pub(crate) original_folder_path: String,
    /// 动作描述（如「已删除」「已归档」「已移动」）。
    pub(crate) action_label: SharedString,
    /// 记录时刻（用于 15 秒过期与定时清理比对）。
    pub(crate) at: Instant,
}

/// F-3：请求发出时捕获的撤销前半段信息（成功回执到达后补全为 [`UndoRecord`]）。
#[derive(Clone, Debug)]
struct PendingUndo {
    account_id: u32,
    message_id: u32,
    original_folder_path: String,
    action_label: SharedString,
    at: Instant,
}

impl PendingUndo {
    /// 用成功回执携带的服务端定位信息补全撤销记录。
    fn into_record(self, dest_folder_path: String, rfc_message_id: String) -> UndoRecord {
        UndoRecord {
            account_id: self.account_id,
            message_id: self.message_id,
            rfc_message_id,
            current_folder_path: dest_folder_path,
            original_folder_path: self.original_folder_path,
            action_label: self.action_label,
            at: self.at,
        }
    }
}

/// 撤销条过期时长（秒）。
const UNDO_TTL_SECS: u64 = 15;

/// B-2：列表行菜单触发的「等正文加载完成后执行」的后续动作。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PendingRowAction {
    Reply,
    ReplyAll,
    Forward,
    EditAsNew,
    CopyBody,
    ViewSource,
}

pub(crate) struct HylkiApp {
    pub(crate) demo_mode: bool,
    pub(crate) demo_store: Arc<DemoStore>,
    pub(crate) worker: Option<Arc<MailWorkerHandle>>,
    pub(crate) accounts: Vec<AccountConfig>,
    pub(crate) folders_by_account: BTreeMap<u32, Vec<FolderSummary>>,
    /// `0` means Unified Inbox across all accounts.
    pub(crate) selected_folder_id: u32,
    /// 左栏统一导航当前视图（决定列表请求走统一视图还是具体文件夹）。
    pub(crate) current_view: CurrentView,
    /// 当前列表已展开的总条数上限（初始为偏好 `page_size`，滚到底部每次递增一批）。
    pub(crate) loaded_limit: usize,
    /// 中栏邮件列表滚动句柄（用于检测滚动到底部自动预加载下一批）。
    pub(crate) list_scroll_handle: ScrollHandle,
    /// 正在从本地缓存增量追加下一批邮件（防止重复触发）。
    pub(crate) loading_more_local: bool,
    /// 正在从远端服务器（IMAP / Graph）预加载更早历史邮件（防止重复触发）。
    pub(crate) loading_more_remote: bool,
    /// 任务四：后台历史索引驱动状态（最多一个在途批次，事件驱动排队）。
    pub(crate) backfill: BackfillDriveState,
    pub(crate) current_page: MailPage,
    pub(crate) selected_message: Option<MessageDetail>,
    /// `selected_message` 是否仅是 `show_message_loading` 写入的空占位详情。
    pub(crate) selected_message_is_placeholder: bool,
    /// 正在等待远端正文的邮件 ID：用于立即占位显示并丢弃过期正文回包。
    pub(crate) pending_body_message_id: Option<u32>,
    /// Ctrl+点击多选的邮件 ID 集合。
    pub(crate) selected_message_ids: HashSet<u32>,
    pub(crate) drafts: Vec<LocalDraft>,
    /// WP-10-5 写信模板（`Save as Template` 保存的整套草稿快照）。
    pub(crate) templates: Vec<MailTemplate>,
    pub(crate) outbox_by_account: BTreeMap<u32, Vec<OutboxItem>>,
    pub(crate) right_mode: RightPaneMode,
    /// WP-10：首次启动欢迎向导当前步骤（0..=4：Welcome -> Theme & Layout -> Add Account -> Sync & AI -> Ready）。
    pub(crate) welcome_step: usize,
    pub(crate) search_input: Entity<InputState>,
    pub(crate) search_query: String,

    // Compose / Reply fields
    pub(crate) compose_account_id: u32,
    pub(crate) compose_draft_id: Option<u32>,
    pub(crate) compose_to: Entity<InputState>,
    pub(crate) compose_cc: Entity<InputState>,
    pub(crate) compose_bcc: Entity<InputState>,
    /// Cc / Bcc 输入行是否展开（默认折叠，方案文档页面 3）。
    pub(crate) compose_show_cc_bcc: bool,
    /// WP-10-2 自定义发件人（`Name <addr>` 或纯地址）；空 = 沿用所选账号默认身份。
    pub(crate) compose_from_addr: String,
    /// WP-10-2 是否处于「自定义 From」手输态（对应原版 `custom_from_row`）。
    pub(crate) compose_from_custom: bool,
    /// WP-10-2 自定义 From 的手输框。
    pub(crate) compose_from: Entity<InputState>,
    /// WP-10-2 `Reply-To` 行（空 = 不写该信头）。
    pub(crate) compose_reply_to: Entity<InputState>,
    pub(crate) compose_subject: Entity<InputState>,
    pub(crate) compose_body: Entity<TextareaState>,
    /// WP-10-3 正文格式（富文本 / Markdown / HTML / 纯文本）。
    ///
    /// 与原版一致，切换格式会把正文转换成目标表示；当前格式随草稿一起
    /// 落盘（`body_format`），重新打开草稿时按原格式恢复。
    pub(crate) compose_format: ComposeFormat,
    pub(crate) compose_preview: bool,
    /// WP-10-4 写信是否以「阅读器在上、写信器在下」的内联分屏呈现
    /// （对齐原版 `compose_inline` / `.reader-split`）。仅回复 / 转发且
    /// 阅读器里有可对照的邮件时成立。
    pub(crate) compose_split: bool,
    /// 内联分屏中写信面板的高度（贴底，px）；拖拽中间手柄时更新。
    pub(crate) compose_split_height: f32,
    /// WP-10-4 写信任意弹出为独立窗口：主窗口回到阅读器，不再重复画写信页。
    pub(crate) compose_detached: bool,
    /// 独立写信窗口的句柄（`None` = 未弹出或在别处已被关掉）。
    pub(crate) compose_window: Option<AnyWindowHandle>,
    /// WP-10-5 定时发送：自定义时间输入行是否展开（预设时间直接从下拉里选）。
    pub(crate) compose_schedule_open: bool,
    /// WP-10-5 自定义定时时间手输框（`YYYY-MM-DD HH:MM`）。
    pub(crate) compose_schedule_input: Entity<InputState>,
    /// WP-10-5 自定义定时时间的校验提示。
    pub(crate) compose_schedule_error: Option<SharedString>,
    /// WP-10-5 保存模板时的名称输入行是否展开。
    pub(crate) compose_template_save_open: bool,
    /// WP-10-5 保存模板的名称输入框。
    pub(crate) compose_template_name: Entity<InputState>,
    pub(crate) compose_in_reply_to: String,
    pub(crate) compose_attachments: Vec<String>,
    /// 写信退出前未保存确认条显示状态。
    pub(crate) compose_confirm_discard: bool,
    /// 当前正在等待导出原附件的转发邮件 ID。
    pub(crate) pending_forward_msg_id: Option<u32>,
    /// 当前触发联系人自动补全的地址框。
    pub(crate) compose_autocomplete_field: Option<ComposeAddressField>,
    /// WP-10-1 收件人胶囊：To / Cc / Bcc 各自已确认的地址列表。
    pub(crate) compose_to_pills: Vec<RecipientPill>,
    pub(crate) compose_cc_pills: Vec<RecipientPill>,
    pub(crate) compose_bcc_pills: Vec<RecipientPill>,
    /// WP-10-1 最近一次点击编辑的胶囊下标（再次提交时尽量放回原位置）。
    compose_pill_resume: Option<(ComposeAddressField, usize)>,
    /// F-4 草稿自动保存：防抖代际（每次输入变更 +1，旧定时器醒来后失效）。
    compose_autosave_generation: u64,
    compose_session_generation: u64,
    file_dialog_open: bool,
    print_preview_generation: u64,
    /// F-4 自上次落库以来写信内容有变更（变更事件仅由真实编辑触发）。
    compose_autosave_dirty: bool,
    /// F-4 自动保存请求已发出（`DraftSaved` 回执到达后显示提示并绑定草稿 ID）。
    compose_autosave_pending: bool,
    /// F-4 当前草稿是否由自动保存静默创建（「放弃」时连同该草稿删除）。
    compose_autosaved_new: bool,
    /// F-4 最近一次自动保存时刻（写信页常驻小字「草稿已自动保存 HH:MM」）。
    pub(crate) compose_autosaved_at: Option<SharedString>,
    /// 已知联系人本地缓存（供写信收件人自动补全）。
    pub(crate) known_contacts: Vec<KnownContact>,
    /// WP-07 侧边栏 Tags 分区展示的自定义标签列表（从 SQLite `tags` 表加载）。
    pub(crate) available_tags: Vec<hylki_mail_core::cache::TagItem>,
    /// WP-07 侧边栏文件夹右键「新建子文件夹 / 重命名 / 删除」内联确认条。
    pub(crate) folder_action: Option<FolderActionPrompt>,
    pub(crate) folder_action_input: Entity<InputState>,
    /// 撤销发送（5 秒缓冲）状态。
    pub(crate) pending_send: Option<PendingSend>,
    quitting: bool,
    quit_preparing: bool,
    next_send_token: u64,
    /// 写信发送前检测到正文/主题提及附件关键词但未附带文件时触发的内联确认横幅（命中关键词）。
    pub(crate) compose_attachment_reminder: Option<String>,

    // Account management banner state
    pub(crate) cfg_feedback: Option<(bool, SharedString)>,

    pub(crate) status_line: SharedString,
    pub(crate) error_banner: Option<SharedString>,
    /// 更新检查 / 下载 / 安装状态。
    pub(crate) update: UpdateState,

    // 迭代 1 新增：纯视觉状态（不影响后端行为）
    /// 当前主题（`theme::THEME_LIST` 中的 id）。
    pub(crate) theme_id: &'static str,
    /// 亮 / 暗模式（菜单「切换亮/暗主题」即时生效）。
    pub(crate) dark_mode: bool,
    /// 系统当前的深浅色外观（由 `window.appearance()` 检测，供 `dark_mode: None` 跟随系统使用）。
    pub(crate) system_dark_mode: bool,
    /// 左栏是否隐藏（中栏顶部开关）。
    pub(crate) sidebar_hidden: bool,
    /// P3：断点驱动的自动窄轨（窗口 ≤ 1094px 时为 true）。不落盘、不覆盖
    /// `sidebar_hidden` 手动偏好（对齐 Linux AutoRail：窄轨自动接管，扩回
    /// 宽窗口时手动偏好原样恢复）。
    pub(crate) auto_rail_active: bool,
    /// P3：窄窗口浮层侧栏是否展开（完整侧栏以浮层盖在中栏之上；导航选择后
    /// 自动关闭，对齐 Linux sidebar_peek）。
    pub(crate) sidebar_peek_open: bool,
    /// P3：列表焦点近似态（Linux focus-within）：最近一次列表交互（点击行 /
    /// 键盘选择）后为 true，点击阅读器后为 false；渲染时再结合文本输入焦点
    /// 判定，驱动选中行药丸在 0.5 / 0.3 两档间切换。
    pub(crate) list_focused: bool,
    /// P3：键盘游标所在邮件（j/k、方向键移动，点击行后清除；对齐 Linux
    /// .kbd-cursor：列表持焦点时非选中行画 accent 轮廓，选中行轮廓改 accent_fg）。
    pub(crate) kbd_cursor_message_id: Option<u32>,
    /// 中栏搜索栏是否展开（对齐原版：顶栏常驻 🔍 图标，点按或 Ctrl+F 展开）。
    pub(crate) search_open: bool,
    /// 列表排序方式（6 种，默认最新优先）。
    pub(crate) list_sort: MessageSort,
    /// 多条件快捷筛选（仅未读 / 仅星标 / 含附件 / 今天）。
    pub(crate) quick_filters: QuickFilterSet,
    /// 本地搜索范围（当前视图 / 全部文件夹）。
    pub(crate) search_scope: SearchScope,
    /// 服务端在线搜索状态与远端命中缓存。
    pub(crate) server_search_loading: bool,
    pub(crate) server_search_query: String,
    pub(crate) server_search_results: Vec<MessageSummary>,
    /// 账号分组折叠状态（缺省展开）。
    pub(crate) collapsed_accounts: HashSet<u32>,
    /// 是否在左栏显示账号列表（对齐原版 show-accounts 偏好，Ctrl+Shift+A / 主菜单切换）。
    pub(crate) show_accounts: bool,
    /// 待二次确认的「清空文件夹」目标 `(account_id, folder_id, folder_path, 显示名)`。
    pub(crate) pending_empty_folder: Option<(u32, u32, String, String)>,
    /// 阅读器是否强制以纯文本渲染（false = 有 HTML 时渲染富文本）。
    pub(crate) reader_prefer_plain: bool,
    /// 本次会话中用户手动放行远程图片的邮件 ID 集合。
    pub(crate) allowed_remote_image_ids: HashSet<u32>,
    /// 当前会话阅读器中处于折叠状态的卡片邮件 ID 集合。
    pub(crate) collapsed_thread_cards: HashSet<u32>,
    /// 当前会话阅读器中处于「查看邮件原文」模式的卡片邮件 ID 集合。
    pub(crate) raw_source_message_ids: HashSet<u32>,
    /// WP-09：阅读器中已展开历史引用块的邮件 ID 集合（默认折叠）。
    pub(crate) expanded_quote_message_ids: HashSet<u32>,
    /// WP-09：底部附件抽屉折叠状态、高度、列表/网格模式与排序方向。
    pub(crate) attachment_drawer_collapsed: bool,
    pub(crate) attachment_drawer_height: f32,
    pub(crate) attachment_drawer_list_view: bool,
    pub(crate) attachment_drawer_sort_desc: bool,
    /// WP-09：附件缩略图内存缓存与已请求标记。
    pub(crate) attachment_thumb_cache: HashMap<u32, Arc<Image>>,
    pub(crate) attachment_thumb_requested: HashSet<u32>,
    /// WP-09：正在从服务器剥离中的附件 ID 集合及待二次确认的附件 ID。
    pub(crate) stripping_attachment_ids: HashSet<u32>,
    pub(crate) pending_strip_attachment_id: Option<u32>,
    /// WP-09：附件抽屉图片点击大图预览 `(attachment_id, filename, image)`。
    pub(crate) drawer_image_preview: Option<(u32, String, Option<Arc<Image>>)>,
    /// WP-09：阅读器页内查找栏状态与每个卡片的 `TextViewState` 缓存。
    pub(crate) reader_find_open: bool,
    pub(crate) reader_find_input: Entity<InputState>,
    pub(crate) reader_find_query: String,
    pub(crate) reader_find_active_match: usize,
    pub(crate) reader_find_total_matches: usize,
    pub(crate) reader_text_states: HashMap<u32, (String, Entity<TextViewState>)>,
    #[cfg(windows)]
    pub(crate) reader_webview: Option<std::rc::Rc<crate::webview_host::WebViewHost>>,
    #[cfg(windows)]
    pub(crate) reader_webview_key: Option<u64>,
    #[cfg(windows)]
    pub(crate) reader_webview_warmup_scheduled: bool,
    pub(crate) reader_popup_open: bool,
    /// WP-09：打印预览浮层状态。
    pub(crate) print_preview: Option<PrintPreviewState>,
    /// B-2：列表行菜单挂起的后续动作（等对应正文加载完成后执行）。
    pub(crate) pending_row_action: Option<(u32, PendingRowAction)>,
    /// 双击列表行或右键「在独立窗口中打开」时若正文尚未拉取完成，记录此邮件 ID 集合，待 MessageBodyLoaded 时自动弹出独立窗口。
    pub(crate) pending_popout_message_ids: std::collections::HashSet<u32>,
    /// 已打开的独立阅读窗口句柄表（按邮件 ID 去重激活，关闭时移除）。
    popout_message_windows: HashMap<u32, AnyWindowHandle>,
    /// B-2：正在抓取原始 MIME 以导出 .eml 的邮件 ID（「导出中…」状态，防重复点击）。
    pub(crate) pending_eml_export: Option<u32>,
    /// F-3：进行中的移动 / 删除 / 归档（成功回执到达后转成 `last_move_undo`）。
    pending_undo: Option<PendingUndo>,
    /// F-3：最近一次可撤销的移动 / 删除 / 归档记录（状态栏撤销条）。
    pub(crate) last_move_undo: Option<UndoRecord>,

    // ---- 阶段 3：全局路由 / 模态 / 偏好（方案文档五.1、五.2）----
    /// 持久化偏好副本（preferences.toml；last_view / last_folder 随切换落盘）。
    pub(crate) prefs: AppPreferences,
    /// 主内容区当前视图（Mail 三栏 / Gallery / Contacts）。
    pub(crate) main_view: MainContentView,
    /// 主窗口 / 全局浮层。
    pub(crate) active_modal: ActiveModal,
    /// 独立非模态设置窗口单实例引用。
    settings_window: Option<(AnyWindowHandle, Entity<SettingsWindow>)>,
    /// 独立设置窗口异步打开防重入标记。
    settings_window_opening: bool,
    /// 进入专注模式并临时强制简化阅读视图前保存的原视图开关，退出专注时恢复。
    focus_saved_prefer_plain: Option<bool>,
    /// 选中邮件已读标记代际（用于 2 秒延迟标已读防抖）。
    read_mark_generation: u64,
    /// 上一次应用到主窗口 `window.set_rem_size` 的界面缩放百分比。
    last_rem_scale: u32,
    /// 命令面板浮层实体（关闭时卸载）。
    palette: Option<Entity<CommandPaletteState>>,
    palette_sub: Option<Subscription>,
    /// 附件总览视图实体（首次进入时懒创建，常驻）。
    gallery: Option<Entity<GalleryState>>,
    gallery_sub: Option<Subscription>,
    /// 联系人视图实体（同上）。
    contacts: Option<Entity<ContactsState>>,
    contacts_sub: Option<Subscription>,
    /// 附件总览最近一次查询结果副本（供保存 / 定位原邮件）。
    gallery_items: Vec<CachedAttachment>,
    /// 附件总览当前搜索词（QueryChanged 时更新，重进视图重新请求）。
    gallery_query: String,
    /// 联系人当前搜索词（同上）。
    contacts_query: String,
    /// F-6 图片预览等待字节回传的附件 ID（`AttachmentBytesFetched` 时匹配）。
    gallery_preview_pending: Option<u32>,
    /// F-7 退订横幅待确认的邮件 ID（横幅内二次确认；切换邮件时清除）。
    pub(crate) pending_unsubscribe: Option<u32>,
    /// 后台自动刷新到期时刻（`None` 表示待重算；demo 模式不轮询）。
    auto_sync_deadline: Option<Instant>,
    /// Windows 系统托盘句柄（退出时显式销毁图标避免残留）。
    pub(crate) tray: Option<Arc<TrayHandle>>,
    /// 主窗口句柄（供托盘菜单触发写信 / 退出前保存窗口几何）。
    window_handle: AnyWindowHandle,
    /// 各账号最近一次收件箱未读数基线（首次加载只建基线不弹窗，后续增长才触发桌面通知）。
    last_unread_by_account: BTreeMap<u32, u32>,

    // ---- L-2：三栏可拖拽调宽（宽度实时更新，停 600ms 后防抖落盘）----
    /// 左栏宽度（180~280；窄轨模式固定 80px 不参与）。
    pub(crate) sidebar_width: f32,
    /// 中栏列表宽度（≥280；Hidden / Welcome 模式下列表可能占满或保持该宽度）。
    pub(crate) list_width: f32,
    /// 中栏 ≥600px 自动单行表格模式的迟滞状态（原版 LIST_WIDE_ON / LIST_WIDE_OFF）。
    pub(crate) list_wide: bool,
    /// 宽度落盘防抖截止时刻（`None` 表示无待写入）。
    layout_save_deadline: Option<Instant>,
    /// 宽度落盘防抖任务是否已在后台轮询（避免每次拖拽都重复 spawn）。
    layout_save_scheduled: bool,

    // ---- L-6：阅读器位置与专注模式 ----
    /// 阅读器位置（右 / 下 / 关，持久化到 prefs.reader_position）。
    pub(crate) reader_position: ReaderPosition,
    /// 专注模式（会话级，不持久化）：隐藏左栏与列表，只保留阅读器。
    pub(crate) focus_mode: bool,
    /// 阅读器在下方时列表的高度（持久化到 prefs.list_below_height）。
    pub(crate) list_below_height: f32,

    // ---- B-5：单封邮件亮 / 暗背景覆盖（msg_id → true=暗 / false=亮，会话级不持久化）----
    pub(crate) message_bg_picks: HashMap<u32, bool>,

    // ---- WP-08：宽表列宽、内联会话展开、日期分组折叠 ----
    pub(crate) list_column_widths: HashMap<hylki_mail_core::config::ListColumn, f32>,
    pub(crate) expanded_threads: HashSet<u32>,
    pub(crate) thread_children_cache: HashMap<u32, Vec<MessageSummary>>,
    pub(crate) collapsed_day_groups: HashSet<String>,
}

impl HylkiApp {
    pub(crate) fn new(
        demo_mode: bool,
        prefs: AppPreferences,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let demo_store = Arc::new(DemoStore::new());
        let window_handle = window.window_handle();
        let tray = TrayHandle::start(crate::APP_ICON_PNG).map(Arc::new);
        window.set_window_title("Hylki");
        // 点右上角关闭按钮时先保存窗口尺寸，再按偏好隐藏到系统托盘（不直接退出），
        // 真正退出由菜单或托盘右键「退出 Hylki」触发。
        let weak_self = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            if let Some(app) = weak_self.upgrade() {
                let should_hide = app.update(cx, |this, _| {
                    this.save_window_geometry(window);
                    this.prefs.close_to_minimize
                });
                if should_hide {
                    let hidden = app
                        .read(cx)
                        .tray
                        .as_ref()
                        .is_some_and(|t| t.hide_main_window());
                    if !hidden {
                        window.minimize_window();
                    }
                    return false;
                }
                app.update(cx, |this, cx| this.quit_app(cx));
                return false;
            }
            true
        });

        // 轮询 Windows 系统托盘命令，并在 minimize_to_tray 开启时将最小化窗口自动隐藏到托盘。
        if let Some(ref tray_handle) = tray {
            let weak_tray = Arc::downgrade(tray_handle);
            cx.spawn(async move |this, cx| loop {
                cx.background_executor()
                    .timer(Duration::from_millis(80))
                    .await;
                let Some(tray_handle) = weak_tray.upgrade() else {
                    break;
                };
                let cmds = tray_handle.drain_commands();
                let minimized = tray_handle.is_main_window_minimized();
                if cmds.is_empty() && !minimized {
                    continue;
                }
                let res = this.update(cx, |app, cx| {
                    if minimized && app.prefs.minimize_to_tray {
                        tray_handle.hide_main_window();
                    }
                    for cmd in cmds {
                        app.handle_tray_command(cmd, cx);
                    }
                });
                if res.is_err() {
                    break;
                }
            })
            .detach();
        }
        let data_dir = config::data_dir();
        let existing_accounts = if demo_mode {
            Vec::new()
        } else {
            config::load_accounts_in(&data_dir).unwrap_or_default()
        };

        // 方案文档五.2：default_view == "last" 时恢复上次浏览的文件夹 / 统一视图
        //（演示模式忽略恢复，仍从统一收件箱开始）。
        let mut start_view = CurrentView::UnifiedInbox;
        let mut start_folder_id = UNIFIED_INBOX_FOLDER_ID;
        if prefs.default_view == "last" && !demo_mode {
            if let Some((account_id, folder_id)) = prefs.last_folder {
                if existing_accounts.iter().any(|a| a.id == account_id) {
                    start_view = CurrentView::AccountFolder;
                    start_folder_id = folder_id;
                }
            } else if let Some(view) = Self::current_view_from_last(prefs.last_view.as_deref()) {
                start_view = view;
            }
        }

        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("搜索主题、发件人、正文（支持 from: / has:attachment）...")
        });

        cx.subscribe(&search_input, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.search_query = input.read(cx).value().to_string();
                this.clear_server_search();
                this.reset_list_pagination();
                this.reload_current_page(cx);
            }
        })
        .detach();

        let compose_to =
            cx.new(|cx| InputState::new(window, cx).placeholder("收件人：name@example.com"));
        let compose_cc = cx.new(|cx| {
            InputState::new(window, cx).placeholder("抄送：cc1@example.com, cc2@example.com")
        });
        let compose_bcc =
            cx.new(|cx| InputState::new(window, cx).placeholder("密送：bcc@example.com"));
        cx.subscribe(&compose_to, |this, _input, event: &InputEvent, cx| {
            this.on_recipient_input_event(ComposeAddressField::To, event, cx);
        })
        .detach();
        cx.subscribe(&compose_cc, |this, _input, event: &InputEvent, cx| {
            this.on_recipient_input_event(ComposeAddressField::Cc, event, cx);
        })
        .detach();
        cx.subscribe(&compose_bcc, |this, _input, event: &InputEvent, cx| {
            this.on_recipient_input_event(ComposeAddressField::Bcc, event, cx);
        })
        .detach();
        let compose_subject = cx.new(|cx| {
            InputState::new(window, cx).placeholder("主题：输入邮件主题（支持中文 IME）")
        });
        // WP-10-2：Reply-To 行（默认折叠在「抄送 / 密送 / Reply-To」开关之后）。
        let compose_reply_to = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Reply-To：回信转发到该地址（可留空）")
        });
        // WP-10-2：自定义 From 手输框（对应原版 `custom_from_row`）。
        let compose_from =
            cx.new(|cx| InputState::new(window, cx).placeholder("发件人：自定义 Name <addr>"));
        // WP-10-5：定时发送的自定义时间（默认收起，点「自定义时间…」才展开）。
        let compose_schedule_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("定时发送时间：2026-11-23 08:00"));
        // WP-10-5：保存模板时的名称输入框（默认取主题）。
        let compose_template_name =
            cx.new(|cx| InputState::new(window, cx).placeholder("模板名称（例如：客户回访）"));
        // WP-10-5：模板名称回车即保存（与「保存」按钮同一条路径）。
        cx.subscribe(
            &compose_template_name,
            |this, _input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.confirm_save_compose_template(cx);
                }
            },
        )
        .detach();
        // WP-10-5：定时时间输入框回车即确认（与「确定」按钮同一条路径）。
        cx.subscribe(
            &compose_schedule_input,
            |this, _input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.confirm_custom_compose_schedule(cx);
                }
            },
        )
        .detach();
        let compose_body = cx.new(|cx| TextareaState::new(window, cx).placeholder("输入邮件正文"));
        // F-4：主题 / 正文变更同样重置 5 秒防抖自动保存定时器。
        cx.subscribe(&compose_subject, |this, _input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.schedule_draft_autosave(cx);
                cx.notify();
            }
        })
        .detach();
        cx.subscribe(&compose_body, |this, _input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.schedule_draft_autosave(cx);
                cx.notify();
            }
        })
        .detach();
        // WP-10-2：自定义 From 与 Reply-To 变更同样参与自动保存；From 手输内容
        // 同步回 `compose_from_addr`，避免切回下拉后丢失。
        cx.subscribe(&compose_from, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.compose_from_addr = input.read(cx).value().to_string();
                this.schedule_draft_autosave(cx);
                cx.notify();
            }
        })
        .detach();
        cx.subscribe(&compose_reply_to, |this, _input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.schedule_draft_autosave(cx);
                cx.notify();
            }
        })
        .detach();

        let reader_find_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("在邮件正文中查找..."));
        cx.subscribe(
            &reader_find_input,
            |this, input, event: &InputEvent, cx| match event {
                InputEvent::Change => {
                    this.reader_find_query = input.read(cx).value().to_string();
                    this.reader_find_active_match = 0;
                    this.refresh_reader_find_highlights(cx);
                }
                InputEvent::PressEnter { secondary, shift } => {
                    this.step_reader_find(if *secondary || *shift { -1 } else { 1 }, cx);
                }
                _ => {}
            },
        )
        .detach();

        let initial_rem_scale = config::clamp_ui_text_scale(prefs.ui_text_scale_percent);
        window.set_rem_size(px(16.0 * initial_rem_scale as f32 / 100.0));
        let initial_show_accounts = prefs.show_accounts;
        let initial_sidebar_hidden = prefs.remember_sidebar_collapsed && prefs.sidebar_collapsed;
        let initial_focus_mode = prefs.focus_start_enabled;
        let initial_reader_prefer_plain =
            (initial_focus_mode && prefs.focus_reader_view) || prefs.reader_default_mode == "on";
        let focus_saved_prefer_plain = if initial_focus_mode && prefs.focus_reader_view {
            Some(prefs.reader_default_mode == "on")
        } else {
            None
        };
        let initial_fold_mode = prefs.effective_fold_messages_mode().to_string();

        let system_dark_mode = matches!(
            window.appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        );
        let initial_theme_id = theme::static_theme_id(&prefs.theme_id);
        let initial_dark_mode = prefs.dark_mode.unwrap_or(system_dark_mode);
        // L-2/L-6：从持久化偏好恢复三栏宽度与阅读器位置（旧配置缺字段回落默认值）。
        let initial_sidebar_width = prefs
            .sidebar_width
            .unwrap_or(crate::ui::sidebar::SIDEBAR_WIDTH)
            .clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH);
        let initial_list_width = prefs
            .list_width
            .unwrap_or(message_list::LIST_WIDTH)
            .max(LIST_MIN_WIDTH);
        let initial_list_wide = initial_list_width >= LIST_WIDE_ON;
        let initial_list_below_height = prefs
            .list_below_height
            .unwrap_or(BELOW_LIST_DEFAULT_HEIGHT)
            .clamp(BELOW_LIST_MIN_HEIGHT, BELOW_PANE_MIN_HEIGHT * 4.0);
        let initial_reader_position =
            ReaderPosition::from_key(&prefs.reader_position).unwrap_or_default();
        // 当偏好为「跟随系统」且系统是暗色时，补一次主题应用对齐 Windows 外观。
        if prefs.dark_mode.is_none() && system_dark_mode {
            theme::apply(initial_theme_id, true, cx);
        }

        if demo_mode {
            let demo_contacts = demo_store.list_known_contacts("", 0, 500);
            let current_page = demo_store.list_messages_page(DEMO_INBOX_ID, "", 0, prefs.page_size);
            let mut selected_message = current_page
                .items
                .first()
                .and_then(|msg| demo_store.get_message_detail(msg.id));
            if !prefs.thread_newest_first {
                if let Some(ref mut d) = selected_message {
                    d.thread_messages.reverse();
                }
            }
            let initial_collapsed = selected_message
                .as_ref()
                .map(|d| {
                    compute_default_collapsed_cards(
                        d,
                        prefs.thread_newest_first,
                        &initial_fold_mode,
                    )
                })
                .unwrap_or_default();
            let mut folders_by_account = BTreeMap::new();
            folders_by_account.insert(1, demo_store.folders.clone());
            let demo_acc = AccountConfig {
                id: 1,
                display_name: demo_store.accounts[0].display_name.clone(),
                email: demo_store.accounts[0].email.clone(),
                ..Default::default()
            };
            return Self {
                demo_mode: true,
                demo_store,
                worker: None,
                accounts: vec![demo_acc],
                folders_by_account,
                selected_folder_id: DEMO_INBOX_ID,
                current_view: CurrentView::UnifiedInbox,
                loaded_limit: prefs.page_size,
                list_scroll_handle: ScrollHandle::new(),
                loading_more_local: false,
                loading_more_remote: false,
                backfill: BackfillDriveState::default(),
                current_page,
                selected_message,
                selected_message_is_placeholder: false,
                pending_body_message_id: None,
                selected_message_ids: HashSet::new(),
                drafts: Vec::new(),
                templates: Vec::new(),
                outbox_by_account: BTreeMap::new(),
                right_mode: RightPaneMode::Reader,
                welcome_step: 0,
                search_input,
                search_query: String::new(),
                compose_account_id: 1,
                compose_draft_id: None,
                compose_to,
                compose_cc,
                compose_bcc,
                compose_show_cc_bcc: false,
                compose_from_addr: String::new(),
                compose_from_custom: false,
                compose_from,
                compose_reply_to,
                compose_subject,
                compose_body,
                compose_format: ComposeFormat::default(),
                compose_preview: false,
                compose_split: false,
                compose_split_height: COMPOSE_SPLIT_DEFAULT_HEIGHT,
                compose_detached: false,
                compose_window: None,
                compose_schedule_open: false,
                compose_schedule_input: compose_schedule_input.clone(),
                compose_schedule_error: None,
                compose_template_save_open: false,
                compose_template_name: compose_template_name.clone(),
                compose_in_reply_to: String::new(),
                compose_attachments: Vec::new(),
                compose_confirm_discard: false,
                pending_forward_msg_id: None,
                compose_autocomplete_field: None,
                compose_to_pills: Vec::new(),
                compose_cc_pills: Vec::new(),
                compose_bcc_pills: Vec::new(),
                compose_pill_resume: None,
                compose_autosave_generation: 0,
                compose_session_generation: 0,
                file_dialog_open: false,
                print_preview_generation: 0,
                compose_autosave_dirty: false,
                compose_autosave_pending: false,
                compose_autosaved_new: false,
                compose_autosaved_at: None,
                known_contacts: demo_contacts,
                available_tags: Vec::new(),
                folder_action: None,
                folder_action_input: cx
                    .new(|cx| InputState::new(window, cx).placeholder("输入文件夹名称…")),
                pending_send: None,
                quitting: false,
                quit_preparing: false,
                next_send_token: 1,
                compose_attachment_reminder: None,
                cfg_feedback: None,
                status_line: SharedString::from("演示模式 (--demo)：已加载 10,000 封模拟邮件"),
                error_banner: None,
                update: UpdateState::default(),
                theme_id: initial_theme_id,
                dark_mode: initial_dark_mode,
                system_dark_mode,
                sidebar_hidden: initial_sidebar_hidden,
                auto_rail_active: false,
                sidebar_peek_open: false,
                list_focused: false,
                kbd_cursor_message_id: None,
                search_open: false,
                list_sort: MessageSort::DateNewest,
                quick_filters: QuickFilterSet::default(),
                search_scope: SearchScope::ThisFolder,
                server_search_loading: false,
                server_search_query: String::new(),
                server_search_results: Vec::new(),
                collapsed_accounts: HashSet::new(),
                show_accounts: initial_show_accounts,
                pending_empty_folder: None,
                reader_prefer_plain: initial_reader_prefer_plain,
                allowed_remote_image_ids: HashSet::new(),
                collapsed_thread_cards: initial_collapsed,
                raw_source_message_ids: HashSet::new(),
                expanded_quote_message_ids: HashSet::new(),
                attachment_drawer_collapsed: false,
                attachment_drawer_height: DRAWER_DEFAULT_HEIGHT,
                attachment_drawer_list_view: false,
                attachment_drawer_sort_desc: false,
                attachment_thumb_cache: HashMap::new(),
                attachment_thumb_requested: HashSet::new(),
                stripping_attachment_ids: HashSet::new(),
                pending_strip_attachment_id: None,
                drawer_image_preview: None,
                reader_find_open: false,
                reader_find_input,
                reader_find_query: String::new(),
                reader_find_active_match: 0,
                reader_find_total_matches: 0,
                reader_text_states: HashMap::new(),
                #[cfg(windows)]
                reader_webview: None,
                #[cfg(windows)]
                reader_webview_key: None,
                #[cfg(windows)]
                reader_webview_warmup_scheduled: false,
                reader_popup_open: false,
                print_preview: None,
                pending_row_action: None,
                pending_popout_message_ids: std::collections::HashSet::new(),
                popout_message_windows: HashMap::new(),
                pending_eml_export: None,
                pending_undo: None,
                last_move_undo: None,
                prefs,
                main_view: MainContentView::Mail,
                active_modal: ActiveModal::None,
                settings_window: None,
                settings_window_opening: false,
                focus_saved_prefer_plain,
                read_mark_generation: 0,
                last_rem_scale: initial_rem_scale,
                palette: None,
                palette_sub: None,
                gallery: None,
                gallery_sub: None,
                contacts: None,
                contacts_sub: None,
                gallery_items: Vec::new(),
                gallery_query: String::new(),
                contacts_query: String::new(),
                gallery_preview_pending: None,
                pending_unsubscribe: None,
                auto_sync_deadline: None,
                tray,
                window_handle,
                last_unread_by_account: BTreeMap::new(),
                sidebar_width: initial_sidebar_width,
                list_width: initial_list_width,
                list_wide: initial_list_wide,
                layout_save_deadline: None,
                layout_save_scheduled: false,
                reader_position: initial_reader_position,
                focus_mode: initial_focus_mode,
                list_below_height: initial_list_below_height,
                message_bg_picks: HashMap::new(),
                list_column_widths: HashMap::new(),
                expanded_threads: HashSet::new(),
                thread_children_cache: HashMap::new(),
                collapsed_day_groups: HashSet::new(),
            };
        }

        let (worker_evt_tx, mut worker_evt_rx) = futures::channel::mpsc::channel::<()>(16);
        let worker = MailWorkerHandle::spawn_with_notifier(
            data_dir,
            Some(Arc::new(move || {
                let _ = worker_evt_tx.clone().try_send(());
            })),
        )
        .ok()
        .map(Arc::new);
        if let Some(ref w) = worker {
            if !existing_accounts.is_empty() {
                w.send(MailRequest::LoadAllCached);
                // WP-10-5：写信模板与缓存一起加载，首次打开写信就有模板可选。
                w.send(MailRequest::LoadTemplates);
                w.send(MailRequest::ListKnownContacts {
                    query: String::new(),
                    offset: 0,
                    limit: 500,
                });
                w.send(MailRequest::SyncAllAccounts);
            }
            let weak_worker = Arc::downgrade(w);
            cx.spawn(async move |this, cx| {
                use futures::StreamExt as _;
                loop {
                    let Some(w) = weak_worker.upgrade() else {
                        break;
                    };
                    let events = w.drain_events();
                    if !events.is_empty() {
                        if this
                            .update(cx, |app, cx| {
                                app.apply_worker_events(events, cx);
                            })
                            .is_err()
                        {
                            break;
                        }
                        continue;
                    }
                    if worker_evt_rx.next().await.is_none() {
                        break;
                    }
                }
            })
            .detach();

            // 方案文档五.2：后台定时刷新——每 30 秒醒来检查一次到期状态，
            // 间隔由偏好 auto_sync_minutes 控制（0 = 手动），demo 模式不轮询。
            cx.spawn(async move |this, cx| loop {
                cx.background_executor()
                    .timer(Duration::from_secs(30))
                    .await;
                let res = this.update(cx, |app, _cx| app.auto_sync_tick());
                if res.is_err() {
                    break;
                }
            })
            .detach();
        }

        let has_account = !existing_accounts.is_empty();
        let compose_account_id = existing_accounts.first().map(|a| a.id).unwrap_or(1);
        let status_line = if has_account {
            SharedString::from("正在从本地缓存加载并同步所有账号...")
        } else {
            SharedString::from("请先添加至少一个 IMAP / SMTP 邮箱账号")
        };

        let mut app = Self {
            welcome_step: 0,
            demo_mode: false,
            demo_store,
            worker,
            accounts: existing_accounts,
            folders_by_account: BTreeMap::new(),
            selected_folder_id: start_folder_id,
            current_view: start_view,
            loaded_limit: prefs.page_size,
            list_scroll_handle: ScrollHandle::new(),
            loading_more_local: false,
            loading_more_remote: false,
            backfill: BackfillDriveState::default(),
            current_page: MailPage {
                folder_id: UNIFIED_INBOX_FOLDER_ID,
                offset: 0,
                total: 0,
                items: Vec::new(),
            },
            selected_message: None,
            selected_message_is_placeholder: false,
            pending_body_message_id: None,
            selected_message_ids: HashSet::new(),
            drafts: Vec::new(),
            templates: Vec::new(),
            outbox_by_account: BTreeMap::new(),
            // 方案文档页面 2：启动且无账号（非 demo）时进入欢迎向导。
            right_mode: if has_account {
                RightPaneMode::Reader
            } else {
                RightPaneMode::Welcome
            },
            search_input,
            search_query: String::new(),
            compose_account_id,
            compose_draft_id: None,
            compose_to,
            compose_cc,
            compose_bcc,
            compose_show_cc_bcc: false,
            compose_from_addr: String::new(),
            compose_from_custom: false,
            compose_from,
            compose_reply_to,
            compose_subject,
            compose_body,
            compose_format: ComposeFormat::default(),
            compose_preview: false,
            compose_split: false,
            compose_split_height: COMPOSE_SPLIT_DEFAULT_HEIGHT,
            compose_detached: false,
            compose_window: None,
            compose_schedule_open: false,
            compose_schedule_input: compose_schedule_input.clone(),
            compose_schedule_error: None,
            compose_template_save_open: false,
            compose_template_name: compose_template_name.clone(),
            compose_in_reply_to: String::new(),
            compose_attachments: Vec::new(),
            compose_confirm_discard: false,
            pending_forward_msg_id: None,
            compose_autocomplete_field: None,
            compose_to_pills: Vec::new(),
            compose_cc_pills: Vec::new(),
            compose_bcc_pills: Vec::new(),
            compose_pill_resume: None,
            compose_autosave_generation: 0,
            compose_session_generation: 0,
            file_dialog_open: false,
            print_preview_generation: 0,
            compose_autosave_dirty: false,
            compose_autosave_pending: false,
            compose_autosaved_new: false,
            compose_autosaved_at: None,
            known_contacts: Vec::new(),
            available_tags: Vec::new(),
            folder_action: None,
            folder_action_input: cx
                .new(|cx| InputState::new(window, cx).placeholder("输入文件夹名称…")),
            pending_send: None,
            quitting: false,
            quit_preparing: false,
            next_send_token: 1,
            compose_attachment_reminder: None,
            cfg_feedback: None,
            status_line,
            error_banner: None,
            update: UpdateState::default(),
            theme_id: initial_theme_id,
            dark_mode: initial_dark_mode,
            system_dark_mode,
            sidebar_hidden: initial_sidebar_hidden,
            auto_rail_active: false,
            sidebar_peek_open: false,
            list_focused: false,
            kbd_cursor_message_id: None,
            search_open: false,
            list_sort: MessageSort::DateNewest,
            quick_filters: QuickFilterSet::default(),
            search_scope: SearchScope::ThisFolder,
            server_search_loading: false,
            server_search_query: String::new(),
            server_search_results: Vec::new(),
            collapsed_accounts: HashSet::new(),
            show_accounts: initial_show_accounts,
            pending_empty_folder: None,
            reader_prefer_plain: initial_reader_prefer_plain,
            allowed_remote_image_ids: HashSet::new(),
            collapsed_thread_cards: HashSet::new(),
            raw_source_message_ids: HashSet::new(),
            expanded_quote_message_ids: HashSet::new(),
            attachment_drawer_collapsed: false,
            attachment_drawer_height: DRAWER_DEFAULT_HEIGHT,
            attachment_drawer_list_view: false,
            attachment_drawer_sort_desc: false,
            attachment_thumb_cache: HashMap::new(),
            attachment_thumb_requested: HashSet::new(),
            stripping_attachment_ids: HashSet::new(),
            pending_strip_attachment_id: None,
            drawer_image_preview: None,
            reader_find_open: false,
            reader_find_input,
            reader_find_query: String::new(),
            reader_find_active_match: 0,
            reader_find_total_matches: 0,
            reader_text_states: HashMap::new(),
            #[cfg(windows)]
            reader_webview: None,
            #[cfg(windows)]
            reader_webview_key: None,
            #[cfg(windows)]
            reader_webview_warmup_scheduled: false,
            reader_popup_open: false,
            print_preview: None,
            pending_row_action: None,
            pending_popout_message_ids: std::collections::HashSet::new(),
            popout_message_windows: HashMap::new(),
            pending_eml_export: None,
            pending_undo: None,
            last_move_undo: None,
            prefs,
            main_view: MainContentView::Mail,
            active_modal: ActiveModal::None,
            settings_window: None,
            settings_window_opening: false,
            focus_saved_prefer_plain,
            read_mark_generation: 0,
            last_rem_scale: initial_rem_scale,
            palette: None,
            palette_sub: None,
            gallery: None,
            gallery_sub: None,
            contacts: None,
            contacts_sub: None,
            gallery_items: Vec::new(),
            gallery_query: String::new(),
            contacts_query: String::new(),
            gallery_preview_pending: None,
            pending_unsubscribe: None,
            auto_sync_deadline: None,
            tray,
            window_handle,
            last_unread_by_account: BTreeMap::new(),
            sidebar_width: initial_sidebar_width,
            list_width: initial_list_width,
            list_wide: initial_list_wide,
            layout_save_deadline: None,
            layout_save_scheduled: false,
            reader_position: initial_reader_position,
            focus_mode: initial_focus_mode,
            list_below_height: initial_list_below_height,
            message_bg_picks: HashMap::new(),
            list_column_widths: HashMap::new(),
            expanded_threads: HashSet::new(),
            thread_children_cache: HashMap::new(),
            collapsed_day_groups: HashSet::new(),
        };

        // 启动视图非统一收件箱时立即请求对应列表（LoadAllCached 只覆盖收件箱）。
        if app.current_view != CurrentView::UnifiedInbox {
            app.reload_current_page(cx);
        }

        // 启动后延迟检查更新：避开首屏加载，结果只写状态栏（设置中可关闭）。
        // 调试构建的版本号与发布标签不一致，自动检查只会误报，跳过。
        if app.prefs.check_updates && !cfg!(debug_assertions) {
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_secs(10))
                    .await;
                let _ = this.update(cx, |app, cx| app.check_updates(false, cx));
            })
            .detach();
        }
        app
    }

    /// 把持久化的 last_view 字符串映射回统一视图（未知值回落 None）。
    fn current_view_from_last(last: Option<&str>) -> Option<CurrentView> {
        match last? {
            "unified" => Some(CurrentView::UnifiedInbox),
            "unread" => Some(CurrentView::Unread),
            "starred" => Some(CurrentView::Starred),
            "sent" => Some(CurrentView::Sent),
            "drafts" => Some(CurrentView::Drafts),
            "archive" => Some(CurrentView::Archive),
            "junk" => Some(CurrentView::Junk),
            "trash" => Some(CurrentView::Trash),
            "snoozed" => Some(CurrentView::Snoozed),
            _ => None,
        }
    }

    fn apply_worker_events(&mut self, events: Vec<WorkerEvent>, cx: &mut Context<Self>) {
        let mut folders_updated = false;
        let mut total_new_unread: u32 = 0;
        let mut notify_account_id: Option<u32> = None;
        let mut latest_unread_preview: Option<(String, String)> = None;

        for evt in events {
            match evt {
                WorkerEvent::AccountVerified { ok, message } => {
                    self.cfg_feedback = Some((ok, SharedString::from(message.clone())));
                    self.status_line = SharedString::from(message);
                    if !ok {
                        self.with_settings_window(cx, |win, _, cx| win.on_account_save_failed(cx));
                    }
                }
                WorkerEvent::AccountSaved { account } => {
                    if let Some(existing) = self.accounts.iter_mut().find(|a| a.id == account.id) {
                        *existing = account.clone();
                    } else {
                        self.accounts.push(account.clone());
                    }
                    if account.enabled {
                        self.compose_account_id = account.id;
                    }
                    self.right_mode = RightPaneMode::Reader;
                    self.cfg_feedback = Some((
                        true,
                        SharedString::from(format!("账号 {} 已保存，正在同步...", account.email)),
                    ));
                    let account_for_win = account.clone();
                    self.with_settings_window(cx, move |win, window, cx| {
                        win.on_account_saved(&account_for_win, window, cx);
                    });
                    if self.current_view.is_unified() {
                        self.reload_current_page(cx);
                    }
                    self.error_banner = None;
                    self.status_line =
                        SharedString::from(format!("账号 {} 已保存，正在同步...", account.email));
                }
                WorkerEvent::AccountDeleted { account_id } => {
                    self.accounts.retain(|a| a.id != account_id);
                    self.folders_by_account.remove(&account_id);
                    self.outbox_by_account.remove(&account_id);
                    self.last_unread_by_account.remove(&account_id);
                    folders_updated = true;
                    self.selected_folder_id = UNIFIED_INBOX_FOLDER_ID;
                    self.current_view = CurrentView::UnifiedInbox;
                    self.selected_message = None;
                    self.with_settings_window(cx, move |win, window, cx| {
                        win.on_account_deleted(account_id, window, cx);
                    });
                    if self.accounts.is_empty() {
                        self.current_page = MailPage {
                            folder_id: UNIFIED_INBOX_FOLDER_ID,
                            offset: 0,
                            total: 0,
                            items: Vec::new(),
                        };
                    } else if self.current_view.is_unified() {
                        self.reload_current_page(cx);
                    }
                    self.right_mode = if self.accounts.is_empty() {
                        RightPaneMode::Welcome
                    } else {
                        RightPaneMode::Reader
                    };
                    self.status_line = SharedString::from("账号及本地缓存已移除");
                }
                WorkerEvent::AccountsUpdated { accounts } => {
                    self.accounts = accounts;
                    // 任务一：停用账号不再静默切换写信账号。写信界面检测到当前所选
                    // 账号停用时显示提示，由用户明确选择可用账号后才能发送。
                    if self.current_view == CurrentView::AccountFolder {
                        let current_disabled =
                            self.folders_by_account.iter().any(|(&acc_id, folders)| {
                                folders.iter().any(|f| f.id == self.selected_folder_id)
                                    && self.accounts.iter().any(|a| a.id == acc_id && !a.enabled)
                            });
                        if current_disabled {
                            self.selected_folder_id = UNIFIED_INBOX_FOLDER_ID;
                            self.current_view = CurrentView::UnifiedInbox;
                            self.selected_message = None;
                        }
                    }
                    folders_updated = true;
                    if self.current_view.is_unified() {
                        self.reload_current_page(cx);
                    }
                    // 任务四：账号列表变化（保存 / 启停）后重新允许后台历史索引
                    // 探测，续排由随后的页面事件驱动。
                    self.backfill.on_scope_refresh();
                }
                WorkerEvent::FoldersLoaded {
                    account_id,
                    folders,
                } => {
                    let inbox_unread: u32 = folders
                        .iter()
                        .filter(|f| {
                            f.path.eq_ignore_ascii_case("INBOX") || f.name.contains("收件箱")
                        })
                        .map(|f| f.unread_count)
                        .sum();
                    let delta = detect_new_unread(
                        &mut self.last_unread_by_account,
                        account_id,
                        inbox_unread,
                    );
                    if delta > 0 {
                        total_new_unread = total_new_unread.saturating_add(delta);
                        notify_account_id = Some(account_id);
                    }
                    self.folders_by_account.insert(account_id, folders);
                    folders_updated = true;
                    // 任务四：新文件夹可能出现（首次同步 / 新账号），重新允许
                    // 后台历史索引探测；实际续排仍由下一个事件驱动点执行。
                    self.backfill.on_scope_refresh();
                }
                WorkerEvent::MessagesPageLoaded(page) => {
                    self.loading_more_local = false;
                    if latest_unread_preview.is_none() {
                        if let Some(msg) = page.items.iter().find(|m| m.unread) {
                            let (name, email) = split_sender(&msg.sender);
                            let from = if name.is_empty() {
                                if email.is_empty() {
                                    msg.sender.clone()
                                } else {
                                    email
                                }
                            } else {
                                name
                            };
                            latest_unread_preview = Some((from, msg.subject.clone()));
                        }
                    }
                    // 仅当事件页与当前视图匹配时才采用；统一视图（星标/已发送/归档）
                    // 忽略文件夹级页面事件。
                    let page_matches = match self.current_view {
                        CurrentView::UnifiedInbox => {
                            !self.demo_mode && page.folder_id == UNIFIED_INBOX_FOLDER_ID
                        }
                        CurrentView::AccountFolder => page.folder_id == self.selected_folder_id,
                        CurrentView::Unread
                        | CurrentView::Starred
                        | CurrentView::Sent
                        | CurrentView::Drafts
                        | CurrentView::Archive
                        | CurrentView::Junk
                        | CurrentView::Trash
                        | CurrentView::Snoozed => false,
                    };
                    if page_matches {
                        self.merge_loaded_page(page, cx);
                    } else if matches!(
                        self.current_view,
                        CurrentView::UnifiedInbox | CurrentView::AccountFolder
                    ) {
                        self.reload_current_page(cx);
                    }
                    // 任务四：首屏页面就绪后启动后台历史索引；此后每次列表页
                    // 事件都会顺带检查是否需要续排（in-flight 保证最多一个批次）。
                    self.maybe_drive_backfill(cx);
                }
                WorkerEvent::OlderMessagesLoaded {
                    folder_id: _,
                    fetched_count,
                    has_more_remote: _,
                } => {
                    self.loading_more_remote = false;
                    if fetched_count > 0 {
                        self.status_line =
                            SharedString::from(format!("已预加载 {fetched_count} 封更早邮件"));
                        self.reload_current_page(cx);
                    } else {
                        self.status_line = SharedString::from("已加载全部历史邮件");
                    }
                }
                WorkerEvent::BackfillHistoryBatchDone {
                    folder_id,
                    fetched_count,
                    body_only,
                    has_more_remote,
                } => {
                    // 仅当本批确有新邮件头落盘且展示范围覆盖该文件夹（统一视图
                    // 或正选中该文件夹）时才刷新页面；后台其他文件夹或纯正文
                    // 索引批次不重绘，避免索引期间列表高频抖动。
                    let view_covers_folder =
                        !matches!(self.current_view, CurrentView::AccountFolder)
                            || folder_id == self.selected_folder_id;
                    if fetched_count > 0 && !body_only && view_covers_folder {
                        self.reload_current_page(cx);
                    }
                    self.backfill.on_batch_done(has_more_remote);
                    self.maybe_drive_backfill(cx);
                }
                WorkerEvent::MessageBodyLoaded(detail) => {
                    let popped_out = self.pending_popout_message_ids.remove(&detail.id);
                    if popped_out {
                        self.pop_out_message_window(detail.clone(), cx);
                    }
                    if !should_update_main_reader_on_body_loaded(
                        detail.id,
                        self.pending_body_message_id,
                        popped_out,
                        self.selected_message.as_ref().map(|m| m.id),
                    ) {
                        // 用户已切换主阅读器邮件，或仅在独立窗口打开：不覆盖主阅读器。
                        continue;
                    }
                    let pending = self.pending_row_action.take();
                    self.set_selected_message(detail.clone());
                    self.error_banner = None;
                    // B-2：行菜单挂起的动作（回复 / 复制正文 / 查看原文）在正文就绪后执行。
                    if let Some((msg_id, action)) = pending {
                        if msg_id == detail.id {
                            // 同样运行在 `HylkiApp::update` 内部，需延后一 tick 再更新自身实体。
                            let entity = cx.entity();
                            let handle = self.window_handle;
                            cx.defer(move |cx| {
                                let _ = handle.update(cx, |_, window, cx| {
                                    entity.update(cx, |app, cx| {
                                        app.finish_pending_row_action(&detail, window, cx);
                                    });
                                });
                            });
                        } else {
                            self.pending_row_action = Some((msg_id, action));
                        }
                    }
                }
                WorkerEvent::RawMessageFetched { message_id, bytes } => {
                    // B-2：.eml 原始字节已取回，弹系统保存对话框写出「主题.eml」。
                    self.pending_eml_export = None;
                    let subject = self
                        .selected_message
                        .as_ref()
                        .filter(|d| d.id == message_id)
                        .map(|d| d.subject.clone())
                        .or_else(|| {
                            self.current_page
                                .items
                                .iter()
                                .find(|m| m.id == message_id)
                                .map(|m| m.subject.clone())
                        })
                        .unwrap_or_else(|| "邮件".to_string());
                    self.export_eml_bytes(&subject, bytes, cx);
                }
                WorkerEvent::DraftsUpdated { drafts } => {
                    self.drafts = drafts;
                }
                WorkerEvent::TemplatesUpdated { templates } => {
                    self.templates = templates;
                }
                WorkerEvent::OutboxUpdated { account_id, items } => {
                    // 撤销缓冲对应的记录已被投递（如侧栏手动重发）或删除时清除缓冲横幅。
                    if let Some(outbox_id) = self.pending_send.as_ref().and_then(|p| {
                        if p.account_id == account_id {
                            p.outbox_id
                        } else {
                            None
                        }
                    }) {
                        if !items.iter().any(|i| i.id == outbox_id) {
                            self.pending_send = None;
                        }
                    }
                    if items.is_empty() {
                        self.outbox_by_account.remove(&account_id);
                    } else {
                        self.outbox_by_account.insert(account_id, items);
                    }
                }
                WorkerEvent::OutboxEnqueued {
                    token,
                    account_id,
                    outbox_id,
                } => {
                    // 撤销发送：待发记录已落盘发件箱（原草稿同步交接删除），回填缓冲 ID。
                    let matched = self.pending_send.as_ref().is_some_and(|p| p.token == token);
                    if !matched {
                        // 已让位给后续发送或撤销流程已完成的旧回执，忽略。
                        continue;
                    }
                    let undo_requested = {
                        let pending = self.pending_send.as_mut().unwrap();
                        pending.outbox_id = Some(outbox_id);
                        pending.undo_requested
                    };
                    if self.quitting && !undo_requested {
                        self.quit_app(cx);
                    } else if undo_requested {
                        let result = self
                            .worker
                            .as_ref()
                            .ok_or_else(|| "邮件后台线程不可用".to_string())
                            .and_then(|w| {
                                w.send_checked(MailRequest::UndoOutboxItem {
                                    account_id,
                                    outbox_id,
                                    token,
                                })
                            });
                        if let Err(message) = result {
                            self.quitting = false;
                            self.update.cancel_install();
                            if let Some(pending) = self.pending_send.as_mut() {
                                pending.undo_requested = false;
                            }
                            self.error_banner = Some(SharedString::from(message.clone()));
                            self.status_line = SharedString::from(message);
                        }
                    } else if self
                        .pending_send
                        .as_ref()
                        .is_some_and(|p| p.commit_requested)
                    {
                        self.commit_pending_send(token, cx);
                    } else if let Some(pending) = self.pending_send.as_ref() {
                        self.start_send_countdown(token, pending.remaining_secs, cx);
                    }
                    cx.notify();
                }
                WorkerEvent::OutboxEnqueueFailed {
                    token,
                    message,
                    recovered_draft_id,
                } => {
                    self.quitting = false;
                    self.update.cancel_install();
                    if self.pending_send.as_ref().is_some_and(|p| p.token == token) {
                        if recovered_draft_id.is_some() {
                            self.pending_send = None;
                        } else if let Some(pending) = self.pending_send.as_mut() {
                            pending.undo_requested = false;
                            pending.enqueue_failed = true;
                        }
                    }
                    self.error_banner = Some(SharedString::from(message.clone()));
                    self.status_line = SharedString::from(if recovered_draft_id.is_some() {
                        format!("{message}；邮件已保存到草稿")
                    } else {
                        format!("{message}；请关闭当前写信页后点击撤销发送恢复邮件")
                    });
                    cx.notify();
                }
                WorkerEvent::QuitPrepared { result } => {
                    self.quit_preparing = false;
                    match result {
                        Ok(()) if self.quitting => {
                            self.shutdown_tray();
                            cx.quit();
                        }
                        Ok(()) => {}
                        Err(message) => {
                            self.quitting = false;
                            self.update.cancel_install();
                            self.error_banner = Some(SharedString::from(message.clone()));
                            self.status_line = SharedString::from(message);
                        }
                    }
                }
                WorkerEvent::OutboxUndoResult { token, result } => {
                    if self.pending_send.as_ref().map(|p| p.token) != Some(token) {
                        continue;
                    }
                    match result {
                        Ok(Some(draft_id)) => {
                            let pending = self.pending_send.take().unwrap();
                            let handle = self.window_handle;
                            cx.spawn(async move |this, cx| {
                                let _ = cx.update(|cx| {
                                    handle.update(cx, |_, window, cx| {
                                        let _ = this.update(cx, |app, cx| {
                                            if !app.has_active_compose() {
                                                app.restore_pending_send_to_compose(
                                                    &pending, window, cx,
                                                );
                                                app.compose_draft_id = Some(draft_id);
                                                app.status_line =
                                                    SharedString::from("已撤销发送，返回编辑邮件");
                                            } else {
                                                app.status_line = SharedString::from(
                                                    "已撤销发送，邮件已保存到草稿",
                                                );
                                            }
                                            cx.notify();
                                        });
                                    })
                                });
                            })
                            .detach();
                        }
                        Ok(None) => {
                            self.pending_send = None;
                            self.status_line = SharedString::from("邮件已开始投递，无法撤销");
                        }
                        Err(message) => {
                            self.quitting = false;
                            self.update.cancel_install();
                            if let Some(pending) = self.pending_send.as_mut() {
                                pending.undo_requested = false;
                            }
                            self.error_banner = Some(SharedString::from(message.clone()));
                            self.status_line = SharedString::from(message);
                        }
                    }
                    if self.quitting {
                        if self.pending_send.as_ref().is_some_and(|p| p.undo_requested) {
                            self.quitting = false;
                            self.update.cancel_install();
                        } else {
                            self.quit_app(cx);
                        }
                    }
                    cx.notify();
                }
                WorkerEvent::DraftSent {
                    account_id: _,
                    subject,
                } => {
                    self.status_line = SharedString::from(format!("邮件已发出：{}", subject));
                }
                WorkerEvent::AttachmentSaved { path } => {
                    self.status_line =
                        SharedString::from(format!("附件已保存至：{}", path.display()));
                }
                WorkerEvent::AttachmentsSaved { count, dest_dir } => {
                    self.status_line = SharedString::from(format!(
                        "已保存 {count} 个附件至：{}",
                        dest_dir.display()
                    ));
                }
                WorkerEvent::Status {
                    account_id: _,
                    text,
                } => {
                    self.status_line = SharedString::from(text);
                }
                WorkerEvent::Error {
                    account_id: _,
                    message,
                } => {
                    // B-2：抓取原文失败时退出「导出中…」状态。
                    self.pending_eml_export = None;
                    // 正文拉取失败：解除占位态，让阅读器重新出现「加载完整正文」入口。
                    self.pending_body_message_id = None;
                    self.pending_popout_message_ids.clear();
                    self.with_settings_window(cx, |win, _, cx| win.on_account_save_failed(cx));
                    self.error_banner = Some(SharedString::from(message.clone()));
                    self.status_line = SharedString::from(message);
                }
                WorkerEvent::OAuthRebindRejected {
                    account_id: _,
                    returned_email,
                } => {
                    // 任务二：Microsoft 重授权选择了另一个邮箱，原账号、凭据与缓存
                    // 保持不动；在设置窗口错误反馈条提示重新授权或走添加账号流程。
                    let message = format!(
                        "微软授权返回了不同的邮箱（{returned_email}），已保留原账号不变。请重新授权并选择原邮箱，或通过「添加账号」接入新邮箱。"
                    );
                    self.cfg_feedback = Some((false, SharedString::from(message.clone())));
                    self.with_settings_window(cx, |win, _, cx| win.on_account_save_failed(cx));
                    self.status_line = SharedString::from(message);
                }
                WorkerEvent::OAuthReauthorized {
                    account_id: _,
                    email,
                } => {
                    // 编辑已有账号重新授权成功：新令牌已写入 Keyring，账号配置与
                    // 缓存未动；仅提示成功，保留当前编辑表单中未保存的修改。
                    let message = format!("账号 {email} 重新授权成功");
                    self.cfg_feedback = Some((true, SharedString::from(message.clone())));
                    self.status_line = SharedString::from(message);
                }
                WorkerEvent::MessageArchived {
                    account_id,
                    message_id,
                    dest_folder_path,
                    rfc_message_id,
                }
                | WorkerEvent::MessageDeleted {
                    account_id,
                    message_id,
                    dest_folder_path,
                    rfc_message_id,
                }
                | WorkerEvent::MessageMarkedSpam {
                    account_id,
                    message_id,
                    dest_folder_path,
                    rfc_message_id,
                } => {
                    // F-3：请求时已捕获撤销前半段，成功回执补全定位信息后转为可撤销条；
                    // 永久删除（无回收站）或缺少 Message-ID 时不提供撤销。
                    if let Some(pending) = self.pending_undo.take() {
                        if pending.account_id == account_id
                            && pending.message_id == message_id
                            && !dest_folder_path.is_empty()
                            && !rfc_message_id.is_empty()
                        {
                            let record = pending.into_record(dest_folder_path, rfc_message_id);
                            self.set_last_move_undo(record, cx);
                        }
                    }
                    // 后端回执：随事件先行到达的 FoldersLoaded 已刷新文件夹计数；
                    // 这里刷新当前列表并在阅读态显示该邮件时回到空态。
                    self.after_message_moved(account_id, message_id, cx);
                }
                WorkerEvent::MessageMoved {
                    account_id,
                    message_id,
                    target_folder_path,
                    rfc_message_id,
                } => {
                    if let Some(pending) = self.pending_undo.take() {
                        if pending.account_id == account_id
                            && pending.message_id == message_id
                            && !rfc_message_id.is_empty()
                        {
                            let record = pending.into_record(target_folder_path, rfc_message_id);
                            self.set_last_move_undo(record, cx);
                        }
                    }
                    self.after_message_moved(account_id, message_id, cx);
                }
                WorkerEvent::AllFoldersSearchLoaded(page) => {
                    self.loading_more_local = false;
                    if self.search_scope == SearchScope::AllFolders
                        && !self.search_query.trim().is_empty()
                    {
                        self.merge_loaded_page(page, cx);
                    }
                }
                WorkerEvent::ServerSearchResults { query, items } => {
                    if query.trim() == self.search_query.trim() {
                        self.server_search_loading = false;
                        self.server_search_query = query;
                        self.server_search_results = items;
                    }
                }
                WorkerEvent::ForwardAttachmentsExported { message_id, paths } => {
                    if self.pending_forward_msg_id == Some(message_id) {
                        let mut added = 0usize;
                        for path in paths {
                            if !self.compose_attachments.contains(&path) {
                                self.compose_attachments.push(path);
                                added += 1;
                            }
                        }
                        if added > 0 {
                            self.status_line =
                                SharedString::from(format!("已自动携带原邮件 {added} 个附件"));
                        }
                    }
                }
                WorkerEvent::UnifiedViewLoaded { view, page } => {
                    self.loading_more_local = false;
                    if self.current_view.unified() == Some(view) {
                        self.merge_loaded_page(page, cx);
                    }
                }
                WorkerEvent::TagsLoaded { tags } => {
                    self.available_tags = tags;
                }
                WorkerEvent::CachedAttachmentsLoaded { items } => {
                    self.gallery_items = items.clone();
                    if let Some(ref g) = self.gallery {
                        g.update(cx, |state, cx| {
                            state.set_items(items.clone());
                            cx.notify();
                        });
                    }
                }
                WorkerEvent::KnownContactsLoaded { items } => {
                    if self.contacts_query.trim().is_empty() || self.known_contacts.is_empty() {
                        self.known_contacts = items.clone();
                    }
                    if let Some(ref c) = self.contacts {
                        c.update(cx, |state, cx| {
                            state.set_contacts(items);
                            cx.notify();
                        });
                    }
                }
                // F-4：草稿落库回执——绑定新分配的本地草稿 ID，自动保存时显示提示。
                WorkerEvent::DraftSaved { draft_id } => {
                    if self.has_active_compose() && draft_id > 0 {
                        if self.compose_draft_id.is_none() {
                            self.compose_draft_id = Some(draft_id);
                            if self.compose_autosave_pending {
                                self.compose_autosaved_new = true;
                            }
                        }
                        if self.compose_autosave_pending {
                            let now = chrono::Local::now().format("%H:%M").to_string();
                            self.compose_autosaved_at = Some(SharedString::from(now));
                        }
                    }
                    self.compose_autosave_pending = false;
                    cx.notify();
                }
                // F-5：批量导入回执（`KnownContactsLoaded` 已先行刷新列表）。
                WorkerEvent::ContactsImported { added, skipped } => {
                    self.status_line = SharedString::from(format!(
                        "联系人导入完成：新增 {added} 个，跳过 {skipped} 个（重复或无效）"
                    ));
                    cx.notify();
                }
                WorkerEvent::ThreadChildrenLoaded {
                    head_message_id,
                    children,
                    ..
                } => {
                    self.thread_children_cache.insert(head_message_id, children);
                    cx.notify();
                }
                WorkerEvent::MessageTagsUpdated {
                    message_id, tags, ..
                } => {
                    for m in &mut self.current_page.items {
                        if m.id == message_id {
                            m.tags = tags.clone();
                        }
                    }
                    self.status_line = SharedString::from("已更新邮件标签");
                    cx.notify();
                }
                WorkerEvent::MessageSnoozed {
                    message_id,
                    until_ts,
                    ..
                } => {
                    for m in &mut self.current_page.items {
                        if m.id == message_id {
                            m.snoozed_until_ts = until_ts;
                        }
                    }
                    self.status_line = SharedString::from(if until_ts > 0 {
                        "已设置稍后提醒"
                    } else {
                        "已取消稍后提醒"
                    });
                    cx.notify();
                }
                WorkerEvent::MessageCopied {
                    target_folder_path, ..
                } => {
                    self.status_line =
                        SharedString::from(format!("已复制邮件到 {target_folder_path}"));
                    cx.notify();
                }
                // F-6：附件总览图片预览字节到达（同时回填 WP-09 底部附件抽屉缩略图与大图预览）。
                WorkerEvent::AttachmentBytesFetched {
                    attachment_id,
                    filename,
                    mime_type,
                    bytes,
                } => {
                    if let Some(fmt) = attachment_drawer::image_format_for_mime(&mime_type) {
                        let image = Arc::new(Image::from_bytes(fmt, bytes.clone()));
                        self.attachment_thumb_cache
                            .insert(attachment_id, image.clone());
                        if let Some((prev_id, _, ref mut slot)) = self.drawer_image_preview {
                            if prev_id == attachment_id {
                                *slot = Some(image);
                            }
                        }
                    }
                    if let Some(ref g) = self.gallery {
                        let mime_c = mime_type.clone();
                        let bytes_c = bytes.clone();
                        g.update(cx, |state, cx| {
                            state.set_thumbnail(attachment_id, &mime_c, bytes_c);
                            cx.notify();
                        });
                    }
                    if self.gallery_preview_pending == Some(attachment_id) {
                        self.gallery_preview_pending = None;
                        if let Some(ref g) = self.gallery {
                            g.update(cx, |state, cx| {
                                state.set_preview_image(&filename, &mime_type, bytes);
                                cx.notify();
                            });
                        }
                    }
                }
                WorkerEvent::AttachmentDownloaded {
                    message_id,
                    attachment_id,
                    size_bytes,
                } => {
                    if let Some(ref mut detail) = self.selected_message {
                        if detail.id == message_id {
                            for att in &mut detail.attachments {
                                if att.id == attachment_id {
                                    att.downloaded = true;
                                    att.size_bytes = size_bytes;
                                }
                            }
                        }
                        for tm in &mut detail.thread_messages {
                            if tm.id == message_id {
                                for att in &mut tm.attachments {
                                    if att.id == attachment_id {
                                        att.downloaded = true;
                                        att.size_bytes = size_bytes;
                                    }
                                }
                            }
                        }
                    }
                    for item in &mut self.gallery_items {
                        if item.id == attachment_id {
                            item.downloaded = true;
                            item.size_bytes = size_bytes;
                        }
                    }
                    if let Some(ref g) = self.gallery {
                        g.update(cx, |state, cx| {
                            state.mark_downloaded(attachment_id, size_bytes);
                            cx.notify();
                        });
                    }
                    cx.notify();
                }
                WorkerEvent::AttachmentStripped {
                    account_id: _,
                    message_id,
                    attachment_id,
                    attachments,
                } => {
                    self.stripping_attachment_ids.remove(&attachment_id);
                    if self.pending_strip_attachment_id == Some(attachment_id) {
                        self.pending_strip_attachment_id = None;
                    }
                    if let Some(ref mut sel) = self.selected_message {
                        if sel.id == message_id {
                            sel.attachments = attachments.clone();
                        }
                    }
                    if let Some(ref mut sel) = self.selected_message {
                        for tm in &mut sel.thread_messages {
                            if tm.id == message_id {
                                tm.attachments = attachments.clone();
                            }
                        }
                    }
                }
                WorkerEvent::AttachmentStripFailed {
                    account_id: _,
                    message_id: _,
                    attachment_id,
                    message,
                } => {
                    self.stripping_attachment_ids.remove(&attachment_id);
                    self.status_line = SharedString::from(format!("剥离附件失败：{message}"));
                }
            }
        }
        if folders_updated {
            if let Some(ref tray) = self.tray {
                tray.set_tooltip(tray::format_tray_tooltip(self.unified_unread_count()));
                if total_new_unread > 0 && self.prefs.desktop_notifications && !self.demo_mode {
                    let account_label = notify_account_id
                        .map(|id| self.account_email_for(id))
                        .filter(|s| !s.is_empty());
                    let (sender, subject) = match latest_unread_preview.as_ref() {
                        Some((s, subj)) => (Some(s.as_str()), Some(subj.as_str())),
                        None => (None, None),
                    };
                    let (title, body) = tray::format_new_mail_notification(
                        total_new_unread,
                        sender,
                        subject,
                        account_label,
                        self.prefs.notification_show_content,
                    );
                    tray.show_notification(&title, &body);
                }
            }
        }
        cx.notify();
    }

    fn merge_loaded_page(&mut self, page: MailPage, cx: &mut Context<Self>) {
        if page.offset > 0 && page.offset == self.current_page.items.len() {
            let existing: HashSet<u32> = self.current_page.items.iter().map(|m| m.id).collect();
            for item in page.items {
                if !existing.contains(&item.id) {
                    self.current_page.items.push(item);
                }
            }
            self.current_page.total = page.total;
        } else {
            self.current_page = page;
            // 若后台同步推回了默认首屏（如 100 条），而用户此前已向下滚动加载了更多条，
            // 则按当前 loaded_limit 补刷一次以保持已展开列表不回弹。
            if self.loaded_limit > self.prefs.page_size
                && self.current_page.items.len() < self.loaded_limit
                && self.current_page.items.len() < self.current_page.total
            {
                self.reload_current_page(cx);
            }
        }
        if self.selected_message.is_none() {
            if let Some(first) = self.current_page.items.first().cloned() {
                // L-6：阅读器关闭或设为仅手动标已读时，首屏自动选中不立即标已读。
                let mark_now = first.unread
                    && self.reader_position != ReaderPosition::Hidden
                    && self.prefs.read_mark_mode == "shown";
                if let Some((detail, true)) = self.try_load_cached_detail(first.id) {
                    self.set_selected_message(detail);
                    self.error_banner = None;
                    if mark_now {
                        self.mark_message_seen_now(
                            first.account_id,
                            first.id,
                            first.uid,
                            &first.folder_path,
                        );
                    }
                } else {
                    self.show_message_loading(first.id);
                    self.request_message_detail(
                        first.account_id,
                        first.id,
                        first.uid,
                        mark_now,
                        &first.folder_path,
                    );
                }
            }
        }
    }

    pub(crate) fn reset_list_pagination(&mut self) {
        self.loaded_limit = self.prefs.page_size;
        self.loading_more_local = false;
        self.loading_more_remote = false;
        self.selected_message_ids.clear();
        self.list_scroll_handle.set_offset(point(px(0.0), px(0.0)));
    }

    pub(crate) fn reload_current_page(&mut self, cx: &mut Context<Self>) {
        let limit = self.loaded_limit.max(self.prefs.page_size);
        let cross_folder_search =
            !self.search_query.trim().is_empty() && self.search_scope == SearchScope::AllFolders;
        if self.demo_mode {
            // 演示模式：支持跨文件夹搜索、多条件快捷筛选与 6 种排序。
            self.current_page = if cross_folder_search {
                self.demo_store.search_all_folders(
                    &self.search_query,
                    self.quick_filters,
                    self.list_sort,
                    0,
                    limit,
                )
            } else {
                match self.current_view {
                    CurrentView::Unread => self.demo_store.list_unread_page_with_options(
                        &self.search_query,
                        self.quick_filters,
                        self.list_sort,
                        0,
                        limit,
                    ),
                    CurrentView::Starred => self.demo_store.list_starred_page_with_options(
                        &self.search_query,
                        self.quick_filters,
                        self.list_sort,
                        0,
                        limit,
                    ),
                    other => {
                        let folder_id = match other {
                            CurrentView::UnifiedInbox | CurrentView::AccountFolder => {
                                self.selected_folder_id
                            }
                            CurrentView::Sent => DEMO_SENT_ID,
                            CurrentView::Drafts => DEMO_DRAFTS_ID,
                            CurrentView::Archive => DEMO_ARCHIVE_ID,
                            CurrentView::Unread
                            | CurrentView::Starred
                            | CurrentView::Junk
                            | CurrentView::Trash
                            | CurrentView::Snoozed => DEMO_INBOX_ID,
                        };
                        self.demo_store.list_messages_page_with_options(
                            folder_id,
                            &self.search_query,
                            self.quick_filters,
                            self.list_sort,
                            0,
                            limit,
                        )
                    }
                }
            };
            cx.notify();
            return;
        }
        // 本地草稿视图：直接由渲染层从 `self.drafts` 合成，无需 worker 请求。
        if self.current_view == CurrentView::Drafts && !cross_folder_search {
            cx.notify();
            return;
        }
        self.send_page_request(0, limit);
    }

    fn send_page_request(&self, offset: usize, limit: usize) {
        let quick_filters = self.quick_filters;
        let sort = self.list_sort;
        let cross_folder_search =
            !self.search_query.trim().is_empty() && self.search_scope == SearchScope::AllFolders;
        if let Some(ref w) = self.worker {
            if cross_folder_search {
                w.send(MailRequest::SearchAllFolders {
                    query: self.search_query.clone(),
                    quick_filters,
                    sort,
                    offset,
                    limit,
                });
                return;
            }
            match self.current_view {
                CurrentView::UnifiedInbox => {
                    w.send(MailRequest::LoadUnifiedInbox {
                        query: self.search_query.clone(),
                        filter: MessageFilter::All,
                        quick_filters,
                        sort,
                        offset,
                        limit,
                    });
                }
                CurrentView::Unread
                | CurrentView::Starred
                | CurrentView::Sent
                | CurrentView::Archive
                | CurrentView::Junk
                | CurrentView::Trash
                | CurrentView::Snoozed => {
                    if let Some(view) = self.current_view.unified() {
                        w.send(MailRequest::LoadUnifiedView {
                            view,
                            query: self.search_query.clone(),
                            filter: MessageFilter::All,
                            quick_filters,
                            sort,
                            offset,
                            limit,
                        });
                    }
                }
                CurrentView::AccountFolder => {
                    w.send(MailRequest::LoadMessagesPage {
                        folder_id: self.selected_folder_id,
                        query: self.search_query.clone(),
                        filter: MessageFilter::All,
                        quick_filters,
                        sort,
                        offset,
                        limit,
                    });
                }
                CurrentView::Drafts => {}
            }
        }
    }

    /// 关联 GPUI 主窗口 HWND 到系统托盘（用于隐藏/恢复主窗口）。
    pub(crate) fn attach_main_hwnd(&self, hwnd: usize) {
        if let Some(ref tray) = self.tray {
            tray.set_main_hwnd(hwnd);
        }
    }

    /// 销毁系统托盘图标（真正退出前调用，避免托盘残留幽灵图标）。
    pub(crate) fn shutdown_tray(&self) {
        if let Some(ref tray) = self.tray {
            tray.shutdown();
        }
    }

    /// 持久化当前窗口尺寸与最大化状态到 `preferences.toml`（顺带写入三栏宽度，
    /// 覆盖拖拽防抖未及落盘的情形，如拖拽后立即关闭 / 隐藏窗口）。
    pub(crate) fn save_window_geometry(&mut self, window: &Window) {
        let geometry_changed = capture_window_geometry(&mut self.prefs, window.window_bounds());
        let layout_changed = self.layout_save_deadline.take().is_some();
        if geometry_changed || layout_changed {
            self.prefs.sidebar_width = Some(self.sidebar_width);
            self.prefs.list_width = Some(self.list_width);
            self.prefs.list_below_height = Some(self.list_below_height);
            let _ = config::save_preferences_in(&config::data_dir(), &self.prefs);
        }
    }

    /// 顶栏最小化按钮动作：偏好 `minimize_to_tray` 为 true 时隐藏到托盘，否则常规最小化。
    pub(crate) fn minimize_window_or_tray(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
        self.save_window_geometry(window);
        if self.prefs.minimize_to_tray {
            if let Some(ref tray) = self.tray {
                if tray.hide_main_window() {
                    return;
                }
            }
        }
        window.minimize_window();
    }

    /// 消费 Windows 系统托盘菜单 / 左键单击事件。
    fn handle_tray_command(&mut self, cmd: TrayCommand, cx: &mut Context<Self>) {
        match cmd {
            TrayCommand::ShowWindow => {
                if let Some(ref tray) = self.tray {
                    tray.show_main_window();
                }
            }
            TrayCommand::NewCompose => {
                if let Some(ref tray) = self.tray {
                    tray.show_main_window();
                }
                // 本方法运行在 `HylkiApp::update` 内部（托盘轮询任务），此处再
                // `entity.update` 会对同一实体重复租借并 panic，故延后到下一 tick。
                let handle = self.window_handle;
                let entity = cx.entity();
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, cx| {
                        entity.update(cx, |this, cx| {
                            this.active_modal = ActiveModal::None;
                            this.main_view = MainContentView::Mail;
                            this.start_new_compose(window, cx);
                        });
                    });
                });
            }
            TrayCommand::SyncAll => {
                self.refresh_all(cx);
            }
            TrayCommand::Quit => {
                self.quit_app(cx);
            }
        }
    }

    /// 彻底退出应用（菜单「退出 Hylki」或托盘右键「退出 Hylki」入口）。
    pub(crate) fn quit_app(&mut self, cx: &mut Context<Self>) {
        let mut bounds_snapshot = None;
        let _ = self.window_handle.update(cx, |_, window, _| {
            bounds_snapshot = Some(window.window_bounds());
        });
        if let Some(wb) = bounds_snapshot {
            if capture_window_geometry(&mut self.prefs, wb) {
                let _ = config::save_preferences_in(&config::data_dir(), &self.prefs);
            }
        }
        // L-2：退出前把未及防抖落盘的三栏宽度一并写入。
        if self.layout_save_deadline.take().is_some() {
            self.persist_layout_prefs();
        }
        self.quitting = true;
        if self.pending_send.as_ref().is_some_and(|p| p.enqueue_failed) {
            self.quitting = false;
            self.update.cancel_install();
            self.status_line = SharedString::from("请先恢复未保存的邮件，再退出");
            cx.notify();
            return;
        }
        if !self.demo_mode
            && self
                .pending_send
                .as_ref()
                .is_some_and(|p| p.outbox_id.is_none())
        {
            self.status_line = SharedString::from("正在保存待发送邮件，完成后退出…");
            cx.notify();
            return;
        }
        if self.pending_send.as_ref().is_some_and(|p| p.undo_requested) {
            self.status_line = SharedString::from("正在完成撤销，然后退出…");
            cx.notify();
            return;
        }
        if self.quit_preparing {
            return;
        }
        if let Some(ref worker) = self.worker {
            // 主线程同步落盘：把仍排队的发件箱记录标记为到期后立即退出。
            // 不再异步等待 worker 的 QuitPrepared——单线程 worker 可能正执行
            // 耗时的同步或超时重连，异步请求会排在队尾导致关窗毫无反应。
            if worker.prepare_quit_now().is_err() && self.pending_send.is_some() {
                self.quitting = false;
                self.update.cancel_install();
                self.error_banner = Some(SharedString::from("退出时保存发件队列失败，请重试"));
                self.status_line = SharedString::from("退出时保存发件队列失败，请重试");
                cx.notify();
                return;
            }
        }
        if let Some((info, bytes)) = self.update.pending_install.take() {
            if let Err(error) = hylki_mail_core::update::install(&info, &bytes) {
                self.quitting = false;
                self.update.downloading = false;
                self.update.status = None;
                self.update.error = Some(SharedString::from(error));
                cx.notify();
                return;
            }
        }
        self.shutdown_tray();
        cx.quit();
    }

    /// 切换左栏统一导航视图（收件箱 / 星标 / 已发送 / 草稿 / 归档）。
    pub(crate) fn select_unified_view(&mut self, view: CurrentView, cx: &mut Context<Self>) {
        // P3：浮层侧栏里点选统一视图即导航完成，收起浮层。
        self.close_sidebar_peek();
        self.current_view = view;
        self.reset_list_pagination();
        self.selected_message = None;
        self.right_mode = RightPaneMode::Reader;
        self.main_view = MainContentView::Mail;
        if self.demo_mode {
            // 演示模式下统一视图映射到对应的演示文件夹。
            self.selected_folder_id = match view {
                CurrentView::UnifiedInbox
                | CurrentView::Unread
                | CurrentView::Starred
                | CurrentView::Junk
                | CurrentView::Trash
                | CurrentView::Snoozed => DEMO_INBOX_ID,
                CurrentView::Sent => DEMO_SENT_ID,
                CurrentView::Drafts => DEMO_DRAFTS_ID,
                CurrentView::Archive => DEMO_ARCHIVE_ID,
                CurrentView::AccountFolder => self.selected_folder_id,
            };
        }
        self.remember_last_location(None);
        self.reload_current_page(cx);
        cx.notify();
    }

    /// WP-07 侧边栏快捷入口（筛选文件夹 / 智能文件夹 / 标签）：
    /// 写入搜索词并切到全部文件夹搜索，立即刷新列表。
    pub(crate) fn apply_sidebar_query_shortcut(
        &mut self,
        query: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_sidebar_peek();
        self.main_view = MainContentView::Mail;
        self.right_mode = RightPaneMode::Reader;
        self.current_view = CurrentView::UnifiedInbox;
        self.search_scope = SearchScope::AllFolders;
        self.search_query = query.clone();
        self.search_input
            .update(cx, |s, cx| s.set_value(query, window, cx));
        self.reset_list_pagination();
        self.selected_message = None;
        self.reload_current_page(cx);
        cx.notify();
    }

    /// WP-07 侧边栏右键「新建子文件夹…」。
    pub(crate) fn begin_create_subfolder(
        &mut self,
        account_id: u32,
        parent_path: String,
        delimiter: char,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.folder_action = Some(FolderActionPrompt::CreateSubfolder {
            account_id,
            parent_path,
            delimiter,
        });
        self.folder_action_input.update(cx, |s, cx| {
            s.set_value("", window, cx);
            s.focus(window, cx);
        });
        cx.notify();
    }

    /// WP-07 侧边栏右键「重命名…」。
    pub(crate) fn begin_rename_folder(
        &mut self,
        account_id: u32,
        folder_id: u32,
        old_path: String,
        current_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.folder_action = Some(FolderActionPrompt::RenameFolder {
            account_id,
            folder_id,
            old_path,
            current_name: current_name.clone(),
        });
        self.folder_action_input.update(cx, |s, cx| {
            s.set_value(current_name, window, cx);
            s.focus(window, cx);
        });
        cx.notify();
    }

    /// WP-07 侧边栏右键「删除…」。
    pub(crate) fn begin_delete_folder(
        &mut self,
        account_id: u32,
        folder_id: u32,
        folder_path: String,
        folder_name: String,
        cx: &mut Context<Self>,
    ) {
        self.folder_action = Some(FolderActionPrompt::DeleteFolder {
            account_id,
            folder_id,
            folder_path,
            folder_name,
        });
        cx.notify();
    }

    pub(crate) fn cancel_folder_action(&mut self, cx: &mut Context<Self>) {
        self.folder_action = None;
        cx.notify();
    }

    pub(crate) fn confirm_folder_action(&mut self, cx: &mut Context<Self>) {
        let Some(action) = self.folder_action.take() else {
            return;
        };
        let input_val = self.folder_action_input.read(cx).value().trim().to_string();
        if let Some(ref w) = self.worker {
            match action {
                FolderActionPrompt::CreateSubfolder {
                    account_id,
                    parent_path,
                    delimiter,
                } => {
                    if !input_val.is_empty() {
                        let folder_path = if parent_path.is_empty() {
                            input_val
                        } else {
                            format!("{parent_path}{delimiter}{input_val}")
                        };
                        w.send(MailRequest::CreateFolder {
                            account_id,
                            folder_path,
                        });
                    }
                }
                FolderActionPrompt::RenameFolder {
                    account_id,
                    folder_id,
                    old_path,
                    current_name,
                } => {
                    if !input_val.is_empty() && input_val != current_name {
                        w.send(MailRequest::RenameFolder {
                            account_id,
                            folder_id,
                            old_path,
                            new_name: input_val,
                        });
                    }
                }
                FolderActionPrompt::DeleteFolder {
                    account_id,
                    folder_id,
                    folder_path,
                    ..
                } => {
                    w.send(MailRequest::DeleteFolder {
                        account_id,
                        folder_id,
                        folder_path,
                    });
                }
            }
        }
        cx.notify();
    }

    pub(crate) fn select_folder(
        &mut self,
        account_id: u32,
        folder_id: u32,
        folder_path: String,
        cx: &mut Context<Self>,
    ) {
        // P3：浮层侧栏里点选文件夹即导航完成，收起浮层。
        self.close_sidebar_peek();
        self.selected_folder_id = folder_id;
        self.current_view = CurrentView::AccountFolder;
        self.reset_list_pagination();
        self.selected_message = None;
        self.right_mode = RightPaneMode::Reader;
        self.main_view = MainContentView::Mail;

        if self.demo_mode {
            self.current_page = self.demo_store.list_messages_page_with_options(
                folder_id,
                &self.search_query,
                self.quick_filters,
                self.list_sort,
                0,
                self.prefs.page_size,
            );
            let first_detail = self
                .current_page
                .items
                .first()
                .and_then(|m| self.demo_message_detail(m.id));
            if let Some(d) = first_detail {
                self.set_selected_message(d);
            } else {
                self.selected_message = None;
            }
            cx.notify();
            return;
        }

        self.remember_last_location(Some(account_id));
        self.reload_current_page(cx);
        if let Some(ref w) = self.worker {
            w.send(MailRequest::SyncFolder {
                account_id,
                folder_id,
                folder_path,
            });
        }
        cx.notify();
    }

    fn try_load_cached_detail(&self, message_id: u32) -> Option<(MessageDetail, bool)> {
        if self.demo_mode {
            return None;
        }
        self.worker.as_ref().and_then(|w| {
            w.try_load_cached_message(message_id, self.prefs.thread_newest_first)
                .ok()
                .flatten()
        })
    }

    #[cfg(windows)]
    pub(crate) fn ensure_reader_webview(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<std::rc::Rc<crate::webview_host::WebViewHost>> {
        if let Some(host) = &self.reader_webview {
            return Some(host.clone());
        }
        let host = crate::webview_host::WebViewHost::for_window(window, Some(window.to_async(cx)))?;
        let (tx, mut rx) = futures::channel::mpsc::channel::<()>(16);
        host.set_notifier(tx);
        let host = std::rc::Rc::new(host);
        self.reader_webview = Some(host.clone());
        cx.spawn(async move |this, cx| {
            use futures::StreamExt as _;
            let mut previous = None;
            while rx.next().await.is_some() {
                if this
                    .update(cx, |this, cx| {
                        let Some(host) = this.reader_webview.clone() else {
                            return;
                        };
                        let status = Some(host.load_status());
                        if status != previous {
                            previous = status;
                            cx.notify();
                        }
                        if this.active_modal == ActiveModal::None
                            && !this.reader_find_open
                            && !this.reader_popup_open
                        {
                            for link in host.take_links() {
                                this.open_mail_link(&link, cx);
                            }
                        } else {
                            let _ = host.take_links();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        Some(host)
    }

    pub(crate) fn select_message(
        &mut self,
        account_id: u32,
        message_id: u32,
        uid: u32,
        unread: bool,
        folder_path: &str,
        cx: &mut Context<Self>,
    ) {
        // P3：选中邮件即导航完成，收起浮层侧栏（浮层打开时列表仍可点选）。
        self.close_sidebar_peek();
        self.right_mode = RightPaneMode::Reader;
        self.read_mark_generation = self.read_mark_generation.wrapping_add(1);
        let gen = self.read_mark_generation;

        // L-6：阅读器关闭时点击邮件仅选中不展示（对齐原版 select_without_reader 语义），
        // 状态栏提示如何重新打开；正文仍拉取以支撑回复 / 复制等动作，但不自动标已读。
        let reader_hidden = self.reader_position == ReaderPosition::Hidden;
        if reader_hidden {
            self.status_line =
                SharedString::from("阅读器已关闭：邮件已选中，按 Ctrl+Alt+1 重新打开");
        }

        let mark_mode = self.prefs.read_mark_mode.as_str();
        let should_mark_now = unread && !reader_hidden && mark_mode == "shown";
        let should_mark_delay = unread && !reader_hidden && mark_mode == "delay";

        if self.demo_mode {
            if let Some(detail) = self.demo_message_detail(message_id) {
                self.set_selected_message(detail);
            } else {
                self.selected_message = None;
            }
            if should_mark_now {
                self.mark_message_seen_now(account_id, message_id, uid, folder_path);
            } else if should_mark_delay {
                self.schedule_delayed_read_mark(
                    gen,
                    account_id,
                    message_id,
                    uid,
                    folder_path.to_string(),
                    cx,
                );
            }
            cx.notify();
            return;
        }
        if let Some((detail, true)) = self.try_load_cached_detail(message_id) {
            self.set_selected_message(detail);
            self.error_banner = None;
            if should_mark_now {
                self.mark_message_seen_now(account_id, message_id, uid, folder_path);
            } else if should_mark_delay {
                self.schedule_delayed_read_mark(
                    gen,
                    account_id,
                    message_id,
                    uid,
                    folder_path.to_string(),
                    cx,
                );
            }
            cx.notify();
            return;
        }
        self.show_message_loading(message_id);
        self.request_message_detail(account_id, message_id, uid, should_mark_now, folder_path);
        if should_mark_delay {
            self.schedule_delayed_read_mark(
                gen,
                account_id,
                message_id,
                uid,
                folder_path.to_string(),
                cx,
            );
        }
        cx.notify();
    }

    fn schedule_delayed_read_mark(
        &self,
        generation: u64,
        account_id: u32,
        message_id: u32,
        uid: u32,
        folder_path: String,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            let _ = this.update(cx, |app, cx| {
                if app.read_mark_generation == generation
                    && app
                        .selected_message
                        .as_ref()
                        .is_some_and(|m| m.id == message_id)
                {
                    app.mark_message_seen_now(account_id, message_id, uid, &folder_path);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn mark_message_seen_now(
        &mut self,
        account_id: u32,
        message_id: u32,
        uid: u32,
        folder_path: &str,
    ) {
        if !self.demo_mode {
            if let Some(ref w) = self.worker {
                w.send(MailRequest::SetSeen {
                    account_id,
                    folder_path: folder_path.to_string(),
                    message_id,
                    uid,
                    seen: true,
                });
            }
        }
        if let Some(item) = self
            .current_page
            .items
            .iter_mut()
            .find(|m| m.id == message_id)
        {
            item.unread = false;
        }
    }

    fn request_message_detail(
        &mut self,
        account_id: u32,
        message_id: u32,
        uid: u32,
        mark_seen: bool,
        folder_path: &str,
    ) {
        if let Some(ref w) = self.worker {
            self.pending_body_message_id = Some(message_id);
            w.send(MailRequest::LoadMessageBody {
                account_id,
                message_id,
            });
            if mark_seen {
                self.mark_message_seen_now(account_id, message_id, uid, folder_path);
            }
        }
    }

    /// 当前阅读邮件在列表中的未读状态（默认已读）。
    pub(crate) fn selected_message_unread(&self) -> bool {
        let Some(detail) = self.selected_message.as_ref() else {
            return false;
        };
        self.current_page
            .items
            .iter()
            .find(|m| m.id == detail.id)
            .map(|m| m.unread)
            .unwrap_or(false)
    }

    /// 标记已读 / 未读（双向切换，复用现有 `MailRequest::SetSeen`）。
    pub(crate) fn toggle_message_seen(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.selected_message.clone() else {
            return;
        };
        // 当前未读则标为已读（seen = true），当前已读则标为未读（seen = false）。
        let next_seen = self.selected_message_unread();
        if let Some(item) = self
            .current_page
            .items
            .iter_mut()
            .find(|m| m.id == detail.id)
        {
            item.unread = !next_seen;
        }
        if !self.demo_mode {
            if let Some(ref w) = self.worker {
                w.send(MailRequest::SetSeen {
                    account_id: detail.account_id,
                    folder_path: detail.folder_path.clone(),
                    message_id: detail.id,
                    uid: detail.uid,
                    seen: next_seen,
                });
            }
        }
        self.status_line = SharedString::from(if next_seen {
            "已标记为已读"
        } else {
            "已标记为未读"
        });
        cx.notify();
    }

    /// 切换阅读器「原样 HTML / 纯文本」显示模式。
    pub(crate) fn toggle_reader_plain_mode(&mut self, cx: &mut Context<Self>) {
        self.reader_prefer_plain = !self.reader_prefer_plain;
        self.status_line = SharedString::from(if self.reader_prefer_plain {
            "阅读器已切换为纯文本模式"
        } else {
            "阅读器已切换为富文本 HTML 模式"
        });
        cx.notify();
    }

    /// 临时允许指定邮件加载远程图片。
    pub(crate) fn allow_remote_images_for(&mut self, message_id: u32, cx: &mut Context<Self>) {
        self.allowed_remote_image_ids.insert(message_id);
        cx.notify();
    }

    /// 总是信任并允许指定发件人加载远程图片（持久化到偏好设置）。
    pub(crate) fn trust_remote_sender(&mut self, sender: &str, cx: &mut Context<Self>) {
        let sender = sender.to_string();
        self.mutate_preferences(cx, |p| {
            p.allow_remote_sender(&sender);
        });
        cx.notify();
    }

    /// 按偏好顺序返回演示邮件详情。
    fn demo_message_detail(&self, message_id: u32) -> Option<MessageDetail> {
        let mut detail = self.demo_store.get_message_detail(message_id)?;
        if !self.prefs.thread_newest_first && detail.thread_messages.len() > 1 {
            detail.thread_messages.reverse();
        }
        Some(detail)
    }

    /// 立即用列表摘要构造占位详情，让列表高亮与阅读器在远端正文返回前先切换。
    fn show_message_loading(&mut self, message_id: u32) {
        if self
            .selected_message
            .as_ref()
            .is_some_and(|d| d.id == message_id)
        {
            return;
        }
        let Some(msg) = self.current_page.items.iter().find(|m| m.id == message_id) else {
            return;
        };
        self.selected_message_is_placeholder = true;
        self.selected_message = Some(MessageDetail {
            id: msg.id,
            account_id: msg.account_id,
            folder_id: msg.folder_id,
            folder_path: msg.folder_path.clone(),
            uid: msg.uid,
            rfc_message_id: String::new(),
            sender: msg.sender.clone(),
            recipients: msg.recipients.clone(),
            cc: String::new(),
            subject: msg.subject.clone(),
            date_ts: msg.date_ts,
            date_display: msg.date_display.clone(),
            body_text: String::new(),
            body_html: None,
            flagged: msg.flagged,
            attachments: Vec::new(),
            thread_messages: Vec::new(),
            unsubscribe_url: None,
            unsubscribe_one_click: false,
            auth_results: None,
        });
    }

    /// 更新当前选中邮件详情并初始化会话卡片折叠态 / 原文态。
    fn set_selected_message(&mut self, detail: MessageDetail) {
        // 占位详情（空正文 / 空会话）切到真实详情时必须按新会话重新初始化折叠态；
        // 已经是同一封完整邮件（重复点击 / 缓存命中）时保留现有折叠态。
        let was_pending = self.pending_body_message_id == Some(detail.id)
            && self.selected_message.as_ref().is_some_and(|prev| {
                prev.id == detail.id && prev.body_text.is_empty() && prev.thread_messages.is_empty()
            });
        let same_thread = !was_pending
            && self.selected_message.as_ref().is_some_and(|prev| {
                prev.id == detail.id || prev.thread_messages.iter().any(|m| m.id == detail.id)
            });
        if same_thread {
            // 同一会话内加载单张卡片正文：保留其他卡片状态，但确保刚加载的卡片处于展开态。
            self.collapsed_thread_cards.remove(&detail.id);
        } else {
            self.raw_source_message_ids.clear();
            // F-7：切换会话时清除未确认的退订横幅。
            self.pending_unsubscribe = None;
            if self.focus_mode && self.prefs.focus_reader_view {
                self.reader_prefer_plain = true;
            } else {
                match self.prefs.reader_default_mode.as_str() {
                    "on" => self.reader_prefer_plain = true,
                    "off" => self.reader_prefer_plain = false,
                    _ => {}
                }
            }
            self.collapsed_thread_cards = compute_default_collapsed_cards(
                &detail,
                self.prefs.thread_newest_first,
                self.prefs.effective_fold_messages_mode(),
            );
        }
        self.pending_body_message_id = None;
        self.selected_message_is_placeholder = false;
        self.selected_message = Some(detail);
    }

    /// 切换指定会话卡片的折叠/展开态。
    pub(crate) fn toggle_thread_card_collapsed(&mut self, msg_id: u32, cx: &mut Context<Self>) {
        if !self.collapsed_thread_cards.remove(&msg_id) {
            self.collapsed_thread_cards.insert(msg_id);
        }
        cx.notify();
    }

    /// 展开当前会话中的全部卡片。
    pub(crate) fn expand_all_thread_cards(&mut self, cx: &mut Context<Self>) {
        self.collapsed_thread_cards.clear();
        cx.notify();
    }

    /// 折叠当前会话中的较早卡片（保留最新一封展开）。
    pub(crate) fn collapse_older_thread_cards(&mut self, cx: &mut Context<Self>) {
        if let Some(ref detail) = self.selected_message {
            if detail.thread_messages.len() >= 2 {
                let newest_id = if self.prefs.thread_newest_first {
                    detail.thread_messages.first().map(|m| m.id)
                } else {
                    detail.thread_messages.last().map(|m| m.id)
                };
                self.collapsed_thread_cards = detail
                    .thread_messages
                    .iter()
                    .map(|m| m.id)
                    .filter(|&id| Some(id) != newest_id)
                    .collect();
            }
        }
        cx.notify();
    }

    /// 切换指定卡片的「查看邮件原文 / 返回正文」视图。
    pub(crate) fn toggle_raw_source(&mut self, msg_id: u32, cx: &mut Context<Self>) {
        if !self.raw_source_message_ids.remove(&msg_id) {
            self.raw_source_message_ids.insert(msg_id);
            self.collapsed_thread_cards.remove(&msg_id);
        }
        cx.notify();
    }

    /// 调整阅读器正文字号缩放（步进 ±10%，范围 50..=250）并持久化。
    pub(crate) fn adjust_reader_zoom(&mut self, delta: i32, cx: &mut Context<Self>) {
        let next = (self.prefs.reader_zoom_percent as i32 + delta).clamp(50, 250) as u32;
        self.set_reader_zoom(next, cx);
    }

    /// 设置阅读器正文字号缩放百分比（范围 50..=250）并持久化。
    pub(crate) fn set_reader_zoom(&mut self, percent: u32, cx: &mut Context<Self>) {
        let clamped = config::clamp_reader_zoom(percent);
        if self.prefs.reader_zoom_percent != clamped {
            self.prefs.reader_zoom_percent = clamped;
            if let Err(e) = config::save_preferences_in(&config::data_dir(), &self.prefs) {
                self.error_banner = Some(SharedString::from(e));
            }
            self.status_line = SharedString::from(format!("阅读字号已调整为 {clamped}%"));
            cx.notify();
        }
    }

    /// 复制任意文本到系统剪贴板（供单张会话卡片复制正文使用）。
    pub(crate) fn copy_text_to_clipboard(&mut self, text: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.status_line = SharedString::from("已复制邮件正文到剪贴板");
        cx.notify();
    }

    /// 复制当前阅读邮件的正文到系统剪贴板。
    pub(crate) fn copy_selected_body(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.selected_message.as_ref() else {
            return;
        };
        let text = detail.body_text.clone();
        self.copy_text_to_clipboard(text, cx);
    }

    pub(crate) fn toggle_flagged(&mut self, detail: &MessageDetail, cx: &mut Context<Self>) {
        let next_flagged = !detail.flagged;
        if let Some(item) = self
            .current_page
            .items
            .iter_mut()
            .find(|m| m.id == detail.id)
        {
            item.flagged = next_flagged;
        }
        if let Some(sel) = self.selected_message.as_mut() {
            sel.flagged = next_flagged;
        }
        if !self.demo_mode {
            if let Some(ref w) = self.worker {
                w.send(MailRequest::SetFlagged {
                    account_id: detail.account_id,
                    folder_path: detail.folder_path.clone(),
                    message_id: detail.id,
                    uid: detail.uid,
                    flagged: next_flagged,
                });
            }
        }
        cx.notify();
    }

    // ---- 迭代 2：归档 / 删除与列表行操作（方案文档五.3）----

    /// 归档 / 删除 / 移动 / 标垃圾完成后的公共收尾：清空正在显示的目标邮件并刷新当前列表；
    /// F-2：被移除邮件正显示在阅读态时，自动推进选中相邻下一封（末尾则上一封）并加载其正文。
    fn after_message_moved(&mut self, account_id: u32, message_id: u32, cx: &mut Context<Self>) {
        self.selected_message_ids.remove(&message_id);
        let was_displayed = self
            .selected_message
            .as_ref()
            .is_some_and(|m| m.account_id == account_id && m.id == message_id);
        // 先在旧列表中定位被移除邮件的下标（F-2），再刷新列表。
        let advance_to = if was_displayed {
            let len = self.current_page.items.len();
            self.current_page
                .items
                .iter()
                .position(|m| m.id == message_id)
                .and_then(|idx| next_index_after_removal(idx, len))
                .and_then(|next_idx| self.current_page.items.get(next_idx).cloned())
        } else {
            None
        };
        if was_displayed {
            self.selected_message = None;
            self.selected_message_is_placeholder = false;
        }
        // 先请求相邻邮件正文、后刷新列表：worker 按顺序处理，详情先回、页面后回，
        // 避免 merge_loaded_page 在 selected_message 仍为空时误选第一封。
        if let Some(next) = advance_to.as_ref() {
            self.select_message(
                next.account_id,
                next.id,
                next.uid,
                next.unread,
                &next.folder_path,
                cx,
            );
        }
        self.reload_current_page(cx);
    }

    /// 阅读态归档当前邮件（发 `MailRequest::ArchiveMessage`）。
    pub(crate) fn archive_selected(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.selected_message.clone() else {
            return;
        };
        self.archive_message(detail.account_id, detail.id, cx);
    }

    /// 阅读态删除当前邮件（发 `MailRequest::DeleteMessage`）。
    pub(crate) fn delete_selected(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.selected_message.clone() else {
            return;
        };
        self.delete_message(detail.account_id, detail.id, cx);
    }

    /// 阅读态标记当前邮件为垃圾邮件（发 `MailRequest::MarkSpam`）。
    pub(crate) fn mark_spam_selected(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.selected_message.clone() else {
            return;
        };
        self.mark_spam_message(detail.account_id, detail.id, cx);
    }

    /// 归档指定邮件（右键菜单入口）。
    pub(crate) fn archive_message(
        &mut self,
        account_id: u32,
        message_id: u32,
        cx: &mut Context<Self>,
    ) {
        if self.demo_mode {
            return;
        }
        if let Some(ref w) = self.worker {
            self.pending_undo = self.capture_undo_record(account_id, message_id, "已归档");
            self.status_line = SharedString::from("正在归档邮件...");
            w.send(MailRequest::ArchiveMessage {
                account_id,
                message_id,
            });
        }
        cx.notify();
    }

    /// 删除指定邮件（右键菜单入口）。
    pub(crate) fn delete_message(
        &mut self,
        account_id: u32,
        message_id: u32,
        cx: &mut Context<Self>,
    ) {
        if self.demo_mode {
            return;
        }
        if let Some(ref w) = self.worker {
            self.pending_undo = self.capture_undo_record(account_id, message_id, "已删除");
            self.status_line = SharedString::from("正在删除邮件...");
            w.send(MailRequest::DeleteMessage {
                account_id,
                message_id,
            });
        }
        cx.notify();
    }

    /// 标记指定邮件为垃圾邮件（移动到账号 Junk / Spam 文件夹）。
    pub(crate) fn mark_spam_message(
        &mut self,
        account_id: u32,
        message_id: u32,
        cx: &mut Context<Self>,
    ) {
        if self.demo_mode {
            return;
        }
        if let Some(ref w) = self.worker {
            self.pending_undo =
                self.capture_undo_record(account_id, message_id, "已标记为垃圾邮件");
            self.status_line = SharedString::from("正在标记为垃圾邮件...");
            w.send(MailRequest::MarkSpam {
                account_id,
                message_id,
            });
        }
        cx.notify();
    }

    /// 移动指定邮件到目标文件夹（顶栏下拉菜单 / 右键子菜单入口）。
    pub(crate) fn move_message(
        &mut self,
        account_id: u32,
        message_id: u32,
        target_folder_path: String,
        cx: &mut Context<Self>,
    ) {
        if self.demo_mode {
            return;
        }
        if let Some(ref w) = self.worker {
            self.pending_undo = self.capture_undo_record(account_id, message_id, "已移动");
            self.status_line = SharedString::from("正在移动邮件...");
            w.send(MailRequest::MoveMessage {
                account_id,
                message_id,
                target_folder_path,
            });
        }
        cx.notify();
    }

    // ---- B-2 行菜单挂起动作 / F-3 撤销 / .eml 导出 ----

    /// F-3：请求发出前从当前列表捕获撤销前半段信息（原文件夹路径；定位信息由成功回执补全）。
    fn capture_undo_record(
        &self,
        account_id: u32,
        message_id: u32,
        action_label: &str,
    ) -> Option<PendingUndo> {
        let msg = self
            .current_page
            .items
            .iter()
            .find(|m| m.id == message_id && m.account_id == account_id)?;
        Some(PendingUndo {
            account_id,
            message_id: msg.id,
            original_folder_path: msg.folder_path.clone(),
            action_label: SharedString::from(action_label.to_string()),
            at: Instant::now(),
        })
    }

    /// F-3：写入可撤销记录并安排 15 秒后自动清理（新记录覆盖旧记录）。
    fn set_last_move_undo(&mut self, record: UndoRecord, cx: &mut Context<Self>) {
        let at = record.at;
        self.last_move_undo = Some(record);
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(UNDO_TTL_SECS))
                .await;
            let _ = this.update(cx, |app, cx| {
                if app.last_move_undo.as_ref().is_some_and(|r| r.at == at) {
                    app.last_move_undo = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// F-3：撤销最近一次移动 / 删除 / 归档——邮件已不在本地缓存，走专用
    /// `UndoMessageMove`（服务端按 Message-ID 定位后移回原文件夹），不再记录新的撤销。
    pub(crate) fn undo_last_move(&mut self, cx: &mut Context<Self>) {
        let Some(record) = self.last_move_undo.take() else {
            return;
        };
        if let Some(ref w) = self.worker {
            self.status_line =
                SharedString::from(format!("正在撤销{}，移回原文件夹...", record.action_label));
            w.send(MailRequest::UndoMessageMove {
                account_id: record.account_id,
                message_id: record.message_id,
                rfc_message_id: record.rfc_message_id,
                current_folder_path: record.current_folder_path,
                original_folder_path: record.original_folder_path,
            });
        }
        cx.notify();
    }

    /// B-2：列表行菜单的回复 / 复制正文 / 查看原文入口。
    ///
    /// 详情已就绪（demo 或当前选中）时立即执行；否则挂起动作并按需加载正文。
    pub(crate) fn begin_row_action(
        &mut self,
        action: PendingRowAction,
        account_id: u32,
        message_id: u32,
        uid: u32,
        unread: bool,
        folder_path: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.demo_mode {
            if let Some(detail) = self.demo_message_detail(message_id) {
                self.right_mode = RightPaneMode::Reader;
                // 查看原文 / 回复针对当前显示的邮件：先切换选中再执行动作。
                self.set_selected_message(detail.clone());
                self.run_row_action(action, &detail, window, cx);
            }
            cx.notify();
            return;
        }
        if self
            .selected_message
            .as_ref()
            .is_some_and(|d| d.id == message_id)
        {
            if let Some(detail) = self.selected_message.clone() {
                self.run_row_action(action, &detail, window, cx);
            }
            cx.notify();
            return;
        }
        self.right_mode = RightPaneMode::Reader;
        self.pending_row_action = Some((message_id, action));
        self.request_message_detail(account_id, message_id, uid, unread, folder_path);
        cx.notify();
    }

    /// 执行已就绪的行菜单动作（B-2）。
    fn run_row_action(
        &mut self,
        action: PendingRowAction,
        detail: &MessageDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            PendingRowAction::Reply => {
                self.start_reply(detail, window, cx);
            }
            PendingRowAction::ReplyAll => {
                self.start_reply_all(detail, window, cx);
            }
            PendingRowAction::Forward => {
                self.start_forward(detail, window, cx);
            }
            PendingRowAction::EditAsNew => self.start_edit_as_new(detail, window, cx),
            PendingRowAction::CopyBody => {
                let text = detail.body_text.clone();
                self.copy_text_to_clipboard(text, cx);
            }
            PendingRowAction::ViewSource => {
                self.raw_source_message_ids.insert(detail.id);
                self.collapsed_thread_cards.remove(&detail.id);
                cx.notify();
            }
        }
    }

    /// 正文加载完成后执行挂起的行菜单动作（MessageBodyLoaded 回调入口）。
    fn finish_pending_row_action(
        &mut self,
        detail: &MessageDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((msg_id, action)) = self.pending_row_action.take() else {
            return;
        };
        if msg_id != detail.id {
            // 回执与挂起动作不匹配（用户切换了目标）：放回等待对应正文。
            self.pending_row_action = Some((msg_id, action));
            return;
        }
        self.run_row_action(action, detail, window, cx);
    }

    /// B-2：请求导出 .eml——真实账号异步抓取原文（导出中…），演示模式直接合成。
    pub(crate) fn request_export_eml(
        &mut self,
        account_id: u32,
        message_id: u32,
        uid: u32,
        folder_path: String,
        cx: &mut Context<Self>,
    ) {
        if self.pending_eml_export == Some(message_id) {
            return;
        }
        if self.demo_mode {
            match self.demo_message_detail(message_id) {
                Some(detail) => {
                    let bytes = compose_demo_eml(&detail);
                    self.export_eml_bytes(&detail.subject, bytes, cx);
                }
                None => {
                    self.status_line = SharedString::from("演示数据没有该邮件的原始正文，无法导出");
                }
            }
            cx.notify();
            return;
        }
        self.pending_eml_export = Some(message_id);
        self.status_line = SharedString::from("正在获取邮件原文...");
        if let Some(ref w) = self.worker {
            w.send(MailRequest::FetchRawMessage {
                account_id,
                message_id,
                uid,
                folder_path,
            });
        }
        cx.notify();
    }

    /// B-2：弹系统保存对话框并把 .eml 字节写出（文件名 = 过滤非法字符后的主题）。
    fn export_eml_bytes(&mut self, subject: &str, bytes: Vec<u8>, cx: &mut Context<Self>) {
        let filename = format!("{}.eml", sanitize_filename(subject));
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rfd::FileDialog::new()
                        .set_file_name(&filename)
                        .save_file()
                        .map(|dest| {
                            let result = std::fs::write(&dest, &bytes);
                            (dest, result)
                        })
                })
                .await;
            if let Some((dest, result)) = result {
                let _ = this.update(cx, |app, cx| {
                    match result {
                        Ok(()) => {
                            app.status_line =
                                SharedString::from(format!("已导出邮件原文至：{}", dest.display()))
                        }
                        Err(e) => {
                            app.error_banner =
                                Some(SharedString::from(format!("保存邮件原文失败: {e}")))
                        }
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// 查找账号的收件箱文件夹路径（B-2 Trash/Junk 恢复目标）。
    pub(crate) fn inbox_folder_path(&self, account_id: u32) -> Option<String> {
        self.folders_by_account
            .get(&account_id)?
            .iter()
            .find(|f| {
                f.path.eq_ignore_ascii_case("INBOX")
                    || f.name.eq_ignore_ascii_case("INBOX")
                    || f.name == "收件箱"
            })
            .map(|f| f.path.clone())
    }

    /// 判定指定文件夹是否为回收站 / 垃圾邮件文件夹（B-2 菜单语义切换）。
    pub(crate) fn folder_is_trash_or_junk(&self, account_id: u32, folder_id: u32) -> bool {
        self.folders_by_account
            .get(&account_id)
            .and_then(|list| list.iter().find(|f| f.id == folder_id))
            .is_some_and(|f| is_trash_or_junk_folder(&f.name, &f.path))
    }

    /// B-6：当前列表视图是否显示收件人（统一已发送 / 草稿视图 / 账号 Sent 文件夹）。
    pub(crate) fn list_view_shows_recipients(&self) -> bool {
        match self.current_view {
            CurrentView::Sent | CurrentView::Drafts => true,
            CurrentView::AccountFolder => self
                .folders_by_account
                .values()
                .flatten()
                .find(|f| f.id == self.selected_folder_id)
                .is_some_and(|f| is_sent_folder(&f.name, &f.path)),
            CurrentView::UnifiedInbox
            | CurrentView::Unread
            | CurrentView::Starred
            | CurrentView::Archive
            | CurrentView::Junk
            | CurrentView::Trash
            | CurrentView::Snoozed => false,
        }
    }

    // ---- 多选与批量操作 ----

    /// Ctrl+点击切换某封邮件的多选状态。
    pub(crate) fn toggle_message_selection(&mut self, message_id: u32, cx: &mut Context<Self>) {
        if self.selected_message_ids.is_empty() {
            if let Some(sel) = self.selected_message.as_ref() {
                if sel.id != message_id {
                    self.selected_message_ids.insert(sel.id);
                }
            }
        }
        if !self.selected_message_ids.remove(&message_id) {
            self.selected_message_ids.insert(message_id);
        }
        cx.notify();
    }

    /// 取消当前多选。
    pub(crate) fn clear_message_selection(&mut self, cx: &mut Context<Self>) {
        self.selected_message_ids.clear();
        cx.notify();
    }

    /// 批量将当前多选邮件标为已读。
    pub(crate) fn batch_mark_read(&mut self, cx: &mut Context<Self>) {
        let targets: Vec<MessageSummary> = self
            .current_page
            .items
            .iter()
            .filter(|m| self.selected_message_ids.contains(&m.id))
            .cloned()
            .collect();
        if targets.is_empty() {
            return;
        }
        for msg in &targets {
            if let Some(item) = self.current_page.items.iter_mut().find(|m| m.id == msg.id) {
                item.unread = false;
            }
            if !self.demo_mode {
                if let Some(ref w) = self.worker {
                    w.send(MailRequest::SetSeen {
                        account_id: msg.account_id,
                        folder_path: msg.folder_path.clone(),
                        message_id: msg.id,
                        uid: msg.uid,
                        seen: true,
                    });
                }
            }
        }
        self.status_line = SharedString::from(format!("已批量将 {} 封邮件标为已读", targets.len()));
        self.selected_message_ids.clear();
        cx.notify();
    }

    /// 批量加星标（若全部已加星则批量取消星标）。
    pub(crate) fn batch_toggle_flagged(&mut self, cx: &mut Context<Self>) {
        let targets: Vec<MessageSummary> = self
            .current_page
            .items
            .iter()
            .filter(|m| self.selected_message_ids.contains(&m.id))
            .cloned()
            .collect();
        if targets.is_empty() {
            return;
        }
        let next_flagged = !targets.iter().all(|m| m.flagged);
        for msg in &targets {
            if let Some(item) = self.current_page.items.iter_mut().find(|m| m.id == msg.id) {
                item.flagged = next_flagged;
            }
            if let Some(sel) = self.selected_message.as_mut() {
                if sel.id == msg.id {
                    sel.flagged = next_flagged;
                }
            }
            if !self.demo_mode {
                if let Some(ref w) = self.worker {
                    w.send(MailRequest::SetFlagged {
                        account_id: msg.account_id,
                        folder_path: msg.folder_path.clone(),
                        message_id: msg.id,
                        uid: msg.uid,
                        flagged: next_flagged,
                    });
                }
            }
        }
        self.status_line = SharedString::from(if next_flagged {
            format!("已批量为 {} 封邮件加星标", targets.len())
        } else {
            format!("已批量取消 {} 封邮件的星标", targets.len())
        });
        self.selected_message_ids.clear();
        cx.notify();
    }

    /// 批量归档当前多选邮件。
    pub(crate) fn batch_archive(&mut self, cx: &mut Context<Self>) {
        if self.demo_mode {
            return;
        }
        let targets: Vec<(u32, u32)> = self
            .current_page
            .items
            .iter()
            .filter(|m| self.selected_message_ids.contains(&m.id))
            .map(|m| (m.account_id, m.id))
            .collect();
        if targets.is_empty() {
            return;
        }
        if let Some(ref w) = self.worker {
            for (account_id, message_id) in &targets {
                w.send(MailRequest::ArchiveMessage {
                    account_id: *account_id,
                    message_id: *message_id,
                });
            }
        }
        self.status_line = SharedString::from(format!("正在批量归档 {} 封邮件...", targets.len()));
        self.selected_message_ids.clear();
        cx.notify();
    }

    /// 批量删除当前多选邮件。
    pub(crate) fn batch_delete(&mut self, cx: &mut Context<Self>) {
        if self.demo_mode {
            return;
        }
        let targets: Vec<(u32, u32)> = self
            .current_page
            .items
            .iter()
            .filter(|m| self.selected_message_ids.contains(&m.id))
            .map(|m| (m.account_id, m.id))
            .collect();
        if targets.is_empty() {
            return;
        }
        if let Some(ref w) = self.worker {
            for (account_id, message_id) in &targets {
                w.send(MailRequest::DeleteMessage {
                    account_id: *account_id,
                    message_id: *message_id,
                });
            }
        }
        self.status_line = SharedString::from(format!("正在批量删除 {} 封邮件...", targets.len()));
        self.selected_message_ids.clear();
        cx.notify();
    }

    /// 列表行「标记已读 / 未读」：真实账号走 `SetSeen`，演示模式仅本地切换。
    pub(crate) fn set_row_seen(
        &mut self,
        account_id: u32,
        message_id: u32,
        uid: u32,
        folder_path: String,
        seen: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(item) = self
            .current_page
            .items
            .iter_mut()
            .find(|m| m.id == message_id)
        {
            item.unread = !seen;
        }
        if self.demo_mode {
            cx.notify();
            return;
        }
        if let Some(ref w) = self.worker {
            w.send(MailRequest::SetSeen {
                account_id,
                folder_path,
                message_id,
                uid,
                seen,
            });
        }
        cx.notify();
    }

    /// 列表行「加星 / 取消星标」：真实账号走 `SetFlagged`，演示模式仅本地切换。
    pub(crate) fn set_row_flagged(
        &mut self,
        account_id: u32,
        message_id: u32,
        uid: u32,
        folder_path: String,
        flagged: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(item) = self
            .current_page
            .items
            .iter_mut()
            .find(|m| m.id == message_id)
        {
            item.flagged = flagged;
        }
        if let Some(sel) = self.selected_message.as_mut() {
            if sel.id == message_id {
                sel.flagged = flagged;
            }
        }
        if self.demo_mode {
            cx.notify();
            return;
        }
        if let Some(ref w) = self.worker {
            w.send(MailRequest::SetFlagged {
                account_id,
                folder_path,
                message_id,
                uid,
                flagged,
            });
        }
        cx.notify();
    }

    /// 删除本地草稿（草稿视图右键菜单入口，走现有 `DeleteLocalDraft`）。
    pub(crate) fn delete_row_draft(&mut self, draft_id: u32, cx: &mut Context<Self>) {
        if self.demo_mode {
            return;
        }
        if let Some(ref w) = self.worker {
            w.send(MailRequest::DeleteLocalDraft { draft_id });
        }
        cx.notify();
    }

    /// 把本地草稿合成为中栏列表行（草稿视图，真实账号模式）。
    pub(crate) fn draft_row_summary(&self, draft: &LocalDraft) -> MessageSummary {
        let subject = if draft.subject.trim().is_empty() {
            "(无主题草稿)".to_string()
        } else {
            draft.subject.clone()
        };
        let preview = draft
            .body_text
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .chars()
            .take(120)
            .collect::<String>();
        let owner_email = self.account_email_for(draft.account_id).to_string();
        MessageSummary {
            id: draft.id,
            account_id: draft.account_id,
            folder_id: 0,
            folder_path: String::new(),
            uid: 0,
            sender: if owner_email.is_empty() {
                "草稿".to_string()
            } else {
                format!("草稿 · {owner_email}")
            },
            subject,
            preview,
            date_ts: draft.updated_at,
            // 本地草稿不展示时钟时间（避免 UI 层引入时区处理）。
            date_display: String::new(),
            unread: false,
            flagged: false,
            has_attachment: !draft.attachment_paths.is_empty(),
            // B-6：草稿视图列表行显示「致：收件人」。
            recipients: draft.to_addr.clone(),
            thread_count: 1,
            importance: 0,
            tags: Vec::new(),
            answered: false,
            forwarded: false,
            snoozed_until_ts: 0,
        }
    }

    /// 展开 / 折叠 Cc / Bcc / Reply-To 输入行（方案文档页面 3 + WP-10-2）。
    pub(crate) fn toggle_compose_cc_bcc(&mut self, cx: &mut Context<Self>) {
        self.compose_show_cc_bcc = !self.compose_show_cc_bcc;
        cx.notify();
    }

    // ---- WP-10-2 发件人 From 下拉 + Reply-To 行 ----

    /// 当前写信账号自身的默认身份文本（`Name <addr>`，无显示名时只有地址）。
    pub(crate) fn compose_account_identity(&self) -> String {
        self.accounts
            .iter()
            .find(|a| a.id == self.compose_account_id)
            .map(|a| format_identity(&a.display_name, &a.email))
            .unwrap_or_default()
    }

    /// From 行显示的文本：自定义地址优先，否则账号默认身份。
    pub(crate) fn compose_from_display(&self) -> String {
        if self.compose_from_addr.trim().is_empty() {
            self.compose_account_identity()
        } else {
            self.compose_from_addr.clone()
        }
    }

    /// From 下拉候选：账号默认身份 + 该账号已配置的别名（按邮箱去重）。
    pub(crate) fn compose_from_choices(&self) -> Vec<ComposeFromChoice> {
        let Some(account) = self
            .accounts
            .iter()
            .find(|a| a.id == self.compose_account_id)
        else {
            return Vec::new();
        };
        let mut choices = vec![ComposeFromChoice {
            label: format_identity(&account.display_name, &account.email),
            from_addr: String::new(),
            reply_to: String::new(),
        }];
        for alias in &account.aliases {
            if alias.email.trim().is_empty() {
                continue;
            }
            let label = format_identity(&alias.name, &alias.email);
            if choices.iter().any(|c| c.label == label) {
                continue;
            }
            choices.push(ComposeFromChoice {
                from_addr: label.clone(),
                label,
                reply_to: alias.reply_to.clone(),
            });
        }
        choices
    }

    /// 选择发件身份：`from_addr` 为空表示账号默认；别名自带的 Reply-To 仅在
    /// 用户尚未填写时预填，避免覆盖手输内容。
    pub(crate) fn select_compose_from(
        &mut self,
        from_addr: String,
        alias_reply_to: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.compose_from_addr = from_addr;
        self.compose_from_custom = false;
        if !alias_reply_to.trim().is_empty()
            && self.compose_reply_to.read(cx).value().trim().is_empty()
        {
            self.compose_reply_to
                .update(cx, |s, cx| s.set_value(alias_reply_to, window, cx));
        }
        self.schedule_draft_autosave(cx);
        cx.notify();
    }

    /// 切到「自定义 From」手输态（对应原版 `custom_from_row` 的铅笔按钮）。
    pub(crate) fn begin_custom_compose_from(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.compose_from_addr.trim().is_empty() {
            self.compose_from_addr = self.compose_account_identity();
        }
        self.compose_from_custom = true;
        let current = self.compose_from_addr.clone();
        self.compose_from
            .update(cx, |s, cx| s.set_value(current, window, cx));
        cx.notify();
    }

    /// 从自定义 From 退回下拉选择。
    pub(crate) fn end_custom_compose_from(&mut self, cx: &mut Context<Self>) {
        self.compose_from_custom = false;
        self.compose_from_addr.clear();
        self.schedule_draft_autosave(cx);
        cx.notify();
    }

    /// 新写信 / 回复 / 转发入口复位发件身份与 Reply-To。
    fn reset_compose_identity(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.compose_from_addr.clear();
        self.compose_from_custom = false;
        self.compose_from
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.compose_reply_to
            .update(cx, |s, cx| s.set_value("", window, cx));
    }

    /// 从草稿 / 撤销缓冲恢复发件身份与 Reply-To。
    fn restore_compose_identity(
        &mut self,
        from_addr: &str,
        reply_to: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.compose_from_addr = from_addr.to_string();
        // 草稿里存的是自定义地址时，恢复后直接进入手输态，避免被下拉覆盖。
        self.compose_from_custom = !from_addr.trim().is_empty();
        self.compose_from
            .update(cx, |s, cx| s.set_value(from_addr.to_string(), window, cx));
        self.compose_reply_to
            .update(cx, |s, cx| s.set_value(reply_to.to_string(), window, cx));
    }

    /// 发送前校验：Reply-To 非空时必须是合法邮箱（多个用逗号分隔）。
    pub(crate) fn compose_reply_to_problem(&self, cx: &Context<Self>) -> Option<String> {
        let raw = self.compose_reply_to.read(cx).value().to_string();
        for part in split_recipient_parts(&raw) {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            if !RecipientPill::parse(part).is_some_and(|pill| pill.valid) {
                return Some(format!("Reply-To「{part}」不是有效的邮箱地址"));
            }
        }
        None
    }

    // ---- WP-10-1 收件人胶囊（对齐 Linux `src/ui/recipient_row.rs`） ----

    /// 取某个收件人字段对应的输入框实体（`Entity` 是引用计数，克隆开销极小）。
    pub(crate) fn compose_field_input(&self, field: ComposeAddressField) -> Entity<InputState> {
        match field {
            ComposeAddressField::To => self.compose_to.clone(),
            ComposeAddressField::Cc => self.compose_cc.clone(),
            ComposeAddressField::Bcc => self.compose_bcc.clone(),
        }
    }

    /// 只读访问某个字段的胶囊列表。
    pub(crate) fn compose_pills(&self, field: ComposeAddressField) -> &[RecipientPill] {
        match field {
            ComposeAddressField::To => &self.compose_to_pills,
            ComposeAddressField::Cc => &self.compose_cc_pills,
            ComposeAddressField::Bcc => &self.compose_bcc_pills,
        }
    }

    fn compose_pills_mut(&mut self, field: ComposeAddressField) -> &mut Vec<RecipientPill> {
        match field {
            ComposeAddressField::To => &mut self.compose_to_pills,
            ComposeAddressField::Cc => &mut self.compose_cc_pills,
            ComposeAddressField::Bcc => &mut self.compose_bcc_pills,
        }
    }

    /// 按当前正文格式算出投递用的 HTML。
    ///
    /// 纯文本模式返回 `None`（只发 `text/plain`）；其余三种格式把正文转换到
    /// HTML，交给 SMTP 层组成 `multipart/alternative`，纯文本作为兜底。
    pub(crate) fn compose_body_html(&self, cx: &Context<Self>) -> Option<String> {
        if self.compose_format == ComposeFormat::Plain {
            return None;
        }
        let body = self.compose_body.read(cx).value().to_string();
        if body.trim().is_empty() {
            return None;
        }
        Some(hylki_mail_core::markup::convert(
            &body,
            self.compose_format,
            ComposeFormat::Html,
        ))
    }

    /// WP-10-3：切换正文格式。
    ///
    /// 与原版 `on_load_as` 一致，切换不是「换一个空编辑器」，而是把当前正文
    /// **转换**到目标表示后写回编辑区，因此切过去不会丢内容。用 `replace_all`
    /// 而不是 `set_value`，保留撤销历史（一次误切换可以 Ctrl+Z 撤回）。
    pub(crate) fn set_compose_format(
        &mut self,
        to: ComposeFormat,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if to == self.compose_format {
            return;
        }
        let from = self.compose_format;
        let body = self.compose_body.read(cx).value().to_string();
        let converted = hylki_mail_core::markup::convert(&body, from, to);
        self.compose_format = to;
        self.compose_preview = false;
        self.compose_body
            .update(cx, |state, cx| state.replace_all(converted, window, cx));
        // `replace_all` 是静默写入（不触发 Change 事件），自动保存要手动续上。
        self.schedule_draft_autosave(cx);
        self.compose_body.focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    pub(crate) fn toggle_compose_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.compose_preview = !self.compose_preview;
        if !self.compose_preview {
            self.compose_body.focus_handle(cx).focus(window, cx);
        }
        cx.notify();
    }

    /// WP-10-3：向正文插入一段格式语法（加粗 / 斜体 / 列表 / 引用 / 代码块 / 链接）。
    ///
    /// 有选区时包裹选区，没有选区时插入带占位文案的完整片段。
    /// 光标只能落在插入内容末尾（`gpui-base` 的 `set_cursor_to` 不对外），
    /// 所以占位文案用的是「一看就知道要改哪个字」的词，而不是空标记。
    pub(crate) fn insert_compose_markup(
        &mut self,
        kind: ComposeMarkup,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.compose_format.has_format_toolbar() {
            return;
        }
        self.compose_body.update(cx, |state, cx| {
            let selected = state.selected_text().to_string();
            let snippet = kind.wrap(&selected);
            state.replace(snippet, window, cx);
        });
        self.schedule_draft_autosave(cx);
        self.compose_body.focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    /// 某个字段的完整收件人串（胶囊 + 仍在输入的那一段），发信与草稿落盘用。
    pub(crate) fn compose_field_text(
        &self,
        field: ComposeAddressField,
        cx: &Context<Self>,
    ) -> String {
        let mut parts: Vec<String> = self
            .compose_pills(field)
            .iter()
            .map(RecipientPill::to_field_text)
            .collect();
        let typed = self
            .compose_field_input(field)
            .read(cx)
            .value()
            .trim()
            .to_string();
        if !typed.is_empty() {
            parts.push(typed);
        }
        parts.join(", ")
    }

    /// 把一个已确认的地址加入胶囊（同地址去重）。返回是否真的新增。
    fn push_recipient_pill(&mut self, field: ComposeAddressField, raw: &str) -> bool {
        let Some(pill) = RecipientPill::parse(raw) else {
            return false;
        };
        // 点击胶囊编辑后再次提交时，尽量放回原位置，避免顺序跳动。
        let resume = self
            .compose_pill_resume
            .filter(|(f, _)| *f == field)
            .map(|(_, idx)| idx);
        let pills = self.compose_pills_mut(field);
        if pills
            .iter()
            .any(|p| p.address.eq_ignore_ascii_case(&pill.address))
        {
            return false;
        }
        match resume {
            Some(idx) if idx <= pills.len() => pills.insert(idx, pill),
            _ => pills.push(pill),
        }
        true
    }

    /// 收件人输入框内容变更后：把已写完的分段吸收成胶囊，输入框只留最后一段。
    ///
    /// 触发点与 Linux 原版一致：逗号 / 分号切分；段末空格且该段已是合法地址
    /// 时也立即成胶囊（`Zhang San` 这类带空格显示名不会被误切）。
    pub(crate) fn absorb_recipient_typed(
        &mut self,
        field: ComposeAddressField,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = self.compose_field_input(field);
        let value = input.read(cx).value().to_string();
        if value.is_empty() {
            return;
        }
        let mut parts = split_recipient_parts(&value);
        let tail = parts.pop().unwrap_or_default();
        let mut changed = false;
        for part in parts {
            changed |= self.push_recipient_pill(field, &part);
        }
        // 段末空格表示「这一段写完了」，但只在该段已是合法地址时才切，
        // 否则会把正在输入的显示名（如 `Zhang San`）提前切走。
        let trimmed = tail.trim_end();
        let mut remaining = tail.clone();
        if trimmed.len() != tail.len() && RecipientPill::parse(trimmed).is_some_and(|p| p.valid) {
            changed |= self.push_recipient_pill(field, trimmed);
            remaining.clear();
        }
        if remaining != value {
            input.update(cx, |state, cx| state.set_value(remaining, window, cx));
            changed = true;
        }
        if changed {
            self.schedule_draft_autosave(cx);
            cx.notify();
        }
    }

    /// 把输入框里剩下的内容整段提交成胶囊（回车 / 失焦触发）。
    pub(crate) fn commit_recipient_typed(
        &mut self,
        field: ComposeAddressField,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = self.compose_field_input(field);
        let value = input.read(cx).value().trim().to_string();
        self.compose_pill_resume = self.compose_pill_resume.filter(|(f, _)| *f != field);
        if value.is_empty() {
            return;
        }
        let mut changed = false;
        for part in split_recipient_parts(&value) {
            changed |= self.push_recipient_pill(field, &part);
        }
        input.update(cx, |state, cx| state.set_value("", window, cx));
        self.compose_pill_resume = None;
        if changed {
            self.schedule_draft_autosave(cx);
        }
        cx.notify();
    }

    /// 收件人输入框事件统一处理。
    ///
    /// 订阅回调没有 `&mut Window`，而改写输入框内容需要窗口，故经
    /// `window_handle` 在 `cx.defer` 的下一 tick 执行，避免借用重入。
    fn on_recipient_input_event(
        &mut self,
        field: ComposeAddressField,
        event: &InputEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                self.compose_autocomplete_field = Some(field);
                self.schedule_draft_autosave(cx);
                let handle = self.compose_window.unwrap_or(self.window_handle);
                let entity = cx.entity();
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, cx| {
                        entity.update(cx, |app, cx| app.absorb_recipient_typed(field, window, cx));
                    });
                });
                cx.notify();
            }
            InputEvent::PressEnter { .. } | InputEvent::Blur => {
                if self.compose_autocomplete_field == Some(field) {
                    self.compose_autocomplete_field = None;
                }
                let handle = self.compose_window.unwrap_or(self.window_handle);
                let entity = cx.entity();
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, cx| {
                        entity.update(cx, |app, cx| app.commit_recipient_typed(field, window, cx));
                    });
                });
                cx.notify();
            }
            _ => {}
        }
    }

    /// 点击胶囊尾部 `×`：删除该地址。
    pub(crate) fn remove_recipient_pill(
        &mut self,
        field: ComposeAddressField,
        idx: usize,
        cx: &mut Context<Self>,
    ) {
        let pills = self.compose_pills_mut(field);
        if idx >= pills.len() {
            return;
        }
        pills.remove(idx);
        self.schedule_draft_autosave(cx);
        cx.notify();
    }

    /// 点击胶囊主体：把地址放回输入框继续编辑（对齐原版「点击胶囊可编辑」）。
    pub(crate) fn edit_recipient_pill(
        &mut self,
        field: ComposeAddressField,
        idx: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pill = {
            let pills = self.compose_pills_mut(field);
            if idx >= pills.len() {
                return;
            }
            pills.remove(idx)
        };
        let input = self.compose_field_input(field);
        let existing = input.read(cx).value().trim().to_string();
        let next = if existing.is_empty() {
            pill.to_field_text()
        } else {
            format!("{}, {}", existing, pill.to_field_text())
        };
        self.compose_pill_resume = Some((field, idx));
        input.update(cx, |state, cx| {
            state.set_value(next, window, cx);
            state.focus(window, cx);
        });
        self.compose_autocomplete_field = Some(field);
        cx.notify();
    }

    /// 点击胶囊上的 `⇄`：移到下一个收件人字段（To -> Cc -> Bcc -> To），
    /// 目标字段为 Cc / Bcc 时自动展开抄送行。
    pub(crate) fn move_recipient_pill(
        &mut self,
        field: ComposeAddressField,
        idx: usize,
        cx: &mut Context<Self>,
    ) {
        let pill = {
            let pills = self.compose_pills_mut(field);
            if idx >= pills.len() {
                return;
            }
            pills.remove(idx)
        };
        let target = field.next();
        let pills = self.compose_pills_mut(target);
        if !pills
            .iter()
            .any(|p| p.address.eq_ignore_ascii_case(&pill.address))
        {
            pills.push(pill);
        }
        if matches!(target, ComposeAddressField::Cc | ComposeAddressField::Bcc) {
            self.compose_show_cc_bcc = true;
        }
        self.schedule_draft_autosave(cx);
        cx.notify();
    }

    /// 整体设置某个收件人字段（逗号分隔文本解析为胶囊，输入框清空）。
    fn set_recipient_field_text(
        &mut self,
        field: ComposeAddressField,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pills: Vec<RecipientPill> = split_recipient_parts(text)
            .iter()
            .filter_map(|part| RecipientPill::parse(part))
            .collect();
        *self.compose_pills_mut(field) = pills;
        let input = self.compose_field_input(field);
        input.update(cx, |state, cx| state.set_value("", window, cx));
    }

    /// 清空某个收件人字段（胶囊 + 输入框）。
    fn clear_recipient_field(
        &mut self,
        field: ComposeAddressField,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_recipient_field_text(field, "", window, cx);
    }

    /// 查询指定账号配置的邮件签名。
    fn account_signature(&self, account_id: u32) -> String {
        self.accounts
            .iter()
            .find(|a| a.id == account_id)
            .map(|a| a.signature.clone())
            .unwrap_or_default()
    }

    /// 确保写信自动补全有联系人缓存。
    fn ensure_contacts_loaded(&self) {
        if !self.demo_mode && self.known_contacts.is_empty() {
            if let Some(ref w) = self.worker {
                w.send(MailRequest::ListKnownContacts {
                    query: String::new(),
                    offset: 0,
                    limit: 500,
                });
            }
        }
    }

    // ---- WP-10-5：定时发送（Send Later） ----

    /// 定时发送下拉里的预设时间（今晚 / 明早 / 明天下午 / 周六 / 周一），
    /// 与 Snooze 共用同一份时间表（对齐原版 `src/ui/later.rs`）。
    pub(crate) fn compose_later_presets(&self) -> Vec<LaterPreset> {
        compose_schedule::later_presets()
    }

    /// 展开「自定义时间…」输入行，预填下一个整点。
    pub(crate) fn begin_custom_compose_schedule(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let at = (chrono::Local::now() + chrono::Duration::hours(1))
            .format("%Y-%m-%d %H:00")
            .to_string();
        self.compose_schedule_open = true;
        self.compose_schedule_error = None;
        self.compose_schedule_input
            .update(cx, |input, cx| input.set_value(at, window, cx));
        self.compose_schedule_input
            .focus_handle(cx)
            .focus(window, cx);
        cx.notify();
    }

    /// 确认手输的定时时间：解析成功后直接排入发件箱。
    pub(crate) fn confirm_custom_compose_schedule(&mut self, cx: &mut Context<Self>) {
        let text = self.compose_schedule_input.read(cx).value().to_string();
        match compose_schedule::parse_schedule_input(&text, chrono::Local::now()) {
            Ok(at) => {
                self.compose_schedule_error = None;
                self.send_compose_scheduled(at, cx);
            }
            Err(message) => {
                self.compose_schedule_error = Some(SharedString::from(message));
                cx.notify();
            }
        }
    }

    /// 收起自定义时间输入行（不影响已排期的邮件）。
    pub(crate) fn cancel_custom_compose_schedule(&mut self, cx: &mut Context<Self>) {
        self.compose_schedule_open = false;
        self.compose_schedule_error = None;
        cx.notify();
    }

    /// WP-10-5 拖拽上传附件：把拖进写信区的文件追加到附件列表。
    ///
    /// 目录直接忽略（拖一整个文件夹进来没有意义），同名/同路径的跳过，
    /// 避免重复拖入同一份文件把附件列表撑成一串重复项。
    pub(crate) fn attach_dropped_files(
        &mut self,
        paths: Vec<std::path::PathBuf>,
        cx: &mut Context<Self>,
    ) {
        let mut added = 0usize;
        for path in paths {
            if path.is_dir() {
                continue;
            }
            let display = path.to_string_lossy().to_string();
            if display.is_empty() || self.compose_attachments.contains(&display) {
                continue;
            }
            self.compose_attachments.push(display);
            added += 1;
        }
        if added > 0 {
            self.status_line = SharedString::from(format!("已添加 {added} 个附件"));
            self.cancel_draft_autosave();
            cx.notify();
        }
    }

    // ---- WP-10-5：写信模板（Save as Template） ----

    /// 把模板内容填回写信器：收件人、主题、正文、正文格式与附件一并恢复。
    pub(crate) fn apply_compose_template(
        &mut self,
        template: &MailTemplate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_recipient_field_text(ComposeAddressField::To, &template.to_addr, window, cx);
        self.set_recipient_field_text(ComposeAddressField::Cc, &template.cc_addr, window, cx);
        self.set_recipient_field_text(ComposeAddressField::Bcc, &template.bcc_addr, window, cx);
        if !template.cc_addr.is_empty() || !template.bcc_addr.is_empty() {
            self.compose_show_cc_bcc = true;
        }
        self.compose_subject.update(cx, |s, cx| {
            s.set_value(template.subject.clone(), window, cx)
        });
        // 正文按模板记录里的格式打开，避免把 Markdown 源当成纯文本发出去。
        self.compose_format = ComposeFormat::from_str_lossy(&template.body_format);
        self.compose_preview = false;
        self.compose_body.update(cx, |s, cx| {
            s.set_value(template.body_text.clone(), window, cx)
        });
        // 附件按路径去重合并，已在信里的不重复添加。
        for path in &template.attachment_paths {
            if !self.compose_attachments.contains(path) {
                self.compose_attachments.push(path.clone());
            }
        }
        self.cancel_draft_autosave();
        self.compose_template_save_open = false;
        self.status_line = SharedString::from(format!("已套用模板「{}」", template.name));
        cx.notify();
    }

    /// 展开「存为模板」的名称输入行，默认用当前主题当名称。
    pub(crate) fn begin_save_compose_template(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let subject = self.compose_subject.read(cx).value().trim().to_string();
        let name = if subject.is_empty() {
            "未命名模板".to_string()
        } else {
            subject
        };
        self.compose_template_save_open = true;
        self.compose_template_name
            .update(cx, |input, cx| input.set_value(name, window, cx));
        self.compose_template_name
            .focus_handle(cx)
            .focus(window, cx);
        cx.notify();
    }

    /// 把当前写信内容存成一条新模板。
    pub(crate) fn confirm_save_compose_template(&mut self, cx: &mut Context<Self>) {
        let name = self
            .compose_template_name
            .read(cx)
            .value()
            .trim()
            .to_string();
        if name.is_empty() {
            self.status_line = SharedString::from("请填写模板名称");
            cx.notify();
            return;
        }
        let template = MailTemplate {
            id: 0,
            name,
            to_addr: self.compose_field_text(ComposeAddressField::To, cx),
            cc_addr: self.compose_field_text(ComposeAddressField::Cc, cx),
            bcc_addr: self.compose_field_text(ComposeAddressField::Bcc, cx),
            subject: self.compose_subject.read(cx).value().to_string(),
            body_text: self.compose_body.read(cx).value().to_string(),
            body_format: self.compose_format.as_str().to_string(),
            attachment_paths: self.compose_attachments.clone(),
            created_at: chrono::Local::now().timestamp(),
        };
        self.compose_template_save_open = false;
        if self.demo_mode {
            self.status_line =
                SharedString::from(format!("演示模式：已生成模板「{}」", template.name));
            cx.notify();
            return;
        }
        let result = self
            .worker
            .as_ref()
            .ok_or_else(|| "邮件后台线程不可用".to_string())
            .and_then(|w| w.send_checked(MailRequest::SaveTemplate { template }));
        self.status_line = SharedString::from(match result {
            Ok(()) => "模板已保存".to_string(),
            Err(message) => message,
        });
        cx.notify();
    }

    /// 收起「存为模板」的名称输入行。
    pub(crate) fn cancel_save_compose_template(&mut self, cx: &mut Context<Self>) {
        self.compose_template_save_open = false;
        cx.notify();
    }

    /// 删除一条模板（下拉里每项的「删除」）。
    pub(crate) fn delete_compose_template(&mut self, template_id: u32, cx: &mut Context<Self>) {
        self.templates.retain(|t| t.id != template_id);
        if self.demo_mode {
            cx.notify();
            return;
        }
        if let Some(worker) = self.worker.as_ref() {
            let _ = worker.send_checked(MailRequest::DeleteTemplate { template_id });
        }
        cx.notify();
    }

    // ---- WP-10-4：内联上下分屏回复与独立写信窗口 ----

    pub(crate) fn has_active_compose(&self) -> bool {
        compose_session_active(self.right_mode, self.compose_detached)
    }

    fn is_selected_message_ready_for_popout(&self, message_id: u32) -> bool {
        is_selected_message_ready_for_popout(
            self.selected_message.as_ref().map(|m| m.id),
            message_id,
            self.pending_body_message_id,
            self.selected_message_is_placeholder,
        )
    }

    pub(crate) fn unregister_popout_message_window(
        &mut self,
        message_id: u32,
        handle: Option<AnyWindowHandle>,
    ) {
        let should_remove = match (self.popout_message_windows.get(&message_id), handle) {
            (Some(current), Some(target)) => *current == target,
            (Some(_), None) => true,
            (None, _) => false,
        };
        if should_remove {
            self.popout_message_windows.remove(&message_id);
        }
    }

    /// 进入写信会话时决定挂载方式。
    ///
    /// 对齐原版 "where composers live"（`src/app/compose_host.rs`）：
    /// - `split = true`（回复 / 回复全部 / 转发 / 作为新邮件编辑）且偏好
    ///   `reply_inline` 打开、阅读器里还留着一封可对照的邮件时，写信面板贴在
    ///   分屏上半部分（默认落点），原邮件在下方对照，中间带可拖拽的 `.split-grab-pill`；
    /// - `split = false`（新写信 / 打开草稿 / 撤销发送回填）时整栏呈现。
    ///
    /// 若此时写信正弹在独立窗口里，先把窗口收回，避免同一份表单在两处渲染。
    fn begin_compose_session(&mut self, split: bool, cx: &mut Context<Self>) {
        self.compose_session_generation += 1;
        self.compose_preview = false;
        if self.compose_detached {
            self.close_compose_window(cx);
        }
        self.compose_detached = false;
        // 定时发送 / 存模板都是「这一次写信」的一次性动作，新会话重新收起。
        self.compose_schedule_open = false;
        self.compose_schedule_error = None;
        self.compose_template_save_open = false;
        self.compose_split = should_use_compose_split(
            split,
            self.prefs.reply_inline,
            self.selected_message.is_some(),
        );
        if self.compose_split_height < COMPOSE_SPLIT_MIN_HEIGHT {
            self.compose_split_height = COMPOSE_SPLIT_DEFAULT_HEIGHT;
        }
    }

    /// 内联分屏把手移动：写信面板贴顶，高度 = 指针 y − 分屏容器顶；
    /// 下方阅读器保底 [`COMPOSE_SPLIT_READER_MIN_HEIGHT`]。
    fn on_compose_split_drag_move(
        &mut self,
        event: &DragMoveEvent<ComposeSplitDrag>,
        cx: &mut Context<Self>,
    ) {
        let pointer_y = event.event.position.y.as_f32();
        let container_top = event.bounds.top().as_f32();
        let container_height = event.bounds.size.height.as_f32();
        let max =
            (container_height - COMPOSE_SPLIT_READER_MIN_HEIGHT).max(COMPOSE_SPLIT_MIN_HEIGHT);
        self.compose_split_height =
            (pointer_y - container_top).clamp(COMPOSE_SPLIT_MIN_HEIGHT, max);
        cx.notify();
    }

    /// 把当前写信会话弹出为独立窗口（写信栏右上角 `↗`）。
    ///
    /// 独立窗口直接渲染同一份 [`HylkiApp`] 写信状态（对齐设置窗口的做法），
    /// 所以草稿自动保存、发送、附件等逻辑无需复制；主窗口同时回到阅读器。
    pub(crate) fn pop_out_compose(&mut self, cx: &mut Context<Self>) {
        if self.compose_detached {
            if let Some(handle) = self.compose_window {
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, _| window.activate_window());
                });
            }
            return;
        }
        self.close_compose_window(cx);
        self.compose_detached = true;
        self.compose_split = false;
        // 独立窗口写信时，主窗口保持在阅读模式，不展示回信页面。
        self.right_mode = RightPaneMode::Reader;

        let subject = self.compose_subject.read(cx).value().to_string();
        let title = if subject.trim().is_empty() {
            "写邮件".to_string()
        } else {
            subject
        };
        let generation = self.compose_session_generation;
        let app_entity = cx.entity();
        let centered = Bounds::centered(None, size(px(780.0), px(700.0)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(centered)),
            titlebar: Some(TitlebarOptions {
                title: Some(title.clone().into()),
                appears_transparent: true,
                traffic_light_position: None,
            }),
            ..Default::default()
        };
        // 独立窗口首帧渲染会借用主窗口实体，等当前更新释放实体借用后再打开，避免重入 panic。
        let app_entity_for_new = app_entity.clone();
        cx.defer(move |cx| {
            let still_valid = {
                let app = app_entity.read(cx);
                app.compose_session_generation == generation && app.compose_detached
            };
            if !still_valid {
                return;
            }
            let opened = gpui_kit::open_window(options, cx, move |window, cx| {
                window.set_window_title(&title);
                cx.new(|cx| ComposeWindow::new(app_entity_for_new, window, cx))
            });
            match opened {
                Ok((handle, _entity)) => {
                    #[cfg(target_os = "windows")]
                    crate::apply_window_icon();
                    let _ = app_entity.update(cx, |app, cx| {
                        app.compose_window = Some(handle);
                        app.compose_detached = true;
                        cx.notify();
                    });
                }
                Err(err) => {
                    let _ = app_entity.update(cx, |app, cx| {
                        app.compose_detached = false;
                        app.status_line =
                            SharedString::from(format!("无法打开独立写信窗口：{err}"));
                        cx.notify();
                    });
                }
            }
        });
        cx.notify();
    }

    /// 把独立窗口里的写信收回主窗口（`↙` 按钮）。
    pub(crate) fn attach_compose_back(&mut self, cx: &mut Context<Self>) {
        self.close_compose_window(cx);
        self.right_mode = RightPaneMode::Compose;
        cx.notify();
    }

    /// 结束写信会话：清掉分屏 / 独立窗口形态，回到阅读器。
    /// 发送成功、放弃、保存草稿退出三条路径共用。
    fn finish_compose_session(&mut self, cx: &mut Context<Self>) {
        self.compose_session_generation += 1;
        self.compose_split = false;
        self.right_mode = RightPaneMode::Reader;
        if self.compose_detached || self.compose_window.is_some() {
            self.close_compose_window(cx);
        }
    }

    /// 关掉残留的独立写信窗口（新写信会话抢占 / 发送 / 放弃 / 收回前调用）。
    fn close_compose_window(&mut self, cx: &mut Context<Self>) {
        self.compose_detached = false;
        if let Some(handle) = self.compose_window.take() {
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            });
        }
    }

    pub(crate) fn start_new_compose(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.main_view = MainContentView::Mail;
        // P3：从浮层侧栏 / 窄轨发起写信时同步收起浮层。
        self.close_sidebar_peek();
        let preferred_id = if !self.prefs.compose_default_from.trim().is_empty() {
            self.accounts
                .iter()
                .find(|a| {
                    a.enabled
                        && a.email
                            .eq_ignore_ascii_case(self.prefs.compose_default_from.trim())
                })
                .map(|a| a.id)
        } else {
            None
        };
        self.compose_account_id = preferred_id
            .or_else(|| {
                self.accounts
                    .iter()
                    .find(|a| a.enabled)
                    .or_else(|| self.accounts.first())
                    .map(|a| a.id)
            })
            .unwrap_or(1);
        self.compose_draft_id = None;
        self.compose_in_reply_to.clear();
        self.compose_attachments.clear();
        self.compose_confirm_discard = false;
        // WP-10-3：正文格式属于会话状态，离开或新开写信一律回到默认格式。
        self.compose_format = ComposeFormat::default();
        self.compose_attachment_reminder = None;
        self.pending_forward_msg_id = None;
        self.compose_autocomplete_field = None;
        self.begin_compose_autosave_tracking();
        self.reset_compose_identity(window, cx);
        let initial_body = append_signature_with_dashes(
            "",
            &self.account_signature(self.compose_account_id),
            self.prefs.signature_dashes,
        );
        self.clear_recipient_field(ComposeAddressField::To, window, cx);
        self.clear_recipient_field(ComposeAddressField::Cc, window, cx);
        self.clear_recipient_field(ComposeAddressField::Bcc, window, cx);
        self.compose_show_cc_bcc = false;
        self.compose_subject
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.compose_body
            .update(cx, |s, cx| s.set_value(initial_body, window, cx));
        // WP-10-4：新写信没有可对照的原邮件，独占右栏。
        self.begin_compose_session(false, cx);
        self.right_mode = RightPaneMode::Compose;
        self.ensure_contacts_loaded();
        if !self.prefs.compose_inline {
            self.pop_out_compose(cx);
        }
        cx.notify();
    }

    // ---- F-4 写信草稿 5 秒防抖自动保存 ----

    /// 进入写信页（新写信 / 回复 / 转发 / 打开草稿）时重置自动保存跟踪。
    fn begin_compose_autosave_tracking(&mut self) {
        self.compose_autosave_generation += 1;
        self.compose_autosave_dirty = false;
        self.compose_autosave_pending = false;
        self.compose_autosaved_new = false;
        self.compose_autosaved_at = None;
    }

    /// 离开写信流程（发送 / 取消 / 存草稿退出）时清除挂起的自动保存。
    fn cancel_draft_autosave(&mut self) {
        self.compose_autosave_generation += 1;
        self.compose_autosave_dirty = false;
        self.compose_autosave_pending = false;
    }

    /// 内容变更后重置 5 秒防抖定时器：代际号 +1 使旧定时器醒来后失效，
    /// 5 秒内无新输入才真正执行静默保存（demo 模式无本地草稿库，跳过）。
    pub(crate) fn schedule_draft_autosave(&mut self, cx: &mut Context<Self>) {
        if self.demo_mode {
            return;
        }
        self.compose_autosave_dirty = true;
        self.compose_autosave_generation += 1;
        let generation = self.compose_autosave_generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(5)).await;
            let _ = this.update(cx, |app, cx| {
                if app.compose_autosave_generation == generation
                    && app.compose_autosave_dirty
                    && app.has_active_compose()
                {
                    app.autosave_draft_now(cx);
                }
            });
        })
        .detach();
    }

    /// 立即执行一次静默草稿保存（`AutosaveLocalDraft`，不回发状态行）。
    fn autosave_draft_now(&mut self, cx: &mut Context<Self>) {
        self.compose_autosave_dirty = false;
        self.compose_autosave_pending = true;
        let draft = LocalDraft {
            id: self.compose_draft_id.unwrap_or(0),
            account_id: self.compose_account_id,
            to_addr: self.compose_field_text(ComposeAddressField::To, cx),
            cc_addr: self.compose_field_text(ComposeAddressField::Cc, cx),
            bcc_addr: self.compose_field_text(ComposeAddressField::Bcc, cx),
            subject: self.compose_subject.read(cx).value().to_string(),
            body_text: self.compose_body.read(cx).value().to_string(),
            in_reply_to: self.compose_in_reply_to.clone(),
            from_addr: self.compose_from_addr.clone(),
            reply_to: self.compose_reply_to.read(cx).value().to_string(),
            body_format: self.compose_format.as_str().to_string(),
            attachment_paths: self.compose_attachments.clone(),
            updated_at: 0,
        };
        if let Some(ref w) = self.worker {
            w.send(MailRequest::AutosaveLocalDraft { draft });
        }
    }

    pub(crate) fn start_popout_compose_from_message(
        &mut self,
        detail: MessageDetail,
        kind: PopoutReplyKind,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.has_active_compose() && self.compose_has_unsaved_content(cx) {
            self.status_line = SharedString::from("当前写信尚未完成，请先发送、存草稿或取消");
            if let Some(handle) = self.compose_window {
                cx.defer(move |cx| {
                    let _ = handle.update(cx, |_, window, _| window.activate_window());
                });
            }
            cx.notify();
            return false;
        }
        self.unregister_popout_message_window(detail.id, None);
        let entity = cx.entity();
        let handle = self.window_handle;
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| {
                entity.update(cx, |app, cx| {
                    let ok = match kind {
                        PopoutReplyKind::Reply => app.start_reply(&detail, window, cx),
                        PopoutReplyKind::ReplyAll => app.start_reply_all(&detail, window, cx),
                        PopoutReplyKind::Forward => app.start_forward(&detail, window, cx),
                    };
                    if ok && !app.compose_detached {
                        app.pop_out_compose(cx);
                    }
                });
            });
        });
        true
    }

    pub(crate) fn start_reply(
        &mut self,
        detail: &MessageDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.start_reply_with(detail, "Re:", &detail.sender, window, cx)
    }

    /// 回复全部：收件人 = 原发件人 + 原收件人（沿用现有写信状态，无新 API）。
    pub(crate) fn start_reply_all(
        &mut self,
        detail: &MessageDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut to = detail.sender.clone();
        let recipients = detail.recipients.trim();
        if !recipients.is_empty() {
            to.push_str(", ");
            to.push_str(recipients);
        }
        self.start_reply_with(detail, "Re:", &to, window, cx)
    }

    /// 转发：主题加 Fwd: 前缀，正文引用原邮件，收件人留空，并自动携带原邮件已缓存附件。
    pub(crate) fn start_forward(
        &mut self,
        detail: &MessageDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        // WP-10-4：内联分屏时原邮件仍在眼前，回复 / 转发按钮随时可点；
        // 当前写信已有内容时先拦下，避免一次误点把正在写的正文换掉。
        if self.has_active_compose() && self.compose_has_unsaved_content(cx) {
            self.status_line = SharedString::from("当前写信尚未完成，请先发送、存草稿或取消");
            cx.notify();
            return false;
        }
        let forward_subject = if detail.subject.to_lowercase().starts_with("fwd:") {
            detail.subject.clone()
        } else {
            format!("Fwd: {}", detail.subject)
        };
        let quoted = format!(
            "\n\n------------------ 转发邮件 ------------------\n发件人：{}\n时间：{}\n主题：{}\n\n{}",
            detail.sender, detail.date_display, detail.subject, detail.body_text
        );
        let body_with_sig = insert_signature_with_quote(
            &quoted,
            &self.account_signature(detail.account_id),
            self.prefs.signature_above_quote,
            self.prefs.signature_dashes,
        );

        self.compose_account_id = detail.account_id;
        self.compose_draft_id = None;
        self.compose_in_reply_to.clear();
        self.compose_attachments.clear();
        self.compose_confirm_discard = false;
        // WP-10-3：正文格式属于会话状态，离开或新开写信一律回到默认格式。
        self.compose_format = ComposeFormat::default();
        self.compose_attachment_reminder = None;
        self.pending_forward_msg_id = Some(detail.id);
        self.compose_autocomplete_field = None;
        self.begin_compose_autosave_tracking();
        self.reset_compose_identity(window, cx);
        self.clear_recipient_field(ComposeAddressField::To, window, cx);
        self.clear_compose_cc_bcc(window, cx);
        self.compose_subject
            .update(cx, |s, cx| s.set_value(forward_subject, window, cx));
        self.compose_body
            .update(cx, |s, cx| s.set_value(body_with_sig, window, cx));
        // WP-10-4：转发时原邮件留在阅读器上半部分对照（偏好可关）。
        self.begin_compose_session(true, cx);
        self.right_mode = RightPaneMode::Compose;
        self.ensure_contacts_loaded();

        if !detail.attachments.is_empty() {
            if self.demo_mode {
                let dest_dir = std::env::temp_dir().join(format!("hylki-forward-{}", detail.id));
                if std::fs::create_dir_all(&dest_dir).is_ok() {
                    for att in &detail.attachments {
                        let path = dest_dir.join(&att.filename);
                        if std::fs::write(&path, format!("Demo attachment: {}", att.filename))
                            .is_ok()
                        {
                            self.compose_attachments
                                .push(path.to_string_lossy().to_string());
                        }
                    }
                }
            } else if let Some(ref w) = self.worker {
                w.send(MailRequest::ExportForwardAttachments {
                    message_id: detail.id,
                });
            }
        }
        if !self.prefs.reply_inline {
            self.pop_out_compose(cx);
        }
        cx.notify();
        true
    }

    /// WP-08：作为新邮件编辑（保留原主题、收件人与正文，打开新写信面板）。
    pub(crate) fn start_edit_as_new(
        &mut self,
        detail: &MessageDetail,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.compose_account_id = detail.account_id;
        self.compose_draft_id = None;
        self.compose_in_reply_to.clear();
        self.compose_attachments.clear();
        self.compose_confirm_discard = false;
        // WP-10-3：正文格式属于会话状态，离开或新开写信一律回到默认格式。
        self.compose_format = ComposeFormat::default();
        self.compose_attachment_reminder = None;
        self.pending_forward_msg_id = None;
        self.compose_autocomplete_field = None;
        self.begin_compose_autosave_tracking();
        self.reset_compose_identity(window, cx);
        self.set_recipient_field_text(ComposeAddressField::To, &detail.recipients, window, cx);
        self.clear_compose_cc_bcc(window, cx);
        self.compose_subject
            .update(cx, |s, cx| s.set_value(detail.subject.clone(), window, cx));
        self.compose_body.update(cx, |s, cx| {
            s.set_value(detail.body_text.clone(), window, cx)
        });
        // WP-10-4：作为新邮件编辑同样保留原邮件对照。
        self.begin_compose_session(true, cx);
        self.right_mode = RightPaneMode::Compose;
        self.ensure_contacts_loaded();
        if !self.prefs.reply_inline {
            self.pop_out_compose(cx);
        }
        cx.notify();
    }

    /// start_reply / start_reply_all 的共用主体。
    fn start_reply_with(
        &mut self,
        detail: &MessageDetail,
        prefix: &str,
        to: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        // WP-10-4：内联分屏时原邮件仍在眼前，回复 / 转发按钮随时可点；
        // 当前写信已有内容时先拦下，避免一次误点把正在写的正文换掉。
        if self.has_active_compose() && self.compose_has_unsaved_content(cx) {
            self.status_line = SharedString::from("当前写信尚未完成，请先发送、存草稿或取消");
            cx.notify();
            return false;
        }
        let reply_subject = if detail
            .subject
            .to_lowercase()
            .starts_with(&prefix.to_lowercase())
        {
            detail.subject.clone()
        } else {
            format!("{} {}", prefix, detail.subject)
        };
        let header = format_reply_header(
            &self.prefs.reply_header_template,
            &detail.sender,
            &detail.date_display,
            &detail.subject,
        );
        let quoted = format!("\n\n{header}\n\n{}", detail.body_text);
        let body_with_sig = insert_signature_with_quote(
            &quoted,
            &self.account_signature(detail.account_id),
            self.prefs.signature_above_quote,
            self.prefs.signature_dashes,
        );

        self.compose_account_id = detail.account_id;
        self.compose_draft_id = None;
        self.compose_in_reply_to = detail.rfc_message_id.clone();
        self.compose_attachments.clear();
        self.compose_confirm_discard = false;
        // WP-10-3：正文格式属于会话状态，离开或新开写信一律回到默认格式。
        self.compose_format = ComposeFormat::default();
        self.compose_attachment_reminder = None;
        self.pending_forward_msg_id = None;
        self.compose_autocomplete_field = None;
        self.begin_compose_autosave_tracking();
        self.reset_compose_identity(window, cx);
        self.set_recipient_field_text(ComposeAddressField::To, to, window, cx);
        self.clear_compose_cc_bcc(window, cx);
        self.compose_subject
            .update(cx, |s, cx| s.set_value(reply_subject, window, cx));
        self.compose_body
            .update(cx, |s, cx| s.set_value(body_with_sig, window, cx));
        // WP-10-4：回复 / 回复全部默认把原邮件留在上半部分（对齐原版 reader-split）。
        self.begin_compose_session(true, cx);
        self.right_mode = RightPaneMode::Compose;
        self.ensure_contacts_loaded();
        if !self.prefs.reply_inline {
            self.pop_out_compose(cx);
        }
        cx.notify();
        true
    }

    /// 判断当前写信表单是否存在未保存内容。
    pub(crate) fn compose_has_unsaved_content(&self, cx: &Context<Self>) -> bool {
        has_unsaved_compose_content(
            &self.compose_field_text(ComposeAddressField::To, cx),
            &self.compose_field_text(ComposeAddressField::Cc, cx),
            &self.compose_field_text(ComposeAddressField::Bcc, cx),
            self.compose_subject.read(cx).value().as_ref(),
            self.compose_body.read(cx).value().as_ref(),
            self.compose_attachments.len(),
        )
    }

    /// 点击「取消」或按 Esc 尝试离开写信页：有内容时弹出内联确认条；
    /// 已在确认态再次按 Esc 则继续编辑（与 Linux 一致）。
    pub(crate) fn request_close_compose(&mut self, cx: &mut Context<Self>) {
        if self.compose_confirm_discard {
            self.compose_confirm_discard = false;
            cx.notify();
            return;
        }
        if self.compose_has_unsaved_content(cx) {
            self.compose_confirm_discard = true;
            cx.notify();
        } else {
            self.discard_compose(cx);
        }
    }

    /// 放弃当前写信内容并返回阅读态。
    /// F-4：若该草稿此前由自动保存静默创建，则连同删除（避免留下孤儿草稿）。
    pub(crate) fn discard_compose(&mut self, cx: &mut Context<Self>) {
        self.cancel_draft_autosave();
        if self.compose_autosaved_new {
            if let Some(draft_id) = self.compose_draft_id.take() {
                if let Some(ref w) = self.worker {
                    w.send(MailRequest::DeleteLocalDraft { draft_id });
                }
            }
            self.compose_autosaved_new = false;
            self.compose_autosaved_at = None;
        }
        self.compose_confirm_discard = false;
        // WP-10-3：正文格式属于会话状态，离开或新开写信一律回到默认格式。
        self.compose_format = ComposeFormat::default();
        self.compose_attachment_reminder = None;
        self.pending_forward_msg_id = None;
        self.compose_autocomplete_field = None;
        self.finish_compose_session(cx);
        self.right_mode = RightPaneMode::Reader;
        cx.notify();
    }

    /// 保存为草稿并退出写信页。
    pub(crate) fn save_draft_and_close_compose(&mut self, cx: &mut Context<Self>) {
        self.cancel_draft_autosave();
        self.save_local_draft(cx);
        self.compose_confirm_discard = false;
        // WP-10-3：正文格式属于会话状态，离开或新开写信一律回到默认格式。
        self.compose_format = ComposeFormat::default();
        self.compose_attachment_reminder = None;
        self.pending_forward_msg_id = None;
        self.compose_autocomplete_field = None;
        self.finish_compose_session(cx);
        self.right_mode = RightPaneMode::Reader;
        cx.notify();
    }

    /// 点击收件人联系人补全建议项：补全当前段并立即转成胶囊。
    pub(crate) fn apply_contact_suggestion(
        &mut self,
        field: ComposeAddressField,
        contact: &KnownContact,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = self.compose_field_input(field);
        let current = target.read(cx).value().to_string();
        let next = apply_contact_completion(&current, contact);
        target.update(cx, |s, cx| s.set_value(next, window, cx));
        self.compose_autocomplete_field = None;
        self.commit_recipient_typed(field, window, cx);
    }

    /// 清空 Cc / Bcc 输入（新写信 / 回复 / 转发入口共用）。
    fn clear_compose_cc_bcc(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.clear_recipient_field(ComposeAddressField::Cc, window, cx);
        self.clear_recipient_field(ComposeAddressField::Bcc, window, cx);
        self.compose_show_cc_bcc = false;
    }

    pub(crate) fn open_local_draft(
        &mut self,
        draft: &LocalDraft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.compose_account_id = draft.account_id;
        self.compose_draft_id = Some(draft.id);
        self.compose_in_reply_to = draft.in_reply_to.clone();
        self.compose_attachments = draft.attachment_paths.clone();
        self.begin_compose_autosave_tracking();
        self.set_recipient_field_text(ComposeAddressField::To, &draft.to_addr, window, cx);
        self.set_recipient_field_text(ComposeAddressField::Cc, &draft.cc_addr, window, cx);
        self.set_recipient_field_text(ComposeAddressField::Bcc, &draft.bcc_addr, window, cx);
        self.restore_compose_identity(&draft.from_addr, &draft.reply_to, window, cx);
        self.compose_show_cc_bcc = !draft.cc_addr.trim().is_empty()
            || !draft.bcc_addr.trim().is_empty()
            || !draft.reply_to.trim().is_empty();
        self.compose_subject
            .update(cx, |s, cx| s.set_value(draft.subject.clone(), window, cx));
        // WP-10-3：草稿里记着正文格式，按原格式打开，避免把 Markdown 源当成纯文本。
        self.compose_format = ComposeFormat::from_str_lossy(&draft.body_format);
        self.compose_body
            .update(cx, |s, cx| s.set_value(draft.body_text.clone(), window, cx));
        // WP-10-4：草稿自带头与正文，整栏呈现。
        self.begin_compose_session(false, cx);
        self.right_mode = RightPaneMode::Compose;
        cx.notify();
    }

    pub(crate) fn save_local_draft(&mut self, cx: &mut Context<Self>) {
        if self.demo_mode {
            self.status_line = SharedString::from("演示模式下不支持写入本地草稿数据库");
            cx.notify();
            return;
        }
        let draft = LocalDraft {
            id: self.compose_draft_id.unwrap_or(0),
            account_id: self.compose_account_id,
            to_addr: self.compose_field_text(ComposeAddressField::To, cx),
            cc_addr: self.compose_field_text(ComposeAddressField::Cc, cx),
            bcc_addr: self.compose_field_text(ComposeAddressField::Bcc, cx),
            subject: self.compose_subject.read(cx).value().to_string(),
            body_text: self.compose_body.read(cx).value().to_string(),
            in_reply_to: self.compose_in_reply_to.clone(),
            from_addr: self.compose_from_addr.clone(),
            reply_to: self.compose_reply_to.read(cx).value().to_string(),
            body_format: self.compose_format.as_str().to_string(),
            attachment_paths: self.compose_attachments.clone(),
            updated_at: 0,
        };
        if let Some(ref w) = self.worker {
            w.send(MailRequest::SaveLocalDraft { draft });
        }
        cx.notify();
    }

    pub(crate) fn pick_mail_merge_csv(&mut self, cx: &mut Context<Self>) {
        if self.file_dialog_open {
            return;
        }
        self.file_dialog_open = true;
        let generation = self.compose_session_generation;
        cx.spawn(async move |this, cx| {
            let table = cx
                .background_executor()
                .spawn(async move {
                    let path = rfd::FileDialog::new()
                        .add_filter("CSV 表格", &["csv", "tsv", "txt"])
                        .pick_file()?;
                    let bytes = std::fs::read(path).ok()?;
                    hylki_mail_core::merge::Table::from_csv_bytes(&bytes).ok()
                })
                .await;
            cx.update(|cx| {
                let Ok(handle) = this.update(cx, |app, _| {
                    app.file_dialog_open = false;
                    app.compose_window.unwrap_or(app.window_handle)
                }) else {
                    return;
                };
                let Some(table) = table else { return };
                let _ = handle.update(cx, |_, window, cx| {
                    let _ = this.update(cx, |app, cx| {
                        if app.compose_session_generation != generation {
                            return;
                        }
                        let cols = table.columns.join(", ");
                        let tpl = hylki_mail_core::merge::Letter {
                            to: app.compose_to.read(cx).value().to_string(),
                            cc: app.compose_cc.read(cx).value().to_string(),
                            bcc: app.compose_bcc.read(cx).value().to_string(),
                            reply_to: String::new(),
                            subject: app.compose_subject.read(cx).value().to_string(),
                            body: app.compose_body.read(cx).value().to_string(),
                        };
                        let generated = table.generate_all(&tpl, false);
                        if let Some(first) = generated.first() {
                            if !first.to.trim().is_empty() {
                                app.compose_to
                                    .update(cx, |s, cx| s.set_value(first.to.clone(), window, cx));
                            }
                            if !first.subject.trim().is_empty() {
                                app.compose_subject.update(cx, |s, cx| {
                                    s.set_value(first.subject.clone(), window, cx)
                                });
                            }
                            if !first.body.trim().is_empty() {
                                app.compose_body.update(cx, |s, cx| {
                                    s.set_value(first.body.clone(), window, cx)
                                });
                            }
                        }
                        app.compose_autosaved_at = Some(
                            format!("已合并 CSV（{} 行，字段：{cols}）", generated.len()).into(),
                        );
                        cx.notify();
                    });
                });
            });
        })
        .detach();
    }

    pub(crate) fn pick_attachments(&mut self, cx: &mut Context<Self>) {
        let generation = self.compose_session_generation;
        cx.spawn(async move |this, cx| {
            let files = cx
                .background_executor()
                .spawn(async move { rfd::FileDialog::new().pick_files() })
                .await;
            if let Some(files) = files {
                let _ = this.update(cx, |app, cx| {
                    if app.compose_session_generation != generation {
                        return;
                    }
                    for file in files {
                        let p = file.to_string_lossy().to_string();
                        if !app.compose_attachments.contains(&p) {
                            app.compose_attachments.push(p);
                        }
                    }
                    app.schedule_draft_autosave(cx);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(crate) fn save_attachment(&self, att_id: u32, filename: &str, cx: &mut Context<Self>) {
        if self.demo_mode {
            return;
        }
        // 在后台线程弹出系统文件保存对话框，避免嵌套 Win32 消息循环阻塞主线程并在事件分发期间重入 AppCell::borrow_mut。
        let filename = filename.to_string();
        cx.spawn(async move |this, cx| {
            let dest = cx
                .background_executor()
                .spawn(async move { rfd::FileDialog::new().set_file_name(&filename).save_file() })
                .await;
            if let Some(dest) = dest {
                let _ = this.update(cx, |app, _cx| {
                    if let Some(ref w) = app.worker {
                        w.send(MailRequest::SaveAttachment {
                            attachment_id: att_id,
                            dest_path: dest,
                        });
                    }
                });
            }
        })
        .detach();
    }

    /// 选择目标目录并批量导出指定附件列表（重名自动追加 `(1)` 后缀）。
    pub(crate) fn save_all_attachments(
        &mut self,
        attachment_ids: Vec<u32>,
        cx: &mut Context<Self>,
    ) {
        if attachment_ids.is_empty() {
            return;
        }
        if self.demo_mode {
            self.status_line = SharedString::from("演示模式下不支持导出真实附件文件");
            cx.notify();
            return;
        }
        // 同上，放到后台线程执行，避免模态目录选择器嵌套消息循环触发 AppCell 重入 panic。
        cx.spawn(async move |this, cx| {
            let dest_dir = cx
                .background_executor()
                .spawn(async move { rfd::FileDialog::new().pick_folder() })
                .await;
            if let Some(dest_dir) = dest_dir {
                let _ = this.update(cx, |app, cx| {
                    if let Some(ref w) = app.worker {
                        app.status_line = SharedString::from(format!(
                            "正在保存 {} 个附件至 {}...",
                            attachment_ids.len(),
                            dest_dir.display()
                        ));
                        w.send(MailRequest::SaveAllAttachments {
                            attachment_ids,
                            dest_dir,
                        });
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    // ---- WP-09：引用折叠、附件抽屉、页内查找、打印预览与独立窗口 ----

    pub(crate) fn toggle_quote_expanded(&mut self, msg_id: u32, cx: &mut Context<Self>) {
        if !self.expanded_quote_message_ids.remove(&msg_id) {
            self.expanded_quote_message_ids.insert(msg_id);
        }
        cx.notify();
    }

    pub(crate) fn toggle_attachment_drawer(&mut self, cx: &mut Context<Self>) {
        self.attachment_drawer_collapsed = !self.attachment_drawer_collapsed;
        cx.notify();
    }

    pub(crate) fn toggle_attachment_drawer_view(&mut self, cx: &mut Context<Self>) {
        self.attachment_drawer_list_view = !self.attachment_drawer_list_view;
        cx.notify();
    }

    pub(crate) fn toggle_attachment_drawer_sort(&mut self, cx: &mut Context<Self>) {
        self.attachment_drawer_sort_desc = !self.attachment_drawer_sort_desc;
        cx.notify();
    }

    pub(crate) fn on_attachment_drawer_drag_move(
        &mut self,
        event: &DragMoveEvent<AttachmentDrawerDrag>,
        cx: &mut Context<Self>,
    ) {
        let pointer_y = event.event.position.y.as_f32();
        let bottom_y = event.bounds.bottom().as_f32();
        // 抽屉位于底部，总高度约为 bottom_y - pointer_y；减去标题与手柄高度（~48px）得到内容区高度。
        let body_h = (bottom_y - pointer_y - 48.0).clamp(DRAWER_MIN_HEIGHT, DRAWER_MAX_HEIGHT);
        self.attachment_drawer_collapsed = false;
        self.attachment_drawer_height = body_h;
        cx.notify();
    }

    pub(crate) fn ensure_attachment_thumbnail(
        &mut self,
        att_id: u32,
        mime_type: &str,
        downloaded: bool,
    ) {
        if self.demo_mode
            || !downloaded
            || attachment_drawer::image_format_for_mime(mime_type).is_none()
            || self.attachment_thumb_cache.contains_key(&att_id)
            || !self.attachment_thumb_requested.insert(att_id)
        {
            return;
        }
        if let Some(ref w) = self.worker {
            w.send(MailRequest::FetchAttachmentBytes {
                attachment_id: att_id,
            });
        }
    }

    pub(crate) fn ask_strip_attachment(&mut self, att_id: u32, cx: &mut Context<Self>) {
        self.pending_strip_attachment_id = Some(att_id);
        cx.notify();
    }

    pub(crate) fn cancel_strip_attachment(&mut self, cx: &mut Context<Self>) {
        self.pending_strip_attachment_id = None;
        cx.notify();
    }

    pub(crate) fn confirm_strip_attachment(
        &mut self,
        account_id: u32,
        message_id: u32,
        attachment_id: u32,
        attachment_name: String,
        size_bytes: usize,
        cx: &mut Context<Self>,
    ) {
        self.pending_strip_attachment_id = None;
        if self.demo_mode {
            return;
        }
        self.stripping_attachment_ids.insert(attachment_id);
        self.status_line =
            SharedString::from(format!("正在从服务器剥离附件：{attachment_name}..."));
        if let Some(ref w) = self.worker {
            w.send(MailRequest::StripAttachment {
                account_id,
                message_id,
                attachment_id,
                attachment_name,
                size_bytes,
            });
        }
        cx.notify();
    }

    pub(crate) fn open_drawer_image_preview(
        &mut self,
        att_id: u32,
        filename: String,
        mime_type: String,
        cx: &mut Context<Self>,
    ) {
        let cached = self.attachment_thumb_cache.get(&att_id).cloned();
        if cached.is_none() {
            self.ensure_attachment_thumbnail(att_id, &mime_type, true);
        }
        self.drawer_image_preview = Some((att_id, filename, cached));
        cx.notify();
    }

    pub(crate) fn close_drawer_image_preview(&mut self, cx: &mut Context<Self>) {
        self.drawer_image_preview = None;
        cx.notify();
    }

    pub(crate) fn toggle_reader_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reader_find_open = !self.reader_find_open;
        if self.reader_find_open {
            self.reader_find_input
                .update(cx, |s, cx| s.focus(window, cx));
            self.refresh_reader_find_highlights(cx);
        } else {
            self.reader_find_query.clear();
            self.reader_find_total_matches = 0;
            self.reader_find_active_match = 0;
            for (_, state) in self.reader_text_states.values() {
                state.update(cx, |s, cx| s.clear_range_highlights(cx));
            }
        }
        cx.notify();
    }

    pub(crate) fn step_reader_find(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.reader_find_total_matches == 0 {
            return;
        }
        let len = self.reader_find_total_matches as isize;
        self.reader_find_active_match =
            ((self.reader_find_active_match as isize + delta).rem_euclid(len)) as usize;
        self.refresh_reader_find_highlights(cx);
    }

    pub(crate) fn retain_current_reader_text_states(&mut self) {
        let Some(detail) = self.selected_message.as_ref() else {
            self.reader_text_states.clear();
            return;
        };
        self.reader_text_states.retain(|id, _| {
            !self.collapsed_thread_cards.contains(id)
                && (*id == detail.id
                    || detail
                        .thread_messages
                        .iter()
                        .any(|message| message.id == *id))
        });
    }

    pub(crate) fn refresh_reader_find_highlights(&mut self, cx: &mut Context<Self>) {
        self.retain_current_reader_text_states();
        let query = self.reader_find_query.trim().to_ascii_lowercase();
        if query.is_empty() {
            self.reader_find_total_matches = 0;
            self.reader_find_active_match = 0;
            for (_, state) in self.reader_text_states.values() {
                state.update(cx, |s, cx| s.clear_range_highlights(cx));
            }
            cx.notify();
            return;
        }
        let mut ordered_ids: Vec<u32> = self.reader_text_states.keys().copied().collect();
        ordered_ids.sort_unstable();

        let mut all_matches = Vec::new();
        let mut total = 0usize;
        for id in ordered_ids {
            if let Some((_, state)) = self.reader_text_states.get(&id) {
                let rendered = state.read(cx).rendered_text();
                let ranges =
                    message_window::find_case_insensitive_ranges(rendered.as_str(), &query);
                total += ranges.len();
                all_matches.push((state.clone(), ranges));
            }
        }
        self.reader_find_total_matches = total;
        if total == 0 || self.reader_find_active_match >= total {
            self.reader_find_active_match = 0;
        }

        let normal_bg = gpui_kit::hsla(48.0 / 360.0, 0.96, 0.62, 0.45);
        let active_bg = gpui_kit::hsla(28.0 / 360.0, 0.96, 0.56, 0.78);
        let mut cursor = 0usize;
        for (state, ranges) in all_matches {
            let mut highlights = Vec::with_capacity(ranges.len());
            let mut reveal = None;
            for r in ranges {
                let is_active = total > 0 && cursor == self.reader_find_active_match;
                if is_active {
                    reveal = Some(r.clone());
                }
                highlights.push(RangeHighlight::new(
                    r,
                    if is_active { active_bg } else { normal_bg },
                ));
                cursor += 1;
            }
            state.update(cx, |s, cx| {
                let _ = s.set_range_highlights(highlights, cx);
                if let Some(r) = reveal {
                    let _ = s.reveal_range(r, cx);
                }
            });
        }
        cx.notify();
    }

    pub(crate) fn ensure_reader_text_state(
        &mut self,
        msg_id: u32,
        html: &str,
        cx: &mut Context<Self>,
    ) -> Entity<TextViewState> {
        if let Some((cached_html, state)) = self.reader_text_states.get(&msg_id) {
            if cached_html == html {
                return state.clone();
            }
        }
        let state = cx.new(|cx| TextViewState::html(html, cx).selectable(true));
        self.reader_text_states
            .insert(msg_id, (html.to_string(), state.clone()));
        if self.reader_find_open && !self.reader_find_query.trim().is_empty() {
            let entity = cx.entity();
            cx.defer(move |cx| {
                entity.update(cx, |this, cx| this.refresh_reader_find_highlights(cx))
            });
        }
        state
    }

    pub(crate) fn open_print_preview(&mut self, detail: MessageDetail, cx: &mut Context<Self>) {
        self.print_preview_generation += 1;
        self.print_preview = Some(PrintPreviewState::new(detail));
        cx.notify();
    }

    pub(crate) fn close_print_preview(&mut self, cx: &mut Context<Self>) {
        self.print_preview_generation += 1;
        self.print_preview = None;
        cx.notify();
    }

    pub(crate) fn export_print_preview_pdf(&mut self, cx: &mut Context<Self>) {
        let Some(ref state) = self.print_preview else {
            return;
        };
        let default_name = format!(
            "{}.pdf",
            sanitize_filename(&state.detail.subject)
                .chars()
                .take(48)
                .collect::<String>()
        );
        let state = state.clone();
        let date_style = self.prefs.date_style.clone();
        let clock_style = self.prefs.clock_style.clone();
        let generation = self.print_preview_generation;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    rfd::FileDialog::new()
                        .add_filter("PDF 文件", &["pdf"])
                        .set_file_name(&default_name)
                        .save_file()
                        .map(|dest| {
                            print_preview::export_print_pdf(
                                &state,
                                &dest,
                                &date_style,
                                &clock_style,
                            )
                        })
                })
                .await;
            if let Some(result) = result {
                let _ = this.update(cx, |app, cx| {
                    if app.print_preview_generation != generation {
                        return;
                    }
                    match result {
                        Ok(msg) => {
                            if let Some(ref mut pp) = app.print_preview {
                                pp.status_note = Some(SharedString::from(msg.clone()));
                            }
                            app.status_line = SharedString::from(msg);
                        }
                        Err(err) => {
                            if let Some(ref mut pp) = app.print_preview {
                                pp.status_note = Some(SharedString::from(err));
                            }
                        }
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    pub(crate) fn run_system_print(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.print_preview.clone() else {
            return;
        };
        let date_style = self.prefs.date_style.clone();
        let clock_style = self.prefs.clock_style.clone();
        let generation = self.print_preview_generation;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    print_preview::launch_system_print(&state, &date_style, &clock_style)
                })
                .await;
            let _ = this.update(cx, |app, cx| {
                if app.print_preview_generation != generation {
                    return;
                }
                if let Some(ref mut pp) = app.print_preview {
                    pp.status_note = Some(SharedString::from(match result {
                        Ok(()) => "已在系统打印页打开，请在弹出的打印对话框中确认打印".to_string(),
                        Err(err) => err,
                    }));
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 右键「在独立窗口中打开」：仅单独弹出该邮件，不改变主窗口当前选中的邮件。
    pub(crate) fn open_message_in_window(
        &mut self,
        account_id: u32,
        message_id: u32,
        uid: u32,
        unread: bool,
        folder_path: &str,
        cx: &mut Context<Self>,
    ) {
        if let Some(&handle) = self.popout_message_windows.get(&message_id) {
            let app_entity = cx.entity();
            let folder_path = folder_path.to_string();
            cx.defer(move |cx| {
                if handle
                    .update(cx, |_, window, _| window.activate_window())
                    .is_err()
                {
                    let _ = app_entity.update(cx, |app, cx| {
                        app.unregister_popout_message_window(message_id, Some(handle));
                        app.open_message_in_window(
                            account_id,
                            message_id,
                            uid,
                            unread,
                            &folder_path,
                            cx,
                        );
                    });
                }
            });
            return;
        }
        if self.demo_mode {
            if unread && self.prefs.read_mark_mode == "shown" {
                self.mark_message_seen_now(account_id, message_id, uid, folder_path);
            }
            if let Some(detail) = self.demo_message_detail(message_id) {
                self.pop_out_message_window(detail, cx);
            }
            cx.notify();
            return;
        }
        if self.is_selected_message_ready_for_popout(message_id) {
            if let Some(detail) = self.selected_message.clone() {
                if unread && self.prefs.read_mark_mode == "shown" {
                    self.mark_message_seen_now(account_id, message_id, uid, folder_path);
                }
                self.pop_out_message_window(detail, cx);
            }
            return;
        }
        if let Some((detail, true)) = self.try_load_cached_detail(message_id) {
            if unread && self.prefs.read_mark_mode == "shown" {
                self.mark_message_seen_now(account_id, message_id, uid, folder_path);
            }
            self.pop_out_message_window(detail, cx);
            cx.notify();
            return;
        }
        self.pending_popout_message_ids.insert(message_id);
        if unread && self.prefs.read_mark_mode == "shown" {
            self.mark_message_seen_now(account_id, message_id, uid, folder_path);
        }
        if self.pending_body_message_id != Some(message_id) {
            if let Some(ref w) = self.worker {
                w.send(MailRequest::LoadMessageBody {
                    account_id,
                    message_id,
                });
            }
        }
        cx.notify();
    }

    /// 列表行双击：在主窗口选中并弹出独立窗口；若正文尚未拉取完成，等正文返回后再弹出完整窗。
    pub(crate) fn open_message_in_window_from_double_click(
        &mut self,
        account_id: u32,
        message_id: u32,
        uid: u32,
        unread: bool,
        folder_path: &str,
        cx: &mut Context<Self>,
    ) {
        if let Some(&handle) = self.popout_message_windows.get(&message_id) {
            let app_entity = cx.entity();
            let folder_path = folder_path.to_string();
            cx.defer(move |cx| {
                if handle
                    .update(cx, |_, window, _| window.activate_window())
                    .is_err()
                {
                    let _ = app_entity.update(cx, |app, cx| {
                        app.unregister_popout_message_window(message_id, Some(handle));
                        app.open_message_in_window_from_double_click(
                            account_id,
                            message_id,
                            uid,
                            unread,
                            &folder_path,
                            cx,
                        );
                    });
                }
            });
            return;
        }
        if self.is_selected_message_ready_for_popout(message_id) {
            if let Some(detail) = self.selected_message.clone() {
                self.pop_out_message_window(detail, cx);
            }
            return;
        }
        if self.pending_body_message_id != Some(message_id) {
            self.select_message(account_id, message_id, uid, unread, folder_path, cx);
        }
        if self.is_selected_message_ready_for_popout(message_id) {
            if let Some(detail) = self.selected_message.clone() {
                self.pop_out_message_window(detail, cx);
            }
            return;
        }
        if let Some((detail, true)) = self.try_load_cached_detail(message_id) {
            self.set_selected_message(detail.clone());
            if unread && self.prefs.read_mark_mode == "shown" {
                self.mark_message_seen_now(account_id, message_id, uid, folder_path);
            }
            self.pop_out_message_window(detail, cx);
            cx.notify();
            return;
        }
        self.pending_popout_message_ids.insert(message_id);
        cx.notify();
    }

    pub(crate) fn pop_out_message_window(&mut self, detail: MessageDetail, cx: &mut Context<Self>) {
        if let Some(&existing) = self.popout_message_windows.get(&detail.id) {
            let app_entity = cx.entity();
            let msg_id = detail.id;
            cx.defer(move |cx| {
                if existing
                    .update(cx, |_, window, _| window.activate_window())
                    .is_err()
                {
                    let _ = app_entity.update(cx, |app, cx| {
                        app.unregister_popout_message_window(msg_id, Some(existing));
                        app.pop_out_message_window(detail, cx);
                    });
                }
            });
            return;
        }
        let msg_id = detail.id;
        let app_entity = cx.entity();
        let app_weak = app_entity.downgrade();
        let prefs = self.prefs.clone();
        let title = if detail.subject.trim().is_empty() {
            "邮件阅读".to_string()
        } else {
            detail.subject.clone()
        };
        let offset = popout_cascade_offset(self.popout_message_windows.len());
        let mut centered = Bounds::centered(None, size(px(860.0), px(720.0)), cx);
        centered.origin.x += px(offset);
        centered.origin.y += px(offset);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(centered)),
            titlebar: Some(TitlebarOptions {
                title: Some(title.clone().into()),
                appears_transparent: true,
                traffic_light_position: None,
            }),
            ..Default::default()
        };
        // 独立窗口首帧会读取主窗口状态，等当前更新释放主窗口实体后再打开。
        cx.defer(move |cx| {
            if let Some(existing) = app_entity
                .read(cx)
                .popout_message_windows
                .get(&msg_id)
                .copied()
            {
                if existing
                    .update(cx, |_, window, _| window.activate_window())
                    .is_ok()
                {
                    return;
                }
                let _ = app_entity.update(cx, |app, _| {
                    app.unregister_popout_message_window(msg_id, Some(existing));
                });
            }
            if let Ok((handle, _entity)) = gpui_kit::open_window(options, cx, move |window, cx| {
                window.set_window_title(&title);
                cx.new(|cx| MessageWindow::new(detail, app_weak, prefs, window, cx))
            }) {
                #[cfg(target_os = "windows")]
                crate::apply_window_icon();
                let _ = app_entity.update(cx, |app, _| {
                    app.popout_message_windows.insert(msg_id, handle);
                });
            }
        });
    }

    pub(crate) fn send_compose(&mut self, cx: &mut Context<Self>) {
        self.send_compose_inner(true, None, cx);
    }

    pub(crate) fn send_compose_forced(&mut self, cx: &mut Context<Self>) {
        self.send_compose_inner(false, None, cx);
    }

    /// WP-10-5 定时发送：走与立即发送完全相同的校验，只是把到期时间换成用户
    /// 挑的那一刻，并且不进入撤销发送倒计时。
    pub(crate) fn send_compose_scheduled(&mut self, at: i64, cx: &mut Context<Self>) {
        self.send_compose_inner(true, Some(at), cx);
    }

    fn send_compose_inner(
        &mut self,
        check_attachments: bool,
        schedule_at: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        if self.quitting
            || (!self.demo_mode
                && self
                    .pending_send
                    .as_ref()
                    .is_some_and(|p| p.outbox_id.is_none()))
        {
            self.status_line = SharedString::from("上一封邮件尚未保存，请先完成或恢复它");
            cx.notify();
            return;
        }
        if self.pending_send.as_ref().is_some_and(|p| p.undo_requested) {
            self.status_line = SharedString::from("正在撤销上一封邮件，请稍候");
            cx.notify();
            return;
        }
        let to = self.compose_field_text(ComposeAddressField::To, cx);
        let cc = self.compose_field_text(ComposeAddressField::Cc, cx);
        let bcc = self.compose_field_text(ComposeAddressField::Bcc, cx);
        let subject = self.compose_subject.read(cx).value().to_string();
        let body = self.compose_body.read(cx).value().to_string();

        if !self.demo_mode && self.accounts.is_empty() {
            self.error_banner = Some(SharedString::from("请先配置并保存邮箱账号"));
            cx.notify();
            return;
        }
        // 发送入口校验（任务一）：所选发件账号必须存在且已启用，停用账号阻断发送。
        if !self.demo_mode
            && !self
                .accounts
                .iter()
                .any(|a| a.id == self.compose_account_id && a.enabled)
        {
            self.error_banner = Some(SharedString::from(
                "当前所选发件账号已停用或不存在，请先选择可用账号再发送",
            ));
            cx.notify();
            return;
        }
        // WP-10-2：自定义 From 与 Reply-To 都必须是可解析的邮箱，否则不发信。
        if !self.demo_mode && !self.compose_from_addr.trim().is_empty() {
            if !RecipientPill::parse(&self.compose_from_addr).is_some_and(|pill| pill.valid) {
                self.error_banner = Some(SharedString::from(
                    "自定义发件人地址格式无效，请填写 `Name <addr>` 或纯邮箱地址",
                ));
                cx.notify();
                return;
            }
        }
        if let Some(problem) = self.compose_reply_to_problem(cx) {
            self.error_banner = Some(SharedString::from(problem));
            cx.notify();
            return;
        }
        if !self.demo_mode && to.trim().is_empty() {
            self.error_banner = Some(SharedString::from("请填写收件人邮箱地址"));
            cx.notify();
            return;
        }
        if check_attachments && self.compose_attachments.is_empty() {
            if let Some(keyword) = detect_attachment_reminder(
                &subject,
                &body,
                &self.prefs.attachment_reminder_keywords,
            ) {
                self.compose_attachment_reminder = Some(keyword);
                cx.notify();
                return;
            }
        }
        self.compose_attachment_reminder = None;

        // 若已有正在倒计时的待发邮件，先立即投递，避免覆盖丢信。
        if let Some(prev) = self.pending_send.as_ref().map(|p| p.token) {
            self.commit_pending_send(prev, cx);
        }
        let delay_secs = self.prefs.undo_send_secs.min(30) as u8;
        // WP-10-5：定时发送把绝对时间折算成剩余秒数交给 worker（`SendDraft.send_at`
        // 的既有语义就是「延迟秒数」），由 worker 定时巡检到点投递。
        let scheduled_secs = match schedule_at {
            Some(at) => {
                let remaining = at - chrono::Local::now().timestamp();
                if remaining <= 0 {
                    self.compose_schedule_error =
                        Some(SharedString::from("定时发送时间必须晚于当前时间"));
                    cx.notify();
                    return;
                }
                Some(remaining)
            }
            None => None,
        };
        let token = self.next_send_token;
        self.next_send_token = self.next_send_token.wrapping_add(1).max(1);
        let pending = PendingSend {
            token,
            account_id: self.compose_account_id,
            draft: ComposeDraft {
                to,
                cc,
                bcc,
                subject,
                body,
                in_reply_to: self.compose_in_reply_to.clone(),
                from_addr: self.compose_from_addr.clone(),
                reply_to: self.compose_reply_to.read(cx).value().to_string(),
                body_format: self.compose_format.as_str().to_string(),
                body_html: self.compose_body_html(cx),
                attachment_paths: self.compose_attachments.clone(),
            },
            local_draft_id: self.compose_draft_id,
            remaining_secs: delay_secs,
            outbox_id: None,
            undo_requested: false,
            commit_requested: false,
            enqueue_failed: false,
        };
        // 任务一：撤销发送先把待发记录持久化到发件箱（Queued + 到期时间）再启动
        // 倒计时；撤销 = 删除该记录，倒计时到期 / 立即发送 = 直接触发投递。原草稿
        // 由 worker 在记录落盘成功后交接删除（保证内容已落盘且不重复）。
        if !self.demo_mode {
            let request = MailRequest::SendDraft {
                account_id: pending.account_id,
                draft: pending.draft.clone(),
                local_draft_id: pending.local_draft_id,
                undo_token: token,
                send_at: Some(scheduled_secs.unwrap_or(delay_secs as i64)),
            };
            let result = self
                .worker
                .as_ref()
                .ok_or_else(|| "邮件后台线程不可用".to_string())
                .and_then(|w| w.send_checked(request));
            if let Err(message) = result {
                self.error_banner = Some(SharedString::from(message));
                cx.notify();
                return;
            }
        }
        // 定时发送不进撤销倒计时：记录已在发件箱排队，等 worker 到点投递即可。
        if scheduled_secs.is_none() {
            self.pending_send = Some(pending);
        }
        self.compose_confirm_discard = false;
        self.compose_autocomplete_field = None;
        self.pending_forward_msg_id = None;
        // F-4：发送即清除挂起的自动保存（草稿由发送流程删除，不再落库）。
        self.cancel_draft_autosave();
        self.compose_autosaved_new = false;
        self.compose_autosaved_at = None;
        self.pending_unsubscribe = None;
        // WP-10-4：发出后分屏 / 独立写信窗口一并收起，回到阅读器。
        self.finish_compose_session(cx);
        self.right_mode = RightPaneMode::Reader;
        self.error_banner = None;

        if let Some(at) = schedule_at {
            self.compose_schedule_open = false;
            self.compose_schedule_error = None;
            let when = compose_schedule::schedule_label(at);
            self.status_line = SharedString::from(if self.demo_mode {
                format!("演示定时发送：{when}")
            } else {
                format!("已排入定时发送：{when}，到点自动发出")
            });
            cx.notify();
            return;
        }

        if self.demo_mode {
            self.start_send_countdown(token, delay_secs, cx);
        } else {
            self.status_line = SharedString::from("正在保存待发送邮件…");
            cx.notify();
        }
    }

    fn start_send_countdown(&mut self, token: u64, delay_secs: u8, cx: &mut Context<Self>) {
        if delay_secs == 0 {
            self.commit_pending_send(token, cx);
            return;
        }

        self.status_line = SharedString::from(format!(
            "邮件将在 {delay_secs} 秒后发出，可随时点击「撤销发送」"
        ));

        cx.spawn(async move |this, cx| {
            for _ in 0..delay_secs {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let keep_going = this
                    .update(cx, |app, cx| {
                        let Some(pending) = app.pending_send.as_mut() else {
                            return false;
                        };
                        if pending.token != token {
                            return false;
                        }
                        if pending.remaining_secs > 1 {
                            pending.remaining_secs -= 1;
                            cx.notify();
                            true
                        } else {
                            app.commit_pending_send(token, cx);
                            false
                        }
                    })
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        })
        .detach();

        cx.notify();
    }

    /// 真正将 `pending_send` 投递给 Worker（倒计时到期或点击「立即发送」）。
    /// 记录已在发送时落盘发件箱，此处仅触发投递；落盘回执未到时挂起等待。
    pub(crate) fn commit_pending_send(&mut self, token: u64, cx: &mut Context<Self>) {
        if self.quitting || self.pending_send.as_ref().map(|p| p.token) != Some(token) {
            return;
        }
        if self.demo_mode {
            let Some(pending) = self.pending_send.take() else {
                return;
            };
            self.status_line = SharedString::from(format!(
                "演示发件成功：收件人=\"{}\"，主题=\"{}\"，附件={} 个",
                if pending.draft.to.is_empty() {
                    "(未填写)"
                } else {
                    &pending.draft.to
                },
                if pending.draft.subject.is_empty() {
                    "(无主题)"
                } else {
                    &pending.draft.subject
                },
                pending.draft.attachment_paths.len()
            ));
            cx.notify();
            return;
        }

        // 撤销请求优先：不投递、不让缓冲失效；记录由撤销流程在落盘回执到达后删除。
        if self.pending_send.as_ref().is_some_and(|p| p.undo_requested) {
            cx.notify();
            return;
        }

        let Some(pending) = self.pending_send.as_mut() else {
            return;
        };
        if pending.outbox_id.is_none() {
            // 记录落盘回执（OutboxEnqueued）尚未到达：挂起投递，回执到达后自动触发。
            pending.commit_requested = true;
            self.status_line = SharedString::from("正在发送...");
            cx.notify();
            return;
        }
        let pending = self.pending_send.take().unwrap();
        if let Some(ref w) = self.worker {
            self.status_line = SharedString::from("正在发送...");
            w.send(MailRequest::RetryOutboxItem {
                account_id: pending.account_id,
                outbox_id: pending.outbox_id.unwrap_or_default(),
            });
        }
        cx.notify();
    }

    /// 撤销发送：删除发件箱排队记录并把缓冲内容恢复到写信页。
    pub(crate) fn undo_pending_send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.quitting {
            return;
        }
        let Some(pending) = self.pending_send.as_mut() else {
            return;
        };
        if pending.undo_requested {
            return;
        }
        if pending.enqueue_failed {
            if self.has_active_compose() {
                self.status_line = SharedString::from("请先关闭当前写信页，再恢复未保存的邮件");
            } else {
                let pending = self.pending_send.take().unwrap();
                self.restore_pending_send_to_compose(&pending, window, cx);
                self.compose_draft_id = pending.local_draft_id;
                self.status_line = SharedString::from("已恢复未保存的邮件，请重新保存或发送");
            }
            cx.notify();
            return;
        }
        pending.undo_requested = true;
        if self.demo_mode {
            let pending = self.pending_send.take().unwrap();
            self.restore_pending_send_to_compose(&pending, window, cx);
            self.status_line = SharedString::from("已撤销发送，返回编辑邮件");
        } else if let (Some(outbox_id), Some(w)) = (pending.outbox_id, self.worker.as_ref()) {
            match w.send_checked(MailRequest::UndoOutboxItem {
                account_id: pending.account_id,
                outbox_id,
                token: pending.token,
            }) {
                Ok(()) => self.status_line = SharedString::from("正在撤销发送…"),
                Err(message) => {
                    pending.undo_requested = false;
                    self.error_banner = Some(SharedString::from(message.clone()));
                    self.status_line = SharedString::from(message);
                }
            }
        }
        cx.notify();
    }

    /// 把撤销缓冲的内容恢复到写信页（原草稿已随落盘交接删除，恢复后由自动保存重建）。
    fn restore_pending_send_to_compose(
        &mut self,
        pending: &PendingSend,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.main_view = MainContentView::Mail;
        self.compose_account_id = pending.account_id;
        self.compose_draft_id = None;
        self.compose_in_reply_to = pending.draft.in_reply_to.clone();
        self.compose_attachments = pending.draft.attachment_paths.clone();
        self.begin_compose_autosave_tracking();
        self.compose_show_cc_bcc = !pending.draft.cc.trim().is_empty()
            || !pending.draft.bcc.trim().is_empty()
            || !pending.draft.reply_to.trim().is_empty();
        let draft = &pending.draft;
        let (to_addr, cc_addr, bcc_addr) = (draft.to.clone(), draft.cc.clone(), draft.bcc.clone());
        self.set_recipient_field_text(ComposeAddressField::To, &to_addr, window, cx);
        self.set_recipient_field_text(ComposeAddressField::Cc, &cc_addr, window, cx);
        self.set_recipient_field_text(ComposeAddressField::Bcc, &bcc_addr, window, cx);
        self.restore_compose_identity(&draft.from_addr, &draft.reply_to, window, cx);
        self.compose_subject
            .update(cx, |s, cx| s.set_value(draft.subject.clone(), window, cx));
        self.compose_format = ComposeFormat::from_str_lossy(&draft.body_format);
        self.compose_body
            .update(cx, |s, cx| s.set_value(draft.body.clone(), window, cx));
        self.compose_confirm_discard = false;
        // WP-10-4：撤销发送回填的邮件整栏呈现（没有原邮件可对照）。
        self.begin_compose_session(false, cx);
        self.right_mode = RightPaneMode::Compose;
    }

    /// 在独立设置窗口编辑或新建指定账号。
    pub(crate) fn open_settings_edit_account(
        &mut self,
        cfg: Option<AccountConfig>,
        cx: &mut Context<Self>,
    ) {
        self.cfg_feedback = None;
        self.open_settings_window(SettingsOpenTarget::EditAccount(cfg), cx);
    }

    /// 欢迎页服务商预设入口：在设置窗口「邮箱账号」直接进入新建页并套用对应服务商。
    pub(crate) fn open_settings_preset_account(
        &mut self,
        provider_idx: usize,
        cx: &mut Context<Self>,
    ) {
        self.cfg_feedback = None;
        self.open_settings_window(SettingsOpenTarget::PresetAccount(provider_idx), cx);
    }

    /// 运行时进入演示模式（欢迎页入口 3）：复用 `--demo` 启动路径的初始化。
    pub(crate) fn enter_demo_mode(&mut self, cx: &mut Context<Self>) {
        if self.demo_mode {
            return;
        }
        self.demo_mode = true;
        let current_page =
            self.demo_store
                .list_messages_page(DEMO_INBOX_ID, "", 0, self.prefs.page_size);
        let selected_message = current_page
            .items
            .first()
            .and_then(|msg| self.demo_message_detail(msg.id));
        self.current_page = current_page;
        if let Some(detail) = selected_message {
            self.set_selected_message(detail);
        } else {
            self.selected_message = None;
        }
        let mut folders_by_account = BTreeMap::new();
        folders_by_account.insert(1, self.demo_store.folders.clone());
        self.folders_by_account = folders_by_account;
        let demo_acc = AccountConfig {
            id: 1,
            display_name: self.demo_store.accounts[0].display_name.clone(),
            email: self.demo_store.accounts[0].email.clone(),
            ..Default::default()
        };
        self.accounts = vec![demo_acc];
        self.selected_folder_id = DEMO_INBOX_ID;
        self.current_view = CurrentView::UnifiedInbox;
        self.reset_list_pagination();
        self.compose_account_id = 1;
        self.right_mode = RightPaneMode::Reader;
        self.main_view = MainContentView::Mail;
        self.error_banner = None;
        self.status_line = SharedString::from("演示模式：已加载 10,000 封模拟邮件");
        cx.notify();
    }

    /// 打开统一设置窗口并定位到「邮箱账号」页（菜单 / 欢迎页 / 命令面板入口），保留当前表单选择。
    pub(crate) fn open_account_manager(&mut self, cx: &mut Context<Self>) {
        self.open_settings_on_tab(PrefTab::MailAccounts, cx);
    }

    /// 确认移除账号：发 `MailRequest::DeleteAccount`（清理配置、缓存与凭据）。
    pub(crate) fn delete_account(&mut self, account_id: u32, cx: &mut Context<Self>) {
        if self.demo_mode {
            return;
        }
        if let Some(ref w) = self.worker {
            w.send(MailRequest::DeleteAccount { account_id });
        }
        cx.notify();
    }

    pub(crate) fn start_outlook_oauth(
        &mut self,
        custom_client_id: String,
        login_hint_email: String,
        editing_account_id: Option<u32>,
        cx: &mut Context<Self>,
    ) {
        if self.demo_mode {
            return;
        }
        self.cfg_feedback = Some((
            true,
            SharedString::from("正在打开系统浏览器，请在微软页面完成 Outlook 授权..."),
        ));
        let login_hint = {
            let trimmed = login_hint_email.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        };
        if let Some(ref w) = self.worker {
            w.send(MailRequest::StartMicrosoftOAuth {
                custom_client_id,
                login_hint_email: login_hint,
                editing_account_id,
            });
        }
        cx.notify();
    }

    pub(crate) fn verify_account_config(
        &mut self,
        config: AccountConfig,
        imap_password: String,
        smtp_password: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.cfg_feedback = Some((true, SharedString::from("正在验证 IMAP / SMTP 连接...")));
        if let Some(ref w) = self.worker {
            w.send(MailRequest::VerifyAccount {
                config,
                imap_password,
                smtp_password,
            });
        }
        cx.notify();
    }

    pub(crate) fn save_account_config(
        &mut self,
        config: AccountConfig,
        imap_password: String,
        smtp_password: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.cfg_feedback = Some((true, SharedString::from("正在保存凭据并启动同步...")));
        if let Some(ref w) = self.worker {
            w.send(MailRequest::SaveAndSyncAccount {
                config,
                imap_password,
                smtp_password,
            });
        }
        cx.notify();
    }

    /// 当前视图对应的远端邮箱是否仍有未拉取的更早历史邮件。
    pub(crate) fn current_view_has_more_remote(&self) -> bool {
        if self.demo_mode || !self.search_query.trim().is_empty() {
            return false;
        }
        match self.current_view {
            // 任务四：统一视图只统计 `enabled && in_unified` 账号（与列表、
            // 未读数、分页拉取同范围）；退出统一视图的账号在独立文件夹视图
            // 走 AccountFolder 分支，不受影响。
            CurrentView::UnifiedInbox => {
                unified_view_has_more_remote(&self.accounts, &self.folders_by_account)
            }
            CurrentView::AccountFolder => self
                .folders_by_account
                .values()
                .flatten()
                .find(|f| f.id == self.selected_folder_id)
                .is_some_and(|f| f.has_more_remote),
            CurrentView::Unread
            | CurrentView::Starred
            | CurrentView::Sent
            | CurrentView::Drafts
            | CurrentView::Archive
            | CurrentView::Junk
            | CurrentView::Trash
            | CurrentView::Snoozed => false,
        }
    }

    /// 滚动位置接近底部（距底不足约 3 行高度）时自动预加载下一批 100 条。
    pub(crate) fn maybe_preload_more(&mut self, cx: &mut Context<Self>) {
        if self.current_view == CurrentView::Drafts && !self.demo_mode {
            return;
        }
        let max_y = self.list_scroll_handle.max_offset().y.abs();
        let offset_y = (-self.list_scroll_handle.offset().y).max(px(0.0));
        if max_y > px(0.0) && offset_y + px(240.0) >= max_y {
            self.trigger_load_more(cx);
        }
    }

    /// 立即预加载下一批邮件（优先从本地缓存/Demo 追加，本地见底后再向远端拉取更早批次）。
    pub(crate) fn trigger_load_more(&mut self, cx: &mut Context<Self>) {
        if self.current_view == CurrentView::Drafts && !self.demo_mode {
            return;
        }
        let batch = self.prefs.page_size.max(1);
        let loaded = self.current_page.items.len();

        // 1. 本地缓存或演示数据还有剩余：直接追加下一批。
        if loaded < self.current_page.total {
            if self.loading_more_local {
                return;
            }
            self.loaded_limit = loaded + batch;
            if self.demo_mode {
                self.reload_current_page(cx);
            } else {
                self.loading_more_local = true;
                self.send_page_request(loaded, batch);
                cx.notify();
            }
            return;
        }

        // 2. 本地已见底，但远端服务器仍有更早历史邮件：发起增量拉取。
        if self.current_view_has_more_remote() && !self.loading_more_remote {
            self.loading_more_remote = true;
            self.loaded_limit = loaded + batch;
            let target_folder = match self.current_view {
                CurrentView::UnifiedInbox => UNIFIED_INBOX_FOLDER_ID,
                CurrentView::AccountFolder => self.selected_folder_id,
                _ => return,
            };
            self.status_line = SharedString::from("正在预加载更早的邮件...");
            if let Some(ref w) = self.worker {
                w.send(MailRequest::LoadOlderMessages {
                    folder_id: target_folder,
                    limit: batch,
                });
            }
            cx.notify();
        }
    }

    /// 任务四：后台历史索引驱动。仅在无在途批次且本轮未结束时向单队列排下一个
    /// 批次——已入队的读信、搜索和发信请求会先于下一批执行，当前批次跑完后
    /// 下一批才排到队尾（事件驱动，无需定时器）。
    fn maybe_drive_backfill(&mut self, cx: &mut Context<Self>) {
        if self.demo_mode || self.quitting || !self.backfill.should_queue() {
            return;
        }
        let Some(ref w) = self.worker else {
            return;
        };
        self.backfill.on_batch_queued();
        w.send(MailRequest::BackfillHistoryBatch);
        cx.notify();
    }

    pub(crate) fn account_email_for(&self, account_id: u32) -> &str {
        self.accounts
            .iter()
            .find(|a| a.id == account_id)
            .map(|a| a.email.as_str())
            .unwrap_or("")
    }

    pub(crate) fn account_label_for(&self, account_id: u32) -> String {
        self.accounts
            .iter()
            .find(|a| a.id == account_id)
            .map(AccountConfig::display_label)
            .unwrap_or_default()
    }

    pub(crate) fn account_color_for(&self, account_id: u32) -> Option<Hsla> {
        self.accounts
            .iter()
            .find(|a| a.id == account_id)
            .map(|a| crate::ui::avatar::account_accent_color(&a.email, a.color.as_deref()))
    }

    pub(crate) fn reorder_accounts(
        &mut self,
        from_idx: usize,
        to_idx: usize,
        cx: &mut Context<Self>,
    ) {
        if self.demo_mode
            || from_idx == to_idx
            || from_idx >= self.accounts.len()
            || to_idx >= self.accounts.len()
        {
            return;
        }
        let moved = self.accounts.remove(from_idx);
        self.accounts.insert(to_idx, moved);
        let account_ids: Vec<u32> = self.accounts.iter().map(|a| a.id).collect();
        if let Some(ref w) = self.worker {
            w.send(MailRequest::ReorderAccounts { account_ids });
        }
        cx.notify();
    }

    pub(crate) fn set_account_enabled(
        &mut self,
        account_id: u32,
        enabled: bool,
        cx: &mut Context<Self>,
    ) {
        if self.demo_mode {
            return;
        }
        let Some(acc) = self.accounts.iter_mut().find(|a| a.id == account_id) else {
            return;
        };
        if acc.enabled == enabled {
            return;
        }
        acc.enabled = enabled;
        // 任务一：停用写信所用账号时不静默切换写信账号；写信界面会提示用户
        // 明确选择可用账号后才能发送。
        if let Some(ref w) = self.worker {
            w.send(MailRequest::SetAccountEnabled {
                account_id,
                enabled,
            });
        }
        cx.notify();
    }

    /// 写信当前所选发件账号是否不可用（真实账号模式下已停用或不存在），
    /// 写信界面据此提示用户明确选择可用账号。
    pub(crate) fn compose_sender_unavailable(&self) -> bool {
        !self.demo_mode
            && !self
                .accounts
                .iter()
                .any(|a| a.id == self.compose_account_id && a.enabled)
    }

    /// 统一收件箱未读总数（仅统计已启用且加入统一收件箱的账号 INBOX 未读之和）。
    pub(crate) fn unified_unread_count(&self) -> u32 {
        self.folders_by_account
            .iter()
            .filter(|(&account_id, _)| {
                self.accounts
                    .iter()
                    .find(|a| a.id == account_id)
                    .map(|a| a.enabled && a.in_unified)
                    .unwrap_or(true)
            })
            .flat_map(|(_, folders)| folders.iter())
            .filter(|f| {
                f.path.eq_ignore_ascii_case("INBOX")
                    || f.name.eq_ignore_ascii_case("INBOX")
                    || f.name.contains("收件箱")
            })
            .map(|f| f.unread_count)
            .sum()
    }

    pub(crate) fn account_expanded(&self, account_id: u32) -> bool {
        !self.collapsed_accounts.contains(&account_id)
    }

    // ---- 迭代 1 新增的纯视觉动作 ----

    /// 菜单「切换亮/暗主题」：即时生效，并把选择固化为 `Some(bool)` 持久化。
    pub(crate) fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        self.dark_mode = !self.dark_mode;
        self.prefs.dark_mode = Some(self.dark_mode);
        theme::apply(self.theme_id, self.dark_mode, cx);
        if let Err(e) = config::save_preferences_in(&config::data_dir(), &self.prefs) {
            self.error_banner = Some(SharedString::from(e));
        }
        self.status_line = SharedString::from(if self.dark_mode {
            "已切换到暗色主题"
        } else {
            "已切换到亮色主题"
        });
        cx.notify();
    }

    /// 左栏当前是否处于 80px 图标窄轨：手动折叠或断点自动折叠任一成立
    /// （对齐 Linux rail_wanted = auto_rail || sidebar_collapsed）。
    pub(crate) fn sidebar_narrow(&self) -> bool {
        self.sidebar_hidden || self.auto_rail_active
    }

    /// P3：窄窗口浮层侧栏开关（中栏顶部按钮）。断点自动窄轨生效时不再原地
    /// 展开窄轨，而是打开 / 关闭覆盖在中栏之上的完整侧栏浮层（对齐 Linux
    /// toggle_sidebar：narrow 窗口下 toggle 只浮动 peek 面板，不落盘）。
    pub(crate) fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        if self.auto_rail_active {
            self.sidebar_peek_open = !self.sidebar_peek_open;
            cx.notify();
            return;
        }
        self.sidebar_hidden = !self.sidebar_hidden;
        if self.prefs.remember_sidebar_collapsed {
            self.prefs.sidebar_collapsed = self.sidebar_hidden;
            let _ = config::save_preferences_in(&config::data_dir(), &self.prefs);
        }
        cx.notify();
    }

    /// P3：关闭窄窗口浮层侧栏（导航 / 选择完成后调用，对齐 Linux
    /// close_sidebar_peek：在浮层里点完导航即收起，浮层不再遮挡中栏）。
    pub(crate) fn close_sidebar_peek(&mut self) {
        if self.sidebar_peek_open {
            self.sidebar_peek_open = false;
        }
    }

    // ---- L-2：三栏宽度拖拽与持久化 ----

    /// 当前左栏在界面上的实际宽度（窄轨模式为固定 80px，不参与拖拽）。
    pub(crate) fn sidebar_visual_width(&self) -> f32 {
        if self.sidebar_narrow() {
            sidebar::RAIL_WIDTH
        } else {
            self.sidebar_width
        }
    }

    /// 把当前三栏宽度写入偏好副本并立即落盘（拖拽结束 / 退出时调用）。
    fn persist_layout_prefs(&mut self) {
        self.prefs.sidebar_width = Some(self.sidebar_width);
        self.prefs.list_width = Some(self.list_width);
        self.prefs.list_below_height = Some(self.list_below_height);
        let _ = config::save_preferences_in(&config::data_dir(), &self.prefs);
    }

    /// 宽度拖拽进行中调用：延迟 600ms 防抖落盘（对齐原版 mail_split.rs 的 SAVE_AFTER）。
    fn schedule_layout_save(&mut self, cx: &mut Context<Self>) {
        self.layout_save_deadline = Some(Instant::now() + LAYOUT_SAVE_DEBOUNCE);
        if self.layout_save_scheduled {
            return;
        }
        self.layout_save_scheduled = true;
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(120))
                .await;
            // 返回 true 表示本轮应结束（无待写入或已完成写入；实体释放也结束）。
            let done = this
                .update(cx, |app, _cx| match app.layout_save_deadline {
                    None => true,
                    Some(deadline) => {
                        if Instant::now() >= deadline {
                            app.layout_save_deadline = None;
                            app.layout_save_scheduled = false;
                            app.persist_layout_prefs();
                            true
                        } else {
                            false
                        }
                    }
                })
                .unwrap_or(true);
            if done {
                break;
            }
        })
        .detach();
    }

    /// 左栏拖拽把手移动：宽度 = 指针 x − 内容行左缘，钳制 180~280。
    fn on_sidebar_drag_move(&mut self, event: &DragMoveEvent<SidebarDrag>, cx: &mut Context<Self>) {
        let pointer_x = event.event.position.x.as_f32();
        let row_left = event.bounds.left().as_f32();
        self.sidebar_width = (pointer_x - row_left).clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH);
        self.schedule_layout_save(cx);
        cx.notify();
    }

    /// 列表 | 右栏分隔把手移动（Right 模式）：列表宽 = 指针 x − 行左缘 − 左栏宽，
    /// 右栏保底 READER_MIN_WIDTH；同时按迟滞阈值更新单行表格模式状态。
    fn on_list_drag_move(&mut self, event: &DragMoveEvent<ListDrag>, cx: &mut Context<Self>) {
        let pointer_x = event.event.position.x.as_f32();
        let row_left = event.bounds.left().as_f32();
        let row_width = event.bounds.size.width.as_f32();
        let max_list =
            (row_width - self.sidebar_visual_width() - READER_MIN_WIDTH).max(LIST_MIN_WIDTH);
        self.list_width =
            (pointer_x - row_left - self.sidebar_visual_width()).clamp(LIST_MIN_WIDTH, max_list);
        // 原版迟滞：进入宽表格 ≥600，退出 <560，两侧留余量防拖拽抖动。
        self.list_wide = list_wide_next(self.list_wide, self.list_width);
        self.schedule_layout_save(cx);
        cx.notify();
    }

    /// 列表 | 下方阅读器分隔把手移动（Below 模式）：列表高 = 指针 y − 行上缘，
    /// 阅读器保底 BELOW_PANE_MIN_HEIGHT。
    fn on_below_drag_move(&mut self, event: &DragMoveEvent<BelowDrag>, cx: &mut Context<Self>) {
        let pointer_y = event.event.position.y.as_f32();
        let row_top = event.bounds.top().as_f32();
        let row_height = event.bounds.size.height.as_f32();
        let max_list = (row_height - BELOW_PANE_MIN_HEIGHT).max(BELOW_LIST_MIN_HEIGHT);
        self.list_below_height = (pointer_y - row_top).clamp(BELOW_LIST_MIN_HEIGHT, max_list);
        self.schedule_layout_save(cx);
        cx.notify();
    }

    // ---- L-6：阅读器位置与专注模式 ----

    /// 阅读器 / 写信窗格是否位于窗口顶部（决定窗口控制按钮放阅读器顶栏还是列表顶栏，
    /// 对齐原版 controls_in_list：阅读器在下方或关闭时列表顶栏即窗口顶栏）。
    pub(crate) fn pane_at_window_top(&self) -> bool {
        self.focus_mode
            || self.main_view != MainContentView::Mail
            || self.right_mode == RightPaneMode::Welcome
            || self.reader_position == ReaderPosition::Right
    }

    /// 切换阅读器位置（菜单 / 快捷键入口），立即落盘（对齐原版 pref! 保存时机）。
    pub(crate) fn set_reader_position(&mut self, position: ReaderPosition, cx: &mut Context<Self>) {
        if self.reader_position == position {
            return;
        }
        self.reader_position = position;
        self.prefs.reader_position = position.key().to_string();
        let _ = config::save_preferences_in(&config::data_dir(), &self.prefs);
        self.status_line = SharedString::from(match position {
            ReaderPosition::Right => "阅读器已移到列表右侧",
            ReaderPosition::Below => "阅读器已移到列表下方",
            ReaderPosition::Hidden => "阅读器已关闭：邮件仅选中，按 Ctrl+Alt+1 重新打开",
        });
        cx.notify();
    }

    /// WP-10：欢迎向导设置主题深浅模式（`None` 跟随系统，`Some(true/false)` 暗/亮）。
    pub(crate) fn set_welcome_dark_mode(&mut self, mode: Option<bool>, cx: &mut Context<Self>) {
        self.prefs.dark_mode = mode;
        self.dark_mode = mode.unwrap_or(self.system_dark_mode);
        let _ = config::save_preferences_in(&config::data_dir(), &self.prefs);
        cx.notify();
    }

    /// WP-10：欢迎向导持久化偏好变更。
    pub(crate) fn save_welcome_prefs(&mut self, cx: &mut Context<Self>) {
        let _ = config::save_preferences_in(&config::data_dir(), &self.prefs);
        cx.notify();
    }

    /// WP-10：完成欢迎向导，有账号则进入收件箱，无账号则进入演示模式。
    pub(crate) fn finish_welcome_wizard(&mut self, cx: &mut Context<Self>) {
        let _ = config::save_preferences_in(&config::data_dir(), &self.prefs);
        if self.accounts.is_empty() {
            self.enter_demo_mode(cx);
        } else {
            self.right_mode = RightPaneMode::Reader;
            cx.notify();
        }
    }

    /// 切换专注模式（会话级）：按专注模式偏好控制侧栏、列表、简化阅读视图及工具栏。
    pub(crate) fn toggle_focus_mode(&mut self, cx: &mut Context<Self>) {
        self.focus_mode = !self.focus_mode;
        if self.focus_mode {
            if self.prefs.focus_reader_view {
                self.focus_saved_prefer_plain = Some(self.reader_prefer_plain);
                self.reader_prefer_plain = true;
            }
            self.status_line = SharedString::from("专注模式已开启（Ctrl+Shift+F 恢复）");
        } else {
            if let Some(prev) = self.focus_saved_prefer_plain.take() {
                self.reader_prefer_plain = prev;
            }
            self.status_line = SharedString::from("专注模式已关闭");
        }
        cx.notify();
    }

    /// WP-05：设置窗口的专注模式开关（与快捷键 / 主菜单切换同一行为路径，
    /// 仅改运行时状态，不写偏好）。
    pub(crate) fn set_focus_mode(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.focus_mode != on {
            self.toggle_focus_mode(cx);
        }
    }

    pub(crate) fn open_mail_link(&mut self, value: &str, cx: &mut Context<Self>) {
        if let Some(link) = crate::webview_host::prepared_mail_link(value, self.prefs.clean_links) {
            if link.starts_with("mailto:") {
                cx.open_url(&link);
            } else {
                self.open_link(&link, cx);
            }
        }
    }

    pub(crate) fn open_clicked_mail_link(
        app: &gpui_kit::WeakEntity<Self>,
        url: &str,
        event: &gpui_kit::ClickEvent,
        cx: &mut gpui_kit::App,
    ) {
        let activate = match event {
            gpui_kit::ClickEvent::Mouse(click) => matches!(
                click.up.button,
                gpui_kit::MouseButton::Left | gpui_kit::MouseButton::Middle
            ),
            gpui_kit::ClickEvent::Keyboard(_) => true,
            gpui_kit::ClickEvent::Touch(click) => !click.long_press,
        };
        if activate {
            let _ = app.update(cx, |app, cx| app.open_mail_link(url, cx));
        }
    }

    pub(crate) fn remote_images_allowed(&self, id: u32, sender: &str) -> bool {
        !self.prefs.block_remote_images
            || self.allowed_remote_image_ids.contains(&id)
            || self.prefs.is_remote_sender_allowed(sender)
    }

    /// WP-05：按「系统 → 链接打开方式」偏好打开邮件内链接。`system` 走
    /// gpui 的系统 shell；指定浏览器时优先探测其安装路径并用其打开，
    /// 未安装时回退系统默认。
    pub(crate) fn open_link(&mut self, url: &str, cx: &mut Context<Self>) {
        let browser = match self.prefs.link_open_mode.as_str() {
            "edge" => (
                "msedge.exe",
                [
                    r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
                    r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
                ],
            ),
            "chrome" => (
                "chrome.exe",
                [
                    r"C:\Program Files\Google\Chrome\Application\chrome.exe",
                    r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
                ],
            ),
            "firefox" => (
                "firefox.exe",
                [
                    r"C:\Program Files\Mozilla Firefox\firefox.exe",
                    r"C:\Program Files (x86)\Mozilla Firefox\firefox.exe",
                ],
            ),
            _ => {
                cx.open_url(url);
                return;
            }
        };
        let exe = browser
            .1
            .into_iter()
            .find(|p| std::path::Path::new(p).is_file());
        match exe {
            Some(path) => {
                let _ = std::process::Command::new(path).arg(url).spawn();
            }
            None => {
                // 指定浏览器未找到：回退系统默认。
                cx.open_url(url);
            }
        }
    }

    // ---- B-5：标题区视图切换 ----

    /// 切换单封邮件正文的亮 / 暗背景覆盖（会话级，键为邮件 ID；不持久化）。
    /// 首次点击取与当前主题相反的背景，再次点击在亮 / 暗之间切换。
    pub(crate) fn toggle_message_bg_pick(&mut self, message_id: u32, cx: &mut Context<Self>) {
        let next = match self.message_bg_picks.get(&message_id).copied() {
            None => !self.dark_mode,
            Some(picked_dark) => !picked_dark,
        };
        self.message_bg_picks.insert(message_id, next);
        self.status_line = SharedString::from(if next {
            "此邮件正文已强制暗色背景（仅本会话）"
        } else {
            "此邮件正文已强制亮色背景（仅本会话）"
        });
        cx.notify();
    }

    pub(crate) fn set_list_sort(&mut self, sort: MessageSort, cx: &mut Context<Self>) {
        if self.list_sort != sort {
            self.list_sort = sort;
            self.reset_list_pagination();
            self.reload_current_page(cx);
        } else {
            cx.notify();
        }
    }

    pub(crate) fn toggle_quick_filter_unread(&mut self, cx: &mut Context<Self>) {
        self.quick_filters.unread = !self.quick_filters.unread;
        self.reset_list_pagination();
        self.reload_current_page(cx);
    }

    pub(crate) fn toggle_quick_filter_starred(&mut self, cx: &mut Context<Self>) {
        self.quick_filters.starred = !self.quick_filters.starred;
        self.reset_list_pagination();
        self.reload_current_page(cx);
    }

    pub(crate) fn toggle_quick_filter_attachment(&mut self, cx: &mut Context<Self>) {
        self.quick_filters.has_attachment = !self.quick_filters.has_attachment;
        self.reset_list_pagination();
        self.reload_current_page(cx);
    }

    pub(crate) fn toggle_quick_filter_today(&mut self, cx: &mut Context<Self>) {
        self.quick_filters.today = !self.quick_filters.today;
        self.reset_list_pagination();
        self.reload_current_page(cx);
    }

    pub(crate) fn toggle_quick_filter_to_me(&mut self, cx: &mut Context<Self>) {
        self.quick_filters.to_me = !self.quick_filters.to_me;
        self.reset_list_pagination();
        self.reload_current_page(cx);
    }

    pub(crate) fn toggle_quick_filter_from_contacts(&mut self, cx: &mut Context<Self>) {
        self.quick_filters.from_contacts = !self.quick_filters.from_contacts;
        self.reset_list_pagination();
        self.reload_current_page(cx);
    }

    pub(crate) fn toggle_quick_filter_high_importance(&mut self, cx: &mut Context<Self>) {
        self.quick_filters.high_importance = !self.quick_filters.high_importance;
        self.reset_list_pagination();
        self.reload_current_page(cx);
    }

    pub(crate) fn toggle_quick_filter_tagged(&mut self, cx: &mut Context<Self>) {
        self.quick_filters.tagged = !self.quick_filters.tagged;
        self.reset_list_pagination();
        self.reload_current_page(cx);
    }

    pub(crate) fn set_quick_filter_date_range(
        &mut self,
        from_ts: Option<i64>,
        to_ts: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        self.quick_filters.date_from_ts = from_ts;
        self.quick_filters.date_to_ts = to_ts;
        self.reset_list_pagination();
        self.reload_current_page(cx);
    }

    /// WP-08：宽表列宽查询（带默认宽度）。
    pub(crate) fn column_width(&self, col: hylki_mail_core::config::ListColumn) -> f32 {
        use hylki_mail_core::config::ListColumn;
        if let Some(&w) = self.list_column_widths.get(&col) {
            return w;
        }
        match col {
            ListColumn::Star => 22.0,
            ListColumn::Replied => 28.0,
            ListColumn::Attachment => 22.0,
            ListColumn::Importance => 22.0,
            ListColumn::Sender | ListColumn::Recipients => 140.0,
            ListColumn::Correspondents => 150.0,
            ListColumn::Account => 96.0,
            ListColumn::Tags => 96.0,
            ListColumn::Date | ListColumn::Due => 86.0,
            ListColumn::Subject => 220.0,
        }
    }

    /// WP-08：拖拽列分隔线实时调整列宽（32px..=500px）。
    pub(crate) fn on_column_drag_move(
        &mut self,
        event: &DragMoveEvent<ColumnResizeDrag>,
        cx: &mut Context<Self>,
    ) {
        let drag = event.drag(cx);
        let delta = event.event.position.x.as_f32() - drag.start_x;
        let next = (drag.start_width + delta).clamp(32.0, 500.0);
        self.list_column_widths.insert(drag.column, next);
        cx.notify();
    }

    /// WP-08：展开 / 收起会话内联子邮件树。
    pub(crate) fn toggle_thread_expand(&mut self, msg: &MessageSummary, cx: &mut Context<Self>) {
        if self.expanded_threads.contains(&msg.id) {
            self.expanded_threads.remove(&msg.id);
            cx.notify();
            return;
        }
        self.expanded_threads.insert(msg.id);
        if self.demo_mode {
            let children = self.demo_store.list_thread_summaries(msg.id);
            self.thread_children_cache.insert(msg.id, children);
            cx.notify();
        } else if !self.thread_children_cache.contains_key(&msg.id) {
            if let Some(ref w) = self.worker {
                w.send(MailRequest::LoadThreadChildren {
                    account_id: msg.account_id,
                    message_id: msg.id,
                    subject: msg.subject.clone(),
                });
            }
            cx.notify();
        } else {
            cx.notify();
        }
    }

    /// WP-08：切换日期分组折叠/展开。
    pub(crate) fn toggle_day_group(&mut self, label: &str, cx: &mut Context<Self>) {
        if self.collapsed_day_groups.contains(label) {
            self.collapsed_day_groups.remove(label);
        } else {
            self.collapsed_day_groups.insert(label.to_string());
        }
        cx.notify();
    }

    pub(crate) fn collapse_all_day_groups(&mut self, cx: &mut Context<Self>) {
        for label in hylki_mail_core::DATE_GROUP_LABELS {
            self.collapsed_day_groups.insert(label.to_string());
        }
        cx.notify();
    }

    pub(crate) fn expand_all_day_groups(&mut self, cx: &mut Context<Self>) {
        self.collapsed_day_groups.clear();
        cx.notify();
    }

    /// WP-08：快捷打标签（1..9）。
    pub(crate) fn toggle_message_tag(
        &mut self,
        account_id: u32,
        message_id: u32,
        tag: &str,
        cx: &mut Context<Self>,
    ) {
        let mut next_tags = self
            .current_page
            .items
            .iter()
            .find(|m| m.id == message_id)
            .map(|m| m.tags.clone())
            .unwrap_or_default();
        if let Some(pos) = next_tags.iter().position(|t| t == tag) {
            next_tags.remove(pos);
        } else {
            next_tags.push(tag.to_string());
        }
        for m in &mut self.current_page.items {
            if m.id == message_id {
                m.tags = next_tags.clone();
            }
        }
        if !self.demo_mode {
            if let Some(ref w) = self.worker {
                w.send(MailRequest::SetMessageTags {
                    account_id,
                    message_id,
                    tags: next_tags,
                });
            }
        } else {
            self.status_line = SharedString::from(format!("已更新标签：{tag}"));
        }
        cx.notify();
    }

    pub(crate) fn clear_message_tags(
        &mut self,
        account_id: u32,
        message_id: u32,
        cx: &mut Context<Self>,
    ) {
        for m in &mut self.current_page.items {
            if m.id == message_id {
                m.tags.clear();
            }
        }
        if !self.demo_mode {
            if let Some(ref w) = self.worker {
                w.send(MailRequest::SetMessageTags {
                    account_id,
                    message_id,
                    tags: Vec::new(),
                });
            }
        } else {
            self.status_line = SharedString::from("已清除邮件标签");
        }
        cx.notify();
    }

    /// WP-08：稍后提醒（Snooze）。
    pub(crate) fn snooze_message(
        &mut self,
        account_id: u32,
        message_id: u32,
        until_ts: i64,
        label: &str,
        cx: &mut Context<Self>,
    ) {
        for m in &mut self.current_page.items {
            if m.id == message_id {
                m.snoozed_until_ts = until_ts;
            }
        }
        if !self.demo_mode {
            if let Some(ref w) = self.worker {
                w.send(MailRequest::SnoozeMessage {
                    account_id,
                    message_id,
                    until_ts,
                });
            }
        }
        self.status_line = SharedString::from(if until_ts > 0 {
            format!("已设置稍后提醒：{label}")
        } else {
            "已取消稍后提醒".to_string()
        });
        cx.notify();
    }

    /// WP-08：跨账号复制到文件夹。
    pub(crate) fn copy_message_to(
        &mut self,
        account_id: u32,
        message_id: u32,
        target_account_id: u32,
        target_folder_path: String,
        cx: &mut Context<Self>,
    ) {
        if self.demo_mode {
            self.status_line =
                SharedString::from(format!("演示模式：已模拟复制到 {target_folder_path}"));
            cx.notify();
            return;
        }
        if let Some(ref w) = self.worker {
            w.send(MailRequest::CopyMessage {
                account_id,
                message_id,
                target_account_id,
                target_folder_path,
            });
        }
        cx.notify();
    }

    /// WP-08：将发件人添加到通讯录。
    pub(crate) fn add_sender_to_contacts(&mut self, sender: &str, cx: &mut Context<Self>) {
        let (name, email) = split_sender(sender);
        let target_email = if email.is_empty() {
            sender.trim().to_string()
        } else {
            email
        };
        if target_email.is_empty() {
            return;
        }
        let display_name = if name.is_empty() {
            target_email.clone()
        } else {
            name
        };
        self.save_user_contact(
            None,
            target_email.clone(),
            display_name,
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            Vec::new(),
            cx,
        );
        self.status_line = SharedString::from(format!("已将 {target_email} 添加到通讯录"));
        cx.notify();
    }

    /// WP-08：以此邮件创建过滤器。
    pub(crate) fn create_filter_from_message(
        &mut self,
        sender: &str,
        subject: &str,
        cx: &mut Context<Self>,
    ) {
        let (_, email) = split_sender(sender);
        let addr = if email.is_empty() {
            sender.trim().to_string()
        } else {
            email
        };
        self.status_line = SharedString::from(format!(
            "已生成过滤条件模板：发件人={addr}，主题包含「{subject}」"
        ));
        cx.notify();
    }

    pub(crate) fn clear_quick_filters(&mut self, cx: &mut Context<Self>) {
        self.quick_filters.clear();
        self.reset_list_pagination();
        self.reload_current_page(cx);
    }

    pub(crate) fn set_search_scope(&mut self, scope: SearchScope, cx: &mut Context<Self>) {
        if self.search_scope != scope {
            self.search_scope = scope;
            self.reset_list_pagination();
            self.reload_current_page(cx);
        } else {
            cx.notify();
        }
    }

    pub(crate) fn clear_server_search(&mut self) {
        self.server_search_loading = false;
        self.server_search_query.clear();
        self.server_search_results.clear();
    }

    pub(crate) fn trigger_server_search(&mut self, cx: &mut Context<Self>) {
        let query = self.search_query.trim().to_string();
        if query.chars().count() < 2 || self.demo_mode || self.server_search_loading {
            return;
        }
        let account_id = if self.search_scope == SearchScope::ThisFolder
            && self.current_view == CurrentView::AccountFolder
        {
            self.folders_by_account
                .iter()
                .find(|(_, fs)| fs.iter().any(|f| f.id == self.selected_folder_id))
                .map(|(aid, _)| *aid)
        } else {
            None
        };
        self.server_search_loading = true;
        self.server_search_query = query.clone();
        self.status_line = SharedString::from(format!("正在搜索服务器：{query}..."));
        if let Some(ref w) = self.worker {
            w.send(MailRequest::SearchServer { query, account_id });
        }
        cx.notify();
    }

    pub(crate) fn toggle_account_expanded(&mut self, account_id: u32, cx: &mut Context<Self>) {
        if self.collapsed_accounts.contains(&account_id) {
            self.collapsed_accounts.remove(&account_id);
        } else {
            self.collapsed_accounts.insert(account_id);
        }
        cx.notify();
    }

    // ---- 阶段二：主菜单补齐（对齐原版 boot.rs / window.rs 的菜单动作）----

    /// 主菜单「显示 / 隐藏账号列表」（Ctrl+Shift+A）：对齐原版
    /// `app.show-accounts`——隐藏或恢复侧栏的账号区段（统一导航保持不动，
    /// 不是隐藏整个侧栏，故不复用 toggle_sidebar）。
    pub(crate) fn toggle_accounts_list(&mut self, cx: &mut Context<Self>) {
        let next = !self.show_accounts;
        self.set_show_accounts(next, cx);
    }

    /// F11 全屏切换（对齐原版 `win.fullscreen`）。
    pub(crate) fn toggle_fullscreen(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
        window.toggle_fullscreen();
    }

    /// F1 / ? 快捷键速查弹窗（原版同键可再次按下关闭）。
    pub(crate) fn open_shortcuts(&mut self, cx: &mut Context<Self>) {
        if self.active_modal == ActiveModal::Shortcuts {
            self.active_modal = ActiveModal::None;
        } else {
            self.active_modal = ActiveModal::Shortcuts;
        }
        cx.notify();
    }

    /// 「关于 Hylki」弹窗（对齐原版 win.about）。
    pub(crate) fn open_about(&mut self, cx: &mut Context<Self>) {
        self.active_modal = ActiveModal::About;
        cx.notify();
    }

    /// 检查 GitHub 最新发布；`manual` 为 true 时在关于浮层展示进度与结论。
    pub(crate) fn check_updates(&mut self, manual: bool, cx: &mut Context<Self>) {
        if self.update.checking || self.update.downloading {
            return;
        }
        self.update.checking = true;
        self.update.error = None;
        if manual {
            self.update.status = Some(SharedString::from("正在检查更新…"));
        }
        cx.notify();
        let current = app_version();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { hylki_mail_core::update::check(current) })
                .await;
            let _ = this.update(cx, |app, cx| {
                app.update.checking = false;
                match result {
                    Ok(Some(info)) => {
                        app.update.status = Some(SharedString::from(format!(
                            "发现新版本 v{}（当前 v{}）",
                            info.version, info.current_version
                        )));
                        app.status_line = SharedString::from(format!(
                            "发现新版本 v{}，可在「关于 Hylki」中更新",
                            info.version
                        ));
                        app.update.info = Some(info);
                    }
                    Ok(None) => {
                        app.update.info = None;
                        app.update.status = Some(SharedString::from("当前已是最新版本"));
                    }
                    Err(error) => {
                        app.update.error = Some(SharedString::from(error));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 下载已发现的更新并安装；安装版静默运行安装包，便携版替换 exe 后重启。
    pub(crate) fn download_and_install_update(&mut self, cx: &mut Context<Self>) {
        let Some(info) = self.update.info.clone() else {
            return;
        };
        if self.update.downloading {
            return;
        }
        self.update.downloading = true;
        self.update.error = None;
        self.update.status = Some(SharedString::from(format!("正在下载 v{}…", info.version)));
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let bytes = hylki_mail_core::update::download(&info)?;
                    Ok::<_, String>((info, bytes))
                })
                .await;
            let _ = this.update(cx, |app, cx| match result {
                Ok((info, bytes)) => {
                    app.update.pending_install = Some((info, bytes));
                    app.status_line = SharedString::from("正在安装更新，程序即将重启…");
                    cx.notify();
                    app.quit_app(cx);
                }
                Err(error) => {
                    app.update.downloading = false;
                    app.update.status = None;
                    app.update.error = Some(SharedString::from(error));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    // ---- 阶段二：文件夹右键动作（对齐原版 sidebar/menus.rs 的 folder_menu_items）----

    /// 右键「刷新此文件夹」：复用现有 `MailRequest::SyncFolder`（demo 模式禁用）。
    pub(crate) fn refresh_folder(
        &mut self,
        account_id: u32,
        folder_id: u32,
        folder_path: String,
        cx: &mut Context<Self>,
    ) {
        if self.demo_mode {
            return;
        }
        if let Some(ref w) = self.worker {
            self.status_line = SharedString::from("正在刷新文件夹...");
            w.send(MailRequest::SyncFolder {
                account_id,
                folder_id,
                folder_path,
            });
        }
        cx.notify();
    }

    /// 右键「全部标为已读」：对一个账号文件夹批量发起 `MarkFolderAllRead`
    /// 并乐观清除当前列表中该文件夹的未读标记（demo 模式禁用）。
    pub(crate) fn mark_folder_all_read(
        &mut self,
        account_id: u32,
        folder_id: u32,
        folder_path: String,
        cx: &mut Context<Self>,
    ) {
        if self.demo_mode {
            return;
        }
        let current_folder =
            self.current_view == CurrentView::AccountFolder && self.selected_folder_id == folder_id;
        if current_folder || self.current_view == CurrentView::UnifiedInbox {
            for item in self
                .current_page
                .items
                .iter_mut()
                .filter(|m| m.folder_id == folder_id)
            {
                item.unread = false;
            }
        }
        if let Some(ref w) = self.worker {
            self.status_line = SharedString::from("正在把文件夹全部标为已读...");
            w.send(MailRequest::MarkFolderAllRead {
                account_id,
                folder_id,
                folder_path,
            });
        }
        cx.notify();
    }

    /// 统一收件箱行右键「全部标为已读」：对所有账号的收件箱批量发起，
    /// 并乐观清除当前统一收件箱页面的未读标记（demo 模式禁用）。
    pub(crate) fn mark_all_inboxes_read(&mut self, cx: &mut Context<Self>) {
        if self.demo_mode {
            return;
        }
        let targets: Vec<(u32, u32, String)> = self
            .folders_by_account
            .values()
            .flatten()
            .filter(|f| f.path.eq_ignore_ascii_case("INBOX") || f.name.contains("收件箱"))
            .map(|f| (f.account_id, f.id, f.path.clone()))
            .collect();
        if targets.is_empty() {
            return;
        }
        if let Some(ref w) = self.worker {
            for (account_id, folder_id, folder_path) in &targets {
                w.send(MailRequest::MarkFolderAllRead {
                    account_id: *account_id,
                    folder_id: *folder_id,
                    folder_path: folder_path.clone(),
                });
            }
        }
        if self.current_view == CurrentView::UnifiedInbox {
            let inbox_folder_ids: HashSet<u32> = targets.iter().map(|(_, id, _)| *id).collect();
            for item in self.current_page.items.iter_mut() {
                if inbox_folder_ids.contains(&item.folder_id) {
                    item.unread = false;
                }
            }
        }
        self.status_line = SharedString::from(format!(
            "正在把 {} 个账号的收件箱全部标为已读...",
            targets.len()
        ));
        cx.notify();
    }

    /// 右键 Trash / Junk「清空文件夹…」：进入二次确认（对齐原版
    /// Empty Trash… 的确认交互；Graph 账号已在菜单禁用）。
    pub(crate) fn request_empty_folder(
        &mut self,
        account_id: u32,
        folder_id: u32,
        folder_path: String,
        folder_name: String,
        cx: &mut Context<Self>,
    ) {
        self.pending_empty_folder = Some((account_id, folder_id, folder_path, folder_name));
        cx.notify();
    }

    /// 取消「清空文件夹」确认。
    pub(crate) fn cancel_empty_folder(&mut self, cx: &mut Context<Self>) {
        self.pending_empty_folder = None;
        cx.notify();
    }

    /// 确认清空：发 `MailRequest::EmptyFolder` 并乐观清空当前列表中该文件夹的条目。
    pub(crate) fn confirm_empty_folder(&mut self, cx: &mut Context<Self>) {
        let Some((account_id, folder_id, folder_path, _)) = self.pending_empty_folder.take() else {
            return;
        };
        if self.current_view == CurrentView::AccountFolder && self.selected_folder_id == folder_id {
            self.current_page.items.clear();
            self.current_page.total = 0;
            self.selected_message = None;
        }
        if let Some(ref w) = self.worker {
            self.status_line = SharedString::from("正在清空文件夹...");
            w.send(MailRequest::EmptyFolder {
                account_id,
                folder_id,
                folder_path,
            });
        }
        cx.notify();
    }

    // ---- 阶段 3：全局路由、偏好、命令面板、快捷键（方案文档第五节）----

    /// 附件总览数据请求（进入视图 / 搜索词变化时触发；demo 模式直接加载模拟附件）。
    fn request_gallery_items(&mut self, cx: &mut Context<Self>) {
        if self.demo_mode {
            let items = self
                .demo_store
                .list_cached_attachments(&self.gallery_query, 0, 200);
            self.gallery_items = items.clone();
            if let Some(ref g) = self.gallery {
                g.update(cx, |state, cx| {
                    state.set_items(items);
                    cx.notify();
                });
            }
            return;
        }
        if let Some(ref w) = self.worker {
            w.send(MailRequest::ListCachedAttachments {
                query: self.gallery_query.clone(),
                offset: 0,
                limit: 200,
            });
        }
    }

    /// 进入附件总览主视图（侧栏底部入口）。
    pub(crate) fn open_gallery(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.gallery.is_none() {
            let entity = cx.new(|cx| GalleryState::new(window, cx));
            let sub = cx.subscribe_in(
                &entity,
                window,
                |this, _gallery, event: &GalleryEvent, _window, cx| match event {
                    GalleryEvent::QueryChanged(query) => {
                        this.gallery_query = query.clone();
                        this.request_gallery_items(cx);
                    }
                    // 账号筛选由组件内部做客户端过滤，无需重新请求。
                    GalleryEvent::AccountFilterChanged(_) => {}
                    GalleryEvent::SaveAttachment(attachment_id) => {
                        this.save_gallery_attachment(*attachment_id, cx);
                    }
                    GalleryEvent::LocateMessage(message_id) => {
                        this.locate_gallery_message(*message_id, cx);
                    }
                    // F-6：图片附件预览——读取本地缓存的附件字节后回填浮层。
                    GalleryEvent::PreviewImage(attachment_id) => {
                        this.fetch_gallery_preview(*attachment_id, cx);
                    }
                    // WP-10：网格视图请求图片缩略图。
                    GalleryEvent::RequestThumbnail(attachment_id) => {
                        if !this.demo_mode {
                            if let Some(ref w) = this.worker {
                                w.send(MailRequest::FetchAttachmentBytes {
                                    attachment_id: *attachment_id,
                                });
                            }
                        }
                    }
                },
            );
            self.gallery = Some(entity);
            self.gallery_sub = Some(sub);
        }
        self.main_view = MainContentView::Gallery;
        // P3：切到 Gallery 即导航完成，收起浮层侧栏。
        self.close_sidebar_peek();
        // 同步账号筛选列表并按当前搜索词重新拉取缓存数据。
        let accounts = self
            .accounts
            .iter()
            .filter(|a| a.enabled)
            .map(|a| (a.id, a.display_label()))
            .collect();
        if let Some(ref g) = self.gallery {
            g.update(cx, |state, _cx| state.set_accounts(accounts));
        }
        self.request_gallery_items(cx);
        cx.notify();
    }

    /// 附件总览「保存」：复用现有 rfd 保存对话框 + `SaveAttachment` 流程。
    fn save_gallery_attachment(&self, attachment_id: u32, cx: &mut Context<Self>) {
        let filename = self
            .gallery_items
            .iter()
            .find(|item| item.id == attachment_id)
            .map(|item| item.filename.clone())
            .unwrap_or_else(|| "attachment".to_string());
        self.save_attachment(attachment_id, &filename, cx);
    }

    /// 附件总览「定位到原邮件」：切回三栏并加载该邮件正文。
    fn locate_gallery_message(&mut self, message_id: u32, cx: &mut Context<Self>) {
        if self.demo_mode {
            self.main_view = MainContentView::Mail;
            self.right_mode = RightPaneMode::Reader;
            if let Some(detail) = self.demo_message_detail(message_id) {
                self.set_selected_message(detail);
            } else {
                self.selected_message = None;
            }
            cx.notify();
            return;
        }
        let Some(item) = self
            .gallery_items
            .iter()
            .find(|item| item.message_id == message_id)
        else {
            return;
        };
        let account_id = item.account_id;
        self.main_view = MainContentView::Mail;
        self.right_mode = RightPaneMode::Reader;
        if let Some((detail, true)) = self.try_load_cached_detail(message_id) {
            self.set_selected_message(detail);
            self.error_banner = None;
            cx.notify();
            return;
        }
        self.show_message_loading(message_id);
        if let Some(ref w) = self.worker {
            self.pending_body_message_id = Some(message_id);
            w.send(MailRequest::LoadMessageBody {
                account_id,
                message_id,
            });
        }
        cx.notify();
    }

    // ---- F-6 附件总览图片预览 ----

    /// 网格 / 列表中的图片附件点击预览：请求读取本地缓存的附件字节。
    /// demo 模式无真实字节，直接以占位浮层展示。
    pub(crate) fn fetch_gallery_preview(&mut self, attachment_id: u32, cx: &mut Context<Self>) {
        self.gallery_preview_pending = Some(attachment_id);
        if self.demo_mode {
            if let Some(ref g) = self.gallery {
                g.update(cx, |state, cx| {
                    state.set_preview_unavailable();
                    cx.notify();
                });
            }
            cx.notify();
            return;
        }
        if let Some(ref w) = self.worker {
            w.send(MailRequest::FetchAttachmentBytes { attachment_id });
        }
        cx.notify();
    }

    // ---- F-7 一键退订 ----

    /// 退订横幅「退订」按钮：进入卡内二次确认（对齐原版 confirm 流程）。
    #[allow(dead_code)]
    pub(crate) fn request_unsubscribe(&mut self, message_id: u32, cx: &mut Context<Self>) {
        self.pending_unsubscribe = Some(message_id);
        cx.notify();
    }

    /// 退订确认条「取消」。
    #[allow(dead_code)]
    pub(crate) fn cancel_unsubscribe(&mut self, cx: &mut Context<Self>) {
        self.pending_unsubscribe = None;
        cx.notify();
    }

    /// 执行退订：one-click 句柄直接 POST（原版行为）；https 打开浏览器；
    /// mailto 交给系统处理（gpui open_url 走系统 shell，可唤起默认邮件客户端）。
    pub(crate) fn trigger_unsubscribe(
        &mut self,
        url: &str,
        one_click: bool,
        cx: &mut Context<Self>,
    ) {
        self.pending_unsubscribe = None;
        let lower = url.to_ascii_lowercase();
        if one_click && lower.starts_with("https://") {
            if self.demo_mode {
                self.status_line =
                    SharedString::from("演示模式：已模拟发送一键退订请求（不访问网络）");
                cx.notify();
                return;
            }
            if let Some(ref w) = self.worker {
                self.status_line = SharedString::from("正在发送一键退订请求...");
                w.send(MailRequest::UnsubscribeOneClick {
                    url: url.to_string(),
                });
            }
        } else if lower.starts_with("http://") || lower.starts_with("https://") {
            self.status_line = SharedString::from("已在浏览器打开退订页面");
            self.open_link(url, cx);
        } else if lower.starts_with("mailto:") {
            // gpui open_url 经系统 shell 打开，mailto 唤起默认邮件客户端。
            self.status_line = SharedString::from("已调用系统邮件客户端发送退订邮件");
            cx.open_url(url);
        } else {
            self.error_banner = Some(SharedString::from(
                "该退订地址不受支持（仅支持 http/https/mailto）",
            ));
        }
        cx.notify();
    }

    /// 确认退订（保留兼容）。
    #[allow(dead_code)]
    pub(crate) fn confirm_unsubscribe(&mut self, _message_id: u32, cx: &mut Context<Self>) {
        if let Some((url, one_click)) = self.selected_message.as_ref().and_then(|d| {
            d.unsubscribe_url
                .clone()
                .map(|u| (u, d.unsubscribe_one_click))
        }) {
            self.trigger_unsubscribe(&url, one_click, cx);
        }
    }

    /// 联系人数据请求（进入视图 / 搜索词变化时触发；demo 模式直接聚合模拟发件人）。
    fn request_known_contacts(&mut self, cx: &mut Context<Self>) {
        if self.demo_mode {
            let items = self
                .demo_store
                .list_known_contacts(&self.contacts_query, 0, 500);
            if let Some(ref c) = self.contacts {
                c.update(cx, |state, cx| {
                    state.set_contacts(items);
                    cx.notify();
                });
            }
            return;
        }
        if let Some(ref w) = self.worker {
            w.send(MailRequest::ListKnownContacts {
                query: self.contacts_query.clone(),
                offset: 0,
                limit: 500,
            });
        }
    }

    /// 进入联系人主视图（侧栏底部入口）。
    pub(crate) fn open_contacts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.contacts.is_none() {
            let entity = cx.new(|cx| ContactsState::new(window, cx));
            let sub = cx.subscribe_in(
                &entity,
                window,
                |this, _contacts, event: &ContactsEvent, window, cx| match event {
                    ContactsEvent::QueryChanged(query) => {
                        this.contacts_query = query.clone();
                        this.request_known_contacts(cx);
                    }
                    ContactsEvent::ComposeTo(email) => {
                        this.compose_to_contact(email.clone(), window, cx);
                    }
                    ContactsEvent::SaveContact {
                        id,
                        email,
                        display_name,
                        notes,
                        phone,
                        organization,
                        job_title,
                        birthday,
                        groups,
                    } => {
                        this.save_user_contact(
                            *id,
                            email.clone(),
                            display_name.clone(),
                            notes.clone(),
                            phone.clone(),
                            organization.clone(),
                            job_title.clone(),
                            birthday.clone(),
                            groups.clone(),
                            cx,
                        );
                    }
                    ContactsEvent::DeleteContact(contact_id) => {
                        this.delete_user_contact(*contact_id, cx);
                    }
                    ContactsEvent::ImportContacts => {
                        this.import_contacts_file(cx);
                    }
                    ContactsEvent::ExportContacts { kind, items } => {
                        this.export_contacts(*kind, items.clone(), cx);
                    }
                },
            );
            self.contacts = Some(entity);
            self.contacts_sub = Some(sub);
        }
        if let Some(ref c) = self.contacts {
            c.update(cx, |state, _cx| state.set_demo_mode(self.demo_mode));
        }
        self.main_view = MainContentView::Contacts;
        // P3：切到联系人页即导航完成，收起浮层侧栏。
        self.close_sidebar_peek();
        self.request_known_contacts(cx);
        cx.notify();
    }

    /// 联系人页「给 TA 写信」：切回三栏并预填收件人。
    fn compose_to_contact(&mut self, email: String, window: &mut Window, cx: &mut Context<Self>) {
        self.main_view = MainContentView::Mail;
        self.start_new_compose(window, cx);
        self.set_recipient_field_text(ComposeAddressField::To, &email, window, cx);
        cx.notify();
    }

    // ---- F-5 用户联系人 CRUD 与导入 / 导出 ----

    /// 保存用户联系人（新建或编辑；去重校验已在联系人页完成）。
    #[allow(clippy::too_many_arguments)]
    fn save_user_contact(
        &mut self,
        id: Option<u32>,
        email: String,
        display_name: String,
        notes: String,
        phone: String,
        organization: String,
        job_title: String,
        birthday: String,
        groups: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        if self.demo_mode {
            self.status_line = SharedString::from("演示模式下不支持修改联系人数据库");
            cx.notify();
            return;
        }
        if let Some(ref w) = self.worker {
            w.send(MailRequest::SaveUserContact {
                contact: UserContact {
                    id: id.unwrap_or(0),
                    email,
                    display_name,
                    notes,
                    phone,
                    organization,
                    job_title,
                    birthday,
                    groups,
                },
            });
        }
        cx.notify();
    }

    /// 删除用户联系人（仅自定义联系人可删，`contact_id` 为行 ID）。
    fn delete_user_contact(&mut self, contact_id: u32, cx: &mut Context<Self>) {
        if self.demo_mode {
            self.status_line = SharedString::from("演示模式下不支持删除联系人");
            cx.notify();
            return;
        }
        if let Some(ref w) = self.worker {
            w.send(MailRequest::DeleteUserContact { contact_id });
        }
        cx.notify();
    }

    /// 联系人导入：rfd 选择 .vcf / .csv 文件，按内容特征识别格式解析后批量入库。
    fn import_contacts_file(&mut self, cx: &mut Context<Self>) {
        if self.demo_mode {
            self.status_line = SharedString::from("演示模式下不支持导入联系人");
            cx.notify();
            return;
        }
        if self.file_dialog_open {
            return;
        }
        self.file_dialog_open = true;
        cx.spawn(async move |this, cx| {
            let path = cx
                .background_executor()
                .spawn(async move {
                    rfd::FileDialog::new()
                        .add_filter("联系人文件 (vCard / CSV)", &["vcf", "csv"])
                        .pick_file()
                })
                .await;
            let _ = this.update(cx, |app, _| app.file_dialog_open = false);
            let Some(path) = path else {
                return;
            };
            let _ = this.update(cx, |app, cx| {
                let text = match std::fs::read_to_string(&path) {
                    Ok(text) => text,
                    Err(e) => {
                        app.error_banner =
                            Some(SharedString::from(format!("读取联系人文件失败: {e}")));
                        cx.notify();
                        return;
                    }
                };
                // 按扩展名或内容特征（BEGIN:VCARD）识别格式；解析失败的行被跳过。
                let contacts = if contacts_io::looks_like_vcard(&path, &text) {
                    contacts_io::parse_vcards(&text)
                } else {
                    contacts_io::parse_csv_contacts(&text)
                };
                if contacts.is_empty() {
                    app.status_line =
                        SharedString::from("未在文件中找到带邮箱的联系人（支持 vCard 与 CSV）");
                    cx.notify();
                    return;
                }
                let count = contacts.len();
                if let Some(ref w) = app.worker {
                    app.status_line = SharedString::from(format!("正在导入 {count} 个联系人..."));
                    w.send(MailRequest::ImportUserContacts { contacts });
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 联系人导出：把联系人页当前列表写入 vCard / CSV 文件（demo 模式同样可用）。
    fn export_contacts(
        &mut self,
        kind: ContactExportKind,
        items: Vec<KnownContact>,
        cx: &mut Context<Self>,
    ) {
        let (title, filter, default_name, content) = match kind {
            ContactExportKind::Vcard => (
                "导出 vCard 联系人",
                ("vCard 文件", &["vcf"][..]),
                "contacts.vcf",
                contacts_io::export_vcards(
                    &items
                        .iter()
                        .map(|c| UserContact {
                            id: c.id,
                            email: c.email.clone(),
                            display_name: c.name.clone(),
                            notes: c.notes.clone(),
                            phone: c.phone.clone(),
                            organization: c.organization.clone(),
                            job_title: c.job_title.clone(),
                            birthday: c.birthday.clone(),
                            groups: c.groups.clone(),
                        })
                        .collect::<Vec<_>>(),
                ),
            ),
            ContactExportKind::Csv => (
                "导出 CSV 联系人",
                ("CSV 文件", &["csv"][..]),
                "contacts.csv",
                contacts_io::export_csv(
                    &items
                        .iter()
                        .map(|c| UserContact {
                            id: c.id,
                            email: c.email.clone(),
                            display_name: c.name.clone(),
                            notes: c.notes.clone(),
                            phone: c.phone.clone(),
                            organization: c.organization.clone(),
                            job_title: c.job_title.clone(),
                            birthday: c.birthday.clone(),
                            groups: c.groups.clone(),
                        })
                        .collect::<Vec<_>>(),
                ),
            ),
        };
        if items.is_empty() {
            self.status_line = SharedString::from("暂无可导出的联系人");
            cx.notify();
            return;
        }
        let count = items.len();
        if self.file_dialog_open {
            return;
        }
        self.file_dialog_open = true;
        cx.spawn(async move |this, cx| {
            let dest = cx
                .background_executor()
                .spawn(async move {
                    rfd::FileDialog::new()
                        .set_title(title)
                        .add_filter(filter.0, filter.1)
                        .set_file_name(default_name)
                        .save_file()
                })
                .await;
            let _ = this.update(cx, |app, _| app.file_dialog_open = false);
            let Some(dest) = dest else {
                return;
            };
            let _ = this.update(cx, |app, cx| {
                match std::fs::write(&dest, content) {
                    Ok(()) => {
                        app.status_line = SharedString::from(format!(
                            "已导出 {} 个联系人至 {}",
                            count,
                            dest.display()
                        ));
                    }
                    Err(e) => {
                        app.error_banner =
                            Some(SharedString::from(format!("写入联系人文件失败: {e}")));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 打开统一设置窗口（侧栏菜单 / 命令面板 / Ctrl+,）。
    pub(crate) fn open_preferences(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let tab = if self.prefs.settings_open_accounts {
            PrefTab::MailAccounts
        } else {
            PrefTab::General
        };
        self.open_settings_on_tab(tab, cx);
    }

    /// 打开独立设置窗口并切换到指定分类页。
    pub(crate) fn open_settings_on_tab(&mut self, tab: PrefTab, cx: &mut Context<Self>) {
        self.open_settings_window(SettingsOpenTarget::Tab(tab), cx);
    }

    fn with_settings_window(
        &self,
        cx: &Context<Self>,
        f: impl FnOnce(&mut SettingsWindow, &mut Window, &mut Context<SettingsWindow>) + 'static,
    ) {
        let Some((handle, entity)) = self.settings_window.clone() else {
            return;
        };
        cx.spawn(async move |_, cx| {
            cx.update(|cx| {
                let _ = handle.update(cx, |_, window, cx| {
                    let _ = entity.update(cx, |win, cx| f(win, window, cx));
                });
            });
        })
        .detach();
    }

    fn open_settings_window(&mut self, target: SettingsOpenTarget, cx: &mut Context<Self>) {
        if self.settings_window_opening {
            return;
        }
        self.settings_window_opening = true;
        let existing = self.settings_window.clone();
        let initial_prefs = self.prefs.clone();
        let app_entity = cx.entity();

        cx.spawn(async move |this, cx| {
            let outcome: Option<(AnyWindowHandle, Entity<SettingsWindow>)> = cx.update(|cx| {
                if let Some((handle, entity)) = existing {
                    let target_copy = target.clone();
                    let ok = handle.update(cx, |_, window, cx| {
                        window.activate_window();
                        let _ = entity.update(cx, |win, cx| match target_copy {
                            SettingsOpenTarget::Tab(tab) => win.select_tab(tab, window, cx),
                            // 任务二：跳转到指定账号编辑 / 新建前先走未保存确认
                            //（保存 / 放弃 / 继续编辑），保存成功后再执行跳转。
                            SettingsOpenTarget::EditAccount(Some(ref cfg)) => {
                                win.request_edit_account_editor(cfg.id, window, cx);
                            }
                            SettingsOpenTarget::EditAccount(None) => {
                                win.request_new_account_editor(window, cx);
                            }
                            SettingsOpenTarget::PresetAccount(idx) => {
                                win.request_preset_account_editor(idx, window, cx);
                            }
                        });
                    });
                    if ok.is_ok() {
                        return Some((handle, entity));
                    }
                }

                let (initial_tab, edit_acc, preset_idx, open_account_editor) = match target {
                    SettingsOpenTarget::Tab(tab) => (tab, None, None, false),
                    SettingsOpenTarget::EditAccount(cfg) => {
                        (PrefTab::MailAccounts, cfg, None, true)
                    }
                    SettingsOpenTarget::PresetAccount(idx) => {
                        (PrefTab::MailAccounts, None, Some(idx), true)
                    }
                };

                let centered = Bounds::centered(None, size(px(920.0), px(848.0)), cx);
                let options = WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(centered)),
                    titlebar: Some(TitlebarOptions {
                        title: Some("偏好设置".into()),
                        appears_transparent: true,
                        traffic_light_position: None,
                    }),
                    ..Default::default()
                };
                match gpui_kit::open_window(options, cx, move |window, cx| {
                    window.set_window_title("偏好设置");
                    cx.new(|cx| {
                        SettingsWindow::new(
                            app_entity,
                            initial_prefs,
                            initial_tab,
                            edit_acc,
                            preset_idx,
                            open_account_editor,
                            window,
                            cx,
                        )
                    })
                }) {
                    Ok((handle, entity)) => {
                        #[cfg(target_os = "windows")]
                        crate::apply_window_icon();
                        Some((handle.into(), entity))
                    }
                    Err(_) => None,
                }
            });

            let _ = this.update(cx, |app, cx| {
                app.settings_window_opening = false;
                app.settings_window = outcome;
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn clear_settings_window_ref(&mut self) {
        self.settings_window = None;
        self.settings_window_opening = false;
    }

    /// 应用偏好变更并统一触发对应副作用与原子落盘。
    pub(crate) fn mutate_preferences(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut AppPreferences),
    ) {
        let prev = self.prefs.clone();
        f(&mut self.prefs);
        self.apply_preferences_snapshot(&prev, cx);
    }

    fn apply_preferences_snapshot(&mut self, prev: &AppPreferences, cx: &mut Context<Self>) {
        self.prefs.ui_text_scale_percent =
            config::clamp_ui_text_scale(self.prefs.ui_text_scale_percent);
        self.prefs.reader_zoom_percent = config::clamp_reader_zoom(self.prefs.reader_zoom_percent);

        if let Err(e) = config::save_preferences_in(&config::data_dir(), &self.prefs) {
            self.error_banner = Some(SharedString::from(e));
        }

        let page_size_changed = self.prefs.page_size != prev.page_size;
        let sync_changed = self.prefs.auto_sync_minutes != prev.auto_sync_minutes;
        let theme_changed =
            self.prefs.theme_id != prev.theme_id || self.prefs.dark_mode != prev.dark_mode;
        let thread_order_changed = self.prefs.thread_newest_first != prev.thread_newest_first;
        let fold_changed =
            self.prefs.effective_fold_messages_mode() != prev.effective_fold_messages_mode();
        let reader_pos_changed = self.prefs.reader_position != prev.reader_position;
        let show_accounts_changed = self.prefs.show_accounts != prev.show_accounts;
        let default_reader_mode_changed =
            self.prefs.reader_default_mode != prev.reader_default_mode;
        let keep_downloads_changed = self.prefs.keep_downloads_days != prev.keep_downloads_days;

        if reader_pos_changed {
            self.reader_position = ReaderPosition::from_key(&self.prefs.reader_position)
                .unwrap_or(ReaderPosition::Right);
        }
        if show_accounts_changed {
            self.show_accounts = self.prefs.show_accounts;
        }
        if theme_changed {
            self.theme_id = theme::static_theme_id(&self.prefs.theme_id);
            self.dark_mode = self.prefs.dark_mode.unwrap_or(self.system_dark_mode);
            theme::apply(self.theme_id, self.dark_mode, cx);
        }
        if sync_changed {
            self.auto_sync_deadline = None;
        }
        if page_size_changed {
            self.reset_list_pagination();
            self.reload_current_page(cx);
        }
        if thread_order_changed {
            if let Some(ref mut detail) = self.selected_message {
                if detail.thread_messages.len() > 1 {
                    detail.thread_messages.reverse();
                }
            }
        }
        if fold_changed || thread_order_changed {
            let fold_mode = self.prefs.effective_fold_messages_mode().to_string();
            if let Some(ref detail) = self.selected_message {
                self.collapsed_thread_cards = compute_default_collapsed_cards(
                    detail,
                    self.prefs.thread_newest_first,
                    &fold_mode,
                );
            }
        }
        if default_reader_mode_changed && !(self.focus_mode && self.prefs.focus_reader_view) {
            match self.prefs.reader_default_mode.as_str() {
                "on" => self.reader_prefer_plain = true,
                "off" => self.reader_prefer_plain = false,
                _ => {}
            }
        }
        if keep_downloads_changed && self.prefs.keep_downloads_days > 0 {
            if let Some(ref w) = self.worker {
                w.send(MailRequest::PruneExpiredDownloads {
                    keep_days: self.prefs.keep_downloads_days,
                });
            }
        }
        cx.notify();
    }

    pub(crate) fn set_push_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.mutate_preferences(cx, |p| p.push_enabled = enabled);
        if let Some(ref w) = self.worker {
            w.send(MailRequest::SetPushEnabled { enabled });
        }
        self.status_line = SharedString::from(if enabled {
            "已启用实时邮件推送"
        } else {
            "已关闭实时邮件推送"
        });
        cx.notify();
    }

    pub(crate) fn set_show_accounts(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.show_accounts = visible;
        self.mutate_preferences(cx, |p| p.show_accounts = visible);
        self.status_line = SharedString::from(if visible {
            "已显示账号列表"
        } else {
            "已隐藏账号列表"
        });
        cx.notify();
    }

    pub(crate) fn clear_body_caches(&mut self, cx: &mut Context<Self>) {
        if self.demo_mode {
            return;
        }
        if let Some(ref w) = self.worker {
            self.status_line = SharedString::from("正在清理正文缓存...");
            w.send(MailRequest::ClearBodyCaches);
        }
        cx.notify();
    }

    pub(crate) fn apply_imported_backup(
        &mut self,
        bundle: SettingsBackupBundle,
        cx: &mut Context<Self>,
    ) -> Result<usize, String> {
        let count = config::import_settings_bundle_in(&config::data_dir(), &bundle)?;
        let prev = self.prefs.clone();
        self.prefs = bundle.preferences;
        if !self.demo_mode {
            self.accounts = bundle.accounts;
            if !self
                .accounts
                .iter()
                .any(|a| a.id == self.compose_account_id && a.enabled)
            {
                if let Some(first) = self
                    .accounts
                    .iter()
                    .find(|a| a.enabled)
                    .or_else(|| self.accounts.first())
                {
                    self.compose_account_id = first.id;
                }
            }
            if let Some(ref w) = self.worker {
                w.send(MailRequest::SetPushEnabled {
                    enabled: self.prefs.push_enabled,
                });
                w.send(MailRequest::LoadAllCached);
            }
        }
        self.apply_preferences_snapshot(&prev, cx);
        self.status_line = SharedString::from(format!("已导入配置备份（{count} 个账号配置）"));
        Ok(count)
    }

    /// 关闭当前浮层并卸载对应实体（命令面板 / 快捷键 / 关于）。
    pub(crate) fn close_modal(&mut self, cx: &mut Context<Self>) {
        self.active_modal = ActiveModal::None;
        self.palette = None;
        self.palette_sub = None;
        cx.notify();
    }

    /// 打开命令面板浮层（侧栏菜单 / Ctrl+K）；`actions_only` 为 true 时
    /// 预填 `>` 仅筛选操作命令（Ctrl+Shift+K，对齐原版「actions only」）。
    pub(crate) fn open_command_palette(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        actions_only: bool,
    ) {
        self.active_modal = ActiveModal::CommandPalette;
        if self.palette.is_none() {
            let commands = self.palette_commands();
            let entity = cx.new(|cx| CommandPaletteState::new(window, cx, commands, actions_only));
            let sub = cx.subscribe_in(
                &entity,
                window,
                |this, _palette, event: &CommandPaletteEvent, window, cx| match event {
                    CommandPaletteEvent::CommandExecuted(command) => {
                        this.execute_palette_command(*command, window, cx);
                    }
                    CommandPaletteEvent::Closed => this.close_modal(cx),
                },
            );
            self.palette = Some(entity);
            self.palette_sub = Some(sub);
        }
        let names = self.palette_folder_names();
        if let Some(ref p) = self.palette {
            p.update(cx, |state, _cx| state.set_folder_names(names));
        }
        cx.notify();
    }

    /// 固定命令 + 选中邮件操作 + 统一视图 / 联系人 / 附件入口 +
    /// 遍历 folders_by_account 生成的文件夹跳转命令。
    fn palette_commands(&self) -> Vec<PaletteCommand> {
        let mut commands = vec![
            PaletteCommand::NewCompose,
            PaletteCommand::RefreshAll,
            PaletteCommand::ToggleUnifiedInbox,
            PaletteCommand::OpenAccountSettings,
            PaletteCommand::OpenPreferences,
            PaletteCommand::ToggleTheme,
        ];
        // 对当前选中邮件的操作：无选中时不显示（对齐原版 acts_on_selection）；
        // 归档 / 删除仅真实账号可用（demo 模式下右键菜单同样隐藏）。
        if self.selected_message.is_some() {
            commands.push(PaletteCommand::ReplySelected);
            commands.push(PaletteCommand::MarkUnreadSelected);
            commands.push(PaletteCommand::ToggleStarSelected);
            if !self.demo_mode {
                commands.push(PaletteCommand::ArchiveSelected);
                commands.push(PaletteCommand::DeleteSelected);
            }
        }
        // 统一视图与联系人 / 附件入口（原版命令面板的 go-to 分组）。
        commands.push(PaletteCommand::GoStarred);
        commands.push(PaletteCommand::GoSent);
        commands.push(PaletteCommand::GoDrafts);
        commands.push(PaletteCommand::GoArchive);
        commands.push(PaletteCommand::OpenContacts);
        commands.push(PaletteCommand::OpenGallery);
        for (&account_id, folders) in &self.folders_by_account {
            for folder in folders {
                commands.push(PaletteCommand::GoFolder {
                    account_id,
                    folder_id: folder.id,
                });
            }
        }
        commands
    }

    /// GoFolder 命令的显示名（多账号时带账号前缀）。
    fn palette_folder_names(&self) -> Vec<(u32, u32, String)> {
        let multi_account = self.accounts.iter().filter(|a| a.enabled).count() > 1;
        self.folders_by_account
            .iter()
            .filter(|(&account_id, _)| {
                self.accounts
                    .iter()
                    .find(|a| a.id == account_id)
                    .map(|a| a.enabled)
                    .unwrap_or(true)
            })
            .flat_map(|(&account_id, folders)| {
                let account_label = self.account_label_for(account_id);
                folders.iter().map(move |folder| {
                    let name = if multi_account {
                        format!("{account_label} / {}", folder.name)
                    } else {
                        folder.name.clone()
                    };
                    (account_id, folder.id, name)
                })
            })
            .collect()
    }

    /// 分发命令面板选中的命令到现有动作。
    fn execute_palette_command(
        &mut self,
        command: PaletteCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match command {
            PaletteCommand::NewCompose => {
                self.main_view = MainContentView::Mail;
                self.start_new_compose(window, cx);
            }
            PaletteCommand::RefreshAll => {
                self.main_view = MainContentView::Mail;
                self.refresh_all(cx);
            }
            PaletteCommand::ToggleUnifiedInbox => {
                self.main_view = MainContentView::Mail;
                self.select_unified_view(CurrentView::UnifiedInbox, cx);
            }
            PaletteCommand::OpenAccountSettings => {
                self.close_modal(cx);
                self.open_account_manager(cx);
                return;
            }
            PaletteCommand::OpenPreferences => {
                // 先关闭命令面板再打开偏好设置，避免下方 close_modal 误关刚打开的偏好设置。
                self.close_modal(cx);
                self.main_view = MainContentView::Mail;
                self.open_preferences(window, cx);
                return;
            }
            PaletteCommand::ToggleTheme => {
                self.toggle_theme(cx);
            }
            PaletteCommand::GoStarred => {
                self.main_view = MainContentView::Mail;
                self.select_unified_view(CurrentView::Starred, cx);
            }
            PaletteCommand::GoSent => {
                self.main_view = MainContentView::Mail;
                self.select_unified_view(CurrentView::Sent, cx);
            }
            PaletteCommand::GoDrafts => {
                self.main_view = MainContentView::Mail;
                self.select_unified_view(CurrentView::Drafts, cx);
            }
            PaletteCommand::GoArchive => {
                self.main_view = MainContentView::Mail;
                self.select_unified_view(CurrentView::Archive, cx);
            }
            PaletteCommand::OpenContacts => {
                self.close_modal(cx);
                self.open_contacts(window, cx);
                return;
            }
            PaletteCommand::OpenGallery => {
                self.close_modal(cx);
                self.open_gallery(window, cx);
                return;
            }
            PaletteCommand::ReplySelected => {
                let Some(detail) = self.selected_message.clone() else {
                    return;
                };
                self.close_modal(cx);
                self.main_view = MainContentView::Mail;
                self.start_reply(&detail, window, cx);
                return;
            }
            PaletteCommand::ArchiveSelected => {
                self.archive_selected(cx);
            }
            PaletteCommand::DeleteSelected => {
                self.delete_selected(cx);
            }
            PaletteCommand::MarkUnreadSelected => {
                let Some(detail) = self.selected_message.clone() else {
                    return;
                };
                self.set_row_seen(
                    detail.account_id,
                    detail.id,
                    detail.uid,
                    detail.folder_path.clone(),
                    false,
                    cx,
                );
            }
            PaletteCommand::ToggleStarSelected => {
                let Some(detail) = self.selected_message.clone() else {
                    return;
                };
                self.toggle_flagged(&detail, cx);
            }
            PaletteCommand::GoFolder {
                account_id,
                folder_id,
            } => {
                self.main_view = MainContentView::Mail;
                let folder_path = self
                    .folders_by_account
                    .get(&account_id)
                    .and_then(|folders| folders.iter().find(|f| f.id == folder_id))
                    .map(|folder| folder.path.clone())
                    .unwrap_or_default();
                self.select_folder(account_id, folder_id, folder_path, cx);
            }
        }
        self.close_modal(cx);
    }

    /// 刷新全部（侧栏刷新按钮 / 命令面板 / F5 共用；阶段三起 Ctrl+R 改为回复）。
    pub(crate) fn refresh_all(&mut self, cx: &mut Context<Self>) {
        if self.demo_mode {
            self.reload_current_page(cx);
            cx.notify();
            return;
        }
        if let Some(ref w) = self.worker {
            self.error_banner = None;
            self.status_line = SharedString::from("正在独立刷新所有账号...");
            w.send(MailRequest::SyncAllAccounts);
        }
        cx.notify();
    }

    /// 后台定时刷新 tick（每 30 秒醒来，到期发 `SyncAllAccounts`）。
    fn auto_sync_tick(&mut self) {
        if self.demo_mode || self.prefs.auto_sync_minutes == 0 {
            return;
        }
        let Some(ref w) = self.worker else {
            return;
        };
        let interval = Duration::from_secs(u64::from(self.prefs.auto_sync_minutes) * 60);
        let now = Instant::now();
        let deadline = self.auto_sync_deadline.unwrap_or(now + interval);
        self.auto_sync_deadline = Some(deadline);
        if now >= deadline {
            w.send(MailRequest::SyncAllAccounts);
            self.auto_sync_deadline = Some(now + interval);
        }
    }

    /// 切换视图 / 文件夹时把「上次位置」写入偏好并落盘（切换频率低，同步写可接受）。
    fn remember_last_location(&mut self, folder_account_id: Option<u32>) {
        if self.demo_mode {
            return;
        }
        match self.current_view {
            CurrentView::UnifiedInbox => {
                self.prefs.last_view = Some("unified".to_string());
                self.prefs.last_folder = None;
            }
            CurrentView::Unread => {
                self.prefs.last_view = Some("unread".to_string());
                self.prefs.last_folder = None;
            }
            CurrentView::Starred => {
                self.prefs.last_view = Some("starred".to_string());
                self.prefs.last_folder = None;
            }
            CurrentView::Sent => {
                self.prefs.last_view = Some("sent".to_string());
                self.prefs.last_folder = None;
            }
            CurrentView::Drafts => {
                self.prefs.last_view = Some("drafts".to_string());
                self.prefs.last_folder = None;
            }
            CurrentView::Archive => {
                self.prefs.last_view = Some("archive".to_string());
                self.prefs.last_folder = None;
            }
            CurrentView::Junk => {
                self.prefs.last_view = Some("junk".to_string());
                self.prefs.last_folder = None;
            }
            CurrentView::Trash => {
                self.prefs.last_view = Some("trash".to_string());
                self.prefs.last_folder = None;
            }
            CurrentView::Snoozed => {
                self.prefs.last_view = Some("snoozed".to_string());
                self.prefs.last_folder = None;
            }
            CurrentView::AccountFolder => {
                if let Some(account_id) = folder_account_id {
                    self.prefs.last_view = None;
                    self.prefs.last_folder = Some((account_id, self.selected_folder_id));
                }
            }
        }
        if let Err(e) = config::save_preferences_in(&config::data_dir(), &self.prefs) {
            self.error_banner = Some(SharedString::from(e));
        }
    }

    /// 检查当前窗口焦点是否位于任意文本输入框（防止 Delete / 方向键与打字冲突；
    /// P3 起也供邮件行渲染判定「列表焦点近似态」使用）。
    pub(crate) fn is_text_input_focused(&self, window: &Window, cx: &Context<Self>) -> bool {
        self.search_input.focus_handle(cx).is_focused(window)
            || self.compose_to.focus_handle(cx).is_focused(window)
            || self.compose_cc.focus_handle(cx).is_focused(window)
            || self.compose_bcc.focus_handle(cx).is_focused(window)
            || self.compose_subject.focus_handle(cx).is_focused(window)
            || self.compose_body.focus_handle(cx).is_focused(window)
    }

    /// 键盘方向键切换中栏下一封 (`delta = 1`) / 上一封 (`delta = -1`) 邮件。
    fn select_adjacent_message(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.current_view == CurrentView::Drafts && !self.demo_mode {
            return;
        }
        let items = &self.current_page.items;
        let current_idx = self
            .selected_message
            .as_ref()
            .and_then(|sel| items.iter().position(|m| m.id == sel.id));
        let Some(next_idx) = next_adjacent_index(current_idx, items.len(), delta) else {
            return;
        };
        let msg = items[next_idx].clone();
        // P3：键盘导航视为列表持焦点，并把键盘游标移到该行（对齐 Linux j/k 的
        // .kbd-cursor：点击行后由行点击处理器清除）。
        self.list_focused = true;
        self.kbd_cursor_message_id = Some(msg.id);
        self.selected_message_ids.clear();
        self.select_message(
            msg.account_id,
            msg.id,
            msg.uid,
            msg.unread,
            &msg.folder_path,
            cx,
        );
    }

    /// F-1：会话内步进下一张 (`delta = 1`) / 上一张 (`delta = -1`) 卡片（单键 w / b）。
    ///
    /// 复用现有选择路径：把选中锚点切到目标卡片并加载其正文——`set_selected_message`
    /// 的同会话分支会自动展开目标折叠卡片，阅读器的 is_current 高亮随锚点移动。
    /// 不做程序化滚动：阅读器滚动容器没有逐卡片锚点（如需可后续加 ScrollHandle）。
    /// 单卡 / 空会话时无邻卡，no-op。
    fn step_thread_card(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(detail) = self.selected_message.clone() else {
            return;
        };
        let cards = &detail.thread_messages;
        let current_idx = cards.iter().position(|tm| tm.id == detail.id);
        let Some(next_idx) = next_adjacent_index(current_idx, cards.len(), delta) else {
            return;
        };
        let card = &cards[next_idx];
        // 卡片未读态优先取当前列表行；不在列表中则不触发已读标记（传 false）。
        let unread = self
            .current_page
            .items
            .iter()
            .find(|m| m.id == card.id)
            .map(|m| m.unread)
            .unwrap_or(false);
        let uid = if card.uid > 0 { card.uid } else { detail.uid };
        let folder_path = if card.folder_path.is_empty() {
            detail.folder_path.clone()
        } else {
            card.folder_path.clone()
        };
        self.select_message(detail.account_id, card.id, uid, unread, &folder_path, cx);
    }

    // ---- 右栏渲染（阅读态 / 写信 / 账号管理 / 欢迎页由 ui:: 渲染）----

    /// L-2/L-6：Mail 内容区组装——专注模式与阅读器位置（Right / Below / Hidden）路由。
    ///
    /// - 专注模式：左栏与列表整体隐藏，右栏（阅读器 / 写信 / 欢迎页）占满。
    /// - Right：侧栏 | 列表（持久化宽度）| 右栏（左缘拖拽把手）。
    /// - Below：侧栏 | 垂直分割列（列表上，持久化高度；阅读器下占满，上缘把手）。
    /// - Hidden：侧栏 | 列表占满（无右栏、无右栏把手）。
    /// - 欢迎页（无账号）：侧栏 | 列表 | 欢迎页（只有侧栏把手）。
    fn render_mail_body(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        sidebar_el: Option<gpui_kit::AnyElement>,
    ) -> gpui_kit::AnyElement {
        let list_el = message_list::render(self, window, cx);

        if self.focus_mode {
            let show_list = !self.prefs.focus_hide_list;
            return h_flex()
                .flex_1()
                .min_h_0()
                .children(sidebar_el)
                .when(show_list, |el| {
                    el.child(
                        div()
                            .w(px(self.list_width))
                            .h_full()
                            .flex_shrink_0()
                            .child(list_el),
                    )
                })
                .child(self.render_right_pane(window, cx))
                .into_any_element();
        }

        match self.right_mode {
            RightPaneMode::Welcome => h_flex()
                .flex_1()
                .min_h_0()
                .children(sidebar_el)
                .child(
                    div()
                        .w(px(self.list_width))
                        .h_full()
                        .flex_shrink_0()
                        .child(list_el),
                )
                .child(self.render_right_pane(window, cx))
                .into_any_element(),
            RightPaneMode::Compose | RightPaneMode::Reader => match self.reader_position {
                ReaderPosition::Right => h_flex()
                    .flex_1()
                    .min_h_0()
                    .children(sidebar_el)
                    .child(
                        div()
                            .w(px(self.list_width))
                            .h_full()
                            .flex_shrink_0()
                            .child(list_el),
                    )
                    .child(self.render_mail_pane(window, cx))
                    .into_any_element(),
                ReaderPosition::Below => h_flex()
                    .flex_1()
                    .min_h_0()
                    .children(sidebar_el)
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .child(
                                div()
                                    .w_full()
                                    .h(px(self.list_below_height))
                                    .min_h(px(BELOW_LIST_MIN_HEIGHT))
                                    .flex_shrink_0()
                                    .child(list_el),
                            )
                            .child(self.render_mail_pane_below(window, cx)),
                    )
                    .into_any_element(),
                ReaderPosition::Hidden => h_flex()
                    .flex_1()
                    .min_h_0()
                    .children(sidebar_el)
                    .child(div().flex_1().min_w_0().h_full().child(list_el))
                    .into_any_element(),
            },
        }
    }

    /// 右栏容器（阅读器 / 写信）：内容 + 左缘拖拽把手（列表与右栏之间，L-2）。
    fn render_mail_pane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let inner = self.render_right_pane(window, cx);
        div()
            .relative()
            .flex_1()
            .min_w(px(READER_MIN_WIDTH))
            .h_full()
            .child(inner)
            .child(
                resize_handle::<ListDrag, ListDrag>("mail-pane-handle", Axis::Horizontal)
                    .inside(HandleEdge::Leading)
                    .on_drag(ListDrag, |_, _, _, cx| cx.new(|_| ListDrag)),
            )
            .into_any_element()
    }

    /// 右栏容器（阅读器在下方）：内容 + 上缘拖拽把手（列表与阅读器之间，L-6）。
    fn render_mail_pane_below(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let inner = self.render_right_pane(window, cx);
        div()
            .relative()
            .flex_1()
            .w_full()
            .min_h(px(BELOW_PANE_MIN_HEIGHT))
            .child(inner)
            .child(
                resize_handle::<BelowDrag, BelowDrag>("mail-below-handle", Axis::Vertical)
                    .inside(HandleEdge::Leading)
                    .on_drag(BelowDrag, |_, _, _, cx| cx.new(|_| BelowDrag)),
            )
            .into_any_element()
    }

    fn render_right_pane(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        match self.right_mode {
            RightPaneMode::Compose => {
                // WP-10-4：写信已弹出为独立窗口时，主窗口回到阅读器。
                if self.compose_detached {
                    return reader::render(self, window, cx);
                }
                // WP-10-4：内联上下分屏——写信面板贴顶（默认落点就在手边），
                // 原邮件留在下半部分可随时对照滚动，中间是 `.split-grab-pill` 拖拽手柄。
                if self.compose_split && self.selected_message.is_some() {
                    let p = theme::current(cx);
                    // 窗口控制按钮画在上半部分写信面板的顶栏里。
                    let compose_el = self.render_compose_pane(window, cx, true, false);
                    let reader_el = reader::render_with_pane_top(self, window, cx, false);
                    return v_flex()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .bg(p.canvas)
                        .child(
                            div()
                                .relative()
                                .h(px(self.compose_split_height))
                                .min_h(px(COMPOSE_SPLIT_MIN_HEIGHT))
                                .flex_shrink_0()
                                .child(compose_el)
                                .child(
                                    resize_handle::<ComposeSplitDrag, ComposeSplitDrag>(
                                        "compose-split-handle",
                                        Axis::Vertical,
                                    )
                                    .inside(HandleEdge::Trailing)
                                    .on_drag(ComposeSplitDrag, |_, _, _, cx| {
                                        cx.new(|_| ComposeSplitDrag)
                                    }),
                                ),
                        )
                        .child(div().flex_1().min_h_0().child(reader_el))
                        .on_drag_move(cx.listener(
                            |this, event: &DragMoveEvent<ComposeSplitDrag>, _, cx| {
                                this.on_compose_split_drag_move(event, cx);
                            },
                        ))
                        .into_any_element();
                }
                let pane_top = self.pane_at_window_top();
                self.render_compose_pane(window, cx, pane_top, false)
            }
            RightPaneMode::Welcome => welcome::render(self, window, cx),
            RightPaneMode::Reader => reader::render(self, window, cx),
        }
    }
    /// 写信页（标题栏 + 表单），供主窗口与独立写信窗口共用。
    ///
    /// - `pane_top`：是否在本页顶栏画窗口控制按钮（内联分屏时由上方写信面板承载）；
    /// - `detached`：本页已在独立窗口里，标题栏换成「收回」按钮。
    pub(crate) fn render_compose_pane(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
        pane_top: bool,
        detached: bool,
    ) -> gpui_kit::AnyElement {
        let p = theme::current(cx);
        // 写信页唯一标题栏（B-8：合并进 48px 顶栏，compose.rs 不再画第二层标题）：
        // 左侧「取消 / 存草稿」，中间拖拽区，右侧「附件 / 发送」，复用现有动作。
        v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(p.canvas)
            .child(
                h_flex()
                    .h(px(window_controls::HEADER_HEIGHT))
                    .flex_shrink_0()
                    .px_2()
                    .gap_1()
                    .items_center()
                    .border_b_1()
                    .border_color(p.border)
                    .child(
                        Button::new("compose-bar-cancel")
                            .ghost()
                            .compact()
                            .label("取消")
                            .tooltip("放弃当前邮件（Esc）")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.request_close_compose(cx);
                            })),
                    )
                    .child(
                        Button::new("compose-bar-save-draft")
                            .ghost()
                            .compact()
                            .label("存草稿")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.save_local_draft(cx);
                            })),
                    )
                    .child(window_controls::drag_spacer())
                    // WP-10-4：内联分屏 / 独立窗口互转。
                    .when(detached, |bar| {
                        bar.child(
                            Button::new("compose-bar-attach-back")
                                .ghost()
                                .compact()
                                .icon(IconName::Minimize2)
                                .label("收回")
                                .tooltip("收回主窗口")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.attach_compose_back(cx);
                                    window.remove_window();
                                })),
                        )
                    })
                    .when(!detached, |bar| {
                        bar.child(
                            Button::new("compose-bar-pop-out")
                                .ghost()
                                .compact()
                                .icon(IconName::ExternalLink)
                                .tooltip("在独立窗口中撰写")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.pop_out_compose(cx);
                                })),
                        )
                    })
                    .child(
                        Button::new("compose-bar-attach")
                            .ghost()
                            .compact()
                            .icon(IconName::Paperclip)
                            .label("附件")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.pick_attachments(cx);
                            })),
                    )
                    .child({
                        // WP-10-5 写信模板：套用 / 存为模板 / 删除。
                        let templates = self.templates.clone();
                        let ent_tpl = cx.entity();
                        let ent_tpl_popup = cx.entity();
                        Button::new("compose-bar-template")
                            .ghost()
                            .compact()
                            .icon(IconName::Bookmark)
                            .tooltip("写信模板")
                            .dropdown_menu(move |mut menu, _, _| {
                                if templates.is_empty() {
                                    menu =
                                        menu.item(PopupMenuItem::label("还没有模板，先写一封再存"));
                                }
                                for tpl in templates.iter() {
                                    let tpl_item = tpl.clone();
                                    let ent_item = ent_tpl.clone();
                                    menu = menu.item(
                                        PopupMenuItem::new(tpl.name.clone())
                                            .icon(Icon::new(IconName::Bookmark))
                                            .on_click(move |_, window, cx| {
                                                ent_item.update(cx, |this, cx| {
                                                    this.apply_compose_template(
                                                        &tpl_item, window, cx,
                                                    )
                                                });
                                            }),
                                    );
                                }
                                let ent_save = ent_tpl.clone();
                                menu = menu.item(PopupMenuItem::separator());
                                menu = menu.item(
                                    PopupMenuItem::new("存为模板…")
                                        .icon(Icon::new(IconName::BookmarkPlus))
                                        .on_click(move |_, window, cx| {
                                            ent_save.update(cx, |this, cx| {
                                                this.begin_save_compose_template(window, cx)
                                            });
                                        }),
                                );
                                if templates.is_empty() {
                                    return menu;
                                }
                                menu = menu.item(PopupMenuItem::separator());
                                for tpl in templates.iter() {
                                    let id = tpl.id;
                                    let name = tpl.name.clone();
                                    let ent_item = ent_tpl.clone();
                                    menu = menu.item(
                                        PopupMenuItem::new(format!("删除「{name}」"))
                                            .icon(Icon::new(IconName::Trash))
                                            .on_click(move |_, _, cx| {
                                                ent_item.update(cx, |this, cx| {
                                                    this.delete_compose_template(id, cx)
                                                });
                                            }),
                                    );
                                }
                                menu
                            })
                            .on_open_change(move |open, _, cx| {
                                ent_tpl_popup.update(cx, |app, cx| {
                                    app.reader_popup_open = *open;
                                    cx.notify();
                                });
                            })
                    })
                    .child({
                        // WP-10-5 定时发送：预设时间与「自定义时间…」。
                        let presets = self.compose_later_presets();
                        let ent_schedule = cx.entity();
                        let ent_schedule_popup = cx.entity();
                        Button::new("compose-bar-schedule")
                            .ghost()
                            .compact()
                            .icon(IconName::AlarmClock)
                            .tooltip("定时发送")
                            .dropdown_menu(move |mut menu, _, _| {
                                for preset in presets.iter() {
                                    let at = preset.at;
                                    let ent_item = ent_schedule.clone();
                                    menu = menu.item(
                                        PopupMenuItem::new(preset.label.clone()).on_click(
                                            move |_, _, cx| {
                                                ent_item.update(cx, |this, cx| {
                                                    this.send_compose_scheduled(at, cx)
                                                });
                                            },
                                        ),
                                    );
                                }
                                let ent_custom = ent_schedule.clone();
                                menu.item(
                                    PopupMenuItem::new("自定义时间…")
                                        .icon(Icon::new(IconName::Pencil))
                                        .on_click(move |_, window, cx| {
                                            ent_custom.update(cx, |this, cx| {
                                                this.begin_custom_compose_schedule(window, cx)
                                            });
                                        }),
                                )
                            })
                            .on_open_change(move |open, _, cx| {
                                ent_schedule_popup.update(cx, |app, cx| {
                                    app.reader_popup_open = *open;
                                    cx.notify();
                                });
                            })
                    })
                    .child(
                        Button::new("compose-bar-send")
                            .primary()
                            .compact()
                            .label("发送")
                            .tooltip("发送（Ctrl+Enter）")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.send_compose(cx);
                            })),
                    )
                    .when(pane_top, |bar| {
                        bar.child(if detached {
                            window_controls::render_standalone_window_controls(
                                "compose-wc",
                                window,
                                cx,
                            )
                        } else {
                            window_controls::render_window_controls("compose-wc", window, cx)
                        })
                    }),
            )
            .when(detached, |el| {
                el.when_some(self.error_banner.as_ref(), |el, err| {
                    el.child(
                        h_flex()
                            .px_3()
                            .py_1()
                            .border_b_1()
                            .border_color(p.border)
                            .justify_between()
                            .items_center()
                            .child(div().text_xs().text_color(p.danger).child(err.clone()))
                            .child(
                                Button::new("compose-dismiss-err")
                                    .ghost()
                                    .compact()
                                    .label("关闭")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.error_banner = None;
                                        cx.notify();
                                    })),
                            ),
                    )
                })
            })
            .child(compose::render(self, cx))
            .into_any_element()
    }

    fn render_status_bar(
        &self,
        top: bool,
        p: &theme::HylkiPalette,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let border = p.border;
        let muted_fg = p.text_muted;
        h_flex()
            .h(px(24.0))
            .flex_shrink_0()
            .px_3()
            .items_center()
            .when(top, |el| el.border_b_1())
            .when(!top, |el| el.border_t_1())
            .border_color(border)
            .bg(p.sidebar)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_xs()
                    .text_color(muted_fg)
                    .truncate()
                    .child(self.status_line.clone()),
            )
            .when_some(self.last_move_undo.clone(), |el, record| {
                el.child(
                    h_flex()
                        .flex_shrink_0()
                        .gap_1()
                        .items_center()
                        .child(
                            div()
                                .text_xs()
                                .text_color(p.text)
                                .child(record.action_label.clone()),
                        )
                        .child(
                            Button::new("undo-last-move")
                                .ghost()
                                .compact()
                                .text_size(px(11.0))
                                .label("撤销")
                                .tooltip("撤销最近一次移动 / 删除 / 归档（Ctrl+Z）")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.undo_last_move(cx);
                                })),
                        ),
                )
            })
            .into_any_element()
    }
}

impl Render for HylkiApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.last_rem_scale != self.prefs.ui_text_scale_percent {
            let scale = config::clamp_ui_text_scale(self.prefs.ui_text_scale_percent);
            window.set_rem_size(px(16.0 * scale as f32 / 100.0));
            self.last_rem_scale = scale;
        }
        self.retain_current_reader_text_states();
        #[cfg(windows)]
        if !self.reader_webview_warmup_scheduled && self.reader_webview.is_none() {
            self.reader_webview_warmup_scheduled = true;
            let handle = self.window_handle;
            cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(150))
                    .await;
                let _ = cx.update(|cx| {
                    let _ = handle.update(cx, |_, window, cx| {
                        let _ = this.update(cx, |app, cx| {
                            let _ = app.ensure_reader_webview(window, cx);
                        });
                    });
                });
            })
            .detach();
        }
        #[cfg(windows)]
        if self.selected_message.is_none() && self.reader_webview_key.take().is_some() {
            if let Some(host) = self.reader_webview.clone() {
                cx.defer_in(window, move |this, _, _| {
                    if this.reader_webview_key.is_none() {
                        let _ = host.load(b"<!doctype html>".to_vec());
                    }
                });
            }
        }
        #[cfg(windows)]
        if let Some(host) = self.reader_webview.clone() {
            let visible = self.main_view == MainContentView::Mail
                && self.selected_message.is_some()
                && self.right_mode == RightPaneMode::Reader
                && self.reader_position != ReaderPosition::Hidden
                && self.active_modal == ActiveModal::None
                && self.print_preview.is_none()
                && !self.reader_find_open
                && !self.compose_split
                && !self.sidebar_peek_open
                && self.drawer_image_preview.is_none()
                && !self.reader_popup_open;
            if !visible {
                cx.defer_in(window, move |_, _, _| host.set_visible(false));
            }
        }
        // 避坑清单第 4 条：先把语义色 Copy 成局部变量再构建 UI。
        let p = theme::current(cx);
        let border = p.border;
        // F-3：撤销条过期（15 秒）时顺手清理（定时器负责主动通知）。
        if self
            .last_move_undo
            .as_ref()
            .is_some_and(|r| r.at.elapsed() > Duration::from_secs(UNDO_TTL_SECS))
        {
            self.last_move_undo = None;
        }

        // P3：半屏自动窄轨断点（对齐 Linux adw::Breakpoint MaxWidth 1094px）。
        // gpui 无断点组件，沿用现有 render 帧读取 viewport 的模式（与
        // preferences/reader 相同）：窗口 resize 必然触发重渲染，此处仅在跨越
        // 断点时翻转状态，不每帧改状态。自动折叠不覆盖手动偏好 `sidebar_hidden`。
        let viewport_w = window.viewport_size().width.as_f32();
        let auto_rail = auto_rail_next(viewport_w);
        if auto_rail != self.auto_rail_active {
            self.auto_rail_active = auto_rail;
            // 扩回宽窗口时收起浮层并还原为分栏侧栏（对齐 Linux AutoRail(false)）。
            if !auto_rail {
                self.sidebar_peek_open = false;
            }
        }

        let hide_sidebar = self.focus_mode && self.prefs.focus_hide_sidebar;
        let sidebar_el = if hide_sidebar {
            None
        } else if self.sidebar_narrow() {
            // 折叠为 80px 图标窄轨（对齐原版 rail），不再整体移除左栏；
            // 断点自动窄轨与手动折叠共用同一窄轨形态。
            Some(sidebar::render_collapsed(self, cx))
        } else {
            Some(sidebar::render(self, cx))
        };
        // P3：浮层侧栏渲染参数（仅断点窄轨下打开；专注模式隐藏左栏时不渲染）。
        let peek_open = self.sidebar_peek_open && sidebar_el.is_some();
        let peek_w = peek_sidebar_width(self.sidebar_width, viewport_w);

        // 主内容区路由（方案文档五.1）：Gallery / Contacts 保留左栏，
        // 中栏 + 右栏区域整体替换为对应组件。
        let body = match self.main_view {
            MainContentView::Mail => self.render_mail_body(window, cx, sidebar_el),
            MainContentView::Gallery => {
                let win_controls =
                    window_controls::render_window_controls("gallery-wc", window, cx);
                let gallery_el = match self.gallery.clone() {
                    Some(entity) => div()
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .h_full()
                        .child(entity)
                        .child(
                            h_flex()
                                .absolute()
                                .top_0()
                                .right_0()
                                .h(px(window_controls::HEADER_HEIGHT))
                                .items_center()
                                .child(win_controls),
                        )
                        .into_any_element(),
                    None => div().flex_1().into_any_element(),
                };
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .children(sidebar_el)
                    .child(gallery_el)
                    .into_any_element()
            }
            MainContentView::Contacts => {
                let win_controls =
                    window_controls::render_window_controls("contacts-wc", window, cx);
                let contacts_el = match self.contacts.clone() {
                    Some(entity) => div()
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .h_full()
                        .child(entity)
                        .child(
                            h_flex()
                                .absolute()
                                .top_0()
                                .left(px(300.0))
                                .right_0()
                                .h(px(window_controls::HEADER_HEIGHT))
                                .items_center()
                                .child(window_controls::drag_spacer())
                                .child(win_controls),
                        )
                        .into_any_element(),
                    None => div().flex_1().into_any_element(),
                };
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .children(sidebar_el)
                    .child(contacts_el)
                    .into_any_element()
            }
        };

        let status_top = self.prefs.status_bar_top;
        let mut root = v_flex()
            .relative()
            .size_full()
            .bg(p.canvas)
            .text_color(p.text)
            .when(status_top && !self.status_line.is_empty(), |el| {
                el.child(self.render_status_bar(true, &p, cx))
            })
            .when_some(self.error_banner.as_ref(), |el, err| {
                el.child(
                    h_flex()
                        .px_3()
                        .py_1()
                        .border_b_1()
                        .border_color(border)
                        .justify_between()
                        .items_center()
                        .child(div().text_xs().text_color(p.danger).child(err.clone()))
                        .child(Button::new("dismiss-err").ghost().label("关闭").on_click(
                            cx.listener(|this, _, _, cx| {
                                this.error_banner = None;
                                cx.notify();
                            }),
                        )),
                )
            })
            .when_some(self.pending_send.as_ref().filter(|p| self.demo_mode || p.outbox_id.is_some() || p.enqueue_failed), |el, pending| {
                let token = pending.token;
                let recipient = if pending.draft.to.trim().is_empty() {
                    "(未填写收件人)".to_string()
                } else {
                    pending.draft.to.trim().to_string()
                };
                let secs = pending.remaining_secs;
                let failed = pending.enqueue_failed;
                el.child(
                    h_flex()
                        .px_3()
                        .py_1p5()
                        .gap_2()
                        .border_b_1()
                        .border_color(border)
                        .bg(p.accent_soft())
                        .justify_between()
                        .items_center()
                        .child(
                            div()
                                .text_xs()
                                .font_weight(gpui_kit::FontWeight::MEDIUM)
                                .text_color(p.text)
                                .truncate()
                                .child(SharedString::from(if failed {
                                    format!("发给 {recipient} 的邮件未保存，请恢复编辑")
                                } else {
                                    format!("正在准备发送给 {recipient}（{secs} 秒后发出）...")
                                })),
                        )
                        .child(
                            h_flex()
                                .gap_1p5()
                                .flex_shrink_0()
                                .items_center()
                                .child(
                                    Button::new("undo-send-btn")
                                        .primary()
                                        .compact()
                                        .label(if failed { "恢复邮件" } else { "撤销发送" })
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.undo_pending_send(window, cx);
                                        })),
                                )
                                .when(!failed, |el| el.child(
                                    Button::new("send-now-btn")
                                        .outline()
                                        .compact()
                                        .label("立即发送")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.commit_pending_send(token, cx);
                                        })),
                                )),
                        ),
                )
            })
            // 「清空文件夹」二次确认条（右键 Trash / Junk 触发，Esc 取消）。
            .when_some(self.pending_empty_folder.as_ref(), |el, pending| {
                let (_, _, _, name) = pending.clone();
                el.child(
                    h_flex()
                        .px_3()
                        .py_1p5()
                        .gap_2()
                        .border_b_1()
                        .border_color(border)
                        .bg(p.danger.opacity(0.12))
                        .justify_between()
                        .items_center()
                        .child(
                            div()
                                .text_xs()
                                .font_weight(gpui_kit::FontWeight::MEDIUM)
                                .text_color(p.danger)
                                .truncate()
                                .child(SharedString::from(format!(
                                    "确认清空文件夹「{name}」？其中全部邮件将被永久删除，此操作不可撤销。"
                                ))),
                        )
                        .child(
                            h_flex()
                                .gap_1p5()
                                .flex_shrink_0()
                                .items_center()
                                .child(
                                    Button::new("empty-folder-confirm")
                                        .primary()
                                        .compact()
                                        .label("确认清空")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.confirm_empty_folder(cx);
                                        })),
                                )
                                .child(
                                    Button::new("empty-folder-cancel")
                                        .outline()
                                        .compact()
                                        .label("取消")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.cancel_empty_folder(cx);
                                        })),
                                ),
                        ),
                )
            })
            .child(body)
            // P3：窄窗口浮层侧栏（对齐 Linux sidebar_peek：完整侧栏从窄轨右缘
            // 滑出、盖在中栏列表 / 阅读器之上；导航选择后由各入口自动关闭）。
            // WP-01：浮层下方先铺全屏半透明遮罩（对齐 Linux OverlaySplitView 的
            // scrim），点击遮罩任意区域收起抽屉；侧边栏浮层后挂、绘制在遮罩之上。
            .when(peek_open, |el| {
                el.child(
                    div()
                        .id("sidebar-peek-scrim")
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(px(sidebar::RAIL_WIDTH))
                        .right_0()
                        .bg(black().opacity(0.18))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.sidebar_peek_open = false;
                                cx.notify();
                            }),
                        ),
                )
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(px(sidebar::RAIL_WIDTH))
                        .w(px(peek_w))
                        .shadow_xl()
                        .bg(p.sidebar)
                        .child(sidebar::render(&*self, cx)),
                )
            })
            // L-2：三栏拖拽把手的移动监听（把手元素只负责发起拖拽，宽度更新在此完成），
            // 挂在根节点上使 Mail / Gallery / Contacts 等任意视图下的侧栏把手都生效。
            .on_drag_move(cx.listener(|this, event: &DragMoveEvent<SidebarDrag>, _, cx| {
                this.on_sidebar_drag_move(event, cx);
            }))
            .on_drag_move(cx.listener(|this, event: &DragMoveEvent<ListDrag>, _, cx| {
                this.on_list_drag_move(event, cx);
            }))
            .on_drag_move(cx.listener(|this, event: &DragMoveEvent<BelowDrag>, _, cx| {
                this.on_below_drag_move(event, cx);
            }))
            .when(!status_top && !self.status_line.is_empty(), |el| {
                el.child(self.render_status_bar(false, &p, cx))
            })
            // 全局快捷键动作（键绑定注册在 main.rs，方案文档五.3）。
            .on_action(cx.listener(|this, _: &NewCompose, window, cx| {
                // 浮层打开时不响应（先 Esc 关闭），避免动作穿透。
                if this.active_modal == ActiveModal::None {
                    this.main_view = MainContentView::Mail;
                    this.start_new_compose(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &OpenCommandPalette, window, cx| {
                if this.active_modal == ActiveModal::None {
                    this.open_command_palette(window, cx, false);
                }
            }))
            // Ctrl+Shift+K：命令面板预填 `>`，仅筛选操作命令。
            .on_action(cx.listener(|this, _: &OpenCommandPaletteActions, window, cx| {
                if this.active_modal == ActiveModal::None {
                    this.open_command_palette(window, cx, true);
                }
            }))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                // 仅三栏视图且无浮层时：若阅读器页内查找已打开或处于专注阅读模式，则聚焦页内查找，
                // 否则展开并聚焦中栏搜索栏（Ctrl+F / `/`）。
                if this.main_view == MainContentView::Mail && this.active_modal == ActiveModal::None
                {
                    if this.reader_find_open
                        || (this.focus_mode
                            && this.right_mode == RightPaneMode::Reader
                            && this.selected_message.is_some())
                    {
                        this.reader_find_open = true;
                        this.reader_find_input.update(cx, |state, cx| {
                            state.focus(window, cx);
                        });
                        cx.notify();
                    } else {
                        this.search_open = true;
                        this.search_input.update(cx, |state, cx| {
                            state.focus(window, cx);
                        });
                        cx.notify();
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &RefreshAllAccounts, _window, cx| {
                if this.active_modal == ActiveModal::None {
                    this.refresh_all(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SendCompose, _window, cx| {
                if this.active_modal == ActiveModal::None
                    && this.main_view == MainContentView::Mail
                    && this.right_mode == RightPaneMode::Compose
                {
                    this.send_compose(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &OpenPreferences, window, cx| {
                if this.active_modal == ActiveModal::None {
                    this.open_preferences(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &QuitApp, _window, cx| {
                this.quit_app(cx);
            }))
            // F11 全屏切换（不设浮层守卫：全屏与浮层互不干扰）。
            .on_action(cx.listener(|this, _: &ToggleFullscreen, window, cx| {
                this.toggle_fullscreen(window, cx);
            }))
            // Ctrl+Shift+A 显示 / 隐藏账号列表（对齐原版 show-accounts）。
            .on_action(cx.listener(|this, _: &ToggleAccountsList, _window, cx| {
                this.toggle_accounts_list(cx);
            }))
            // L-6：阅读器位置（Ctrl+Alt+1/2/3，对齐原版 reading-pane 单选动作）。
            .on_action(cx.listener(|this, _: &SetReaderPositionRight, _window, cx| {
                if this.active_modal == ActiveModal::None {
                    this.set_reader_position(ReaderPosition::Right, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SetReaderPositionBelow, _window, cx| {
                if this.active_modal == ActiveModal::None {
                    this.set_reader_position(ReaderPosition::Below, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SetReaderPositionHidden, _window, cx| {
                if this.active_modal == ActiveModal::None {
                    this.set_reader_position(ReaderPosition::Hidden, cx);
                }
            }))
            // L-6：专注模式开关（Ctrl+Shift+F，对齐原版 focus-mode）。
            .on_action(cx.listener(|this, _: &ToggleFocusMode, _window, cx| {
                if this.active_modal == ActiveModal::None {
                    this.toggle_focus_mode(cx);
                }
            }))
            // F1 / ? 快捷键速查：无浮层或已在速查浮层时响应（同键再按关闭）。
            .on_action(cx.listener(|this, _: &OpenShortcutsWindow, _window, cx| {
                if this.active_modal == ActiveModal::None
                    || this.active_modal == ActiveModal::Shortcuts
                {
                    this.open_shortcuts(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &DeleteSelectedMessage, window, cx| {
                if this.active_modal == ActiveModal::None
                    && this.main_view == MainContentView::Mail
                    && this.right_mode == RightPaneMode::Reader
                    && !this.is_text_input_focused(window, cx)
                {
                    if this.selected_message_ids.len() > 1 {
                        this.batch_delete(cx);
                    } else {
                        this.delete_selected(cx);
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &SelectNextMessage, window, cx| {
                if this.active_modal == ActiveModal::None
                    && this.main_view == MainContentView::Mail
                    && this.right_mode == RightPaneMode::Reader
                    && !this.is_text_input_focused(window, cx)
                {
                    this.select_adjacent_message(1, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SelectPrevMessage, window, cx| {
                if this.active_modal == ActiveModal::None
                    && this.main_view == MainContentView::Mail
                    && this.right_mode == RightPaneMode::Reader
                    && !this.is_text_input_focused(window, cx)
                {
                    this.select_adjacent_message(-1, cx);
                }
            }))
            // F-3：Ctrl+Z 撤销最近一次移动 / 删除 / 归档（条件与 DeleteSelectedMessage 一致）。
            .on_action(cx.listener(|this, _: &UndoLastMove, window, cx| {
                if this.active_modal == ActiveModal::None
                    && this.main_view == MainContentView::Mail
                    && this.right_mode == RightPaneMode::Reader
                    && !this.is_text_input_focused(window, cx)
                {
                    this.undo_last_move(cx);
                }
            }))
            // 阅读器缩放快捷键（Ctrl+= / Ctrl+- / Ctrl+0，复用现有持久化路径）。
            .on_action(cx.listener(|this, _: &ZoomIn, window, cx| {
                if this.active_modal == ActiveModal::None
                    && this.main_view == MainContentView::Mail
                    && this.right_mode == RightPaneMode::Reader
                    && !this.is_text_input_focused(window, cx)
                {
                    this.adjust_reader_zoom(10, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ZoomOut, window, cx| {
                if this.active_modal == ActiveModal::None
                    && this.main_view == MainContentView::Mail
                    && this.right_mode == RightPaneMode::Reader
                    && !this.is_text_input_focused(window, cx)
                {
                    this.adjust_reader_zoom(-10, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ZoomReset, window, cx| {
                if this.active_modal == ActiveModal::None
                    && this.main_view == MainContentView::Mail
                    && this.right_mode == RightPaneMode::Reader
                    && !this.is_text_input_focused(window, cx)
                {
                    this.set_reader_zoom(100, cx);
                }
            }))
            // ---- F-1 阶段三：组合键动作（始终生效，不受单键偏好控制）----
            // Ctrl+R 回复当前邮件（原 RefreshAllAccounts 语义移除，刷新仅保留 F5）。
            .on_action(cx.listener(|this, _: &ReplyCurrent, window, cx| {
                if this.active_modal == ActiveModal::None
                    && this.main_view == MainContentView::Mail
                    && this.right_mode == RightPaneMode::Reader
                    && !this.is_text_input_focused(window, cx)
                {
                    if let Some(detail) = this.selected_message.clone() {
                        this.start_reply(&detail, window, cx);
                    }
                }
            }))
            // Ctrl+Shift+R 回复全部。
            .on_action(cx.listener(|this, _: &ReplyAllCurrent, window, cx| {
                if this.active_modal == ActiveModal::None
                    && this.main_view == MainContentView::Mail
                    && this.right_mode == RightPaneMode::Reader
                    && !this.is_text_input_focused(window, cx)
                {
                    if let Some(detail) = this.selected_message.clone() {
                        this.start_reply_all(&detail, window, cx);
                    }
                }
            }))
            // Ctrl+U 查看原文（再按返回正文，复用 toggle_raw_source 的切换语义）。
            .on_action(cx.listener(|this, _: &ViewMessageSource, window, cx| {
                if this.active_modal == ActiveModal::None
                    && this.main_view == MainContentView::Mail
                    && this.right_mode == RightPaneMode::Reader
                    && !this.is_text_input_focused(window, cx)
                {
                    if let Some(detail) = this.selected_message.clone() {
                        this.toggle_raw_source(detail.id, cx);
                    }
                }
            }))
            // ---- F-1 阶段三：单键动作（single_key_guard_allows 含偏好开关检查）----
            .on_action(cx.listener(|this, _: &SingleKeyNextMessage, window, cx| {
                if single_key_guard_allows(
                    this.active_modal,
                    this.main_view,
                    this.right_mode,
                    this.is_text_input_focused(window, cx),
                    this.prefs.single_key_shortcuts,
                ) {
                    this.select_adjacent_message(1, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SingleKeyPrevMessage, window, cx| {
                if single_key_guard_allows(
                    this.active_modal,
                    this.main_view,
                    this.right_mode,
                    this.is_text_input_focused(window, cx),
                    this.prefs.single_key_shortcuts,
                ) {
                    this.select_adjacent_message(-1, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SingleKeyReply, window, cx| {
                if single_key_guard_allows(
                    this.active_modal,
                    this.main_view,
                    this.right_mode,
                    this.is_text_input_focused(window, cx),
                    this.prefs.single_key_shortcuts,
                ) {
                    if let Some(detail) = this.selected_message.clone() {
                        this.start_reply(&detail, window, cx);
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &SingleKeyReplyAll, window, cx| {
                if single_key_guard_allows(
                    this.active_modal,
                    this.main_view,
                    this.right_mode,
                    this.is_text_input_focused(window, cx),
                    this.prefs.single_key_shortcuts,
                ) {
                    if let Some(detail) = this.selected_message.clone() {
                        this.start_reply_all(&detail, window, cx);
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &SingleKeyForward, window, cx| {
                if single_key_guard_allows(
                    this.active_modal,
                    this.main_view,
                    this.right_mode,
                    this.is_text_input_focused(window, cx),
                    this.prefs.single_key_shortcuts,
                ) {
                    if let Some(detail) = this.selected_message.clone() {
                        this.start_forward(&detail, window, cx);
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &SingleKeyArchive, window, cx| {
                if single_key_guard_allows(
                    this.active_modal,
                    this.main_view,
                    this.right_mode,
                    this.is_text_input_focused(window, cx),
                    this.prefs.single_key_shortcuts,
                ) {
                    this.archive_selected(cx);
                }
            }))
            // d / Backspace 删除：多选时走批量删除（与 DeleteSelectedMessage 一致）。
            .on_action(cx.listener(|this, _: &SingleKeyDelete, window, cx| {
                if single_key_guard_allows(
                    this.active_modal,
                    this.main_view,
                    this.right_mode,
                    this.is_text_input_focused(window, cx),
                    this.prefs.single_key_shortcuts,
                ) {
                    if this.selected_message_ids.len() > 1 {
                        this.batch_delete(cx);
                    } else {
                        this.delete_selected(cx);
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &SingleKeyMarkSpam, window, cx| {
                if single_key_guard_allows(
                    this.active_modal,
                    this.main_view,
                    this.right_mode,
                    this.is_text_input_focused(window, cx),
                    this.prefs.single_key_shortcuts,
                ) {
                    this.mark_spam_selected(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SingleKeyToggleStar, window, cx| {
                if single_key_guard_allows(
                    this.active_modal,
                    this.main_view,
                    this.right_mode,
                    this.is_text_input_focused(window, cx),
                    this.prefs.single_key_shortcuts,
                ) {
                    if let Some(detail) = this.selected_message.clone() {
                        this.toggle_flagged(&detail, cx);
                    }
                }
            }))
            .on_action(cx.listener(|this, _: &SingleKeyToggleRead, window, cx| {
                if single_key_guard_allows(
                    this.active_modal,
                    this.main_view,
                    this.right_mode,
                    this.is_text_input_focused(window, cx),
                    this.prefs.single_key_shortcuts,
                ) {
                    this.toggle_message_seen(cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SingleKeyCompose, window, cx| {
                if single_key_guard_allows(
                    this.active_modal,
                    this.main_view,
                    this.right_mode,
                    this.is_text_input_focused(window, cx),
                    this.prefs.single_key_shortcuts,
                ) {
                    this.start_new_compose(window, cx);
                }
            }))
            // u 返回列表：清空选中锚点，阅读态回到「选择一封邮件查看正文」空态。
            .on_action(cx.listener(|this, _: &SingleKeyBackToList, window, cx| {
                if single_key_guard_allows(
                    this.active_modal,
                    this.main_view,
                    this.right_mode,
                    this.is_text_input_focused(window, cx),
                    this.prefs.single_key_shortcuts,
                ) {
                    if this.selected_message.take().is_some() {
                        cx.notify();
                    }
                }
            }))
            // w / b 会话内下一张 / 上一张卡片。
            .on_action(cx.listener(|this, _: &SingleKeyNextCard, window, cx| {
                if single_key_guard_allows(
                    this.active_modal,
                    this.main_view,
                    this.right_mode,
                    this.is_text_input_focused(window, cx),
                    this.prefs.single_key_shortcuts,
                ) {
                    this.step_thread_card(1, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SingleKeyPrevCard, window, cx| {
                if single_key_guard_allows(
                    this.active_modal,
                    this.main_view,
                    this.right_mode,
                    this.is_text_input_focused(window, cx),
                    this.prefs.single_key_shortcuts,
                ) {
                    this.step_thread_card(-1, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &CloseOverlay, window, cx| {
                if this.print_preview.is_some() {
                    this.close_print_preview(cx);
                    return;
                }
                if this.drawer_image_preview.is_some() {
                    this.close_drawer_image_preview(cx);
                    return;
                }
                if this.reader_find_open {
                    this.toggle_reader_find(window, cx);
                    return;
                }
                // Esc 优先级：收起抽屉侧栏 > 关闭浮层 > 清空文件夹确认 > 写信退出确认
                // > 撤销发送 > 取消多选。WP-01：半屏抽屉对齐 Linux OverlaySplitView，
                // Esc 先收起抽屉。
                if this.sidebar_peek_open {
                    this.sidebar_peek_open = false;
                    cx.notify();
                    return;
                }
                if this.active_modal != ActiveModal::None {
                    this.close_modal(cx);
                } else if this.pending_empty_folder.is_some() {
                    this.cancel_empty_folder(cx);
                } else if this.main_view == MainContentView::Mail
                    && this.right_mode == RightPaneMode::Compose
                {
                    this.request_close_compose(cx);
                } else if this.pending_send.is_some() && !this.is_text_input_focused(window, cx) {
                    this.undo_pending_send(window, cx);
                } else if !this.selected_message_ids.is_empty() {
                    this.clear_message_selection(cx);
                }
            }));

        // 全局浮层：快捷键速查 / 关于 / 命令面板。
        match self.active_modal {
            ActiveModal::CommandPalette => {
                if let Some(entity) = self.palette.clone() {
                    root = root.child(entity);
                }
            }
            ActiveModal::Shortcuts => {
                root = root.child(
                    div()
                        .absolute()
                        .top(px(0.0))
                        .left(px(0.0))
                        .size_full()
                        .bg(black().opacity(0.4))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.close_modal(cx);
                            }),
                        )
                        .child(
                            h_flex().size_full().items_center().justify_center().child(
                                v_flex()
                                    .w(px(600.0))
                                    .h(px(560.0))
                                    .bg(p.surface)
                                    .rounded(px(12.0))
                                    .border_1()
                                    .border_color(border)
                                    .shadow_xl()
                                    .overflow_hidden()
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|_, _, _, cx| {
                                            cx.stop_propagation();
                                        }),
                                    )
                                    .child(shortcuts_modal(&p, cx)),
                            ),
                        ),
                );
            }
            ActiveModal::About => {
                root = root.child(
                    div()
                        .absolute()
                        .top(px(0.0))
                        .left(px(0.0))
                        .size_full()
                        .bg(black().opacity(0.4))
                        // 挡住下层（收件箱列表）的 hover/cursor：否则指针移到浮层上，
                        // 下层命中盒仍判定为悬停，出现行高亮和手型光标。
                        .occlude()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.close_modal(cx);
                            }),
                        )
                        .child(
                            h_flex().size_full().items_center().justify_center().child(
                                v_flex()
                                    .w(px(440.0))
                                    .h(px(620.0))
                                    .bg(p.surface)
                                    .rounded(px(12.0))
                                    .border_1()
                                    .border_color(border)
                                    .shadow_xl()
                                    .overflow_hidden()
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(|_, _, _, cx| {
                                            cx.stop_propagation();
                                        }),
                                    )
                                    .child(about_modal(self, &p, border, cx)),
                            ),
                        ),
                );
            }
            ActiveModal::None => {}
        }
        if let Some(ref pp) = self.print_preview {
            root = root.child(print_preview::render_modal(
                pp,
                &self.prefs.date_style,
                &self.prefs.clock_style,
                cx,
            ));
        }
        if let Some((_att_id, ref filename, ref image_opt)) = self.drawer_image_preview {
            let fname = filename.clone();
            let img_opt = image_opt.clone();
            root = root.child(
                div()
                    .id("drawer-image-lightbox")
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .bg(black().opacity(0.65))
                    .flex()
                    .items_center()
                    .justify_center()
                    .p_8()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.close_drawer_image_preview(cx);
                        }),
                    )
                    .child(
                        v_flex()
                            .max_w(px(860.0))
                            .max_h(px(680.0))
                            .bg(p.surface)
                            .rounded(px(12.0))
                            .border_1()
                            .border_color(border)
                            .p_4()
                            .gap_3()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                cx.stop_propagation();
                            })
                            .child(
                                h_flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                            .child(SharedString::from(fname)),
                                    )
                                    .child(
                                        Button::new("drawer-lightbox-close")
                                            .ghost()
                                            .compact()
                                            .icon(IconName::X)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.close_drawer_image_preview(cx);
                                            })),
                                    ),
                            )
                            .child(if let Some(image) = img_opt {
                                img(image)
                                    .object_fit(ObjectFit::Contain)
                                    .max_w(px(820.0))
                                    .max_h(px(580.0))
                                    .into_any_element()
                            } else {
                                div()
                                    .p_8()
                                    .text_xs()
                                    .text_color(p.text_muted)
                                    .child("正在加载图片预览...")
                                    .into_any_element()
                            }),
                    ),
            );
        }
        root
    }
}

/// 快捷键速查分组数据（键位 + 说明）。F-1 阶段三起与 main.rs bind_keys /
/// on_action 实际生效的绑定逐条对应；未实现的键（l / h / x 等）不列出，
/// 避免「写了但不生效」的误导。
const SHORTCUT_GROUPS: &[(&str, &[(&str, &str)])] = &[
    (
        "移动",
        &[
            ("下一条邮件", "j / ↓"),
            ("上一条邮件", "k / ↑"),
            ("返回邮件列表", "u"),
            ("会话内下一张卡片", "w"),
            ("会话内上一张卡片", "b"),
            ("搜索", "/"),
            ("写邮件", "c"),
            ("本清单", "?"),
            ("删除选中邮件", "Delete"),
        ],
    ),
    (
        "对邮件的操作",
        &[
            ("回复", "r"),
            ("回复全部", "R"),
            ("转发", "f"),
            ("归档", "a"),
            ("删除", "d / Backspace / Delete"),
            ("标记为垃圾邮件", "!"),
            ("加星标 / 取消星标", "s"),
            ("已读 / 未读切换", "m"),
        ],
    ),
    (
        "全局快捷键（始终可用）",
        &[
            ("写新邮件", "Ctrl+N"),
            ("命令面板", "Ctrl+K"),
            ("命令面板（仅操作）", "Ctrl+Shift+K"),
            ("回复当前邮件", "Ctrl+R"),
            ("回复全部", "Ctrl+Shift+R"),
            ("查看邮件原文", "Ctrl+U"),
            ("检查新邮件", "F5"),
            ("搜索邮件列表", "Ctrl+F"),
            ("显示 / 隐藏侧栏账号列表", "Ctrl+Shift+A"),
            ("阅读器移到右侧 / 下方 / 关闭", "Ctrl+Alt+1 / 2 / 3"),
            ("专注模式", "Ctrl+Shift+F"),
            ("全屏切换", "F11"),
            ("本清单", "F1 / ?"),
            ("打开偏好设置", "Ctrl+,"),
            ("退出 Hylki", "Ctrl+Q"),
            ("关闭浮层 / 返回", "Esc"),
        ],
    ),
];

/// 键盘快捷键速查弹窗内容（F1 / ?，风格复用现有模态卡片）。
fn shortcuts_modal(p: &theme::HylkiPalette, cx: &mut Context<HylkiApp>) -> gpui_kit::AnyElement {
    let (text, muted_fg, accent, accent_soft, border) =
        (p.text, p.text_muted, p.accent, p.accent_soft(), p.border);
    let mut list = v_flex()
        .id("shortcuts-list")
        .flex_1()
        .min_h_0()
        .overflow_y_scrollbar()
        .px_5()
        .py_3()
        .gap_1p5();
    for (group_title, rows) in SHORTCUT_GROUPS {
        list = list
            .child(
                div()
                    .pt_2()
                    .pb_1()
                    .text_sm()
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .text_color(accent)
                    .child(SharedString::from(*group_title)),
            )
            .children(rows.iter().map(|(desc, keys)| {
                h_flex()
                    .justify_between()
                    .items_center()
                    .min_h(px(28.0))
                    .gap_3()
                    .child(
                        div()
                            .text_sm()
                            .text_color(text)
                            .truncate()
                            .child(SharedString::from(*desc)),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .px_1p5()
                            .py_0p5()
                            .rounded(px(6.0))
                            .bg(accent_soft)
                            .text_xs()
                            .text_color(accent)
                            .child(SharedString::from(*keys)),
                    )
            }));
    }
    v_flex()
        .flex_1()
        .min_h_0()
        .child(
            // 标题栏：标题 + 关闭按钮（Esc 亦可关闭）。
            h_flex()
                .flex_shrink_0()
                .px_5()
                .py_3()
                .items_start()
                .justify_between()
                .border_b_1()
                .border_color(border)
                .child(
                    v_flex().gap_1().child(
                        div()
                            .text_base()
                            .font_weight(gpui_kit::FontWeight::BOLD)
                            .text_color(text)
                            .child("键盘快捷键"),
                    ),
                )
                .child(
                    Button::new("shortcuts-close")
                        .ghost()
                        .compact()
                        .icon(IconName::X)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.close_modal(cx);
                        })),
                ),
        )
        .child(list)
        .child(
            div()
                .flex_shrink_0()
                .px_5()
                .py_2()
                .border_t_1()
                .border_color(border)
                .text_xs()
                .text_color(muted_fg)
                .child("单键快捷键可在 偏好设置 → 常规 → 「启用单键快捷键」关闭；关闭后上表单键（j / k / r / a 等）停用，Ctrl 组合键与 Delete / F5 不受影响。"),
        )
        .into_any_element()
}

/// 「关于 Hylki」弹窗内容（对齐原版 about.rs：版本、描述、协议、链接与致谢）。
fn about_modal(
    app: &HylkiApp,
    p: &theme::HylkiPalette,
    border: gpui_kit::Hsla,
    cx: &mut Context<HylkiApp>,
) -> gpui_kit::AnyElement {
    let (text, muted_fg, accent, accent_soft) = (p.text, p.text_muted, p.accent, p.accent_soft());
    let has_update = app.update.info.is_some();
    let busy = app.update.checking || app.update.downloading;
    let update_status: SharedString = if app.update.downloading {
        "正在下载更新包…".into()
    } else if app.update.checking {
        "正在检查更新…".into()
    } else if let Some(error) = &app.update.error {
        error.clone()
    } else if let Some(info) = &app.update.info {
        format!(
            "发现新版本 v{}（当前 v{}）",
            info.version, info.current_version
        )
        .into()
    } else if let Some(status) = &app.update.status {
        status.clone()
    } else {
        SharedString::default()
    };
    let update_status_color = if app.update.error.is_some() {
        p.danger
    } else {
        muted_fg
    };
    let update_button = if app.update.downloading {
        "正在下载…"
    } else if app.update.checking {
        "检查中…"
    } else if has_update {
        "下载并安装"
    } else {
        "检查更新"
    };
    let notes = app.update.info.as_ref().and_then(|info| {
        let notes = info.notes.trim().replace("\r\n", "\n");
        if notes.is_empty() {
            return None;
        }
        let mut short: String = notes.chars().take(280).collect();
        if notes.chars().count() > 280 {
            short.push('…');
        }
        Some(short)
    });
    let repo = "https://github.com/ISuuuu/hylki";
    let issues = "https://github.com/ISuuuu/hylki/issues";
    let license_url = "https://www.gnu.org/licenses/agpl-3.0.html";
    let link_row =
        |cx: &mut Context<HylkiApp>, id: &'static str, label: &'static str, url: &'static str| {
            h_flex()
                .w_full()
                .justify_between()
                .items_center()
                .min_h(px(30.0))
                .child(
                    div()
                        .text_sm()
                        .truncate()
                        .text_color(p.text)
                        .child(SharedString::from(label)),
                )
                .child(
                    Button::new(id)
                        .outline()
                        .compact()
                        .label("打开")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.open_link(url, cx);
                        })),
                )
        };
    v_flex()
        .flex_1()
        .min_h_0()
        .px_6()
        .py_5()
        .gap_2()
        .child(
            h_flex()
                .w_full()
                .justify_between()
                .items_center()
                .child(div())
                .child(
                    Button::new("about-close")
                        .ghost()
                        .compact()
                        .icon(IconName::X)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.close_modal(cx);
                        })),
                ),
        )
        .child(
            v_flex().items_center().gap_1().child(
                div()
                    .text_2xl()
                    .font_weight(gpui_kit::FontWeight::BOLD)
                    .text_color(text)
                    .child("Hylki"),
            ),
        )
        .child(
            div()
                .mx_auto()
                .px_2()
                .py_0p5()
                .rounded_full()
                .bg(accent_soft)
                .text_xs()
                .text_color(accent)
                .child(SharedString::from(format!("版本 {}", app_version()))),
        )
        .child(
            div()
                .mt_1()
                .text_center()
                .text_sm()
                .text_color(muted_fg)
                .child("一个快速、原生的电子邮件客户端"),
        )
        .child(
            div()
                .mt_1()
                .text_center()
                .text_xs()
                .text_color(muted_fg)
                .child("Windows 版：GPUI 原生界面，共享邮件内核。"),
        )
        .child(
            div()
                .mt_1()
                .text_center()
                .text_xs()
                .text_color(muted_fg)
                .child("本程序基于 GNU AGPL-3.0 或更新版本授权发布，不含任何形式的担保。"),
        )
        .child(div().mt_3().w_full().h(px(1.0)).bg(border))
        .child(
            div()
                .mt_2()
                .text_sm()
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .text_color(accent)
                .child("项目"),
        )
        .children([
            link_row(cx, "about-link-repo", "源代码", repo),
            link_row(cx, "about-link-issues", "问题反馈", issues),
            link_row(
                cx,
                "about-link-license",
                "许可证 (GNU AGPL v3)",
                license_url,
            ),
        ])
        .child(div().mt_3().w_full().h(px(1.0)).bg(border))
        .child(
            div()
                .mt_2()
                .text_sm()
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .text_color(accent)
                .child("更新"),
        )
        .child(
            h_flex()
                .w_full()
                .justify_between()
                .items_center()
                .min_h(px(30.0))
                .gap_3()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_xs()
                        .text_color(update_status_color)
                        .child(update_status),
                )
                .child(
                    Button::new("about-update")
                        .outline()
                        .compact()
                        .disabled(busy)
                        .label(update_button)
                        .on_click(cx.listener(|this, _, _, cx| {
                            if this.update.info.is_some() {
                                this.download_and_install_update(cx);
                            } else {
                                this.check_updates(true, cx);
                            }
                        })),
                ),
        )
        .when_some(notes, |el, notes| {
            el.child(
                div()
                    .mt_1()
                    .text_xs()
                    .text_color(muted_fg)
                    .child(SharedString::from(notes)),
            )
        })
        .child(div().mt_3().w_full().h(px(1.0)).bg(border))
        .child(
            div()
                .mt_2()
                .text_center()
                .text_xs()
                .text_color(muted_fg)
                .child("感谢所有贡献者与翻译者（详见仓库 data/CONTRIBUTORS 与 data/TRANSLATORS）"),
        )
        .child(
            div()
                .text_center()
                .text_xs()
                .text_color(muted_fg)
                .child("© 2026 Hyprlab"),
        )
        .into_any_element()
}

/// 检测单个账号的收件箱未读增长量：首次出现仅建立基线返回 0，避免启动加载误报通知。
pub(crate) fn detect_new_unread(
    prev_by_account: &mut BTreeMap<u32, u32>,
    account_id: u32,
    current_inbox_unread: u32,
) -> u32 {
    let delta = match prev_by_account.get(&account_id).copied() {
        Some(prev) if current_inbox_unread > prev => current_inbox_unread - prev,
        _ => 0,
    };
    prev_by_account.insert(account_id, current_inbox_unread);
    delta
}

/// 账号是否在统一视图范围内：已启用且加入统一收件箱（任务四）。
/// 与 worker 端 `unified_account_ids` 语义一致；本地未知的账号按范围外处理。
pub(crate) fn account_in_unified_scope(accounts: &[AccountConfig], account_id: u32) -> bool {
    accounts
        .iter()
        .find(|a| a.id == account_id)
        .is_some_and(|a| a.enabled && a.in_unified)
}

/// 统一收件箱视图是否仍有更早的远端历史邮件（任务四纯逻辑抽出便于单测）。
/// 只统计 `enabled && in_unified` 账号的 INBOX，与列表、未读数、分页拉取同范围。
pub(crate) fn unified_view_has_more_remote(
    accounts: &[AccountConfig],
    folders_by_account: &BTreeMap<u32, Vec<FolderSummary>>,
) -> bool {
    folders_by_account
        .iter()
        .filter(|(&account_id, _)| account_in_unified_scope(accounts, account_id))
        .flat_map(|(_, folders)| folders.iter())
        .any(|f| {
            (f.path.eq_ignore_ascii_case("INBOX")
                || f.name.eq_ignore_ascii_case("INBOX")
                || f.name == "收件箱")
                && f.has_more_remote
        })
}

/// 任务四：后台历史索引驱动状态（纯逻辑抽出便于单测）。
/// 事件驱动排队——最多保留一个待处理批次，防止批次在队列里堆积；
/// worker 回「无待索引目标」后本轮停止，账号范围变化后重新允许探测。
#[derive(Default)]
pub(crate) struct BackfillDriveState {
    /// 已向 worker 排队但完成事件未到的批次。
    pub(crate) in_flight: bool,
    /// worker 已回无待索引目标：本轮结束，不再续排。
    pub(crate) exhausted: bool,
}

impl BackfillDriveState {
    /// 当前是否应向队列排下一个后台索引批次。
    pub(crate) fn should_queue(&self) -> bool {
        !self.in_flight && !self.exhausted
    }

    fn on_batch_queued(&mut self) {
        self.in_flight = true;
    }

    /// 批次完成事件到达：清除在途标志；无剩余候选时结束本轮。
    fn on_batch_done(&mut self, has_more_remote: bool) {
        self.in_flight = false;
        if !has_more_remote {
            self.exhausted = true;
        }
    }

    /// 账号范围可能变化（账号保存 / 启停 / 新文件夹出现）：重新允许探测。
    fn on_scope_refresh(&mut self) {
        self.exhausted = false;
    }
}

/// 将 GPUI `WindowBounds` 写入偏好中的窗口尺寸与最大化标记（忽略最小化时的退化尺寸）。
pub(crate) fn capture_window_geometry(prefs: &mut AppPreferences, bounds: WindowBounds) -> bool {
    let (maximized, rect) = match bounds {
        WindowBounds::Windowed(b) => (false, b),
        WindowBounds::Maximized(b) => (true, b),
        WindowBounds::Fullscreen(b) => (prefs.window_maximized, b),
    };
    let w: f32 = rect.size.width.into();
    let h: f32 = rect.size.height.into();
    let mut changed = false;
    if prefs.window_maximized != maximized {
        prefs.window_maximized = maximized;
        changed = true;
    }
    if w.is_finite() && h.is_finite() && w >= 320.0 && h >= 240.0 {
        if prefs.window_width != Some(w) || prefs.window_height != Some(h) {
            prefs.window_width = Some(w);
            prefs.window_height = Some(h);
            changed = true;
        }
    }
    changed
}

/// 计算多封会话打开时默认折叠的卡片 ID 集合：
/// - `"never"`: 全部展开；
/// - `"read"`: 折叠已读非当前、非最新的历史邮件；
/// - `"all_but_newest"`: 折叠除最新一封与当前选中封之外的全部邮件。
fn compute_default_collapsed_cards(
    detail: &MessageDetail,
    thread_newest_first: bool,
    fold_mode: &str,
) -> HashSet<u32> {
    if fold_mode == "never" || detail.thread_messages.len() < 2 {
        return HashSet::new();
    }
    let newest_id = if thread_newest_first {
        detail.thread_messages.first().map(|m| m.id)
    } else {
        detail.thread_messages.last().map(|m| m.id)
    };
    detail
        .thread_messages
        .iter()
        .filter(|m| {
            let keep_open = m.id == detail.id || Some(m.id) == newest_id;
            if keep_open {
                return false;
            }
            if fold_mode == "read" {
                !m.unread
            } else {
                true
            }
        })
        .map(|m| m.id)
        .collect()
}

/// 拆分发件人字段为 (显示名, 邮箱)：`"Marcus Chen <marcus@x>"` →
/// `("Marcus Chen", "marcus@x")`；无尖括号时整体为显示名。
pub(crate) fn split_sender(sender: &str) -> (String, String) {
    if let (Some(open), Some(close)) = (sender.find('<'), sender.rfind('>')) {
        if close > open {
            let name = sender[..open].trim().to_string();
            let email = sender[open + 1..close].trim().to_string();
            return (name, email);
        }
    }
    (sender.trim().to_string(), String::new())
}

/// 若账号配置了非空签名，则按是否需要 `-- ` 分隔符格式化后插入正文末尾（新建草稿用）。
pub(crate) fn append_signature_with_dashes(
    body: &str,
    signature: &str,
    with_dashes: bool,
) -> String {
    let sig = signature.trim_end();
    if sig.trim().is_empty() {
        return body.to_string();
    }
    let block = if with_dashes {
        format!("-- \n{sig}")
    } else {
        sig.to_string()
    };
    if body.is_empty() {
        format!("\n\n{block}")
    } else {
        format!("{}\n\n{block}", body.trim_end_matches('\n'))
    }
}

/// 若账号配置了非空签名，则使用标准 `-- \n` 分隔符将其插入正文末尾。
#[cfg(test)]
pub(crate) fn append_signature(body: &str, signature: &str) -> String {
    append_signature_with_dashes(body, signature, true)
}

/// 回复/转发时按设置决定将签名置于引用正文之前或邮件最末尾。
pub(crate) fn insert_signature_with_quote(
    quote_block: &str,
    signature: &str,
    above_quote: bool,
    with_dashes: bool,
) -> String {
    let sig = signature.trim_end();
    if sig.trim().is_empty() {
        return quote_block.to_string();
    }
    let block = if with_dashes {
        format!("-- \n{sig}")
    } else {
        sig.to_string()
    };
    if above_quote {
        format!("\n\n{block}\n\n{}", quote_block.trim_start_matches('\n'))
    } else {
        format!("{}\n\n{block}", quote_block.trim_end_matches('\n'))
    }
}

/// 判断写信表单是否存在未保存内容（任意地址/主题/正文非空或有附件）。
pub(crate) fn has_unsaved_compose_content(
    to: &str,
    cc: &str,
    bcc: &str,
    subject: &str,
    body: &str,
    attachment_count: usize,
) -> bool {
    !to.trim().is_empty()
        || !cc.trim().is_empty()
        || !bcc.trim().is_empty()
        || !subject.trim().is_empty()
        || !body.trim().is_empty()
        || attachment_count > 0
}

/// 根据收件人输入框最后一段（最后一个逗号后）过滤匹配的已知联系人，最多返回 `max_items` 条。
pub(crate) fn filter_contact_suggestions<'a>(
    input: &str,
    contacts: &'a [KnownContact],
    max_items: usize,
) -> Vec<&'a KnownContact> {
    if max_items == 0 {
        return Vec::new();
    }
    let parts: Vec<&str> = input.split(',').collect();
    let Some(last) = parts.last() else {
        return Vec::new();
    };
    let token = last.trim().to_lowercase();
    if token.is_empty() {
        return Vec::new();
    }
    // 排除前面已输入的邮箱，避免重复建议。
    let existing: HashSet<String> = parts[..parts.len().saturating_sub(1)]
        .iter()
        .map(|p| {
            let (_, email) = split_sender(p);
            if email.is_empty() {
                p.trim().to_lowercase()
            } else {
                email.to_lowercase()
            }
        })
        .filter(|s| !s.is_empty())
        .collect();

    contacts
        .iter()
        .filter(|c| !existing.contains(&c.email.to_lowercase()))
        .filter(|c| {
            c.name.to_lowercase().contains(&token) || c.email.to_lowercase().contains(&token)
        })
        .take(max_items)
        .collect()
}

/// 将选中的联系人补全到当前收件人输入字符串末尾，并追加 `, `。
pub(crate) fn apply_contact_completion(current: &str, contact: &KnownContact) -> String {
    let formatted = if contact.name.trim().is_empty() {
        contact.email.clone()
    } else {
        format!("{} <{}>", contact.name.trim(), contact.email)
    };
    let mut parts: Vec<String> = current.split(',').map(|p| p.trim().to_string()).collect();
    if !parts.is_empty() {
        parts.pop();
    }
    parts.retain(|p| !p.is_empty());
    parts.push(formatted);
    format!("{}, ", parts.join(", "))
}

/// 计算方向键切换上一封 / 下一封邮件的目标索引。
pub(crate) fn next_adjacent_index(
    current: Option<usize>,
    len: usize,
    delta: isize,
) -> Option<usize> {
    if len == 0 {
        return None;
    }
    match current {
        None => {
            if delta >= 0 {
                Some(0)
            } else {
                Some(len - 1)
            }
        }
        Some(idx) => {
            let next = idx as isize + delta;
            if next < 0 || next >= len as isize {
                None
            } else {
                Some(next as usize)
            }
        }
    }
}

/// F-2：删除 / 归档后自动推进的目标下标——优先选中同位置的后一封（原 `removed + 1`，
/// 移除后即原位），已是最后一封则选上一封；列表只剩这一封时不选。
pub(crate) fn next_index_after_removal(removed: usize, len: usize) -> Option<usize> {
    if len <= 1 {
        return None;
    }
    if removed + 1 < len {
        Some(removed + 1)
    } else {
        Some(removed - 1)
    }
}

/// B-2：判定文件夹是否为回收站 / 垃圾邮件（语义对齐 cache.rs 的
/// TRASH_FOLDER_MATCH / JUNK_FOLDER_MATCH 与原版 FolderKind::Trash / Junk）。
pub(crate) fn is_trash_or_junk_folder(name: &str, path: &str) -> bool {
    let upper = path.to_ascii_uppercase();
    name == "已删除"
        || name == "垃圾邮件"
        || upper.contains("TRASH")
        || upper.contains("DELETED")
        || upper.contains("JUNK")
        || upper.contains("SPAM")
}

/// B-6：判定文件夹是否为已发送（语义对齐 cache.rs 的 SENT_FOLDER_MATCH）。
pub(crate) fn is_sent_folder(name: &str, path: &str) -> bool {
    name == "已发送" || path.to_ascii_uppercase().contains("SENT")
}

/// F-1：单键快捷键动作的共用守卫（纯函数，便于单测）。
///
/// 条件对齐 `DeleteSelectedMessage` 的既有守卫（Mail 阅读态 + 无浮层 +
/// 非文本输入聚焦），并额外要求单键开关开启。Ctrl 组合键动作不走此守卫
/// （不受单键偏好控制）。
fn single_key_guard_allows(
    active_modal: ActiveModal,
    main_view: MainContentView,
    right_mode: RightPaneMode,
    text_input_focused: bool,
    single_key_enabled: bool,
) -> bool {
    active_modal == ActiveModal::None
        && main_view == MainContentView::Mail
        && right_mode == RightPaneMode::Reader
        && !text_input_focused
        && single_key_enabled
}

/// B-2：演示模式合成 .eml 内容（信头 + 正文；非严格 RFC，仅用于演示导出链路）。
pub(crate) fn compose_demo_eml(detail: &MessageDetail) -> Vec<u8> {
    let eml = format!(
        "From: {}\r\nTo: {}\r\nSubject: {}\r\nDate: {}\r\nMessage-ID: {}\r\nContent-Type: text/plain; charset=UTF-8\r\n\r\n{}",
        detail.sender,
        detail.recipients,
        detail.subject,
        detail.date_display,
        detail.rfc_message_id,
        detail.body_text,
    );
    eml.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- WP-10-3 正文格式与语法插入 ----

    /// 有选区时包裹选区，无选区时用占位文案。
    #[test]
    fn compose_markup_wraps_selection_or_uses_placeholder() {
        assert_eq!(ComposeMarkup::Bold.wrap("重点"), "**重点**");
        assert_eq!(ComposeMarkup::Bold.wrap(""), "**粗体**");
        assert_eq!(ComposeMarkup::Italic.wrap("斜"), "*斜*");
        assert_eq!(ComposeMarkup::Strike.wrap("删"), "~~删~~");
        assert_eq!(ComposeMarkup::InlineCode.wrap("a=1"), "`a=1`");
        assert_eq!(ComposeMarkup::Link.wrap("官网"), "[官网](https://)");
        assert_eq!(ComposeMarkup::CodeBlock.wrap("x=1"), "```\nx=1\n```");
    }

    /// 列表 / 引用按行加前缀，空行原样保留；有序列表逐行递增编号。
    #[test]
    fn compose_markup_prefixes_lines() {
        assert_eq!(ComposeMarkup::BulletList.wrap("a\nb"), "- a\n- b");
        assert_eq!(
            ComposeMarkup::Quote.wrap("一句\n\n两句"),
            "> 一句\n\n> 两句"
        );
        assert_eq!(ComposeMarkup::NumberedList.wrap("甲\n乙"), "1. 甲\n2. 乙");
    }

    /// 源码与纯文本模式直接编辑正文，格式化工具只显示在富文本模式。
    #[test]
    fn only_rich_offers_the_toolbar() {
        assert!(ComposeFormat::Rich.has_format_toolbar());
        assert!(!ComposeFormat::Markdown.has_format_toolbar());
        assert!(!ComposeFormat::Html.has_format_toolbar());
        assert!(!ComposeFormat::Plain.has_format_toolbar());
    }

    /// 格式标识的存入 / 读回是可逆的，未知值退回默认格式。
    #[test]
    fn compose_format_round_trips_through_storage_string() {
        for format in ComposeFormat::ALL {
            assert_eq!(ComposeFormat::from_str_lossy(format.as_str()), format);
        }
        assert_eq!(ComposeFormat::from_str_lossy(""), ComposeFormat::Rich);
        assert_eq!(ComposeFormat::from_str_lossy("md"), ComposeFormat::Markdown);
    }

    // ---- WP-10-1 收件人胶囊 ----

    /// 裸地址、`Name <addr>` 与半成品显示名三种输入的解析结果。
    #[test]
    fn recipient_pill_parses_addresses_and_display_names() {
        let bare = RecipientPill::parse("marcus@studio.dev").expect("裸地址应解析成功");
        assert_eq!(bare.label, "marcus@studio.dev");
        assert_eq!(bare.address, "marcus@studio.dev");
        assert!(bare.valid);

        let named =
            RecipientPill::parse(" 张三 <zhang@exmail.qq.com> ").expect("带显示名应解析成功");
        assert_eq!(named.label, "张三");
        assert_eq!(named.address, "zhang@exmail.qq.com");
        assert!(named.valid);
        assert_eq!(named.to_field_text(), "张三 <zhang@exmail.qq.com>");

        // 只写显示名：成胶囊但标红（valid = false），提示用户补全地址。
        let half = RecipientPill::parse("李四").expect("显示名也应成胶囊");
        assert!(!half.valid);
        assert_eq!(half.label, "李四");
        assert_eq!(half.address, "李四");

        assert!(RecipientPill::parse("   ").is_none());
    }

    /// 邮箱格式校验的边界值。
    #[test]
    fn looks_like_address_rejects_malformed_input() {
        assert!(looks_like_address("a@b.co"));
        assert!(looks_like_address("first.last+tag@mail.example.com"));
        assert!(!looks_like_address(""));
        assert!(!looks_like_address("张三"));
        assert!(!looks_like_address("no-at-sign.com"));
        assert!(!looks_like_address("a@b")); // 域名无点
        assert!(!looks_like_address("a@.com")); // 域名以点开头
        assert!(!looks_like_address("a@b.")); // 域名以点结尾
        assert!(!looks_like_address("@b.com")); // 本地段为空
        assert!(!looks_like_address("a b@c.com")); // 含空格
        assert!(!looks_like_address("a@b@c.com")); // 多个 @ 时域名段仍可解析，但本地段含 @
    }

    /// WP-10-2：From 身份文本的三种形态（有名字、只有地址、名字为空）。
    #[test]
    fn format_identity_covers_name_and_address_combinations() {
        assert_eq!(
            format_identity("Marcus Chen", "marcus@studio.dev"),
            "Marcus Chen <marcus@studio.dev>"
        );
        assert_eq!(
            format_identity("", "marcus@studio.dev"),
            "marcus@studio.dev"
        );
        assert_eq!(
            format_identity("  Marcus  ", "  marcus@studio.dev  "),
            "Marcus <marcus@studio.dev>"
        );
        // 地址为空时退化为只显示名字（账号未配置完的中间态）。
        assert_eq!(format_identity("Marcus", ""), "Marcus");
    }

    /// WP-10-2：Reply-To 允许逗号分隔多个地址，非法项必须被识别出来。
    #[test]
    fn reply_to_parts_validate_like_recipients() {
        let valid = |raw: &str| -> bool {
            split_recipient_parts(raw)
                .into_iter()
                .filter(|p| !p.trim().is_empty())
                .all(|p| RecipientPill::parse(p.trim()).is_some_and(|pill| pill.valid))
        };
        assert!(valid("reply@example.com"));
        assert!(valid("Support <support@example.com>, ops@example.com"));
        assert!(!valid("not-an-address"));
        assert!(!valid("ok@example.com, broken@"));
    }

    /// 逗号 / 分号切分：`<>` 内的分隔符不算分隔符，全角符号同样识别。
    #[test]
    fn split_recipient_parts_respects_angle_brackets() {
        assert_eq!(
            split_recipient_parts("a@x.com, b@y.com"),
            vec!["a@x.com", " b@y.com"]
        );
        // `<>` 内的分隔符不切分（地址本身含逗号的极端情形）。
        assert_eq!(
            split_recipient_parts("Name <a,b@x.com>, k@y.com"),
            vec!["Name <a,b@x.com>", " k@y.com"]
        );
        // 尖括号外的逗号一律视为分隔符：未加引号的显示名 `Doe, John` 无法与
        // 分隔符区分（RFC 5322 要求用 `"Doe, John" <..>`），按分隔符处理。
        assert_eq!(
            split_recipient_parts("Doe, John <j@x.com>"),
            vec!["Doe", " John <j@x.com>"]
        );
        assert_eq!(
            split_recipient_parts("a@x.com；b@y.com，c@z.com"),
            vec!["a@x.com", "b@y.com", "c@z.com"]
        );
        // 末尾分隔符切出一个空段，由调用方按空白丢弃。
        assert_eq!(split_recipient_parts("a@x.com,"), vec!["a@x.com", ""]);
    }

    /// 胶囊「移到下一个字段」按 To -> Cc -> Bcc -> To 循环。
    #[test]
    fn compose_address_field_cycles_forward() {
        assert_eq!(ComposeAddressField::To.next(), ComposeAddressField::Cc);
        assert_eq!(ComposeAddressField::Cc.next(), ComposeAddressField::Bcc);
        assert_eq!(ComposeAddressField::Bcc.next(), ComposeAddressField::To);
        assert_eq!(ComposeAddressField::To.index(), 0);
        assert_eq!(ComposeAddressField::Bcc.index(), 2);
    }

    #[test]
    fn split_sender_parses_display_name_and_email() {
        let (name, email) = split_sender("Marcus Chen <marcus@studio.dev>");
        assert_eq!(name, "Marcus Chen");
        assert_eq!(email, "marcus@studio.dev");
        let (name, email) = split_sender("marcus@studio.dev");
        assert_eq!(name, "marcus@studio.dev");
        assert_eq!(email, "");
        let (name, email) = split_sender(" 张三 <zhang@exmail.qq.com> ");
        assert_eq!(name, "张三");
        assert_eq!(email, "zhang@exmail.qq.com");
    }

    #[test]
    fn unified_inbox_constant_stays_zero() {
        assert_eq!(UNIFIED_INBOX_FOLDER_ID, 0);
    }

    /// 阶段 3：last_view 字符串 ↔ 统一视图映射（未知值回落 None）。
    #[test]
    fn last_view_strings_map_back_to_views() {
        assert_eq!(
            HylkiApp::current_view_from_last(Some("unified")),
            Some(CurrentView::UnifiedInbox)
        );
        assert_eq!(
            HylkiApp::current_view_from_last(Some("unread")),
            Some(CurrentView::Unread)
        );
        assert_eq!(
            HylkiApp::current_view_from_last(Some("starred")),
            Some(CurrentView::Starred)
        );
        assert_eq!(
            HylkiApp::current_view_from_last(Some("sent")),
            Some(CurrentView::Sent)
        );
        assert_eq!(
            HylkiApp::current_view_from_last(Some("drafts")),
            Some(CurrentView::Drafts)
        );
        assert_eq!(
            HylkiApp::current_view_from_last(Some("archive")),
            Some(CurrentView::Archive)
        );
        assert_eq!(HylkiApp::current_view_from_last(Some("bogus")), None);
        assert_eq!(HylkiApp::current_view_from_last(None), None);
    }

    #[test]
    fn detect_new_unread_skips_initial_load_and_reports_subsequent_growth() {
        let mut baseline = BTreeMap::new();
        // 首次启动加载缓存或初始同步：只记录基线，不触发新邮件通知。
        assert_eq!(detect_new_unread(&mut baseline, 1, 10), 0);
        // 未读数不变：不触发通知。
        assert_eq!(detect_new_unread(&mut baseline, 1, 10), 0);
        // 后台同步新增 3 封未读：返回增量 3。
        assert_eq!(detect_new_unread(&mut baseline, 1, 13), 3);
        // 用户阅读后未读下降：更新基线且不触发通知。
        assert_eq!(detect_new_unread(&mut baseline, 1, 8), 0);
        // 再次收到 1 封新邮件：基于最新基线 8 返回 1。
        assert_eq!(detect_new_unread(&mut baseline, 1, 9), 1);
    }

    /// 任务四：统一视图历史可用性只统计 `enabled && in_unified` 账号的 INBOX，
    /// 与列表、未读数、分页拉取同范围（对齐 worker 端 unified_account_ids）。
    #[test]
    fn unified_view_has_more_remote_filters_account_scope() {
        let acc_a = AccountConfig {
            id: 1,
            email: "a@qq.com".into(),
            ..Default::default()
        };
        let mut acc_b = AccountConfig {
            id: 2,
            email: "b@qq.com".into(),
            ..Default::default()
        };
        acc_b.in_unified = false;
        let mut acc_c = AccountConfig {
            id: 3,
            email: "c@qq.com".into(),
            ..Default::default()
        };
        acc_c.enabled = false;
        let accounts = vec![acc_a.clone(), acc_b.clone(), acc_c];

        let folder = |id: u32, account_id: u32, more: bool| FolderSummary {
            id,
            account_id,
            name: "收件箱".into(),
            path: "INBOX".into(),
            unread_count: 0,
            total_count: 10,
            has_more_remote: more,
        };
        let mut folders_by_account = BTreeMap::new();
        folders_by_account.insert(1u32, vec![folder(11, 1, false)]);
        folders_by_account.insert(2u32, vec![folder(22, 2, true)]);
        folders_by_account.insert(3u32, vec![folder(33, 3, true)]);

        // 退出统一视图（B）与停用（C）账号的历史不计入统一视图可用性。
        assert!(!unified_view_has_more_remote(
            &accounts,
            &folders_by_account
        ));
        assert!(!account_in_unified_scope(&accounts, 2));
        assert!(!account_in_unified_scope(&accounts, 3));

        folders_by_account.insert(1u32, vec![folder(11, 1, true)]);
        assert!(unified_view_has_more_remote(&accounts, &folders_by_account));

        // 未知账号（本地列表中不存在）按范围外处理。
        assert!(!account_in_unified_scope(&accounts, 99));
    }

    /// 任务四：批次完成事件驱动的排队节制——同一索引任务最多一个在途批次，
    /// 批次按完成事件逐个排队（事件驱动而非循环连发）。
    #[test]
    fn backfill_drive_state_queues_one_batch_at_a_time() {
        let mut state = BackfillDriveState::default();
        let mut queued = 0usize;
        let drive = |state: &mut BackfillDriveState, queued: &mut usize| {
            if state.should_queue() {
                state.on_batch_queued();
                *queued += 1;
            }
        };

        // 首屏就绪：排第 1 批；在途期间任何再次驱动（FoldersLoaded / 页面刷新）都不排队。
        drive(&mut state, &mut queued);
        drive(&mut state, &mut queued);
        assert_eq!(queued, 1);

        // 批次完成（仍有候选）：清除在途并续排下一批——每个完成事件至多排队一次，
        // 单队列顺序消费保证前台读写信、搜索请求可在批次之间插队。
        state.on_batch_done(true);
        drive(&mut state, &mut queued);
        assert_eq!(queued, 2);
        assert!(state.in_flight);
        drive(&mut state, &mut queued);
        assert_eq!(queued, 2);

        // worker 回无待索引目标：本轮结束，空闲也不再排队。
        state.on_batch_done(false);
        assert!(!state.should_queue());
        drive(&mut state, &mut queued);
        assert_eq!(queued, 2);

        // 账号范围变化（保存 / 启停 / 新文件夹）重新允许探测。
        state.on_scope_refresh();
        drive(&mut state, &mut queued);
        assert_eq!(queued, 3);
        assert!(state.in_flight);
    }

    #[test]
    fn capture_window_geometry_updates_windowed_and_maximized_states() {
        use gpui_kit::{size, Bounds};
        let mut prefs = AppPreferences::default();

        let b1 = Bounds::new(point(px(10.0), px(20.0)), size(px(1400.0), px(860.0)));
        assert!(capture_window_geometry(
            &mut prefs,
            WindowBounds::Windowed(b1)
        ));
        assert_eq!(prefs.window_width, Some(1400.0));
        assert_eq!(prefs.window_height, Some(860.0));
        assert!(!prefs.window_maximized);

        // 相同尺寸再次保存应返回 false。
        assert!(!capture_window_geometry(
            &mut prefs,
            WindowBounds::Windowed(b1)
        ));

        // 最大化状态：更新 window_maximized 且保留 restore bounds。
        assert!(capture_window_geometry(
            &mut prefs,
            WindowBounds::Maximized(b1)
        ));
        assert!(prefs.window_maximized);
        assert_eq!(prefs.window_width, Some(1400.0));
        assert_eq!(prefs.window_height, Some(860.0));

        // 最小化退化尺寸 (0x0) 不应覆盖已记录的有效宽高。
        let degenerate = Bounds::new(point(px(0.0), px(0.0)), size(px(0.0), px(0.0)));
        assert!(!capture_window_geometry(
            &mut prefs,
            WindowBounds::Maximized(degenerate)
        ));
        assert_eq!(prefs.window_width, Some(1400.0));
        assert_eq!(prefs.window_height, Some(860.0));
    }

    #[test]
    fn compute_default_collapsed_cards_keeps_newest_and_current_expanded() {
        let make_tm = |id: u32, unread: bool| hylki_mail_core::ThreadMessage {
            id,
            rfc_message_id: format!("<{id}@test>"),
            subject: "Test".to_string(),
            sender: "a@test".to_string(),
            recipients: "b@test".to_string(),
            cc: String::new(),
            date_display: "10:00".to_string(),
            date_ts: 0,
            unread,
            body_text: "body".to_string(),
            body_html: None,
            body_loaded: true,
            flagged: false,
            attachments: Vec::new(),
            uid: id,
            folder_path: "INBOX".to_string(),
        };
        let detail = MessageDetail {
            id: 2,
            account_id: 1,
            folder_id: 1,
            folder_path: "INBOX".to_string(),
            uid: 2,
            rfc_message_id: "<2@test>".to_string(),
            sender: "a@test".to_string(),
            recipients: "b@test".to_string(),
            cc: String::new(),
            subject: "Test".to_string(),
            date_display: "10:00".to_string(),
            date_ts: 0,
            body_text: "body".to_string(),
            body_html: None,
            flagged: false,
            attachments: Vec::new(),
            // 降序：4 最新，3 未读，2 当前，1 已读最早。
            thread_messages: vec![
                make_tm(4, false),
                make_tm(3, true),
                make_tm(2, false),
                make_tm(1, false),
            ],
            unsubscribe_url: None,
            unsubscribe_one_click: false,
            auth_results: None,
        };

        let folded_all = compute_default_collapsed_cards(&detail, true, "all_but_newest");
        assert!(folded_all.contains(&1));
        assert!(folded_all.contains(&3));
        assert!(!folded_all.contains(&2));
        assert!(!folded_all.contains(&4));

        // 仅折叠已读模式应保留未读邮件 #3 展开。
        let folded_read = compute_default_collapsed_cards(&detail, true, "read");
        assert!(folded_read.contains(&1));
        assert!(!folded_read.contains(&3));
        assert!(!folded_read.contains(&2));
        assert!(!folded_read.contains(&4));

        let unfolded = compute_default_collapsed_cards(&detail, true, "never");
        assert!(unfolded.is_empty());
    }

    #[test]
    fn append_signature_inserts_standard_separator() {
        assert_eq!(append_signature("hello", ""), "hello");
        assert_eq!(append_signature("hello", "   \n "), "hello");
        assert_eq!(
            append_signature("", "张三\n研发部"),
            "\n\n-- \n张三\n研发部"
        );
        assert_eq!(
            append_signature("\n\n--- 原始邮件 ---\n正文\n", "Best,\nAlice  "),
            "\n\n--- 原始邮件 ---\n正文\n\n-- \nBest,\nAlice"
        );
        assert_eq!(
            append_signature_with_dashes("", "Alice", false),
            "\n\nAlice"
        );
        let above = insert_signature_with_quote("\n\n--- 引用 ---\n原文", "Alice", true, true);
        assert!(above.starts_with("\n\n-- \nAlice\n\n--- 引用 ---"));
        let below = insert_signature_with_quote("\n\n--- 引用 ---\n原文", "Alice", false, false);
        assert!(below.ends_with("--- 引用 ---\n原文\n\nAlice"));
    }

    #[test]
    fn has_unsaved_compose_content_detects_any_non_empty_field_or_attachment() {
        assert!(!has_unsaved_compose_content("", " ", "", "\n", "  ", 0));
        assert!(has_unsaved_compose_content("a@b.com", "", "", "", "", 0));
        assert!(has_unsaved_compose_content("", "", "", "主题", "", 0));
        assert!(has_unsaved_compose_content("", "", "", "", "正文", 0));
        assert!(has_unsaved_compose_content("", "", "", "", "", 1));
    }

    #[test]
    fn contact_autocomplete_filters_limits_dedupes_and_applies() {
        let contacts = vec![
            KnownContact {
                name: "林晓明".to_string(),
                email: "xiaoming@example.com".to_string(),
                message_count: 10,
                ..KnownContact::default()
            },
            KnownContact {
                name: "陈思远".to_string(),
                email: "siyuan@example.org".to_string(),
                message_count: 8,
                ..KnownContact::default()
            },
            KnownContact {
                name: String::new(),
                email: "alice@example.com".to_string(),
                message_count: 6,
                ..KnownContact::default()
            },
            KnownContact {
                name: "Bob".to_string(),
                email: "bob@example.com".to_string(),
                message_count: 5,
                ..KnownContact::default()
            },
            KnownContact {
                name: "Carol".to_string(),
                email: "carol@example.com".to_string(),
                message_count: 4,
                ..KnownContact::default()
            },
            KnownContact {
                name: "Dave".to_string(),
                email: "dave@example.com".to_string(),
                message_count: 3,
                ..KnownContact::default()
            },
            KnownContact {
                name: "Eve".to_string(),
                email: "eve@example.com".to_string(),
                message_count: 2,
                ..KnownContact::default()
            },
        ];

        // 空输入不弹建议
        assert!(filter_contact_suggestions("", &contacts, 5).is_empty());
        assert!(filter_contact_suggestions("xiaoming@example.com, ", &contacts, 5).is_empty());

        // 最多返回 5 条
        let many = filter_contact_suggestions("example", &contacts, 5);
        assert_eq!(many.len(), 5);

        // 排除前面已经填过的联系人
        let after_first =
            filter_contact_suggestions("林晓明 <xiaoming@example.com>, xiao", &contacts, 5);
        assert!(after_first.is_empty());

        // 中文姓名匹配 + 补全追加逗号
        let matched = filter_contact_suggestions("思远", &contacts, 5);
        assert_eq!(matched.len(), 1);
        assert_eq!(
            apply_contact_completion("林晓明 <xiaoming@example.com>, 思远", matched[0]),
            "林晓明 <xiaoming@example.com>, 陈思远 <siyuan@example.org>, "
        );

        // 无姓名联系人直接补全邮箱并追加逗号
        assert_eq!(
            apply_contact_completion("ali", &contacts[2]),
            "alice@example.com, "
        );
    }

    #[test]
    fn next_adjacent_index_respects_bounds() {
        assert_eq!(next_adjacent_index(None, 0, 1), None);
        assert_eq!(next_adjacent_index(None, 3, 1), Some(0));
        assert_eq!(next_adjacent_index(None, 3, -1), Some(2));
        assert_eq!(next_adjacent_index(Some(0), 3, 1), Some(1));
        assert_eq!(next_adjacent_index(Some(0), 3, -1), None);
        assert_eq!(next_adjacent_index(Some(2), 3, 1), None);
        assert_eq!(next_adjacent_index(Some(2), 3, -1), Some(1));
    }

    /// F-2：删除 / 归档后自动推进的目标下标（优先后一封，末尾则上一封，单封不推进）。
    #[test]
    fn next_index_after_removal_advances_to_neighbor() {
        assert_eq!(next_index_after_removal(0, 0), None);
        assert_eq!(next_index_after_removal(0, 1), None);
        assert_eq!(next_index_after_removal(0, 2), Some(1));
        assert_eq!(next_index_after_removal(1, 2), Some(0));
        assert_eq!(next_index_after_removal(2, 5), Some(3));
        assert_eq!(next_index_after_removal(4, 5), Some(3));
    }

    /// F-1：单键动作共用守卫（Mail 阅读态 + 无浮层 + 非文本输入 + 偏好开关）。
    #[test]
    fn single_key_guard_requires_mail_reader_no_modal_and_pref() {
        let ok = |text_input_focused, single_key_enabled| {
            single_key_guard_allows(
                ActiveModal::None,
                MainContentView::Mail,
                RightPaneMode::Reader,
                text_input_focused,
                single_key_enabled,
            )
        };
        // 成功路径：阅读态、无浮层、无输入焦点、开关开启。
        assert!(ok(false, true));
        // 关闭单键偏好后不生效。
        assert!(!ok(false, false));
        // 文本输入聚焦（搜索框 / 写信输入）时不生效。
        assert!(!ok(true, true));
        // 浮层打开 / 非阅读态 / 非邮件视图均不生效。
        assert!(!single_key_guard_allows(
            ActiveModal::Shortcuts,
            MainContentView::Mail,
            RightPaneMode::Reader,
            false,
            true
        ));
        assert!(!single_key_guard_allows(
            ActiveModal::None,
            MainContentView::Mail,
            RightPaneMode::Compose,
            false,
            true
        ));
        assert!(!single_key_guard_allows(
            ActiveModal::None,
            MainContentView::Contacts,
            RightPaneMode::Reader,
            false,
            true
        ));
    }

    /// B-2：Trash / Junk 文件夹判定（对齐 cache.rs 匹配规则）。
    #[test]
    fn trash_or_junk_folder_detection_matches_cache_rules() {
        assert!(is_trash_or_junk_folder("已删除", "INBOX/Trash"));
        assert!(is_trash_or_junk_folder("", "Deleted Messages"));
        assert!(is_trash_or_junk_folder("垃圾邮件", "Junk"));
        assert!(is_trash_or_junk_folder("", "spam"));
        // 普通文件夹不误判。
        assert!(!is_trash_or_junk_folder("收件箱", "INBOX"));
        assert!(!is_trash_or_junk_folder("已发送", "Sent"));
        assert!(!is_trash_or_junk_folder("归档", "Archive"));
    }

    /// L-6：阅读器位置枚举与偏好字符串互转（未知值返回 None，加载侧回落 Right）。
    #[test]
    fn reader_position_key_roundtrip() {
        assert_eq!(ReaderPosition::default(), ReaderPosition::Right);
        for (key, expected) in [
            ("right", ReaderPosition::Right),
            ("below", ReaderPosition::Below),
            ("off", ReaderPosition::Hidden),
        ] {
            assert_eq!(ReaderPosition::from_key(key), Some(expected));
            assert_eq!(expected.key(), key);
        }
        // 未知 / 旧值不解析成功（加载端 unwrap_or_default 回落 Right）。
        assert_eq!(ReaderPosition::from_key("under"), None);
        assert_eq!(ReaderPosition::from_key(""), None);
        assert_eq!(ReaderPosition::from_key("true"), None);
    }

    /// L-2：单行表格模式迟滞阈值（进入 ≥600，退出 <560，区间内保持原状态）。
    #[test]
    fn list_wide_hysteresis_matches_original_thresholds() {
        // 未进入宽模式：≥600 才进入。
        assert!(!list_wide_next(false, 559.0));
        assert!(!list_wide_next(false, 599.0));
        assert!(list_wide_next(false, 600.0));
        assert!(list_wide_next(false, 900.0));
        // 已进入宽模式：<560 才退出，560~599 区间保持。
        assert!(list_wide_next(true, 600.0));
        assert!(list_wide_next(true, 560.0));
        assert!(list_wide_next(true, 599.0));
        assert!(!list_wide_next(true, 559.0));
    }

    /// P3：半屏自动窄轨断点（对齐 Linux adw::Breakpoint MaxWidth 1094px：
    /// ≤1094 进入窄轨，>1094 还原宽栏，无迟滞）。
    #[test]
    fn auto_rail_breakpoint_matches_linux_semantics() {
        // 半屏 1920/2 与验收边界：1094 及以下进入窄轨。
        assert!(auto_rail_next(960.0));
        assert!(auto_rail_next(1093.0));
        assert!(auto_rail_next(1094.0));
        // 1095 起恢复宽栏。
        assert!(!auto_rail_next(1095.0));
        assert!(!auto_rail_next(1280.0));
        assert!(!auto_rail_next(1440.0));
    }

    /// P3：浮层侧栏宽度夹取（沿用手动宽度偏好，上限再夹到窄轨右侧剩余空间）。
    #[test]
    fn peek_sidebar_width_clamps_to_preference_and_viewport() {
        // 常规半屏窗口：夹取到偏好宽度区间 180~280。
        assert_eq!(peek_sidebar_width(240.0, 1094.0), 240.0);
        assert_eq!(peek_sidebar_width(120.0, 1094.0), 180.0);
        assert_eq!(peek_sidebar_width(400.0, 1094.0), 280.0);
        // 极窄窗口：不超过窄轨右侧剩余空间，且不为负。
        assert_eq!(peek_sidebar_width(280.0, 700.0), 280.0);
        assert_eq!(peek_sidebar_width(280.0, 200.0), 120.0);
        assert_eq!(peek_sidebar_width(280.0, 60.0), 0.0);
    }

    /// L-2：布局常量自洽（侧栏范围、列表 / 阅读器最小值与顶栏收纳阈值关系）。
    #[test]
    fn layout_constants_are_self_consistent() {
        assert!(SIDEBAR_MIN_WIDTH < sidebar::SIDEBAR_WIDTH);
        assert!(sidebar::SIDEBAR_WIDTH < SIDEBAR_MAX_WIDTH);
        assert!(LIST_MIN_WIDTH < message_list::LIST_WIDTH);
        assert!(LIST_WIDE_OFF < LIST_WIDE_ON);
        // 顶栏收纳阈值语义：完整平铺阅读器顶栏至少需要 侧栏上限 + 列表下限 + 阅读器
        // 最小宽度（280+280+441=1001px），阈值 880px 落在其下——窗口低于阈值时右栏
        // 必然已被压缩，低频动作应收进 ⋯ 菜单。
        assert!(SIDEBAR_MAX_WIDTH + LIST_MIN_WIDTH + READER_MIN_WIDTH > READER_OVERFLOW_BREAKPOINT);
        // 阈值本身仍大于「侧栏下限 + 列表下限」，保证折叠时右栏保有可用空间。
        assert!(READER_OVERFLOW_BREAKPOINT > SIDEBAR_MIN_WIDTH + LIST_MIN_WIDTH);
        assert!(BELOW_LIST_MIN_HEIGHT >= BELOW_PANE_MIN_HEIGHT);
    }

    /// B-6：已发送文件夹判定（对齐 cache.rs SENT_FOLDER_MATCH）。
    #[test]
    fn sent_folder_detection_matches_cache_rules() {
        assert!(is_sent_folder("已发送", "INBOX/Sent"));
        assert!(is_sent_folder("", "Sent Messages"));
        assert!(is_sent_folder("", "sentitems"));
        assert!(!is_sent_folder("收件箱", "INBOX"));
        assert!(!is_sent_folder("垃圾邮件", "Junk"));
    }

    /// B-2：演示模式合成 .eml 包含基本信头与正文。
    #[test]
    fn compose_demo_eml_includes_headers_and_body() {
        let detail = MessageDetail {
            id: 7,
            account_id: 1,
            folder_id: 1,
            folder_path: "INBOX".to_string(),
            uid: 7,
            rfc_message_id: "<demo-7@hylki.example>".to_string(),
            sender: "林晓明 <xiaoming@example.com>".to_string(),
            recipients: "demo@hylki.example".to_string(),
            cc: "陈思远 <siyuan@example.org>".to_string(),
            subject: "周报".to_string(),
            date_display: "11-21 09:30".to_string(),
            date_ts: 0,
            body_text: "正文第一行".to_string(),
            body_html: None,
            flagged: false,
            attachments: Vec::new(),
            thread_messages: Vec::new(),
            unsubscribe_url: None,
            unsubscribe_one_click: false,
            auth_results: None,
        };
        let bytes = compose_demo_eml(&detail);
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.starts_with("From: 林晓明 <xiaoming@example.com>\r\n"));
        assert!(text.contains("To: demo@hylki.example\r\n"));
        assert!(text.contains("Subject: 周报\r\n"));
        assert!(text.contains("Message-ID: <demo-7@hylki.example>\r\n"));
        assert!(text.contains("\r\n\r\n正文第一行"));
    }

    #[test]
    fn remote_images_allowed_respects_whitelist_and_sender_trust() {
        let mut prefs = hylki_mail_core::config::AppPreferences::default();
        prefs.block_remote_images = true;
        prefs.allow_remote_sender("trusted@example.com");

        // 验证发件人白名单解析与放行
        assert!(prefs.is_remote_sender_allowed("trusted@example.com"));
        assert!(prefs.is_remote_sender_allowed("Trusted User <trusted@example.com>"));
        assert!(!prefs.is_remote_sender_allowed("untrusted@example.com"));
    }

    #[test]
    fn compose_session_and_split_preferences_respect_detached_and_reply_inline() {
        assert!(compose_session_active(RightPaneMode::Compose, false));
        assert!(compose_session_active(RightPaneMode::Reader, true));
        assert!(!compose_session_active(RightPaneMode::Reader, false));
        assert!(!compose_session_active(RightPaneMode::Welcome, false));

        // 新建邮件关闭内联（compose_inline=false）但回复内联开启（reply_inline=true）时仍应启用分屏。
        assert!(should_use_compose_split(true, true, true));
        assert!(!should_use_compose_split(true, false, true));
        assert!(!should_use_compose_split(false, true, true));
        assert!(!should_use_compose_split(true, true, false));
    }

    #[test]
    fn popout_readiness_and_main_reader_update_rules_work() {
        // 仅当不是占位详情、未在加载中且 ID 匹配时才允许直接用 selected_message 弹窗。
        assert!(is_selected_message_ready_for_popout(
            Some(42),
            42,
            None,
            false
        ));
        assert!(!is_selected_message_ready_for_popout(
            Some(42),
            42,
            None,
            true
        ));
        assert!(!is_selected_message_ready_for_popout(
            Some(42),
            42,
            Some(42),
            false
        ));
        assert!(!is_selected_message_ready_for_popout(
            Some(10),
            42,
            None,
            false
        ));
        assert!(!is_selected_message_ready_for_popout(None, 42, None, false));

        // 右键独立打开其他邮件：不改变主窗口当前邮件。
        assert!(!should_update_main_reader_on_body_loaded(
            20,
            None,
            true,
            Some(10)
        ));
        // 主窗口正在加载 10 时，独立弹出 20 返回不应更新主窗口。
        assert!(!should_update_main_reader_on_body_loaded(
            20,
            Some(10),
            true,
            Some(10)
        ));
        // 双击打开 10 且主窗口也在等待 10：主窗口同步更新。
        assert!(should_update_main_reader_on_body_loaded(
            10,
            Some(10),
            true,
            Some(10)
        ));
        // 主窗口无在途选中请求时，会话内卡片手动「加载完整正文」仍正常更新主窗口。
        assert!(should_update_main_reader_on_body_loaded(
            11,
            None,
            false,
            Some(10)
        ));
    }

    #[test]
    fn popout_cascade_offset_wraps_every_eight_windows() {
        assert_eq!(popout_cascade_offset(0), 0.0);
        assert_eq!(popout_cascade_offset(1), 28.0);
        assert_eq!(popout_cascade_offset(7), 196.0);
        assert_eq!(popout_cascade_offset(8), 0.0);
    }
}
