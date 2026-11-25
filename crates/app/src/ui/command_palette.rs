//! 命令面板（方案文档第三节页面 6）：
//! - 居中悬浮卡片 560x440（阴影 + 圆角 12px），带半透明遮罩，
//!   点击遮罩或按 Esc 发 [`CommandPaletteEvent::Closed`]。
//! - 顶部单行搜索框（placeholder「输入命令、文件夹或 > 仅筛选操作...」），
//!   输入以 `>` 开头时仅显示操作类命令；其余为忽略大小写的名称子串过滤。
//! - 下方滚动结果列表：图标 + 名称 + 右侧类型提示（「操作」/「文件夹」/
//!   快捷键）；↑ / ↓ 循环移动高亮，Enter（或点击行）执行并发
//!   [`CommandPaletteEvent::CommandExecuted`]。
//!
//! 自包含组件：键盘依赖 gpui-kit Input 的既有全局绑定（单行输入对
//! ↑ / ↓ / Esc 不消费事件，会冒泡到卡片上的 `on_key_down`）。

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::Icon;
use gpui_kit::component::{h_flex, v_flex, Sizable};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    black, div, px, AppContext, Context, EventEmitter, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, ParentElement, Render, SharedString, Stateful,
    StatefulInteractiveElement, Styled,
};

/// 命令面板可执行的命令（由集成者构造并传入）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteCommand {
    /// 写新邮件。
    NewCompose,
    /// 刷新所有账号。
    RefreshAll,
    /// 切换统一收件箱视图。
    ToggleUnifiedInbox,
    /// 打开账号管理。
    OpenAccountSettings,
    /// 打开偏好设置。
    OpenPreferences,
    /// 切换亮 / 暗主题。
    ToggleTheme,
    /// 跳转到指定账号的指定文件夹。
    GoFolder { account_id: u32, folder_id: u32 },
    /// 跳转到「已加星标」统一视图。
    GoStarred,
    /// 跳转到「已发送」统一视图。
    GoSent,
    /// 跳转到「草稿箱」视图。
    GoDrafts,
    /// 跳转到「归档」统一视图。
    GoArchive,
    /// 打开联系人视图。
    OpenContacts,
    /// 打开附件总览视图。
    OpenGallery,
    /// 回复当前选中邮件（无选中时不注入）。
    ReplySelected,
    /// 归档当前选中邮件（无选中时不注入）。
    ArchiveSelected,
    /// 删除当前选中邮件（无选中时不注入）。
    DeleteSelected,
    /// 当前选中邮件标为未读（无选中时不注入）。
    MarkUnreadSelected,
    /// 切换当前选中邮件星标（无选中时不注入）。
    ToggleStarSelected,
}

impl PaletteCommand {
    /// 命令显示名（GoFolder 为通用名，动态名见 [`Self::label_with`]）。
    pub fn label(&self) -> &'static str {
        match self {
            PaletteCommand::NewCompose => "写新邮件",
            PaletteCommand::RefreshAll => "刷新全部",
            PaletteCommand::ToggleUnifiedInbox => "切换统一收件箱",
            PaletteCommand::OpenAccountSettings => "打开账号管理",
            PaletteCommand::OpenPreferences => "打开偏好设置",
            PaletteCommand::ToggleTheme => "切换亮暗主题",
            PaletteCommand::GoFolder { .. } => "跳转到文件夹",
            PaletteCommand::GoStarred => "跳转到已加星标",
            PaletteCommand::GoSent => "跳转到已发送",
            PaletteCommand::GoDrafts => "跳转到草稿箱",
            PaletteCommand::GoArchive => "跳转到归档",
            PaletteCommand::OpenContacts => "打开联系人",
            PaletteCommand::OpenGallery => "打开附件总览",
            PaletteCommand::ReplySelected => "回复当前邮件",
            PaletteCommand::ArchiveSelected => "归档当前邮件",
            PaletteCommand::DeleteSelected => "删除当前邮件",
            PaletteCommand::MarkUnreadSelected => "标当前邮件为未读",
            PaletteCommand::ToggleStarSelected => "切换当前邮件星标",
        }
    }

    /// 带文件夹名的显示名（非 GoFolder 命令与 [`Self::label`] 相同）。
    pub fn label_with(&self, folder_name: &str) -> String {
        match self {
            PaletteCommand::GoFolder { .. } => format!("跳转到 {folder_name}"),
            _ => self.label().to_string(),
        }
    }

    /// 是否为文件夹跳转类命令（`>` 前缀过滤时被排除）。
    pub fn is_folder(&self) -> bool {
        matches!(self, PaletteCommand::GoFolder { .. })
    }

    /// 是否为视图跳转类导航（`>` 前缀过滤时与文件夹一并排除，
    /// 对齐原版命令面板 `action: false` 的 go-to 语义）。
    pub fn is_view(&self) -> bool {
        matches!(
            self,
            PaletteCommand::GoStarred
                | PaletteCommand::GoSent
                | PaletteCommand::GoDrafts
                | PaletteCommand::GoArchive
                | PaletteCommand::OpenContacts
                | PaletteCommand::OpenGallery
        )
    }

    /// 命令图标。
    fn icon(self) -> IconName {
        match self {
            PaletteCommand::NewCompose => IconName::Plus,
            PaletteCommand::RefreshAll => IconName::RefreshCw,
            PaletteCommand::ToggleUnifiedInbox => IconName::Inbox,
            PaletteCommand::OpenAccountSettings => IconName::Settings,
            PaletteCommand::OpenPreferences => IconName::Settings2,
            PaletteCommand::ToggleTheme => IconName::Moon,
            PaletteCommand::GoFolder { .. } => IconName::Folder,
            PaletteCommand::GoStarred => IconName::Star,
            PaletteCommand::GoSent => IconName::Send,
            PaletteCommand::GoDrafts => IconName::FilePen,
            PaletteCommand::GoArchive => IconName::Archive,
            PaletteCommand::OpenContacts => IconName::CircleUser,
            PaletteCommand::OpenGallery => IconName::Paperclip,
            PaletteCommand::ReplySelected => IconName::Reply,
            PaletteCommand::ArchiveSelected => IconName::Archive,
            PaletteCommand::DeleteSelected => IconName::Trash,
            PaletteCommand::MarkUnreadSelected => IconName::Mail,
            PaletteCommand::ToggleStarSelected => IconName::Star,
        }
    }

    /// 已知全局快捷键提示（无则返回 None，列表显示类型提示）。
    fn shortcut(self) -> Option<&'static str> {
        match self {
            PaletteCommand::NewCompose => Some("Ctrl+N"),
            // 阶段三起刷新仅保留 F5（Ctrl+R 改绑回复）。
            PaletteCommand::RefreshAll => Some("F5"),
            _ => None,
        }
    }

    /// 命令详情/检索辅助标签（与标题共同参与多词分词匹配，对齐原版 `title + detail`）。
    pub fn detail(self) -> &'static str {
        match self {
            PaletteCommand::NewCompose => "操作 compose new message",
            PaletteCommand::RefreshAll => "操作 refresh sync 同步",
            PaletteCommand::ToggleUnifiedInbox => "操作 unified inbox 视图",
            PaletteCommand::OpenAccountSettings => "操作 accounts settings 设置",
            PaletteCommand::OpenPreferences => "操作 preferences settings 偏好",
            PaletteCommand::ToggleTheme => "操作 dark light theme 外观",
            PaletteCommand::GoFolder { .. } => "文件夹 folder move 移动",
            PaletteCommand::GoStarred => "视图 starred 星标",
            PaletteCommand::GoSent => "视图 sent 已发送",
            PaletteCommand::GoDrafts => "视图 drafts 草稿",
            PaletteCommand::GoArchive => "视图 archive 归档",
            PaletteCommand::OpenContacts => "视图 contacts 通讯录",
            PaletteCommand::OpenGallery => "视图 attachments gallery 画廊",
            PaletteCommand::ReplySelected => "操作 reply 回复",
            PaletteCommand::ArchiveSelected => "操作 archive 归档",
            PaletteCommand::DeleteSelected => "操作 delete trash 删除",
            PaletteCommand::MarkUnreadSelected => "操作 unread 未读",
            PaletteCommand::ToggleStarSelected => "操作 star 星标",
        }
    }
}

/// 命令面板事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandPaletteEvent {
    /// 用户通过 Enter 或点击行执行了命令。
    CommandExecuted(PaletteCommand),
    /// 用户按 Esc 或点击遮罩关闭面板。
    Closed,
}

/// 命令面板视图状态（独立 Entity，由 HylkiApp 作为浮层挂载）。
pub struct CommandPaletteState {
    search_input: gpui_kit::Entity<InputState>,
    /// 当前搜索词（含可能的 `>` 前缀）。
    query: String,
    /// 集成者传入的候选命令。
    commands: Vec<PaletteCommand>,
    /// GoFolder 命令的动态显示名 `(account_id, folder_id, 文件夹名)`，
    /// 由集成者通过 [`Self::set_folder_names`] 提供。
    folder_names: Vec<(u32, u32, String)>,
    /// 当前高亮下标（指向过滤后的列表）。
    selected: usize,
}

impl EventEmitter<CommandPaletteEvent> for CommandPaletteState {}

impl CommandPaletteState {
    /// `actions_only` 为 true 时预填 `>`（Ctrl+Shift+K 入口，仅筛选操作命令）。
    pub fn new(
        window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
        commands: Vec<PaletteCommand>,
        actions_only: bool,
    ) -> Self {
        let search_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("输入命令、文件夹或 > 仅筛选操作...")
        });
        cx.subscribe(
            &search_input,
            |this, input, event: &InputEvent, cx| match event {
                InputEvent::Change => {
                    this.query = input.read(cx).value().to_string();
                    this.selected = 0;
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => {
                    this.execute_selected(cx);
                }
                _ => {}
            },
        )
        .detach();
        // Ctrl+Shift+K 入口预填 `>`，与原版「actions only」语义一致。
        if actions_only {
            search_input.update(cx, |state, cx| state.set_value(">", window, cx));
        }
        // 打开面板即聚焦搜索框，保证键盘输入与 ↑ / ↓ / Esc 可用。
        search_input.update(cx, |state, cx| state.focus(window, cx));
        Self {
            search_input,
            query: if actions_only {
                ">".to_string()
            } else {
                String::new()
            },
            commands,
            folder_names: Vec::new(),
            selected: 0,
        }
    }

    /// 注入 GoFolder 命令的文件夹显示名（`(account_id, folder_id, 名称)`）。
    pub fn set_folder_names(&mut self, names: Vec<(u32, u32, String)>) {
        self.folder_names = names;
    }

    /// GoFolder 命令的显示名（找不到动态名时回落到通用名）。
    fn display_name(&self, command: &PaletteCommand) -> String {
        match command {
            PaletteCommand::GoFolder {
                account_id,
                folder_id,
            } => self
                .folder_names
                .iter()
                .find(|(acc, folder, _)| acc == account_id && folder == folder_id)
                .map(|(_, _, name)| command.label_with(name))
                .unwrap_or_else(|| command.label().to_string()),
            _ => command.label().to_string(),
        }
    }

    /// 命令是否匹配当前搜索词（空词匹配全部）。
    /// 过滤主链路使用独立的 [`filter_commands`]，此方法保留单命令语义。
    #[allow(dead_code)]
    fn command_matches(&self, command: &PaletteCommand, query: &str) -> bool {
        let name = self.display_name(command).to_lowercase();
        name.contains(&query.to_lowercase())
    }

    /// 过滤后的命令列表；查询以 `>` 开头时仅保留操作类命令，
    /// 其余按忽略大小写的名称子串匹配。
    fn filtered_commands(&self) -> Vec<PaletteCommand> {
        filter_commands(&self.commands, &self.query, &self.folder_names)
    }

    /// 执行当前高亮命令并发事件；无候选时不动。
    fn execute_selected(&mut self, cx: &mut Context<Self>) {
        let filtered = self.filtered_commands();
        if filtered.is_empty() {
            return;
        }
        let index = self.selected.min(filtered.len() - 1);
        let command = filtered[index];
        cx.emit(CommandPaletteEvent::CommandExecuted(command));
        cx.notify();
    }

    /// ↑ / ↓ 循环移动高亮；无候选时不动。
    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = self.filtered_commands().len();
        if len == 0 {
            return;
        }
        let current = self.selected.min(len - 1) as isize;
        let next = (current + delta).rem_euclid(len as isize) as usize;
        self.selected = next;
        cx.notify();
    }
}

/// 纯过滤逻辑（便于单元测试）：见 [`CommandPaletteState::filtered_commands`]。
///
/// 对齐原版 `src/ui/command_palette.rs:35-52`：将 `query.split_whitespace()`
/// 拆分为多个 token，要求每个 token 在 `title + detail` 拼接串中均命中（`tokens.all(...)`）。
fn filter_commands(
    commands: &[PaletteCommand],
    query: &str,
    folder_names: &[(u32, u32, String)],
) -> Vec<PaletteCommand> {
    let query = query.trim();
    // `>` 前缀：仅显示操作类命令。
    let (actions_only, keyword) = match query.strip_prefix('>') {
        Some(rest) => (true, rest.trim().to_lowercase()),
        None => (false, query.to_lowercase()),
    };
    let tokens: Vec<&str> = keyword.split_whitespace().collect();
    commands
        .iter()
        .copied()
        .filter(|command| !actions_only || (!command.is_folder() && !command.is_view()))
        .filter(|command| {
            tokens.is_empty() || {
                let name = match command {
                    PaletteCommand::GoFolder {
                        account_id,
                        folder_id,
                    } => folder_names
                        .iter()
                        .find(|(acc, folder, _)| acc == account_id && folder == folder_id)
                        .map(|(_, _, name)| format!("跳转到 {name}"))
                        .unwrap_or_else(|| command.label().to_string()),
                    _ => command.label().to_string(),
                };
                let haystack = format!("{name} {}", command.detail()).to_lowercase();
                tokens.iter().all(|token| haystack.contains(token))
            }
        })
        .collect()
}

impl Render for CommandPaletteState {
    fn render(
        &mut self,
        _window: &mut gpui_kit::Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // 避坑清单第 4 条：render 开头先把语义色 Copy 成局部变量。
        let p = crate::theme::current(cx);
        let (surface_raised, border, text, muted_fg, accent, accent_soft, danger) = (
            p.surface_raised,
            p.border,
            p.text,
            p.text_muted,
            p.accent,
            p.accent_soft(),
            p.danger,
        );

        let filtered = self.filtered_commands();
        let selected = if filtered.is_empty() {
            0
        } else {
            self.selected.min(filtered.len() - 1)
        };
        let is_empty_result = filtered.is_empty();

        let rows: Vec<Stateful<gpui_kit::Div>> = filtered
            .iter()
            .enumerate()
            .map(|(ix, command)| {
                let is_selected = ix == selected;
                let name = self.display_name(command);
                let hint = command
                    .shortcut()
                    .map(SharedString::from)
                    .unwrap_or_else(|| {
                        SharedString::from(if command.is_folder() {
                            "文件夹"
                        } else if command.is_view() {
                            "视图"
                        } else {
                            "操作"
                        })
                    });
                let icon = command.icon();
                let command = *command;
                h_flex()
                    .id(("palette-row", ix))
                    .h(px(36.0))
                    .mx_2()
                    .px_2()
                    .gap_2()
                    .items_center()
                    .rounded(px(8.0))
                    .cursor_pointer()
                    .when(is_selected, |row| row.bg(accent_soft))
                    .when(!is_selected, |row| row.hover(move |s| s.bg(accent_soft)))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(CommandPaletteEvent::CommandExecuted(command));
                        cx.notify();
                    }))
                    .child(
                        Icon::new(icon)
                            .with_size(px(15.0))
                            .text_color(if is_selected { accent } else { muted_fg }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .text_color(if is_selected { accent } else { text })
                            .truncate()
                            .child(SharedString::from(name)),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(muted_fg)
                            .child(hint),
                    )
            })
            .collect();

        // 遮罩：点击关闭（卡片内部 mousedown 阻止冒泡，避免误关）。
        let backdrop = div()
            .absolute()
            .top(px(0.0))
            .left(px(0.0))
            .size_full()
            .bg(black().opacity(0.4))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| {
                    cx.emit(CommandPaletteEvent::Closed);
                }),
            );

        // 居中卡片 560x440：搜索框 + 滚动结果列表。
        let card = v_flex()
            .id("palette-card")
            .w(px(560.0))
            .h(px(440.0))
            .bg(surface_raised)
            .rounded(px(12.0))
            .border_1()
            .border_color(border)
            .shadow_xl()
            .overflow_hidden()
            // 吃掉卡片内部的鼠标按下事件，避免点击行 / 输入框时冒泡到遮罩。
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| {
                    cx.stop_propagation();
                }),
            )
            // ↑ / ↓ 移动高亮、Esc 关闭：搜索框聚焦时按键冒泡到此处。
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "up" => {
                        this.move_selection(-1, cx);
                        window.prevent_default();
                        cx.stop_propagation();
                    }
                    "down" => {
                        this.move_selection(1, cx);
                        window.prevent_default();
                        cx.stop_propagation();
                    }
                    "escape" => {
                        cx.emit(CommandPaletteEvent::Closed);
                        window.prevent_default();
                        cx.stop_propagation();
                    }
                    _ => {}
                }
            }))
            .child(
                h_flex()
                    .flex_shrink_0()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(border)
                    .child(
                        Icon::new(IconName::Search)
                            .with_size(px(16.0))
                            .text_color(muted_fg),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&self.search_input)),
                    ),
            )
            .child(
                v_flex()
                    .id("palette-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .py_1()
                    .when(is_empty_result, |el| {
                        el.child(
                            v_flex()
                                .flex_1()
                                .items_center()
                                .justify_center()
                                .gap_1()
                                .child(
                                    Icon::new(IconName::Search)
                                        .with_size(px(22.0))
                                        .text_color(muted_fg),
                                )
                                .child(div().text_sm().text_color(muted_fg).child(
                                    if self.query.starts_with('>') {
                                        "没有匹配的操作"
                                    } else {
                                        "没有匹配的命令"
                                    },
                                )),
                        )
                    })
                    .children(rows),
            )
            .child(
                h_flex()
                    .flex_shrink_0()
                    .px_3()
                    .py_1p5()
                    .gap_2()
                    .items_center()
                    .justify_between()
                    .border_t_1()
                    .border_color(border)
                    .child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .child(
                                Icon::new(IconName::Keyboard)
                                    .with_size(px(12.0))
                                    .text_color(muted_fg),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(muted_fg)
                                    .child("↑↓ 选择  Enter 执行  Esc 关闭  > 仅操作"),
                            ),
                    )
                    .child(
                        h_flex().gap_1().items_center().child(
                            div()
                                .text_xs()
                                .text_color(if is_empty_result { danger } else { muted_fg })
                                .child(SharedString::from(format!(
                                    "{} / {}",
                                    if is_empty_result { 0 } else { selected + 1 },
                                    filtered.len()
                                ))),
                        ),
                    ),
            );

        backdrop.child(
            h_flex()
                .size_full()
                .items_center()
                .justify_center()
                .child(card),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_commands() -> Vec<PaletteCommand> {
        vec![
            PaletteCommand::NewCompose,
            PaletteCommand::RefreshAll,
            PaletteCommand::ToggleUnifiedInbox,
            PaletteCommand::OpenAccountSettings,
            PaletteCommand::OpenPreferences,
            PaletteCommand::ToggleTheme,
            PaletteCommand::GoFolder {
                account_id: 1,
                folder_id: 7,
            },
        ]
    }

    #[test]
    fn labels_are_visible_and_typed() {
        assert_eq!(PaletteCommand::NewCompose.label(), "写新邮件");
        assert_eq!(PaletteCommand::ToggleTheme.label(), "切换亮暗主题");
        assert_eq!(
            PaletteCommand::GoFolder {
                account_id: 1,
                folder_id: 7
            }
            .label(),
            "跳转到文件夹"
        );
        assert_eq!(
            PaletteCommand::GoFolder {
                account_id: 1,
                folder_id: 7
            }
            .label_with("收件箱"),
            "跳转到 收件箱"
        );
        assert_eq!(PaletteCommand::NewCompose.label_with("收件箱"), "写新邮件");
        assert!(PaletteCommand::GoFolder {
            account_id: 1,
            folder_id: 7
        }
        .is_folder());
        assert!(!PaletteCommand::RefreshAll.is_folder());
    }

    #[test]
    fn filter_empty_query_returns_all() {
        let result = filter_commands(&all_commands(), "", &[]);
        assert_eq!(result.len(), 7);
    }

    #[test]
    fn filter_substring_is_case_insensitive() {
        // 中文子串。
        let result = filter_commands(&all_commands(), "刷新", &[]);
        assert_eq!(result, vec![PaletteCommand::RefreshAll]);
        // 大小写不敏感（对含 ASCII 的显示名）。
        let result = filter_commands(&all_commands(), "CTRL", &[]);
        assert!(result.is_empty(), "按显示名过滤而非快捷键");
        // 无匹配。
        assert!(filter_commands(&all_commands(), "zzz", &[]).is_empty());
    }

    #[test]
    fn filter_gt_prefix_shows_actions_only() {
        // `>` 前缀 + 空关键词：仅操作类，排除文件夹跳转。
        let result = filter_commands(&all_commands(), ">", &[]);
        assert_eq!(result.len(), 6);
        assert!(result.iter().all(|c| !c.is_folder()));

        // `>` 前缀 + 关键词：在操作类里继续子串过滤。
        let result = filter_commands(&all_commands(), "> 刷新", &[]);
        assert_eq!(result, vec![PaletteCommand::RefreshAll]);

        // `>` 前缀下文件夹动态名不参与匹配：命中「收件箱」的只有
        // 「切换统一收件箱」这个操作，GoFolder（动态名收件箱）被排除。
        let folders = vec![(1u32, 7u32, "收件箱".to_string())];
        let result = filter_commands(&all_commands(), "> 收件箱", &folders);
        assert_eq!(result, vec![PaletteCommand::ToggleUnifiedInbox]);
    }

    #[test]
    fn filter_matches_folder_display_name() {
        let folders = vec![(1u32, 7u32, "收件箱".to_string())];
        let result = filter_commands(&all_commands(), "收件箱", &folders);
        assert_eq!(
            result,
            vec![
                PaletteCommand::ToggleUnifiedInbox,
                PaletteCommand::GoFolder {
                    account_id: 1,
                    folder_id: 7
                },
            ]
        );
        // 没有动态名时回落到通用名。
        let result = filter_commands(&all_commands(), "跳转到", &[]);
        assert_eq!(
            result,
            vec![PaletteCommand::GoFolder {
                account_id: 1,
                folder_id: 7
            }]
        );
    }

    /// 含视图跳转与「对当前邮件操作」命令的扩充命令集（阶段二）。
    fn extended_commands() -> Vec<PaletteCommand> {
        let mut commands = all_commands();
        commands.push(PaletteCommand::GoStarred);
        commands.push(PaletteCommand::GoSent);
        commands.push(PaletteCommand::GoDrafts);
        commands.push(PaletteCommand::GoArchive);
        commands.push(PaletteCommand::OpenContacts);
        commands.push(PaletteCommand::OpenGallery);
        commands.push(PaletteCommand::ReplySelected);
        commands.push(PaletteCommand::ArchiveSelected);
        commands.push(PaletteCommand::DeleteSelected);
        commands.push(PaletteCommand::MarkUnreadSelected);
        commands.push(PaletteCommand::ToggleStarSelected);
        commands
    }

    #[test]
    fn gt_prefix_excludes_view_navigation_but_keeps_message_actions() {
        // `>` 仅操作：视图跳转（GoStarred / OpenContacts 等）与文件夹一并排除，
        // 对当前邮件的操作保留（对齐原版 action: true / go-to 语义）。
        let result = filter_commands(&extended_commands(), ">", &[]);
        assert!(result.contains(&PaletteCommand::ReplySelected));
        assert!(result.contains(&PaletteCommand::ToggleStarSelected));
        assert!(result.iter().all(|c| !c.is_folder() && !c.is_view()));

        // 无 `>` 时视图跳转与联系人 / 附件入口正常参与匹配。
        assert!(filter_commands(&extended_commands(), "联系人", &[])
            .contains(&PaletteCommand::OpenContacts));
        assert!(filter_commands(&extended_commands(), "已加星标", &[])
            .contains(&PaletteCommand::GoStarred));
        assert!(filter_commands(&extended_commands(), "回复", &[])
            .contains(&PaletteCommand::ReplySelected));
    }

    #[test]
    fn message_action_commands_are_labeled_for_selection() {
        assert_eq!(PaletteCommand::ReplySelected.label(), "回复当前邮件");
        assert_eq!(PaletteCommand::ArchiveSelected.label(), "归档当前邮件");
        assert_eq!(PaletteCommand::DeleteSelected.label(), "删除当前邮件");
        assert_eq!(
            PaletteCommand::MarkUnreadSelected.label(),
            "标当前邮件为未读"
        );
        assert_eq!(
            PaletteCommand::ToggleStarSelected.label(),
            "切换当前邮件星标"
        );
        assert!(!PaletteCommand::ReplySelected.is_view());
        assert!(PaletteCommand::OpenGallery.is_view());
    }

    #[test]
    fn filter_multi_token_matches_title_and_detail() {
        let folders = vec![(1u32, 7u32, "收件箱".to_string())];
        let result = filter_commands(&extended_commands(), "dark theme", &folders);
        assert_eq!(result, vec![PaletteCommand::ToggleTheme]);

        let result = filter_commands(&extended_commands(), "移动 收件箱", &folders);
        assert_eq!(
            result,
            vec![PaletteCommand::GoFolder {
                account_id: 1,
                folder_id: 7
            }]
        );

        let result = filter_commands(&extended_commands(), "> 星标 切换", &folders);
        assert_eq!(result, vec![PaletteCommand::ToggleStarSelected]);
    }
}
