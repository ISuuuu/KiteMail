/**
 * 设置窗左栏导航：22 个页签（账号 / 设置两组）、栏宽与窄窗断点。
 * 页签名称、顺序、分组严格对齐 GPUI `PrefTab::ALL`（crates/app/src/ui/preferences/mod.rs#L63），
 * 栏宽对齐 `compute_sidebar_width`（27% 钳制 180–250px），窄窗断点 640px。
 */

export type SettingsTabGroup = "accounts" | "settings";

export type SettingsTabId =
  | "mailAccounts"
  | "tags"
  | "filters"
  | "smartFolders"
  | "senders"
  | "openPgp"
  | "cloudStorage"
  | "translation"
  | "contactsSources"
  | "general"
  | "appearance"
  | "focusMode"
  | "sidebar"
  | "messageList"
  | "conversations"
  | "reading"
  | "composing"
  | "privacy"
  | "mcp"
  | "dateTime"
  | "system"
  | "backup";

export interface SettingsTabMeta {
  id: SettingsTabId;
  label: string;
  icon: string;
  group: SettingsTabGroup;
  /** 内容顶栏下的页说明行（GPUI `PrefTab::subtitle`）。 */
  subtitle: string;
}

export const SETTINGS_TABS: readonly SettingsTabMeta[] = [
  // 账号组（9 项）
  {
    id: "mailAccounts",
    label: "邮箱账号",
    icon: "circle-user",
    group: "accounts",
    subtitle: "添加与管理 IMAP / SMTP / Outlook OAuth2 邮箱账号",
  },
  {
    id: "tags",
    label: "标签",
    icon: "tag",
    group: "accounts",
    subtitle: "增删改自定义邮件标签、调色板颜色与快捷键 1..9 绑定",
  },
  {
    id: "filters",
    label: "过滤器",
    icon: "search",
    group: "accounts",
    subtitle: "创建自动化邮件过滤规则，并在现有缓存邮件上立即执行",
  },
  {
    id: "smartFolders",
    label: "智能文件夹",
    icon: "folder",
    group: "accounts",
    subtitle: "基于搜索表达式创建动态虚拟文件夹",
  },
  {
    id: "senders",
    label: "发件人",
    icon: "users",
    group: "accounts",
    subtitle: "管理发件人黑名单",
  },
  {
    id: "openPgp",
    label: "OpenPGP 密钥",
    icon: "shield",
    group: "accounts",
    subtitle: "管理本地 OpenPGP 密钥信息与按邮箱绑定的签名/加密策略",
  },
  {
    id: "cloudStorage",
    label: "云存储",
    icon: "upload",
    group: "accounts",
    subtitle: "配置超大附件云盘账号（Nextcloud / WebDAV / S3 / Dropbox）与自动上传阈值",
  },
  {
    id: "translation",
    label: "翻译",
    icon: "globe",
    group: "accounts",
    subtitle: "配置阅读器与写信器一键翻译引擎及目标语言",
  },
  {
    id: "contactsSources",
    label: "通讯录源",
    icon: "book-open",
    group: "accounts",
    subtitle: "管理 CardDAV 远程通讯录同步源与自动收集发件人开关",
  },
  // 设置组（13 项）
  {
    id: "general",
    label: "常规",
    icon: "settings",
    group: "settings",
    subtitle: "系统托盘、启动默认视图、自动同步与通知提示",
  },
  {
    id: "appearance",
    label: "外观",
    icon: "palette",
    group: "settings",
    subtitle: "明暗模式、主题配色、界面文字缩放与状态栏位置",
  },
  {
    id: "focusMode",
    label: "专注模式",
    icon: "eye",
    group: "settings",
    subtitle: "沉浸阅读时自动调整边栏、列表、简化视图及工具栏",
  },
  {
    id: "sidebar",
    label: "侧边栏",
    icon: "panel-left",
    group: "settings",
    subtitle: "控制导航区段与统一文件夹入口的显示及折叠记忆",
  },
  {
    id: "messageList",
    label: "邮件列表",
    icon: "list",
    group: "settings",
    subtitle: "紧凑列表、头像、日期分组及搜索 / 批量操作栏位置",
  },
  {
    id: "conversations",
    label: "会话",
    icon: "book-open",
    group: "settings",
    subtitle: "对话线程排序方向、自动折叠策略与卡片附件",
  },
  {
    id: "reading",
    label: "阅读",
    icon: "mail-open",
    group: "settings",
    subtitle: "自动标为已读时机、阅读器布局位置、信纸主题与顶栏开关",
  },
  {
    id: "composing",
    label: "写信",
    icon: "pencil",
    group: "settings",
    subtitle: "默认发件人、签名位置、撤销发送秒数、回复头与附件检测",
  },
  {
    id: "privacy",
    label: "隐私",
    icon: "ban",
    group: "settings",
    subtitle: "默认拦截外部图片、提示横幅及链接追踪参数清理",
  },
  {
    id: "mcp",
    label: "MCP 服务",
    icon: "bot",
    group: "settings",
    subtitle: "管理本地 MCP Server 开关、命名管道路径与 AI 工具权限",
  },
  {
    id: "dateTime",
    label: "日期和时间",
    icon: "info",
    group: "settings",
    subtitle: "统一配置日期年月顺序与 12 / 24 小时时间格式",
  },
  {
    id: "system",
    label: "系统",
    icon: "hard-drive",
    group: "settings",
    subtitle: "查看本地配置/缓存根目录、清理缓存与版本引擎信息",
  },
  {
    id: "backup",
    label: "备份",
    icon: "archive",
    group: "settings",
    subtitle: "导出或导入便携 TOML 配置与账号服务器参数备份",
  },
];

export const SETTINGS_TAB_GROUPS: ReadonlyArray<{
  id: SettingsTabGroup;
  label: string;
}> = [
  { id: "accounts", label: "账号" },
  { id: "settings", label: "设置" },
];

export const NARROW_BREAKPOINT = 640;
export const SIDEBAR_MIN_WIDTH = 180;
export const SIDEBAR_MAX_WIDTH = 250;
/** 窗口默认尺寸下 920 × 27% ≈ 248.4px。 */
export const SIDEBAR_WIDTH_RATIO = 0.27;

/** 左栏宽度：窗口总宽的 27%，钳制到 [180, 250]（GPUI compute_sidebar_width）。 */
export function computeSidebarWidth(windowWidth: number): number {
  const raw = windowWidth * SIDEBAR_WIDTH_RATIO;
  return Math.min(SIDEBAR_MAX_WIDTH, Math.max(SIDEBAR_MIN_WIDTH, raw));
}

/** 窄窗判定：客户区宽度 ≤ 640px 切换为全宽导航 / 全宽内容。 */
export function isNarrowViewport(windowWidth: number): boolean {
  return windowWidth <= NARROW_BREAKPOINT;
}

/** 打开设置窗时的默认页签（对齐 GPUI `open_preferences` → 默认「常规」）。 */
export const DEFAULT_SETTINGS_TAB: SettingsTabId = "general";

export function findSettingsTab(id: string): SettingsTabMeta {
  return (
    SETTINGS_TABS.find((tab) => tab.id === id) ??
    SETTINGS_TABS.find((tab) => tab.id === DEFAULT_SETTINGS_TAB)!
  );
}

/** 按 GPUI 分组顺序返回该组页签。 */
export function tabsOfGroup(group: SettingsTabGroup): SettingsTabMeta[] {
  return SETTINGS_TABS.filter((tab) => tab.group === group);
}
