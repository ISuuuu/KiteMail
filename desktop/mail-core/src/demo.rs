//! Deterministic demo dataset for `--demo` mode and list pagination verification.

use std::collections::BTreeMap;

use crate::cache::{CachedAttachment, KnownContact};
use crate::search_query::SearchQuery;
use crate::{
    start_of_today_ts, AccountSummary, AttachmentMeta, FolderSummary, MailPage, MessageDetail,
    MessageFilter, MessageSort, MessageSummary, QuickFilterSet, ThreadMessage,
};

pub const DEMO_INBOX_ID: u32 = 1;
pub const DEMO_SENT_ID: u32 = 2;
pub const DEMO_DRAFTS_ID: u32 = 3;
pub const DEMO_ARCHIVE_ID: u32 = 4;

/// Total number of generated messages in the demo inbox for scroll/pagination tests.
pub const DEMO_INBOX_TOTAL: usize = 10_000;

/// In-memory demo store serving accounts, folders, paginated headers, and bodies.
#[derive(Clone, Debug)]
pub struct DemoStore {
    pub accounts: Vec<AccountSummary>,
    pub folders: Vec<FolderSummary>,
    inbox_messages: Vec<MessageSummary>,
    sent_messages: Vec<MessageSummary>,
    draft_messages: Vec<MessageSummary>,
    archive_messages: Vec<MessageSummary>,
}

impl Default for DemoStore {
    fn default() -> Self {
        Self::new()
    }
}

impl DemoStore {
    /// Build a demo dataset with 10,000 inbox messages and realistic Chinese/English samples.
    pub fn new() -> Self {
        let accounts = vec![AccountSummary {
            id: 1,
            display_name: "测试工作邮箱".to_string(),
            email: "demo@hylki.example".to_string(),
        }];

        let folders = vec![
            FolderSummary {
                id: DEMO_INBOX_ID,
                account_id: 1,
                name: "收件箱".to_string(),
                path: "INBOX".to_string(),
                unread_count: 128,
                total_count: DEMO_INBOX_TOTAL,
                has_more_remote: false,
            },
            FolderSummary {
                id: DEMO_SENT_ID,
                account_id: 1,
                name: "已发送".to_string(),
                path: "Sent".to_string(),
                unread_count: 0,
                total_count: 12,
                has_more_remote: false,
            },
            FolderSummary {
                id: DEMO_DRAFTS_ID,
                account_id: 1,
                name: "草稿箱".to_string(),
                path: "Drafts".to_string(),
                unread_count: 0,
                total_count: 3,
                has_more_remote: false,
            },
            FolderSummary {
                id: DEMO_ARCHIVE_ID,
                account_id: 1,
                name: "归档".to_string(),
                path: "Archive".to_string(),
                unread_count: 0,
                total_count: 24,
                has_more_remote: false,
            },
        ];

        let mut inbox_messages = Vec::with_capacity(DEMO_INBOX_TOTAL);
        let pinned_samples = [
            (
                "林晓明 <xiaoming@example.com>",
                "Hylki Windows 阶段 1 构建基线确认",
                "请确认 MSVC 工具链、中文 IME 输入以及 100%/150% DPI 缩放表现。",
                "14:32",
                true,
                true,
            ),
            (
                "Build Bot <ci@hylki.example>",
                "[CI] x86_64-pc-windows-msvc 夜间构建完成",
                "工作区 desktop/Cargo.toml 编译通过，未引入 GTK/Wayland 依赖。",
                "11:18",
                true,
                false,
            ),
            (
                "产品设计组 <design@hylki.example>",
                "三栏布局与阅读区字号反馈（含附件清单）",
                "附件包含三栏间距规范和暗色/亮色主题色板对比，请查收。",
                "昨天",
                false,
                true,
            ),
            (
                "陈思远 <siyuan@example.org>",
                "Re: 周五邮件客户端性能压测记录",
                "一万封邮件分页加载耗时稳定在毫秒级，列表滚动无明显掉帧。",
                "昨天",
                false,
                false,
            ),
        ];

        let now_ts = chrono::Utc::now().timestamp();
        for (idx, (sender, subject, preview, date_display, unread, has_attachment)) in
            pinned_samples.iter().enumerate()
        {
            let id = (idx + 1) as u32;
            let date_ts = match idx {
                0 => now_ts - 600,
                1 => now_ts - 3600,
                2 => now_ts - 86_400 - 1800,
                _ => now_ts - 86_400 - 7200,
            };
            inbox_messages.push(MessageSummary {
                id,
                account_id: 1,
                folder_id: DEMO_INBOX_ID,
                folder_path: "INBOX".to_string(),
                uid: id,
                sender: (*sender).to_string(),
                subject: (*subject).to_string(),
                preview: (*preview).to_string(),
                date_ts,
                date_display: (*date_display).to_string(),
                unread: *unread,
                flagged: idx == 0,
                has_attachment: *has_attachment,
                recipients: if idx == 0 {
                    "demo@hylki.example".to_string()
                } else {
                    String::new()
                },
                thread_count: Self::demo_thread_count(id),
                importance: if idx == 0 { 1 } else { 0 },
                tags: if idx == 0 {
                    vec!["重要".to_string()]
                } else {
                    Vec::new()
                },
                answered: idx == 1,
                forwarded: false,
                snoozed_until_ts: 0,
            });
        }

        for idx in pinned_samples.len()..DEMO_INBOX_TOTAL {
            let id = (idx + 1) as u32;
            let day = (idx % 28) + 1;
            let hour = (idx * 7) % 24;
            let minute = (idx * 13) % 60;
            let days_ago = if idx < 10 {
                (idx as i64) - 2
            } else {
                10 + (idx as i64 % 90)
            };
            let date_ts = now_ts - days_ago * 86_400 - (idx as i64 * 17);
            inbox_messages.push(MessageSummary {
                id,
                account_id: 1,
                folder_id: DEMO_INBOX_ID,
                folder_path: "INBOX".to_string(),
                uid: id,
                sender: format!("发件人 #{:04} <sender{}@example.com>", id, id % 50),
                subject: format!("测试邮件 #{id:05} — Windows 客户端列表分页与滚动验证"),
                preview: format!(
                    "这是第 {id} 封模拟邮件摘要，用于验证 10,000 封邮件下的分页查询、中文排版与列表滚动性能。"
                ),
                date_ts,
                date_display: format!("11-{day:02} {hour:02}:{minute:02}"),
                unread: idx < 128,
                flagged: idx < 30 && idx % 7 == 0,
                has_attachment: idx % 9 == 0,
                recipients: if idx == 0 {
                    "demo@hylki.example".to_string()
                } else {
                    String::new()
                },
                thread_count: Self::demo_thread_count(id),
                importance: if idx == 0 { 1 } else { 0 },
                tags: if idx == 0 {
                    vec!["重要".to_string()]
                } else {
                    Vec::new()
                },
                answered: idx == 1,
                forwarded: false,
                snoozed_until_ts: 0,
            });
        }

        let sent_messages = (0..12)
            .map(|i| {
                let id = 20_000 + i + 1;
                MessageSummary {
                    id,
                    account_id: 1,
                    folder_id: DEMO_SENT_ID,
                    folder_path: "Sent".to_string(),
                    uid: id,
                    sender: "我 <demo@hylki.example>".to_string(),
                    subject: format!("已发送测试主题 #{}", i + 1),
                    preview: "这封邮件已通过测试发件流程发出，用于检查已发送列表显示。".to_string(),
                    date_ts: now_ts - ((i as i64 + 1) * 86_400),
                    date_display: format!("11-{:02}", 20 - i),
                    unread: false,
                    flagged: false,
                    has_attachment: i % 3 == 0,
                    // B-6：已发送视图列表行显示「致：收件人」。
                    recipients: format!("收件人 #{i:02} <to{i:02}@example.com>"),
                    thread_count: Self::demo_thread_count(id),
                    importance: 0,
                    tags: Vec::new(),
                    answered: false,
                    forwarded: false,
                    snoozed_until_ts: 0,
                }
            })
            .collect();

        let draft_messages = (0..3)
            .map(|i| {
                let id = 30_000 + i + 1;
                MessageSummary {
                    id,
                    account_id: 1,
                    folder_id: DEMO_DRAFTS_ID,
                    folder_path: "Drafts".to_string(),
                    uid: id,
                    sender: "草稿 <demo@hylki.example>".to_string(),
                    subject: format!("未完成草稿 #{}", i + 1),
                    preview: "待补充正文内容与收件人地址...".to_string(),
                    date_ts: now_ts - (i as i64 * 3600),
                    date_display: "草稿".to_string(),
                    unread: false,
                    flagged: false,
                    has_attachment: false,
                    // B-6：草稿视图显示收件人（首条有收件人，其余留空回落显示发件人）。
                    recipients: if i == 0 {
                        "王五 <wangwu@example.com>".to_string()
                    } else {
                        String::new()
                    },
                    thread_count: Self::demo_thread_count(id),
                    importance: 0,
                    tags: Vec::new(),
                    answered: false,
                    forwarded: false,
                    snoozed_until_ts: 0,
                }
            })
            .collect();

        let archive_messages = (0..24)
            .map(|i| {
                let id = 40_000 + i + 1;
                MessageSummary {
                    id,
                    account_id: 1,
                    folder_id: DEMO_ARCHIVE_ID,
                    folder_path: "Archive".to_string(),
                    uid: id,
                    sender: format!("归档联系人 #{}", i + 1),
                    subject: format!("历史归档记录 #{}", i + 1),
                    preview: "该邮件已归档保存，用于离线检索与分页切换测试。".to_string(),
                    date_ts: now_ts - ((i as i64 + 15) * 86_400),
                    date_display: "10-15".to_string(),
                    unread: false,
                    flagged: false,
                    has_attachment: false,
                    recipients: String::new(),
                    thread_count: Self::demo_thread_count(id),
                    importance: 0,
                    tags: Vec::new(),
                    answered: false,
                    forwarded: false,
                    snoozed_until_ts: 0,
                }
            })
            .collect();

        Self {
            accounts,
            folders,
            inbox_messages,
            sent_messages,
            draft_messages,
            archive_messages,
        }
    }

    fn messages_for_folder(&self, folder_id: u32) -> &[MessageSummary] {
        match folder_id {
            DEMO_INBOX_ID => &self.inbox_messages,
            DEMO_SENT_ID => &self.sent_messages,
            DEMO_DRAFTS_ID => &self.draft_messages,
            DEMO_ARCHIVE_ID => &self.archive_messages,
            _ => &[],
        }
    }

    /// B-4：演示模式会话计数（与 `get_message_detail` 的多封往来会话样例保持一致：
    /// 邮件 #1 与 #3 各有 2 封往来，其余为独立邮件）。
    fn demo_thread_count(id: u32) -> u32 {
        if id == 1 || id == 3 {
            2
        } else {
            1
        }
    }

    /// Return a slice of messages in `folder_id` starting at `offset` with at most `limit` rows.
    pub fn list_messages_page(
        &self,
        folder_id: u32,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> MailPage {
        self.list_messages_page_filtered(folder_id, query, MessageFilter::All, offset, limit)
    }

    /// Return a slice of messages in `folder_id` applying both `query` and `filter`.
    pub fn list_messages_page_filtered(
        &self,
        folder_id: u32,
        query: &str,
        filter: MessageFilter,
        offset: usize,
        limit: usize,
    ) -> MailPage {
        self.list_messages_page_with_options(
            folder_id,
            query,
            QuickFilterSet::from_message_filter(filter),
            MessageSort::DateNewest,
            offset,
            limit,
        )
    }

    /// Return a slice of messages in `folder_id` with structured query, quick filters, and sort.
    pub fn list_messages_page_with_options(
        &self,
        folder_id: u32,
        query: &str,
        quick_filters: QuickFilterSet,
        sort: MessageSort,
        offset: usize,
        limit: usize,
    ) -> MailPage {
        let all = self.messages_for_folder(folder_id);
        let parsed = SearchQuery::parse(query);
        if parsed.is_empty() && quick_filters.is_empty() && sort == MessageSort::DateNewest {
            let total = all.len();
            let start = offset.min(total);
            let end = start.saturating_add(limit).min(total);
            return MailPage {
                folder_id,
                offset: start,
                total,
                items: all[start..end].to_vec(),
            };
        }

        let folder_name = self.folder_name_for(folder_id);
        let today_start = start_of_today_ts();
        let mut filtered: Vec<MessageSummary> = all
            .iter()
            .filter(|m| Self::matches_quick_filters(m, quick_filters, today_start))
            .filter(|m| {
                parsed.is_empty()
                    || parsed.matches_summary(m, folder_name, "demo@hylki.example", None)
            })
            .cloned()
            .collect();

        Self::sort_summaries(&mut filtered, sort);

        let total = filtered.len();
        let start = offset.min(total);
        let end = start.saturating_add(limit).min(total);
        MailPage {
            folder_id,
            offset: start,
            total,
            items: filtered[start..end].to_vec(),
        }
    }

    /// Return a page of unread demo messages with structured query, quick filters, and sort.
    pub fn list_unread_page_with_options(
        &self,
        query: &str,
        quick_filters: QuickFilterSet,
        sort: MessageSort,
        offset: usize,
        limit: usize,
    ) -> MailPage {
        let parsed = SearchQuery::parse(query);
        let today_start = start_of_today_ts();
        let mut filtered: Vec<MessageSummary> = self
            .inbox_messages
            .iter()
            .chain(self.sent_messages.iter())
            .chain(self.archive_messages.iter())
            .filter(|m| m.unread && Self::matches_quick_filters(m, quick_filters, today_start))
            .filter(|m| {
                parsed.is_empty()
                    || parsed.matches_summary(
                        m,
                        self.folder_name_for(m.folder_id),
                        "demo@hylki.example",
                        None,
                    )
            })
            .cloned()
            .collect();

        Self::sort_summaries(&mut filtered, sort);

        let total = filtered.len();
        let start = offset.min(total);
        let end = start.saturating_add(limit).min(total);
        MailPage {
            folder_id: 0,
            offset: start,
            total,
            items: filtered[start..end].to_vec(),
        }
    }

    /// Return a page of starred demo messages across all demo folders.
    pub fn list_starred_page(
        &self,
        query: &str,
        filter: MessageFilter,
        offset: usize,
        limit: usize,
    ) -> MailPage {
        self.list_starred_page_with_options(
            query,
            QuickFilterSet::from_message_filter(filter),
            MessageSort::DateNewest,
            offset,
            limit,
        )
    }

    /// Return a page of starred demo messages with structured query, quick filters, and sort.
    pub fn list_starred_page_with_options(
        &self,
        query: &str,
        quick_filters: QuickFilterSet,
        sort: MessageSort,
        offset: usize,
        limit: usize,
    ) -> MailPage {
        let parsed = SearchQuery::parse(query);
        let today_start = start_of_today_ts();
        let mut filtered: Vec<MessageSummary> = self
            .inbox_messages
            .iter()
            .chain(self.sent_messages.iter())
            .chain(self.archive_messages.iter())
            .filter(|m| m.flagged && Self::matches_quick_filters(m, quick_filters, today_start))
            .filter(|m| {
                parsed.is_empty()
                    || parsed.matches_summary(
                        m,
                        self.folder_name_for(m.folder_id),
                        "demo@hylki.example",
                        None,
                    )
            })
            .cloned()
            .collect();

        Self::sort_summaries(&mut filtered, sort);

        let total = filtered.len();
        let start = offset.min(total);
        let end = start.saturating_add(limit).min(total);
        MailPage {
            folder_id: 0,
            offset: start,
            total,
            items: filtered[start..end].to_vec(),
        }
    }

    /// Cross-folder search across all demo folders.
    pub fn search_all_folders(
        &self,
        query: &str,
        quick_filters: QuickFilterSet,
        sort: MessageSort,
        offset: usize,
        limit: usize,
    ) -> MailPage {
        let parsed = SearchQuery::parse(query);
        let today_start = start_of_today_ts();
        let mut filtered: Vec<MessageSummary> = self
            .inbox_messages
            .iter()
            .chain(self.sent_messages.iter())
            .chain(self.archive_messages.iter())
            .filter(|m| Self::matches_quick_filters(m, quick_filters, today_start))
            .filter(|m| {
                parsed.is_empty()
                    || parsed.matches_summary(
                        m,
                        self.folder_name_for(m.folder_id),
                        "demo@hylki.example",
                        None,
                    )
            })
            .cloned()
            .collect();

        Self::sort_summaries(&mut filtered, sort);

        let total = filtered.len();
        let start = offset.min(total);
        let end = start.saturating_add(limit).min(total);
        MailPage {
            folder_id: 0,
            offset: start,
            total,
            items: filtered[start..end].to_vec(),
        }
    }

    fn folder_name_for(&self, folder_id: u32) -> &str {
        self.folders
            .iter()
            .find(|f| f.id == folder_id)
            .map(|f| f.name.as_str())
            .unwrap_or("")
    }

    fn matches_quick_filters(
        m: &MessageSummary,
        filters: QuickFilterSet,
        today_start_ts: i64,
    ) -> bool {
        let sender_lower = m.sender.to_lowercase();
        let recip_lower = m.recipients.to_lowercase();
        (!filters.unread || m.unread)
            && (!filters.starred || m.flagged)
            && (!filters.has_attachment || m.has_attachment)
            && (!filters.today || m.date_ts >= today_start_ts)
            && (!filters.to_me
                || recip_lower.contains("demo@hylki.example")
                || recip_lower.contains("you@"))
            && (!filters.from_contacts
                || sender_lower.contains("zhangwei")
                || sender_lower.contains("billing")
                || sender_lower.contains("wangwu"))
            && (!filters.high_importance || m.importance > 0)
            && (!filters.tagged || !m.tags.is_empty())
            && filters.date_from_ts.is_none_or(|from| m.date_ts >= from)
            && filters.date_to_ts.is_none_or(|to| m.date_ts < to)
    }

    fn sort_summaries(items: &mut [MessageSummary], sort: MessageSort) {
        match sort {
            MessageSort::DateNewest => {
                items.sort_by(|a, b| b.date_ts.cmp(&a.date_ts).then_with(|| b.uid.cmp(&a.uid)));
            }
            MessageSort::DateOldest => {
                items.sort_by(|a, b| a.date_ts.cmp(&b.date_ts).then_with(|| a.uid.cmp(&b.uid)));
            }
            MessageSort::SenderAsc => {
                items.sort_by(|a, b| {
                    a.sender
                        .to_lowercase()
                        .cmp(&b.sender.to_lowercase())
                        .then_with(|| b.date_ts.cmp(&a.date_ts))
                });
            }
            MessageSort::SenderDesc => {
                items.sort_by(|a, b| {
                    b.sender
                        .to_lowercase()
                        .cmp(&a.sender.to_lowercase())
                        .then_with(|| b.date_ts.cmp(&a.date_ts))
                });
            }
            MessageSort::SubjectAsc => {
                items.sort_by(|a, b| {
                    a.subject
                        .to_lowercase()
                        .cmp(&b.subject.to_lowercase())
                        .then_with(|| b.date_ts.cmp(&a.date_ts))
                });
            }
            MessageSort::SubjectDesc => {
                items.sort_by(|a, b| {
                    b.subject
                        .to_lowercase()
                        .cmp(&a.subject.to_lowercase())
                        .then_with(|| b.date_ts.cmp(&a.date_ts))
                });
            }
            MessageSort::UnreadFirst => {
                items.sort_by(|a, b| {
                    b.unread
                        .cmp(&a.unread)
                        .then_with(|| b.date_ts.cmp(&a.date_ts))
                });
            }
            MessageSort::StarredFirst => {
                items.sort_by(|a, b| {
                    b.flagged
                        .cmp(&a.flagged)
                        .then_with(|| b.date_ts.cmp(&a.date_ts))
                });
            }
            MessageSort::AccountAsc => {
                items.sort_by(|a, b| {
                    a.account_id
                        .cmp(&b.account_id)
                        .then_with(|| b.date_ts.cmp(&a.date_ts))
                });
            }
            MessageSort::AccountDesc => {
                items.sort_by(|a, b| {
                    b.account_id
                        .cmp(&a.account_id)
                        .then_with(|| b.date_ts.cmp(&a.date_ts))
                });
            }
            MessageSort::AttachmentFirst => {
                items.sort_by(|a, b| {
                    b.has_attachment
                        .cmp(&a.has_attachment)
                        .then_with(|| b.date_ts.cmp(&a.date_ts))
                });
            }
            MessageSort::ImportanceFirst => {
                items.sort_by(|a, b| {
                    b.importance
                        .cmp(&a.importance)
                        .then_with(|| b.date_ts.cmp(&a.date_ts))
                });
            }
            MessageSort::TagsAsc => {
                items.sort_by(|a, b| {
                    a.tags
                        .join(",")
                        .to_lowercase()
                        .cmp(&b.tags.join(",").to_lowercase())
                        .then_with(|| b.date_ts.cmp(&a.date_ts))
                });
            }
        }
    }

    /// WP-08: Return thread child summaries for inline expansion in demo mode.
    pub fn list_thread_summaries(&self, head_message_id: u32) -> Vec<MessageSummary> {
        let Some(detail) = self.get_message_detail(head_message_id) else {
            return Vec::new();
        };
        if detail.thread_messages.len() <= 1 {
            return Vec::new();
        }
        detail
            .thread_messages
            .iter()
            .skip(1)
            .map(|tm| MessageSummary {
                id: tm.id,
                account_id: detail.account_id,
                folder_id: detail.folder_id,
                folder_path: if tm.folder_path.is_empty() {
                    detail.folder_path.clone()
                } else {
                    tm.folder_path.clone()
                },
                uid: tm.uid,
                sender: tm.sender.clone(),
                subject: if tm.subject.is_empty() {
                    detail.subject.clone()
                } else {
                    tm.subject.clone()
                },
                preview: tm.body_text.lines().next().unwrap_or("").trim().to_string(),
                date_ts: tm.date_ts,
                date_display: tm.date_display.clone(),
                unread: tm.unread,
                flagged: tm.flagged,
                has_attachment: !tm.attachments.is_empty(),
                recipients: tm.recipients.clone(),
                thread_count: 1,
                importance: 0,
                tags: Vec::new(),
                answered: false,
                forwarded: false,
                snoozed_until_ts: 0,
            })
            .collect()
    }

    /// WP-08: Update tags on a demo message.
    pub fn set_message_tags(&mut self, message_id: u32, tags: Vec<String>) {
        for list in [
            &mut self.inbox_messages,
            &mut self.sent_messages,
            &mut self.draft_messages,
            &mut self.archive_messages,
        ] {
            if let Some(m) = list.iter_mut().find(|m| m.id == message_id) {
                m.tags = tags;
                return;
            }
        }
    }

    /// WP-08: Snooze or unsnooze a demo message.
    pub fn snooze_message(&mut self, message_id: u32, until_ts: i64) {
        for list in [
            &mut self.inbox_messages,
            &mut self.sent_messages,
            &mut self.draft_messages,
            &mut self.archive_messages,
        ] {
            if let Some(m) = list.iter_mut().find(|m| m.id == message_id) {
                m.snoozed_until_ts = until_ts;
                return;
            }
        }
    }

    /// Return sample cached attachments for the gallery view in `--demo` mode.
    pub fn list_cached_attachments(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Vec<CachedAttachment> {
        let q = query.trim().to_lowercase();
        self.inbox_messages
            .iter()
            .take(120)
            .filter(|m| m.has_attachment)
            .flat_map(|m| {
                [
                    CachedAttachment {
                        id: m.id * 10 + 1,
                        message_id: m.id,
                        account_id: 1,
                        filename: "windows-baseline-notes.txt".to_string(),
                        mime_type: "text/plain".to_string(),
                        size_bytes: 12_288,
                        sender: m.sender.clone(),
                        subject: m.subject.clone(),
                        date_display: m.date_display.clone(),
                        date_ts: m.date_ts,
                    },
                    CachedAttachment {
                        id: m.id * 10 + 2,
                        message_id: m.id,
                        account_id: 1,
                        filename: "三栏布局截图.png".to_string(),
                        mime_type: "image/png".to_string(),
                        size_bytes: 188_416,
                        sender: m.sender.clone(),
                        subject: m.subject.clone(),
                        date_display: m.date_display.clone(),
                        date_ts: m.date_ts,
                    },
                ]
            })
            .filter(|att| {
                q.is_empty()
                    || att.filename.to_lowercase().contains(&q)
                    || att.sender.to_lowercase().contains(&q)
                    || att.subject.to_lowercase().contains(&q)
            })
            .skip(offset)
            .take(limit)
            .collect()
    }

    /// Return aggregated contacts for the contacts view in `--demo` mode.
    pub fn list_known_contacts(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Vec<KnownContact> {
        let q = query.trim().to_lowercase();
        let mut counts: BTreeMap<String, (String, usize)> = BTreeMap::new();
        for msg in self.inbox_messages.iter().take(400) {
            let (name, email) =
                if let (Some(o), Some(c)) = (msg.sender.find('<'), msg.sender.rfind('>')) {
                    (
                        msg.sender[..o].trim().to_string(),
                        msg.sender[o + 1..c].trim().to_lowercase(),
                    )
                } else {
                    (msg.sender.clone(), msg.sender.to_lowercase())
                };
            if email.is_empty() {
                continue;
            }
            let entry = counts.entry(email).or_insert_with(|| (name, 0));
            entry.1 += 1;
        }
        let mut items: Vec<KnownContact> = counts
            .into_iter()
            .map(|(email, (name, message_count))| {
                // F-5：演示数据中把「产品设计组」标注为自定义联系人（带备注与
                // 来源徽标，CRUD 按钮在演示模式下禁用）。
                let (id, notes, custom) = if email == "design@hylki.example" {
                    (1u32, "产品设计组通讯录（演示）".to_string(), true)
                } else {
                    (0u32, String::new(), false)
                };
                KnownContact {
                    name,
                    email,
                    message_count,
                    id,
                    notes,
                    custom,
                    ..KnownContact::default()
                }
            })
            .filter(|c| {
                q.is_empty()
                    || c.name.to_lowercase().contains(&q)
                    || c.email.to_lowercase().contains(&q)
            })
            .collect();
        // 与 cache 合并排序一致：自定义联系人优先，其余按往来邮件数降序。
        items.sort_by(|a, b| {
            b.custom
                .cmp(&a.custom)
                .then_with(|| b.message_count.cmp(&a.message_count))
                .then_with(|| a.name.cmp(&b.name))
        });
        items.into_iter().skip(offset).take(limit).collect()
    }

    /// Return full message details for `message_id`.
    pub fn get_message_detail(&self, message_id: u32) -> Option<MessageDetail> {
        let summary = self
            .inbox_messages
            .iter()
            .chain(self.sent_messages.iter())
            .chain(self.draft_messages.iter())
            .chain(self.archive_messages.iter())
            .find(|m| m.id == message_id)?;

        let attachments = if summary.has_attachment {
            vec![
                AttachmentMeta {
                    id: summary.id * 10 + 1,
                    filename: "windows-baseline-notes.txt".to_string(),
                    mime_type: "text/plain".to_string(),
                    size_bytes: 12_288,
                },
                AttachmentMeta {
                    id: summary.id * 10 + 2,
                    filename: "三栏布局截图.png".to_string(),
                    mime_type: "image/png".to_string(),
                    size_bytes: 188_416,
                },
            ]
        } else {
            Vec::new()
        };

        let body_text = format!(
            "你好，\n\n这是演示邮件「{}」的正文内容（编号 #{}）。\n\n\
            当前页面用于验证 Hylki Windows 移植的核心能力：\n\
            1. 左栏账号与文件夹切换，中栏分页列表（收件箱共 {} 封模拟邮件），右栏正文阅读；\n\
            2. 多行中文输入、IME 候选框定位与剪贴板粘贴；\n\
            3. 支持切换至真实邮箱账号，完成 IMAP 收信、SMTP 发信与本地缓存离线阅读。\n\n\
            摘要：{}\n",
            summary.subject, summary.id, DEMO_INBOX_TOTAL, summary.preview
        );

        let folder_path = match summary.folder_id {
            DEMO_INBOX_ID => "INBOX",
            DEMO_SENT_ID => "Sent",
            DEMO_DRAFTS_ID => "Drafts",
            DEMO_ARCHIVE_ID => "Archive",
            _ => "INBOX",
        }
        .to_string();

        let demo_html = match summary.id {
            1 => Some(
                "<h3>Windows 移植阶段 1 构建基线确认</h3>\
                <p>你好，</p>\
                <p>请确认以下三项核心指标已通过验证：</p>\
                <ul>\
                  <li><strong>MSVC 工具链</strong>：独立 <code>desktop/Cargo.toml</code> 工作区编译通过</li>\
                  <li><strong>中文输入与复制</strong>：阅读器正文支持鼠标选中与 <code>Ctrl+C</code> 复制</li>\
                  <li><strong>隐私保护</strong>：默认拦截远程跟踪像素 <img src=\"https://tracker.hylki.example/pixel.png\"></li>\
                </ul>\
                <p>详细文档见 <a href=\"https://github.com/hyprlab/hylki\">Hylki 上游仓库</a>。</p>"
                    .to_string(),
            ),
            3 => Some(
                "<p>三栏布局已完成 <strong>48px 顶栏对齐</strong> 与亮/暗主题语义色板统一。</p>\
                <blockquote><p>左栏柔和选中态 + 中栏主蓝高亮 + 右栏卡片式会话阅读。</p></blockquote>"
                    .to_string(),
            ),
            _ => None,
        };

        // 为演示邮件 #1 与 #3 构造多封往来会话（默认最新在前降序），便于在 --demo 直观体验线程卡片阅读。
        let thread_messages = match summary.id {
            1 => vec![
                ThreadMessage {
                    id: summary.id,
                    rfc_message_id: format!("<demo-{}@hylki.example>", summary.id),
                    subject: summary.subject.clone(),
                    sender: summary.sender.clone(),
                    recipients: "demo@hylki.example, 陈思远 <siyuan@example.org>".to_string(),
                    date_ts: summary.date_ts,
                    date_display: summary.date_display.clone(),
                    body_text: body_text.clone(),
                    body_html: demo_html.clone(),
                    body_loaded: true,
                    unread: summary.unread,
                    flagged: summary.flagged,
                    attachments: attachments.clone(),
                    uid: summary.uid,
                    folder_path: folder_path.clone(),
                },
                ThreadMessage {
                    id: 90_001,
                    rfc_message_id: "<demo-90001@hylki.example>".to_string(),
                    subject: summary.subject.clone(),
                    sender: "陈思远 <siyuan@example.org>".to_string(),
                    recipients: "林晓明 <xiaoming@example.com>, demo@hylki.example".to_string(),
                    date_ts: summary.date_ts.saturating_sub(7200),
                    date_display: "13:10".to_string(),
                    body_text: "我这边已经用 MSVC 工具链验证完 x86_64-pc-windows-msvc 独立工作区编译，依赖树完全隔离了 GTK 和 Wayland。".to_string(),
                    body_html: None,
                    body_loaded: true,
                    unread: false,
                    flagged: false,
                    attachments: Vec::new(),
                    uid: 90_001,
                    folder_path: folder_path.clone(),
                },
            ],
            3 => vec![
                ThreadMessage {
                    id: summary.id,
                    rfc_message_id: format!("<demo-{}@hylki.example>", summary.id),
                    subject: summary.subject.clone(),
                    sender: summary.sender.clone(),
                    recipients: "demo@hylki.example".to_string(),
                    date_ts: summary.date_ts,
                    date_display: summary.date_display.clone(),
                    body_text: body_text.clone(),
                    body_html: demo_html.clone(),
                    body_loaded: true,
                    unread: summary.unread,
                    flagged: summary.flagged,
                    attachments: attachments.clone(),
                    uid: summary.uid,
                    folder_path: folder_path.clone(),
                },
                ThreadMessage {
                    id: 90_002,
                    rfc_message_id: "<demo-90002@hylki.example>".to_string(),
                    subject: summary.subject.clone(),
                    sender: "林晓明 <xiaoming@example.com>".to_string(),
                    recipients: "产品设计组 <design@hylki.example>".to_string(),
                    date_ts: summary.date_ts.saturating_sub(172_800),
                    date_display: "前天 16:40".to_string(),
                    body_text: "请设计组同步一份三栏间距与亮/暗主题语义色板，我们按 48px 顶栏对齐。".to_string(),
                    body_html: None,
                    body_loaded: true,
                    unread: false,
                    flagged: false,
                    attachments: Vec::new(),
                    uid: 90_002,
                    folder_path: folder_path.clone(),
                },
            ],
            _ => Vec::new(),
        };

        // F-7：演示邮件 #2 携带 `List-Unsubscribe` / `List-Unsubscribe-Post`
        // 头样例，用于在 --demo 模式下展示一键退订横幅。
        let (unsubscribe_url, unsubscribe_one_click) = if summary.id == 2 {
            (
                Some("https://lists.hylki.example/unsubscribe?id=demo".to_string()),
                true,
            )
        } else {
            (None, false)
        };

        Some(MessageDetail {
            id: summary.id,
            account_id: 1,
            folder_id: summary.folder_id,
            folder_path,
            uid: summary.uid,
            rfc_message_id: format!("<demo-{}@hylki.example>", summary.id),
            sender: summary.sender.clone(),
            recipients: "demo@hylki.example".to_string(),
            subject: summary.subject.clone(),
            date_ts: summary.date_ts,
            date_display: summary.date_display.clone(),
            body_text,
            body_html: demo_html,
            flagged: summary.flagged,
            attachments,
            thread_messages,
            unsubscribe_url,
            unsubscribe_one_click,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_store_has_ten_thousand_inbox_messages_and_paginates() {
        let store = DemoStore::new();
        let page0 = store.list_messages_page(DEMO_INBOX_ID, "", 0, 50);
        assert_eq!(page0.total, 10_000);
        assert_eq!(page0.items.len(), 50);
        assert_eq!(page0.items[0].id, 1);

        let last_page = store.list_messages_page(DEMO_INBOX_ID, "", 9_980, 50);
        assert_eq!(last_page.total, 10_000);
        assert_eq!(last_page.items.len(), 20);
        assert_eq!(last_page.items[19].id, 10_000);

        let detail = store
            .get_message_detail(1)
            .expect("first demo message should exist");
        assert!(!detail.attachments.is_empty());
        assert!(detail.body_text.contains("Hylki Windows"));
    }

    #[test]
    fn demo_store_supports_filters_starred_attachments_and_contacts() {
        let store = DemoStore::new();
        let unread =
            store.list_messages_page_filtered(DEMO_INBOX_ID, "", MessageFilter::Unread, 0, 50);
        assert_eq!(unread.total, 126);
        assert!(unread.items.iter().all(|m| m.unread));

        let starred = store.list_starred_page("", MessageFilter::All, 0, 50);
        assert!(starred.total >= 2);
        assert!(starred.items.iter().all(|m| m.flagged));

        let attachments = store.list_cached_attachments("", 0, 50);
        assert!(!attachments.is_empty());

        let contacts = store.list_known_contacts("", 0, 50);
        assert!(!contacts.is_empty());
        // F-5：自定义联系人排在最前；聚合联系人按往来邮件数降序。
        let aggregated: Vec<_> = contacts.iter().filter(|c| !c.custom).collect();
        assert!(!aggregated.is_empty());
        assert!(aggregated
            .windows(2)
            .all(|w| w[0].message_count >= w[1].message_count));
        if contacts[0].custom {
            assert!(contacts.iter().skip_while(|c| c.custom).all(|c| !c.custom));
        }
    }
}
