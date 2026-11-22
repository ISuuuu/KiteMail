//! 偏好设置全局关键词索引与匹配排序（对齐 Linux 版 `search.rs`）。

use crate::ui::preferences::PrefTab;

/// 单条可检索设置项索引记录。
#[derive(Clone, Copy, Debug)]
pub(crate) struct PrefSearchItem {
    pub(crate) tab: PrefTab,
    pub(crate) row_id: &'static str,
    pub(crate) title: &'static str,
    pub(crate) subtitle: &'static str,
    pub(crate) keywords: &'static str,
}

/// 静态设置项目录：覆盖 2 个分组、14 个分页下的全部真实设置项。
pub(crate) const SEARCH_ITEMS: &[PrefSearchItem] = &[
    // ---- 账号：邮箱账号 ----
    PrefSearchItem {
        tab: PrefTab::MailAccounts,
        row_id: "accounts-manage",
        title: "邮箱账号管理与添加",
        subtitle: "QQ/163/126/企业邮/Gmail/Outlook 等服务商预设、账号昵称/强调色/头像、推送覆盖、统一收件箱与拖拽排序",
        keywords: "account mail provider imap smtp oauth outlook qq 163 126 exmail aliyun gmail icloud yahoo proton fastmail aol zoho gmx yandex nickname label color emoji avatar unified push reorder 邮箱 账号 服务商 网易 腾讯企业邮 阿里企业邮 登录 密码 签名 昵称 颜色 强调色 头像 推送 排序 统一收件箱",
    },
    // ---- 账号：发件人 ----
    PrefSearchItem {
        tab: PrefTab::Senders,
        row_id: "senders-allowlist",
        title: "允许加载远程图像的发件人",
        subtitle: "将可信发件人加入或移出远程图片白名单",
        keywords: "sender whitelist allowlist remote images 发件人 白名单 远程 图片 图像",
    },
    // ---- 常规 ----
    PrefSearchItem {
        tab: PrefTab::General,
        row_id: "pref-close-to-minimize",
        title: "关闭主窗口时最小化到托盘",
        subtitle: "点击关闭按钮时保持后台运行并常驻系统托盘",
        keywords: "close tray minimize 托盘 关闭 最小化 后台",
    },
    PrefSearchItem {
        tab: PrefTab::General,
        row_id: "pref-minimize-to-tray",
        title: "最小化时隐藏到托盘",
        subtitle: "最小化窗口后仅保留系统托盘图标",
        keywords: "minimize tray hide 托盘 最小化 隐藏",
    },
    PrefSearchItem {
        tab: PrefTab::General,
        row_id: "pref-default-view",
        title: "启动时打开的默认视图",
        subtitle: "选择打开「统一收件箱」或恢复上次关闭时的文件夹",
        keywords: "startup default view last folder unified 启动 视图 默认 上次",
    },
    PrefSearchItem {
        tab: PrefTab::General,
        row_id: "pref-start-fullscreen",
        title: "启动时全屏",
        subtitle: "启动 Hylki 主窗口后自动进入全屏模式",
        keywords: "fullscreen startup window 启动 全屏",
    },
    PrefSearchItem {
        tab: PrefTab::General,
        row_id: "pref-page-size",
        title: "单页加载邮件数量",
        subtitle: "每次分页拉取与渲染的邮件条数",
        keywords: "page size pagination 50 100 200 分页 每页 邮件数量",
    },
    PrefSearchItem {
        tab: PrefTab::General,
        row_id: "pref-auto-sync",
        title: "自动后台同步间隔",
        subtitle: "每隔指定分钟检查新邮件，设为「仅手动」则关闭定时拉取",
        keywords: "sync interval auto refresh 同步 间隔 自动 刷新 分钟",
    },
    PrefSearchItem {
        tab: PrefTab::General,
        row_id: "pref-push-enabled",
        title: "实时邮件推送（IDLE）",
        subtitle: "保持 IMAP IDLE 长连接实时感知新邮件到达",
        keywords: "push idle realtime notification 推送 实时 新邮件",
    },
    PrefSearchItem {
        tab: PrefTab::General,
        row_id: "pref-desktop-notifications",
        title: "新邮件桌面通知",
        subtitle: "后台同步发现新未读邮件时弹出 Windows 托盘通知",
        keywords: "desktop notification toast alert 桌面 通知 提醒",
    },
    PrefSearchItem {
        tab: PrefTab::General,
        row_id: "pref-notification-content",
        title: "在通知中显示发件人与主题",
        subtitle: "关闭后仅提示未读封数，隐藏邮件隐私摘要",
        keywords: "notification preview sender subject content 通知 内容 隐私 摘要",
    },
    PrefSearchItem {
        tab: PrefTab::General,
        row_id: "pref-single-key-shortcuts",
        title: "启用单键快捷键",
        subtitle: "开启 Gmail 风格单键导航（j / k / r / a / d / s / m）",
        keywords: "shortcut keybindings gmail single key 快捷键 单键",
    },
    PrefSearchItem {
        tab: PrefTab::General,
        row_id: "pref-settings-open-accounts",
        title: "打开偏好设置时优先显示「邮箱账号」",
        subtitle: "新建快捷入口时直接定位至账号管理页",
        keywords: "settings default tab accounts 设置 启动页 邮箱账号",
    },
    // ---- 外观 ----
    PrefSearchItem {
        tab: PrefTab::Appearance,
        row_id: "pref-dark-mode",
        title: "明暗外观模式",
        subtitle: "跟随系统或强制使用浅色 / 深色界面",
        keywords: "theme dark light system appearance 深色 浅色 暗黑 跟随系统",
    },
    PrefSearchItem {
        tab: PrefTab::Appearance,
        row_id: "pref-theme-gallery",
        title: "主题配色画廊",
        subtitle: "经典蓝、子夜、海潮、玫瑰、大地、森林",
        keywords: "color palette accent default midnight tidal rose earth forest 主题 配色",
    },
    PrefSearchItem {
        tab: PrefTab::Appearance,
        row_id: "pref-ui-text-scale",
        title: "界面字体缩放",
        subtitle: "调整全窗口排版基准字号（90% ~ 150%）",
        keywords: "text scale font size ui zoom rem 字体 缩放 界面 字号",
    },
    PrefSearchItem {
        tab: PrefTab::Appearance,
        row_id: "pref-reader-zoom",
        title: "阅读器正文字号",
        subtitle: "调整邮件阅读正文的显示比例（70% ~ 160%）",
        keywords: "reader zoom font body 阅读器 正文 缩放 字号",
    },
    PrefSearchItem {
        tab: PrefTab::Appearance,
        row_id: "pref-status-bar-top",
        title: "将状态栏置于窗口顶部",
        subtitle: "把底部同步状态与快捷操作提示条移动至主内容顶部",
        keywords: "status bar top bottom 状态栏 顶部 底部",
    },
    // ---- 专注模式 ----
    PrefSearchItem {
        tab: PrefTab::FocusMode,
        row_id: "pref-focus-start",
        title: "启动时进入专注模式",
        subtitle: "打开 Hylki 时自动启用专注模式（Ctrl+Shift+F）",
        keywords: "focus mode startup distraction free 专注 模式 启动",
    },
    PrefSearchItem {
        tab: PrefTab::FocusMode,
        row_id: "pref-focus-hide-sidebar",
        title: "专注模式下隐藏侧边栏",
        subtitle: "关闭后专注模式仍保留左侧导航栏",
        keywords: "focus hide sidebar navigation 专注 隐藏 侧边栏",
    },
    PrefSearchItem {
        tab: PrefTab::FocusMode,
        row_id: "pref-focus-hide-list",
        title: "专注模式下隐藏邮件列表",
        subtitle: "聚焦阅读当前邮件，隐藏中间列表窗格",
        keywords: "focus hide list pane 专注 隐藏 邮件列表",
    },
    PrefSearchItem {
        tab: PrefTab::FocusMode,
        row_id: "pref-focus-reader-view",
        title: "专注模式下开启简化阅读视图",
        subtitle: "进入专注模式时自动切换为纯文本精简视图",
        keywords: "focus reader view plain 专注 简化 阅读视图",
    },
    PrefSearchItem {
        tab: PrefTab::FocusMode,
        row_id: "pref-focus-fold-toolbar",
        title: "专注模式下折叠阅读器工具栏",
        subtitle: "把次要阅读操作收入「更多」菜单以减少视觉干扰",
        keywords: "focus fold toolbar overflow 专注 工具栏 折叠",
    },
    // ---- 侧边栏 ----
    PrefSearchItem {
        tab: PrefTab::Sidebar,
        row_id: "pref-show-accounts",
        title: "在侧边栏显示账号与文件夹列表",
        subtitle: "控制是否展开各邮箱账号及其文件夹树",
        keywords: "sidebar accounts folders list 侧边栏 账号 文件夹",
    },
    PrefSearchItem {
        tab: PrefTab::Sidebar,
        row_id: "pref-sidebar-contacts",
        title: "显示「联系人」入口",
        subtitle: "在侧边栏底部展示联系人管理入口",
        keywords: "sidebar contacts footer 侧边栏 联系人",
    },
    PrefSearchItem {
        tab: PrefTab::Sidebar,
        row_id: "pref-sidebar-attachments",
        title: "显示「附件总览」入口",
        subtitle: "在侧边栏底部展示附件画廊入口",
        keywords: "sidebar attachments gallery footer 侧边栏 附件",
    },
    PrefSearchItem {
        tab: PrefTab::Sidebar,
        row_id: "pref-remember-sidebar",
        title: "记住侧边栏折叠状态",
        subtitle: "重启后保持上次窄图标栏 / 完整侧栏的展开状态",
        keywords: "sidebar remember collapsed rail 侧边栏 记住 折叠",
    },
    PrefSearchItem {
        tab: PrefTab::Sidebar,
        row_id: "pref-rail-dots",
        title: "窄栏模式以未读红点代替数字",
        subtitle: "折叠为图标栏时用简洁小圆点指示未读邮件",
        keywords: "rail unread dots badge 窄栏 未读 红点 徽标",
    },
    PrefSearchItem {
        tab: PrefTab::Sidebar,
        row_id: "pref-unified-inbox",
        title: "显示统一视图导航项",
        subtitle: "分别控制统一收件箱、已加星标、已发送、草稿箱、已归档的可见性",
        keywords: "unified inbox starred sent drafts archive 统一 收件箱 星标 已发送 草稿 归档",
    },
    // ---- 邮件列表 ----
    PrefSearchItem {
        tab: PrefTab::MessageList,
        row_id: "pref-compact-list",
        title: "紧凑列表行样式",
        subtitle: "减小行高与内边距以同屏显示更多邮件",
        keywords: "compact list density rows 紧凑 列表 行高",
    },
    PrefSearchItem {
        tab: PrefTab::MessageList,
        row_id: "pref-show-avatars",
        title: "显示发件人头像",
        subtitle: "在邮件列表每行左侧显示字母头像圆章",
        keywords: "avatar sender photo initials list 头像 列表",
    },
    PrefSearchItem {
        tab: PrefTab::MessageList,
        row_id: "pref-list-group-date",
        title: "按日期分组邮件列表",
        subtitle: "在列表内按「今天 / 昨天 / 本周 / 更早」插入分组标题",
        keywords: "group date headers today yesterday 分组 日期 今天 昨天",
    },
    PrefSearchItem {
        tab: PrefTab::MessageList,
        row_id: "pref-list-search-always",
        title: "始终显示列表搜索栏",
        subtitle: "无需先点击放大镜按钮即始终展示搜索输入框",
        keywords: "search bar always open filter 搜索栏 常驻 始终显示",
    },
    PrefSearchItem {
        tab: PrefTab::MessageList,
        row_id: "pref-list-search-bottom",
        title: "搜索栏置于列表底部",
        subtitle: "将搜索输入与筛选芯片移动到列表窗格底部",
        keywords: "search bar bottom position 搜索栏 底部",
    },
    PrefSearchItem {
        tab: PrefTab::MessageList,
        row_id: "pref-list-bulk-bottom",
        title: "批量操作栏置于列表底部",
        subtitle: "多选邮件时将批量归档/删除/标记工具条置底",
        keywords: "bulk selection bar bottom 批量 操作栏 底部",
    },
    // ---- 会话 ----
    PrefSearchItem {
        tab: PrefTab::Conversations,
        row_id: "pref-thread-newest-first",
        title: "会话内最新邮件置顶",
        subtitle: "开启后最新回复显示在最上方，关闭则按时间从早到晚排列",
        keywords: "thread newest first order descending 会话 排序 最新 置顶",
    },
    PrefSearchItem {
        tab: PrefTab::Conversations,
        row_id: "pref-fold-messages-mode",
        title: "多封会话自动折叠策略",
        subtitle: "选择不折叠、仅折叠已读邮件或折叠除最新/当前封外的其余邮件",
        keywords: "fold collapse thread read all 会话 折叠 已读 展开",
    },
    PrefSearchItem {
        tab: PrefTab::Conversations,
        row_id: "pref-card-attachments",
        title: "在每张会话卡片中显示附件",
        subtitle: "在邮件正文中下方呈现附件卡片列表与保存按钮",
        keywords: "card attachments conversation 卡片 附件 会话",
    },
    // ---- 阅读 ----
    PrefSearchItem {
        tab: PrefTab::Reading,
        row_id: "pref-read-mark-mode",
        title: "自动标记为已读时机",
        subtitle: "选择选中即标已读、停留 2 秒后标已读或仅手动标记",
        keywords: "mark read delay manual shown 自动 已读 标记 延迟",
    },
    PrefSearchItem {
        tab: PrefTab::Reading,
        row_id: "pref-reader-position",
        title: "阅读器窗格位置",
        subtitle: "在右侧并排显示、在列表下方显示或关闭阅读窗格",
        keywords: "reader position right below off pane layout 阅读器 位置 右侧 下方 布局",
    },
    PrefSearchItem {
        tab: PrefTab::Reading,
        row_id: "pref-single-message-card",
        title: "单封邮件以卡片样式显示",
        subtitle: "关闭后单封邮件铺满阅读窗格边缘而不绘制卡片外框",
        keywords: "single message card border edge 阅读 单封 卡片",
    },
    PrefSearchItem {
        tab: PrefTab::Reading,
        row_id: "pref-message-theme",
        title: "邮件正文外观主题",
        subtitle: "跟随界面主题、始终浅色信纸或始终深色信纸",
        keywords: "message theme light dark paper 信纸 正文 主题 深色 浅色",
    },
    PrefSearchItem {
        tab: PrefTab::Reading,
        row_id: "pref-always-show-recipients",
        title: "始终显示收件人行",
        subtitle: "在发件人卡片头部固定展开「收件人：...」信息行",
        keywords: "recipients header to cc always show 收件人 卡片 头部",
    },
    PrefSearchItem {
        tab: PrefTab::Reading,
        row_id: "pref-reader-show-account-chip",
        title: "阅读器顶栏显示账号标签",
        subtitle: "在顶部动作栏显示当前邮件所属的邮箱账号芯片",
        keywords: "reader chip account topbar 阅读器 账号 标签",
    },
    PrefSearchItem {
        tab: PrefTab::Reading,
        row_id: "pref-reader-show-theme-switch",
        title: "阅读器顶栏显示信纸明暗切换",
        subtitle: "提供单封正文浅色/深色快捷开关按钮",
        keywords: "reader theme toggle switch 阅读器 明暗 切换",
    },
    PrefSearchItem {
        tab: PrefTab::Reading,
        row_id: "pref-reader-show-view-switch",
        title: "阅读器顶栏显示简化阅读视图按钮",
        subtitle: "显示纯文本/HTML 富文本视图切换按钮",
        keywords: "reader view plain html switch 阅读器 视图 纯文本",
    },
    PrefSearchItem {
        tab: PrefTab::Reading,
        row_id: "pref-reader-default-mode",
        title: "默认简化阅读视图行为",
        subtitle: "保持上次选择、打开邮件时默认开启或始终显示完整富文本",
        keywords: "reader default mode remember on off 简化视图 默认 纯文本",
    },
    PrefSearchItem {
        tab: PrefTab::Reading,
        row_id: "pref-reader-fold-right",
        title: "始终折叠阅读器右侧工具栏",
        subtitle: "把标记未读、查看原文、另存 EML 等按钮收敛到菜单中",
        keywords: "reader toolbar fold overflow 更多 工具栏 折叠",
    },
    // ---- 写信 ----
    PrefSearchItem {
        tab: PrefTab::Composing,
        row_id: "pref-compose-default-from",
        title: "默认发件账号",
        subtitle: "撰写全新邮件时默认选中的发件身份",
        keywords: "compose from default account sender 默认 发件人 写信",
    },
    PrefSearchItem {
        tab: PrefTab::Composing,
        row_id: "pref-signature-above-quote",
        title: "回复与转发时签名置于引用正文上方",
        subtitle: "将签名插在输入区之后、原始邮件引用块之前",
        keywords: "signature above quote reply forward 签名 引用 上方 回复",
    },
    PrefSearchItem {
        tab: PrefTab::Composing,
        row_id: "pref-signature-dashes",
        title: "在签名前添加 \"-- \" 标准分隔线",
        subtitle: "关闭后直接插入签名文本而不附加 RFC 分隔符",
        keywords: "signature dashes separator 签名 分隔符",
    },
    PrefSearchItem {
        tab: PrefTab::Composing,
        row_id: "pref-undo-send-secs",
        title: "撤销发送等待时长",
        subtitle: "点击发送后可反悔撤回的缓冲秒数（0 秒表示立即投递）",
        keywords: "undo send delay timer cancel 撤销 发送 延时 缓冲",
    },
    PrefSearchItem {
        tab: PrefTab::Composing,
        row_id: "pref-reply-header-template",
        title: "自定义回复引用头模板",
        subtitle: "支持 {name}、{address}、{date}、{time}、{subject} 占位符",
        keywords: "reply header attribution template 回复 引用 模板 占位符",
    },
    PrefSearchItem {
        tab: PrefTab::Composing,
        row_id: "pref-attachment-keywords",
        title: "遗漏附件检测关键词",
        subtitle: "主题或正文出现这些词但未添加附件时发出发送前确认",
        keywords: "attachment reminder keyword detect 附件 提醒 关键词 遗漏",
    },
    // ---- 隐私 ----
    PrefSearchItem {
        tab: PrefTab::Privacy,
        row_id: "pref-block-remote-images",
        title: "默认屏蔽邮件中的远程图像",
        subtitle: "阻止未加入白名单发件人的外部追踪像素与图片自动加载",
        keywords: "privacy remote images block tracking pixel 隐私 远程 图像 屏蔽 追踪",
    },
    PrefSearchItem {
        tab: PrefTab::Privacy,
        row_id: "pref-show-remote-banner",
        title: "显示远程图像已拦截横幅",
        subtitle: "关闭后静默屏蔽远程图片，不在阅读卡片顶部显示信任提示条",
        keywords: "remote banner notification blocked 拦截 横幅 提示",
    },
    PrefSearchItem {
        tab: PrefTab::Privacy,
        row_id: "pref-clean-links",
        title: "自动净化邮件链接追踪参数",
        subtitle: "渲染 HTML 正文时剥离 utm_*、fbclid、gclid、spm 等跟踪查询参数",
        keywords: "clean links utm fbclid gclid spm tracking 链接 净化 追踪 参数",
    },
    // ---- 日期和时间 ----
    PrefSearchItem {
        tab: PrefTab::DateTime,
        row_id: "pref-date-style",
        title: "日期显示格式",
        subtitle: "调整列表与阅读器头部中的月/日/年顺序",
        keywords: "date format month day year ymd mdy 日期 格式",
    },
    PrefSearchItem {
        tab: PrefTab::DateTime,
        row_id: "pref-clock-style",
        title: "时间制式（12 / 24 小时）",
        subtitle: "调整时间显示为 24 小时制或 12 小时 AM/PM 制",
        keywords: "clock time 12h 24h am pm 时间 小时制 格式",
    },
    // ---- 系统 ----
    PrefSearchItem {
        tab: PrefTab::System,
        row_id: "pref-data-dir",
        title: "本地数据与缓存目录",
        subtitle: "查看 preferences.toml、accounts.toml 与 SQLite 缓存路径",
        keywords: "system cache data directory path sqlite 系统 缓存 目录 路径",
    },
    PrefSearchItem {
        tab: PrefTab::System,
        row_id: "pref-clear-body-cache",
        title: "清除本地邮件正文缓存",
        subtitle: "释放已下载邮件正文的磁盘空间（保留账号配置与标题索引）",
        keywords: "clear body cache storage clean 清除 正文 缓存 磁盘",
    },
    PrefSearchItem {
        tab: PrefTab::System,
        row_id: "pref-about-version",
        title: "Hylki Windows 版本与引擎信息",
        subtitle: "查看当前客户端版本与桌面底层框架说明",
        keywords: "about version gpui info 关于 版本",
    },
    // ---- 备份 ----
    PrefSearchItem {
        tab: PrefTab::Backup,
        row_id: "pref-backup-export",
        title: "导出配置备份文件",
        subtitle: "将所有偏好设置与账号服务器参数导出为 TOML 文件（不含密码与邮件）",
        keywords: "backup export toml config settings 备份 导出 配置",
    },
    PrefSearchItem {
        tab: PrefTab::Backup,
        row_id: "pref-backup-import",
        title: "从备份文件恢复配置",
        subtitle: "校验 TOML 备份文件版本与结构并在确认后恢复设置与账号配置",
        keywords: "backup import restore toml config 备份 导入 恢复",
    },
];

/// 按查询词检索设置项并排序：标题前向/子串命中优先，最多返回 60 条。
pub(crate) fn filter_search_items(query: &str) -> Vec<&'static PrefSearchItem> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }

    let mut scored: Vec<(u8, usize, &'static PrefSearchItem)> = SEARCH_ITEMS
        .iter()
        .enumerate()
        .filter_map(|(idx, item)| {
            let title = item.title.to_lowercase();
            let subtitle = item.subtitle.to_lowercase();
            let tab_label = item.tab.label().to_lowercase();
            let keywords = item.keywords.to_lowercase();

            let score = if title.starts_with(&q) {
                4
            } else if title.contains(&q) {
                3
            } else if subtitle.contains(&q) || tab_label.contains(&q) {
                2
            } else if keywords.contains(&q) {
                1
            } else {
                return None;
            };
            Some((score, idx, item))
        })
        .collect();

    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored.truncate(60);
    scored.into_iter().map(|(_, _, item)| item).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_search_items_ranks_title_matches_and_caps_at_60() {
        assert!(filter_search_items("   ").is_empty());

        let remote = filter_search_items("远程图像");
        assert!(!remote.is_empty());
        assert!(remote.iter().any(|item| item.tab == PrefTab::Senders));
        assert!(remote.iter().any(|item| item.tab == PrefTab::Privacy));

        let privacy_prefix = filter_search_items("默认屏蔽");
        assert!(!privacy_prefix.is_empty());
        assert_eq!(privacy_prefix[0].tab, PrefTab::Privacy);

        // 单字符宽泛命中应被上限裁剪为 <= 60。
        let broad = filter_search_items("a");
        assert!(!broad.is_empty());
        assert!(broad.len() <= 60);
    }

    #[test]
    fn search_items_cover_every_pref_tab() {
        for tab in PrefTab::ALL {
            assert!(
                SEARCH_ITEMS.iter().any(|item| item.tab == *tab),
                "missing search entry for tab {:?}",
                tab
            );
        }
    }
}
