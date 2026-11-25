//! 左栏（方案文档第三节 1.1）：
//! - 顶部栏 48px：刷新按钮 + 居中 `Hylki` 粗体标题 + 菜单按钮
//!   （账号管理 / 显示隐藏账号列表 / 全屏 / 切换亮暗主题 / 偏好设置 /
//!   命令面板 Ctrl+K / 键盘快捷键 F1 / 关于，分组顺序对齐原版主菜单）。
//! - 「写邮件」分割按钮：全宽蓝色主按钮（38px 高、圆角 8px、accent 色），
//!   存在草稿模板时右侧紧贴 28px 下拉小箭头（从模板新建；模板数据源在
//!   Windows 端尚未落地，见 [`compose_templates`]）。
//! - 统一导航区（34px 高、圆角 8、左图标 + 名称 + 右侧未读数胶囊）：
//!   收件箱 / 已加星标 / 已发送 / 草稿箱 / 归档 / 发件箱待处理。
//!   WP-07：≥2 个启用账号时，收件箱 / 已发送 / 草稿箱 / 归档行左侧显示
//!   旋转小箭头，点击展开各账号对应文件夹的子行（账号彩色圆点 + 未读数）。
//! - 收藏分区（WP-07 Favorites）：收藏的文件夹列表，右键「收藏」添加。
//! - 账号折叠分组：彩色头像（角上未读小蓝圈）+ 账号名 + 邮箱小字 +
//!   折叠箭头（刷新 / 设置 / 新建文件夹 / 移除账号等操作收口至账号右键菜单，
//!   对齐 Linux 版）；展开先列标准角色文件夹，随后直接平级列出自定义文件夹
//!   并按 IMAP 分隔符构建多级树（WP-07），每行带完整右键菜单。
//! - 底部固定区：联系人、附件入口。

use std::collections::HashSet;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{
    ContextMenu, ContextMenuExt, DropdownMenu, PopupMenu, PopupMenuItem,
};
use gpui_kit::component::resizable::{resize_handle, HandleEdge};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::Icon;
use gpui_kit::component::{h_flex, v_flex, Disableable, Selectable, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    div, px, App, AppContext, Axis, Context, Div, Entity, Global, InteractiveElement, IntoElement,
    ParentElement, SharedString, Stateful, StatefulInteractiveElement, Styled, Window,
    WindowControlArea,
};
use hylki_mail_core::config::{
    self, AccountConfig, AuthBackend, SidebarFolderRef, SidebarFolderRole,
};
use hylki_mail_core::{FolderSummary, OutboxItem, OutboxStatus};

use crate::app::{
    CurrentView, FolderActionPrompt, HylkiApp, MainContentView, ReaderPosition, SidebarDrag,
};
use crate::theme;
use crate::ui::avatar;

/// 左栏默认宽度（方案文档：230 ~ 250px；L-2 起可拖拽调整为 180~280，持久化到偏好）。
pub(crate) const SIDEBAR_WIDTH: f32 = 240.0;
/// 顶部栏高度。
const HEADER_HEIGHT: f32 = 48.0;
/// 导航行高度。
const NAV_ROW_HEIGHT: f32 = 34.0;

/// 未读 / 计数胶囊（柔和区分普通与告警状态）。
fn count_pill(
    n: u32,
    selected: bool,
    danger: bool,
    palette: &theme::HylkiPalette,
) -> gpui_kit::Div {
    div()
        .rounded_full()
        .px_1p5()
        .min_w(px(20.0))
        .h(px(18.0))
        .flex()
        .items_center()
        .justify_center()
        .text_xs()
        .font_weight(gpui_kit::FontWeight::BOLD)
        .child(SharedString::from(if n > 999 {
            "999+".to_string()
        } else {
            n.to_string()
        }))
        .when(danger, |pill| {
            pill.bg(palette.danger).text_color(palette.accent_fg)
        })
        .when(!danger && selected, |pill| {
            pill.bg(palette.accent).text_color(palette.accent_fg)
        })
        .when(!danger && !selected, |pill| {
            pill.bg(palette.accent).text_color(palette.accent_fg)
        })
}

/// 根据文件夹路径或中文名挑选语义图标。
fn folder_icon(path: &str, name: &str) -> IconName {
    let p = path.to_ascii_uppercase();
    if p == "INBOX" || name.contains("收件箱") {
        IconName::Inbox
    } else if p.contains("SENT") || name.contains("已发送") {
        IconName::Send
    } else if p.contains("DRAFT") || name.contains("草稿") {
        IconName::FilePen
    } else if p.contains("ARCHIVE") || name.contains("归档") {
        IconName::Archive
    } else if p.contains("TRASH") || p.contains("DELETED") || name.contains("已删除") {
        IconName::Trash
    } else if p.contains("JUNK") || p.contains("SPAM") || name.contains("垃圾") {
        IconName::TriangleAlert
    } else {
        IconName::Folder
    }
}

// ---------------------------------------------------------------------------
// WP-07：文件夹角色分类与多级树纯逻辑（对齐 Linux `src/ui/sidebar/tree.rs`
// 的语义：按已列出祖先计算父子，分隔符严格后代判定）。
// ---------------------------------------------------------------------------

/// 标准角色文件夹（含用户右键「指定特殊角色」指派的角色）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FolderRole {
    Inbox,
    Drafts,
    Sent,
    Archive,
    Junk,
    Trash,
}

impl FolderRole {
    /// 偏好持久化的角色键（`sidebar_special_roles.role`）。
    fn from_key(key: &str) -> Option<Self> {
        match key {
            "inbox" => Some(Self::Inbox),
            "drafts" => Some(Self::Drafts),
            "sent" => Some(Self::Sent),
            "archive" => Some(Self::Archive),
            "junk" => Some(Self::Junk),
            "trash" => Some(Self::Trash),
            _ => None,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Drafts => "drafts",
            Self::Sent => "sent",
            Self::Archive => "archive",
            Self::Junk => "junk",
            Self::Trash => "trash",
        }
    }

    /// 右键「指定特殊角色」子菜单的展示名。
    fn label(self) -> &'static str {
        match self {
            Self::Inbox => "收件箱",
            Self::Drafts => "草稿箱",
            Self::Sent => "已发送",
            Self::Archive => "归档",
            Self::Junk => "垃圾邮件",
            Self::Trash => "已删除",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::Inbox => IconName::Inbox,
            Self::Drafts => IconName::FilePen,
            Self::Sent => IconName::Send,
            Self::Archive => IconName::Archive,
            Self::Junk => IconName::TriangleAlert,
            Self::Trash => IconName::Trash,
        }
    }
}

/// 文件夹角色分类：先看用户「指定特殊角色」（偏好持久化，覆盖路径推断），
/// 再按 IMAP 路径 / 显示名推断（匹配规则与 [`folder_icon`] 一致）。
fn classified_role(
    account_id: u32,
    path: &str,
    name: &str,
    special_roles: &[SidebarFolderRole],
) -> Option<FolderRole> {
    if let Some(role) = special_roles
        .iter()
        .find(|r| r.account_id == account_id && r.path == path)
        .and_then(|r| FolderRole::from_key(&r.role))
    {
        return Some(role);
    }
    intrinsic_role(path, name)
}

/// 按 IMAP 路径 / 显示名推断标准角色（未指定特殊角色时使用）。
fn intrinsic_role(path: &str, name: &str) -> Option<FolderRole> {
    let p = path.to_ascii_uppercase();
    if p == "INBOX" || name.contains("收件箱") {
        Some(FolderRole::Inbox)
    } else if p.contains("DRAFT") || name.contains("草稿") {
        Some(FolderRole::Drafts)
    } else if p.contains("SENT") || name.contains("已发送") {
        Some(FolderRole::Sent)
    } else if p.contains("ARCHIVE") || name.contains("归档") {
        Some(FolderRole::Archive)
    } else if p.contains("TRASH") || p.contains("DELETED") || name.contains("已删除") {
        Some(FolderRole::Trash)
    } else if p.contains("JUNK") || p.contains("SPAM") || name.contains("垃圾") {
        Some(FolderRole::Junk)
    } else {
        None
    }
}

/// 推断 IMAP 层级分隔符：任一路径含 `/` 优先取 `/`，其次 `.`（Dovecot 风格
/// `INBOX.Sub`），都没有则视为平铺。
fn detect_delimiter(paths: &[&str]) -> Option<char> {
    if paths.iter().any(|p| p.contains('/')) {
        Some('/')
    } else if paths.iter().any(|p| p.contains('.')) {
        Some('.')
    } else {
        None
    }
}

/// `child` 是否为 `parent` 在服务器分隔符下的严格后代（对齐 Linux
/// `path_is_under`：按字节比较分隔符边界，`v1.2` 在 `/` 服务器上不是
/// `v1` 的子级）。
fn path_is_under(child: &str, parent: &str, delim: char) -> bool {
    let (c, p) = (child.as_bytes(), parent.as_bytes());
    c.len() > p.len() && c[p.len()] == delim as u8 && c[..p.len()].eq_ignore_ascii_case(p)
}

/// 自定义文件夹的最近已列出祖先（对齐 Linux `custom_parent`：父子关系按
/// 「已列出文件夹」计算，保持命名空间前缀诚实——`INBOX.Clients` 在未列出
/// `INBOX` 自定义项的账号上是顶层文件夹）。
fn custom_tree_parent(path: &str, listed: &[&str], delim: Option<char>) -> Option<String> {
    let d = delim?;
    listed
        .iter()
        .filter(|g| **g != path && path_is_under(path, g, d))
        .max_by_key(|g| g.len())
        .map(|g| (*g).to_string())
}

/// 树深度：其他已列出文件夹中多少个是其祖先（对齐 Linux `folder_depth`）。
fn folder_depth(path: &str, listed: &[&str], delim: Option<char>) -> usize {
    let Some(d) = delim else { return 0 };
    listed
        .iter()
        .filter(|g| **g != path && path_is_under(path, g, d))
        .count()
}

/// 是否存在已列出的子文件夹（决定渲染折叠箭头）。
fn has_listed_children(path: &str, listed: &[&str], delim: Option<char>) -> bool {
    let Some(d) = delim else { return false };
    listed
        .iter()
        .any(|g| *g != path && path_is_under(g, path, d))
}

/// 自定义文件夹的树序（深度优先；同级按叶子显示名排序，对齐 Linux 默认
/// NameAsc）。父节点未列出的文件夹按名次序原地保留。
fn order_custom_tree(customs: &[&str], delim: Option<char>) -> Vec<String> {
    let leaf = |p: &str| -> String {
        match delim {
            Some(d) if p.contains(d) => p.rsplit(d).next().unwrap_or(p).to_string(),
            _ => p.to_string(),
        }
    };
    let mut sorted: Vec<&str> = customs.to_vec();
    sorted.sort_by(|a, b| {
        leaf(a)
            .to_lowercase()
            .cmp(&leaf(b).to_lowercase())
            .then_with(|| (*a).cmp(*b))
    });
    let mut emitted: Vec<String> = Vec::with_capacity(sorted.len());
    let mut remaining: Vec<&str> = sorted;
    while !remaining.is_empty() {
        // 依次取「父节点已输出（或无父）」的最靠前项，得到深度优先顺序。
        let at = remaining
            .iter()
            .position(|p| match custom_tree_parent(p, customs, delim) {
                None => true,
                Some(parent) => emitted.iter().any(|e| e == &parent),
            });
        match at {
            Some(i) => emitted.push(remaining.remove(i).to_string()),
            // 父链成环（不应发生）时兜底按名次序追加，保证不丢文件夹。
            None => emitted.extend(remaining.drain(..).map(String::from)),
        }
    }
    emitted
}

/// 账号文件夹分区：标准角色文件夹（含用户指定角色）在前，其余为自定义
/// 文件夹；两组均保持服务端列出顺序。`hidden`（账号 + 路径）中的文件夹整行
/// 剔除（其子级在树计算时重新挂到最近可见祖先下，对齐 Linux 隐藏语义）。
fn partition_folders<'a>(
    account_id: u32,
    folders: &'a [FolderSummary],
    special_roles: &[SidebarFolderRole],
    hidden: &[SidebarFolderRef],
) -> (Vec<(&'a FolderSummary, FolderRole)>, Vec<&'a FolderSummary>) {
    let is_hidden = |path: &str| {
        hidden
            .iter()
            .any(|h| h.account_id == account_id && h.path == path)
    };
    let mut roles = Vec::new();
    let mut customs = Vec::new();
    for f in folders {
        if is_hidden(&f.path) {
            continue;
        }
        match classified_role(account_id, &f.path, &f.name, special_roles) {
            Some(role) => roles.push((f, role)),
            None => customs.push(f),
        }
    }
    (roles, customs)
}

/// 统一视图子行对应的账号角色文件夹（该账号没有对应文件夹时不出现在子行中）。
/// 已加星标（Starred）无对应服务器文件夹且 Windows 端暂无按账号的统一星标
/// 查询，因此该行不提供子行（数据源不存在时优雅降级）。
fn unified_subrow_folder(
    view: FolderRole,
    account_id: u32,
    folders: &[FolderSummary],
    special_roles: &[SidebarFolderRole],
) -> Option<FolderSummary> {
    folders
        .iter()
        .find(|f| classified_role(account_id, &f.path, &f.name, special_roles) == Some(view))
        .cloned()
}

// ---------------------------------------------------------------------------
// WP-07：侧边栏会话级展开状态（统一子行 / 收藏分区 / 自定义文件夹树）。
// 对齐原版「折叠状态由应用层保存」的语义；本工作包落在进程内全局状态、
// 不持久化（重启后全部展开），避免触碰 app.rs 的 HylkiApp 字段（WP-03
// 并行实施期间冻结）。
// ---------------------------------------------------------------------------

/// 侧边栏会话级 UI 状态（gpui 全局；缺省即全部展开、统一子行收起）。
#[derive(Clone, Debug, Default)]
pub(crate) struct SidebarUiState {
    /// 已展开的统一行账号子行（nav row id，如 `"nav-inbox"`）。
    unified_expanded: HashSet<&'static str>,
    /// 收藏分区是否折叠（默认展开）。
    favorites_collapsed: bool,
    /// 筛选文件夹分区是否折叠（默认展开）。
    filtered_collapsed: bool,
    /// 智能文件夹分区是否折叠（默认展开）。
    smart_collapsed: bool,
    /// 标签分区是否折叠（默认展开）。
    tags_collapsed: bool,
    /// 折叠的自定义文件夹树节点（账号 id, IMAP 路径；默认展开）。
    tree_collapsed: HashSet<(u32, String)>,
}

impl Global for SidebarUiState {}

/// 读取侧边栏会话级状态（缺失时返回默认值）。
fn sidebar_ui_state(cx: &App) -> SidebarUiState {
    cx.try_global::<SidebarUiState>()
        .cloned()
        .unwrap_or_default()
}

/// 原地修改侧边栏会话级状态（读出-修改-写回；调用方需随后 `cx.notify()`）。
fn mutate_sidebar_ui_state(cx: &mut App, f: impl FnOnce(&mut SidebarUiState)) {
    let mut state = cx
        .try_global::<SidebarUiState>()
        .cloned()
        .unwrap_or_default();
    f(&mut state);
    cx.set_global(state);
}

/// 节点是否因某个已列出祖先被折叠而隐藏（对齐 Linux `hidden_by_collapse`）。
fn hidden_by_collapsed_ancestors(
    account_id: u32,
    path: &str,
    listed: &[&str],
    delim: Option<char>,
    collapsed: &HashSet<(u32, String)>,
) -> bool {
    let mut current = Some(path.to_string());
    while let Some(p) = current {
        let parent = match custom_tree_parent(&p, listed, delim) {
            Some(parent) => parent,
            None => return false,
        };
        if collapsed.contains(&(account_id, parent.clone())) {
            return true;
        }
        current = Some(parent);
    }
    false
}

/// 写信模板列表（Split Compose Button 的数据源，按钮仅在非空时渲染）。
///
/// Windows 端尚无「模板」存储（原版保存在 IMAP Templates 文件夹，Windows 端
/// 的「保存为模板」在 WP-10 落地），因此当前恒为空：写信主按钮右侧的 28px
/// 下拉箭头按「无模板时隐藏」的降级语义不渲染。WP-10 落地模板后，把真实
/// 模板填入该函数即可点亮，UI 与弹窗逻辑已就绪。
fn compose_templates(_app: &HylkiApp) -> Vec<SharedString> {
    Vec::new()
}

/// 统一导航行：图标 + 名称 + 右侧胶囊（34px 高、圆角 8px，柔和选中态避免与中栏撞色）。
fn nav_row(
    id: &'static str,
    icon: IconName,
    label: &'static str,
    count: Option<(u32, bool)>,
    selected: bool,
    palette: &theme::HylkiPalette,
) -> Stateful<Div> {
    let mut row = h_flex()
        .id(id)
        .h(px(NAV_ROW_HEIGHT))
        .px_2()
        .gap_2()
        .items_center()
        .rounded(px(8.0))
        .cursor_pointer()
        .hover(|s| s.bg(palette.accent_soft()))
        .when(selected, |el| el.bg(palette.accent_soft()));

    row = row.child(Icon::new(icon).with_size(px(16.0)).text_color(if selected {
        palette.accent
    } else {
        palette.text_muted
    }));
    row = row.child(
        div()
            .flex_1()
            .text_sm()
            .truncate()
            .font_weight(if selected {
                gpui_kit::FontWeight::SEMIBOLD
            } else {
                gpui_kit::FontWeight::NORMAL
            })
            .text_color(if selected {
                palette.accent
            } else {
                palette.text
            })
            .child(SharedString::from(label)),
    );
    if let Some((n, danger)) = count {
        if n > 0 {
            row = row.child(count_pill(n, selected, danger, palette));
        }
    }
    row
}

// ---------------------------------------------------------------------------
// WP-07：统一行账号子行、收藏分区与文件夹树的渲染辅助。
// ---------------------------------------------------------------------------

/// 统一行左侧的展开小箭头（独立点击目标，不触发行选中；`▸`/`▾` 旋转切换）。
fn subrow_expander(
    row_id: &'static str,
    expanded: bool,
    p: &theme::HylkiPalette,
    cx: &mut Context<HylkiApp>,
) -> Stateful<Div> {
    let muted = p.text_muted;
    let soft = p.accent_soft();
    div()
        .id(SharedString::from(format!("exp-{row_id}")))
        .w(px(18.0))
        .h(px(NAV_ROW_HEIGHT))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .cursor_pointer()
        .hover(move |s| s.bg(soft))
        .child(
            Icon::new(if expanded {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .with_size(px(13.0))
            .text_color(muted),
        )
        .on_click(cx.listener(move |_, _, _, cx| {
            mutate_sidebar_ui_state(cx, |s| {
                if expanded {
                    s.unified_expanded.remove(row_id);
                } else {
                    s.unified_expanded.insert(row_id);
                }
            });
            cx.notify();
        }))
}

/// 无子行的统一行的箭头占位（保持图标列对齐，对齐 Linux `TREE_EXPANDER_WIDTH`
/// 叶子占位的做法）。
fn expander_spacer() -> Div {
    div().w(px(18.0)).h(px(NAV_ROW_HEIGHT)).flex_shrink_0()
}

/// 统一行的账号子行（账号彩色圆点 + 名称 + 未读数；点击选中该账号对应文件夹）。
fn unified_subrow(
    app: &HylkiApp,
    acc: &AccountConfig,
    folder: &FolderSummary,
    p: &theme::HylkiPalette,
    cx: &mut Context<HylkiApp>,
) -> Stateful<Div> {
    let acc_id = acc.id;
    let folder_id = folder.id;
    let path = folder.path.clone();
    let unread = folder.unread_count;
    let dot = avatar::account_accent_color(&acc.email, acc.color.as_deref());
    let is_selected =
        app.current_view == CurrentView::AccountFolder && app.selected_folder_id == folder_id;
    let (text, accent, soft) = (p.text, p.accent, p.accent_soft());
    h_flex()
        .id(SharedString::from(format!(
            "unified-subrow-{acc_id}-{folder_id}"
        )))
        .h(px(26.0))
        .pl(px(34.0))
        .pr_2()
        .gap_2()
        .items_center()
        .rounded(px(8.0))
        .cursor_pointer()
        .hover(move |s| s.bg(soft))
        .when(is_selected, |row| row.bg(soft))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.select_folder(acc_id, folder_id, path.clone(), cx);
        }))
        .child(div().rounded_full().size(px(8.0)).flex_shrink_0().bg(dot))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_xs()
                .truncate()
                .text_color(if is_selected { accent } else { text })
                .font_weight(if is_selected {
                    gpui_kit::FontWeight::SEMIBOLD
                } else {
                    gpui_kit::FontWeight::NORMAL
                })
                .child(SharedString::from(acc.display_label())),
        )
        .when(unread > 0, |row| {
            row.child(count_pill(unread, is_selected, false, p))
        })
}

/// 统一行整块：左侧展开箭头（≥2 启用账号时）+ 行本体 + 账号子行列表。
#[allow(clippy::too_many_arguments)]
fn unified_row_with_subrows(
    app: &HylkiApp,
    p: &theme::HylkiPalette,
    cx: &mut Context<HylkiApp>,
    row_id: &'static str,
    icon: IconName,
    label: &'static str,
    count: Option<(u32, bool)>,
    selected: bool,
    multi_account: bool,
    expanded: bool,
    subrows: &[(&AccountConfig, FolderSummary)],
    on_select: impl Fn(&mut HylkiApp, &mut Context<HylkiApp>) + Copy + 'static,
    context_menu: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
) -> gpui_kit::AnyElement {
    let has_subrows = multi_account && !subrows.is_empty();
    v_flex()
        .gap_0p5()
        .child(
            h_flex()
                .gap_0p5()
                .items_center()
                .when(has_subrows, |row| {
                    row.child(subrow_expander(row_id, expanded, p, cx))
                })
                .when(!has_subrows && multi_account, |row| {
                    row.child(expander_spacer())
                })
                .child(
                    nav_row(
                        row_id,
                        icon,
                        label,
                        if expanded && has_subrows { None } else { count },
                        selected,
                        p,
                    )
                    .flex_1()
                    .on_click(cx.listener(move |this, _, _, cx| on_select(this, cx)))
                    .context_menu(context_menu),
                ),
        )
        .when(expanded && has_subrows, |col| {
            col.children(
                subrows
                    .iter()
                    .map(|(acc, folder)| unified_subrow(app, acc, folder, p, cx)),
            )
        })
        .into_any_element()
}

/// 收藏分区（WP-07 Favorites）：分区头（星标 + 名称 + 折叠箭头）+ 收藏文件夹行。
/// 收藏数据来自偏好 `sidebar_favorite_folders`（右键「收藏」写入），行右侧
/// 显示该文件夹未读数；无收藏时整个分区由调用方隐藏（空态降级）。
fn favorites_section(
    app: &HylkiApp,
    favorites: &[(AccountConfig, FolderSummary)],
    p: &theme::HylkiPalette,
    ui: &SidebarUiState,
    cx: &mut Context<HylkiApp>,
) -> Div {
    let collapsed = ui.favorites_collapsed;
    let (text, accent, muted, soft, gold) = (
        p.text,
        p.accent,
        p.text_muted,
        p.accent_soft(),
        theme::star_gold(),
    );
    let entity = cx.entity();
    let demo_mode = app.demo_mode;
    v_flex()
        .gap_0p5()
        .child(
            h_flex()
                .id("fav-header")
                .h(px(NAV_ROW_HEIGHT))
                .px_2()
                .gap_2()
                .items_center()
                .rounded(px(8.0))
                .cursor_pointer()
                .hover(move |s| s.bg(soft))
                .on_click(cx.listener(|_, _, _, cx| {
                    mutate_sidebar_ui_state(cx, |s| s.favorites_collapsed = !s.favorites_collapsed);
                    cx.notify();
                }))
                .child(
                    Icon::new(IconName::Star)
                        .with_size(px(15.0))
                        .text_color(gold),
                )
                .child(
                    div()
                        .flex_1()
                        .text_sm()
                        .font_weight(gpui_kit::FontWeight::MEDIUM)
                        .text_color(text)
                        .child("收藏"),
                )
                .child(
                    Icon::new(if collapsed {
                        IconName::ChevronRight
                    } else {
                        IconName::ChevronDown
                    })
                    .with_size(px(14.0))
                    .text_color(muted),
                ),
        )
        .when(!collapsed, |col| {
            col.children(favorites.iter().map(|(acc, folder)| {
                let acc_id = acc.id;
                let folder_id = folder.id;
                let path = folder.path.clone();
                let name = folder.name.clone();
                let unread = folder.unread_count;
                let dot = avatar::account_accent_color(&acc.email, acc.color.as_deref());
                let is_selected = app.current_view == CurrentView::AccountFolder
                    && app.selected_folder_id == folder_id;
                let menu_entity = entity.clone();
                h_flex()
                    .id(SharedString::from(format!("fav-{acc_id}-{folder_id}")))
                    .h(px(NAV_ROW_HEIGHT))
                    .pl(px(24.0))
                    .pr_2()
                    .gap_2()
                    .items_center()
                    .rounded(px(8.0))
                    .cursor_pointer()
                    .hover(move |s| s.bg(soft))
                    .when(is_selected, |row| row.bg(soft))
                    .on_click(cx.listener({
                        let path = path.clone();
                        move |this, _, _, cx| {
                            this.select_folder(acc_id, folder_id, path.clone(), cx);
                        }
                    }))
                    .child(div().rounded_full().size(px(8.0)).flex_shrink_0().bg(dot))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .truncate()
                            .text_color(if is_selected { accent } else { text })
                            .font_weight(if is_selected {
                                gpui_kit::FontWeight::SEMIBOLD
                            } else {
                                gpui_kit::FontWeight::NORMAL
                            })
                            .child(SharedString::from(name)),
                    )
                    .when(unread > 0, |row| {
                        row.child(count_pill(unread, is_selected, false, p))
                    })
                    .context_menu(move |menu, _, _| {
                        let menu_entity = menu_entity.clone();
                        if demo_mode {
                            return menu
                                .item(PopupMenuItem::new("全部标为已读").disabled(true))
                                .item(PopupMenuItem::new("刷新此文件夹").disabled(true))
                                .item(PopupMenuItem::new("移出收藏").disabled(true));
                        }
                        menu.item(PopupMenuItem::new("全部标为已读").on_click({
                            let e = menu_entity.clone();
                            let path = path.clone();
                            move |_, _, cx| {
                                e.update(cx, |this, cx| {
                                    this.mark_folder_all_read(acc_id, folder_id, path.clone(), cx);
                                });
                            }
                        }))
                        .item(PopupMenuItem::new("刷新此文件夹").on_click({
                            let e = menu_entity.clone();
                            let path = path.clone();
                            move |_, _, cx| {
                                e.update(cx, |this, cx| {
                                    this.refresh_folder(acc_id, folder_id, path.clone(), cx);
                                });
                            }
                        }))
                        .item(PopupMenuItem::new("移出收藏").on_click({
                            let e = menu_entity.clone();
                            let path = path.clone();
                            move |_, _, cx| {
                                e.update(cx, |this, cx| {
                                    this.mutate_preferences(cx, |prefs| {
                                        prefs.sidebar_favorite_folders.retain(|r| {
                                            !(r.account_id == acc_id && r.path == path)
                                        });
                                    });
                                });
                            }
                        }))
                    })
            }))
        })
}

/// 筛选文件夹分区（WP-07 Filtered Folders）：快捷筛选入口（未读 / 带附件 / 已加星标）。
fn filtered_section(
    app: &HylkiApp,
    p: &theme::HylkiPalette,
    ui: &SidebarUiState,
    cx: &mut Context<HylkiApp>,
) -> gpui_kit::Div {
    let collapsed = ui.filtered_collapsed;
    let soft = p.accent_soft();
    let text = p.text;
    let muted = p.text_muted;
    let accent = p.accent;
    let items: [(&'static str, IconName, &'static str, &'static str); 3] = [
        ("filt-unread", IconName::Mail, "未读邮件", "is:unread"),
        (
            "filt-attachment",
            IconName::Paperclip,
            "带附件",
            "has:attachment",
        ),
        ("filt-starred", IconName::Star, "已加星标", "is:starred"),
    ];
    let current_q = app.search_query.trim();
    v_flex()
        .pb_1()
        .gap_0p5()
        .child(
            h_flex()
                .id("filtered-section-header")
                .h(px(30.0))
                .px_2()
                .gap_2()
                .items_center()
                .rounded(px(8.0))
                .cursor_pointer()
                .hover(move |s| s.bg(soft))
                .on_click(cx.listener(|_, _, _, cx| {
                    mutate_sidebar_ui_state(cx, |s| s.filtered_collapsed = !s.filtered_collapsed);
                    cx.notify();
                }))
                .child(
                    Icon::new(IconName::Search)
                        .with_size(px(14.0))
                        .text_color(muted),
                )
                .child(
                    div()
                        .flex_1()
                        .text_sm()
                        .font_weight(gpui_kit::FontWeight::MEDIUM)
                        .text_color(text)
                        .child("筛选文件夹"),
                )
                .child(
                    Icon::new(if collapsed {
                        IconName::ChevronRight
                    } else {
                        IconName::ChevronDown
                    })
                    .with_size(px(14.0))
                    .text_color(muted),
                ),
        )
        .when(!collapsed, |col| {
            col.children(items.into_iter().map(|(id, icon, label, query)| {
                let selected = current_q.eq_ignore_ascii_case(query);
                h_flex()
                    .id(id)
                    .h(px(NAV_ROW_HEIGHT))
                    .pl(px(24.0))
                    .pr_2()
                    .gap_2()
                    .items_center()
                    .rounded(px(8.0))
                    .cursor_pointer()
                    .hover(move |s| s.bg(soft))
                    .when(selected, |r| r.bg(soft))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.apply_sidebar_query_shortcut(query.to_string(), window, cx);
                    }))
                    .child(Icon::new(icon).with_size(px(14.0)).text_color(if selected {
                        accent
                    } else {
                        muted
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .truncate()
                            .text_color(if selected { accent } else { text })
                            .child(label),
                    )
            }))
        })
}

/// 智能文件夹分区（WP-07 Smart Folders）：基于保存的搜索表达式一键过滤。
fn smart_folders_section(
    app: &HylkiApp,
    p: &theme::HylkiPalette,
    ui: &SidebarUiState,
    cx: &mut Context<HylkiApp>,
) -> gpui_kit::Div {
    let collapsed = ui.smart_collapsed;
    let soft = p.accent_soft();
    let text = p.text;
    let muted = p.text_muted;
    let accent = p.accent;
    let current_q = app.search_query.trim();
    v_flex()
        .pb_1()
        .gap_0p5()
        .child(
            h_flex()
                .id("smart-section-header")
                .h(px(30.0))
                .px_2()
                .gap_2()
                .items_center()
                .rounded(px(8.0))
                .cursor_pointer()
                .hover(move |s| s.bg(soft))
                .on_click(cx.listener(|_, _, _, cx| {
                    mutate_sidebar_ui_state(cx, |s| s.smart_collapsed = !s.smart_collapsed);
                    cx.notify();
                }))
                .child(
                    Icon::new(IconName::Folder)
                        .with_size(px(14.0))
                        .text_color(muted),
                )
                .child(
                    div()
                        .flex_1()
                        .text_sm()
                        .font_weight(gpui_kit::FontWeight::MEDIUM)
                        .text_color(text)
                        .child("智能文件夹"),
                )
                .child(
                    Icon::new(if collapsed {
                        IconName::ChevronRight
                    } else {
                        IconName::ChevronDown
                    })
                    .with_size(px(14.0))
                    .text_color(muted),
                ),
        )
        .when(!collapsed, |col| {
            col.children(app.prefs.smart_folders.iter().enumerate().map(|(idx, sf)| {
                let query = sf.query.clone();
                let name = sf.name.clone();
                let selected = !query.trim().is_empty() && current_q == query.trim();
                h_flex()
                    .id(("smart-folder", idx))
                    .h(px(NAV_ROW_HEIGHT))
                    .pl(px(24.0))
                    .pr_2()
                    .gap_2()
                    .items_center()
                    .rounded(px(8.0))
                    .cursor_pointer()
                    .hover(move |s| s.bg(soft))
                    .when(selected, |r| r.bg(soft))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.apply_sidebar_query_shortcut(query.clone(), window, cx);
                    }))
                    .child(
                        Icon::new(IconName::Folder)
                            .with_size(px(14.0))
                            .text_color(if selected { accent } else { muted }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .truncate()
                            .text_color(if selected { accent } else { text })
                            .child(SharedString::from(name)),
                    )
            }))
        })
}

/// 标签分区（WP-07 Tags）：彩色圆点 + 标签名，点击按 `tag:"..."` 过滤。
fn tags_section(
    app: &HylkiApp,
    p: &theme::HylkiPalette,
    ui: &SidebarUiState,
    cx: &mut Context<HylkiApp>,
) -> gpui_kit::Div {
    let collapsed = ui.tags_collapsed;
    let soft = p.accent_soft();
    let text = p.text;
    let muted = p.text_muted;
    let accent = p.accent;
    let current_q = app.search_query.trim();
    v_flex()
        .pb_1()
        .gap_0p5()
        .child(
            h_flex()
                .id("tags-section-header")
                .h(px(30.0))
                .px_2()
                .gap_2()
                .items_center()
                .rounded(px(8.0))
                .cursor_pointer()
                .hover(move |s| s.bg(soft))
                .on_click(cx.listener(|_, _, _, cx| {
                    mutate_sidebar_ui_state(cx, |s| s.tags_collapsed = !s.tags_collapsed);
                    cx.notify();
                }))
                .child(
                    Icon::new(IconName::Tag)
                        .with_size(px(14.0))
                        .text_color(muted),
                )
                .child(
                    div()
                        .flex_1()
                        .text_sm()
                        .font_weight(gpui_kit::FontWeight::MEDIUM)
                        .text_color(text)
                        .child("标签"),
                )
                .child(
                    Icon::new(if collapsed {
                        IconName::ChevronRight
                    } else {
                        IconName::ChevronDown
                    })
                    .with_size(px(14.0))
                    .text_color(muted),
                ),
        )
        .when(!collapsed, |col| {
            col.children(app.available_tags.iter().map(|tag| {
                let query = if tag.name.contains(' ') {
                    format!("tag:\"{}\"", tag.name)
                } else {
                    format!("tag:{}", tag.name)
                };
                let selected = current_q.eq_ignore_ascii_case(&query);
                let dot = avatar::parse_hex_color(&tag.color).unwrap_or(accent);
                let name = tag.name.clone();
                h_flex()
                    .id(("sidebar-tag", tag.id))
                    .h(px(NAV_ROW_HEIGHT))
                    .pl(px(24.0))
                    .pr_2()
                    .gap_2()
                    .items_center()
                    .rounded(px(8.0))
                    .cursor_pointer()
                    .hover(move |s| s.bg(soft))
                    .when(selected, |r| r.bg(soft))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.apply_sidebar_query_shortcut(query.clone(), window, cx);
                    }))
                    .child(div().rounded_full().size(px(8.0)).flex_shrink_0().bg(dot))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .truncate()
                            .text_color(if selected { accent } else { text })
                            .child(SharedString::from(name)),
                    )
            }))
        })
}

/// WP-07 侧边栏文件夹右键操作内联确认条（新建子文件夹 / 重命名 / 删除确认）。
fn folder_action_prompt_bar(
    app: &HylkiApp,
    action: &FolderActionPrompt,
    p: &theme::HylkiPalette,
    cx: &mut Context<HylkiApp>,
) -> gpui_kit::Div {
    let (title, needs_input, is_danger) = match action {
        FolderActionPrompt::CreateSubfolder { parent_path, .. } => (
            if parent_path.is_empty() {
                "新建文件夹".to_string()
            } else {
                format!("在「{parent_path}」下新建")
            },
            true,
            false,
        ),
        FolderActionPrompt::RenameFolder { current_name, .. } => {
            (format!("重命名「{current_name}」"), true, false)
        }
        FolderActionPrompt::DeleteFolder { folder_name, .. } => {
            (format!("确认删除「{folder_name}」？"), false, true)
        }
    };
    v_flex()
        .mx_2()
        .my_1()
        .p_2()
        .gap_1p5()
        .rounded(px(8.0))
        .border_1()
        .border_color(if is_danger { p.danger } else { p.border })
        .bg(p.surface_raised)
        .child(
            div()
                .text_xs()
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .text_color(if is_danger { p.danger } else { p.text })
                .truncate()
                .child(SharedString::from(title)),
        )
        .when(needs_input, |el| {
            el.child(Input::new(&app.folder_action_input))
        })
        .when_some(app.folder_action_error.as_ref(), |el, err| {
            el.child(div().text_xs().text_color(p.danger).child(err.clone()))
        })
        .child(
            h_flex()
                .justify_end()
                .gap_1()
                .child(
                    Button::new("folder-action-cancel")
                        .ghost()
                        .compact()
                        .label("取消")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.cancel_folder_action(cx);
                        })),
                )
                .child(
                    Button::new("folder-action-confirm")
                        .primary()
                        .compact()
                        .disabled(app.folder_action_busy)
                        .label(if app.folder_action_busy {
                            "处理中…"
                        } else if is_danger {
                            "删除"
                        } else {
                            "确定"
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.confirm_folder_action(cx);
                        })),
                ),
        )
}

/// 树折叠箭头槽宽度（与 `tree_chevron` 保持一致）。
const TREE_CHEVRON_WIDTH: f32 = 18.0;
/// 带折叠箭头行中箭头与文件夹行本体的水平间距。
const TREE_CHEVRON_GAP: f32 = 2.0;

/// 自定义文件夹树节点的折叠箭头（独立点击目标；行本体负责选中）。
fn tree_chevron(
    acc_id: u32,
    folder: &FolderSummary,
    expanded: bool,
    p: &theme::HylkiPalette,
    cx: &mut Context<HylkiApp>,
) -> Stateful<Div> {
    let path = folder.path.clone();
    let muted = p.text_muted;
    let soft = p.accent_soft();
    div()
        .id(SharedString::from(format!(
            "tree-chev-{acc_id}-{}",
            folder.id
        )))
        .w(px(TREE_CHEVRON_WIDTH))
        .h(px(NAV_ROW_HEIGHT))
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.0))
        .cursor_pointer()
        .hover(move |s| s.bg(soft))
        .child(
            Icon::new(if expanded {
                IconName::ChevronDown
            } else {
                IconName::ChevronRight
            })
            .with_size(px(13.0))
            .text_color(muted),
        )
        .on_click(cx.listener(move |_, _, _, cx| {
            mutate_sidebar_ui_state(cx, |s| {
                if !s.tree_collapsed.remove(&(acc_id, path.clone())) {
                    s.tree_collapsed.insert((acc_id, path.clone()));
                }
            });
            cx.notify();
        }))
}

/// 文件夹右键菜单（对齐 Linux `folder_menu_items` 的 Windows 可用集）：
/// 全部标为已读 / 刷新 / 收藏；自定义文件夹另有 新建子文件夹 / 重命名 /
/// 删除（后端待接线，先禁用占位）/ 隐藏文件夹 / 指定特殊角色；
/// 垃圾与已删除可清空。
#[allow(clippy::too_many_arguments)]
fn folder_context_menu(
    menu: PopupMenu,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
    app_entity: Entity<HylkiApp>,
    demo_mode: bool,
    acc_id: u32,
    folder_id: u32,
    folder_path: String,
    folder_name: String,
    favorite: bool,
    is_custom: bool,
    is_trash_or_junk: bool,
    is_graph: bool,
    assigned_role: Option<FolderRole>,
) -> PopupMenu {
    if demo_mode {
        // 演示模式：均为真实账号动作，全部禁用（与行内按钮惯例一致）。
        return menu
            .item(PopupMenuItem::new("全部标为已读").disabled(true))
            .item(PopupMenuItem::new("刷新此文件夹").disabled(true))
            .item(PopupMenuItem::new(if favorite { "移出收藏" } else { "收藏" }).disabled(true));
    }
    let mut menu = menu
        .item(PopupMenuItem::new("全部标为已读").on_click({
            let e = app_entity.clone();
            let path = folder_path.clone();
            move |_, _, cx| {
                e.update(cx, |this, cx| {
                    this.mark_folder_all_read(acc_id, folder_id, path.clone(), cx);
                });
            }
        }))
        .item(PopupMenuItem::new("刷新此文件夹").on_click({
            let e = app_entity.clone();
            let path = folder_path.clone();
            move |_, _, cx| {
                e.update(cx, |this, cx| {
                    this.refresh_folder(acc_id, folder_id, path.clone(), cx);
                });
            }
        }))
        .item(
            PopupMenuItem::new(if favorite { "移出收藏" } else { "收藏" }).on_click({
                let e = app_entity.clone();
                let path = folder_path.clone();
                let favorite = favorite;
                move |_, _, cx| {
                    e.update(cx, |this, cx| {
                        this.mutate_preferences(cx, |prefs| {
                            prefs
                                .sidebar_favorite_folders
                                .retain(|r| !(r.account_id == acc_id && r.path == path));
                            if !favorite {
                                prefs.sidebar_favorite_folders.push(SidebarFolderRef {
                                    account_id: acc_id,
                                    path: path.clone(),
                                });
                            }
                        });
                    });
                }
            }),
        );
    if is_custom {
        let delimiter = if folder_path.contains('.') && !folder_path.contains('/') {
            '.'
        } else {
            '/'
        };
        menu = menu
            .separator()
            .item(
                PopupMenuItem::new("新建子文件夹…")
                    .disabled(is_graph)
                    .on_click({
                        let e = app_entity.clone();
                        let path = folder_path.clone();
                        move |_, window, cx| {
                            e.update(cx, |this, cx| {
                                this.begin_create_subfolder(
                                    acc_id,
                                    path.clone(),
                                    delimiter,
                                    window,
                                    cx,
                                );
                            });
                        }
                    }),
            )
            .item(PopupMenuItem::new("重命名…").disabled(is_graph).on_click({
                let e = app_entity.clone();
                let path = folder_path.clone();
                let name = folder_name.clone();
                move |_, window, cx| {
                    e.update(cx, |this, cx| {
                        this.begin_rename_folder(
                            acc_id,
                            folder_id,
                            path.clone(),
                            name.clone(),
                            window,
                            cx,
                        );
                    });
                }
            }))
            .item(PopupMenuItem::new("删除…").disabled(is_graph).on_click({
                let e = app_entity.clone();
                let path = folder_path.clone();
                let name = folder_name.clone();
                move |_, _, cx| {
                    e.update(cx, |this, cx| {
                        this.begin_delete_folder(acc_id, folder_id, path.clone(), name.clone(), cx);
                    });
                }
            }))
            .item(PopupMenuItem::new("隐藏文件夹").on_click({
                let e = app_entity.clone();
                let path = folder_path.clone();
                move |_, _, cx| {
                    e.update(cx, |this, cx| {
                        this.mutate_preferences(cx, |prefs| {
                            prefs.sidebar_hidden_folders.push(SidebarFolderRef {
                                account_id: acc_id,
                                path: path.clone(),
                            });
                        });
                    });
                }
            }))
            .submenu("指定特殊角色", window, cx, {
                let e = app_entity.clone();
                let path = folder_path.clone();
                move |sub, _, _| {
                    let mut sub = sub;
                    for role in [
                        FolderRole::Inbox,
                        FolderRole::Drafts,
                        FolderRole::Sent,
                        FolderRole::Archive,
                        FolderRole::Junk,
                        FolderRole::Trash,
                    ] {
                        let e = e.clone();
                        let path = path.clone();
                        let key = role.key();
                        sub = sub.item(
                            PopupMenuItem::new(role.label())
                                .checked(assigned_role == Some(role))
                                .on_click(move |_, _, cx| {
                                    e.update(cx, |this, cx| {
                                        this.mutate_preferences(cx, |prefs| {
                                            prefs.sidebar_special_roles.retain(|r| {
                                                !(r.account_id == acc_id && r.path == path)
                                            });
                                            prefs.sidebar_special_roles.push(SidebarFolderRole {
                                                account_id: acc_id,
                                                path: path.clone(),
                                                role: key.to_string(),
                                            });
                                        });
                                    });
                                }),
                        );
                    }
                    if assigned_role.is_some() {
                        let e = e.clone();
                        let path = path.clone();
                        sub = sub.item(PopupMenuItem::new("清除指定角色").on_click(
                            move |_, _, cx| {
                                e.update(cx, |this, cx| {
                                    this.mutate_preferences(cx, |prefs| {
                                        prefs.sidebar_special_roles.retain(|r| {
                                            !(r.account_id == acc_id && r.path == path)
                                        });
                                    });
                                });
                            },
                        ));
                    }
                    sub
                }
            });
    }
    if is_trash_or_junk {
        menu = menu.separator().item(
            // Graph（Outlook OAuth）账号暂无批量清空能力，禁用「清空文件夹」。
            PopupMenuItem::new("清空文件夹…")
                .disabled(is_graph)
                .on_click({
                    let e = app_entity.clone();
                    let name = folder_name.clone();
                    move |_, _, cx| {
                        e.update(cx, |this, cx| {
                            this.request_empty_folder(
                                acc_id,
                                folder_id,
                                folder_path.clone(),
                                name.clone(),
                                cx,
                            );
                        });
                    }
                }),
        );
    }
    menu
}

/// 文件夹行本体左侧内边距（保证有无折叠箭头时图标左起点严格对齐）。
fn folder_row_left_padding(depth: usize, has_leading_chevron: bool) -> f32 {
    if has_leading_chevron {
        4.0
    } else {
        28.0 + depth as f32 * 16.0
    }
}

/// 带前置折叠箭头父级行外层容器的左侧内边距：
/// `tree_parent_row_left_padding(d) + TREE_CHEVRON_WIDTH + TREE_CHEVRON_GAP + folder_row_left_padding(d, true)`
/// 与无箭头行的 `folder_row_left_padding(d, false)` 完全相等。
fn tree_parent_row_left_padding(depth: usize) -> f32 {
    4.0 + depth as f32 * 16.0
}

/// 账号内文件夹行（角色与自定义共用）：图标 + 名称 + 计数 + 完整右键菜单。
/// `role` 为 `Some` 时按角色图标渲染（含右键「指定特殊角色」指派的角色）；
/// `depth` 为多级树缩进层级（自定义文件夹）；`has_leading_chevron` 表示行前是否
/// 挂有 `tree_chevron` 折叠箭头（此时由外层容器承担深度缩进）。
#[allow(clippy::too_many_arguments)]
fn account_folder_row(
    app: &HylkiApp,
    acc_id: u32,
    folder: &FolderSummary,
    role: Option<FolderRole>,
    depth: usize,
    has_leading_chevron: bool,
    p: &theme::HylkiPalette,
    cx: &mut Context<HylkiApp>,
) -> ContextMenu<Stateful<Div>> {
    let folder_id = folder.id;
    let folder_path = folder.path.clone();
    let folder_name = folder.name.clone();
    // 仅在「账号文件夹」视图下高亮具体文件夹行。
    let is_selected =
        folder_id == app.selected_folder_id && app.current_view == CurrentView::AccountFolder;
    let unread = folder.unread_count;
    let total = folder.total_count as u32;
    let icon = role
        .map(FolderRole::icon)
        .unwrap_or_else(|| folder_icon(&folder.path, &folder.name));
    let is_trash_or_junk = {
        let up = folder.path.to_ascii_uppercase();
        up.contains("TRASH")
            || up.contains("DELETED")
            || folder.name.contains("已删除")
            || up.contains("JUNK")
            || up.contains("SPAM")
            || folder.name.contains("垃圾")
    };
    // Graph（Outlook OAuth）账号暂无批量清空能力，禁用「清空文件夹」。
    let is_graph = app
        .accounts
        .iter()
        .find(|a| a.id == acc_id)
        .map(|a| a.auth_backend == AuthBackend::MicrosoftGraphOAuth2)
        .unwrap_or(false);
    let favorite = app
        .prefs
        .sidebar_favorite_folders
        .iter()
        .any(|r| r.account_id == acc_id && r.path == folder.path);
    // 用户右键「指定特殊角色」指派的角色（路径推断的角色不算）。
    let assigned_role = app
        .prefs
        .sidebar_special_roles
        .iter()
        .find(|r| r.account_id == acc_id && r.path == folder.path)
        .and_then(|r| FolderRole::from_key(&r.role));
    let menu_entity = cx.entity();
    let (text, accent, muted_fg, soft) = (p.text, p.accent, p.text_muted, p.accent_soft());
    let demo_mode = app.demo_mode;
    h_flex()
        .id(("folder", folder_id))
        .h(px(NAV_ROW_HEIGHT))
        .pl(px(folder_row_left_padding(depth, has_leading_chevron)))
        .pr_2()
        .gap_2()
        .items_center()
        .rounded(px(8.0))
        .cursor_pointer()
        .hover(move |s| s.bg(soft))
        .when(is_selected, |row| row.bg(soft))
        .on_click(cx.listener({
            let folder_path = folder_path.clone();
            move |this, _, _, cx| {
                this.select_folder(acc_id, folder_id, folder_path.clone(), cx);
            }
        }))
        .child(
            Icon::new(icon)
                .with_size(px(15.0))
                .text_color(if is_selected { accent } else { muted_fg }),
        )
        .child(
            div()
                .flex_1()
                .text_sm()
                .truncate()
                .font_weight(if is_selected {
                    gpui_kit::FontWeight::SEMIBOLD
                } else {
                    gpui_kit::FontWeight::NORMAL
                })
                .text_color(if is_selected { accent } else { text })
                .child(SharedString::from(folder_name.clone())),
        )
        .when(unread > 0, |row| {
            row.child(count_pill(unread, is_selected, false, p))
        })
        .when(unread == 0 && total > 0, |row| {
            row.child(
                div()
                    .text_xs()
                    .text_color(if is_selected { accent } else { muted_fg })
                    .child(SharedString::from(total.to_string())),
            )
        })
        .context_menu(move |menu, window, cx| {
            folder_context_menu(
                menu,
                window,
                cx,
                menu_entity.clone(),
                demo_mode,
                acc_id,
                folder_id,
                folder_path.clone(),
                folder_name.clone(),
                favorite,
                role.is_none(),
                is_trash_or_junk,
                is_graph,
                assigned_role,
            )
        })
}

/// 账号头右键菜单规格（由账号配置与文件夹状态派生，便于单测）。
#[derive(Clone, Debug, PartialEq, Eq)]
struct AccountContextMenuSpec {
    account_id: u32,
    account_label: String,
    inbox_target: Option<(u32, String)>,
    delimiter: char,
    is_graph: bool,
    hidden_count: usize,
    demo_mode: bool,
}

/// 生成账号右键菜单所需的纯逻辑规格（对齐 Linux `attach_account_menu`）。
fn account_context_menu_spec(
    acc: &AccountConfig,
    folders: &[FolderSummary],
    special_roles: &[SidebarFolderRole],
    hidden_folders: &[SidebarFolderRef],
    demo_mode: bool,
) -> AccountContextMenuSpec {
    let paths: Vec<&str> = folders.iter().map(|f| f.path.as_str()).collect();
    let inbox_target = unified_subrow_folder(FolderRole::Inbox, acc.id, folders, special_roles)
        .map(|f| (f.id, f.path));
    let delimiter = detect_delimiter(&paths).unwrap_or('/');
    let is_graph = acc.auth_backend == AuthBackend::MicrosoftGraphOAuth2;
    let hidden_count = hidden_folders
        .iter()
        .filter(|h| h.account_id == acc.id)
        .count();
    AccountContextMenuSpec {
        account_id: acc.id,
        account_label: acc.display_label(),
        inbox_target,
        delimiter,
        is_graph,
        hidden_count,
        demo_mode,
    }
}

/// 构造账号右键菜单（宽栏账号头与折叠窄轨头像共用）。
fn account_context_menu(
    menu: PopupMenu,
    app_entity: Entity<HylkiApp>,
    spec: AccountContextMenuSpec,
) -> PopupMenu {
    let acc_id = spec.account_id;
    if spec.demo_mode {
        let mut menu = menu;
        if spec.inbox_target.is_some() {
            menu = menu.item(PopupMenuItem::new("收件箱全部标为已读").disabled(true));
        }
        return menu
            .item(PopupMenuItem::new("刷新此账号").disabled(true))
            .separator()
            .item(PopupMenuItem::new("账号设置…").disabled(true))
            .item(PopupMenuItem::new("移除账号…").disabled(true));
    }

    let mut menu = menu;
    if let Some((inbox_id, inbox_path)) = spec.inbox_target.clone() {
        let e_read = app_entity.clone();
        menu = menu.item(
            PopupMenuItem::new("收件箱全部标为已读").on_click(move |_, _, cx| {
                let path = inbox_path.clone();
                e_read.update(cx, |this, cx| {
                    this.mark_folder_all_read(acc_id, inbox_id, path, cx);
                });
            }),
        );
    }
    let e_sync = app_entity.clone();
    let e_new = app_entity.clone();
    let e_cfg = app_entity.clone();
    let e_remove = app_entity.clone();
    let delimiter = spec.delimiter;
    let label_for_remove = spec.account_label.clone();

    menu = menu
        .item(PopupMenuItem::new("刷新此账号").on_click(move |_, _, cx| {
            e_sync.update(cx, |this, cx| {
                this.sync_account(acc_id, cx);
            });
        }))
        .item(
            PopupMenuItem::new("新建文件夹…")
                .disabled(spec.is_graph)
                .on_click(move |_, window, cx| {
                    e_new.update(cx, |this, cx| {
                        if this.auto_rail_active {
                            this.sidebar_peek_open = true;
                        } else if this.sidebar_hidden {
                            this.sidebar_hidden = false;
                        }
                        this.begin_create_subfolder(acc_id, String::new(), delimiter, window, cx);
                    });
                }),
        );
    if spec.hidden_count > 0 {
        let e_unhide = app_entity.clone();
        menu = menu.item(
            PopupMenuItem::new(format!("显示全部隐藏文件夹 ({})", spec.hidden_count)).on_click(
                move |_, _, cx| {
                    e_unhide.update(cx, |this, cx| {
                        this.mutate_preferences(cx, |prefs| {
                            prefs
                                .sidebar_hidden_folders
                                .retain(|h| h.account_id != acc_id);
                        });
                    });
                },
            ),
        );
    }
    menu.separator()
        .item(PopupMenuItem::new("账号设置…").on_click(move |_, _, cx| {
            e_cfg.update(cx, |this, cx| {
                let cfg = this.accounts.iter().find(|a| a.id == acc_id).cloned();
                this.open_settings_edit_account(cfg, cx);
            });
        }))
        .item(PopupMenuItem::new("移除账号…").on_click(move |_, _, cx| {
            let label = label_for_remove.clone();
            e_remove.update(cx, |this, cx| {
                this.request_remove_account(acc_id, label, cx);
            });
        }))
}

/// 账号折叠分组（头像 + 名称 + 邮箱 + 折叠箭头 + 文件夹列表；操作项在右键菜单）。
fn account_section(
    app: &HylkiApp,
    acc: &AccountConfig,
    folders: &[FolderSummary],
    data_dir: &std::path::Path,
    p: &theme::HylkiPalette,
    ui: &SidebarUiState,
    cx: &mut Context<HylkiApp>,
) -> gpui_kit::AnyElement {
    let acc_id = acc.id;
    let primary_label = acc.display_label();
    let secondary_label = if primary_label == acc.email {
        acc.display_name.trim().to_string()
    } else {
        acc.email.clone()
    };
    let (sidebar_bg, text, muted_fg, accent) = (p.sidebar, p.text, p.text_muted, p.accent);
    let expanded = app.account_expanded(acc_id);
    let account_unread: u32 = folders
        .iter()
        .filter(|f| f.path.eq_ignore_ascii_case("INBOX") || f.name.contains("收件箱"))
        .map(|f| f.unread_count)
        .sum();
    let menu_spec = account_context_menu_spec(
        acc,
        folders,
        &app.prefs.sidebar_special_roles,
        &app.prefs.sidebar_hidden_folders,
        app.demo_mode,
    );
    let acc_menu_entity = cx.entity();

    let is_syncing = app.syncing_accounts.contains(&acc_id);
    let sync_error = app.account_sync_errors.get(&acc_id).cloned();
    let sync_err_expanded = app.expanded_account_errors.contains(&acc_id);

    v_flex()
        .gap_0p5()
        .child(
            h_flex()
                .id(("acc-header", acc_id))
                .h(px(40.0))
                .pl_1()
                .pr_2()
                .gap_2()
                .items_center()
                .rounded(px(8.0))
                .cursor_pointer()
                .hover(|s| s.bg(p.accent_soft()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.toggle_account_expanded(acc_id, cx);
                }))
                .child(avatar::account_circle_with_badge(
                    acc,
                    data_dir,
                    28.0,
                    // 展开时「收件箱」行已有未读胶囊，头像角标去重；窄轨模式始终显示角标。
                    if expanded { 0 } else { account_unread },
                    accent,
                    sidebar_bg,
                ))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(gpui_kit::FontWeight::MEDIUM)
                                .text_color(text)
                                .truncate()
                                .child(SharedString::from(primary_label)),
                        )
                        .when(is_syncing, |col| {
                            col.child(
                                div()
                                    .text_xs()
                                    .text_color(accent)
                                    .truncate()
                                    .child("正在同步…"),
                            )
                        })
                        .when(!is_syncing && !secondary_label.is_empty(), |col| {
                            col.child(
                                div()
                                    .text_xs()
                                    .text_color(muted_fg)
                                    .truncate()
                                    .child(SharedString::from(secondary_label)),
                            )
                        }),
                )
                .when(sync_error.is_some(), |row| {
                    row.child(
                        Button::new(("acc-sync-err-btn", acc_id))
                            .ghost()
                            .compact()
                            .icon(IconName::TriangleAlert)
                            .tooltip("同步异常（点击查看详情）")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_account_sync_error_detail(acc_id, cx);
                            })),
                    )
                })
                .child(
                    Icon::new(if expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .with_size(px(14.0))
                    .text_color(muted_fg),
                )
                .context_menu(move |menu, _, _| {
                    account_context_menu(menu, acc_menu_entity.clone(), menu_spec.clone())
                }),
        )
        .when(sync_err_expanded, |section| {
            section.when_some(sync_error.clone(), |section, err| {
                section.child(
                    v_flex()
                        .mx_2()
                        .px_2()
                        .py_1p5()
                        .gap_1()
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(p.danger.opacity(0.4))
                        .bg(p.danger.opacity(0.08))
                        .child(div().text_xs().text_color(p.danger).child(err))
                        .child(
                            h_flex()
                                .justify_end()
                                .gap_1()
                                .child(
                                    Button::new(("acc-sync-err-dismiss", acc_id))
                                        .ghost()
                                        .compact()
                                        .label("忽略")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.dismiss_account_sync_error(acc_id, cx);
                                        })),
                                )
                                .child(
                                    Button::new(("acc-sync-err-retry", acc_id))
                                        .outline()
                                        .compact()
                                        .label("重试同步")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.sync_account(acc_id, cx);
                                        })),
                                ),
                        ),
                )
            })
        })
        .when(expanded, |section| {
            // WP-07：标准角色文件夹置顶（服务端顺序），随后直接平级列出
            // 自定义文件夹，并按 IMAP 分隔符构建多级树（带折叠箭头）。
            let (roles, customs) = partition_folders(
                acc_id,
                folders,
                &app.prefs.sidebar_special_roles,
                &app.prefs.sidebar_hidden_folders,
            );
            let mut section = section.children(roles.iter().map(|(folder, role)| {
                account_folder_row(app, acc_id, folder, Some(*role), 0, false, p, cx)
                    .into_any_element()
            }));
            let paths: Vec<&str> = customs.iter().map(|f| f.path.as_str()).collect();
            let delim = detect_delimiter(&paths);
            let tree_order = order_custom_tree(&paths, delim);
            let visible: Vec<&String> = tree_order
                .iter()
                .filter(|path| {
                    !hidden_by_collapsed_ancestors(acc_id, path, &paths, delim, &ui.tree_collapsed)
                })
                .collect();
            section = section.children(visible.iter().map(|path| {
                let folder = customs
                    .iter()
                    .find(|f| &f.path == *path)
                    .expect("树序路径必然来自自定义文件夹集合");
                let depth = folder_depth(path, &paths, delim);
                let has_children = has_listed_children(path, &paths, delim);
                if has_children {
                    let children_expanded = !ui.tree_collapsed.contains(&(acc_id, (*path).clone()));
                    let row = account_folder_row(app, acc_id, folder, None, depth, true, p, cx);
                    h_flex()
                        .pl(px(tree_parent_row_left_padding(depth)))
                        .gap(px(TREE_CHEVRON_GAP))
                        .items_center()
                        .child(tree_chevron(acc_id, folder, children_expanded, p, cx))
                        .child(row.flex_1())
                        .into_any_element()
                } else {
                    account_folder_row(app, acc_id, folder, None, depth, false, p, cx)
                        .into_any_element()
                }
            }));
            section
        })
        .into_any_element()
}

pub(crate) fn render(app: &HylkiApp, cx: &mut Context<HylkiApp>) -> gpui_kit::AnyElement {
    // 避坑清单第 4 条：render 开头先把语义色 Copy 出来。
    let p = theme::current(cx);
    let (sidebar_bg, border, text, muted_fg, danger) =
        (p.sidebar, p.border, p.text, p.text_muted, p.danger);
    let unified_unread = app.unified_unread_count();
    let primary_outbox_key = app.primary_send_toast_outbox_key();
    let outbox_items: Vec<OutboxItem> = app
        .outbox_by_account
        .values()
        .flat_map(|v| v.iter().cloned())
        // 仅当发件箱面板未手动展开且非定位目标时隐藏主发送 Toast 正展示的那一封；
        // 其余排队件、定时件、失败件和待确认件始终可从发件箱访问。
        .filter(|item| {
            app.outbox_expanded
                || app.focused_outbox_item == Some((item.account_id, item.id))
                || primary_outbox_key != Some((item.account_id, item.id))
        })
        .collect();
    let outbox_pending = outbox_items
        .iter()
        .filter(|i| matches!(i.status, OutboxStatus::Uncertain | OutboxStatus::Failed))
        .count() as u32;
    let demo_mode = app.demo_mode;
    let current_view = app.current_view;
    // 统一视图行选中态由 `current_view` 决定（演示模式同样映射）。
    let is_unified_selected = current_view == CurrentView::UnifiedInbox;
    let is_unread_selected = current_view == CurrentView::Unread;
    let is_starred_selected = current_view == CurrentView::Starred;
    let is_sent_selected = current_view == CurrentView::Sent;
    let is_drafts_selected = current_view == CurrentView::Drafts;
    let is_archive_selected = current_view == CurrentView::Archive;
    let is_junk_selected = current_view == CurrentView::Junk;
    let is_trash_selected = current_view == CurrentView::Trash;
    let is_snoozed_selected = current_view == CurrentView::Snoozed;
    // 草稿计数：真实账号取本地草稿数，演示模式取演示草稿箱邮件总数。
    let drafts_count = if demo_mode {
        app.demo_store
            .folders
            .iter()
            .find(|f| f.id == hylki_mail_core::demo::DEMO_DRAFTS_ID)
            .map(|f| f.total_count as u32)
            .unwrap_or(0)
    } else {
        app.drafts.len() as u32
    };

    let data_dir = config::data_dir();
    let ui = sidebar_ui_state(cx);
    let special_roles = app.prefs.sidebar_special_roles.clone();
    // WP-07：≥2 个启用账号时统一行可展开各账号子行（收件箱 / 已发送 /
    // 草稿箱 / 归档；已加星标无对应服务器文件夹，不提供子行）。
    let enabled_accounts: Vec<&AccountConfig> =
        app.accounts.iter().filter(|acc| acc.enabled).collect();
    let multi_account = enabled_accounts.len() >= 2;
    let subrow_targets = |view: FolderRole| -> Vec<(&AccountConfig, FolderSummary)> {
        if !multi_account {
            return Vec::new();
        }
        enabled_accounts
            .iter()
            .filter_map(|acc| {
                let folders = app.folders_by_account.get(&acc.id)?;
                Some((
                    *acc,
                    unified_subrow_folder(view, acc.id, folders, &special_roles)?,
                ))
            })
            .collect()
    };
    let inbox_subrows = subrow_targets(FolderRole::Inbox);
    let sent_subrows = subrow_targets(FolderRole::Sent);
    let drafts_subrows = subrow_targets(FolderRole::Drafts);
    let archive_subrows = subrow_targets(FolderRole::Archive);
    let junk_subrows = subrow_targets(FolderRole::Junk);
    let trash_subrows = subrow_targets(FolderRole::Trash);
    // WP-07 Favorites 分区：收藏文件夹（按账号 + 路径解析到当前文件夹列表，
    // 解析不到的条目跳过）。
    let favorites: Vec<(AccountConfig, FolderSummary)> = app
        .prefs
        .sidebar_favorite_folders
        .iter()
        .filter_map(|fav| {
            let acc = app
                .accounts
                .iter()
                .find(|a| a.id == fav.account_id && a.enabled)?;
            let folder = app
                .folders_by_account
                .get(&fav.account_id)?
                .iter()
                .find(|f| f.path == fav.path)
                .cloned()?;
            Some((acc.clone(), folder))
        })
        .collect();
    let account_sections: Vec<gpui_kit::AnyElement> = app
        .accounts
        .iter()
        .filter(|acc| acc.enabled)
        .map(|acc| {
            let folders = app
                .folders_by_account
                .get(&acc.id)
                .cloned()
                .unwrap_or_default();
            account_section(app, acc, &folders, &data_dir, &p, &ui, cx)
        })
        .collect();

    // 菜单按钮的弹出菜单（账号管理 / 切换亮暗主题 / 偏好设置 / 命令面板 / 退出）。
    let menu_button = main_menu_button(app, cx);

    v_flex()
        .w(px(app.sidebar_width))
        .min_w(px(crate::app::SIDEBAR_MIN_WIDTH))
        .max_w(px(crate::app::SIDEBAR_MAX_WIDTH))
        .h_full()
        .flex_shrink_0()
        .relative()
        .bg(sidebar_bg)
        .border_r_1()
        .border_color(border)
        // L-2：右缘拖拽把手（180~280 调宽，移动监听与持久化在 app.rs 根节点）；
        // 窄轨模式（render_collapsed）不挂把手。WP-01：断点自动窄轨下完整侧栏
        // 只会出现在半屏抽屉浮层里（对齐 Linux OverlaySplitView 抽屉模式宽度
        // 固定不可拖拽），此时也不挂把手。
        .when(!app.auto_rail_active, |sidebar| {
            sidebar.child(
                resize_handle::<SidebarDrag, SidebarDrag>(
                    "sidebar-resize-handle",
                    Axis::Horizontal,
                )
                .inside(HandleEdge::Trailing)
                .on_drag(SidebarDrag, |_, _, _, cx| cx.new(|_| SidebarDrag)),
            )
        })
        // 顶部栏 48px：刷新 + Hylki + 菜单。
        .child(
            h_flex()
                .h(px(HEADER_HEIGHT))
                .flex_shrink_0()
                .px_2()
                .gap_1()
                .items_center()
                .justify_between()
                .child(
                    Button::new("sb-refresh")
                        .ghost()
                        .compact()
                        .icon(IconName::RefreshCw)
                        .tooltip(if demo_mode {
                            "刷新演示数据"
                        } else {
                            "同步所有账号"
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            // 演示模式刷新演示数据；真实账号同步所有账号（阶段 3 起
                            // 与命令面板 / Ctrl+R 共用 refresh_all）。
                            this.refresh_all(cx);
                        })),
                )
                .child(
                    h_flex()
                        .flex_1()
                        .h_full()
                        .items_center()
                        .justify_center()
                        .window_control_area(WindowControlArea::Drag)
                        .child(
                            div()
                                .text_base()
                                .font_weight(gpui_kit::FontWeight::BOLD)
                                .text_color(text)
                                .child("Hylki"),
                        ),
                )
                .child(menu_button),
        )
        // 全宽蓝色「写邮件」按钮（38px、圆角 8px）；存在草稿模板时右侧紧贴
        // 28px 下拉小箭头，弹出模板列表从模板新建（WP-07 Split Compose
        // Button；模板数据源见 compose_templates，当前为空时箭头隐藏）。
        .child(div().px_3().pb_2().flex_shrink_0().child({
            let templates = compose_templates(app);
            let entity_for_templates = cx.entity();
            h_flex()
                .gap_1()
                .child(
                    div().flex_1().child(
                        Button::new("sb-compose")
                            .primary()
                            .label("写邮件")
                            .icon(IconName::Plus)
                            .rounded(px(8.0))
                            .h(px(38.0))
                            .w_full()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.start_new_compose(window, cx);
                            })),
                    ),
                )
                .when(!templates.is_empty(), |el| {
                    let entity_for_tpl_popup = entity_for_templates.clone();
                    el.child(
                        Button::new("sb-compose-templates")
                            .primary()
                            .icon(IconName::ChevronDown)
                            .rounded(px(8.0))
                            .h(px(38.0))
                            .w(px(28.0))
                            .dropdown_menu(move |menu, _, _| {
                                let entity_for_templates = entity_for_templates.clone();
                                let mut menu = menu;
                                for name in templates.iter().cloned() {
                                    let entity = entity_for_templates.clone();
                                    menu = menu.item(PopupMenuItem::new(name).on_click(
                                        move |_, window, cx| {
                                            entity.update(cx, |this, cx| {
                                                this.start_new_compose(window, cx);
                                            });
                                        },
                                    ));
                                }
                                menu
                            })
                            .on_open_change(move |open, _, cx| {
                                entity_for_tpl_popup.update(cx, |app, cx| {
                                    app.set_reader_popup_open(*open);
                                    cx.notify();
                                });
                            }),
                    )
                })
        }))
        // 统一导航固定区（对齐原版 pinned_box 三段式）：置于滚动区上方，
        // 账号多时统一收件箱等入口不会被滚走。
        .child(
            v_flex()
                .flex_shrink_0()
                .px_2()
                .gap_0p5()
                .when(app.prefs.unified_show_inbox, |el| {
                    let expanded = multi_account && ui.unified_expanded.contains("nav-inbox");
                    el.child(unified_row_with_subrows(
                        app,
                        &p,
                        cx,
                        "nav-inbox",
                        IconName::Inbox,
                        "收件箱",
                        Some((unified_unread, false)),
                        is_unified_selected,
                        multi_account,
                        expanded,
                        &inbox_subrows,
                        |this, cx| this.select_unified_view(CurrentView::UnifiedInbox, cx),
                        // 统一视图行右键：仅「全部标为已读」（对所有账号收件箱批量）。
                        {
                            let menu_entity = cx.entity();
                            move |menu, _, _| {
                                let menu_entity = menu_entity.clone();
                                menu.item(
                                    PopupMenuItem::new("全部标为已读（所有账号收件箱）")
                                        .disabled(demo_mode)
                                        .on_click(move |_, _, cx| {
                                            menu_entity.update(cx, |this, cx| {
                                                this.mark_all_inboxes_read(cx);
                                            });
                                        }),
                                )
                            }
                        },
                    ))
                })
                // 迭代 2：未读 / 已加星标 / 已发送 / 归档接入 mail-core 统一视图查询。
                .when(app.prefs.unified_show_unread, |el| {
                    el.child(unified_row_with_subrows(
                        app,
                        &p,
                        cx,
                        "nav-unread",
                        IconName::Mail,
                        "未读",
                        None,
                        is_unread_selected,
                        multi_account,
                        false,
                        &[],
                        |this, cx| this.select_unified_view(CurrentView::Unread, cx),
                        |menu, _, _| menu,
                    ))
                })
                .when(app.prefs.unified_show_starred, |el| {
                    el.child(unified_row_with_subrows(
                        app,
                        &p,
                        cx,
                        "nav-starred",
                        IconName::Star,
                        "已加星标",
                        None,
                        is_starred_selected,
                        multi_account,
                        false,
                        &[],
                        |this, cx| this.select_unified_view(CurrentView::Starred, cx),
                        |menu, _, _| menu,
                    ))
                })
                .when(app.prefs.unified_show_sent, |el| {
                    let expanded = multi_account && ui.unified_expanded.contains("nav-sent");
                    el.child(unified_row_with_subrows(
                        app,
                        &p,
                        cx,
                        "nav-sent",
                        IconName::Send,
                        "已发送",
                        None,
                        is_sent_selected,
                        multi_account,
                        expanded,
                        &sent_subrows,
                        |this, cx| this.select_unified_view(CurrentView::Sent, cx),
                        |menu, _, _| menu,
                    ))
                })
                // 草稿箱：真实账号展示本地草稿（中栏列表 + 侧栏条目），
                // 演示模式映射到演示草稿箱文件夹。
                .when(app.prefs.unified_show_drafts, |el| {
                    let expanded = multi_account && ui.unified_expanded.contains("nav-drafts");
                    el.child(unified_row_with_subrows(
                        app,
                        &p,
                        cx,
                        "nav-drafts",
                        IconName::FilePen,
                        "草稿箱",
                        Some((drafts_count, false)),
                        is_drafts_selected,
                        multi_account,
                        expanded,
                        &drafts_subrows,
                        |this, cx| this.select_unified_view(CurrentView::Drafts, cx),
                        |menu, _, _| menu,
                    ))
                })
                .when(app.prefs.unified_show_archive, |el| {
                    let expanded = multi_account && ui.unified_expanded.contains("nav-archive");
                    el.child(unified_row_with_subrows(
                        app,
                        &p,
                        cx,
                        "nav-archive",
                        IconName::Archive,
                        "归档",
                        None,
                        is_archive_selected,
                        multi_account,
                        expanded,
                        &archive_subrows,
                        |this, cx| this.select_unified_view(CurrentView::Archive, cx),
                        |menu, _, _| menu,
                    ))
                })
                .when(app.prefs.unified_show_junk, |el| {
                    let expanded = multi_account && ui.unified_expanded.contains("nav-junk");
                    el.child(unified_row_with_subrows(
                        app,
                        &p,
                        cx,
                        "nav-junk",
                        IconName::TriangleAlert,
                        "垃圾邮件",
                        None,
                        is_junk_selected,
                        multi_account,
                        expanded,
                        &junk_subrows,
                        |this, cx| this.select_unified_view(CurrentView::Junk, cx),
                        |menu, _, _| menu,
                    ))
                })
                .when(app.prefs.unified_show_trash, |el| {
                    let expanded = multi_account && ui.unified_expanded.contains("nav-trash");
                    el.child(unified_row_with_subrows(
                        app,
                        &p,
                        cx,
                        "nav-trash",
                        IconName::Trash,
                        "已删除",
                        None,
                        is_trash_selected,
                        multi_account,
                        expanded,
                        &trash_subrows,
                        |this, cx| this.select_unified_view(CurrentView::Trash, cx),
                        |menu, _, _| menu,
                    ))
                })
                .when(app.prefs.unified_show_snoozed, |el| {
                    el.child(unified_row_with_subrows(
                        app,
                        &p,
                        cx,
                        "nav-snoozed",
                        IconName::AlarmClock,
                        "稍后处理",
                        None,
                        is_snoozed_selected,
                        multi_account,
                        false,
                        &[],
                        |this, cx| this.select_unified_view(CurrentView::Snoozed, cx),
                        |menu, _, _| menu,
                    ))
                })
                .when(!outbox_items.is_empty(), |el| {
                    let now_utc = chrono::Utc::now().timestamp();
                    el.child(
                        nav_row(
                            "nav-outbox",
                            IconName::TriangleAlert,
                            "发件箱待处理",
                            Some((outbox_pending.max(outbox_items.len() as u32), true)),
                            app.outbox_expanded,
                            &p,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.outbox_expanded = !this.outbox_expanded;
                            cx.notify();
                        })),
                    )
                    .children(outbox_items.iter().map(|item| {
                        let outbox_id = item.id;
                        let account_id = item.account_id;
                        let in_active_undo = app.is_active_undo_outbox(account_id, outbox_id);
                        let is_focused = app.focused_outbox_item == Some((account_id, outbox_id));
                        let is_detail_open = app
                            .expanded_outbox_errors
                            .contains(&(account_id, outbox_id));
                        let (label, subline) = match item.status {
                            OutboxStatus::Uncertain => (
                                "发送结果待确认".to_string(),
                                Some("请先核对已发送文件夹或收件人，避免重复发送".to_string()),
                            ),
                            OutboxStatus::Failed => ("发送失败".to_string(), None),
                            OutboxStatus::Sending => ("正在发送".to_string(), None),
                            OutboxStatus::Queued => match item.send_at {
                                Some(ts) if !in_active_undo && ts > now_utc => (
                                    "已定时".to_string(),
                                    Some(format!(
                                        "计划于 {} 发送",
                                        crate::compose_schedule::schedule_label(ts)
                                    )),
                                ),
                                Some(ts) if in_active_undo && ts > now_utc => (
                                    "等待发送".to_string(),
                                    Some(format!("剩余 {} 秒", ts - now_utc)),
                                ),
                                _ => (
                                    "等待发送".to_string(),
                                    Some("已到投递时刻，等待后台处理".to_string()),
                                ),
                            },
                        };
                        let label_color = match item.status {
                            OutboxStatus::Failed | OutboxStatus::Uncertain => danger,
                            OutboxStatus::Sending => muted_fg,
                            OutboxStatus::Queued => p.accent,
                        };
                        let subj_text = if item.subject.trim().is_empty() {
                            "(无主题)".to_string()
                        } else {
                            item.subject.clone()
                        };
                        v_flex()
                            .ml(px(28.0))
                            .mr_2()
                            .my_0p5()
                            .p_2()
                            .gap_1()
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(if is_focused {
                                p.accent
                            } else if matches!(
                                item.status,
                                OutboxStatus::Failed | OutboxStatus::Uncertain
                            ) {
                                danger.opacity(0.35)
                            } else {
                                p.border
                            })
                            .bg(if is_focused {
                                p.accent_soft()
                            } else {
                                p.canvas
                            })
                            .child(
                                h_flex()
                                    .justify_between()
                                    .items_center()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_weight(gpui_kit::FontWeight::BOLD)
                                            .text_color(label_color)
                                            .child(SharedString::from(label)),
                                    )
                                    .when(!item.last_error.is_empty(), |row| {
                                        row.child(
                                            Button::new(("outbox-detail-toggle", outbox_id))
                                                .ghost()
                                                .compact()
                                                .text_size(px(11.0))
                                                .label(if is_detail_open {
                                                    "收起"
                                                } else {
                                                    "详情"
                                                })
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.toggle_outbox_error_detail(
                                                        account_id, outbox_id, cx,
                                                    );
                                                })),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(p.text)
                                    .truncate()
                                    .child(SharedString::from(subj_text)),
                            )
                            .when_some(subline, |col, s| {
                                col.child(
                                    div()
                                        .text_xs()
                                        .text_color(muted_fg)
                                        .child(SharedString::from(s)),
                                )
                            })
                            .when(!item.last_error.is_empty(), |col| {
                                col.child(
                                    div()
                                        .text_xs()
                                        .text_color(danger)
                                        .when(!is_detail_open, |d| d.truncate())
                                        .child(SharedString::from(item.last_error.clone())),
                                )
                            })
                            .when(item.status == OutboxStatus::Queued, |col| {
                                col.child(
                                    h_flex()
                                        .gap_1()
                                        .flex_wrap()
                                        .child(
                                            Button::new(("undo-outbox-sidebar", outbox_id))
                                                .outline()
                                                .compact()
                                                .label(if in_active_undo {
                                                    "撤销发送"
                                                } else {
                                                    "恢复草稿"
                                                })
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        this.undo_outbox_item_from_sidebar(
                                                            account_id, outbox_id, window, cx,
                                                        );
                                                    },
                                                )),
                                        )
                                        .child(
                                            Button::new(("send-now-outbox-sidebar", outbox_id))
                                                .ghost()
                                                .compact()
                                                .label("立即发送")
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.retry_outbox_item_from_sidebar(
                                                        account_id, outbox_id, cx,
                                                    );
                                                })),
                                        ),
                                )
                            })
                            .when(item.status == OutboxStatus::Failed, |col| {
                                col.child(
                                    h_flex()
                                        .gap_1()
                                        .flex_wrap()
                                        .child(
                                            Button::new(("retry-outbox", outbox_id))
                                                .primary()
                                                .compact()
                                                .label("重试")
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.retry_outbox_item_from_sidebar(
                                                        account_id, outbox_id, cx,
                                                    );
                                                })),
                                        )
                                        .child(
                                            Button::new(("discard-failed-outbox", outbox_id))
                                                .ghost()
                                                .compact()
                                                .label("移除记录")
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.delete_outbox_item_from_sidebar(
                                                        account_id, outbox_id, cx,
                                                    );
                                                })),
                                        ),
                                )
                            })
                            .when(item.status == OutboxStatus::Uncertain, |col| {
                                col.child(
                                    h_flex()
                                        .gap_1()
                                        .flex_wrap()
                                        .child(
                                            Button::new(("view-sent-uncertain", outbox_id))
                                                .outline()
                                                .compact()
                                                .label("核对已发送")
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.select_unified_view(
                                                        crate::app::CurrentView::Sent,
                                                        cx,
                                                    );
                                                })),
                                        )
                                        .child(
                                            Button::new(("dismiss-uncertain-outbox", outbox_id))
                                                .ghost()
                                                .compact()
                                                .label("已核对移除")
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.delete_outbox_item_from_sidebar(
                                                        account_id, outbox_id, cx,
                                                    );
                                                })),
                                        ),
                                )
                            })
                    }))
                }),
        )
        .when_some(app.folder_action.as_ref(), |el, action| {
            el.child(folder_action_prompt_bar(app, action, &p, cx))
        })
        // 账号区段：细分隔线 + 可滚动区（WP-07 收藏 / 筛选 / 智能文件夹 / 标签 / 账号分组树）。
        // Ctrl+Shift+A / 主菜单「显示 / 隐藏账号列表」（show_accounts，对齐
        // 原版 Show Accounts）只隐藏账号分组；其余分区有内容时保持可见。
        .when(
            app.show_accounts
                || !favorites.is_empty()
                || app.prefs.sidebar_show_filtered
                || (app.prefs.sidebar_show_smart && !app.prefs.smart_folders.is_empty())
                || (app.prefs.sidebar_show_tags && !app.available_tags.is_empty()),
            |el| {
                el.child(div().mx_2().my_1().h(px(1.0)).bg(border)).child(
                    v_flex()
                        .id("sidebar-scroll")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scrollbar()
                        .px_2()
                        .py_1()
                        .gap_0p5()
                        .when(!favorites.is_empty(), |el| {
                            el.child(favorites_section(app, &favorites, &p, &ui, cx))
                        })
                        .when(app.prefs.sidebar_show_filtered, |el| {
                            el.child(filtered_section(app, &p, &ui, cx))
                        })
                        .when(
                            app.prefs.sidebar_show_smart && !app.prefs.smart_folders.is_empty(),
                            |el| el.child(smart_folders_section(app, &p, &ui, cx)),
                        )
                        .when(
                            app.prefs.sidebar_show_tags && !app.available_tags.is_empty(),
                            |el| el.child(tags_section(app, &p, &ui, cx)),
                        )
                        .when(app.show_accounts, |el| {
                            el.when(app.accounts.is_empty(), |el| {
                                el.child(
                                    div()
                                        .pt_1()
                                        .text_xs()
                                        .text_color(muted_fg)
                                        .child("尚未配置邮箱，请在右侧添加账号。"),
                                )
                            })
                            .children(account_sections)
                        }),
                )
            },
        )
        // 底部固定区：联系人 / 附件入口。
        .when(
            app.prefs.sidebar_show_contacts || app.prefs.sidebar_show_attachments,
            |el| {
                el.child(
                    v_flex()
                        .flex_shrink_0()
                        .px_2()
                        .py_2()
                        .gap_0p5()
                        .border_t_1()
                        .border_color(border)
                        // 联系人 / 附件总览主内容视图入口（阶段 3，保留左栏切换）。
                        .when(app.prefs.sidebar_show_contacts, |el| {
                            el.child(
                                nav_row(
                                    "nav-contacts",
                                    IconName::CircleUser,
                                    "联系人",
                                    None,
                                    app.main_view == MainContentView::Contacts,
                                    &p,
                                )
                                .on_click(cx.listener(
                                    |this, _, window, cx| {
                                        this.open_contacts(window, cx);
                                    },
                                )),
                            )
                        })
                        .when(app.prefs.sidebar_show_attachments, |el| {
                            el.child(
                                nav_row(
                                    "nav-attachments",
                                    IconName::Paperclip,
                                    "附件",
                                    None,
                                    app.main_view == MainContentView::Gallery,
                                    &p,
                                )
                                .on_click(cx.listener(
                                    |this, _, window, cx| {
                                        this.open_gallery(window, cx);
                                    },
                                )),
                            )
                        }),
                )
            },
        )
        .into_any_element()
}

/// 主菜单按钮（宽栏顶栏与窄轨共用）。分组与顺序对齐原版 boot.rs
/// `build_main_menu`：设置类（账号管理 / 显示隐藏账号列表）→ 窗口类
/// （全屏 / 主题 / 偏好设置）→ 帮助类（命令面板 / 快捷键 / 关于）→ 退出。
fn main_menu_button(app: &HylkiApp, cx: &mut Context<HylkiApp>) -> impl IntoElement {
    let dark = app.dark_mode;
    let demo_mode = app.demo_mode;
    let show_accounts = app.show_accounts;
    let reader_position = app.reader_position;
    let focus_mode = app.focus_mode;
    let entity = cx.entity();
    let entity_for_accounts = entity.clone();
    let entity_for_fullscreen = entity.clone();
    let entity_for_theme = entity.clone();
    let entity_for_prefs = entity.clone();
    let entity_for_palette = entity.clone();
    let entity_for_shortcuts = entity.clone();
    let entity_for_about = entity.clone();
    let entity_for_quit = entity.clone();
    let entity_for_reader_pos = entity.clone();
    let entity_for_focus = entity.clone();
    let entity_for_popup = entity.clone();
    Button::new("sb-menu")
        .ghost()
        .compact()
        .icon(IconName::Menu)
        .dropdown_menu(move |menu, window, cx| {
            let entity = entity.clone();
            let entity_for_accounts = entity_for_accounts.clone();
            let entity_for_fullscreen = entity_for_fullscreen.clone();
            let entity_for_theme = entity_for_theme.clone();
            let entity_for_prefs = entity_for_prefs.clone();
            let entity_for_palette = entity_for_palette.clone();
            let entity_for_shortcuts = entity_for_shortcuts.clone();
            let entity_for_about = entity_for_about.clone();
            let entity_for_quit = entity_for_quit.clone();
            let entity_for_reader_pos = entity_for_reader_pos.clone();
            let entity_for_focus = entity_for_focus.clone();
            menu.item(
                // 演示模式下账号管理不可用（无 worker 账号可编辑）。
                PopupMenuItem::new("账号管理")
                    .icon(Icon::new(IconName::Settings))
                    .disabled(demo_mode)
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |app, cx| {
                            app.open_account_manager(cx);
                        });
                    }),
            )
            .item(
                // 对齐原版「Show Accounts」（Ctrl+Shift+A）：隐藏 / 恢复侧栏账号区段。
                PopupMenuItem::new(if show_accounts {
                    "隐藏账号列表  Ctrl+Shift+A"
                } else {
                    "显示账号列表  Ctrl+Shift+A"
                })
                .icon(Icon::new(IconName::PanelLeft))
                .on_click(move |_, _, cx| {
                    entity_for_accounts.update(cx, |app, cx| {
                        app.toggle_accounts_list(cx);
                    });
                }),
            )
            .separator()
            // L-6：阅读器位置子菜单（右上 / 下方 / 关闭，当前项打勾，Ctrl+Alt+1/2/3），
            // 对齐原版主菜单「Reading Pane」单选组（#475）。
            .submenu("阅读器位置", window, cx, move |mut sub, _, _| {
                for (label, position) in [
                    ("右侧  Ctrl+Alt+1", ReaderPosition::Right),
                    ("下方  Ctrl+Alt+2", ReaderPosition::Below),
                    ("关闭  Ctrl+Alt+3", ReaderPosition::Hidden),
                ] {
                    let e_pos = entity_for_reader_pos.clone();
                    let is_current = reader_position == position;
                    sub = sub.item(PopupMenuItem::new(label).checked(is_current).on_click(
                        move |_, _, cx| {
                            e_pos.update(cx, |app, cx| {
                                app.set_reader_position(position, cx);
                            });
                        },
                    ));
                }
                sub
            })
            .item(
                // L-6：专注模式开关（会话级，Ctrl+Shift+F，对齐原版 focus-mode）。
                PopupMenuItem::new(if focus_mode {
                    "关闭专注模式  Ctrl+Shift+F"
                } else {
                    "专注模式  Ctrl+Shift+F"
                })
                .checked(focus_mode)
                .icon(Icon::new(IconName::Eye))
                .on_click(move |_, _, cx| {
                    entity_for_focus.update(cx, |app, cx| {
                        app.toggle_focus_mode(cx);
                    });
                }),
            )
            .item(
                PopupMenuItem::new("全屏切换  F11")
                    .icon(Icon::new(IconName::Maximize))
                    .on_click(move |_, window, cx| {
                        entity_for_fullscreen.update(cx, |app, cx| {
                            app.toggle_fullscreen(window, cx);
                        });
                    }),
            )
            .item(
                PopupMenuItem::new(if dark {
                    "切换亮色主题"
                } else {
                    "切换暗色主题"
                })
                .icon(Icon::new(if dark { IconName::Sun } else { IconName::Moon }))
                .on_click(move |_, _, cx| {
                    entity_for_theme.update(cx, |app, cx| {
                        app.toggle_theme(cx);
                    });
                }),
            )
            .item(
                PopupMenuItem::new("设置  Ctrl+,")
                    .icon(Icon::new(IconName::Settings2))
                    .on_click(move |_, window, cx| {
                        entity_for_prefs.update(cx, |app, cx| {
                            app.open_preferences(window, cx);
                        });
                    }),
            )
            .separator()
            .item(
                PopupMenuItem::new("命令面板  Ctrl+K")
                    .icon(Icon::new(IconName::Command))
                    .on_click(move |_, window, cx| {
                        entity_for_palette.update(cx, |app, cx| {
                            app.open_command_palette(window, cx, false);
                        });
                    }),
            )
            .item(
                PopupMenuItem::new("键盘快捷键  F1")
                    .icon(Icon::new(IconName::Keyboard))
                    .on_click(move |_, _, cx| {
                        entity_for_shortcuts.update(cx, |app, cx| {
                            app.open_shortcuts(cx);
                        });
                    }),
            )
            .item(
                PopupMenuItem::new("关于 Hylki")
                    .icon(Icon::new(IconName::Info))
                    .on_click(move |_, _, cx| {
                        entity_for_about.update(cx, |app, cx| {
                            app.open_about(cx);
                        });
                    }),
            )
            .separator()
            .item(
                PopupMenuItem::new("退出 Hylki")
                    .icon(Icon::new(IconName::LogOut))
                    .on_click(move |_, _, cx| {
                        entity_for_quit.update(cx, |app, cx| {
                            app.quit_app(cx);
                        });
                    }),
            )
        })
        .on_open_change(move |open, _, cx| {
            entity_for_popup.update(cx, |app, cx| {
                app.set_reader_popup_open(*open);
                cx.notify();
            });
        })
}

/// 折叠窄轨宽度（对齐原版 `SIDEBAR_RAIL_WIDTH` = 80px）。
pub(crate) const RAIL_WIDTH: f32 = 80.0;

/// 左栏折叠窄轨（`sidebar_hidden` 时替代宽栏，对齐原版图标窄轨）：
/// - 顶部固定：主菜单 + 刷新；下方写邮件图标按钮（触发现有写信动作）。
/// - 固定统一收件箱图标按钮，右上角迷你角标显示全账号未读总数。
/// - 中部滚动区：各账号圆形头像（28px，右上角该账号未读数角标始终显示），
///   点击恢复宽栏并展开该账号分组。
/// - 底部固定：联系人 / 附件入口。
/// - 无账号（欢迎向导阶段）时仅显示固定按钮（账号区为空即降级）。
pub(crate) fn render_collapsed(app: &HylkiApp, cx: &mut Context<HylkiApp>) -> gpui_kit::AnyElement {
    // 避坑清单第 4 条：render 开头先把语义色 Copy 出来。
    let p = theme::current(cx);
    let (sidebar_bg, border, accent, accent_fg) = (p.sidebar, p.border, p.accent, p.accent_fg);
    let demo_mode = app.demo_mode;
    let data_dir = config::data_dir();
    let unified_unread = app.unified_unread_count();
    let is_unified_selected = app.current_view == CurrentView::UnifiedInbox;

    v_flex()
        .w(px(RAIL_WIDTH))
        .h_full()
        .flex_shrink_0()
        .bg(sidebar_bg)
        .border_r_1()
        .border_color(border)
        // 顶部固定：主菜单 + 刷新（与宽栏 48px 顶栏同高）。
        .child(
            h_flex()
                .h(px(HEADER_HEIGHT))
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .gap_0p5()
                .child(main_menu_button(app, cx))
                .child(
                    Button::new("rail-refresh")
                        .ghost()
                        .compact()
                        .icon(IconName::RefreshCw)
                        .tooltip(if demo_mode {
                            "刷新演示数据"
                        } else {
                            "同步所有账号"
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.refresh_all(cx);
                        })),
                ),
        )
        // 写邮件图标按钮（触发现有 NewCompose 写信动作）。
        .child(
            div().px_3().pb_2().flex_shrink_0().child(
                Button::new("rail-compose")
                    .primary()
                    .icon(IconName::Plus)
                    .tooltip("写邮件")
                    .rounded(px(8.0))
                    .h(px(38.0))
                    .w_full()
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.start_new_compose(window, cx);
                    })),
            ),
        )
        // 统一收件箱图标按钮 + 右上角全账号未读迷你角标（固定不随账号区滚动）。
        .child(
            h_flex().flex_shrink_0().justify_center().pb_2().child(
                div()
                    .relative()
                    .child(
                        Button::new("rail-unified")
                            .ghost()
                            .compact()
                            .icon(IconName::Inbox)
                            .tooltip("统一收件箱")
                            .selected(is_unified_selected)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.select_unified_view(CurrentView::UnifiedInbox, cx);
                            })),
                    )
                    .when(unified_unread > 0 && app.prefs.rail_unread_dots, |el| {
                        el.child(
                            div()
                                .absolute()
                                .top(px(-2.0))
                                .right(px(-2.0))
                                .rounded_full()
                                .w(px(8.0))
                                .h(px(8.0))
                                .bg(accent)
                                .border_1()
                                .border_color(sidebar_bg),
                        )
                    })
                    .when(unified_unread > 0 && !app.prefs.rail_unread_dots, |el| {
                        el.child(
                            div()
                                .absolute()
                                .top(px(-4.0))
                                .right(px(-6.0))
                                .rounded_full()
                                .min_w(px(16.0))
                                .h(px(16.0))
                                .bg(accent)
                                .border_1()
                                .border_color(sidebar_bg)
                                .flex()
                                .items_center()
                                .justify_center()
                                .px_0p5()
                                .text_size(px(10.0))
                                .text_color(accent_fg)
                                .font_weight(gpui_kit::FontWeight::BOLD)
                                .child(SharedString::from(if unified_unread > 99 {
                                    "99+".to_string()
                                } else {
                                    unified_unread.to_string()
                                })),
                        )
                    }),
            ),
        )
        // 账号头像滚动区（窄轨模式头像右上角未读角标始终显示）；
        // show_accounts 关闭时随宽栏一并隐藏（对齐原版 Show Accounts）。
        .when(app.show_accounts, |el| {
            let rail_dots = app.prefs.rail_unread_dots;
            el.child(
                v_flex()
                    .id("rail-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .items_center()
                    .py_1()
                    .gap_1()
                    .children(app.accounts.iter().filter(|acc| acc.enabled).map(|acc| {
                        let acc_id = acc.id;
                        let empty_folders: Vec<FolderSummary> = Vec::new();
                        let acc_folders = app
                            .folders_by_account
                            .get(&acc.id)
                            .unwrap_or(&empty_folders);
                        let account_unread: u32 = acc_folders
                            .iter()
                            .filter(|f| {
                                f.path.eq_ignore_ascii_case("INBOX") || f.name.contains("收件箱")
                            })
                            .map(|f| f.unread_count)
                            .sum();
                        let menu_spec = account_context_menu_spec(
                            acc,
                            acc_folders,
                            &app.prefs.sidebar_special_roles,
                            &app.prefs.sidebar_hidden_folders,
                            demo_mode,
                        );
                        let rail_menu_entity = cx.entity();
                        div()
                            .id(("rail-acc", acc_id))
                            .relative()
                            .cursor_pointer()
                            .p(px(2.0))
                            .rounded(px(8.0))
                            .hover(|s| s.bg(p.accent_soft()))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                // P3：断点窄轨下点击头像改为打开浮层侧栏（窄轨本身
                                // 不原地展开，对齐 Linux rail 交互）；宽窗口 / 手动
                                // 折叠时维持原行为：恢复宽栏并展开该账号分组。
                                if this.auto_rail_active {
                                    this.sidebar_peek_open = true;
                                } else {
                                    this.sidebar_hidden = false;
                                }
                                this.collapsed_accounts.remove(&acc_id);
                                cx.notify();
                            }))
                            .child(avatar::account_circle_with_badge(
                                acc,
                                &data_dir,
                                28.0,
                                if rail_dots { 0 } else { account_unread },
                                accent,
                                sidebar_bg,
                            ))
                            .when(rail_dots && account_unread > 0, |el| {
                                el.child(
                                    div()
                                        .absolute()
                                        .top(px(0.0))
                                        .right(px(0.0))
                                        .rounded_full()
                                        .w(px(8.0))
                                        .h(px(8.0))
                                        .bg(accent)
                                        .border_1()
                                        .border_color(sidebar_bg),
                                )
                            })
                            .context_menu(move |menu, _, _| {
                                account_context_menu(
                                    menu,
                                    rail_menu_entity.clone(),
                                    menu_spec.clone(),
                                )
                            })
                    })),
            )
        })
        // 底部固定：联系人 / 附件图标入口（现有入口的窄轨形态）。
        .when(
            app.prefs.sidebar_show_contacts || app.prefs.sidebar_show_attachments,
            |el| {
                el.child(
                    v_flex()
                        .flex_shrink_0()
                        .py_2()
                        .items_center()
                        .gap_1()
                        .border_t_1()
                        .border_color(border)
                        .when(app.prefs.sidebar_show_contacts, |el| {
                            el.child(
                                Button::new("rail-contacts")
                                    .ghost()
                                    .compact()
                                    .icon(IconName::CircleUser)
                                    .tooltip("联系人")
                                    .selected(app.main_view == MainContentView::Contacts)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_contacts(window, cx);
                                    })),
                            )
                        })
                        .when(app.prefs.sidebar_show_attachments, |el| {
                            el.child(
                                Button::new("rail-attachments")
                                    .ghost()
                                    .compact()
                                    .icon(IconName::Paperclip)
                                    .tooltip("附件")
                                    .selected(app.main_view == MainContentView::Gallery)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.open_gallery(window, cx);
                                    })),
                            )
                        }),
                )
            },
        )
        .into_any_element()
}

// ---------------------------------------------------------------------------
// WP-07：文件夹分类与多级树纯逻辑的单元测试（不依赖 gpui App 上下文）。
// ---------------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    fn folder(id: u32, name: &str, path: &str) -> FolderSummary {
        FolderSummary {
            id,
            account_id: 1,
            name: name.to_string(),
            path: path.to_string(),
            unread_count: 0,
            total_count: 0,
            has_more_remote: false,
        }
    }

    #[test]
    fn detect_delimiter_prefers_slash_then_dot() {
        assert_eq!(detect_delimiter(&["INBOX", "INBOX/Sent"]), Some('/'));
        assert_eq!(detect_delimiter(&["INBOX", "INBOX.Sent"]), Some('.'));
        assert_eq!(detect_delimiter(&["INBOX", "Sent", "Trash"]), None);
        // '/' 服务器上的含点路径仍按 '/' 处理。
        assert_eq!(detect_delimiter(&["a/b", "user.dom"]), Some('/'));
        assert_eq!(detect_delimiter(&[]), None);
    }

    #[test]
    fn path_is_under_is_strict_delimiter_descendant() {
        assert!(path_is_under("INBOX/Sub", "INBOX", '/'));
        assert!(path_is_under("inbox/sub/deep", "INBOX", '/'));
        assert!(!path_is_under("INBOX", "INBOX", '/'));
        // 「v1.2」在 '/' 服务器上不是「v1」的子级（对齐 Linux #554 规则）。
        assert!(!path_is_under("v1.2", "v1", '/'));
        assert!(path_is_under("v1.2", "v1", '.'));
    }

    #[test]
    fn custom_parent_and_depth_use_listed_ancestors_only() {
        // 「INBOX.Clients」在未列出 INBOX 自定义项时是顶层文件夹。
        let listed = ["INBOX.Clients", "INBOX.Clients.Acme"];
        assert_eq!(
            custom_tree_parent("INBOX.Clients", &listed, Some('.')),
            None
        );
        assert_eq!(
            custom_tree_parent("INBOX.Clients.Acme", &listed, Some('.')).as_deref(),
            Some("INBOX.Clients")
        );
        assert_eq!(folder_depth("INBOX.Clients", &listed, Some('.')), 0);
        assert_eq!(folder_depth("INBOX.Clients.Acme", &listed, Some('.')), 1);
        assert!(has_listed_children("INBOX.Clients", &listed, Some('.')));
        assert!(!has_listed_children(
            "INBOX.Clients.Acme",
            &listed,
            Some('.')
        ));
        assert_eq!(folder_depth("anything", &listed, None), 0);
    }

    #[test]
    fn order_custom_tree_is_depth_first_by_leaf_name() {
        let customs = ["Z", "Z/aaaa", "Zb", "A", "A/x", "A/x/y"];
        let out = order_custom_tree(&customs, Some('/'));
        assert_eq!(out, vec!["A", "A/x", "A/x/y", "Z", "Z/aaaa", "Zb"]);
        // 无分隔符时按名次序平铺。
        let flat = order_custom_tree(&["b", "a"], None);
        assert_eq!(flat, vec!["a", "b"]);
    }

    #[test]
    fn classified_role_prefers_assigned_over_intrinsic() {
        let assigned = vec![SidebarFolderRole {
            account_id: 1,
            path: "Mail/Outbox mirror".to_string(),
            role: "sent".to_string(),
        }];
        assert_eq!(
            classified_role(1, "Mail/Outbox mirror", "Outbox mirror", &assigned),
            Some(FolderRole::Sent)
        );
        // 其他账号的同名路径不受影响。
        assert_eq!(
            classified_role(2, "Mail/Outbox mirror", "Outbox mirror", &assigned),
            None
        );
        // 未指派时按路径推断。
        assert_eq!(
            classified_role(1, "INBOX", "收件箱", &[]),
            Some(FolderRole::Inbox)
        );
        assert_eq!(
            classified_role(1, "Folders/Junk", "Junk", &[]),
            Some(FolderRole::Junk)
        );
        assert_eq!(classified_role(1, "Notes", "Notes", &[]), None);
    }

    #[test]
    fn partition_folders_keeps_roles_first_and_hides_marked() {
        let folders = vec![
            folder(1, "INBOX", "INBOX"),
            folder(2, "Projects", "Projects"),
            folder(3, "Sent", "Sent"),
            folder(4, "Archive", "Archive"),
            folder(5, "Trash", "Trash"),
            folder(6, "Archived 2024", "Archive/2024"),
        ];
        let hidden = vec![SidebarFolderRef {
            account_id: 1,
            path: "Archive/2024".to_string(),
        }];
        let (roles, customs) = partition_folders(1, &folders, &[], &hidden);
        let role_paths: Vec<&str> = roles.iter().map(|(f, _)| f.path.as_str()).collect();
        assert_eq!(role_paths, vec!["INBOX", "Sent", "Archive", "Trash"]);
        // 隐藏文件夹整行剔除，剩余自定义只有 Projects。
        let custom_paths: Vec<&str> = customs.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(custom_paths, vec!["Projects"]);
    }

    #[test]
    fn unified_subrow_targets_map_role_folders() {
        let folders = vec![
            folder(1, "INBOX", "INBOX"),
            folder(2, "Sent Items", "Sent Items"),
            folder(3, "Notes", "Notes"),
        ];
        let inbox = unified_subrow_folder(FolderRole::Inbox, 1, &folders, &[]);
        let sent = unified_subrow_folder(FolderRole::Sent, 1, &folders, &[]);
        assert_eq!(inbox.map(|f| f.id), Some(1));
        assert_eq!(sent.map(|f| f.id), Some(2));
        // 已加星标没有对应服务器文件夹 → 无子行。
        assert_eq!(
            unified_subrow_folder(FolderRole::Trash, 1, &folders, &[]),
            None
        );
    }

    #[test]
    fn hidden_by_collapsed_ancestors_walks_parent_chain() {
        let listed = ["Work", "Work/2024", "Work/2024/Q1"];
        let collapsed: HashSet<(u32, String)> = HashSet::from([(1, "Work".to_string())]);
        assert!(hidden_by_collapsed_ancestors(
            1,
            "Work/2024/Q1",
            &listed,
            Some('/'),
            &collapsed
        ));
        assert!(hidden_by_collapsed_ancestors(
            1,
            "Work/2024",
            &listed,
            Some('/'),
            &collapsed
        ));
        assert!(!hidden_by_collapsed_ancestors(
            1,
            "Work",
            &listed,
            Some('/'),
            &collapsed
        ));
        // 其他账号的同名折叠不影响。
        let other: HashSet<(u32, String)> = HashSet::from([(2, "Work".to_string())]);
        assert!(!hidden_by_collapsed_ancestors(
            1,
            "Work/2024",
            &listed,
            Some('/'),
            &other
        ));
    }

    #[test]
    fn folder_row_padding_aligns_icons_with_and_without_chevron() {
        assert_eq!(folder_row_left_padding(0, false), 28.0);
        for depth in 0..=2 {
            let plain_icon_x = folder_row_left_padding(depth, false);
            let parent_icon_x = tree_parent_row_left_padding(depth)
                + TREE_CHEVRON_WIDTH
                + TREE_CHEVRON_GAP
                + folder_row_left_padding(depth, true);
            assert_eq!(
                parent_icon_x, plain_icon_x,
                "depth={depth} 时带折叠箭头与普通文件夹行的图标左起点必须对齐"
            );
        }
    }

    #[test]
    fn account_context_menu_spec_covers_imap_special_role_graph_and_demo() {
        let mut imap_acc = AccountConfig {
            id: 7,
            email: "work@example.com".to_string(),
            label: Some("工作邮箱".to_string()),
            ..Default::default()
        };
        let folders = vec![
            folder(10, "收件箱", "INBOX"),
            folder(11, "Projects", "INBOX.Projects"),
        ];
        let hidden = vec![
            SidebarFolderRef {
                account_id: 7,
                path: "INBOX.Projects".to_string(),
            },
            SidebarFolderRef {
                account_id: 99,
                path: "Other".to_string(),
            },
        ];
        let spec = account_context_menu_spec(&imap_acc, &folders, &[], &hidden, false);
        assert_eq!(spec.account_id, 7);
        assert_eq!(spec.account_label, "工作邮箱");
        assert_eq!(spec.inbox_target, Some((10, "INBOX".to_string())));
        assert_eq!(spec.delimiter, '.');
        assert!(!spec.is_graph);
        assert_eq!(spec.hidden_count, 1);
        assert!(!spec.demo_mode);

        // 通过 sidebar_special_roles 手动指定收件箱时优先匹配指定角色文件夹。
        let custom_folders = vec![folder(20, "Mail", "CustomMail")];
        let special_roles = vec![SidebarFolderRole {
            account_id: 7,
            path: "CustomMail".to_string(),
            role: "inbox".to_string(),
        }];
        let assigned_spec =
            account_context_menu_spec(&imap_acc, &custom_folders, &special_roles, &[], false);
        assert_eq!(
            assigned_spec.inbox_target,
            Some((20, "CustomMail".to_string()))
        );
        assert_eq!(assigned_spec.delimiter, '/');

        // 尚未同步出文件夹时 inbox_target 为 None，默认分隔符为 '/'。
        let empty_spec = account_context_menu_spec(&imap_acc, &[], &[], &[], false);
        assert_eq!(empty_spec.inbox_target, None);
        assert_eq!(empty_spec.delimiter, '/');

        // Microsoft Graph OAuth 账号禁用新建文件夹；demo 模式标记 demo_mode。
        imap_acc.auth_backend = AuthBackend::MicrosoftGraphOAuth2;
        let graph_spec = account_context_menu_spec(&imap_acc, &folders, &[], &[], true);
        assert!(graph_spec.is_graph);
        assert!(graph_spec.demo_mode);
    }
}
