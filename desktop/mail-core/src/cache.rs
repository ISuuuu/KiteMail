//! SQLite local cache for folders, message headers/bodies, attachments, and outbox journal.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::{
    AttachmentMeta, FolderSummary, LocalDraft, MailPage, MessageDetail, MessageFilter,
    MessageSummary, OutboxItem, OutboxStatus, ThreadMessage,
};

/// Remote folder metadata returned by IMAP LIST/STATUS.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteFolder {
    pub name: String,
    pub path: String,
    pub unread_count: u32,
    pub total_count: usize,
}

/// Parsed message header batch item ready to commit into the cache.
#[derive(Clone, Debug)]
pub struct CachedHeaderInput {
    pub uid: u32,
    pub message_id: String,
    pub sender: String,
    pub recipients: String,
    pub subject: String,
    pub preview: String,
    pub date_ts: i64,
    pub date_display: String,
    pub unread: bool,
    pub flagged: bool,
    pub has_attachment: bool,
}

/// Attachment payload extracted from a fetched RFC 822 message.
#[derive(Clone, Debug)]
pub struct CachedAttachmentInput {
    pub filename: String,
    pub mime_type: String,
    pub content: Vec<u8>,
}

/// Cached attachment joined with its owning message for the gallery view.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedAttachment {
    pub id: u32,
    pub message_id: u32,
    pub account_id: u32,
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: usize,
    pub sender: String,
    pub subject: String,
    pub date_display: String,
}

/// Contact aggregated from cached senders/recipients for the contacts view.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownContact {
    pub name: String,
    pub email: String,
    pub message_count: usize,
}

/// SQL predicate matching every account's Sent folder by friendly name or raw path.
const SENT_FOLDER_MATCH: &str = "(f.name = '已发送' OR UPPER(f.path) LIKE '%SENT%')";

/// SQL predicate matching every account's Archive folder by friendly name or raw path.
const ARCHIVE_FOLDER_MATCH: &str = "(f.name = '归档' OR UPPER(f.path) LIKE '%ARCHIVE%')";

/// SQL predicate matching every account's Trash/Deleted folder by friendly name or raw path.
const TRASH_FOLDER_MATCH: &str =
    "(f.name = '已删除' OR UPPER(f.path) LIKE '%TRASH%' OR UPPER(f.path) LIKE '%DELETED%')";

/// Normalize a subject line for conversation grouping by stripping common
/// reply/forward prefixes (`Re:`, `Fwd:`, `Fw:`, `回复:`, `转发:`).
pub fn normalize_thread_subject(subject: &str) -> String {
    let mut s = subject.trim();
    loop {
        let lower = s.to_ascii_lowercase();
        let stripped = if lower.starts_with("re:") {
            Some(&s[3..])
        } else if lower.starts_with("fwd:") {
            Some(&s[4..])
        } else if lower.starts_with("fw:") {
            Some(&s[3..])
        } else if let Some(rest) = s.strip_prefix("回复：") {
            Some(rest)
        } else if let Some(rest) = s.strip_prefix("回复:") {
            Some(rest)
        } else if let Some(rest) = s.strip_prefix("转发：") {
            Some(rest)
        } else if let Some(rest) = s.strip_prefix("转发:") {
            Some(rest)
        } else {
            None
        };
        match stripped {
            Some(rest) => s = rest.trim_start(),
            None => break,
        }
    }
    s.trim().to_lowercase()
}

/// Persistent SQLite mail cache.
pub struct CacheStore {
    conn: Connection,
}

impl CacheStore {
    /// Open (or create) the persistent SQLite cache at `path`.
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("创建缓存目录失败 ({}): {e}", parent.display()))?;
        }
        let conn = Connection::open(path)
            .map_err(|e| format!("打开缓存数据库失败 ({}): {e}", path.display()))?;
        let store = Self { conn };
        store.configure_and_migrate()?;
        Ok(store)
    }

    /// Open an isolated in-memory cache database for unit tests.
    pub fn open_in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| format!("打开内存缓存失败: {e}"))?;
        let store = Self { conn };
        store.configure_and_migrate()?;
        Ok(store)
    }

    fn configure_and_migrate(&self) -> Result<(), String> {
        self.conn
            .execute_batch(
                "
                PRAGMA journal_mode = WAL;
                PRAGMA foreign_keys = ON;
                PRAGMA synchronous = NORMAL;

                CREATE TABLE IF NOT EXISTS folders (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    account_id INTEGER NOT NULL,
                    name TEXT NOT NULL,
                    path TEXT NOT NULL,
                    uid_validity INTEGER NOT NULL DEFAULT 0,
                    unread_count INTEGER NOT NULL DEFAULT 0,
                    total_count INTEGER NOT NULL DEFAULT 0,
                    is_inbox INTEGER NOT NULL DEFAULT 0,
                    UNIQUE(account_id, path)
                );

                CREATE TABLE IF NOT EXISTS messages (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    account_id INTEGER NOT NULL,
                    folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
                    uid INTEGER NOT NULL,
                    remote_id TEXT NOT NULL DEFAULT '',
                    message_id TEXT NOT NULL DEFAULT '',
                    sender TEXT NOT NULL,
                    recipients TEXT NOT NULL DEFAULT '',
                    subject TEXT NOT NULL,
                    preview TEXT NOT NULL DEFAULT '',
                    date_ts INTEGER NOT NULL DEFAULT 0,
                    date_display TEXT NOT NULL DEFAULT '',
                    unread INTEGER NOT NULL DEFAULT 1,
                    flagged INTEGER NOT NULL DEFAULT 0,
                    has_attachment INTEGER NOT NULL DEFAULT 0,
                    body_text TEXT,
                    UNIQUE(folder_id, uid)
                );

                CREATE INDEX IF NOT EXISTS idx_messages_folder_date
                    ON messages(folder_id, date_ts DESC, uid DESC);

                CREATE TABLE IF NOT EXISTS attachments (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
                    filename TEXT NOT NULL,
                    mime_type TEXT NOT NULL,
                    size_bytes INTEGER NOT NULL,
                    content BLOB NOT NULL
                );

                CREATE TABLE IF NOT EXISTS outbox (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    account_id INTEGER NOT NULL,
                    to_addr TEXT NOT NULL,
                    cc_addr TEXT NOT NULL DEFAULT '',
                    bcc_addr TEXT NOT NULL DEFAULT '',
                    subject TEXT NOT NULL,
                    body_text TEXT NOT NULL,
                    in_reply_to TEXT NOT NULL DEFAULT '',
                    attachment_paths_json TEXT NOT NULL DEFAULT '[]',
                    status TEXT NOT NULL DEFAULT 'queued',
                    last_error TEXT NOT NULL DEFAULT '',
                    created_at INTEGER NOT NULL
                );

                CREATE TABLE IF NOT EXISTS drafts (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    account_id INTEGER NOT NULL,
                    to_addr TEXT NOT NULL,
                    cc_addr TEXT NOT NULL DEFAULT '',
                    bcc_addr TEXT NOT NULL DEFAULT '',
                    subject TEXT NOT NULL,
                    body_text TEXT NOT NULL,
                    in_reply_to TEXT NOT NULL DEFAULT '',
                    attachment_paths_json TEXT NOT NULL DEFAULT '[]',
                    updated_at INTEGER NOT NULL
                );
                ",
            )
            .map_err(|e| format!("初始化缓存表结构失败: {e}"))?;
        self.ensure_column("folders", "is_inbox", "INTEGER NOT NULL DEFAULT 0")?;
        self.ensure_column("messages", "remote_id", "TEXT NOT NULL DEFAULT ''")?;
        // Cc/Bcc（迭代 2）：旧库无列时补充，历史数据视为空字符串。
        self.ensure_column("outbox", "cc_addr", "TEXT NOT NULL DEFAULT ''")?;
        self.ensure_column("outbox", "bcc_addr", "TEXT NOT NULL DEFAULT ''")?;
        self.ensure_column("drafts", "cc_addr", "TEXT NOT NULL DEFAULT ''")?;
        self.ensure_column("drafts", "bcc_addr", "TEXT NOT NULL DEFAULT ''")?;
        // HTML 正文（迭代 3）：旧库无列时补充，历史数据视为 NULL。
        self.ensure_column("messages", "body_html", "TEXT")?;
        Ok(())
    }

    fn ensure_column(&self, table: &str, col: &str, def: &str) -> Result<(), String> {
        let mut stmt = self
            .conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .map_err(|e| format!("读取表结构失败: {e}"))?;
        let exists = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|e| format!("遍历表字段失败: {e}"))?
            .filter_map(Result::ok)
            .any(|name| name == col);
        if !exists {
            self.conn
                .execute(&format!("ALTER TABLE {table} ADD COLUMN {col} {def}"), [])
                .map_err(|e| format!("迁移字段 {table}.{col} 失败: {e}"))?;
        }
        Ok(())
    }

    /// Upsert folder list for `account_id` and return the resulting local folder summaries.
    pub fn sync_folders(
        &mut self,
        account_id: u32,
        remote_folders: &[RemoteFolder],
    ) -> Result<Vec<FolderSummary>, String> {
        let with_flags: Vec<(RemoteFolder, bool)> = remote_folders
            .iter()
            .map(|f| {
                let is_inbox = f.path.eq_ignore_ascii_case("INBOX");
                (f.clone(), is_inbox)
            })
            .collect();
        self.sync_folders_with_flags(account_id, &with_flags)
    }

    /// Upsert folder list for `account_id` with explicit `is_inbox` flags.
    pub fn sync_folders_with_flags(
        &mut self,
        account_id: u32,
        remote_folders: &[(RemoteFolder, bool)],
    ) -> Result<Vec<FolderSummary>, String> {
        let tx = self
            .conn
            .transaction()
            .map_err(|e| format!("开启文件夹同步事务失败: {e}"))?;

        for (folder, is_inbox) in remote_folders {
            tx.execute(
                "
                INSERT INTO folders (account_id, name, path, unread_count, total_count, is_inbox)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                ON CONFLICT(account_id, path) DO UPDATE SET
                    name = excluded.name,
                    unread_count = excluded.unread_count,
                    total_count = excluded.total_count,
                    is_inbox = excluded.is_inbox
                ",
                params![
                    account_id,
                    folder.name,
                    folder.path,
                    folder.unread_count,
                    folder.total_count as i64,
                    *is_inbox as i32
                ],
            )
            .map_err(|e| format!("写入文件夹失败 ({}): {e}", folder.path))?;
        }

        tx.commit()
            .map_err(|e| format!("提交文件夹事务失败: {e}"))?;
        self.list_folders(account_id)
    }

    /// List cached folders for `account_id`.
    pub fn list_folders(&self, account_id: u32) -> Result<Vec<FolderSummary>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "
                SELECT id, account_id, name, path, unread_count, total_count
                FROM folders
                WHERE account_id = ?1
                ORDER BY CASE WHEN UPPER(path) = 'INBOX' OR is_inbox = 1 THEN 0 ELSE 1 END, id ASC
                ",
            )
            .map_err(|e| format!("查询文件夹失败: {e}"))?;

        let rows = stmt
            .query_map(params![account_id], |row| {
                Ok(FolderSummary {
                    id: row.get::<_, u32>(0)?,
                    account_id: row.get::<_, u32>(1)?,
                    name: row.get(2)?,
                    path: row.get(3)?,
                    unread_count: row.get::<_, u32>(4)?,
                    total_count: row.get::<_, i64>(5)?.max(0) as usize,
                })
            })
            .map_err(|e| format!("读取文件夹列表失败: {e}"))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析文件夹行失败: {e}"))
    }

    /// Look up folder path and stored UIDVALIDITY for `folder_id`.
    pub fn folder_meta(&self, folder_id: u32) -> Result<Option<(String, u32)>, String> {
        self.conn
            .query_row(
                "SELECT path, uid_validity FROM folders WHERE id = ?1",
                params![folder_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?)),
            )
            .optional()
            .map_err(|e| format!("读取文件夹元数据失败: {e}"))
    }

    /// Commit a batch of synced headers and folder UIDVALIDITY in a single transaction.
    pub fn commit_folder_headers(
        &mut self,
        account_id: u32,
        folder_id: u32,
        uid_validity: u32,
        headers: &[CachedHeaderInput],
        prune_missing_uids: Option<&[u32]>,
    ) -> Result<(), String> {
        let wrapped: Vec<(CachedHeaderInput, String)> =
            headers.iter().map(|h| (h.clone(), String::new())).collect();
        self.commit_folder_headers_with_remote(
            account_id,
            folder_id,
            uid_validity,
            &wrapped,
            prune_missing_uids,
        )
    }

    /// Commit a batch of synced headers (with optional Graph `remote_id`) and folder UIDVALIDITY.
    pub fn commit_folder_headers_with_remote(
        &mut self,
        account_id: u32,
        folder_id: u32,
        uid_validity: u32,
        headers: &[(CachedHeaderInput, String)],
        prune_missing_uids: Option<&[u32]>,
    ) -> Result<(), String> {
        let tx = self
            .conn
            .transaction()
            .map_err(|e| format!("开启邮件同步事务失败: {e}"))?;

        let previous_validity: u32 = tx
            .query_row(
                "SELECT uid_validity FROM folders WHERE id = ?1",
                params![folder_id],
                |row| row.get(0),
            )
            .map_err(|e| format!("读取文件夹 UIDVALIDITY 失败: {e}"))?;

        if previous_validity != 0 && previous_validity != uid_validity {
            tx.execute(
                "DELETE FROM messages WHERE folder_id = ?1",
                params![folder_id],
            )
            .map_err(|e| format!("UIDVALIDITY 变化后清理旧消息失败: {e}"))?;
        }

        if let Some(live_uids) = prune_missing_uids {
            // Remove messages that were deleted or moved on the server.
            let mut stmt = tx
                .prepare("SELECT id, uid FROM messages WHERE folder_id = ?1")
                .map_err(|e| format!("读取现有 UID 失败: {e}"))?;
            let existing: Vec<(u32, u32)> = stmt
                .query_map(params![folder_id], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(|e| format!("遍历现有 UID 失败: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("收集现有 UID 失败: {e}"))?;
            drop(stmt);

            let live_set: std::collections::HashSet<u32> = live_uids.iter().copied().collect();
            for (msg_id, uid) in existing {
                if !live_set.contains(&uid) {
                    tx.execute("DELETE FROM messages WHERE id = ?1", params![msg_id])
                        .map_err(|e| format!("清理服务端已删除邮件失败: {e}"))?;
                }
            }
        }

        for (item, remote_id) in headers {
            tx.execute(
                "
                INSERT INTO messages (
                    account_id, folder_id, uid, remote_id, message_id, sender, recipients,
                    subject, preview, date_ts, date_display, unread, flagged, has_attachment
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                ON CONFLICT(folder_id, uid) DO UPDATE SET
                    remote_id = CASE
                        WHEN excluded.remote_id = '' THEN messages.remote_id
                        ELSE excluded.remote_id
                    END,
                    message_id = excluded.message_id,
                    sender = excluded.sender,
                    recipients = excluded.recipients,
                    subject = excluded.subject,
                    preview = CASE
                        WHEN messages.body_text IS NULL THEN excluded.preview
                        ELSE messages.preview
                    END,
                    date_ts = excluded.date_ts,
                    date_display = excluded.date_display,
                    unread = excluded.unread,
                    flagged = excluded.flagged,
                    has_attachment = excluded.has_attachment
                ",
                params![
                    account_id,
                    folder_id,
                    item.uid,
                    remote_id,
                    item.message_id,
                    item.sender,
                    item.recipients,
                    item.subject,
                    item.preview,
                    item.date_ts,
                    item.date_display,
                    item.unread as i32,
                    item.flagged as i32,
                    item.has_attachment as i32,
                ],
            )
            .map_err(|e| format!("写入邮件头失败 (UID {}): {e}", item.uid))?;
        }

        let unread_count: u32 = tx
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE folder_id = ?1 AND unread = 1",
                params![folder_id],
                |row| row.get(0),
            )
            .map_err(|e| format!("统计未读数失败: {e}"))?;
        let total_count: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE folder_id = ?1",
                params![folder_id],
                |row| row.get(0),
            )
            .map_err(|e| format!("统计总数失败: {e}"))?;

        tx.execute(
            "
            UPDATE folders
            SET uid_validity = ?1, unread_count = ?2, total_count = ?3
            WHERE id = ?4
            ",
            params![uid_validity, unread_count, total_count, folder_id],
        )
        .map_err(|e| format!("更新文件夹同步进度失败: {e}"))?;

        tx.commit()
            .map_err(|e| format!("提交邮件同步事务失败: {e}"))?;
        Ok(())
    }

    /// SQL filter suffix for [`MessageFilter`].
    fn filter_sql(filter: MessageFilter) -> &'static str {
        match filter {
            MessageFilter::All => "",
            MessageFilter::Unread => " AND m.unread = 1",
            MessageFilter::Starred => " AND m.flagged = 1",
        }
    }

    /// Query a page of cached message summaries for `folder_id`, optionally filtered by `query`.
    pub fn list_messages_page(
        &self,
        folder_id: u32,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<MailPage, String> {
        self.list_messages_page_filtered(folder_id, query, MessageFilter::All, offset, limit)
    }

    /// Query a page of cached message summaries for `folder_id`, applying both `query` and `filter`.
    pub fn list_messages_page_filtered(
        &self,
        folder_id: u32,
        query: &str,
        filter: MessageFilter,
        offset: usize,
        limit: usize,
    ) -> Result<MailPage, String> {
        let trimmed = query.trim();
        let extra = Self::filter_sql(filter);
        let (total, items) = if trimmed.is_empty() {
            let total: i64 = self
                .conn
                .query_row(
                    &format!("SELECT COUNT(*) FROM messages m WHERE m.folder_id = ?1{extra}"),
                    params![folder_id],
                    |row| row.get(0),
                )
                .map_err(|e| format!("统计邮件分页总数失败: {e}"))?;

            let mut stmt = self
                .conn
                .prepare(&format!(
                    "
                    SELECT m.id, m.account_id, m.folder_id, f.path, m.uid, m.sender, m.subject,
                           m.preview, m.date_display, m.unread, m.flagged, m.has_attachment
                    FROM messages m
                    JOIN folders f ON f.id = m.folder_id
                    WHERE m.folder_id = ?1{extra}
                    ORDER BY m.date_ts DESC, m.uid DESC
                    LIMIT ?2 OFFSET ?3
                    "
                ))
                .map_err(|e| format!("准备分页查询失败: {e}"))?;

            let rows = stmt
                .query_map(
                    params![folder_id, limit as i64, offset as i64],
                    map_summary_row,
                )
                .map_err(|e| format!("执行分页查询失败: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("解析分页结果失败: {e}"))?;

            (total.max(0) as usize, rows)
        } else {
            let pattern = format!("%{trimmed}%");
            let total: i64 = self
                .conn
                .query_row(
                    &format!(
                        "
                        SELECT COUNT(*) FROM messages m
                        WHERE m.folder_id = ?1{extra} AND (m.subject LIKE ?2 OR m.sender LIKE ?2)
                        "
                    ),
                    params![folder_id, pattern],
                    |row| row.get(0),
                )
                .map_err(|e| format!("统计搜索结果总数失败: {e}"))?;

            let mut stmt = self
                .conn
                .prepare(&format!(
                    "
                    SELECT m.id, m.account_id, m.folder_id, f.path, m.uid, m.sender, m.subject,
                           m.preview, m.date_display, m.unread, m.flagged, m.has_attachment
                    FROM messages m
                    JOIN folders f ON f.id = m.folder_id
                    WHERE m.folder_id = ?1{extra} AND (m.subject LIKE ?2 OR m.sender LIKE ?2)
                    ORDER BY m.date_ts DESC, m.uid DESC
                    LIMIT ?3 OFFSET ?4
                    "
                ))
                .map_err(|e| format!("准备搜索分页查询失败: {e}"))?;

            let rows = stmt
                .query_map(
                    params![folder_id, pattern, limit as i64, offset as i64],
                    map_summary_row,
                )
                .map_err(|e| format!("执行搜索分页查询失败: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("解析搜索结果失败: {e}"))?;

            (total.max(0) as usize, rows)
        };

        Ok(MailPage {
            folder_id,
            offset,
            total,
            items,
        })
    }

    /// Query a unified inbox page across all accounts (`UPPER(f.path) = 'INBOX'`),
    /// preserving each message's owning `account_id` and `folder_path`.
    pub fn list_unified_inbox(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<MailPage, String> {
        self.list_unified_inbox_filtered(query, MessageFilter::All, offset, limit)
    }

    /// Query a unified inbox page across all accounts with [`MessageFilter`].
    pub fn list_unified_inbox_filtered(
        &self,
        query: &str,
        filter: MessageFilter,
        offset: usize,
        limit: usize,
    ) -> Result<MailPage, String> {
        self.list_role_messages(
            "(UPPER(f.path) = 'INBOX' OR f.is_inbox = 1)",
            "统一收件箱",
            query,
            filter,
            offset,
            limit,
        )
    }

    /// Query a page of starred (flagged) messages across all accounts and folders.
    pub fn list_starred_messages(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<MailPage, String> {
        self.list_starred_messages_filtered(query, MessageFilter::All, offset, limit)
    }

    /// Query a page of starred (flagged) messages with [`MessageFilter`].
    pub fn list_starred_messages_filtered(
        &self,
        query: &str,
        filter: MessageFilter,
        offset: usize,
        limit: usize,
    ) -> Result<MailPage, String> {
        self.list_role_messages("m.flagged = 1", "星标邮件", query, filter, offset, limit)
    }

    /// Query a page of messages aggregated across every account's Sent folder
    /// (matched by friendly name or raw IMAP/Graph path, like [`Self::list_unified_inbox`]).
    pub fn list_sent_messages(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<MailPage, String> {
        self.list_sent_messages_filtered(query, MessageFilter::All, offset, limit)
    }

    /// Query a page of sent messages across all accounts with [`MessageFilter`].
    pub fn list_sent_messages_filtered(
        &self,
        query: &str,
        filter: MessageFilter,
        offset: usize,
        limit: usize,
    ) -> Result<MailPage, String> {
        self.list_role_messages(SENT_FOLDER_MATCH, "已发送", query, filter, offset, limit)
    }

    /// Query a page of messages aggregated across every account's Archive folder.
    pub fn list_archived_messages(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<MailPage, String> {
        self.list_archived_messages_filtered(query, MessageFilter::All, offset, limit)
    }

    /// Query a page of archived messages across all accounts with [`MessageFilter`].
    pub fn list_archived_messages_filtered(
        &self,
        query: &str,
        filter: MessageFilter,
        offset: usize,
        limit: usize,
    ) -> Result<MailPage, String> {
        self.list_role_messages(ARCHIVE_FOLDER_MATCH, "归档", query, filter, offset, limit)
    }

    /// Shared implementation for unified message pages: `predicate` is a trusted
    /// constant SQL fragment applied to the `messages JOIN folders` row set.
    fn list_role_messages(
        &self,
        predicate: &str,
        label: &str,
        query: &str,
        filter: MessageFilter,
        offset: usize,
        limit: usize,
    ) -> Result<MailPage, String> {
        let trimmed = query.trim();
        let extra = Self::filter_sql(filter);
        let (total, items) = if trimmed.is_empty() {
            let total: i64 = self
                .conn
                .query_row(
                    &format!(
                        "
                        SELECT COUNT(*)
                        FROM messages m
                        JOIN folders f ON f.id = m.folder_id
                        WHERE {predicate}{extra}
                        "
                    ),
                    [],
                    |row| row.get(0),
                )
                .map_err(|e| format!("统计{label}总数失败: {e}"))?;

            let mut stmt = self
                .conn
                .prepare(&format!(
                    "
                    SELECT m.id, m.account_id, m.folder_id, f.path, m.uid, m.sender, m.subject,
                           m.preview, m.date_display, m.unread, m.flagged, m.has_attachment
                    FROM messages m
                    JOIN folders f ON f.id = m.folder_id
                    WHERE {predicate}{extra}
                    ORDER BY m.date_ts DESC, m.uid DESC
                    LIMIT ?1 OFFSET ?2
                    "
                ))
                .map_err(|e| format!("准备{label}分页查询失败: {e}"))?;

            let rows = stmt
                .query_map(params![limit as i64, offset as i64], map_summary_row)
                .map_err(|e| format!("执行{label}分页查询失败: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("解析{label}结果失败: {e}"))?;

            (total.max(0) as usize, rows)
        } else {
            let pattern = format!("%{trimmed}%");
            let total: i64 = self
                .conn
                .query_row(
                    &format!(
                        "
                        SELECT COUNT(*)
                        FROM messages m
                        JOIN folders f ON f.id = m.folder_id
                        WHERE {predicate}{extra}
                          AND (m.subject LIKE ?1 OR m.sender LIKE ?1)
                        "
                    ),
                    params![pattern],
                    |row| row.get(0),
                )
                .map_err(|e| format!("统计{label}搜索总数失败: {e}"))?;

            let mut stmt = self
                .conn
                .prepare(&format!(
                    "
                    SELECT m.id, m.account_id, m.folder_id, f.path, m.uid, m.sender, m.subject,
                           m.preview, m.date_display, m.unread, m.flagged, m.has_attachment
                    FROM messages m
                    JOIN folders f ON f.id = m.folder_id
                    WHERE {predicate}{extra}
                      AND (m.subject LIKE ?1 OR m.sender LIKE ?1)
                    ORDER BY m.date_ts DESC, m.uid DESC
                    LIMIT ?2 OFFSET ?3
                    "
                ))
                .map_err(|e| format!("准备{label}搜索查询失败: {e}"))?;

            let rows = stmt
                .query_map(
                    params![pattern, limit as i64, offset as i64],
                    map_summary_row,
                )
                .map_err(|e| format!("执行{label}搜索查询失败: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("解析{label}搜索结果失败: {e}"))?;

            (total.max(0) as usize, rows)
        };

        Ok(MailPage {
            folder_id: 0,
            offset,
            total,
            items,
        })
    }

    /// Return cached message detail and whether its body has already been downloaded.
    pub fn get_message_detail(
        &self,
        message_id: u32,
    ) -> Result<Option<(MessageDetail, u32, bool)>, String> {
        let row = self
            .conn
            .query_row(
                "
                SELECT m.id, m.account_id, m.folder_id, m.uid, m.message_id, m.sender, m.recipients,
                       m.subject, m.date_display, m.body_text, m.body_html, m.flagged, f.path
                FROM messages m
                JOIN folders f ON f.id = m.folder_id
                WHERE m.id = ?1
                ",
                params![message_id],
                |row| {
                    let body: Option<String> = row.get(9)?;
                    let body_html: Option<String> = row.get(10)?;
                    Ok((
                        row.get::<_, u32>(0)?,
                        row.get::<_, u32>(1)?,
                        row.get::<_, u32>(2)?,
                        row.get::<_, u32>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, String>(8)?,
                        body,
                        body_html,
                        row.get::<_, i32>(11)? != 0,
                        row.get::<_, String>(12)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| format!("查询邮件详情失败: {e}"))?;

        let Some((
            id,
            account_id,
            folder_id,
            uid,
            rfc_message_id,
            sender,
            recipients,
            subject,
            date_display,
            body_opt,
            body_html,
            flagged,
            folder_path,
        )) = row
        else {
            return Ok(None);
        };

        let attachments = self.list_attachments(id)?;
        let body_loaded = body_opt.is_some();
        let body_text = body_opt.unwrap_or_default();
        let thread_messages = self
            .list_thread_messages(account_id, &subject)
            .unwrap_or_default();
        let detail = MessageDetail {
            id,
            account_id,
            folder_id,
            folder_path,
            uid,
            rfc_message_id,
            sender,
            recipients,
            subject,
            date_display,
            body_text,
            body_html,
            flagged,
            attachments,
            thread_messages,
        };
        Ok(Some((detail, uid, body_loaded)))
    }

    /// Find the account's Sent folder `(folder_id, path)` if synced.
    pub fn find_sent_folder(&self, account_id: u32) -> Result<Option<(u32, String)>, String> {
        self.conn
            .query_row(
                &format!(
                    "
                    SELECT f.id, f.path
                    FROM folders f
                    WHERE f.account_id = ?1 AND {SENT_FOLDER_MATCH}
                    ORDER BY f.id ASC
                    LIMIT 1
                    "
                ),
                params![account_id],
                |row| Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|e| format!("查找已发送文件夹失败: {e}"))
    }

    /// Aggregate chronological messages in the same account whose normalized subject
    /// matches `subject` (up to 20 messages).
    pub fn list_thread_messages(
        &self,
        account_id: u32,
        subject: &str,
    ) -> Result<Vec<ThreadMessage>, String> {
        let key = normalize_thread_subject(subject);
        if key.is_empty() {
            return Ok(Vec::new());
        }
        let pattern = format!("%{key}%");
        let mut stmt = self
            .conn
            .prepare(
                "
                SELECT id, sender, recipients, subject, date_display, body_text, body_html, preview, flagged
                FROM messages
                WHERE account_id = ?1 AND LOWER(subject) LIKE ?2
                ORDER BY date_ts ASC, uid ASC
                LIMIT 40
                ",
            )
            .map_err(|e| format!("准备会话查询失败: {e}"))?;

        let raw_rows = stmt
            .query_map(params![account_id, pattern], |row| {
                Ok((
                    row.get::<_, u32>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, i32>(8)? != 0,
                ))
            })
            .map_err(|e| format!("执行会话查询失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析会话结果失败: {e}"))?;

        let mut out = Vec::new();
        for (
            id,
            sender,
            recipients,
            row_subject,
            date_display,
            body_opt,
            body_html,
            preview,
            flagged,
        ) in raw_rows
        {
            if normalize_thread_subject(&row_subject) != key {
                continue;
            }
            let body_loaded = body_opt.is_some();
            let body_text = body_opt.unwrap_or(preview);
            let attachments = self.list_attachments(id).unwrap_or_default();
            out.push(ThreadMessage {
                id,
                sender,
                recipients,
                date_display,
                body_text,
                body_html,
                body_loaded,
                flagged,
                attachments,
            });
            if out.len() >= 20 {
                break;
            }
        }
        Ok(out)
    }

    fn list_attachments(&self, message_id: u32) -> Result<Vec<AttachmentMeta>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "
                SELECT id, filename, mime_type, size_bytes
                FROM attachments
                WHERE message_id = ?1
                ORDER BY id ASC
                ",
            )
            .map_err(|e| format!("查询附件列表失败: {e}"))?;

        let rows = stmt
            .query_map(params![message_id], |row| {
                Ok(AttachmentMeta {
                    id: row.get(0)?,
                    filename: row.get(1)?,
                    mime_type: row.get(2)?,
                    size_bytes: row.get::<_, i64>(3)?.max(0) as usize,
                })
            })
            .map_err(|e| format!("读取附件记录失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析附件记录失败: {e}"))?;
        Ok(rows)
    }

    /// Save fetched message body and attachments in one transaction.
    pub fn store_message_body(
        &mut self,
        message_id: u32,
        body_text: &str,
        attachments: &[CachedAttachmentInput],
    ) -> Result<(), String> {
        self.store_message_body_with_html(message_id, body_text, None, attachments)
    }

    /// Save fetched message plain text, optional HTML body, and attachments in one transaction.
    pub fn store_message_body_with_html(
        &mut self,
        message_id: u32,
        body_text: &str,
        body_html: Option<&str>,
        attachments: &[CachedAttachmentInput],
    ) -> Result<(), String> {
        let tx = self
            .conn
            .transaction()
            .map_err(|e| format!("开启正文缓存事务失败: {e}"))?;

        tx.execute(
            "
            UPDATE messages
            SET body_text = ?1,
                body_html = ?2,
                has_attachment = CASE WHEN ?3 > 0 THEN 1 ELSE has_attachment END
            WHERE id = ?4
            ",
            params![body_text, body_html, attachments.len() as i64, message_id],
        )
        .map_err(|e| format!("保存邮件正文失败: {e}"))?;

        tx.execute(
            "DELETE FROM attachments WHERE message_id = ?1",
            params![message_id],
        )
        .map_err(|e| format!("清理旧附件缓存失败: {e}"))?;

        for att in attachments {
            tx.execute(
                "
                INSERT INTO attachments (message_id, filename, mime_type, size_bytes, content)
                VALUES (?1, ?2, ?3, ?4, ?5)
                ",
                params![
                    message_id,
                    att.filename,
                    att.mime_type,
                    att.content.len() as i64,
                    att.content,
                ],
            )
            .map_err(|e| format!("保存附件失败 ({}): {e}", att.filename))?;
        }

        tx.commit()
            .map_err(|e| format!("提交正文缓存事务失败: {e}"))?;
        Ok(())
    }

    /// Clear all cached message bodies and attachment blobs while keeping
    /// message headers (sender / subject / preview / flags) intact.
    /// Returns the number of messages whose cached body was dropped.
    pub fn clear_message_bodies(&mut self) -> Result<usize, String> {
        let cleared: usize = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE body_text IS NOT NULL",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n.max(0) as usize)
            .map_err(|e| format!("统计已缓存正文数失败: {e}"))?;

        let tx = self
            .conn
            .transaction()
            .map_err(|e| format!("开启正文缓存清理事务失败: {e}"))?;
        tx.execute("UPDATE messages SET body_text = NULL, body_html = NULL", [])
            .map_err(|e| format!("清空邮件正文缓存失败: {e}"))?;
        tx.execute("DELETE FROM attachments", [])
            .map_err(|e| format!("清空附件缓存失败: {e}"))?;
        tx.commit()
            .map_err(|e| format!("提交正文缓存清理事务失败: {e}"))?;
        Ok(cleared)
    }

    /// Look up the Graph remote string ID for `message_id`.
    pub fn get_message_remote_id(&self, message_id: u32) -> Result<Option<String>, String> {
        self.conn
            .query_row(
                "SELECT remote_id FROM messages WHERE id = ?1",
                params![message_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map(|opt| opt.filter(|s| !s.is_empty()))
            .map_err(|e| format!("读取邮件远端 ID 失败: {e}"))
    }

    /// List cached attachments joined with their owning messages for the gallery view,
    /// filtered by `query` against filename / sender / subject, newest first.
    pub fn list_cached_attachments(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<CachedAttachment>, String> {
        let trimmed = query.trim();
        let rows = if trimmed.is_empty() {
            let mut stmt = self
                .conn
                .prepare(
                    "
                    SELECT a.id, a.message_id, m.account_id, a.filename, a.mime_type, a.size_bytes,
                           m.sender, m.subject, m.date_display
                    FROM attachments a
                    JOIN messages m ON m.id = a.message_id
                    ORDER BY m.date_ts DESC, a.id ASC
                    LIMIT ?1 OFFSET ?2
                    ",
                )
                .map_err(|e| format!("准备附件总览查询失败: {e}"))?;
            let rows = stmt
                .query_map(params![limit as i64, offset as i64], map_attachment_row)
                .map_err(|e| format!("执行附件总览查询失败: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("解析附件总览结果失败: {e}"))?;
            rows
        } else {
            let pattern = format!("%{trimmed}%");
            let mut stmt = self
                .conn
                .prepare(
                    "
                    SELECT a.id, a.message_id, m.account_id, a.filename, a.mime_type, a.size_bytes,
                           m.sender, m.subject, m.date_display
                    FROM attachments a
                    JOIN messages m ON m.id = a.message_id
                    WHERE (a.filename LIKE ?1 OR m.sender LIKE ?1 OR m.subject LIKE ?1)
                    ORDER BY m.date_ts DESC, a.id ASC
                    LIMIT ?2 OFFSET ?3
                    ",
                )
                .map_err(|e| format!("准备附件总览搜索查询失败: {e}"))?;
            let rows = stmt
                .query_map(
                    params![pattern, limit as i64, offset as i64],
                    map_attachment_row,
                )
                .map_err(|e| format!("执行附件总览搜索查询失败: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("解析附件总览搜索结果失败: {e}"))?;
            rows
        };
        Ok(rows)
    }

    /// Aggregate known contacts from cached senders/recipients, deduplicated by
    /// e-mail address, filtered by `query` against name / e-mail, ranked by
    /// message count in descending order.
    pub fn list_known_contacts(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<KnownContact>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT sender, recipients FROM messages")
            .map_err(|e| format!("查询联系人来源失败: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("读取联系人来源失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析联系人来源失败: {e}"))?;

        let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        let mut names: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        let mut emails: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for (sender, recipients) in rows {
            // Count each contact once per message, even if listed twice in it.
            let mut seen_in_message: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            let mut pairs = vec![split_address(&sender)];
            for part in recipients.split(',') {
                pairs.push(split_address(part));
            }
            for (name, email) in pairs {
                // Skip empty parts and display-name fallbacks like `(未知)`.
                if !email.contains('@') {
                    continue;
                }
                let key = email.to_lowercase();
                if !seen_in_message.insert(key.clone()) {
                    continue;
                }
                emails.entry(key.clone()).or_insert(email);
                let name_slot = names.entry(key.clone()).or_default();
                if name_slot.is_empty() && !name.is_empty() {
                    *name_slot = name;
                }
                *counts.entry(key).or_insert(0) += 1;
            }
        }

        let trimmed = query.trim().to_lowercase();
        let mut contacts: Vec<KnownContact> = counts
            .into_iter()
            .map(|(key, message_count)| KnownContact {
                name: names.get(&key).cloned().unwrap_or_default(),
                email: emails.get(&key).cloned().unwrap_or(key.clone()),
                message_count,
            })
            .filter(|c| {
                trimmed.is_empty()
                    || c.name.to_lowercase().contains(&trimmed)
                    || c.email.to_lowercase().contains(&trimmed)
            })
            .collect();

        // Rank by message count (descending) with a stable e-mail tie-break.
        contacts.sort_by(|a, b| {
            b.message_count
                .cmp(&a.message_count)
                .then_with(|| a.email.cmp(&b.email))
        });
        Ok(contacts.into_iter().skip(offset).take(limit).collect())
    }

    /// Find the account's Archive folder `(folder_id, path)` using the same
    /// rules as [`Self::list_archived_messages`].
    pub fn find_archive_folder(&self, account_id: u32) -> Result<Option<(u32, String)>, String> {
        self.find_role_folder(account_id, ARCHIVE_FOLDER_MATCH)
    }

    /// Find the account's Trash/Deleted folder `(folder_id, path)` using the
    /// same rules as the unified views.
    pub fn find_trash_folder(&self, account_id: u32) -> Result<Option<(u32, String)>, String> {
        self.find_role_folder(account_id, TRASH_FOLDER_MATCH)
    }

    fn find_role_folder(
        &self,
        account_id: u32,
        predicate: &str,
    ) -> Result<Option<(u32, String)>, String> {
        self.conn
            .query_row(
                &format!(
                    "
                    SELECT f.id, f.path FROM folders f
                    WHERE f.account_id = ?1 AND {predicate}
                    ORDER BY f.id ASC LIMIT 1
                    "
                ),
                params![account_id],
                |row| Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|e| format!("查找角色文件夹失败: {e}"))
    }

    /// Remove a cached message (with its body and attachments) after it was
    /// moved or deleted on the server. Returns the owning folder ID if the
    /// message existed, so callers can refresh folder counters.
    pub fn remove_message(&mut self, message_id: u32) -> Result<Option<u32>, String> {
        let tx = self
            .conn
            .transaction()
            .map_err(|e| format!("开启邮件清理事务失败: {e}"))?;
        let folder_id: Option<u32> = tx
            .query_row(
                "SELECT folder_id FROM messages WHERE id = ?1",
                params![message_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| format!("查询邮件所在文件夹失败: {e}"))?;
        let Some(folder_id) = folder_id else {
            return Ok(None);
        };
        tx.execute("DELETE FROM messages WHERE id = ?1", params![message_id])
            .map_err(|e| format!("移除缓存邮件失败: {e}"))?;
        tx.execute(
            "
            UPDATE folders
            SET unread_count = (SELECT COUNT(*) FROM messages WHERE folder_id = ?1 AND unread = 1),
                total_count = (SELECT COUNT(*) FROM messages WHERE folder_id = ?1)
            WHERE id = ?1
            ",
            params![folder_id],
        )
        .map_err(|e| format!("更新文件夹计数失败: {e}"))?;
        tx.commit()
            .map_err(|e| format!("提交邮件清理事务失败: {e}"))?;
        Ok(Some(folder_id))
    }

    /// Read raw attachment bytes from local cache.
    pub fn get_attachment_bytes(
        &self,
        attachment_id: u32,
    ) -> Result<Option<(String, Vec<u8>)>, String> {
        self.conn
            .query_row(
                "SELECT filename, content FROM attachments WHERE id = ?1",
                params![attachment_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?)),
            )
            .optional()
            .map_err(|e| format!("读取附件内容失败: {e}"))
    }

    /// Update local unread state and refresh folder unread counter.
    pub fn set_message_seen(&mut self, message_id: u32, seen: bool) -> Result<(), String> {
        let tx = self
            .conn
            .transaction()
            .map_err(|e| format!("开启已读状态事务失败: {e}"))?;
        let folder_id: Option<u32> = tx
            .query_row(
                "SELECT folder_id FROM messages WHERE id = ?1",
                params![message_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| format!("查询邮件所在文件夹失败: {e}"))?;
        let Some(folder_id) = folder_id else {
            return Ok(());
        };
        tx.execute(
            "UPDATE messages SET unread = ?1 WHERE id = ?2",
            params![(!seen) as i32, message_id],
        )
        .map_err(|e| format!("更新已读状态失败: {e}"))?;
        tx.execute(
            "
            UPDATE folders
            SET unread_count = (SELECT COUNT(*) FROM messages WHERE folder_id = ?1 AND unread = 1)
            WHERE id = ?1
            ",
            params![folder_id],
        )
        .map_err(|e| format!("更新文件夹未读数失败: {e}"))?;
        tx.commit().map_err(|e| format!("提交已读状态失败: {e}"))
    }

    /// Update local flagged/starred state.
    pub fn set_message_flagged(&mut self, message_id: u32, flagged: bool) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE messages SET flagged = ?1 WHERE id = ?2",
                params![flagged as i32, message_id],
            )
            .map_err(|e| format!("更新星标状态失败: {e}"))?;
        Ok(())
    }

    /// Persist a new outgoing message in the Outbox before SMTP transmission begins.
    #[allow(clippy::too_many_arguments)]
    pub fn enqueue_outbox(
        &mut self,
        account_id: u32,
        to_addr: &str,
        cc_addr: &str,
        bcc_addr: &str,
        subject: &str,
        body_text: &str,
        in_reply_to: &str,
        attachment_paths: &[String],
        created_at: i64,
    ) -> Result<u32, String> {
        let paths_json = serde_json::to_string(attachment_paths)
            .map_err(|e| format!("序列化附件路径失败: {e}"))?;
        self.conn
            .execute(
                "
                INSERT INTO outbox (
                    account_id, to_addr, cc_addr, bcc_addr, subject, body_text, in_reply_to,
                    attachment_paths_json, status, last_error, created_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'queued', '', ?9)
                ",
                params![
                    account_id,
                    to_addr,
                    cc_addr,
                    bcc_addr,
                    subject,
                    body_text,
                    in_reply_to,
                    paths_json,
                    created_at
                ],
            )
            .map_err(|e| format!("写入发件箱持久化记录失败: {e}"))?;
        Ok(self.conn.last_insert_rowid() as u32)
    }

    /// Update the status of an Outbox item.
    pub fn update_outbox_status(
        &mut self,
        outbox_id: u32,
        status: OutboxStatus,
        last_error: &str,
    ) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE outbox SET status = ?1, last_error = ?2 WHERE id = ?3",
                params![status.as_str(), last_error, outbox_id],
            )
            .map_err(|e| format!("更新发件箱状态失败: {e}"))?;
        Ok(())
    }

    /// Delete an Outbox item once the SMTP server confirms acceptance.
    pub fn delete_outbox(&mut self, outbox_id: u32) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM outbox WHERE id = ?1", params![outbox_id])
            .map_err(|e| format!("删除发件箱记录失败: {e}"))?;
        Ok(())
    }

    /// List all pending / uncertain / failed outbox items for `account_id`.
    pub fn list_outbox(&self, account_id: u32) -> Result<Vec<OutboxItem>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "
                SELECT id, account_id, to_addr, cc_addr, bcc_addr, subject, body_text,
                       in_reply_to, attachment_paths_json, status, last_error
                FROM outbox
                WHERE account_id = ?1
                ORDER BY id DESC
                ",
            )
            .map_err(|e| format!("查询发件箱失败: {e}"))?;

        let rows = stmt
            .query_map(params![account_id], |row| {
                let paths_json: String = row.get(8)?;
                let attachment_paths: Vec<String> =
                    serde_json::from_str(&paths_json).unwrap_or_default();
                let status_str: String = row.get(9)?;
                Ok(OutboxItem {
                    id: row.get(0)?,
                    account_id: row.get(1)?,
                    to_addr: row.get(2)?,
                    cc_addr: row.get(3)?,
                    bcc_addr: row.get(4)?,
                    subject: row.get(5)?,
                    body_text: row.get(6)?,
                    in_reply_to: row.get(7)?,
                    attachment_paths,
                    status: OutboxStatus::from_str(&status_str),
                    last_error: row.get(10)?,
                })
            })
            .map_err(|e| format!("读取发件箱失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析发件箱行失败: {e}"))?;
        Ok(rows)
    }

    /// Save or update a local compose draft.
    pub fn upsert_draft(&mut self, draft: &LocalDraft) -> Result<u32, String> {
        let paths_json = serde_json::to_string(&draft.attachment_paths)
            .map_err(|e| format!("序列化草稿附件路径失败: {e}"))?;
        if draft.id == 0 {
            self.conn
                .execute(
                    "
                    INSERT INTO drafts (
                        account_id, to_addr, cc_addr, bcc_addr, subject, body_text,
                        in_reply_to, attachment_paths_json, updated_at
                    )
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                    ",
                    params![
                        draft.account_id,
                        draft.to_addr,
                        draft.cc_addr,
                        draft.bcc_addr,
                        draft.subject,
                        draft.body_text,
                        draft.in_reply_to,
                        paths_json,
                        draft.updated_at
                    ],
                )
                .map_err(|e| format!("保存草稿失败: {e}"))?;
            Ok(self.conn.last_insert_rowid() as u32)
        } else {
            self.conn
                .execute(
                    "
                    UPDATE drafts
                    SET account_id = ?1, to_addr = ?2, cc_addr = ?3, bcc_addr = ?4,
                        subject = ?5, body_text = ?6, in_reply_to = ?7,
                        attachment_paths_json = ?8, updated_at = ?9
                    WHERE id = ?10
                    ",
                    params![
                        draft.account_id,
                        draft.to_addr,
                        draft.cc_addr,
                        draft.bcc_addr,
                        draft.subject,
                        draft.body_text,
                        draft.in_reply_to,
                        paths_json,
                        draft.updated_at,
                        draft.id
                    ],
                )
                .map_err(|e| format!("更新草稿失败: {e}"))?;
            Ok(draft.id)
        }
    }

    /// List all persisted local drafts ordered by most recently updated.
    pub fn list_drafts(&self) -> Result<Vec<LocalDraft>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "
                SELECT id, account_id, to_addr, cc_addr, bcc_addr, subject, body_text,
                       in_reply_to, attachment_paths_json, updated_at
                FROM drafts
                ORDER BY updated_at DESC, id DESC
                ",
            )
            .map_err(|e| format!("查询本地草稿失败: {e}"))?;

        let rows = stmt
            .query_map([], |row| {
                let paths_json: String = row.get(8)?;
                let attachment_paths: Vec<String> =
                    serde_json::from_str(&paths_json).unwrap_or_default();
                Ok(LocalDraft {
                    id: row.get(0)?,
                    account_id: row.get(1)?,
                    to_addr: row.get(2)?,
                    cc_addr: row.get(3)?,
                    bcc_addr: row.get(4)?,
                    subject: row.get(5)?,
                    body_text: row.get(6)?,
                    in_reply_to: row.get(7)?,
                    attachment_paths,
                    updated_at: row.get(9)?,
                })
            })
            .map_err(|e| format!("读取本地草稿失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析本地草稿失败: {e}"))?;
        Ok(rows)
    }

    /// Delete a local draft by ID.
    pub fn delete_draft(&mut self, draft_id: u32) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM drafts WHERE id = ?1", params![draft_id])
            .map_err(|e| format!("删除草稿失败: {e}"))?;
        Ok(())
    }

    /// Remove all cached folders, messages, outbox, and drafts belonging to `account_id`.
    pub fn delete_account_data(&mut self, account_id: u32) -> Result<(), String> {
        let tx = self
            .conn
            .transaction()
            .map_err(|e| format!("开启账号清理事务失败: {e}"))?;
        tx.execute(
            "DELETE FROM folders WHERE account_id = ?1",
            params![account_id],
        )
        .map_err(|e| format!("清理账号文件夹失败: {e}"))?;
        tx.execute(
            "DELETE FROM outbox WHERE account_id = ?1",
            params![account_id],
        )
        .map_err(|e| format!("清理账号发件箱失败: {e}"))?;
        tx.execute(
            "DELETE FROM drafts WHERE account_id = ?1",
            params![account_id],
        )
        .map_err(|e| format!("清理账号草稿失败: {e}"))?;
        tx.commit()
            .map_err(|e| format!("提交账号清理事务失败: {e}"))
    }
}

fn map_summary_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MessageSummary> {
    Ok(MessageSummary {
        id: row.get(0)?,
        account_id: row.get(1)?,
        folder_id: row.get(2)?,
        folder_path: row.get(3)?,
        uid: row.get(4)?,
        sender: row.get(5)?,
        subject: row.get(6)?,
        preview: row.get(7)?,
        date_display: row.get(8)?,
        unread: row.get::<_, i32>(9)? != 0,
        flagged: row.get::<_, i32>(10)? != 0,
        has_attachment: row.get::<_, i32>(11)? != 0,
    })
}

fn map_attachment_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<CachedAttachment> {
    Ok(CachedAttachment {
        id: row.get(0)?,
        message_id: row.get(1)?,
        account_id: row.get(2)?,
        filename: row.get(3)?,
        mime_type: row.get(4)?,
        size_bytes: row.get::<_, i64>(5)?.max(0) as usize,
        sender: row.get(6)?,
        subject: row.get(7)?,
        date_display: row.get(8)?,
    })
}

/// Split a `Name <email>` (or bare `email`) address string into its parts.
fn split_address(raw: &str) -> (String, String) {
    let trimmed = raw.trim();
    if let (Some(start), Some(end)) = (trimmed.find('<'), trimmed.find('>')) {
        if start < end {
            return (
                trimmed[..start].trim().to_string(),
                trimmed[start + 1..end].trim().to_string(),
            );
        }
    }
    (String::new(), trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_header(uid: u32, subject: &str) -> CachedHeaderInput {
        CachedHeaderInput {
            uid,
            message_id: format!("<msg-{uid}@example.com>"),
            sender: "Alice <alice@example.com>".to_string(),
            recipients: "bob@example.com".to_string(),
            subject: subject.to_string(),
            preview: "正文摘要".to_string(),
            date_ts: 1_700_000_000 + uid as i64,
            date_display: "15:00".to_string(),
            unread: true,
            flagged: false,
            has_attachment: false,
        }
    }

    fn folder(name: &str, path: &str) -> RemoteFolder {
        RemoteFolder {
            name: name.to_string(),
            path: path.to_string(),
            unread_count: 0,
            total_count: 0,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn header_with(
        uid: u32,
        subject: &str,
        sender: &str,
        recipients: &str,
        flagged: bool,
        unread: bool,
        date_ts: i64,
    ) -> CachedHeaderInput {
        CachedHeaderInput {
            uid,
            message_id: format!("<msg-{uid}@example.com>"),
            sender: sender.to_string(),
            recipients: recipients.to_string(),
            subject: subject.to_string(),
            preview: String::new(),
            date_ts,
            date_display: String::new(),
            unread,
            flagged,
            has_attachment: false,
        }
    }

    #[test]
    fn same_uid_across_two_folders_stays_isolated() {
        let mut cache = CacheStore::open_in_memory().expect("open memory db");
        let folders = cache
            .sync_folders(
                1,
                &[
                    RemoteFolder {
                        name: "收件箱".to_string(),
                        path: "INBOX".to_string(),
                        unread_count: 1,
                        total_count: 1,
                    },
                    RemoteFolder {
                        name: "已发送".to_string(),
                        path: "Sent".to_string(),
                        unread_count: 0,
                        total_count: 1,
                    },
                ],
            )
            .expect("sync folders");

        let inbox_id = folders[0].id;
        let sent_id = folders[1].id;

        cache
            .commit_folder_headers(
                1,
                inbox_id,
                100,
                &[sample_header(42, "收件箱 UID 42")],
                None,
            )
            .expect("commit inbox");
        cache
            .commit_folder_headers(1, sent_id, 200, &[sample_header(42, "已发送 UID 42")], None)
            .expect("commit sent");

        let inbox_page = cache.list_messages_page(inbox_id, "", 0, 10).unwrap();
        let sent_page = cache.list_messages_page(sent_id, "", 0, 10).unwrap();
        assert_eq!(inbox_page.items[0].subject, "收件箱 UID 42");
        assert_eq!(sent_page.items[0].subject, "已发送 UID 42");
    }

    #[test]
    fn uidvalidity_change_rebuilds_folder_cache() {
        let mut cache = CacheStore::open_in_memory().expect("open memory db");
        let folders = cache
            .sync_folders(
                1,
                &[RemoteFolder {
                    name: "收件箱".to_string(),
                    path: "INBOX".to_string(),
                    unread_count: 1,
                    total_count: 1,
                }],
            )
            .unwrap();
        let inbox_id = folders[0].id;

        cache
            .commit_folder_headers(1, inbox_id, 1001, &[sample_header(1, "旧纪元邮件")], None)
            .unwrap();
        cache
            .commit_folder_headers(1, inbox_id, 1002, &[sample_header(1, "新纪元邮件")], None)
            .unwrap();

        let page = cache.list_messages_page(inbox_id, "", 0, 10).unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].subject, "新纪元邮件");
    }

    #[test]
    fn outbox_preserves_uncertain_state_without_auto_resend() {
        let mut cache = CacheStore::open_in_memory().expect("open memory db");
        let id = cache
            .enqueue_outbox(
                1,
                "to@example.com",
                "",
                "",
                "带附件回复",
                "测试正文",
                "<orig@example.com>",
                &["C:\\temp\\test.txt".to_string()],
                1_700_000_123,
            )
            .unwrap();

        cache
            .update_outbox_status(id, OutboxStatus::Uncertain, "SMTP 响应超时")
            .unwrap();

        let items = cache.list_outbox(1).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].status, OutboxStatus::Uncertain);
        assert_eq!(items[0].attachment_paths, vec!["C:\\temp\\test.txt"]);
    }

    #[test]
    fn outbox_round_trips_cc_and_bcc() {
        let mut cache = CacheStore::open_in_memory().expect("open memory db");
        cache
            .enqueue_outbox(
                1,
                "to@example.com",
                "c1@example.com, Carol <carol@c.com>",
                "secret@example.com",
                "带抄送的邮件",
                "正文",
                "",
                &[],
                1_700_000_200,
            )
            .unwrap();

        let items = cache.list_outbox(1).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].to_addr, "to@example.com");
        assert_eq!(items[0].cc_addr, "c1@example.com, Carol <carol@c.com>");
        assert_eq!(items[0].bcc_addr, "secret@example.com");
    }

    #[test]
    fn legacy_db_without_cc_bcc_columns_is_migrated() {
        // 模拟迭代 2 之前的旧库结构（outbox/drafts 无 cc_addr/bcc_addr 列）。
        let dir = std::env::temp_dir().join(format!("hylki-legacy-db-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let db_path = dir.join("legacy-cache.db");
        {
            let conn = Connection::open(&db_path).expect("open legacy db");
            conn.execute_batch(
                "
                CREATE TABLE folders (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    account_id INTEGER NOT NULL,
                    name TEXT NOT NULL,
                    path TEXT NOT NULL,
                    uid_validity INTEGER NOT NULL DEFAULT 0,
                    unread_count INTEGER NOT NULL DEFAULT 0,
                    total_count INTEGER NOT NULL DEFAULT 0,
                    UNIQUE(account_id, path)
                );
                CREATE TABLE messages (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    account_id INTEGER NOT NULL,
                    folder_id INTEGER NOT NULL,
                    uid INTEGER NOT NULL,
                    message_id TEXT NOT NULL DEFAULT '',
                    sender TEXT NOT NULL,
                    recipients TEXT NOT NULL DEFAULT '',
                    subject TEXT NOT NULL,
                    preview TEXT NOT NULL DEFAULT '',
                    date_ts INTEGER NOT NULL DEFAULT 0,
                    date_display TEXT NOT NULL DEFAULT '',
                    unread INTEGER NOT NULL DEFAULT 1,
                    flagged INTEGER NOT NULL DEFAULT 0,
                    has_attachment INTEGER NOT NULL DEFAULT 0,
                    body_text TEXT,
                    UNIQUE(folder_id, uid)
                );
                CREATE TABLE attachments (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    message_id INTEGER NOT NULL,
                    filename TEXT NOT NULL,
                    mime_type TEXT NOT NULL,
                    size_bytes INTEGER NOT NULL,
                    content BLOB NOT NULL
                );
                CREATE TABLE outbox (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    account_id INTEGER NOT NULL,
                    to_addr TEXT NOT NULL,
                    subject TEXT NOT NULL,
                    body_text TEXT NOT NULL,
                    in_reply_to TEXT NOT NULL DEFAULT '',
                    attachment_paths_json TEXT NOT NULL DEFAULT '[]',
                    status TEXT NOT NULL DEFAULT 'queued',
                    last_error TEXT NOT NULL DEFAULT '',
                    created_at INTEGER NOT NULL
                );
                CREATE TABLE drafts (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    account_id INTEGER NOT NULL,
                    to_addr TEXT NOT NULL,
                    subject TEXT NOT NULL,
                    body_text TEXT NOT NULL,
                    in_reply_to TEXT NOT NULL DEFAULT '',
                    attachment_paths_json TEXT NOT NULL DEFAULT '[]',
                    updated_at INTEGER NOT NULL
                );
                INSERT INTO outbox (account_id, to_addr, subject, body_text, in_reply_to,
                                    attachment_paths_json, status, last_error, created_at)
                VALUES (1, 'old@example.com', '旧邮件', '正文', '', '[]', 'queued', '', 100);
                INSERT INTO drafts (account_id, to_addr, subject, body_text, in_reply_to,
                                    attachment_paths_json, updated_at)
                VALUES (1, 'old-draft@example.com', '旧草稿', '正文', '', '[]', 100);
                ",
            )
            .expect("create legacy schema");
        }

        let mut cache = CacheStore::open(&db_path).expect("open legacy db through migration");

        // 旧数据 cc/bcc 视为空字符串。
        let outbox = cache.list_outbox(1).unwrap();
        assert_eq!(outbox.len(), 1);
        assert_eq!(outbox[0].cc_addr, "");
        assert_eq!(outbox[0].bcc_addr, "");
        let drafts = cache.list_drafts().unwrap();
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].cc_addr, "");
        assert_eq!(drafts[0].bcc_addr, "");

        // 迁移后可正常写入并读回 cc/bcc。
        cache
            .enqueue_outbox(
                1,
                "to@example.com",
                "cc@example.com",
                "bcc@example.com",
                "迁移后新邮件",
                "正文",
                "",
                &[],
                200,
            )
            .unwrap();
        let outbox = cache.list_outbox(1).unwrap();
        assert_eq!(outbox.len(), 2);
        let fresh = outbox.iter().find(|i| i.subject == "迁移后新邮件").unwrap();
        assert_eq!(fresh.cc_addr, "cc@example.com");
        assert_eq!(fresh.bcc_addr, "bcc@example.com");

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unified_inbox_and_chinese_two_char_search_keep_account_ownership() {
        let mut cache = CacheStore::open_in_memory().expect("open memory db");
        let f1 = cache
            .sync_folders(
                1,
                &[RemoteFolder {
                    name: "收件箱".to_string(),
                    path: "INBOX".to_string(),
                    unread_count: 1,
                    total_count: 1,
                }],
            )
            .unwrap();
        let f2 = cache
            .sync_folders(
                2,
                &[RemoteFolder {
                    name: "收件箱".to_string(),
                    path: "INBOX".to_string(),
                    unread_count: 1,
                    total_count: 1,
                }],
            )
            .unwrap();

        cache
            .commit_folder_headers(
                1,
                f1[0].id,
                10,
                &[sample_header(1, "项目周报：Windows 移植进度")],
                None,
            )
            .unwrap();
        cache
            .commit_folder_headers(
                2,
                f2[0].id,
                20,
                &[sample_header(2, "Invoice 发票报销单确认")],
                None,
            )
            .unwrap();

        let all = cache.list_unified_inbox("", 0, 10).unwrap();
        assert_eq!(all.total, 2);
        assert_eq!(all.items[0].account_id, 2);
        assert_eq!(all.items[1].account_id, 1);

        // Two-character Chinese keyword search
        let zh_search = cache.list_unified_inbox("周报", 0, 10).unwrap();
        assert_eq!(zh_search.total, 1);
        assert_eq!(zh_search.items[0].account_id, 1);

        // Mixed Chinese-English search
        let mixed_search = cache.list_unified_inbox("Invoice 发票", 0, 10).unwrap();
        assert_eq!(mixed_search.total, 1);
        assert_eq!(mixed_search.items[0].account_id, 2);
    }

    #[test]
    fn local_draft_persists_updates_and_deletes() {
        let mut cache = CacheStore::open_in_memory().expect("open memory db");
        let id = cache
            .upsert_draft(&LocalDraft {
                id: 0,
                account_id: 1,
                to_addr: "alice@example.com".to_string(),
                cc_addr: "bob@example.com".to_string(),
                bcc_addr: String::new(),
                subject: "草稿主题".to_string(),
                body_text: "第一版内容".to_string(),
                in_reply_to: String::new(),
                attachment_paths: vec!["C:\\tmp\\report.pdf".to_string()],
                updated_at: 100,
            })
            .unwrap();

        let drafts = cache.list_drafts().unwrap();
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].id, id);
        assert_eq!(drafts[0].cc_addr, "bob@example.com");
        assert_eq!(drafts[0].bcc_addr, "");
        assert_eq!(drafts[0].attachment_paths, vec!["C:\\tmp\\report.pdf"]);

        // 更新 cc/bcc 也走同一 upsert 链路。
        let mut updated = drafts[0].clone();
        updated.bcc_addr = "secret@example.com".to_string();
        updated.updated_at = 200;
        cache.upsert_draft(&updated).unwrap();
        let drafts = cache.list_drafts().unwrap();
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].bcc_addr, "secret@example.com");

        cache.delete_draft(id).unwrap();
        assert!(cache.list_drafts().unwrap().is_empty());
    }

    #[test]
    fn starred_view_filters_flagged_and_searches() {
        let mut cache = CacheStore::open_in_memory().expect("open memory db");
        let f1 = cache.sync_folders(1, &[folder("收件箱", "INBOX")]).unwrap();
        let f2 = cache.sync_folders(2, &[folder("收件箱", "INBOX")]).unwrap();

        cache
            .commit_folder_headers(
                1,
                f1[0].id,
                10,
                &[
                    header_with(1, "星标周报", "Alice <alice@a.com>", "", true, false, 100),
                    header_with(2, "普通邮件", "Bob <bob@b.com>", "", false, false, 200),
                ],
                None,
            )
            .unwrap();
        cache
            .commit_folder_headers(
                2,
                f2[0].id,
                20,
                &[header_with(
                    1,
                    "另一账号星标",
                    "Carol <carol@c.com>",
                    "",
                    true,
                    false,
                    300,
                )],
                None,
            )
            .unwrap();

        let starred = cache.list_starred_messages("", 0, 10).unwrap();
        assert_eq!(starred.total, 2);
        assert_eq!(starred.folder_id, 0);
        assert_eq!(starred.items[0].subject, "另一账号星标");
        assert_eq!(starred.items[0].account_id, 2);
        assert_eq!(starred.items[1].subject, "星标周报");
        assert!(starred.items.iter().all(|m| m.flagged));

        let search = cache.list_starred_messages("周报", 0, 10).unwrap();
        assert_eq!(search.total, 1);
        assert_eq!(search.items[0].subject, "星标周报");

        let paged = cache.list_starred_messages("", 1, 1).unwrap();
        assert_eq!(paged.total, 2);
        assert_eq!(paged.items.len(), 1);
        assert_eq!(paged.items[0].subject, "星标周报");
    }

    #[test]
    fn sent_view_aggregates_by_name_and_path() {
        let mut cache = CacheStore::open_in_memory().expect("open memory db");
        let f1 = cache
            .sync_folders(
                1,
                &[
                    folder("收件箱", "INBOX"),
                    folder("已发送", "Sent"),
                    folder("垃圾邮件", "Junk"),
                ],
            )
            .unwrap();
        let f2 = cache
            .sync_folders(2, &[folder("已发送", "AAMkGraphFolderId")])
            .unwrap();

        let inbox = f1.iter().find(|f| f.path == "INBOX").unwrap().id;
        let sent = f1.iter().find(|f| f.path == "Sent").unwrap().id;
        let junk = f1.iter().find(|f| f.path == "Junk").unwrap().id;

        cache
            .commit_folder_headers(
                1,
                inbox,
                10,
                &[header_with(
                    1,
                    "收到的邮件",
                    "x@x.com",
                    "",
                    false,
                    false,
                    50,
                )],
                None,
            )
            .unwrap();
        cache
            .commit_folder_headers(
                1,
                sent,
                11,
                &[header_with(
                    1,
                    "发出的邮件",
                    "me@me.com",
                    "you@you.com",
                    false,
                    false,
                    100,
                )],
                None,
            )
            .unwrap();
        cache
            .commit_folder_headers(
                1,
                junk,
                12,
                &[header_with(
                    1,
                    "广告邮件",
                    "ad@ad.com",
                    "",
                    false,
                    false,
                    200,
                )],
                None,
            )
            .unwrap();
        cache
            .commit_folder_headers(
                2,
                f2[0].id,
                20,
                &[header_with(
                    1,
                    "Graph 已发送",
                    "me2@me.com",
                    "",
                    false,
                    false,
                    300,
                )],
                None,
            )
            .unwrap();

        let sent_page = cache.list_sent_messages("", 0, 10).unwrap();
        assert_eq!(sent_page.total, 2);
        assert_eq!(sent_page.items[0].subject, "Graph 已发送");
        assert_eq!(sent_page.items[0].account_id, 2);
        assert_eq!(sent_page.items[1].subject, "发出的邮件");
        assert_eq!(sent_page.items[1].account_id, 1);

        let search = cache.list_sent_messages("发出", 0, 10).unwrap();
        assert_eq!(search.total, 1);
        assert_eq!(search.items[0].subject, "发出的邮件");
    }

    #[test]
    fn archived_view_aggregates_by_name_and_path() {
        let mut cache = CacheStore::open_in_memory().expect("open memory db");
        let f1 = cache
            .sync_folders(
                1,
                &[
                    folder("收件箱", "INBOX"),
                    folder("归档", "Archive"),
                    folder("Notes", "Notes"),
                ],
            )
            .unwrap();
        let f2 = cache
            .sync_folders(2, &[folder("Archive", "INBOX/Archive")])
            .unwrap();

        let inbox = f1.iter().find(|f| f.path == "INBOX").unwrap().id;
        let archive = f1.iter().find(|f| f.path == "Archive").unwrap().id;
        let notes = f1.iter().find(|f| f.path == "Notes").unwrap().id;

        cache
            .commit_folder_headers(
                1,
                inbox,
                10,
                &[header_with(
                    1,
                    "收件箱邮件",
                    "x@x.com",
                    "",
                    false,
                    false,
                    50,
                )],
                None,
            )
            .unwrap();
        cache
            .commit_folder_headers(
                1,
                archive,
                11,
                &[header_with(
                    1,
                    "已归档邮件",
                    "y@y.com",
                    "",
                    false,
                    false,
                    100,
                )],
                None,
            )
            .unwrap();
        cache
            .commit_folder_headers(
                1,
                notes,
                12,
                &[header_with(1, "笔记", "z@z.com", "", false, false, 200)],
                None,
            )
            .unwrap();
        cache
            .commit_folder_headers(
                2,
                f2[0].id,
                20,
                &[header_with(
                    1,
                    "INBOX 下归档",
                    "w@w.com",
                    "",
                    false,
                    false,
                    300,
                )],
                None,
            )
            .unwrap();

        let archived = cache.list_archived_messages("", 0, 10).unwrap();
        assert_eq!(archived.total, 2);
        assert_eq!(archived.items[0].subject, "INBOX 下归档");
        assert_eq!(archived.items[1].subject, "已归档邮件");

        let found = cache.find_archive_folder(1).unwrap();
        assert_eq!(found, Some((archive, "Archive".to_string())));
    }

    #[test]
    fn attachment_gallery_joins_messages_and_filters() {
        let mut cache = CacheStore::open_in_memory().expect("open memory db");
        let folders = cache
            .sync_folders(1, &[folder("收件箱", "INBOX"), folder("已发送", "Sent")])
            .unwrap();
        let inbox = folders.iter().find(|f| f.path == "INBOX").unwrap().id;
        let sent = folders.iter().find(|f| f.path == "Sent").unwrap().id;

        cache
            .commit_folder_headers(
                1,
                inbox,
                10,
                &[
                    header_with(1, "照片分享", "Bob <bob@b.com>", "", false, false, 200),
                    header_with(2, "季度报告", "Alice <alice@a.com>", "", false, false, 100),
                ],
                None,
            )
            .unwrap();
        cache
            .commit_folder_headers(
                1,
                sent,
                20,
                &[header_with(
                    1,
                    "合同扫描件",
                    "Carol <carol@c.com>",
                    "",
                    false,
                    false,
                    300,
                )],
                None,
            )
            .unwrap();

        let inbox_page = cache.list_messages_page(inbox, "", 0, 10).unwrap();
        let photo_id = inbox_page.items[0].id;
        let sent_page = cache.list_messages_page(sent, "", 0, 10).unwrap();
        let contract_id = sent_page.items[0].id;

        cache
            .store_message_body(
                photo_id,
                "正文",
                &[CachedAttachmentInput {
                    filename: "holiday.jpg".to_string(),
                    mime_type: "image/jpeg".to_string(),
                    content: vec![1, 2, 3],
                }],
            )
            .unwrap();
        cache
            .store_message_body(
                contract_id,
                "正文",
                &[CachedAttachmentInput {
                    filename: "contract.pdf".to_string(),
                    mime_type: "application/pdf".to_string(),
                    content: vec![4],
                }],
            )
            .unwrap();

        let all = cache.list_cached_attachments("", 0, 10).unwrap();
        assert_eq!(all.len(), 2);
        // Newest message first.
        assert_eq!(all[0].filename, "contract.pdf");
        assert_eq!(all[0].subject, "合同扫描件");
        assert_eq!(all[0].sender, "Carol <carol@c.com>");
        assert_eq!(all[0].size_bytes, 1);
        assert_eq!(all[1].filename, "holiday.jpg");
        assert_eq!(all[1].message_id, photo_id);
        assert_eq!(all[1].account_id, 1);
        assert_eq!(all[1].mime_type, "image/jpeg");

        let by_name = cache.list_cached_attachments("contract", 0, 10).unwrap();
        assert_eq!(by_name.len(), 1);
        assert_eq!(by_name[0].filename, "contract.pdf");

        let by_subject = cache.list_cached_attachments("照片", 0, 10).unwrap();
        assert_eq!(by_subject.len(), 1);
        assert_eq!(by_subject[0].filename, "holiday.jpg");

        let by_sender = cache.list_cached_attachments("bob@", 0, 10).unwrap();
        assert_eq!(by_sender.len(), 1);
        assert_eq!(by_sender[0].filename, "holiday.jpg");

        let paged = cache.list_cached_attachments("", 1, 1).unwrap();
        assert_eq!(paged.len(), 1);
        assert_eq!(paged[0].filename, "holiday.jpg");
    }

    #[test]
    fn known_contacts_dedupe_rank_and_filter() {
        let mut cache = CacheStore::open_in_memory().expect("open memory db");
        let folders = cache.sync_folders(1, &[folder("收件箱", "INBOX")]).unwrap();

        cache
            .commit_folder_headers(
                1,
                folders[0].id,
                10,
                &[
                    // Alice sends once with display name, once bare (same address, lowercased).
                    header_with(
                        1,
                        "A",
                        "Alice <alice@a.com>",
                        "bob@b.com, Carol <carol@c.com>",
                        false,
                        false,
                        100,
                    ),
                    header_with(2, "B", "alice@a.com", "Bob <BOB@b.com>", false, false, 200),
                    // No usable address at all.
                    header_with(3, "C", "(未知)", "名字 <>", false, false, 300),
                ],
                None,
            )
            .unwrap();

        let contacts = cache.list_known_contacts("", 0, 50).unwrap();
        assert_eq!(contacts.len(), 3);
        assert_eq!(contacts[0].email, "alice@a.com");
        assert_eq!(contacts[0].name, "Alice");
        assert_eq!(contacts[0].message_count, 2);
        assert_eq!(contacts[1].email, "bob@b.com");
        assert_eq!(contacts[1].name, "Bob");
        assert_eq!(contacts[1].message_count, 2);
        assert_eq!(contacts[2].email, "carol@c.com");
        assert_eq!(contacts[2].name, "Carol");
        assert_eq!(contacts[2].message_count, 1);

        let filtered = cache.list_known_contacts("carol", 0, 50).unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].email, "carol@c.com");

        let paged = cache.list_known_contacts("", 1, 1).unwrap();
        assert_eq!(paged.len(), 1);
        assert_eq!(paged[0].email, "bob@b.com");
    }

    /// 阶段 3：清理正文缓存——正文与附件 BLOB 清空，头信息保留。
    #[test]
    fn clear_message_bodies_keeps_headers_and_clears_attachments() {
        let mut cache = CacheStore::open_in_memory().expect("open memory db");
        let folders = cache
            .sync_folders(1, &[folder("收件箱", "INBOX"), folder("归档", "Archive")])
            .unwrap();
        let inbox = folders.iter().find(|f| f.path == "INBOX").unwrap().id;

        cache
            .commit_folder_headers(
                1,
                inbox,
                10,
                &[
                    sample_header(1, "带正文与附件"),
                    sample_header(2, "仅邮件头"),
                ],
                None,
            )
            .unwrap();
        let page = cache.list_messages_page(inbox, "", 0, 10).unwrap();
        // 列表按日期排序，按主题定位避免依赖行序。
        let with_body_id = page
            .items
            .iter()
            .find(|m| m.subject == "带正文与附件")
            .unwrap()
            .id;
        let header_only_id = page
            .items
            .iter()
            .find(|m| m.subject == "仅邮件头")
            .unwrap()
            .id;

        cache
            .store_message_body(
                with_body_id,
                "这是邮件正文",
                &[CachedAttachmentInput {
                    filename: "report.pdf".to_string(),
                    mime_type: "application/pdf".to_string(),
                    content: vec![9, 9, 9],
                }],
            )
            .unwrap();

        // 清理前：正文已缓存。
        let (_, _, body_loaded) = cache.get_message_detail(with_body_id).unwrap().unwrap();
        assert!(body_loaded);
        assert_eq!(cache.list_cached_attachments("", 0, 10).unwrap().len(), 1);

        let cleared = cache.clear_message_bodies().unwrap();
        assert_eq!(cleared, 1);
        // 重复清理应返回 0。
        assert_eq!(cache.clear_message_bodies().unwrap(), 0);

        // 正文清空（body_loaded = false，下次 LoadMessageBody 会重新拉取），
        // 附件 BLOB 清空，头信息（发件人 / 主题 / 摘要 / 标记）保留。
        let (detail, _, body_loaded) = cache.get_message_detail(with_body_id).unwrap().unwrap();
        assert!(!body_loaded);
        assert!(detail.body_text.is_empty());
        assert!(detail.attachments.is_empty());
        assert_eq!(detail.subject, "带正文与附件");
        assert!(cache.list_cached_attachments("", 0, 10).unwrap().is_empty());

        let (_, _, header_body_loaded) = cache.get_message_detail(header_only_id).unwrap().unwrap();
        assert!(!header_body_loaded);

        // 头信息仍可被统一收件箱列出（顺序按日期，不做顺序断言）。
        let unified = cache.list_unified_inbox("", 0, 10).unwrap();
        assert_eq!(unified.total, 2);
        let subjects: Vec<&str> = unified.items.iter().map(|m| m.subject.as_str()).collect();
        assert!(subjects.contains(&"带正文与附件"));
        assert!(subjects.contains(&"仅邮件头"));
    }

    #[test]
    fn role_folder_finders_match_names_and_paths() {
        let mut cache = CacheStore::open_in_memory().expect("open memory db");
        let folders = cache
            .sync_folders(
                1,
                &[
                    folder("收件箱", "INBOX"),
                    folder("已删除", "Trash"),
                    folder("归档", "Archive"),
                    folder("Notes", "Notes"),
                ],
            )
            .unwrap();
        let archive = folders.iter().find(|f| f.path == "Archive").unwrap();
        let trash = folders.iter().find(|f| f.path == "Trash").unwrap();

        assert_eq!(
            cache.find_archive_folder(1).unwrap(),
            Some((archive.id, "Archive".to_string()))
        );
        assert_eq!(
            cache.find_trash_folder(1).unwrap(),
            Some((trash.id, "Trash".to_string()))
        );
        // Account 2 has no cached folders at all.
        assert_eq!(cache.find_archive_folder(2).unwrap(), None);
        assert_eq!(cache.find_trash_folder(2).unwrap(), None);
    }

    #[test]
    fn remove_message_updates_counts_and_cascades_attachments() {
        let mut cache = CacheStore::open_in_memory().expect("open memory db");
        let folders = cache.sync_folders(1, &[folder("收件箱", "INBOX")]).unwrap();
        let inbox = folders[0].id;

        cache
            .commit_folder_headers(
                1,
                inbox,
                10,
                &[
                    header_with(
                        1,
                        "被移除的邮件",
                        "Alice <alice@a.com>",
                        "",
                        false,
                        false,
                        100,
                    ),
                    header_with(2, "保留的未读邮件", "Bob <bob@b.com>", "", false, true, 200),
                ],
                None,
            )
            .unwrap();

        let page = cache.list_messages_page(inbox, "", 0, 10).unwrap();
        let target_id = page
            .items
            .iter()
            .find(|m| m.subject == "被移除的邮件")
            .unwrap()
            .id;
        cache
            .store_message_body(
                target_id,
                "正文",
                &[CachedAttachmentInput {
                    filename: "gone.txt".to_string(),
                    mime_type: "text/plain".to_string(),
                    content: vec![1],
                }],
            )
            .unwrap();
        assert_eq!(cache.list_cached_attachments("", 0, 10).unwrap().len(), 1);

        let removed = cache.remove_message(target_id).unwrap();
        assert_eq!(removed, Some(inbox));
        assert!(cache.get_message_detail(target_id).unwrap().is_none());
        assert!(cache.list_cached_attachments("", 0, 10).unwrap().is_empty());

        let folders_after = cache.list_folders(1).unwrap();
        assert_eq!(folders_after[0].total_count, 1);
        assert_eq!(folders_after[0].unread_count, 1);

        let page_after = cache.list_messages_page(inbox, "", 0, 10).unwrap();
        assert_eq!(page_after.total, 1);
        assert_eq!(page_after.items[0].subject, "保留的未读邮件");

        // Removing again is a no-op.
        assert_eq!(cache.remove_message(target_id).unwrap(), None);
    }

    #[test]
    fn message_filter_queries_return_exact_totals_and_items() {
        let mut cache = CacheStore::open_in_memory().unwrap();
        let folders = cache
            .sync_folders(
                1,
                &[RemoteFolder {
                    name: "收件箱".to_string(),
                    path: "INBOX".to_string(),
                    unread_count: 0,
                    total_count: 0,
                }],
            )
            .unwrap();
        let inbox = folders[0].id;
        cache
            .commit_folder_headers(
                1,
                inbox,
                1,
                &[
                    CachedHeaderInput {
                        uid: 1,
                        message_id: "<1@test>".to_string(),
                        sender: "Alice <a@test>".to_string(),
                        recipients: "me@test".to_string(),
                        subject: "未读且星标".to_string(),
                        preview: String::new(),
                        date_ts: 300,
                        date_display: "10:00".to_string(),
                        unread: true,
                        flagged: true,
                        has_attachment: false,
                    },
                    CachedHeaderInput {
                        uid: 2,
                        message_id: "<2@test>".to_string(),
                        sender: "Bob <b@test>".to_string(),
                        recipients: "me@test".to_string(),
                        subject: "仅未读".to_string(),
                        preview: String::new(),
                        date_ts: 200,
                        date_display: "09:00".to_string(),
                        unread: true,
                        flagged: false,
                        has_attachment: false,
                    },
                    CachedHeaderInput {
                        uid: 3,
                        message_id: "<3@test>".to_string(),
                        sender: "Carol <c@test>".to_string(),
                        recipients: "me@test".to_string(),
                        subject: "已读星标".to_string(),
                        preview: String::new(),
                        date_ts: 100,
                        date_display: "08:00".to_string(),
                        unread: false,
                        flagged: true,
                        has_attachment: false,
                    },
                ],
                Some(&[1, 2, 3]),
            )
            .unwrap();

        let unread = cache
            .list_messages_page_filtered(inbox, "", MessageFilter::Unread, 0, 10)
            .unwrap();
        assert_eq!(unread.total, 2);
        assert!(unread.items.iter().all(|m| m.unread));

        let starred = cache
            .list_unified_inbox_filtered("", MessageFilter::Starred, 0, 10)
            .unwrap();
        assert_eq!(starred.total, 2);
        assert!(starred.items.iter().all(|m| m.flagged));

        let unread_starred = cache
            .list_starred_messages_filtered("", MessageFilter::Unread, 0, 10)
            .unwrap();
        assert_eq!(unread_starred.total, 1);
        assert_eq!(unread_starred.items[0].subject, "未读且星标");
    }

    #[test]
    fn thread_subject_normalization_and_grouping_work() {
        assert_eq!(
            normalize_thread_subject("Re: Fwd: 回复：Q3 Roadmap "),
            "q3 roadmap"
        );
        assert_eq!(normalize_thread_subject("转发: 项目进度"), "项目进度");

        let mut cache = CacheStore::open_in_memory().unwrap();
        let folders = cache
            .sync_folders(
                1,
                &[
                    RemoteFolder {
                        name: "收件箱".to_string(),
                        path: "INBOX".to_string(),
                        unread_count: 0,
                        total_count: 0,
                    },
                    RemoteFolder {
                        name: "已发送".to_string(),
                        path: "Sent".to_string(),
                        unread_count: 0,
                        total_count: 0,
                    },
                ],
            )
            .unwrap();
        let inbox = folders[0].id;
        assert_eq!(
            cache.find_sent_folder(1).unwrap().map(|(_, p)| p),
            Some("Sent".to_string())
        );

        cache
            .commit_folder_headers(
                1,
                inbox,
                1,
                &[
                    CachedHeaderInput {
                        uid: 1,
                        message_id: "<m1@test>".to_string(),
                        sender: "Alice <a@test>".to_string(),
                        recipients: "me@test".to_string(),
                        subject: "Q3 Roadmap".to_string(),
                        preview: "第一封".to_string(),
                        date_ts: 100,
                        date_display: "09:00".to_string(),
                        unread: false,
                        flagged: false,
                        has_attachment: false,
                    },
                    CachedHeaderInput {
                        uid: 2,
                        message_id: "<m2@test>".to_string(),
                        sender: "Bob <b@test>".to_string(),
                        recipients: "me@test".to_string(),
                        subject: "Re: Q3 Roadmap".to_string(),
                        preview: "第二封回复".to_string(),
                        date_ts: 200,
                        date_display: "10:00".to_string(),
                        unread: true,
                        flagged: false,
                        has_attachment: false,
                    },
                ],
                Some(&[1, 2]),
            )
            .unwrap();

        let thread = cache.list_thread_messages(1, "Re: Q3 Roadmap").unwrap();
        assert_eq!(thread.len(), 2);
        assert_eq!(thread[0].sender, "Alice <a@test>");
        assert_eq!(thread[1].sender, "Bob <b@test>");
    }
}
