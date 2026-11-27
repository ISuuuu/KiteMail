/**
 * 设置窗搜索：索引数据 + 查询评分排序（纯逻辑，可在 Node 测试中验证）。
 *
 * 排序镜像 GPUI crates/app/src/ui/preferences/search.rs#L714：
 * 标题前缀命中 4 > 标题包含 3 > 页签名包含 2 > 关键词包含 1，
 * 同分按索引顺序，最多返回 60 条。
 *
 * 定位：anchorId 为目标设置行的 DOM id；null 表示本轮仅支持「切页不滚动」
 * （并行代理正在开发的云存储/翻译/通讯录源三页，锚点留待 WP10 补齐）。
 * 高亮样式与时长见 HIGHLIGHT_CLASS / HIGHLIGHT_MS（对齐 widgets.rs#L113 / SEARCH_HIGHLIGHT_MS）。
 */

import { SETTINGS_TABS, type SettingsTabId } from "./nav";

export interface PrefSearchEntry {
  /** 稳定 id（测试与去重用） */
  id: string;
  tabId: SettingsTabId;
  /** 结果第一行：设置名 */
  title: string;
  /** 目标行 DOM 锚点；null = 仅切页 */
  anchorId: string | null;
  /** 空格分隔的检索关键词（含英文别名） */
  keywords: string;
}

export const HIGHLIGHT_MS = 1600;
export const HIGHLIGHT_CLASS = "km-row-hit";
/** 账号列表卡片级高亮（AccountsPage 列表层） */
export const ACCOUNT_CARD_HIGHLIGHT_CLASS = "km-account-card-hit";
export const SEARCH_RESULT_CAP = 60;

function e(
  id: string,
  tabId: SettingsTabId,
  title: string,
  anchorId: string | null,
  keywords: string
): PrefSearchEntry {
  return { id, tabId, title, anchorId, keywords };
}

export const SEARCH_ENTRIES: readonly PrefSearchEntry[] = [
  // ---- 邮箱账号（列表 / 编辑关键项；卡片级高亮） ----
  e("acct-add", "mailAccounts", "添加邮箱账号", "sp-account-list", "account add new 账号 添加 新建 邮箱"),
  e("acct-list", "mailAccounts", "邮箱账号列表", "sp-account-list", "account list imap smtp 账号 列表 管理"),
  e("acct-enabled", "mailAccounts", "启用或停用邮箱账号", "sp-account-list", "account enable disable 启用 停用 停用账号"),
  // ---- 常规 ----
  e("general-page-size", "general", "单页加载邮件数", "sp-page-size", "page size 分页 每页 邮件数"),
  e("general-auto-sync", "general", "自动后台同步间隔", "sp-auto-sync", "sync interval 同步 轮询 间隔 分钟"),
  e("general-push", "general", "启用实时推送（IMAP IDLE / 增量脉冲）", "sp-push", "push idle 实时 推送 增量"),
  e("general-metered", "general", "按流量计费连接省流", "sp-metered", "metered 流量 省流 计费"),
  e("general-desktop-notif", "general", "新邮件桌面通知", "sp-desktop-notifications", "notification desktop 通知 桌面 新邮件"),
  e("general-notif-content", "general", "在桌面通知中显示发件人与主题", "sp-notification-content", "notification content 隐私 通知 内容"),
  e("general-notif-sound", "general", "新邮件提示音", "sp-notification-sound", "sound 提示音 声音 新邮件"),
  e("general-sound-choice", "general", "提示音选择", "sp-notification-sound-choice", "sound click hum string swing 提示音 选择"),
  e("general-send-sound", "general", "发送邮件提示音", "sp-send-sound", "send sound 发送 提示音"),
  ...["mark_read", "archive", "delete", "reply", "forward", "spam"].map((key, idx) =>
    e(
      `general-notify-${key}`,
      "general",
      `通知操作按钮：${["标记为已读", "归档", "删除", "回复", "转发", "标记为垃圾邮件"][idx]}`,
      `sp-notify-${key}`,
      "notification buttons 通知 按钮 操作"
    )
  ),
  // ---- 外观 ----
  e("appearance-dark-mode", "appearance", "外观模式（跟随系统 / 浅色 / 深色）", "sp-dark-mode", "dark light theme mode 深色 浅色 外观 明暗"),
  e("appearance-theme", "appearance", "主题配色", "sp-theme-gallery", "theme palette color 主题 配色 经典蓝 子夜 海潮 玫瑰 大地 森林"),
  e("appearance-app-icon", "appearance", "应用图标", "sp-app-icon", "icon app 图标 应用"),
  e("appearance-text-scale", "appearance", "界面字体缩放", "sp-ui-text-scale", "font scale zoom 字体 缩放 字号"),
  e("appearance-open-accounts", "appearance", "打开偏好设置时优先显示「邮箱账号」", "sp-settings-open-accounts", "open accounts default tab 首选 页 账号"),
  e("appearance-toolbar-fold", "appearance", "始终折叠右侧工具栏辅助按钮", "sp-tb-fold", "toolbar fold 工具栏 折叠"),
  ...["reply", "replyAll", "forward", "star", "archive", "snooze", "delete", "spam", "readUnread", "tags", "moveTo", "find", "print"].map(
    (key) =>
      e(
        `appearance-tb-${key}`,
        "appearance",
        `阅读器工具栏：${key}`,
        `sp-tb-${key}`,
        "toolbar reader 工具栏 按钮 分区"
      )
  ),
  e("appearance-toolbar-restore", "appearance", "恢复默认工具栏布局", "sp-toolbar-restore", "toolbar restore 工具栏 恢复 默认"),
  // ---- 专注模式 ----
  e("focus-enabled", "focusMode", "启用专注模式", "sp-focus-enabled", "focus mode 专注 沉浸"),
  e("focus-start", "focusMode", "启动时自动进入专注模式", "sp-focusStartEnabled", "focus startup 专注 启动"),
  e("focus-rail", "focusMode", "折叠侧边栏为图标栏", "sp-focusRailSidebar", "focus rail sidebar 图标栏 折叠"),
  e("focus-unread-counts", "focusMode", "统一视图行显示未读计数", "sp-focusUnreadCounts", "focus unread count 未读 计数"),
  e("focus-rail-dots", "focusMode", "图标栏以未读圆点代替计数", "sp-focusRailDots", "focus dots 圆点 图标栏"),
  e("focus-hover-expand", "focusMode", "悬停图标栏时滑出完整侧边栏", "sp-focusHoverExpand", "focus hover expand 悬停 展开"),
  e("focus-hide-accounts", "focusMode", "隐藏账号分区", "sp-focusHideAccounts", "focus hide accounts 隐藏 账号"),
  e("focus-fold-accounts", "focusMode", "折叠账号为头像", "sp-focusFoldAccounts", "focus fold avatar 折叠 头像"),
  e("focus-click-folds", "focusMode", "单击账号即折叠", "sp-focusClickFolds", "focus click fold 单击 折叠"),
  e("focus-fold-unified", "focusMode", "折叠统一视图行", "sp-focusFoldUnified", "focus fold unified 折叠 统一"),
  e("focus-hide-sidebar", "focusMode", "隐藏整个侧边栏（Windows 行为）", "sp-focusHideSidebar", "focus hide sidebar 隐藏 侧边栏"),
  e("focus-inboxes", "focusMode", "统一收件箱", "sp-focusShowInboxes", "focus inbox 统一 收件箱"),
  e("focus-starred", "focusMode", "已加星标", "sp-focusUnifiedStarred", "focus starred 星标"),
  e("focus-sent", "focusMode", "已发送", "sp-focusUnifiedSent", "focus sent 已发送"),
  e("focus-drafts", "focusMode", "草稿箱", "sp-focusUnifiedDrafts", "focus drafts 草稿"),
  e("focus-archive", "focusMode", "已归档", "sp-focusUnifiedArchive", "focus archive 归档"),
  e("focus-junk", "focusMode", "垃圾邮件", "sp-focusUnifiedJunk", "focus junk spam 垃圾"),
  e("focus-trash", "focusMode", "废纸篓", "sp-focusUnifiedTrash", "focus trash 废纸篓"),
  e("focus-snoozed", "focusMode", "稍后处理", "sp-focusUnifiedSnoozed", "focus snoozed 稍后"),
  e("focus-favorites", "focusMode", "收藏", "sp-focusShowFavorites", "focus favorites 收藏"),
  e("focus-filters", "focusMode", "筛选文件夹", "sp-focusShowFilters", "focus filters 筛选"),
  e("focus-tags", "focusMode", "标签", "sp-focusShowTags", "focus tags 标签"),
  e("focus-contacts", "focusMode", "联系人", "sp-focusShowContacts", "focus contacts 联系人"),
  e("focus-attachments", "focusMode", "附件", "sp-focusShowAttachments", "focus attachments 附件"),
  e("focus-layout", "focusMode", "专注模式布局", "sp-focus-layout", "focus layout cards single 布局 卡片 单行"),
  e("focus-column-headings", "focusMode", "显示列标题", "sp-focusColumnHeadings", "focus column headings 列 标题"),
  e("focus-preview-lines", "focusMode", "摘要行数", "sp-focus-preview-lines", "focus preview lines 摘要 行数"),
  e("focus-list-header", "focusMode", "折叠列表头部", "sp-focusListHeader", "focus list header 列表 头部 折叠"),
  e("focus-hide-avatars", "focusMode", "隐藏发件人头像", "sp-focusHideAvatars", "focus hide avatars 头像"),
  e("focus-hide-subject", "focusMode", "隐藏主题", "sp-focusHideSubject", "focus hide subject 主题"),
  e("focus-tags-below", "focusMode", "标签置于文本下方", "sp-focusTagsBelow", "focus tags below 标签 下方"),
  e("focus-hide-stars", "focusMode", "隐藏星标", "sp-focusHideStars", "focus hide stars 星标"),
  e("focus-hide-attachment-icons", "focusMode", "隐藏附件图标", "sp-focusHideAttachmentIcons", "focus attachment icons 附件 图标"),
  e("focus-hide-list", "focusMode", "隐藏中间邮件列表（Windows 行为）", "sp-focusHideList", "focus hide list 隐藏 列表"),
  e("focus-fold-toolbar", "focusMode", "折叠阅读器工具栏", "sp-focusFoldToolbar", "focus fold toolbar 工具栏"),
  e("focus-reader-view", "focusMode", "简化阅读视图", "sp-focusReaderView", "focus reader view 简化 阅读"),
  // ---- 侧边栏 ----
  e("sidebar-default-view", "sidebar", "启动时默认打开视图", "sp-default-view", "default view startup 启动 视图"),
  e("sidebar-show-accounts", "sidebar", "显示账号与文件夹列表", "sp-showAccounts", "sidebar accounts 侧边栏 账号 文件夹"),
  e("sidebar-contacts", "sidebar", "显示「联系人」入口", "sp-sidebarShowContacts", "sidebar contacts 联系人 入口"),
  e("sidebar-attachments", "sidebar", "显示「附件总览」入口", "sp-sidebarShowAttachments", "sidebar attachments 附件 入口"),
  e("sidebar-remember", "sidebar", "记住侧边栏折叠状态", "sp-rememberSidebarCollapsed", "sidebar remember collapse 记住 折叠"),
  e("sidebar-rail-dots", "sidebar", "窄栏模式下用未读圆点代替数字计数", "sp-railUnreadDots", "rail dots unread 圆点 计数"),
  e("sidebar-unified-inbox", "sidebar", "显示「统一收件箱」", "sp-unifiedShowInbox", "unified inbox 统一 收件箱"),
  e("sidebar-unified-unread", "sidebar", "显示「未读」", "sp-unifiedShowUnread", "unified unread 未读"),
  e("sidebar-unified-starred", "sidebar", "显示「已加星标」", "sp-unifiedShowStarred", "unified starred 星标"),
  e("sidebar-unified-sent", "sidebar", "显示「已发送」", "sp-unifiedShowSent", "unified sent 已发送"),
  e("sidebar-unified-drafts", "sidebar", "显示「草稿箱」", "sp-unifiedShowDrafts", "unified drafts 草稿"),
  e("sidebar-unified-archive", "sidebar", "显示「已归档」", "sp-unifiedShowArchive", "unified archive 归档"),
  e("sidebar-unified-junk", "sidebar", "显示「垃圾邮件」", "sp-unifiedShowJunk", "unified junk 垃圾"),
  e("sidebar-unified-trash", "sidebar", "显示「废纸篓」", "sp-unifiedShowTrash", "unified trash 废纸篓"),
  e("sidebar-unified-snoozed", "sidebar", "显示「稍后处理」", "sp-unifiedShowSnoozed", "unified snoozed 稍后"),
  e("sidebar-filtered", "sidebar", "显示「筛选文件夹」分区", "sp-sidebarShowFiltered", "filtered section 筛选 分区"),
  e("sidebar-smart", "sidebar", "显示「智能文件夹」分区", "sp-sidebarShowSmart", "smart section 智能 分区"),
  e("sidebar-tags", "sidebar", "显示「标签」分区", "sp-sidebarShowTags", "tags section 标签 分区"),
  // ---- 邮件列表 ----
  e("list-layout", "messageList", "邮件列表布局", "sp-list-layout", "list layout wide cards 布局 宽表 卡片"),
  e("list-preview-lines", "messageList", "摘要行数", "sp-list-preview-lines", "preview lines 摘要 行数"),
  e("list-compact", "messageList", "紧凑列表行", "sp-compactList", "compact list 紧凑 行"),
  e("list-avatars", "messageList", "显示发件人头像", "sp-showAvatars", "avatars 头像"),
  e("list-date-group", "messageList", "按日期分组显示", "sp-listGroupByDate", "date group 日期 分组"),
  e("list-search-always", "messageList", "始终显示搜索栏", "sp-listSearchAlwaysOpen", "search bar always 搜索栏 常显"),
  e("list-search-bottom", "messageList", "搜索栏放在列表底部", "sp-listSearchBottom", "search bottom 搜索栏 底部"),
  e("list-bulk-bottom", "messageList", "多选批量操作栏放在列表底部", "sp-listBulkBarBottom", "bulk bar bottom 批量 底部"),
  e("list-show-headings", "messageList", "显示列标题栏", "sp-listShowHeadings", "column headings 列 标题"),
  ...["star", "replied", "sender", "recipients", "correspondents", "subject", "tags", "attachment", "importance", "account", "due", "date"].map(
    (key) =>
      e(
        `list-col-${key}`,
        "messageList",
        `列顺序与显隐：${key}`,
        `sp-col-${key}`,
        "list column order 列 顺序 显隐 排序"
      )
  ),
  e("list-columns-restore", "messageList", "恢复默认列", "sp-columns-restore", "columns restore 列 恢复 默认"),
  // ---- 会话 ----
  e("conversations-newest", "conversations", "按最新回复置顶排列会话卡片", "sp-threadNewestFirst", "thread newest first 会话 排序 最新"),
  e("conversations-fold-mode", "conversations", "自动折叠多封会话卡片", "sp-fold-mode", "fold messages collapse 折叠 会话"),
  e("conversations-card-attachments", "conversations", "在各会话卡片中展示附件", "sp-card-attachments", "card attachments 卡片 附件"),
  // ---- 阅读 ----
  e("reading-read-mark", "reading", "自动标记为已读时机", "sp-readMarkMode", "read mark 已读 标记 时机"),
  e("reading-position", "reading", "阅读器窗格位置", "sp-readerPosition", "reader position 阅读器 位置 右侧 下方"),
  e("reading-single-card", "reading", "单封邮件显示卡片框线", "sp-singleMessageCard", "single card border 卡片 框线"),
  e("reading-message-theme", "reading", "默认信纸主题", "sp-messageTheme", "letterhead theme 信纸 主题"),
  e("reading-zoom", "reading", "阅读器正文字号", "sp-reader-zoom", "reader zoom 正文 字号 缩放"),
  e("reading-recipients", "reading", "始终显示收件人行", "sp-alwaysShowRecipients", "recipients row 收件人 行"),
  e("reading-account-chip", "reading", "顶栏显示账号邮箱标签", "sp-readerShowAccountChip", "reader topbar account chip 顶栏 账号"),
  e("reading-theme-switch", "reading", "顶栏显示明暗背景切换按钮", "sp-readerShowThemeSwitch", "reader topbar theme switch 顶栏 明暗"),
  e("reading-view-switch", "reading", "顶栏显示简化阅读视图切换按钮", "sp-readerShowViewSwitch", "reader topbar view switch 顶栏 简化"),
  e("reading-default-mode", "reading", "打开邮件时默认视图", "sp-readerDefaultMode", "reader default view 邮件 默认 视图"),
  // ---- 写信 ----
  e("composing-default-from", "composing", "新邮件默认发件账号", "sp-compose-default-from", "compose from default 发件 默认 账号"),
  e("composing-inline", "composing", "在主窗口中新建邮件", "sp-compose-inline", "compose inline window 新建 主窗口"),
  e("composing-reply-inline", "composing", "在主窗口中回复与转发", "sp-reply-inline", "reply forward inline 回复 转发 主窗口"),
  e("composing-sig-position", "composing", "回复与转发时的签名位置", "sp-signature-position", "signature position 签名 位置"),
  e("composing-sig-dashes", "composing", "签名前插入 \"-- \" 分隔符", "sp-signatureDashes", "signature dashes 签名 分隔符"),
  e("composing-undo-send", "composing", "撤销发送缓冲时间", "sp-undo-send", "undo send 撤销 发送 缓冲"),
  e("composing-reply-header", "composing", "自定义回复引用头模板", "sp-reply-header", "reply header template 引用头 模板"),
  e("composing-attach-keywords", "composing", "附件遗漏检测关键词", "sp-attach-keywords", "attachment reminder keywords 附件 遗漏 关键词"),
  // ---- 隐私 ----
  e("privacy-lock-status", "privacy", "锁屏密码", "sp-app-lock-status", "lock passcode password 锁屏 密码 密码锁"),
  e("privacy-lock-idle", "privacy", "无操作自动锁定", "sp-app-lock-idle", "lock idle 自动锁定 空闲"),
  e("privacy-lock-background", "privacy", "切到后台或最小化后自动锁定", "sp-app-lock-background", "lock background 后台 锁定 最小化"),
  e("privacy-lock-system", "privacy", "系统锁屏或休眠时立即锁定", "sp-app-lock-system", "lock system sleep 系统锁屏 休眠"),
  e("privacy-block-remote", "privacy", "默认屏蔽远程图像", "sp-blockRemoteImages", "remote images block 远程 图像 屏蔽"),
  e("privacy-remote-banner", "privacy", "显示已屏蔽远程图像提示横幅", "sp-showRemoteBanner", "remote banner 横幅 提示"),
  e("privacy-clean-links", "privacy", "自动剥离链接中的追踪参数", "sp-cleanLinks", "clean links tracker 追踪 参数 链接"),
  e("privacy-spoof", "privacy", "警告仿冒域名发件人", "sp-showSpoofBanner", "spoof phishing 仿冒 钓鱼 警告"),
  e("privacy-gravatar", "privacy", "加载 Gravatar 头像", "sp-useGravatar", "gravatar avatar 头像"),
  e("privacy-sender-logos", "privacy", "加载公司徽标", "sp-senderLogos", "sender logos 徽标 公司"),
  e("privacy-read-receipt", "privacy", "发送已读回执", "sp-read-receipt", "read receipt 回执"),
  // ---- MCP 服务 ----
  e("mcp-enabled", "mcp", "启用本地 MCP Server", "sp-mcp-enabled", "mcp server ai enable 启用"),
  e("mcp-socket", "mcp", "MCP 命名管道路径", "sp-mcp-socket", "mcp pipe socket path 管道 路径"),
  e("mcp-read", "mcp", "允许读取邮件内容", "sp-mcpAllowRead", "mcp read permission 读取 权限"),
  e("mcp-search", "mcp", "允许搜索邮件", "sp-mcpAllowSearch", "mcp search permission 搜索 权限"),
  e("mcp-draft", "mcp", "允许创建草稿", "sp-mcpAllowDraft", "mcp draft permission 草稿 权限"),
  e("mcp-flags", "mcp", "允许标记已读 / 星标 / 标签", "sp-mcpAllowModifyFlags", "mcp flags permission 标记 权限"),
  // ---- 日期和时间 ----
  e("datetime-preview", "dateTime", "当前格式实时预览", "sp-datetime-preview", "datetime preview 预览 日期 时间"),
  e("datetime-date-style", "dateTime", "日期排列顺序", "sp-date-style", "date style order 日期 顺序"),
  e("datetime-clock-style", "dateTime", "时间制式", "sp-clock-style", "clock style 12h 24h 时间 制式"),
  // ---- 系统 ----
  e("system-autostart", "system", "登录 Windows 时自动启动", "sp-startAtLogin", "autostart login 自启动 登录"),
  e("system-link-open", "system", "邮件内链接打开方式", "sp-linkOpenMode", "link open browser 链接 浏览器"),
  e("system-close-tray", "system", "关闭主窗口时最小化到托盘", "sp-closeToMinimize", "close tray minimize 关闭 托盘"),
  e("system-min-tray", "system", "最小化主窗口时隐藏到托盘", "sp-minimizeToTray", "minimize tray 最小化 托盘"),
  e("system-fullscreen", "system", "启动时进入全屏模式", "sp-startFullscreen", "fullscreen startup 全屏"),
  e("system-single-key", "system", "启用单键快捷键", "sp-singleKeyShortcuts", "single key shortcuts 单键 快捷键"),
  e("system-status-bar", "system", "将状态栏置于窗口顶部", "sp-statusBarTop", "status bar top 状态栏 顶部"),
  e("system-app-dir", "system", "程序配置目录", "sp-app-dir", "data directory config 配置 目录"),
  e("system-mail-dir", "system", "邮件存储目录", "sp-mail-dir", "mail directory storage 邮件 存储 目录"),
  e("system-storage-stats", "system", "本地缓存与附件占用", "sp-storage-stats", "storage usage cache 占用 缓存 统计"),
  e("system-keep-downloads", "system", "已下载邮件保留时长", "sp-keepDownloadsDays", "keep downloads retention 保留 时长"),
  e("system-run-mode", "system", "运行模式", "sp-run-mode", "run mode portable 运行模式 便携"),
  e("system-clear-cache", "system", "清除本地邮件正文缓存", "sp-clear-cache", "clear body cache 清除 缓存 正文"),
  e("system-check-updates", "system", "启动时检查更新", "sp-checkUpdates", "check updates 检查 更新"),
  e("system-version", "system", "版本号", "sp-version", "version about 版本 关于"),
  e("system-engine", "system", "界面与通信内核", "sp-engine", "engine webview 内核"),
  // ---- 备份 ----
  e("backup-export", "backup", "导出当前偏好与账号配置", "sp-backup-export", "backup export 备份 导出"),
  e("backup-import", "backup", "从备份文件恢复", "sp-backup-import", "backup import restore 备份 导入 恢复"),
  // ---- 账号组五页（WP06，页面级锚点） ----
  e("tags-page", "tags", "自定义标签与快捷键", "sp-tags-page", "tag label color shortcut 标签 颜色 快捷键"),
  e("filters-page", "filters", "过滤规则与动作", "sp-filters-page", "filter rule move archive 过滤 规则"),
  e("smart-folders-page", "smartFolders", "智能文件夹与查询", "sp-smart-folders-page", "smart folder query virtual 智能文件夹 查询"),
  e("senders-page", "senders", "发件人黑名单", "sp-senders-page", "sender block blacklist 发件人 黑名单 拦截"),
  e("openpgp-page", "openPgp", "OpenPGP 密钥与信任", "sp-openpgp-page", "pgp key fingerprint trust 密钥 指纹 信任"),
  // ---- 账号组三页（并行代理 B 开发中，本轮切页不滚动） ----
  e("cloud-storage-page", "cloudStorage", "云存储与自动转存", null, "cloud storage webdav s3 nextcloud 云存储 转存 附件"),
  e("translation-page", "translation", "翻译引擎与目标语言", null, "translation deepl libretranslate 翻译 引擎 语言"),
  e("contacts-sources-page", "contactsSources", "通讯录源与自动收集", null, "contacts carddav sync 通讯录 源 同步"),
];

export interface ScoredSearchEntry extends PrefSearchEntry {
  /** 命中得分（4 标题前缀 / 3 标题包含 / 2 页签包含 / 1 关键词包含） */
  score: number;
}

/** 按查询检索并排序（镜像 GPUI filter_search_items：分数降序、索引升序、上限 60）。 */
export function filterSearchEntries(query: string): PrefSearchEntry[] {
  const q = query.trim().toLowerCase();
  if (q.length === 0) return [];

  const scored: Array<{ score: number; index: number; entry: PrefSearchEntry }> = [];
  SEARCH_ENTRIES.forEach((entry, index) => {
    const title = entry.title.toLowerCase();
    const tabLabel = tabLabelOf(entry.tabId);
    const keywords = entry.keywords.toLowerCase();

    let score: number | null = null;
    if (title.startsWith(q)) score = 4;
    else if (title.includes(q)) score = 3;
    else if (tabLabel.includes(q)) score = 2;
    else if (keywords.includes(q)) score = 1;
    if (score !== null) scored.push({ score, index, entry });
  });

  scored.sort((a, b) => b.score - a.score || a.index - b.index);
  return scored.slice(0, SEARCH_RESULT_CAP).map((s) => s.entry);
}

/** 页签 id → 中文名（复用 nav.ts 的 22 页签定义）。 */
function tabLabelOf(tabId: SettingsTabId): string {
  return SETTINGS_TABS.find((tab) => tab.id === tabId)?.label ?? "";
}
