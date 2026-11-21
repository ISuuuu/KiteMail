//! Deterministic demo dataset for `--demo` mode and list pagination verification.

use crate::{
    AccountSummary, AttachmentMeta, FolderSummary, MailPage, MessageDetail, MessageSummary,
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
            },
            FolderSummary {
                id: DEMO_SENT_ID,
                account_id: 1,
                name: "已发送".to_string(),
                path: "Sent".to_string(),
                unread_count: 0,
                total_count: 12,
            },
            FolderSummary {
                id: DEMO_DRAFTS_ID,
                account_id: 1,
                name: "草稿箱".to_string(),
                path: "Drafts".to_string(),
                unread_count: 0,
                total_count: 3,
            },
            FolderSummary {
                id: DEMO_ARCHIVE_ID,
                account_id: 1,
                name: "归档".to_string(),
                path: "Archive".to_string(),
                unread_count: 0,
                total_count: 24,
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

        for (idx, (sender, subject, preview, date_display, unread, has_attachment)) in
            pinned_samples.iter().enumerate()
        {
            let id = (idx + 1) as u32;
            inbox_messages.push(MessageSummary {
                id,
                account_id: 1,
                folder_id: DEMO_INBOX_ID,
                folder_path: "INBOX".to_string(),
                uid: id,
                sender: (*sender).to_string(),
                subject: (*subject).to_string(),
                preview: (*preview).to_string(),
                date_display: (*date_display).to_string(),
                unread: *unread,
                flagged: idx == 0,
                has_attachment: *has_attachment,
            });
        }

        for idx in pinned_samples.len()..DEMO_INBOX_TOTAL {
            let id = (idx + 1) as u32;
            let day = (idx % 28) + 1;
            let hour = (idx * 7) % 24;
            let minute = (idx * 13) % 60;
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
                date_display: format!("11-{day:02} {hour:02}:{minute:02}"),
                unread: idx < 128,
                flagged: false,
                has_attachment: idx % 9 == 0,
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
                    date_display: format!("11-{:02}", 20 - i),
                    unread: false,
                    flagged: false,
                    has_attachment: i % 3 == 0,
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
                    date_display: "草稿".to_string(),
                    unread: false,
                    flagged: false,
                    has_attachment: false,
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
                    date_display: "10-15".to_string(),
                    unread: false,
                    flagged: false,
                    has_attachment: false,
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

    /// Return a slice of messages in `folder_id` starting at `offset` with at most `limit` rows.
    pub fn list_messages_page(
        &self,
        folder_id: u32,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> MailPage {
        let all = self.messages_for_folder(folder_id);
        let q = query.trim().to_lowercase();
        let filtered: Vec<MessageSummary> = if q.is_empty() {
            let total = all.len();
            let start = offset.min(total);
            let end = start.saturating_add(limit).min(total);
            return MailPage {
                folder_id,
                offset: start,
                total,
                items: all[start..end].to_vec(),
            };
        } else {
            all.iter()
                .filter(|m| {
                    m.subject.to_lowercase().contains(&q) || m.sender.to_lowercase().contains(&q)
                })
                .cloned()
                .collect()
        };

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
            date_display: summary.date_display.clone(),
            body_text,
            flagged: summary.flagged,
            attachments,
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
}
