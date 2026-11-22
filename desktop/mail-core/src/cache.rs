//! SQLite local cache for folders, message headers/bodies, attachments, and outbox journal.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::search_query::SearchQuery;
use crate::{
    start_of_today_ts, AttachmentMeta, FolderSummary, LocalDraft, MailPage, MessageDetail,
    MessageFilter, MessageSort, MessageSummary, OutboxItem, OutboxStatus, QuickFilterSet,
    ThreadMessage, UserContact,
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
    pub in_reply_to: String,
    pub references_header: String,
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
    /// F-6 表头日期排序用的 Unix 时间戳（`date_display` 字符串排序无法跨年比较）。
    #[serde(default)]
    pub date_ts: i64,
}

/// WP-06：持久化自定义标签（存入 SQLite `tags` 表，支持调色板颜色与 `1..9` 快捷键绑定）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagItem {
    pub id: u32,
    pub name: String,
    pub color: String,
    pub shortcut: Option<u8>,
}

/// Contact aggregated from cached senders/recipients for the contacts view.
/// `custom == true` 的行来自 `user_contacts` 表（用户手动维护，可编辑 / 删除），
/// 其余为已同步邮件自动聚合的结果（只读展示）。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownContact {
    pub name: String,
    pub email: String,
    pub message_count: usize,
    /// 自定义联系人行 ID（聚合联系人恒为 0）。
    #[serde(default)]
    pub id: u32,
    /// 自定义联系人备注（聚合联系人为空）。
    #[serde(default)]
    pub notes: String,
    /// 是否为用户自定义联系人（列表中标注来源徽标并开放编辑 / 删除）。
    #[serde(default)]
    pub custom: bool,
}

/// SQL predicate matching every account's Sent folder by friendly name or raw path.
const SENT_FOLDER_MATCH: &str = "(f.name = '已发送' OR UPPER(f.path) LIKE '%SENT%')";

/// SQL predicate matching every account's Archive folder by friendly name or raw path.
const ARCHIVE_FOLDER_MATCH: &str = "(f.name = '归档' OR UPPER(f.path) LIKE '%ARCHIVE%')";

/// SQL predicate matching every account's Trash/Deleted folder by friendly name or raw path.
const TRASH_FOLDER_MATCH: &str =
    "(f.name = '已删除' OR UPPER(f.path) LIKE '%TRASH%' OR UPPER(f.path) LIKE '%DELETED%')";

/// SQL predicate matching every account's Junk/Spam folder by friendly name or raw path.
const JUNK_FOLDER_MATCH: &str =
    "(f.name = '垃圾邮件' OR UPPER(f.path) LIKE '%JUNK%' OR UPPER(f.path) LIKE '%SPAM%')";
const SPAM_FOLDER_MATCH: &str = JUNK_FOLDER_MATCH;

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

/// Extract normalized RFC Message-ID tokens (without angle brackets, lowercased)
/// from `Message-ID`, `In-Reply-To`, `References`, or Graph `conversationId` headers.
pub fn extract_rfc_message_ids(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let mut ids = Vec::new();
    let mut rest = trimmed;
    while let Some(open) = rest.find('<') {
        let after_open = &rest[open + 1..];
        let Some(close) = after_open.find('>') else {
            break;
        };
        let token = after_open[..close].trim().to_ascii_lowercase();
        if !token.is_empty() && !ids.contains(&token) {
            ids.push(token);
        }
        rest = &after_open[close + 1..];
    }
    if ids.is_empty() {
        for part in trimmed.split_whitespace() {
            let token = part
                .trim_matches(|c| c == '<' || c == '>' || c == ',' || c == ';')
                .trim()
                .to_ascii_lowercase();
            if !token.is_empty() && !ids.contains(&token) {
                ids.push(token);
            }
        }
    }
    ids
}

/// Check whether a folder's friendly name or raw path represents a Trash/Deleted folder.
fn is_trash_folder(name: &str, path: &str) -> bool {
    let upper_path = path.to_ascii_uppercase();
    name == "已删除" || upper_path.contains("TRASH") || upper_path.contains("DELETED")
}

/// Check whether a folder's friendly name or raw path represents a Junk/Spam folder.
fn is_junk_folder(name: &str, path: &str) -> bool {
    let upper_path = path.to_ascii_uppercase();
    name == "垃圾邮件"
        || upper_path.contains("JUNK")
        || upper_path.contains("SPAM")
        || upper_path.contains("BULK")
}

/// Strip characters that Windows forbids in file names (same mapping as
/// [`unique_attachment_path`]) and trim surrounding whitespace / dots;
/// fall back to `attachment` when nothing usable remains.
pub fn sanitize_filename(name: &str) -> String {
    let safe: String = name
        .trim()
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect();
    let safe = safe.trim().trim_matches('.').trim().to_string();
    if safe.trim_matches('.').is_empty() {
        "attachment".to_string()
    } else {
        safe
    }
}

/// F-7：从 `List-Unsubscribe` 头的值中提取第一个可执行的退订地址。
///
/// 按 RFC 2369 该头携带一个或多个 `<uri>`；真实邮件常有不带尖括号的写法，
/// 因此先取 `<...>` 段，取不到再按空白 / 逗号拆分裸值。只接受
/// `http://` / `https://` / `mailto:` 开头的地址，命中第一个即返回。
pub fn extract_unsubscribe_url(header_value: &str) -> Option<String> {
    let value = header_value.trim();
    if value.is_empty() {
        return None;
    }
    let mut candidates: Vec<&str> = Vec::new();
    let mut rest = value;
    while let Some(open) = rest.find('<') {
        let after_open = &rest[open + 1..];
        match after_open.find('>') {
            Some(close) => {
                candidates.push(after_open[..close].trim());
                rest = &after_open[close + 1..];
            }
            None => {
                // 缺少右尖括号：把剩余内容当作一个候选后结束。
                candidates.push(after_open.trim());
                break;
            }
        }
    }
    if candidates.is_empty() {
        candidates = value
            .split([',', ';', ' ', '\t'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
    }
    candidates
        .into_iter()
        .map(|c| c.trim_end_matches(',').trim())
        .find(|c| {
            c.to_ascii_lowercase().starts_with("https://")
                || c.to_ascii_lowercase().starts_with("http://")
                || c.to_ascii_lowercase().starts_with("mailto:")
        })
        .map(str::to_string)
}

/// F-7：判断 `List-Unsubscribe-Post` 头是否为 RFC 8058 规定的
/// `List-Unsubscribe=One-Click`（比较忽略大小写、空白与引号）。
pub fn is_one_click_post(header_value: &str) -> bool {
    header_value
        .trim()
        .trim_matches('"')
        .split([',', ';'])
        .any(|part| {
            let compact: String = part.chars().filter(|c| !c.is_whitespace()).collect();
            compact.eq_ignore_ascii_case("List-Unsubscribe=One-Click")
        })
}

/// Compute a non-colliding file path in `dir` for `filename`, appending `(1)`, `(2)`, ...
/// before the file extension when a file with the same name already exists.
pub fn unique_attachment_path(dir: &Path, filename: &str) -> PathBuf {
    let base = Path::new(filename)
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("attachment.bin");
    let safe_name: String = base
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect();
    let safe_name = if safe_name.trim_matches('.').is_empty() {
        "attachment.bin".to_string()
    } else {
        safe_name
    };

    let candidate = dir.join(&safe_name);
    if !candidate.exists() {
        return candidate;
    }

    let path_obj = Path::new(&safe_name);
    let stem = path_obj
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(&safe_name);
    let ext = path_obj.extension().and_then(|e| e.to_str());

    let mut idx = 1usize;
    loop {
        let numbered = match ext {
            Some(e) if !e.is_empty() => format!("{stem} ({idx}).{e}"),
            _ => format!("{stem} ({idx})"),
        };
        let next = dir.join(numbered);
        if !next.exists() {
            return next;
        }
        idx += 1;
    }
}

/// Persistent SQLite mail cache.
pub struct CacheStore {
    conn: Connection,
    own_emails: Vec<String>,
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
        let store = Self {
            conn,
            own_emails: Vec::new(),
        };
        store.configure_and_migrate()?;
        Ok(store)
    }

    /// Open an isolated in-memory cache database for unit tests.
    pub fn open_in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| format!("打开内存缓存失败: {e}"))?;
        let store = Self {
            conn,
            own_emails: Vec::new(),
        };
        store.configure_and_migrate()?;
        Ok(store)
    }

    /// Update configured account email addresses used by the `To Me` quick filter.
    pub fn set_own_emails(&mut self, emails: Vec<String>) {
        self.own_emails = emails
            .into_iter()
            .map(|e| e.trim().to_lowercase())
            .filter(|e| !e.is_empty())
            .collect();
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
                    in_reply_to TEXT NOT NULL DEFAULT '',
                    references_header TEXT NOT NULL DEFAULT '',
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
        // 撤销发送 / 停用账号排队（任务一）：到期时间戳；NULL = 立即到期。
        // 历史数据视为已到期，启动恢复后可直接投递。
        self.ensure_column("outbox", "send_at", "INTEGER")?;
        self.ensure_column("drafts", "cc_addr", "TEXT NOT NULL DEFAULT ''")?;
        self.ensure_column("drafts", "bcc_addr", "TEXT NOT NULL DEFAULT ''")?;
        // HTML 正文（迭代 3）：旧库无列时补充，历史数据视为 NULL。
        self.ensure_column("messages", "body_html", "TEXT")?;
        self.ensure_column("messages", "index_text", "TEXT")?;
        // 历史邮件增量预加载游标：标记服务端是否仍有更早邮件及 Graph nextLink。
        self.ensure_column("folders", "has_more_remote", "INTEGER NOT NULL DEFAULT 0")?;
        self.ensure_column("folders", "remote_next_link", "TEXT NOT NULL DEFAULT ''")?;
        // RFC 会话线程关联头（In-Reply-To / References 或 Graph conversationId）。
        self.ensure_column("messages", "in_reply_to", "TEXT NOT NULL DEFAULT ''")?;
        self.ensure_column("messages", "references_header", "TEXT NOT NULL DEFAULT ''")?;
        // F-7 一键退订：`List-Unsubscribe` 第一个可执行地址与 RFC 8058 one-click
        // 标记（正文抓取时随头解析写入；旧库补列，历史数据视为无退订头）。
        self.ensure_column("messages", "list_unsubscribe", "TEXT NOT NULL DEFAULT ''")?;
        self.ensure_column(
            "messages",
            "list_unsubscribe_post",
            "INTEGER NOT NULL DEFAULT 0",
        )?;
        // WP-08：重要性、标签、回复/转发标记与稍后提醒时间戳。
        self.ensure_column("messages", "importance", "INTEGER NOT NULL DEFAULT 0")?;
        self.ensure_column("messages", "tags", "TEXT NOT NULL DEFAULT ''")?;
        self.ensure_column("messages", "answered", "INTEGER NOT NULL DEFAULT 0")?;
        self.ensure_column("messages", "forwarded", "INTEGER NOT NULL DEFAULT 0")?;
        self.ensure_column("messages", "snoozed_until_ts", "INTEGER NOT NULL DEFAULT 0")?;
        // F-5 用户手动维护的联系人（与邮件聚合联系人合并展示；新增表对旧库无侵入，
        // `CREATE TABLE IF NOT EXISTS` 即完成迁移）。
        self.conn
            .execute_batch(
                "
                CREATE TABLE IF NOT EXISTS user_contacts (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    email TEXT NOT NULL UNIQUE,
                    display_name TEXT NOT NULL DEFAULT '',
                    notes TEXT NOT NULL DEFAULT '',
                    created_at INTEGER NOT NULL DEFAULT 0,
                    updated_at INTEGER NOT NULL DEFAULT 0
                );
                CREATE TABLE IF NOT EXISTS tags (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL UNIQUE,
                    color TEXT NOT NULL DEFAULT '#3584e4',
                    shortcut INTEGER
                );
                ",
            )
            .map_err(|e| format!("初始化用户联系人与标签表失败: {e}"))?;
        self.conn
            .execute(
                "CREATE INDEX IF NOT EXISTS idx_messages_account_msgid ON messages(account_id, message_id)",
                [],
            )
            .map_err(|e| format!("创建会话索引失败: {e}"))?;
        // FTS5 全文索引：覆盖主题、发件人、收件人、摘要与正文，由触发器自动维护。
        self.conn
            .execute_batch(
                "
                CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
                    subject,
                    sender,
                    recipients,
                    preview,
                    body_text,
                    content='messages',
                    content_rowid='id',
                    tokenize='unicode61 remove_diacritics 2'
                );

                CREATE TRIGGER IF NOT EXISTS messages_ai AFTER INSERT ON messages BEGIN
                    INSERT INTO messages_fts(rowid, subject, sender, recipients, preview, body_text)
                    VALUES (new.id, new.subject, new.sender, new.recipients, new.preview, COALESCE(new.body_text, ''));
                END;

                CREATE TRIGGER IF NOT EXISTS messages_ad AFTER DELETE ON messages BEGIN
                    INSERT INTO messages_fts(messages_fts, rowid, subject, sender, recipients, preview, body_text)
                    VALUES ('delete', old.id, old.subject, old.sender, old.recipients, old.preview, COALESCE(old.body_text, ''));
                END;

                CREATE TRIGGER IF NOT EXISTS messages_au
                AFTER UPDATE OF subject, sender, recipients, preview, body_text ON messages BEGIN
                    INSERT INTO messages_fts(messages_fts, rowid, subject, sender, recipients, preview, body_text)
                    VALUES ('delete', old.id, old.subject, old.sender, old.recipients, old.preview, COALESCE(old.body_text, ''));
                    INSERT INTO messages_fts(rowid, subject, sender, recipients, preview, body_text)
                    VALUES (new.id, new.subject, new.sender, new.recipients, new.preview, COALESCE(new.body_text, ''));
                END;

                CREATE INDEX IF NOT EXISTS idx_messages_needing_index
                    ON messages(folder_id, id) WHERE body_text IS NULL AND index_text IS NULL;

                CREATE VIRTUAL TABLE IF NOT EXISTS messages_index_fts USING fts5(
                    index_text, content='messages', content_rowid='id',
                    tokenize='unicode61 remove_diacritics 2'
                );
                CREATE TRIGGER IF NOT EXISTS messages_index_ad AFTER DELETE ON messages
                WHEN old.index_text <> '' BEGIN
                    INSERT INTO messages_index_fts(messages_index_fts, rowid, index_text)
                    VALUES ('delete', old.id, old.index_text);
                END;
                CREATE TRIGGER IF NOT EXISTS messages_index_au AFTER UPDATE OF index_text ON messages BEGIN
                    INSERT INTO messages_index_fts(messages_index_fts, rowid, index_text)
                    SELECT 'delete', old.id, old.index_text WHERE old.index_text <> '';
                    INSERT INTO messages_index_fts(rowid, index_text)
                    SELECT new.id, new.index_text WHERE new.index_text <> '';
                END;
                ",
            )
            .map_err(|e| format!("初始化 FTS5 全文检索索引失败: {e}"))?;

        let schema_version: i32 = self
            .conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap_or(0);
        if schema_version < 1 {
            self.conn
                .execute(
                    "INSERT INTO messages_fts(messages_fts) VALUES('rebuild')",
                    [],
                )
                .map_err(|e| format!("重建 FTS5 全文索引失败: {e}"))?;
            self.conn
                .execute("PRAGMA user_version = 1", [])
                .map_err(|e| format!("更新缓存 schema 版本失败: {e}"))?;
        }
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
                SELECT id, account_id, name, path, unread_count, total_count, has_more_remote
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
                    has_more_remote: row.get::<_, i32>(6)? != 0,
                })
            })
            .map_err(|e| format!("读取文件夹列表失败: {e}"))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析文件夹行失败: {e}"))
    }

    /// Return `(account_id, path, has_more_remote, remote_next_link, min_cached_uid, cached_count)`
    /// for `folder_id` to drive incremental older-message fetching.
    pub fn folder_remote_cursor(
        &self,
        folder_id: u32,
    ) -> Result<Option<(u32, String, bool, String, Option<u32>, usize)>, String> {
        self.conn
            .query_row(
                "
                SELECT
                    f.account_id,
                    f.path,
                    f.has_more_remote,
                    f.remote_next_link,
                    (SELECT MIN(m.uid) FROM messages m WHERE m.folder_id = f.id),
                    (SELECT COUNT(*) FROM messages m WHERE m.folder_id = f.id)
                FROM folders f
                WHERE f.id = ?1
                ",
                params![folder_id],
                |row| {
                    Ok((
                        row.get::<_, u32>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i32>(2)? != 0,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<u32>>(4)?,
                        row.get::<_, i64>(5)?.max(0) as usize,
                    ))
                },
            )
            .optional()
            .map_err(|e| format!("读取文件夹增量游标失败: {e}"))
    }

    /// Return `(folder_id, account_id, path)` for all inbox folders that still have older remote mail.
    pub fn list_inbox_folders_with_more_remote(&self) -> Result<Vec<(u32, u32, String)>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "
                SELECT id, account_id, path
                FROM folders
                WHERE (UPPER(path) = 'INBOX' OR is_inbox = 1) AND has_more_remote = 1
                ORDER BY id ASC
                ",
            )
            .map_err(|e| format!("查询待加载历史邮件的收件箱失败: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, u32>(0)?,
                    row.get::<_, u32>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| format!("遍历待加载历史邮件的收件箱失败: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析待加载历史邮件的收件箱失败: {e}"))
    }

    /// 后台索引候选：尚未同步、仍有远端历史或有待索引正文的文件夹。
    /// 跳过 Trash / Junk / Drafts / Templates 等不值得索引的文件夹（对齐原版
    /// `gallery_folder` 语义），INBOX 排在最前优先索引。
    pub fn list_backfill_folders(&self) -> Result<Vec<(u32, u32, String, bool)>, String> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT f.id, f.account_id, f.path, f.uid_validity = 0
                 FROM folders f
                 WHERE (
                     f.has_more_remote = 1 OR f.uid_validity = 0 OR EXISTS (
                         SELECT 1 FROM messages m
                         WHERE m.folder_id = f.id AND m.body_text IS NULL AND m.index_text IS NULL
                     )
                 )
                 AND NOT ({TRASH_FOLDER_MATCH} OR {JUNK_FOLDER_MATCH}
                     OR f.name = '草稿' OR UPPER(f.path) LIKE '%DRAFT%'
                     OR f.name = '模板' OR UPPER(f.path) LIKE '%TEMPLATE%')
                 ORDER BY (f.is_inbox = 1 OR UPPER(f.path) = 'INBOX') DESC, f.id"
            ))
            .map_err(|e| format!("查询后台索引文件夹失败: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .map_err(|e| format!("读取后台索引文件夹失败: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析后台索引文件夹失败: {e}"))
    }

    /// 返回一批仍缺少可搜索正文的邮件，包括升级前缓存的邮件头。
    pub fn list_unindexed_messages(
        &self,
        folder_id: u32,
        limit: usize,
    ) -> Result<Vec<(u32, u32, String)>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, uid, remote_id FROM messages
                 WHERE folder_id = ?1 AND body_text IS NULL AND index_text IS NULL
                 ORDER BY id LIMIT ?2",
            )
            .map_err(|e| format!("查询待索引邮件失败: {e}"))?;
        let rows = stmt
            .query_map(params![folder_id, limit as i64], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .map_err(|e| format!("读取待索引邮件失败: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析待索引邮件失败: {e}"))
    }

    /// 保存独立的搜索正文，空串表示已检查此邮件。完整正文由阅读器按需缓存。
    pub fn store_index_text(&self, message_id: u32, text: &str) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE messages SET index_text = ?1
                 WHERE id = ?2 AND body_text IS NULL AND index_text IS NULL",
                params![text, message_id],
            )
            .map_err(|e| format!("写入邮件搜索正文失败: {e}"))?;
        Ok(())
    }

    /// Persist whether `folder_id` has more older remote messages and its Graph `@odata.nextLink`.
    pub fn set_folder_remote_cursor(
        &self,
        folder_id: u32,
        has_more_remote: bool,
        remote_next_link: &str,
    ) -> Result<(), String> {
        self.conn
            .execute(
                "
                UPDATE folders
                SET has_more_remote = ?1, remote_next_link = ?2
                WHERE id = ?3
                ",
                params![has_more_remote as i32, remote_next_link, folder_id],
            )
            .map_err(|e| format!("更新文件夹远程游标失败: {e}"))?;
        Ok(())
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
                    account_id, folder_id, uid, remote_id, message_id, in_reply_to, references_header,
                    sender, recipients, subject, preview, date_ts, date_display, unread, flagged, has_attachment
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
                ON CONFLICT(folder_id, uid) DO UPDATE SET
                    remote_id = CASE
                        WHEN excluded.remote_id = '' THEN messages.remote_id
                        ELSE excluded.remote_id
                    END,
                    message_id = excluded.message_id,
                    in_reply_to = CASE
                        WHEN excluded.in_reply_to = '' THEN messages.in_reply_to
                        ELSE excluded.in_reply_to
                    END,
                    references_header = CASE
                        WHEN excluded.references_header = '' THEN messages.references_header
                        ELSE excluded.references_header
                    END,
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
                    item.in_reply_to,
                    item.references_header,
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

    /// Reconcile `unread` and `flagged` state for all cached messages in `folder_id`
    /// against server-side `UID SEARCH FLAGGED` and `UID SEARCH UNSEEN` snapshots.
    /// Returns the number of rows whose state changed.
    pub fn reconcile_folder_flags(
        &mut self,
        folder_id: u32,
        flagged_uids: &HashSet<u32>,
        unseen_uids: &HashSet<u32>,
    ) -> Result<usize, String> {
        let tx = self
            .conn
            .transaction()
            .map_err(|e| format!("开启状态位对账事务失败: {e}"))?;

        let rows: Vec<(u32, u32, bool, bool)> = {
            let mut stmt = tx
                .prepare("SELECT id, uid, unread, flagged FROM messages WHERE folder_id = ?1")
                .map_err(|e| format!("读取本地状态位失败: {e}"))?;
            let collected = stmt
                .query_map(params![folder_id], |row| {
                    Ok((
                        row.get::<_, u32>(0)?,
                        row.get::<_, u32>(1)?,
                        row.get::<_, i32>(2)? != 0,
                        row.get::<_, i32>(3)? != 0,
                    ))
                })
                .map_err(|e| format!("遍历本地状态位失败: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("收集本地状态位失败: {e}"))?;
            collected
        };

        let mut updated = 0usize;
        for (msg_id, uid, was_unread, was_flagged) in rows {
            let now_unread = unseen_uids.contains(&uid);
            let now_flagged = flagged_uids.contains(&uid);
            if now_unread != was_unread || now_flagged != was_flagged {
                tx.execute(
                    "UPDATE messages SET unread = ?1, flagged = ?2 WHERE id = ?3",
                    params![now_unread as i32, now_flagged as i32, msg_id],
                )
                .map_err(|e| format!("更新邮件状态位失败 (UID {uid}): {e}"))?;
                updated += 1;
            }
        }

        if updated > 0 {
            let unread_count: u32 = tx
                .query_row(
                    "SELECT COUNT(*) FROM messages WHERE folder_id = ?1 AND unread = 1",
                    params![folder_id],
                    |row| row.get(0),
                )
                .map_err(|e| format!("统计未读数失败: {e}"))?;
            tx.execute(
                "UPDATE folders SET unread_count = ?1 WHERE id = ?2",
                params![unread_count, folder_id],
            )
            .map_err(|e| format!("更新文件夹未读数失败: {e}"))?;
        }

        tx.commit()
            .map_err(|e| format!("提交状态位对账事务失败: {e}"))?;
        Ok(updated)
    }

    /// Build an FTS5 MATCH expression for pure ASCII alphanumeric/phrase terms.
    /// Non-ASCII terms (e.g. Chinese like "发票", "合同") and punctuation terms fall
    /// back to SQLite `LIKE` so 100% of Chinese substrings and email addresses match.
    fn fts5_term(term: &str) -> Option<String> {
        if !term.is_ascii() || !term.chars().any(|c| c.is_ascii_alphanumeric()) {
            return None;
        }
        if !term
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c.is_whitespace())
        {
            return None;
        }
        let escaped = term.replace('"', "\"\"");
        if term.contains(char::is_whitespace) {
            Some(format!("\"{escaped}\""))
        } else {
            Some(format!("\"{escaped}\"*"))
        }
    }

    /// ORDER BY clause for [`MessageSort`] on table alias `m`.
    fn sort_sql_prefixed(sort: MessageSort) -> &'static str {
        match sort {
            MessageSort::DateNewest => "m.date_ts DESC, m.uid DESC",
            MessageSort::DateOldest => "m.date_ts ASC, m.uid ASC",
            MessageSort::SenderAsc => "LOWER(m.sender) ASC, m.date_ts DESC, m.uid DESC",
            MessageSort::SenderDesc => "LOWER(m.sender) DESC, m.date_ts DESC, m.uid DESC",
            MessageSort::SubjectAsc => "LOWER(m.subject) ASC, m.date_ts DESC, m.uid DESC",
            MessageSort::SubjectDesc => "LOWER(m.subject) DESC, m.date_ts DESC, m.uid DESC",
            MessageSort::UnreadFirst => "m.unread DESC, m.date_ts DESC, m.uid DESC",
            MessageSort::StarredFirst => "m.flagged DESC, m.date_ts DESC, m.uid DESC",
            MessageSort::AccountAsc => "m.account_id ASC, m.date_ts DESC, m.uid DESC",
            MessageSort::AccountDesc => "m.account_id DESC, m.date_ts DESC, m.uid DESC",
            MessageSort::AttachmentFirst => "m.has_attachment DESC, m.date_ts DESC, m.uid DESC",
            MessageSort::ImportanceFirst => "m.importance DESC, m.date_ts DESC, m.uid DESC",
            MessageSort::TagsAsc => "LOWER(m.tags) ASC, m.date_ts DESC, m.uid DESC",
        }
    }

    /// ORDER BY clause for [`MessageSort`] on unprefixed CTE columns.
    fn sort_sql_bare(sort: MessageSort) -> &'static str {
        match sort {
            MessageSort::DateNewest => "date_ts DESC, uid DESC",
            MessageSort::DateOldest => "date_ts ASC, uid ASC",
            MessageSort::SenderAsc => "LOWER(sender) ASC, date_ts DESC, uid DESC",
            MessageSort::SenderDesc => "LOWER(sender) DESC, date_ts DESC, uid DESC",
            MessageSort::SubjectAsc => "LOWER(subject) ASC, date_ts DESC, uid DESC",
            MessageSort::SubjectDesc => "LOWER(subject) DESC, date_ts DESC, uid DESC",
            MessageSort::UnreadFirst => "unread DESC, date_ts DESC, uid DESC",
            MessageSort::StarredFirst => "flagged DESC, date_ts DESC, uid DESC",
            MessageSort::AccountAsc => "account_id ASC, date_ts DESC, uid DESC",
            MessageSort::AccountDesc => "account_id DESC, date_ts DESC, uid DESC",
            MessageSort::AttachmentFirst => "has_attachment DESC, date_ts DESC, uid DESC",
            MessageSort::ImportanceFirst => "importance DESC, date_ts DESC, uid DESC",
            MessageSort::TagsAsc => "LOWER(tags) ASC, date_ts DESC, uid DESC",
        }
    }

    /// Append WHERE predicates and bind values for `quick_filters` and `parsed` search query.
    fn append_search_and_filter_clauses(
        parsed: &SearchQuery,
        quick_filters: QuickFilterSet,
        today_start_ts: i64,
        own_emails: &[String],
        clauses: &mut Vec<String>,
        args: &mut Vec<rusqlite::types::Value>,
    ) {
        if quick_filters.unread {
            clauses.push("m.unread = 1".to_string());
        }
        if quick_filters.starred {
            clauses.push("m.flagged = 1".to_string());
        }
        if quick_filters.has_attachment {
            clauses.push("m.has_attachment = 1".to_string());
        }
        if quick_filters.today {
            clauses.push("m.date_ts >= ?".to_string());
            args.push(today_start_ts.into());
        }
        if quick_filters.to_me {
            if own_emails.is_empty() {
                clauses.push("1 = 0".to_string());
            } else {
                let parts: Vec<&str> = own_emails
                    .iter()
                    .map(|_| "LOWER(m.recipients) LIKE ?")
                    .collect();
                clauses.push(format!("({})", parts.join(" OR ")));
                for email in own_emails {
                    args.push(format!("%{email}%").into());
                }
            }
        }
        if quick_filters.from_contacts {
            clauses.push(
                "EXISTS (SELECT 1 FROM user_contacts uc WHERE TRIM(uc.email) != '' AND LOWER(m.sender) LIKE '%' || LOWER(uc.email) || '%')"
                    .to_string(),
            );
        }
        if quick_filters.high_importance {
            clauses.push("m.importance > 0".to_string());
        }
        if quick_filters.tagged {
            clauses.push("TRIM(COALESCE(m.tags, '')) != ''".to_string());
        }
        if let Some(from_ts) = quick_filters.date_from_ts {
            clauses.push("m.date_ts >= ?".to_string());
            args.push(from_ts.into());
        }
        if let Some(to_ts) = quick_filters.date_to_ts {
            clauses.push("m.date_ts < ?".to_string());
            args.push(to_ts.into());
        }

        if parsed.attachment {
            clauses.push("m.has_attachment = 1".to_string());
        }
        if let Some(want_unread) = parsed.unread {
            clauses.push("m.unread = ?".to_string());
            args.push(i64::from(want_unread).into());
        }
        if parsed.starred {
            clauses.push("m.flagged = 1".to_string());
        }
        for val in &parsed.from {
            clauses.push("LOWER(m.sender) LIKE ?".to_string());
            args.push(format!("%{val}%").into());
        }
        for val in &parsed.to {
            clauses.push("LOWER(m.recipients) LIKE ?".to_string());
            args.push(format!("%{val}%").into());
        }
        for val in &parsed.subject {
            clauses.push("LOWER(m.subject) LIKE ?".to_string());
            args.push(format!("%{val}%").into());
        }
        for val in &parsed.folder {
            clauses.push("(LOWER(f.name) LIKE ? OR LOWER(f.path) LIKE ?)".to_string());
            let pat = format!("%{val}%");
            args.push(pat.clone().into());
            args.push(pat.into());
        }
        for term in &parsed.terms {
            if let Some(fts_q) = Self::fts5_term(term) {
                clauses.push(
                    "m.id IN (
                        SELECT rowid FROM messages_fts WHERE messages_fts MATCH ?
                        UNION ALL
                        SELECT rowid FROM messages_index_fts WHERE messages_index_fts MATCH ?
                    )"
                    .to_string(),
                );
                args.push(fts_q.clone().into());
                args.push(fts_q.into());
            } else {
                clauses.push(
                    "(LOWER(m.subject) LIKE ? OR LOWER(m.sender) LIKE ? OR LOWER(m.recipients) LIKE ? OR LOWER(m.preview) LIKE ? OR LOWER(COALESCE(m.body_text, m.index_text, '')) LIKE ?)"
                        .to_string(),
                );
                let pat = format!("%{term}%");
                for _ in 0..5 {
                    args.push(pat.clone().into());
                }
            }
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
        self.list_messages_page_with_options(
            folder_id,
            query,
            QuickFilterSet::from_message_filter(filter),
            MessageSort::DateNewest,
            offset,
            limit,
        )
    }

    /// Query a page of cached message summaries for `folder_id`, with structured search,
    /// multi-condition quick filters, and sort order.
    pub fn list_messages_page_with_options(
        &self,
        folder_id: u32,
        query: &str,
        quick_filters: QuickFilterSet,
        sort: MessageSort,
        offset: usize,
        limit: usize,
    ) -> Result<MailPage, String> {
        let parsed = SearchQuery::parse(query);
        let mut clauses = vec!["m.folder_id = ?".to_string()];
        let mut args: Vec<rusqlite::types::Value> = vec![i64::from(folder_id).into()];
        Self::append_search_and_filter_clauses(
            &parsed,
            quick_filters,
            start_of_today_ts(),
            &self.own_emails,
            &mut clauses,
            &mut args,
        );
        let where_sql = clauses.join(" AND ");
        let order_sql = Self::sort_sql_prefixed(sort);

        let total: i64 = self
            .conn
            .query_row(
                &format!(
                    "
                    SELECT COUNT(*)
                    FROM messages m
                    JOIN folders f ON f.id = m.folder_id
                    WHERE {where_sql}
                    "
                ),
                rusqlite::params_from_iter(args.iter()),
                |row| row.get(0),
            )
            .map_err(|e| format!("统计邮件分页总数失败: {e}"))?;

        let mut page_args = args;
        page_args.push((limit as i64).into());
        page_args.push((offset as i64).into());

        let mut stmt = self
            .conn
            .prepare(&format!(
                "
                SELECT m.id, m.account_id, m.folder_id, f.path, m.uid, m.sender, m.subject,
                       m.preview, m.date_ts, m.date_display, m.unread, m.flagged, m.has_attachment,
                       m.recipients, m.importance, m.tags, m.answered, m.forwarded, m.snoozed_until_ts
                FROM messages m
                JOIN folders f ON f.id = m.folder_id
                WHERE {where_sql}
                ORDER BY {order_sql}
                LIMIT ? OFFSET ?
                "
            ))
            .map_err(|e| format!("准备分页查询失败: {e}"))?;

        let mut items = stmt
            .query_map(
                rusqlite::params_from_iter(page_args.iter()),
                map_summary_row,
            )
            .map_err(|e| format!("执行分页查询失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析分页结果失败: {e}"))?;
        self.fill_thread_counts(&mut items);

        Ok(MailPage {
            folder_id,
            offset,
            total: total.max(0) as usize,
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
        self.list_unified_inbox_with_options(
            query,
            QuickFilterSet::from_message_filter(filter),
            MessageSort::DateNewest,
            offset,
            limit,
        )
    }

    /// Query a unified inbox page across all accounts with [`QuickFilterSet`] and [`MessageSort`].
    pub fn list_unified_inbox_with_options(
        &self,
        query: &str,
        quick_filters: QuickFilterSet,
        sort: MessageSort,
        offset: usize,
        limit: usize,
    ) -> Result<MailPage, String> {
        self.list_role_messages_with_options(
            "(UPPER(f.path) = 'INBOX' OR f.is_inbox = 1)",
            "统一收件箱",
            query,
            quick_filters,
            sort,
            offset,
            limit,
        )
    }

    /// Query a unified inbox page restricted to `allowed_account_ids`.
    #[allow(clippy::too_many_arguments)]
    pub fn list_unified_inbox_for_accounts(
        &self,
        allowed_account_ids: &[u32],
        query: &str,
        quick_filters: QuickFilterSet,
        sort: MessageSort,
        offset: usize,
        limit: usize,
    ) -> Result<MailPage, String> {
        self.list_role_messages_with_options_for_accounts(
            "(UPPER(f.path) = 'INBOX' OR f.is_inbox = 1)",
            "统一收件箱",
            Some(allowed_account_ids),
            query,
            quick_filters,
            sort,
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
        self.list_starred_messages_with_options(
            query,
            QuickFilterSet::from_message_filter(filter),
            MessageSort::DateNewest,
            offset,
            limit,
        )
    }

    /// Query a page of starred messages with [`QuickFilterSet`] and [`MessageSort`].
    pub fn list_starred_messages_with_options(
        &self,
        query: &str,
        quick_filters: QuickFilterSet,
        sort: MessageSort,
        offset: usize,
        limit: usize,
    ) -> Result<MailPage, String> {
        self.list_role_messages_with_options(
            "m.flagged = 1",
            "星标邮件",
            query,
            quick_filters,
            sort,
            offset,
            limit,
        )
    }

    /// Query a page of starred messages restricted to `allowed_account_ids`.
    #[allow(clippy::too_many_arguments)]
    pub fn list_starred_messages_for_accounts(
        &self,
        allowed_account_ids: &[u32],
        query: &str,
        quick_filters: QuickFilterSet,
        sort: MessageSort,
        offset: usize,
        limit: usize,
    ) -> Result<MailPage, String> {
        self.list_role_messages_with_options_for_accounts(
            "m.flagged = 1",
            "星标邮件",
            Some(allowed_account_ids),
            query,
            quick_filters,
            sort,
            offset,
            limit,
        )
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
        self.list_sent_messages_with_options(
            query,
            QuickFilterSet::from_message_filter(filter),
            MessageSort::DateNewest,
            offset,
            limit,
        )
    }

    /// Query a page of sent messages across all accounts with [`QuickFilterSet`] and [`MessageSort`].
    pub fn list_sent_messages_with_options(
        &self,
        query: &str,
        quick_filters: QuickFilterSet,
        sort: MessageSort,
        offset: usize,
        limit: usize,
    ) -> Result<MailPage, String> {
        self.list_role_messages_with_options(
            SENT_FOLDER_MATCH,
            "已发送",
            query,
            quick_filters,
            sort,
            offset,
            limit,
        )
    }

    /// Query a page of sent messages restricted to `allowed_account_ids`.
    #[allow(clippy::too_many_arguments)]
    pub fn list_sent_messages_for_accounts(
        &self,
        allowed_account_ids: &[u32],
        query: &str,
        quick_filters: QuickFilterSet,
        sort: MessageSort,
        offset: usize,
        limit: usize,
    ) -> Result<MailPage, String> {
        self.list_role_messages_with_options_for_accounts(
            SENT_FOLDER_MATCH,
            "已发送",
            Some(allowed_account_ids),
            query,
            quick_filters,
            sort,
            offset,
            limit,
        )
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
        self.list_archived_messages_with_options(
            query,
            QuickFilterSet::from_message_filter(filter),
            MessageSort::DateNewest,
            offset,
            limit,
        )
    }

    /// Query a page of archived messages across all accounts with [`QuickFilterSet`] and [`MessageSort`].
    pub fn list_archived_messages_with_options(
        &self,
        query: &str,
        quick_filters: QuickFilterSet,
        sort: MessageSort,
        offset: usize,
        limit: usize,
    ) -> Result<MailPage, String> {
        self.list_role_messages_with_options(
            ARCHIVE_FOLDER_MATCH,
            "归档",
            query,
            quick_filters,
            sort,
            offset,
            limit,
        )
    }

    /// Query a page of archived messages restricted to `allowed_account_ids`.
    #[allow(clippy::too_many_arguments)]
    pub fn list_archived_messages_for_accounts(
        &self,
        allowed_account_ids: &[u32],
        query: &str,
        quick_filters: QuickFilterSet,
        sort: MessageSort,
        offset: usize,
        limit: usize,
    ) -> Result<MailPage, String> {
        self.list_role_messages_with_options_for_accounts(
            ARCHIVE_FOLDER_MATCH,
            "归档",
            Some(allowed_account_ids),
            query,
            quick_filters,
            sort,
            offset,
            limit,
        )
    }

    /// Cross-account, cross-folder local search.
    ///
    /// Only messages of `allowed_account_ids` are searched (the enabled
    /// accounts, see `folder_select` in the Linux app). Excludes Trash and
    /// Junk folders by default (unless the query explicitly specifies an
    /// `in:` / `folder:` operator), and deduplicates by non-empty
    /// `message_id` preferring Inbox > regular folders > Archive > Trash/Junk.
    pub fn search_all_folders(
        &self,
        allowed_account_ids: &[u32],
        query: &str,
        quick_filters: QuickFilterSet,
        sort: MessageSort,
        offset: usize,
        limit: usize,
    ) -> Result<MailPage, String> {
        if allowed_account_ids.is_empty() {
            return Ok(MailPage {
                folder_id: 0,
                offset,
                total: 0,
                items: Vec::new(),
            });
        }
        let parsed = SearchQuery::parse(query);
        let mut clauses: Vec<String> = Vec::new();
        let mut args: Vec<rusqlite::types::Value> = Vec::new();
        let placeholders = vec!["?"; allowed_account_ids.len()].join(", ");
        clauses.push(format!("m.account_id IN ({placeholders})"));
        for &id in allowed_account_ids {
            args.push(i64::from(id).into());
        }
        if parsed.folder.is_empty() {
            clauses.push(format!("NOT ({TRASH_FOLDER_MATCH} OR {JUNK_FOLDER_MATCH})"));
        }
        Self::append_search_and_filter_clauses(
            &parsed,
            quick_filters,
            start_of_today_ts(),
            &self.own_emails,
            &mut clauses,
            &mut args,
        );
        let where_sql = if clauses.is_empty() {
            "1 = 1".to_string()
        } else {
            clauses.join(" AND ")
        };
        let order_sql = Self::sort_sql_bare(sort);
        let cte = format!(
            "
            WITH ranked AS (
                SELECT m.id, m.account_id, m.folder_id, f.path AS folder_path, m.uid, m.sender, m.subject,
                       m.preview, m.date_ts, m.date_display, m.unread, m.flagged, m.has_attachment,
                       m.recipients, m.importance, m.tags, m.answered, m.forwarded, m.snoozed_until_ts,
                       ROW_NUMBER() OVER (
                           PARTITION BY CASE
                               WHEN m.message_id != '' THEN m.message_id
                               ELSE 'id:' || CAST(m.id AS TEXT)
                           END
                           ORDER BY CASE
                               WHEN UPPER(f.path) = 'INBOX' OR f.is_inbox = 1 THEN 0
                               WHEN {TRASH_FOLDER_MATCH} OR {JUNK_FOLDER_MATCH} THEN 3
                               WHEN {ARCHIVE_FOLDER_MATCH} THEN 2
                               ELSE 1
                           END ASC, m.id ASC
                       ) AS rn
                FROM messages m
                JOIN folders f ON f.id = m.folder_id
                WHERE {where_sql}
            )
            "
        );

        let total: i64 = self
            .conn
            .query_row(
                &format!("{cte} SELECT COUNT(*) FROM ranked WHERE rn = 1"),
                rusqlite::params_from_iter(args.iter()),
                |row| row.get(0),
            )
            .map_err(|e| format!("统计跨文件夹搜索总数失败: {e}"))?;

        let mut page_args = args;
        page_args.push((limit as i64).into());
        page_args.push((offset as i64).into());

        let mut stmt = self
            .conn
            .prepare(&format!(
                "
                {cte}
                SELECT id, account_id, folder_id, folder_path, uid, sender, subject,
                       preview, date_ts, date_display, unread, flagged, has_attachment,
                       recipients, importance, tags, answered, forwarded, snoozed_until_ts
                FROM ranked
                WHERE rn = 1
                ORDER BY {order_sql}
                LIMIT ? OFFSET ?
                "
            ))
            .map_err(|e| format!("准备跨文件夹搜索查询失败: {e}"))?;

        let mut items = stmt
            .query_map(
                rusqlite::params_from_iter(page_args.iter()),
                map_summary_row,
            )
            .map_err(|e| format!("执行跨文件夹搜索查询失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析跨文件夹搜索结果失败: {e}"))?;
        self.fill_thread_counts(&mut items);

        Ok(MailPage {
            folder_id: 0,
            offset,
            total: total.max(0) as usize,
            items,
        })
    }

    /// Find the account's Inbox folder `(folder_id, path)` if synced.
    pub fn find_inbox_folder(&self, account_id: u32) -> Result<Option<(u32, String)>, String> {
        self.find_role_folder(account_id, "(UPPER(f.path) = 'INBOX' OR f.is_inbox = 1)")
    }

    /// B-4：为页面内邮件回填会话计数 `thread_count`（同账号内同一会话的邮件总数）。
    ///
    /// 数据来源（只读扩展，不改 schema）：按 `normalize_thread_subject` 归一化主题计数。
    /// 说明：RFC 会话闭包需要逐封递归展开（见 [`Self::list_thread_messages`]），对列表页
    /// 代价过高，这里降级为「同主题（归一化）计数」——先按原始 subject 在 SQL 侧
    /// `GROUP BY` 计数，再在 Rust 侧按归一化键聚合，语义与 `list_thread_messages`
    /// 的主题回退路径一致。
    fn fill_thread_counts(&self, items: &mut [MessageSummary]) {
        if items.is_empty() {
            return;
        }
        // 每个账号一次查询：页内主题分批绑定（避免超出 SQLite 变量上限）。
        let mut accounts: Vec<u32> = items.iter().map(|m| m.account_id).collect();
        accounts.sort_unstable();
        accounts.dedup();

        let mut counts: HashMap<(u32, String), u32> = HashMap::new();
        for account_id in accounts {
            let mut subjects: Vec<String> = items
                .iter()
                .filter(|m| m.account_id == account_id)
                .map(|m| m.subject.to_lowercase())
                .collect();
            subjects.sort_unstable();
            subjects.dedup();
            for chunk in subjects.chunks(200) {
                let placeholders = vec!["?"; chunk.len()].join(", ");
                let sql = format!(
                    "
                    SELECT subject, COUNT(*) FROM messages
                    WHERE account_id = ? AND LOWER(subject) IN ({placeholders})
                    GROUP BY subject
                    "
                );
                let mut args: Vec<rusqlite::types::Value> = vec![i64::from(account_id).into()];
                args.extend(chunk.iter().map(|s| s.clone().into()));
                let Ok(mut stmt) = self.conn.prepare(&sql) else {
                    continue;
                };
                let Ok(rows) = stmt
                    .query_map(rusqlite::params_from_iter(args.iter()), |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                    })
                    .map(|iter| iter.collect::<Result<Vec<_>, _>>())
                else {
                    continue;
                };
                if let Ok(rows) = rows {
                    for (subject, count) in rows {
                        let key = (account_id, normalize_thread_subject(&subject));
                        *counts.entry(key).or_insert(0) += count.max(0) as u32;
                    }
                }
            }
        }

        for m in items.iter_mut() {
            let key = (m.account_id, normalize_thread_subject(&m.subject));
            m.thread_count = counts.get(&key).copied().unwrap_or(1).max(1);
        }
    }

    /// Look up cached [`MessageSummary`] rows by `(folder_id, uids)` (used after server search commits).
    pub fn list_summaries_by_uids(
        &self,
        folder_id: u32,
        uids: &[u32],
    ) -> Result<Vec<MessageSummary>, String> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        let marks = vec!["?"; uids.len()].join(", ");
        let mut args: Vec<rusqlite::types::Value> = vec![i64::from(folder_id).into()];
        args.extend(uids.iter().map(|u| i64::from(*u).into()));
        let mut stmt = self
            .conn
            .prepare(&format!(
                "
                SELECT m.id, m.account_id, m.folder_id, f.path, m.uid, m.sender, m.subject,
                       m.preview, m.date_ts, m.date_display, m.unread, m.flagged, m.has_attachment,
                       m.recipients, m.importance, m.tags, m.answered, m.forwarded, m.snoozed_until_ts
                FROM messages m
                JOIN folders f ON f.id = m.folder_id
                WHERE m.folder_id = ? AND m.uid IN ({marks})
                ORDER BY m.date_ts DESC, m.uid DESC
                "
            ))
            .map_err(|e| format!("准备按 UID 查询邮件摘要失败: {e}"))?;
        let mut rows = stmt
            .query_map(rusqlite::params_from_iter(args.iter()), map_summary_row)
            .map_err(|e| format!("执行按 UID 查询邮件摘要失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析按 UID 查询邮件摘要失败: {e}"))?;
        self.fill_thread_counts(&mut rows);
        Ok(rows)
    }

    /// Shared implementation for unified message pages: `predicate` is a trusted
    /// constant SQL fragment applied to the `messages JOIN folders` row set.
    #[allow(clippy::too_many_arguments)]
    fn list_role_messages_with_options(
        &self,
        predicate: &str,
        label: &str,
        query: &str,
        quick_filters: QuickFilterSet,
        sort: MessageSort,
        offset: usize,
        limit: usize,
    ) -> Result<MailPage, String> {
        self.list_role_messages_with_options_for_accounts(
            predicate,
            label,
            None,
            query,
            quick_filters,
            sort,
            offset,
            limit,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn list_role_messages_with_options_for_accounts(
        &self,
        predicate: &str,
        label: &str,
        allowed_account_ids: Option<&[u32]>,
        query: &str,
        quick_filters: QuickFilterSet,
        sort: MessageSort,
        offset: usize,
        limit: usize,
    ) -> Result<MailPage, String> {
        if let Some(ids) = allowed_account_ids {
            if ids.is_empty() {
                return Ok(MailPage {
                    folder_id: 0,
                    offset,
                    total: 0,
                    items: Vec::new(),
                });
            }
        }
        let parsed = SearchQuery::parse(query);
        let mut clauses = vec![predicate.to_string()];
        let mut args: Vec<rusqlite::types::Value> = Vec::new();
        if let Some(ids) = allowed_account_ids {
            let placeholders = vec!["?"; ids.len()].join(", ");
            clauses.push(format!("m.account_id IN ({placeholders})"));
            for &id in ids {
                args.push(i64::from(id).into());
            }
        }
        Self::append_search_and_filter_clauses(
            &parsed,
            quick_filters,
            start_of_today_ts(),
            &self.own_emails,
            &mut clauses,
            &mut args,
        );
        let where_sql = clauses.join(" AND ");
        let order_sql = Self::sort_sql_prefixed(sort);

        let total: i64 = self
            .conn
            .query_row(
                &format!(
                    "
                    SELECT COUNT(*)
                    FROM messages m
                    JOIN folders f ON f.id = m.folder_id
                    WHERE {where_sql}
                    "
                ),
                rusqlite::params_from_iter(args.iter()),
                |row| row.get(0),
            )
            .map_err(|e| format!("统计{label}总数失败: {e}"))?;

        let mut page_args = args;
        page_args.push((limit as i64).into());
        page_args.push((offset as i64).into());

        let mut stmt = self
            .conn
            .prepare(&format!(
                "
                SELECT m.id, m.account_id, m.folder_id, f.path, m.uid, m.sender, m.subject,
                       m.preview, m.date_ts, m.date_display, m.unread, m.flagged, m.has_attachment,
                       m.recipients, m.importance, m.tags, m.answered, m.forwarded, m.snoozed_until_ts
                FROM messages m
                JOIN folders f ON f.id = m.folder_id
                WHERE {where_sql}
                ORDER BY {order_sql}
                LIMIT ? OFFSET ?
                "
            ))
            .map_err(|e| format!("准备{label}分页查询失败: {e}"))?;

        let mut items = stmt
            .query_map(
                rusqlite::params_from_iter(page_args.iter()),
                map_summary_row,
            )
            .map_err(|e| format!("执行{label}分页查询失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析{label}结果失败: {e}"))?;
        self.fill_thread_counts(&mut items);

        Ok(MailPage {
            folder_id: 0,
            offset,
            total: total.max(0) as usize,
            items,
        })
    }

    /// Return cached message detail (default newest-first thread order) and whether its body has already been downloaded.
    pub fn get_message_detail(
        &self,
        message_id: u32,
    ) -> Result<Option<(MessageDetail, u32, bool)>, String> {
        self.get_message_detail_ordered(message_id, true)
    }

    /// Return cached message detail with explicit thread ordering (`newest_first`)
    /// and whether its body has already been downloaded.
    pub fn get_message_detail_ordered(
        &self,
        message_id: u32,
        newest_first: bool,
    ) -> Result<Option<(MessageDetail, u32, bool)>, String> {
        let row = self
            .conn
            .query_row(
                "
                SELECT m.id, m.account_id, m.folder_id, m.uid, m.message_id, m.sender, m.recipients,
                       m.subject, m.date_ts, m.date_display, m.body_text, m.body_html, m.flagged, f.path,
                       m.list_unsubscribe, m.list_unsubscribe_post
                FROM messages m
                JOIN folders f ON f.id = m.folder_id
                WHERE m.id = ?1
                ",
                params![message_id],
                |row| {
                    let body: Option<String> = row.get(10)?;
                    let body_html: Option<String> = row.get(11)?;
                    let unsub: String = row.get(14)?;
                    Ok((
                        row.get::<_, u32>(0)?,
                        row.get::<_, u32>(1)?,
                        row.get::<_, u32>(2)?,
                        row.get::<_, u32>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, String>(9)?,
                        body,
                        body_html,
                        row.get::<_, i32>(12)? != 0,
                        row.get::<_, String>(13)?,
                        (!unsub.trim().is_empty()).then_some(unsub),
                        row.get::<_, i32>(15)? != 0,
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
            date_ts,
            date_display,
            body_opt,
            body_html,
            flagged,
            folder_path,
            unsubscribe_url,
            unsubscribe_one_click,
        )) = row
        else {
            return Ok(None);
        };

        let attachments = self.list_attachments(id)?;
        let body_loaded = body_opt.is_some();
        let body_text = body_opt.unwrap_or_default();
        let thread_messages = self
            .list_thread_messages(account_id, id, &subject, newest_first)
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
            date_ts,
            date_display,
            body_text,
            body_html,
            flagged,
            attachments,
            thread_messages,
            unsubscribe_url,
            unsubscribe_one_click,
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

    /// Aggregate messages belonging to the same conversation thread for `account_id`:
    /// 1. Prefer RFC threading via `message_id` / `in_reply_to` / `references_header`
    ///    (including Graph `conversationId` stored in `references_header`).
    /// 2. Fall back to `normalize_thread_subject(subject)` when no RFC reply headers link
    ///    multiple messages.
    /// 3. Deduplicate cross-folder copies sharing `(message_id, sender, date_ts)`,
    ///    preferring the anchor message > anchor folder > non-Trash folder.
    /// 4. Order by `newest_first` (`true` = `date_ts DESC, uid DESC`, `false` = `date_ts ASC, uid ASC`).
    pub fn list_thread_messages(
        &self,
        account_id: u32,
        message_id: u32,
        subject: &str,
        newest_first: bool,
    ) -> Result<Vec<ThreadMessage>, String> {
        struct RawRow {
            id: u32,
            folder_id: u32,
            folder_name: String,
            folder_path: String,
            uid: u32,
            rfc_message_id: String,
            in_reply_to: String,
            references_header: String,
            sender: String,
            recipients: String,
            subject: String,
            date_ts: i64,
            date_display: String,
            body_opt: Option<String>,
            body_html: Option<String>,
            preview: String,
            unread: bool,
            flagged: bool,
        }

        // 1. Inspect the anchor message (if provided) for folder & RFC thread headers.
        let anchor_info: Option<(u32, String, String, String)> = if message_id != 0 {
            self.conn
                .query_row(
                    "
                    SELECT folder_id, message_id, in_reply_to, references_header
                    FROM messages
                    WHERE id = ?1 AND account_id = ?2
                    ",
                    params![message_id, account_id],
                    |row| {
                        Ok((
                            row.get::<_, u32>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                        ))
                    },
                )
                .optional()
                .map_err(|e| format!("查询会话锚点邮件失败: {e}"))?
        } else {
            None
        };

        let anchor_folder_id = anchor_info.as_ref().map(|(fid, _, _, _)| *fid);
        let mut seed_ids: HashSet<String> = HashSet::new();
        let mut anchor_has_reply_headers = false;
        if let Some((_, ref mid, ref irt, ref refs)) = anchor_info {
            for tok in extract_rfc_message_ids(mid) {
                seed_ids.insert(tok);
            }
            let irt_ids = extract_rfc_message_ids(irt);
            let ref_ids = extract_rfc_message_ids(refs);
            if !irt_ids.is_empty() || !ref_ids.is_empty() {
                anchor_has_reply_headers = true;
            }
            for tok in irt_ids.into_iter().chain(ref_ids) {
                seed_ids.insert(tok);
            }
        }

        // 2. Try RFC header-based transitive grouping first.
        let mut matched_ids: HashSet<u32> = HashSet::new();
        if !seed_ids.is_empty() {
            let mut stmt = self
                .conn
                .prepare(
                    "
                    SELECT id, message_id, in_reply_to, references_header
                    FROM messages
                    WHERE account_id = ?1
                      AND (message_id != '' OR in_reply_to != '' OR references_header != '')
                    ",
                )
                .map_err(|e| format!("准备 RFC 会话索引查询失败: {e}"))?;
            let candidates: Vec<(u32, Vec<String>)> = stmt
                .query_map(params![account_id], |row| {
                    let id: u32 = row.get(0)?;
                    let mid: String = row.get(1)?;
                    let irt: String = row.get(2)?;
                    let refs: String = row.get(3)?;
                    let mut ids = extract_rfc_message_ids(&mid);
                    ids.extend(extract_rfc_message_ids(&irt));
                    ids.extend(extract_rfc_message_ids(&refs));
                    Ok((id, ids))
                })
                .map_err(|e| format!("执行 RFC 会话索引查询失败: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("解析 RFC 会话索引失败: {e}"))?;

            let mut thread_ids = seed_ids;
            for _ in 0..8 {
                let mut changed = false;
                for (cid, cids) in &candidates {
                    if cids.iter().any(|t| thread_ids.contains(t)) {
                        if matched_ids.insert(*cid) {
                            changed = true;
                        }
                        for t in cids {
                            if thread_ids.insert(t.clone()) {
                                changed = true;
                            }
                        }
                    }
                }
                if !changed {
                    break;
                }
            }
            if message_id != 0 && anchor_info.is_some() {
                matched_ids.insert(message_id);
            }
        }

        // 3. Load candidate rows either from RFC matches or from subject fallback.
        let use_rfc =
            matched_ids.len() >= 2 || (anchor_has_reply_headers && !matched_ids.is_empty());

        let mut raw_rows: Vec<RawRow> = Vec::new();
        if use_rfc {
            let mut stmt = self
                .conn
                .prepare(
                    "
                    SELECT m.id, m.folder_id, f.name, f.path, m.uid, m.message_id,
                           m.in_reply_to, m.references_header, m.sender, m.recipients,
                           m.subject, m.date_ts, m.date_display, m.body_text, m.body_html,
                           m.preview, m.unread, m.flagged
                    FROM messages m
                    JOIN folders f ON f.id = m.folder_id
                    WHERE m.id = ?1
                    ",
                )
                .map_err(|e| format!("准备 RFC 会话详情查询失败: {e}"))?;
            for cid in matched_ids {
                if let Some(row) = stmt
                    .query_row(params![cid], |row| {
                        Ok(RawRow {
                            id: row.get(0)?,
                            folder_id: row.get(1)?,
                            folder_name: row.get(2)?,
                            folder_path: row.get(3)?,
                            uid: row.get(4)?,
                            rfc_message_id: row.get(5)?,
                            in_reply_to: row.get(6)?,
                            references_header: row.get(7)?,
                            sender: row.get(8)?,
                            recipients: row.get(9)?,
                            subject: row.get(10)?,
                            date_ts: row.get(11)?,
                            date_display: row.get(12)?,
                            body_opt: row.get(13)?,
                            body_html: row.get(14)?,
                            preview: row.get(15)?,
                            unread: row.get::<_, i32>(16)? != 0,
                            flagged: row.get::<_, i32>(17)? != 0,
                        })
                    })
                    .optional()
                    .map_err(|e| format!("读取 RFC 会话行失败: {e}"))?
                {
                    let _ = (&row.in_reply_to, &row.references_header);
                    raw_rows.push(row);
                }
            }
        } else {
            let key = normalize_thread_subject(subject);
            if key.is_empty() {
                return Ok(Vec::new());
            }
            let pattern = format!("%{key}%");
            let order_sql = if newest_first {
                "m.date_ts DESC, m.uid DESC"
            } else {
                "m.date_ts ASC, m.uid ASC"
            };
            let mut stmt = self
                .conn
                .prepare(&format!(
                    "
                    SELECT m.id, m.folder_id, f.name, f.path, m.uid, m.message_id,
                           m.in_reply_to, m.references_header, m.sender, m.recipients,
                           m.subject, m.date_ts, m.date_display, m.body_text, m.body_html,
                           m.preview, m.unread, m.flagged
                    FROM messages m
                    JOIN folders f ON f.id = m.folder_id
                    WHERE m.account_id = ?1 AND LOWER(m.subject) LIKE ?2
                    ORDER BY {order_sql}
                    LIMIT 60
                    "
                ))
                .map_err(|e| format!("准备会话查询失败: {e}"))?;

            let rows = stmt
                .query_map(params![account_id, pattern], |row| {
                    Ok(RawRow {
                        id: row.get(0)?,
                        folder_id: row.get(1)?,
                        folder_name: row.get(2)?,
                        folder_path: row.get(3)?,
                        uid: row.get(4)?,
                        rfc_message_id: row.get(5)?,
                        in_reply_to: row.get(6)?,
                        references_header: row.get(7)?,
                        sender: row.get(8)?,
                        recipients: row.get(9)?,
                        subject: row.get(10)?,
                        date_ts: row.get(11)?,
                        date_display: row.get(12)?,
                        body_opt: row.get(13)?,
                        body_html: row.get(14)?,
                        preview: row.get(15)?,
                        unread: row.get::<_, i32>(16)? != 0,
                        flagged: row.get::<_, i32>(17)? != 0,
                    })
                })
                .map_err(|e| format!("执行会话查询失败: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("解析会话结果失败: {e}"))?;

            for row in rows {
                if normalize_thread_subject(&row.subject) == key {
                    let _ = (&row.in_reply_to, &row.references_header);
                    raw_rows.push(row);
                }
            }
        }

        // 4. Exclude Trash/Junk rows unless the anchor message itself lives in Trash/Junk,
        //    then deduplicate cross-folder copies with same (message_id, sender, date_ts),
        //    preferring anchor message > anchor folder > non-Trash/Junk folder.
        let anchor_in_trash_or_junk = anchor_folder_id
            .and_then(|fid| {
                raw_rows.iter().find(|r| r.folder_id == fid).map(|r| {
                    is_trash_folder(&r.folder_name, &r.folder_path)
                        || is_junk_folder(&r.folder_name, &r.folder_path)
                })
            })
            .unwrap_or(false);

        if !anchor_in_trash_or_junk {
            raw_rows.retain(|r| {
                (message_id != 0 && r.id == message_id)
                    || (!is_trash_folder(&r.folder_name, &r.folder_path)
                        && !is_junk_folder(&r.folder_name, &r.folder_path))
            });
        }

        let copy_priority = |r: &RawRow| -> u8 {
            if message_id != 0 && r.id == message_id {
                3
            } else if anchor_folder_id == Some(r.folder_id) {
                2
            } else if !is_trash_folder(&r.folder_name, &r.folder_path)
                && !is_junk_folder(&r.folder_name, &r.folder_path)
            {
                1
            } else {
                0
            }
        };

        let mut deduped: Vec<RawRow> = Vec::with_capacity(raw_rows.len());
        let mut by_key: HashMap<(String, String, i64), usize> = HashMap::new();
        for row in raw_rows {
            let norm_mid = extract_rfc_message_ids(&row.rfc_message_id)
                .into_iter()
                .next()
                .unwrap_or_default();
            if norm_mid.is_empty() {
                deduped.push(row);
                continue;
            }
            let key = (norm_mid, row.sender.clone(), row.date_ts);
            if let Some(&existing_idx) = by_key.get(&key) {
                if copy_priority(&row) > copy_priority(&deduped[existing_idx]) {
                    deduped[existing_idx] = row;
                }
            } else {
                let idx = deduped.len();
                deduped.push(row);
                by_key.insert(key, idx);
            }
        }

        // 5. Sort by requested direction.
        if newest_first {
            deduped.sort_by(|a, b| {
                b.date_ts
                    .cmp(&a.date_ts)
                    .then_with(|| b.uid.cmp(&a.uid))
                    .then_with(|| b.id.cmp(&a.id))
            });
        } else {
            deduped.sort_by(|a, b| {
                a.date_ts
                    .cmp(&b.date_ts)
                    .then_with(|| a.uid.cmp(&b.uid))
                    .then_with(|| a.id.cmp(&b.id))
            });
        }
        deduped.truncate(40);

        let mut out = Vec::with_capacity(deduped.len());
        for row in deduped {
            let body_loaded = row.body_opt.is_some();
            let body_text = row.body_opt.unwrap_or(row.preview);
            let attachments = self.list_attachments(row.id).unwrap_or_default();
            out.push(ThreadMessage {
                id: row.id,
                rfc_message_id: row.rfc_message_id,
                subject: row.subject,
                sender: row.sender,
                recipients: row.recipients,
                date_ts: row.date_ts,
                date_display: row.date_display,
                body_text,
                body_html: row.body_html,
                body_loaded,
                unread: row.unread,
                flagged: row.flagged,
                attachments,
                uid: row.uid,
                folder_path: row.folder_path,
            });
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
        self.store_message_body_with_html(message_id, body_text, None, attachments, None, false)
    }

    /// Save fetched message plain text, optional HTML body, attachments, and the
    /// `List-Unsubscribe` handles parsed from the raw headers (F-7) in one transaction.
    pub fn store_message_body_with_html(
        &mut self,
        message_id: u32,
        body_text: &str,
        body_html: Option<&str>,
        attachments: &[CachedAttachmentInput],
        unsubscribe_url: Option<&str>,
        unsubscribe_one_click: bool,
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
                index_text = NULL,
                has_attachment = CASE WHEN ?3 > 0 THEN 1 ELSE has_attachment END,
                list_unsubscribe = ?5,
                list_unsubscribe_post = ?6
            WHERE id = ?4
            ",
            params![
                body_text,
                body_html,
                attachments.len() as i64,
                message_id,
                unsubscribe_url.unwrap_or_default(),
                unsubscribe_one_click as i64,
            ],
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

    /// Clear cached bodies, search text and attachment blobs while keeping
    /// message headers (sender / subject / preview / flags) intact.
    /// Returns the number of messages whose cached body was dropped.
    pub fn clear_message_bodies(&mut self) -> Result<usize, String> {
        let cleared: usize = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE body_text IS NOT NULL OR index_text <> ''",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n.max(0) as usize)
            .map_err(|e| format!("统计已缓存正文数失败: {e}"))?;

        let tx = self
            .conn
            .transaction()
            .map_err(|e| format!("开启正文缓存清理事务失败: {e}"))?;
        tx.execute(
            "UPDATE messages SET body_text = NULL, body_html = NULL, index_text = NULL
             WHERE body_text IS NOT NULL OR body_html IS NOT NULL OR index_text IS NOT NULL",
            [],
        )
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
                           m.sender, m.subject, m.date_display, m.date_ts
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
                           m.sender, m.subject, m.date_display, m.date_ts
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

    /// Aggregate known contacts from cached senders/recipients, merged with the
    /// user-maintained `user_contacts` table (F-5)：自定义联系人排在最前并带
    /// `custom` 标记（往来邮件数沿用聚合统计，无记录则为 0），其余按邮件数
    /// 降序。按 `query` 对姓名 / 邮箱过滤，邮箱去重（小写键）。
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
                ..KnownContact::default()
            })
            .collect();

        // F-5：叠加用户自定义联系人（邮箱冲突时覆盖聚合行的姓名 / 备注）。
        for user in self.list_user_contacts()? {
            let key = user.email.trim().to_lowercase();
            if key.is_empty() {
                continue;
            }
            if let Some(existing) = contacts.iter_mut().find(|c| c.email.to_lowercase() == key) {
                existing.id = user.id;
                existing.notes = user.notes;
                existing.custom = true;
                if !user.display_name.trim().is_empty() {
                    existing.name = user.display_name;
                }
            } else {
                contacts.push(KnownContact {
                    name: user.display_name,
                    email: user.email,
                    message_count: 0,
                    id: user.id,
                    notes: user.notes,
                    custom: true,
                });
            }
        }

        let mut contacts: Vec<KnownContact> = contacts
            .into_iter()
            .filter(|c| {
                trimmed.is_empty()
                    || c.name.to_lowercase().contains(&trimmed)
                    || c.email.to_lowercase().contains(&trimmed)
            })
            .collect();

        // 自定义联系人优先，其余按往来邮件数降序（平局按邮箱稳定次序）。
        contacts.sort_by(|a, b| {
            b.custom
                .cmp(&a.custom)
                .then_with(|| b.message_count.cmp(&a.message_count))
                .then_with(|| a.email.cmp(&b.email))
        });
        Ok(contacts.into_iter().skip(offset).take(limit).collect())
    }

    /// List every user-maintained contact (F-5)，按创建顺序返回。
    pub fn list_user_contacts(&self) -> Result<Vec<UserContact>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "
                SELECT id, email, display_name, notes
                FROM user_contacts
                ORDER BY id ASC
                ",
            )
            .map_err(|e| format!("查询用户联系人失败: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(UserContact {
                    id: row.get::<_, i64>(0)? as u32,
                    email: row.get(1)?,
                    display_name: row.get(2)?,
                    notes: row.get(3)?,
                })
            })
            .map_err(|e| format!("读取用户联系人失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析用户联系人失败: {e}"))?;
        Ok(rows)
    }

    /// Insert or update a user-maintained contact (F-5)。`id == 0` 插入；
    /// 插入时邮箱唯一冲突报错，更新时不得改与其他行冲突的邮箱。
    pub fn upsert_user_contact(&mut self, contact: &UserContact) -> Result<u32, String> {
        let email = contact.email.trim().to_string();
        if !email.contains('@') {
            return Err("邮箱地址不合法".to_string());
        }
        let now = chrono::Utc::now().timestamp();
        let display_name = contact.display_name.trim().to_string();
        let notes = contact.notes.clone();
        if contact.id == 0 {
            self.conn
                .execute(
                    "
                    INSERT INTO user_contacts (email, display_name, notes, created_at, updated_at)
                    VALUES (?1, ?2, ?3, ?4, ?4)
                    ",
                    params![email, display_name, notes, now],
                )
                .map_err(|e| {
                    if format!("{e}").contains("UNIQUE") {
                        format!("已存在邮箱为 {email} 的联系人")
                    } else {
                        format!("保存联系人失败: {e}")
                    }
                })?;
            Ok(self.conn.last_insert_rowid() as u32)
        } else {
            self.conn
                .execute(
                    "
                    UPDATE user_contacts
                    SET email = ?1, display_name = ?2, notes = ?3, updated_at = ?4
                    WHERE id = ?5
                    ",
                    params![email, display_name, notes, now, contact.id],
                )
                .map_err(|e| {
                    if format!("{e}").contains("UNIQUE") {
                        format!("已存在邮箱为 {email} 的联系人")
                    } else {
                        format!("更新联系人失败: {e}")
                    }
                })?;
            Ok(contact.id)
        }
    }

    /// Delete a user-maintained contact by row ID (F-5)。返回是否存在对应行。
    pub fn delete_user_contact(&mut self, contact_id: u32) -> Result<bool, String> {
        let affected = self
            .conn
            .execute(
                "DELETE FROM user_contacts WHERE id = ?1",
                params![contact_id],
            )
            .map_err(|e| format!("删除联系人失败: {e}"))?;
        Ok(affected > 0)
    }

    /// WP-06：读取所有自定义标签（按快捷键升序、名称升序）。
    pub fn list_tags(&self) -> Result<Vec<TagItem>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "
                SELECT id, name, color, shortcut
                FROM tags
                ORDER BY CASE WHEN shortcut IS NULL THEN 1 ELSE 0 END, shortcut ASC, name COLLATE NOCASE ASC
                ",
            )
            .map_err(|e| format!("查询标签失败: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                let id: i64 = row.get(0)?;
                let name: String = row.get(1)?;
                let color: String = row.get(2)?;
                let shortcut: Option<i64> = row.get(3)?;
                Ok(TagItem {
                    id: id.max(0) as u32,
                    name,
                    color,
                    shortcut: shortcut
                        .and_then(|s| u8::try_from(s).ok())
                        .filter(|s| (1..=9).contains(s)),
                })
            })
            .map_err(|e| format!("读取标签失败: {e}"))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.map_err(|e| format!("解析标签行失败: {e}"))?);
        }
        Ok(out)
    }

    /// WP-06：新增或更新自定义标签（快捷键 `1..=9` 若被其他标签占用会自动解绑）。
    pub fn upsert_tag(
        &mut self,
        id: Option<u32>,
        name: &str,
        color: &str,
        shortcut: Option<u8>,
    ) -> Result<TagItem, String> {
        let clean_name = name.trim();
        if clean_name.is_empty() {
            return Err("标签名称不能为空".to_string());
        }
        let clean_color = if color.trim().starts_with('#') && color.trim().len() == 7 {
            color.trim().to_ascii_lowercase()
        } else {
            "#3584e4".to_string()
        };
        let clean_shortcut = shortcut.filter(|s| (1..=9).contains(s));
        if let Some(sc) = clean_shortcut {
            self.conn
                .execute(
                    "UPDATE tags SET shortcut = NULL WHERE shortcut = ?1 AND (?2 IS NULL OR id != ?2)",
                    params![i64::from(sc), id.map(i64::from)],
                )
                .map_err(|e| format!("更新标签快捷键失败: {e}"))?;
        }
        let tag_id = match id {
            Some(existing_id) => {
                let affected = self
                    .conn
                    .execute(
                        "UPDATE tags SET name = ?1, color = ?2, shortcut = ?3 WHERE id = ?4",
                        params![
                            clean_name,
                            clean_color,
                            clean_shortcut.map(i64::from),
                            existing_id
                        ],
                    )
                    .map_err(|e| {
                        if format!("{e}").contains("UNIQUE") {
                            format!("已存在名为「{clean_name}」的标签")
                        } else {
                            format!("更新标签失败: {e}")
                        }
                    })?;
                if affected == 0 {
                    return Err("未找到要更新的标签".to_string());
                }
                existing_id
            }
            None => {
                self.conn
                    .execute(
                        "INSERT INTO tags (name, color, shortcut) VALUES (?1, ?2, ?3)",
                        params![clean_name, clean_color, clean_shortcut.map(i64::from)],
                    )
                    .map_err(|e| {
                        if format!("{e}").contains("UNIQUE") {
                            format!("已存在名为「{clean_name}」的标签")
                        } else {
                            format!("创建标签失败: {e}")
                        }
                    })?;
                self.conn.last_insert_rowid().max(0) as u32
            }
        };
        Ok(TagItem {
            id: tag_id,
            name: clean_name.to_string(),
            color: clean_color,
            shortcut: clean_shortcut,
        })
    }

    /// WP-06：按 ID 删除自定义标签。
    pub fn delete_tag(&mut self, tag_id: u32) -> Result<bool, String> {
        let affected = self
            .conn
            .execute("DELETE FROM tags WHERE id = ?1", params![tag_id])
            .map_err(|e| format!("删除标签失败: {e}"))?;
        Ok(affected > 0)
    }

    /// WP-06：立即在本地缓存邮件上运行已启用的过滤器规则（支持标记已读、标星、移动/归档到本地已缓存文件夹），
    /// 返回命中并更新的邮件数量。
    pub fn apply_filter_rules(
        &mut self,
        rules: &[crate::config::FilterRule],
        accounts: &[crate::config::AccountConfig],
    ) -> Result<usize, String> {
        use crate::config::{FilterField, FilterMatch};

        let enabled_rules: Vec<&crate::config::FilterRule> = rules
            .iter()
            .filter(|r| r.enabled && !r.value.trim().is_empty())
            .collect();
        if enabled_rules.is_empty() {
            return Ok(0);
        }

        let mut stmt = self
            .conn
            .prepare(
                "
                SELECT m.id, m.account_id, m.folder_id, m.sender, m.recipients, m.subject,
                       COALESCE(m.body_text, m.preview), m.unread, m.flagged
                FROM messages m
                ",
            )
            .map_err(|e| format!("读取缓存邮件失败: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)? as u32,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, i64>(7)? != 0,
                    row.get::<_, i64>(8)? != 0,
                ))
            })
            .map_err(|e| format!("遍历缓存邮件失败: {e}"))?;

        let mut messages = Vec::new();
        for r in rows {
            messages.push(r.map_err(|e| format!("解析邮件行失败: {e}"))?);
        }
        drop(stmt);

        let mut updated = 0usize;
        for (msg_id, account_id, folder_id, sender, recipients, subject, body, unread, flagged) in
            messages
        {
            let acc_email = accounts
                .iter()
                .find(|a| a.id == account_id)
                .map(|a| a.email.as_str())
                .unwrap_or("");

            for rule in &enabled_rules {
                if !rule.account_email.trim().is_empty()
                    && !rule.account_email.trim().eq_ignore_ascii_case(acc_email)
                {
                    continue;
                }
                let haystack = match rule.field {
                    FilterField::From => sender.as_str(),
                    FilterField::To => recipients.as_str(),
                    FilterField::Subject => subject.as_str(),
                    FilterField::Body => body.as_str(),
                }
                .to_lowercase();
                let needle = rule.value.trim().to_lowercase();
                let matched = match rule.matcher {
                    FilterMatch::Contains => haystack.contains(&needle),
                    FilterMatch::Equals => haystack.trim() == needle,
                    FilterMatch::StartsWith => haystack.trim_start().starts_with(&needle),
                    FilterMatch::EndsWith => haystack.trim_end().ends_with(&needle),
                };
                if !matched {
                    continue;
                }

                let mut changed = false;
                if rule.mark_read && unread {
                    self.conn
                        .execute(
                            "UPDATE messages SET unread = 0 WHERE id = ?1",
                            params![msg_id],
                        )
                        .map_err(|e| format!("过滤器标记已读失败: {e}"))?;
                    changed = true;
                }
                if rule.star && !flagged {
                    self.conn
                        .execute(
                            "UPDATE messages SET flagged = 1 WHERE id = ?1",
                            params![msg_id],
                        )
                        .map_err(|e| format!("过滤器标星失败: {e}"))?;
                    changed = true;
                }
                let target_folder = if !rule.move_to_folder.trim().is_empty() {
                    Some(rule.move_to_folder.trim())
                } else if rule.archive {
                    Some("Archive")
                } else {
                    None
                };
                if let Some(target) = target_folder {
                    if let Some(dest_id) = self
                        .conn
                        .query_row(
                            "SELECT id FROM folders WHERE account_id = ?1 AND (LOWER(path) = LOWER(?2) OR LOWER(name) = LOWER(?2)) LIMIT 1",
                            params![account_id, target],
                            |r| r.get::<_, i64>(0),
                        )
                        .optional()
                        .map_err(|e| format!("查询目标文件夹失败: {e}"))?
                    {
                        if dest_id != folder_id {
                            let _ = self.conn.execute(
                                "UPDATE messages SET folder_id = ?1 WHERE id = ?2",
                                params![dest_id, msg_id],
                            );
                            changed = true;
                        }
                    }
                }
                if changed {
                    updated += 1;
                }
            }
        }
        Ok(updated)
    }

    /// Bulk-import user contacts (F-5)：按邮箱去重（`INSERT OR IGNORE`），
    /// 返回 `(新增, 跳过)` 计数。
    pub fn import_user_contacts(
        &mut self,
        contacts: &[UserContact],
    ) -> Result<(usize, usize), String> {
        let now = chrono::Utc::now().timestamp();
        let mut added = 0usize;
        let mut skipped = 0usize;
        for contact in contacts {
            let email = contact.email.trim().to_string();
            if !email.contains('@') {
                skipped += 1;
                continue;
            }
            let affected = self
                .conn
                .execute(
                    "
                    INSERT OR IGNORE INTO user_contacts
                        (email, display_name, notes, created_at, updated_at)
                    VALUES (?1, ?2, ?3, ?4, ?4)
                    ",
                    params![email, contact.display_name.trim(), contact.notes, now],
                )
                .map_err(|e| format!("导入联系人失败: {e}"))?;
            if affected > 0 {
                added += 1;
            } else {
                skipped += 1;
            }
        }
        Ok((added, skipped))
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

    /// Find the account's Junk/Spam folder `(folder_id, path)` by friendly name
    /// or raw path (`Junk`, `Spam`, `垃圾邮件`, etc.).
    pub fn find_spam_folder(&self, account_id: u32) -> Result<Option<(u32, String)>, String> {
        self.find_role_folder(account_id, SPAM_FOLDER_MATCH)
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

    /// Read raw attachment bytes (with filename and MIME type) from local cache.
    pub fn get_attachment_bytes(
        &self,
        attachment_id: u32,
    ) -> Result<Option<(String, String, Vec<u8>)>, String> {
        self.conn
            .query_row(
                "SELECT filename, mime_type, content FROM attachments WHERE id = ?1",
                params![attachment_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| format!("读取附件内容失败: {e}"))
    }

    /// Look up `(folder_path, uid)` for a cached message ID.
    pub fn get_message_location(&self, message_id: u32) -> Result<Option<(String, u32)>, String> {
        self.conn
            .query_row(
                "SELECT f.path, m.uid FROM messages m JOIN folders f ON f.id = m.folder_id WHERE m.id = ?1",
                params![message_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?)),
            )
            .optional()
            .map_err(|e| format!("查询邮件位置失败: {e}"))
    }

    /// Remove a stripped attachment from local cache, optionally update the
    /// message's IMAP UID to the appended copy, refresh `has_attachment`, and
    /// return the remaining attachments for `message_id`.
    pub fn delete_attachment_and_update_uid(
        &self,
        message_id: u32,
        attachment_id: u32,
        new_uid: Option<u32>,
    ) -> Result<Vec<AttachmentMeta>, String> {
        self.conn
            .execute(
                "DELETE FROM attachments WHERE id = ?1 AND message_id = ?2",
                params![attachment_id, message_id],
            )
            .map_err(|e| format!("删除本地附件缓存失败: {e}"))?;
        let remaining = self.list_attachments(message_id)?;
        let has_att = if remaining.is_empty() { 0i64 } else { 1i64 };
        if let Some(uid) = new_uid {
            self.conn
                .execute(
                    "UPDATE messages SET has_attachment = ?1, uid = ?2 WHERE id = ?3",
                    params![has_att, uid, message_id],
                )
                .map_err(|e| format!("更新邮件附件状态失败: {e}"))?;
        } else {
            self.conn
                .execute(
                    "UPDATE messages SET has_attachment = ?1 WHERE id = ?2",
                    params![has_att, message_id],
                )
                .map_err(|e| format!("更新邮件附件状态失败: {e}"))?;
        }
        Ok(remaining)
    }

    /// Export all cached attachments of `message_id` into `dest_dir` (used when
    /// forwarding a message with its original attachments). Returns exported file paths.
    pub fn export_message_attachments(
        &self,
        message_id: u32,
        dest_dir: &Path,
    ) -> Result<Vec<std::path::PathBuf>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT filename, content FROM attachments WHERE message_id = ?1 ORDER BY id ASC",
            )
            .map_err(|e| format!("查询邮件附件失败: {e}"))?;
        let rows = stmt
            .query_map(params![message_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
            })
            .map_err(|e| format!("读取邮件附件失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析邮件附件失败: {e}"))?;

        if rows.is_empty() {
            return Ok(Vec::new());
        }
        std::fs::create_dir_all(dest_dir)
            .map_err(|e| format!("创建转发附件临时目录失败 ({}): {e}", dest_dir.display()))?;

        let mut used_names: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut exported = Vec::with_capacity(rows.len());
        for (raw_name, bytes) in rows {
            let safe = sanitize_attachment_filename(&raw_name);
            let unique = deduplicate_filename(&safe, &mut used_names);
            let path = dest_dir.join(unique);
            std::fs::write(&path, bytes)
                .map_err(|e| format!("写入转发附件失败 ({}): {e}", path.display()))?;
            exported.push(path);
        }
        Ok(exported)
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

    /// Batch-mark every cached message of `folder_id` as read and zero the
    /// folder's unread counter (right-click "Mark as Read" on a folder).
    pub fn set_folder_all_seen(&mut self, folder_id: u32) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE messages SET unread = 0 WHERE folder_id = ?1",
                params![folder_id],
            )
            .map_err(|e| format!("批量更新已读状态失败: {e}"))?;
        self.conn
            .execute(
                "
                UPDATE folders
                SET unread_count = (SELECT COUNT(*) FROM messages WHERE folder_id = ?1 AND unread = 1)
                WHERE id = ?1
                ",
                params![folder_id],
            )
            .map_err(|e| format!("更新文件夹未读数失败: {e}"))?;
        Ok(())
    }

    /// Remove every cached message of `folder_id` (attachments cascade) and
    /// zero its counters (right-click "Empty Folder" after server expunge).
    pub fn clear_folder(&mut self, folder_id: u32) -> Result<(), String> {
        self.conn
            .execute(
                "DELETE FROM messages WHERE folder_id = ?1",
                params![folder_id],
            )
            .map_err(|e| format!("清空文件夹缓存失败: {e}"))?;
        self.conn
            .execute(
                "
                UPDATE folders
                SET unread_count = (SELECT COUNT(*) FROM messages WHERE folder_id = ?1 AND unread = 1),
                    total_count = (SELECT COUNT(*) FROM messages WHERE folder_id = ?1)
                WHERE id = ?1
                ",
                params![folder_id],
            )
            .map_err(|e| format!("更新文件夹计数失败: {e}"))?;
        Ok(())
    }

    /// `(message_id, remote_id)` of every cached unread message in
    /// `folder_id` — drives the per-message Graph path of MarkFolderAllRead.
    pub fn folder_unread_remote_ids(&self, folder_id: u32) -> Result<Vec<(u32, String)>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "
                SELECT id, remote_id
                FROM messages
                WHERE folder_id = ?1 AND unread = 1 AND remote_id != ''
                ORDER BY id ASC
                ",
            )
            .map_err(|e| format!("查询文件夹未读邮件失败: {e}"))?;
        let rows = stmt
            .query_map(params![folder_id], |row| {
                Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("读取文件夹未读邮件失败: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析文件夹未读邮件失败: {e}"))
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
    /// `send_at` 为到期时间戳（`None` = 立即到期）；撤销发送缓冲在倒计时结束前不投递。
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
        send_at: Option<i64>,
    ) -> Result<u32, String> {
        self.enqueue_outbox_from_draft(
            account_id,
            to_addr,
            cc_addr,
            bcc_addr,
            subject,
            body_text,
            in_reply_to,
            attachment_paths,
            created_at,
            send_at,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn enqueue_outbox_from_draft(
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
        send_at: Option<i64>,
        local_draft_id: Option<u32>,
    ) -> Result<u32, String> {
        let paths_json = serde_json::to_string(attachment_paths)
            .map_err(|e| format!("序列化附件路径失败: {e}"))?;
        let tx = self
            .conn
            .transaction()
            .map_err(|e| format!("启动发件事务失败: {e}"))?;
        tx.execute(
            "
                INSERT INTO outbox (
                    account_id, to_addr, cc_addr, bcc_addr, subject, body_text, in_reply_to,
                    attachment_paths_json, status, last_error, created_at, send_at
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'queued', '', ?9, ?10)
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
                created_at,
                send_at
            ],
        )
        .map_err(|e| format!("写入发件箱持久化记录失败: {e}"))?;
        let id = tx.last_insert_rowid() as u32;
        if let Some(draft_id) = local_draft_id {
            tx.execute(
                "DELETE FROM drafts WHERE id = ?1 AND account_id = ?2",
                params![draft_id, account_id],
            )
            .map_err(|e| format!("交接草稿失败: {e}"))?;
        }
        tx.commit().map_err(|e| format!("提交发件事务失败: {e}"))?;
        Ok(id)
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

    /// 原子领取一条自动投递记录：仅 Queued 且已到期（`send_at` 为空或不晚于 `now`）
    /// 的记录会被置为 Sending 并返回，其余（未到期 / 已被其他路径领取 / 状态不符）
    /// 返回 `None`。单条 UPDATE + 条件过滤保证同一记录不会被重复领取。
    pub fn claim_outbox_for_delivery(
        &mut self,
        outbox_id: u32,
        now: i64,
    ) -> Result<Option<OutboxItem>, String> {
        let claimed = self
            .conn
            .execute(
                "
                UPDATE outbox SET status = 'sending', last_error = ''
                WHERE id = ?1 AND status = 'queued'
                  AND (send_at IS NULL OR send_at <= ?2)
                ",
                params![outbox_id, now],
            )
            .map_err(|e| format!("领取发件箱记录失败: {e}"))?;
        if claimed == 0 {
            return Ok(None);
        }
        self.load_outbox_row(outbox_id)
    }

    /// 原子领取一条用户显式重试记录：Queued / Failed 均可（绕过到期检查，
    /// 对应「手动重发」与撤销倒计时到期的立即投递）。
    pub fn claim_outbox_for_retry(&mut self, outbox_id: u32) -> Result<Option<OutboxItem>, String> {
        let claimed = self
            .conn
            .execute(
                "
                UPDATE outbox SET status = 'sending', last_error = ''
                WHERE id = ?1 AND status IN ('queued', 'failed')
                ",
                params![outbox_id],
            )
            .map_err(|e| format!("领取发件箱记录失败: {e}"))?;
        if claimed == 0 {
            return Ok(None);
        }
        self.load_outbox_row(outbox_id)
    }

    /// Read back one Outbox row already marked `sending` by a claim above.
    fn load_outbox_row(&self, outbox_id: u32) -> Result<Option<OutboxItem>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "
                SELECT id, account_id, to_addr, cc_addr, bcc_addr, subject, body_text,
                       in_reply_to, attachment_paths_json, status, last_error
                FROM outbox
                WHERE id = ?1
                ",
            )
            .map_err(|e| format!("查询发件箱失败: {e}"))?;
        let rows = stmt
            .query_map(params![outbox_id], |row| {
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
        Ok(rows.into_iter().next())
    }

    /// 列出到期（`send_at` 为空或不晚于 `now`）的 Queued 记录，供启动恢复与
    /// 重新启用账号后的自动续发领取。`account_id` 为 `None` 时扫描全部账号。
    /// Failed / Uncertain / Sending 记录不参与自动续发。
    pub fn list_due_queued_ids(
        &self,
        account_id: Option<u32>,
        now: i64,
    ) -> Result<Vec<(u32, u32)>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "
                SELECT account_id, id
                FROM outbox
                WHERE status = 'queued'
                  AND (send_at IS NULL OR send_at <= ?1)
                  AND (?2 IS NULL OR account_id = ?2)
                ORDER BY id ASC
                ",
            )
            .map_err(|e| format!("查询到期发件箱失败: {e}"))?;
        let rows = stmt
            .query_map(params![now, account_id], |row| {
                Ok((row.get::<_, u32>(0)?, row.get::<_, u32>(1)?))
            })
            .map_err(|e| format!("读取到期发件箱失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析到期发件箱行失败: {e}"))?;
        Ok(rows)
    }

    pub fn expire_queued_outbox(&mut self) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE outbox SET send_at = NULL WHERE status = 'queued' AND send_at IS NOT NULL",
                [],
            )
            .map_err(|e| format!("准备退出发件队列失败: {e}"))?;
        Ok(())
    }

    /// 在同一事务中将仍排队的邮件转回草稿，避免撤销和投递同时成功。
    pub fn undo_queued_outbox(
        &mut self,
        account_id: u32,
        outbox_id: u32,
    ) -> Result<Option<u32>, String> {
        let tx = self
            .conn
            .transaction()
            .map_err(|e| format!("撤销发送事务失败: {e}"))?;
        let inserted = tx.execute(
            "INSERT INTO drafts (account_id, to_addr, cc_addr, bcc_addr, subject, body_text, in_reply_to, attachment_paths_json, updated_at)
             SELECT account_id, to_addr, cc_addr, bcc_addr, subject, body_text, in_reply_to, attachment_paths_json, ?3
             FROM outbox WHERE id = ?1 AND account_id = ?2 AND status = 'queued'",
            params![outbox_id, account_id, chrono::Utc::now().timestamp()],
        ).map_err(|e| format!("恢复草稿失败: {e}"))?;
        if inserted == 0 {
            return Ok(None);
        }
        let draft_id = tx.last_insert_rowid() as u32;
        tx.execute(
            "DELETE FROM outbox WHERE id = ?1 AND account_id = ?2 AND status = 'queued'",
            params![outbox_id, account_id],
        )
        .map_err(|e| format!("撤销发件箱记录失败: {e}"))?;
        tx.commit().map_err(|e| format!("提交撤销发送失败: {e}"))?;
        Ok(Some(draft_id))
    }

    /// 撤销发送：仅删除仍在排队（Queued）的记录；已被领取投递的记录返回 `false` 不可撤销。
    pub fn delete_queued_outbox(&mut self, outbox_id: u32) -> Result<bool, String> {
        let deleted = self
            .conn
            .execute(
                "DELETE FROM outbox WHERE id = ?1 AND status = 'queued'",
                params![outbox_id],
            )
            .map_err(|e| format!("删除发件箱记录失败: {e}"))?;
        Ok(deleted > 0)
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

    /// WP-08: Return lightweight [`MessageSummary`] items for a conversation thread
    /// (excluding `head_message_id`), ordered oldest-first for inline tree expansion.
    pub fn list_thread_summaries(
        &self,
        account_id: u32,
        head_message_id: u32,
        subject: &str,
    ) -> Result<Vec<MessageSummary>, String> {
        let thread = self.list_thread_messages(account_id, head_message_id, subject, false)?;
        let ids: Vec<u32> = thread
            .into_iter()
            .filter(|m| m.id != head_message_id)
            .map(|m| m.id)
            .collect();
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = vec!["?"; ids.len()].join(", ");
        let args: Vec<rusqlite::types::Value> =
            ids.iter().map(|&id| i64::from(id).into()).collect();
        let mut stmt = self
            .conn
            .prepare(&format!(
                "
                SELECT m.id, m.account_id, m.folder_id, f.path, m.uid, m.sender, m.subject,
                       m.preview, m.date_ts, m.date_display, m.unread, m.flagged, m.has_attachment,
                       m.recipients, m.importance, m.tags, m.answered, m.forwarded, m.snoozed_until_ts
                FROM messages m
                JOIN folders f ON f.id = m.folder_id
                WHERE m.id IN ({placeholders})
                ORDER BY m.date_ts ASC, m.uid ASC
                "
            ))
            .map_err(|e| format!("准备会话子邮件查询失败: {e}"))?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(args.iter()), map_summary_row)
            .map_err(|e| format!("执行会话子邮件查询失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析会话子邮件失败: {e}"))?;
        Ok(rows)
    }

    /// WP-08: Persist comma-separated tags for `message_id`.
    pub fn set_message_tags(&mut self, message_id: u32, tags: &[String]) -> Result<(), String> {
        let joined = tags
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(",");
        self.conn
            .execute(
                "UPDATE messages SET tags = ?1 WHERE id = ?2",
                params![joined, message_id],
            )
            .map_err(|e| format!("更新邮件标签失败: {e}"))?;
        Ok(())
    }

    /// WP-08: Persist snooze timestamp for `message_id` (`0` = clear).
    pub fn snooze_message(&mut self, message_id: u32, until_ts: i64) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE messages SET snoozed_until_ts = ?1 WHERE id = ?2",
                params![until_ts, message_id],
            )
            .map_err(|e| format!("更新稍后提醒失败: {e}"))?;
        Ok(())
    }

    /// WP-08: Copy a cached message into `target_account_id` / `target_folder_path`.
    pub fn copy_message_to_folder(
        &mut self,
        message_id: u32,
        target_account_id: u32,
        target_folder_path: &str,
    ) -> Result<(), String> {
        let target_folder_id: u32 = self
            .conn
            .query_row(
                "SELECT id FROM folders WHERE account_id = ?1 AND path = ?2",
                params![target_account_id, target_folder_path],
                |row| row.get(0),
            )
            .map_err(|e| format!("未找到目标文件夹 {target_folder_path}: {e}"))?;
        let next_uid: u32 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(uid), 0) + 1 FROM messages WHERE folder_id = ?1",
                params![target_folder_id],
                |row| row.get(0),
            )
            .unwrap_or(1);
        self.conn
            .execute(
                "
                INSERT INTO messages (
                    account_id, folder_id, uid, remote_id, message_id, in_reply_to, references_header,
                    sender, recipients, subject, preview, date_ts, date_display,
                    unread, flagged, has_attachment, body_text, body_html, index_text,
                    importance, tags, answered, forwarded, snoozed_until_ts
                )
                SELECT
                    ?1, ?2, ?3, '', message_id, in_reply_to, references_header,
                    sender, recipients, subject, preview, date_ts, date_display,
                    unread, flagged, has_attachment, body_text, body_html, index_text,
                    importance, tags, answered, forwarded, snoozed_until_ts
                FROM messages
                WHERE id = ?4
                ",
                params![target_account_id, target_folder_id, next_uid, message_id],
            )
            .map_err(|e| format!("复制邮件到本地缓存失败: {e}"))?;
        Ok(())
    }
}

fn map_summary_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<MessageSummary> {
    let tags_raw: String = row.get(15)?;
    let tags = tags_raw
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    Ok(MessageSummary {
        id: row.get(0)?,
        account_id: row.get(1)?,
        folder_id: row.get(2)?,
        folder_path: row.get(3)?,
        uid: row.get(4)?,
        sender: row.get(5)?,
        subject: row.get(6)?,
        preview: row.get(7)?,
        date_ts: row.get(8)?,
        date_display: row.get(9)?,
        unread: row.get::<_, i32>(10)? != 0,
        flagged: row.get::<_, i32>(11)? != 0,
        has_attachment: row.get::<_, i32>(12)? != 0,
        recipients: row.get(13)?,
        // 会话计数由 `fill_thread_counts` 在查询后统一回填。
        thread_count: 1,
        importance: row.get::<_, i32>(14)?.clamp(-1, 1) as i8,
        tags,
        answered: row.get::<_, i32>(16)? != 0,
        forwarded: row.get::<_, i32>(17)? != 0,
        snoozed_until_ts: row.get(18)?,
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
        date_ts: row.get(9)?,
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
            in_reply_to: String::new(),
            references_header: String::new(),
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
            in_reply_to: String::new(),
            references_header: String::new(),
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
                None,
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
                None,
            )
            .unwrap();

        let items = cache.list_outbox(1).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].to_addr, "to@example.com");
        assert_eq!(items[0].cc_addr, "c1@example.com, Carol <carol@c.com>");
        assert_eq!(items[0].bcc_addr, "secret@example.com");
    }

    #[test]
    fn outbox_claim_requires_due_and_is_idempotent() {
        // 任务一：启用续发 / 启动恢复只领取「已到期的 Queued」记录，且同一条
        // 记录只能被领取一次（Queued -> Sending 原子转换，防重复投递）。
        let mut cache = CacheStore::open_in_memory().expect("open memory db");
        let now = 1_700_000_000i64;
        let due_id = cache
            .enqueue_outbox(
                1,
                "a@x.com",
                "",
                "",
                "已到期",
                "正文",
                "",
                &[],
                now - 60,
                Some(now - 30),
            )
            .unwrap();
        let not_due_id = cache
            .enqueue_outbox(
                1,
                "b@x.com",
                "",
                "",
                "未到期",
                "正文",
                "",
                &[],
                now - 10,
                Some(now + 300),
            )
            .unwrap();

        // 到期扫描只含已到期的 Queued 记录（撤销倒计时中的未到期记录不参与）。
        assert_eq!(
            cache.list_due_queued_ids(Some(1), now).unwrap(),
            vec![(1, due_id)]
        );

        // 未到期的 Queued 不可自动领取（撤销倒计时期间不会被提前投递）。
        assert!(cache
            .claim_outbox_for_delivery(not_due_id, now)
            .unwrap()
            .is_none());
        // 已到期的 Queued 可领取一次。
        let claimed = cache
            .claim_outbox_for_delivery(due_id, now)
            .unwrap()
            .unwrap();
        assert_eq!(claimed.id, due_id);
        assert_eq!(claimed.status, OutboxStatus::Sending);
        // 再次领取同一条返回 None（幂等：反复启停账号不会重复投递）。
        assert!(cache
            .claim_outbox_for_delivery(due_id, now)
            .unwrap()
            .is_none());
        // 已进入 Sending 的记录对显式重试也不再可领取。
        assert!(cache.claim_outbox_for_retry(due_id).unwrap().is_none());

        // 领取后到期扫描为空；未到期记录仍在排队但不出现在到期列表。
        assert!(cache.list_due_queued_ids(Some(1), now).unwrap().is_empty());
        assert!(cache.list_due_queued_ids(None, now).unwrap().is_empty());
    }

    #[test]
    fn outbox_disabled_account_stays_queued_and_resumes() {
        // 任务一：停用账号的待发记录保持 Queued（不标 Failed），保留发件身份
        // 与正文；恢复 Queued 后可再次领取（重新启用账号后续发）。
        let mut cache = CacheStore::open_in_memory().expect("open memory db");
        let now = 1_700_000_000i64;
        let id = cache
            .enqueue_outbox(
                1,
                "a@x.com",
                "c@x.com",
                "",
                "停用期间排队",
                "正文",
                "<r@x.com>",
                &[],
                now,
                Some(now - 5),
            )
            .unwrap();

        // 投递前发现账号停用：回滚为 Queued 并记录原因，正文与收件人保持不变。
        let claimed = cache.claim_outbox_for_delivery(id, now).unwrap().unwrap();
        cache
            .update_outbox_status(id, OutboxStatus::Queued, "账号已停用，暂缓投递")
            .unwrap();

        let items = cache.list_outbox(1).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].status, OutboxStatus::Queued);
        assert_eq!(items[0].to_addr, claimed.to_addr);
        assert_eq!(items[0].body_text, claimed.body_text);
        assert_eq!(items[0].subject, claimed.subject);
        assert_eq!(items[0].last_error, "账号已停用，暂缓投递");

        // 恢复 Queued 后重新启用账号：可再次领取继续投递。
        assert!(cache.claim_outbox_for_delivery(id, now).unwrap().is_some());
        assert!(cache.list_due_queued_ids(Some(1), now).unwrap().is_empty());
    }

    #[test]
    fn outbox_auto_delivery_never_claims_failed_or_uncertain() {
        // 任务一：Failed / Uncertain 不参与自动续发（Uncertain never auto-resend），
        // 仅用户显式重试可领取 Failed 记录。
        let mut cache = CacheStore::open_in_memory().expect("open memory db");
        let now = 1_700_000_000i64;
        let failed_id = cache
            .enqueue_outbox(1, "a@x.com", "", "", "失败", "正文", "", &[], now, None)
            .unwrap();
        let uncertain_id = cache
            .enqueue_outbox(1, "b@x.com", "", "", "不确定", "正文", "", &[], now, None)
            .unwrap();
        cache
            .update_outbox_status(failed_id, OutboxStatus::Failed, "SMTP 拒绝")
            .unwrap();
        cache
            .update_outbox_status(uncertain_id, OutboxStatus::Uncertain, "连接中断")
            .unwrap();

        assert!(cache.list_due_queued_ids(Some(1), now).unwrap().is_empty());
        assert!(cache
            .claim_outbox_for_delivery(failed_id, now)
            .unwrap()
            .is_none());
        assert!(cache
            .claim_outbox_for_delivery(uncertain_id, now)
            .unwrap()
            .is_none());
        // 显式重试仍可领取 Failed（Queued / Uncertain 之外的用户触发路径）。
        assert!(cache.claim_outbox_for_retry(failed_id).unwrap().is_some());
        // Uncertain 连显式重试也不领取，只能人工确认后删除或重建。
        assert!(cache
            .claim_outbox_for_retry(uncertain_id)
            .unwrap()
            .is_none());
    }

    #[test]
    fn enqueue_and_draft_handoff_are_atomic() {
        let mut cache = CacheStore::open_in_memory().unwrap();
        let draft = LocalDraft {
            id: 0,
            account_id: 1,
            to_addr: "to".into(),
            cc_addr: "cc".into(),
            bcc_addr: "bcc".into(),
            subject: "subject".into(),
            body_text: "body".into(),
            in_reply_to: "ref".into(),
            attachment_paths: vec!["file".into()],
            updated_at: 10,
        };
        let id = cache.upsert_draft(&draft).unwrap();
        cache.conn.execute_batch("CREATE TRIGGER reject_handoff BEFORE DELETE ON drafts BEGIN SELECT RAISE(ABORT, 'write failed'); END;").unwrap();
        assert!(cache
            .enqueue_outbox_from_draft(
                1,
                "to",
                "cc",
                "bcc",
                "subject",
                "body",
                "ref",
                &["file".into()],
                10,
                Some(20),
                Some(id)
            )
            .is_err());
        assert_eq!(cache.list_drafts().unwrap().len(), 1);
        assert!(cache.list_outbox(1).unwrap().is_empty());
        cache
            .conn
            .execute_batch("DROP TRIGGER reject_handoff")
            .unwrap();
        cache
            .enqueue_outbox_from_draft(
                1,
                "to",
                "cc",
                "bcc",
                "subject",
                "body",
                "ref",
                &["file".into()],
                10,
                Some(20),
                Some(id),
            )
            .unwrap();
        assert!(cache.list_drafts().unwrap().is_empty());
        assert_eq!(cache.list_outbox(1).unwrap().len(), 1);
    }

    #[test]
    fn undo_outbox_is_atomic_and_preserves_all_draft_fields() {
        let mut cache = CacheStore::open_in_memory().unwrap();
        let id = cache
            .enqueue_outbox(
                1,
                "to",
                "cc",
                "bcc",
                "subject",
                "body",
                "ref",
                &["file".into()],
                10,
                Some(100),
            )
            .unwrap();
        assert_eq!(cache.undo_queued_outbox(2, id).unwrap(), None);
        let draft_id = cache.undo_queued_outbox(1, id).unwrap().unwrap();
        let drafts = cache.list_drafts().unwrap();
        let d = drafts.iter().find(|d| d.id == draft_id).unwrap();
        assert_eq!(
            (
                &*d.to_addr,
                &*d.cc_addr,
                &*d.bcc_addr,
                &*d.subject,
                &*d.body_text,
                &*d.in_reply_to
            ),
            ("to", "cc", "bcc", "subject", "body", "ref")
        );
        assert_eq!(d.attachment_paths, vec!["file"]);
        assert!(cache.list_outbox(1).unwrap().is_empty());
        assert!(cache.claim_outbox_for_retry(id).unwrap().is_none());
        assert_eq!(cache.undo_queued_outbox(1, id).unwrap(), None);

        let claimed = cache
            .enqueue_outbox(1, "to", "", "", "", "", "", &[], 10, None)
            .unwrap();
        assert!(cache.claim_outbox_for_retry(claimed).unwrap().is_some());
        assert_eq!(cache.undo_queued_outbox(1, claimed).unwrap(), None);
        assert_eq!(cache.list_drafts().unwrap().len(), 1);
    }

    #[test]
    fn undo_rolls_back_when_draft_write_fails() {
        let mut cache = CacheStore::open_in_memory().unwrap();
        let id = cache
            .enqueue_outbox(1, "to", "", "", "subject", "body", "", &[], 10, Some(100))
            .unwrap();
        cache.conn.execute_batch("CREATE TRIGGER reject_draft BEFORE INSERT ON drafts BEGIN SELECT RAISE(ABORT, 'write failed'); END;").unwrap();
        assert!(cache.undo_queued_outbox(1, id).is_err());
        assert_eq!(cache.list_outbox(1).unwrap().len(), 1);
        assert!(cache.list_drafts().unwrap().is_empty());
    }

    #[test]
    fn quit_expiry_only_changes_queued_records() {
        let mut cache = CacheStore::open_in_memory().unwrap();
        let q = cache
            .enqueue_outbox(1, "to", "", "", "", "", "", &[], 10, Some(100))
            .unwrap();
        let f = cache
            .enqueue_outbox(1, "to", "", "", "", "", "", &[], 10, Some(100))
            .unwrap();
        cache
            .update_outbox_status(f, OutboxStatus::Failed, "error")
            .unwrap();
        cache.expire_queued_outbox().unwrap();
        assert_eq!(cache.list_due_queued_ids(None, 10).unwrap(), vec![(1, q)]);
        assert_eq!(
            cache
                .list_outbox(1)
                .unwrap()
                .iter()
                .find(|i| i.id == f)
                .unwrap()
                .last_error,
            "error"
        );
    }

    #[test]
    fn outbox_undo_deletes_only_queued_records() {
        // 任务一：撤销发送 = 删除仍在排队的记录；已进入投递的不可撤销。
        let mut cache = CacheStore::open_in_memory().expect("open memory db");
        let now = 1_700_000_000i64;
        let queued_id = cache
            .enqueue_outbox(
                1,
                "a@x.com",
                "",
                "",
                "撤销目标",
                "正文",
                "",
                &[],
                now,
                Some(now + 60),
            )
            .unwrap();
        let sending_id = cache
            .enqueue_outbox(1, "b@x.com", "", "", "投递中", "正文", "", &[], now, None)
            .unwrap();
        cache
            .update_outbox_status(sending_id, OutboxStatus::Sending, "")
            .unwrap();

        assert!(cache.delete_queued_outbox(queued_id).unwrap());
        assert!(cache
            .list_outbox(1)
            .unwrap()
            .iter()
            .all(|i| i.id != queued_id));
        assert!(!cache.delete_queued_outbox(sending_id).unwrap());
        assert!(cache
            .list_outbox(1)
            .unwrap()
            .iter()
            .any(|i| i.id == sending_id));
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
                None,
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
                        in_reply_to: String::new(),
                        references_header: String::new(),
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
                        in_reply_to: String::new(),
                        references_header: String::new(),
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
                        in_reply_to: String::new(),
                        references_header: String::new(),
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
                        in_reply_to: String::new(),
                        references_header: String::new(),
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
                        in_reply_to: String::new(),
                        references_header: String::new(),
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

        let page = cache.list_messages_page(inbox, "", 0, 10).unwrap();
        let latest_id = page.items[0].id;

        // 默认降序（newest_first = true）：最新回复在最前。
        let desc = cache
            .list_thread_messages(1, latest_id, "Re: Q3 Roadmap", true)
            .unwrap();
        assert_eq!(desc.len(), 2);
        assert_eq!(desc[0].sender, "Bob <b@test>");
        assert_eq!(desc[1].sender, "Alice <a@test>");

        // 升序（newest_first = false）：最早邮件在最前。
        let asc = cache
            .list_thread_messages(1, latest_id, "Re: Q3 Roadmap", false)
            .unwrap();
        assert_eq!(asc.len(), 2);
        assert_eq!(asc[0].sender, "Alice <a@test>");
        assert_eq!(asc[1].sender, "Bob <b@test>");
    }

    #[test]
    fn rfc_threading_links_different_subjects_and_dedupes_cross_folder_copies() {
        let mut cache = CacheStore::open_in_memory().unwrap();
        let folders = cache
            .sync_folders(
                1,
                &[
                    folder("收件箱", "INBOX"),
                    folder("已发送", "Sent"),
                    folder("已删除", "Trash"),
                ],
            )
            .unwrap();
        let inbox = folders.iter().find(|f| f.path == "INBOX").unwrap().id;
        let sent = folders.iter().find(|f| f.path == "Sent").unwrap().id;
        let trash = folders.iter().find(|f| f.path == "Trash").unwrap().id;

        // A 在 INBOX，B 在 Sent（改了主题但通过 In-Reply-To 关联 A），C 在 INBOX（通过 References 关联 A/B）。
        cache
            .commit_folder_headers(
                1,
                inbox,
                1,
                &[
                    CachedHeaderInput {
                        uid: 10,
                        message_id: "<root@example.com>".to_string(),
                        in_reply_to: String::new(),
                        references_header: String::new(),
                        sender: "Alice <alice@example.com>".to_string(),
                        recipients: "me@example.com".to_string(),
                        subject: "原始需求讨论".to_string(),
                        preview: "第一封".to_string(),
                        date_ts: 1000,
                        date_display: "09:00".to_string(),
                        unread: false,
                        flagged: false,
                        has_attachment: false,
                    },
                    CachedHeaderInput {
                        uid: 12,
                        message_id: "<reply2@example.com>".to_string(),
                        in_reply_to: "<reply1@example.com>".to_string(),
                        references_header: "<root@example.com> <reply1@example.com>".to_string(),
                        sender: "Alice <alice@example.com>".to_string(),
                        recipients: "me@example.com".to_string(),
                        subject: "全新主题名（依然同线程）".to_string(),
                        preview: "第三封".to_string(),
                        date_ts: 3000,
                        date_display: "11:00".to_string(),
                        unread: true,
                        flagged: false,
                        has_attachment: false,
                    },
                ],
                None,
            )
            .unwrap();

        cache
            .commit_folder_headers(
                1,
                sent,
                2,
                &[CachedHeaderInput {
                    uid: 20,
                    message_id: "<reply1@example.com>".to_string(),
                    in_reply_to: "<root@example.com>".to_string(),
                    references_header: "<root@example.com>".to_string(),
                    sender: "Me <me@example.com>".to_string(),
                    recipients: "alice@example.com".to_string(),
                    subject: "改过标题的回复".to_string(),
                    preview: "第二封".to_string(),
                    date_ts: 2000,
                    date_display: "10:00".to_string(),
                    unread: false,
                    flagged: false,
                    has_attachment: false,
                }],
                None,
            )
            .unwrap();

        // 在 Trash 中放一份与 INBOX root 相同的重复副本（同 message_id + sender + date_ts）。
        cache
            .commit_folder_headers(
                1,
                trash,
                3,
                &[CachedHeaderInput {
                    uid: 99,
                    message_id: "<root@example.com>".to_string(),
                    in_reply_to: String::new(),
                    references_header: String::new(),
                    sender: "Alice <alice@example.com>".to_string(),
                    recipients: "me@example.com".to_string(),
                    subject: "原始需求讨论".to_string(),
                    preview: "回收站副本".to_string(),
                    date_ts: 1000,
                    date_display: "09:00".to_string(),
                    unread: false,
                    flagged: false,
                    has_attachment: false,
                }],
                None,
            )
            .unwrap();

        let inbox_page = cache.list_messages_page(inbox, "", 0, 10).unwrap();
        let reply2_id = inbox_page.items.iter().find(|m| m.uid == 12).unwrap().id;
        let root_inbox_id = inbox_page.items.iter().find(|m| m.uid == 10).unwrap().id;

        // 降序：reply2 (3000) -> reply1 (2000) -> root (1000)，且 Trash 副本被去重。
        let desc = cache
            .list_thread_messages(1, reply2_id, "全新主题名（依然同线程）", true)
            .unwrap();
        assert_eq!(desc.len(), 3);
        assert_eq!(desc[0].rfc_message_id, "<reply2@example.com>");
        assert_eq!(desc[1].rfc_message_id, "<reply1@example.com>");
        assert_eq!(desc[2].rfc_message_id, "<root@example.com>");
        assert_eq!(
            desc[2].id, root_inbox_id,
            "跨文件夹去重应优先保留当前文件夹/非 Trash 副本"
        );

        // 升序：root -> reply1 -> reply2。
        let asc = cache
            .list_thread_messages(1, reply2_id, "全新主题名（依然同线程）", false)
            .unwrap();
        assert_eq!(asc.len(), 3);
        assert_eq!(asc[0].rfc_message_id, "<root@example.com>");
        assert_eq!(asc[1].rfc_message_id, "<reply1@example.com>");
        assert_eq!(asc[2].rfc_message_id, "<reply2@example.com>");

        // get_message_detail 默认使用降序。
        let (detail, _, _) = cache.get_message_detail(reply2_id).unwrap().unwrap();
        assert_eq!(detail.thread_messages.len(), 3);
        assert_eq!(detail.thread_messages[0].id, reply2_id);
    }

    #[test]
    fn list_thread_messages_excludes_trash_and_junk_unless_anchor_is_there() {
        let mut cache = CacheStore::open_in_memory().unwrap();
        cache
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
                        name: "已删除".to_string(),
                        path: "Trash".to_string(),
                        unread_count: 0,
                        total_count: 0,
                    },
                    RemoteFolder {
                        name: "垃圾邮件".to_string(),
                        path: "Junk".to_string(),
                        unread_count: 0,
                        total_count: 0,
                    },
                ],
            )
            .unwrap();
        let folders = cache.list_folders(1).unwrap();
        let inbox = folders.iter().find(|f| f.path == "INBOX").unwrap().id;
        let trash = folders.iter().find(|f| f.path == "Trash").unwrap().id;
        let junk = folders.iter().find(|f| f.path == "Junk").unwrap().id;

        cache
            .commit_folder_headers(
                1,
                inbox,
                1,
                &[CachedHeaderInput {
                    uid: 1,
                    message_id: "<root@example.com>".to_string(),
                    in_reply_to: String::new(),
                    references_header: String::new(),
                    sender: "Alice <alice@example.com>".to_string(),
                    recipients: "me@example.com".to_string(),
                    subject: "Thread Cleanup".to_string(),
                    preview: "root".to_string(),
                    date_ts: 1000,
                    date_display: "09:00".to_string(),
                    unread: false,
                    flagged: false,
                    has_attachment: false,
                }],
                None,
            )
            .unwrap();
        cache
            .commit_folder_headers(
                1,
                trash,
                2,
                &[CachedHeaderInput {
                    uid: 2,
                    message_id: "<deleted-reply@example.com>".to_string(),
                    in_reply_to: "<root@example.com>".to_string(),
                    references_header: "<root@example.com>".to_string(),
                    sender: "Bob <bob@example.com>".to_string(),
                    recipients: "me@example.com".to_string(),
                    subject: "Re: Thread Cleanup".to_string(),
                    preview: "in trash".to_string(),
                    date_ts: 2000,
                    date_display: "10:00".to_string(),
                    unread: false,
                    flagged: false,
                    has_attachment: false,
                }],
                None,
            )
            .unwrap();
        cache
            .commit_folder_headers(
                1,
                junk,
                3,
                &[CachedHeaderInput {
                    uid: 3,
                    message_id: "<spam-reply@example.com>".to_string(),
                    in_reply_to: "<root@example.com>".to_string(),
                    references_header: "<root@example.com>".to_string(),
                    sender: "Spammer <spam@example.com>".to_string(),
                    recipients: "me@example.com".to_string(),
                    subject: "Re: Thread Cleanup".to_string(),
                    preview: "in junk".to_string(),
                    date_ts: 3000,
                    date_display: "11:00".to_string(),
                    unread: false,
                    flagged: false,
                    has_attachment: false,
                }],
                None,
            )
            .unwrap();

        let root_id = cache.list_messages_page(inbox, "", 0, 10).unwrap().items[0].id;
        let from_inbox = cache
            .list_thread_messages(1, root_id, "Thread Cleanup", true)
            .unwrap();
        assert_eq!(from_inbox.len(), 1);
        assert_eq!(from_inbox[0].rfc_message_id, "<root@example.com>");

        let trash_id = cache.list_messages_page(trash, "", 0, 10).unwrap().items[0].id;
        let from_trash = cache
            .list_thread_messages(1, trash_id, "Re: Thread Cleanup", true)
            .unwrap();
        assert!(from_trash
            .iter()
            .any(|m| m.rfc_message_id == "<deleted-reply@example.com>"));
    }

    #[test]
    fn reconcile_folder_flags_updates_older_messages_and_unread_count() {
        let mut cache = CacheStore::open_in_memory().unwrap();
        cache
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
        let inbox = cache.list_folders(1).unwrap()[0].id;

        let mk = |uid: u32, unread: bool, flagged: bool| CachedHeaderInput {
            uid,
            message_id: format!("<m-{uid}@example.com>"),
            in_reply_to: String::new(),
            references_header: String::new(),
            sender: "Alice <alice@example.com>".to_string(),
            recipients: "me@example.com".to_string(),
            subject: format!("Mail {uid}"),
            preview: "preview".to_string(),
            date_ts: 1000 + uid as i64,
            date_display: "10:00".to_string(),
            unread,
            flagged,
            has_attachment: false,
        };
        cache
            .commit_folder_headers(
                1,
                inbox,
                1,
                &[mk(1, true, false), mk(2, false, true), mk(3, true, true)],
                None,
            )
            .unwrap();

        let flagged: HashSet<u32> = [1, 3].into();
        let unseen: HashSet<u32> = [2, 3].into();
        let updated = cache
            .reconcile_folder_flags(inbox, &flagged, &unseen)
            .unwrap();
        assert_eq!(updated, 2);

        let page = cache.list_messages_page(inbox, "", 0, 10).unwrap();
        let m1 = page.items.iter().find(|m| m.uid == 1).unwrap();
        let m2 = page.items.iter().find(|m| m.uid == 2).unwrap();
        let m3 = page.items.iter().find(|m| m.uid == 3).unwrap();
        assert!(!m1.unread && m1.flagged);
        assert!(m2.unread && !m2.flagged);
        assert!(m3.unread && m3.flagged);

        let folder = cache.list_folders(1).unwrap().into_iter().next().unwrap();
        assert_eq!(folder.unread_count, 2);
    }

    #[test]
    fn unique_attachment_path_appends_incrementing_suffix_on_collision() {
        let dir = std::env::temp_dir().join(format!("hylki-att-collision-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let p0 = unique_attachment_path(&dir, "report.pdf");
        assert_eq!(p0, dir.join("report.pdf"));
        std::fs::write(&p0, b"first").unwrap();

        let p1 = unique_attachment_path(&dir, "report.pdf");
        assert_eq!(p1, dir.join("report (1).pdf"));
        std::fs::write(&p1, b"second").unwrap();

        let p2 = unique_attachment_path(&dir, "report.pdf");
        assert_eq!(p2, dir.join("report (2).pdf"));

        // 无扩展名文件也能正确追加后缀。
        let no_ext0 = unique_attachment_path(&dir, "README");
        assert_eq!(no_ext0, dir.join("README"));
        std::fs::write(&no_ext0, b"readme").unwrap();
        let no_ext1 = unique_attachment_path(&dir, "README");
        assert_eq!(no_ext1, dir.join("README (1)"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remote_cursor_and_incremental_older_batch_preserve_cached_history() {
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
        assert!(!folders[0].has_more_remote);

        // 首次拉取最新一批（UID 101..=102），服务端还有更早的 1..=100。
        cache
            .commit_folder_headers(
                1,
                inbox,
                1,
                &[
                    sample_header(101, "较新邮件1"),
                    sample_header(102, "最新邮件2"),
                ],
                Some(&[50, 51, 101, 102]),
            )
            .unwrap();
        cache
            .set_folder_remote_cursor(inbox, true, "https://graph.example/next?skip=100")
            .unwrap();

        let (acc_id, path, has_more, next_link, min_uid, count) =
            cache.folder_remote_cursor(inbox).unwrap().unwrap();
        assert_eq!(acc_id, 1);
        assert_eq!(path, "INBOX");
        assert!(has_more);
        assert_eq!(next_link, "https://graph.example/next?skip=100");
        assert_eq!(min_uid, Some(101));
        assert_eq!(count, 2);

        let pending_inboxes = cache.list_inbox_folders_with_more_remote().unwrap();
        assert_eq!(pending_inboxes.len(), 1);
        assert_eq!(pending_inboxes[0].0, inbox);

        // 预加载更早一批（UID 50..=51），随后再执行 Graph 式不带 prune 的顶部刷新，历史邮件不得丢失。
        cache
            .commit_folder_headers(
                1,
                inbox,
                1,
                &[
                    sample_header(50, "更早邮件1"),
                    sample_header(51, "更早邮件2"),
                ],
                Some(&[50, 51, 101, 102]),
            )
            .unwrap();
        cache.set_folder_remote_cursor(inbox, false, "").unwrap();

        // 模拟后续增量同步最新一批（prune = None），已预加载的 50/51 仍应保留。
        cache
            .commit_folder_headers(1, inbox, 1, &[sample_header(103, "新到达邮件")], None)
            .unwrap();

        let (_, _, has_more_after, next_after, min_uid_after, count_after) =
            cache.folder_remote_cursor(inbox).unwrap().unwrap();
        assert!(!has_more_after);
        assert!(next_after.is_empty());
        assert_eq!(min_uid_after, Some(50));
        assert_eq!(count_after, 5);
        assert!(cache
            .list_inbox_folders_with_more_remote()
            .unwrap()
            .is_empty());

        let reloaded = cache.list_folders(1).unwrap();
        assert!(!reloaded[0].has_more_remote);
        assert_eq!(reloaded[0].total_count, 5);
    }

    #[test]
    fn background_search_text_covers_cached_archive_without_marking_body_loaded() {
        let mut cache = CacheStore::open_in_memory().unwrap();
        let archive = cache.sync_folders(1, &[folder("归档", "Archive")]).unwrap()[0].id;
        cache
            .commit_folder_headers(1, archive, 1, &[sample_header(9, "旧邮件")], None)
            .unwrap();

        let pending = cache.list_unindexed_messages(archive, 5).unwrap();
        assert_eq!(pending.len(), 1);
        let (id, uid, _) = pending[0];
        assert_eq!(uid, 9);
        assert_eq!(
            cache.list_backfill_folders().unwrap(),
            vec![(archive, 1, "Archive".to_string(), false)]
        );

        let text = format!("{}末尾合同 archivecode", "正文开头".repeat(1500));
        cache.store_index_text(id, &text).unwrap();
        assert!(cache
            .list_unindexed_messages(archive, 5)
            .unwrap()
            .is_empty());
        assert!(cache.list_backfill_folders().unwrap().is_empty());
        assert!(!cache.get_message_detail(id).unwrap().unwrap().2);

        for query in ["archivecode", "末尾合同", "archivecode 末尾合同"] {
            let page = cache
                .search_all_folders(
                    &[1],
                    query,
                    QuickFilterSet::default(),
                    MessageSort::DateNewest,
                    0,
                    5,
                )
                .unwrap();
            assert_eq!(page.items[0].id, id);
        }

        assert_eq!(cache.clear_message_bodies().unwrap(), 1);
        assert_eq!(
            cache
                .list_messages_page(archive, "archivecode", 0, 5)
                .unwrap()
                .total,
            0
        );
        assert_eq!(cache.list_unindexed_messages(archive, 5).unwrap().len(), 1);

        cache.store_index_text(id, "stalesearchexample").unwrap();
        cache
            .store_message_body(id, "fullsearchexample", &[])
            .unwrap();
        assert!(cache.get_message_detail(id).unwrap().unwrap().2);
        assert_eq!(
            cache
                .list_messages_page(archive, "stalesearchexample", 0, 5)
                .unwrap()
                .total,
            0
        );
        assert_eq!(
            cache
                .list_messages_page(archive, "fullsearchexample", 0, 5)
                .unwrap()
                .total,
            1
        );
    }

    #[test]
    fn fts5_and_chinese_body_search_with_structured_operators() {
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
                        name: "归档".to_string(),
                        path: "Archive".to_string(),
                        unread_count: 0,
                        total_count: 0,
                    },
                ],
            )
            .unwrap();
        let inbox = folders[0].id;
        let archive = folders[1].id;

        let mut h1 = sample_header(1, "Q4 Financial Review");
        h1.sender = "Mário Rossi <mario@example.com>".to_string();
        h1.recipients = "财务部 <finance@company.cn>".to_string();
        h1.unread = true;
        h1.flagged = true;
        h1.has_attachment = true;
        h1.date_ts = 1_700_000_200;

        let mut h2 = sample_header(2, "Weekly Standup");
        h2.sender = "Bob Lee <bob@example.com>".to_string();
        h2.recipients = "team@company.cn".to_string();
        h2.unread = false;
        h2.flagged = false;
        h2.has_attachment = false;
        h2.date_ts = 1_700_000_100;

        let mut h3 = sample_header(3, "采购协议归档");
        h3.sender = "法务部 <legal@company.cn>".to_string();
        h3.recipients = "张总 <ceo@company.cn>".to_string();
        h3.unread = false;
        h3.flagged = true;
        h3.has_attachment = true;
        h3.date_ts = 1_700_000_300;

        cache
            .commit_folder_headers(1, inbox, 1, &[h1, h2], None)
            .unwrap();
        cache
            .commit_folder_headers(1, archive, 1, &[h3], None)
            .unwrap();

        let inbox_page = cache.list_messages_page(inbox, "", 0, 10).unwrap();
        let m1_id = inbox_page.items.iter().find(|m| m.uid == 1).unwrap().id;
        let m2_id = inbox_page.items.iter().find(|m| m.uid == 2).unwrap().id;
        let archive_page = cache.list_messages_page(archive, "", 0, 10).unwrap();
        let m3_id = archive_page.items[0].id;

        cache
            .store_message_body(m1_id, "请核对本月增值税发票及 quarterly budget 报表。", &[])
            .unwrap();
        cache
            .store_message_body(m2_id, "Everything is on track for Friday release.", &[])
            .unwrap();
        cache
            .store_message_body(m3_id, "附件为最终签署的年度采购合同扫描件。", &[])
            .unwrap();

        // 1. 中文正文搜索（发票 / 合同）
        let cn_invoice = cache.list_messages_page(inbox, "发票", 0, 10).unwrap();
        assert_eq!(cn_invoice.total, 1);
        assert_eq!(cn_invoice.items[0].uid, 1);

        let cn_contract = cache
            .search_all_folders(
                &[1],
                "合同",
                QuickFilterSet::default(),
                MessageSort::DateNewest,
                0,
                10,
            )
            .unwrap();
        assert_eq!(cn_contract.total, 1);
        assert_eq!(cn_contract.items[0].uid, 3);

        // 2. FTS5 英文正文前缀、短语与重音折叠搜索
        let fts_prefix = cache.list_messages_page(inbox, "quart", 0, 10).unwrap();
        assert_eq!(fts_prefix.total, 1);
        assert_eq!(fts_prefix.items[0].uid, 1);

        let fts_phrase = cache
            .list_messages_page(inbox, r#""quarterly budget""#, 0, 10)
            .unwrap();
        assert_eq!(fts_phrase.total, 1);
        assert_eq!(fts_phrase.items[0].uid, 1);

        let fts_diacritic = cache.list_messages_page(inbox, "mario", 0, 10).unwrap();
        assert_eq!(fts_diacritic.total, 1);
        assert_eq!(fts_diacritic.items[0].uid, 1);

        // 3. 结构化操作符：from / to / in / has:attachment / is:unread / is:read / is:starred
        let op_from_to = cache
            .search_all_folders(
                &[1],
                "from:mario to:finance has:attachment is:unread is:starred",
                QuickFilterSet::default(),
                MessageSort::DateNewest,
                0,
                10,
            )
            .unwrap();
        assert_eq!(op_from_to.total, 1);
        assert_eq!(op_from_to.items[0].uid, 1);

        let op_read_in = cache
            .search_all_folders(
                &[1],
                "is:read in:archive",
                QuickFilterSet::default(),
                MessageSort::DateNewest,
                0,
                10,
            )
            .unwrap();
        assert_eq!(op_read_in.total, 1);
        assert_eq!(op_read_in.items[0].uid, 3);
    }

    #[test]
    fn sort_orders_and_multi_quick_filters() {
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
        let now_ts = chrono::Utc::now().timestamp();
        let old_ts = now_ts - 10 * 86_400;

        let mut a = sample_header(1, "Zebra");
        a.sender = "Charlie <c@x.com>".to_string();
        a.date_ts = old_ts;
        a.unread = false;
        a.flagged = true;
        a.has_attachment = false;

        let mut b = sample_header(2, "Alpha");
        b.sender = "Alice <a@x.com>".to_string();
        b.date_ts = now_ts;
        b.unread = true;
        b.flagged = true;
        b.has_attachment = true;

        let mut c = sample_header(3, "Middle");
        c.sender = "Bob <b@x.com>".to_string();
        c.date_ts = now_ts - 3600;
        c.unread = true;
        c.flagged = false;
        c.has_attachment = true;

        cache
            .commit_folder_headers(1, inbox, 1, &[a, b, c], None)
            .unwrap();

        let uids_for = |sort: MessageSort| -> Vec<u32> {
            cache
                .list_messages_page_with_options(inbox, "", QuickFilterSet::default(), sort, 0, 10)
                .unwrap()
                .items
                .into_iter()
                .map(|m| m.uid)
                .collect()
        };

        assert_eq!(uids_for(MessageSort::DateNewest), vec![2, 3, 1]);
        assert_eq!(uids_for(MessageSort::DateOldest), vec![1, 3, 2]);
        assert_eq!(uids_for(MessageSort::SenderAsc), vec![2, 3, 1]);
        assert_eq!(uids_for(MessageSort::SubjectAsc), vec![2, 3, 1]);
        assert_eq!(uids_for(MessageSort::UnreadFirst), vec![2, 3, 1]);
        assert_eq!(uids_for(MessageSort::StarredFirst), vec![2, 1, 3]);

        // 多条件组合：未读 + 星标 + 含附件 + 今天
        let multi = QuickFilterSet {
            unread: true,
            starred: true,
            has_attachment: true,
            today: true,
            ..QuickFilterSet::default()
        };
        let filtered = cache
            .list_messages_page_with_options(inbox, "", multi, MessageSort::DateNewest, 0, 10)
            .unwrap();
        assert_eq!(filtered.total, 1);
        assert_eq!(filtered.items[0].uid, 2);

        // WP-08：标签、稍后提醒、复制到文件夹与扩展筛选
        let msg_id = filtered.items[0].id;
        cache
            .set_message_tags(msg_id, &["重要".to_string(), "工作".to_string()])
            .unwrap();
        cache.snooze_message(msg_id, 1_900_000_000).unwrap();
        let tagged_page = cache
            .list_messages_page_with_options(
                inbox,
                "",
                QuickFilterSet {
                    tagged: true,
                    ..QuickFilterSet::default()
                },
                MessageSort::DateNewest,
                0,
                10,
            )
            .unwrap();
        assert_eq!(tagged_page.total, 1);
        assert_eq!(tagged_page.items[0].tags, vec!["重要", "工作"]);
        assert_eq!(tagged_page.items[0].snoozed_until_ts, 1_900_000_000);
    }

    #[test]
    fn cross_folder_search_excludes_waste_and_deduplicates_by_message_id() {
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
                        name: "归档".to_string(),
                        path: "Archive".to_string(),
                        unread_count: 0,
                        total_count: 0,
                    },
                    RemoteFolder {
                        name: "已删除".to_string(),
                        path: "Trash".to_string(),
                        unread_count: 0,
                        total_count: 0,
                    },
                    RemoteFolder {
                        name: "垃圾邮件".to_string(),
                        path: "Junk".to_string(),
                        unread_count: 0,
                        total_count: 0,
                    },
                ],
            )
            .unwrap();
        let inbox = folders[0].id;
        let archive = folders[1].id;
        let trash = folders[2].id;
        let junk = folders[3].id;

        // 同一封邮件（相同 message_id）同时存在于 INBOX 与 Archive，应去重并保留 INBOX 副本。
        let mut in_copy = sample_header(10, "重要合同通知");
        in_copy.message_id = "<dup-001@example.com>".to_string();
        let mut arch_copy = sample_header(20, "重要合同通知");
        arch_copy.message_id = "<dup-001@example.com>".to_string();
        let mut arch_only = sample_header(21, "归档独有合同");
        arch_only.message_id = "<arch-002@example.com>".to_string();
        let mut trash_msg = sample_header(30, "已删除合同");
        trash_msg.message_id = "<trash-003@example.com>".to_string();
        let mut junk_msg = sample_header(40, "垃圾合同广告");
        junk_msg.message_id = "<junk-004@example.com>".to_string();

        cache
            .commit_folder_headers(1, inbox, 1, &[in_copy], None)
            .unwrap();
        cache
            .commit_folder_headers(1, archive, 1, &[arch_copy, arch_only], None)
            .unwrap();
        cache
            .commit_folder_headers(1, trash, 1, &[trash_msg], None)
            .unwrap();
        cache
            .commit_folder_headers(1, junk, 1, &[junk_msg], None)
            .unwrap();

        let all = cache
            .search_all_folders(
                &[1],
                "合同",
                QuickFilterSet::default(),
                MessageSort::DateNewest,
                0,
                20,
            )
            .unwrap();
        assert_eq!(all.total, 2, "应排除 Trash/Junk 并按 message_id 去重");
        assert!(all
            .items
            .iter()
            .any(|m| m.folder_path == "INBOX" && m.uid == 10));
        assert!(all
            .items
            .iter()
            .any(|m| m.folder_path == "Archive" && m.uid == 21));

        // 显式指定 in:trash 时可搜到已删除邮件。
        let in_trash = cache
            .search_all_folders(
                &[1],
                "in:trash 合同",
                QuickFilterSet::default(),
                MessageSort::DateNewest,
                0,
                20,
            )
            .unwrap();
        assert_eq!(in_trash.total, 1);
        assert_eq!(in_trash.items[0].uid, 30);
    }

    #[test]
    fn find_spam_folder_matches_junk_spam_and_chinese_names() {
        let mut cache = CacheStore::open_in_memory().unwrap();
        cache
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
                        name: "垃圾邮件".to_string(),
                        path: "Junk".to_string(),
                        unread_count: 0,
                        total_count: 0,
                    },
                ],
            )
            .unwrap();
        let (_, path1) = cache
            .find_spam_folder(1)
            .unwrap()
            .expect("should find Junk");
        assert_eq!(path1, "Junk");

        cache
            .sync_folders(
                2,
                &[RemoteFolder {
                    name: "Spam".to_string(),
                    path: "[Gmail]/Spam".to_string(),
                    unread_count: 0,
                    total_count: 0,
                }],
            )
            .unwrap();
        let (_, path2) = cache
            .find_spam_folder(2)
            .unwrap()
            .expect("should find Spam");
        assert_eq!(path2, "[Gmail]/Spam");

        assert!(cache.find_spam_folder(99).unwrap().is_none());
    }

    #[test]
    fn export_message_attachments_writes_sanitized_unique_files() {
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
            .commit_folder_headers(1, inbox, 1, &[sample_header(10, "带附件邮件")], None)
            .unwrap();
        let page = cache.list_messages_page(inbox, "", 0, 10).unwrap();
        let msg_id = page.items[0].id;
        cache
            .store_message_body(
                msg_id,
                "hello",
                &[
                    CachedAttachmentInput {
                        filename: "report.pdf".to_string(),
                        mime_type: "application/pdf".to_string(),
                        content: b"pdf-1".to_vec(),
                    },
                    CachedAttachmentInput {
                        filename: "report.pdf".to_string(),
                        mime_type: "application/pdf".to_string(),
                        content: b"pdf-2".to_vec(),
                    },
                    CachedAttachmentInput {
                        filename: "../bad:name?.txt".to_string(),
                        mime_type: "text/plain".to_string(),
                        content: b"txt".to_vec(),
                    },
                ],
            )
            .unwrap();

        let dest_dir = std::env::temp_dir().join("hylki-test-forward-export");
        let _ = std::fs::remove_dir_all(&dest_dir);
        let paths = cache.export_message_attachments(msg_id, &dest_dir).unwrap();
        assert_eq!(paths.len(), 3);
        assert_eq!(std::fs::read(&paths[0]).unwrap(), b"pdf-1");
        assert_eq!(std::fs::read(&paths[1]).unwrap(), b"pdf-2");
        assert_eq!(std::fs::read(&paths[2]).unwrap(), b"txt");
        assert_ne!(paths[0], paths[1]);
        assert!(paths[2].starts_with(&dest_dir));
        let _ = std::fs::remove_dir_all(&dest_dir);
    }

    /// B-2：.eml 导出文件名过滤 Windows 非法字符（与附件文件名同一规则）。
    #[test]
    fn sanitize_filename_replaces_windows_illegal_chars() {
        assert_eq!(
            sanitize_filename("周报 Q3/四季度:评审?"),
            "周报 Q3_四季度_评审_"
        );
        assert_eq!(sanitize_filename("  a*b|c<d>e \"f\"  "), "a_b_c_d_e _f_");
        // 全部为点 / 空白时回落默认名。
        assert_eq!(sanitize_filename(" . .. "), "attachment");
        assert_eq!(sanitize_filename(""), "attachment");
        assert_eq!(sanitize_filename("普通主题"), "普通主题");
    }

    /// B-4：会话计数按归一化主题聚合（Re: / Fwd: 前缀剔除后同键），并随分页查询返回。
    #[test]
    fn list_page_fills_thread_counts_by_normalized_subject() {
        let mut cache = CacheStore::open_in_memory().expect("open memory db");
        let folders = cache
            .sync_folders(
                1,
                &[
                    RemoteFolder {
                        name: "收件箱".to_string(),
                        path: "INBOX".to_string(),
                        unread_count: 0,
                        total_count: 3,
                    },
                    RemoteFolder {
                        name: "已发送".to_string(),
                        path: "Sent".to_string(),
                        unread_count: 0,
                        total_count: 1,
                    },
                ],
            )
            .unwrap();
        let inbox_id = folders[0].id;
        let sent_id = folders[1].id;

        let mut h1 = sample_header(1, "项目排期讨论");
        h1.date_ts = 1_700_000_100;
        let mut h2 = sample_header(2, "Re: 项目排期讨论");
        h2.date_ts = 1_700_000_200;
        h2.recipients = "team@example.com".to_string();
        let mut h3 = sample_header(3, "Fwd: 项目排期讨论");
        h3.date_ts = 1_700_000_300;
        let mut h4 = sample_header(4, "独立主题邮件");
        h4.date_ts = 1_700_000_400;
        cache
            .commit_folder_headers(1, inbox_id, 100, &[h1, h2, h3, h4], None)
            .unwrap();
        let mut hs = sample_header(5, "Re: 项目排期讨论");
        hs.date_ts = 1_700_000_500;
        cache
            .commit_folder_headers(1, sent_id, 200, &[hs], None)
            .unwrap();

        let page = cache.list_messages_page(inbox_id, "", 0, 10).unwrap();
        let by_subject: std::collections::HashMap<String, u32> = page
            .items
            .iter()
            .map(|m| (m.subject.clone(), m.thread_count))
            .collect();
        // 同一归一化主题跨文件夹共 4 封；独立主题只有 1 封（不显示胶囊）。
        assert_eq!(by_subject["项目排期讨论"], 4);
        assert_eq!(by_subject["Re: 项目排期讨论"], 4);
        assert_eq!(by_subject["Fwd: 项目排期讨论"], 4);
        assert_eq!(by_subject["独立主题邮件"], 1);
        // B-6：收件人列随摘要一并返回。
        let with_recipients = page
            .items
            .iter()
            .find(|m| m.subject == "Re: 项目排期讨论")
            .unwrap();
        assert_eq!(with_recipients.recipients, "team@example.com");
    }

    #[test]
    fn extract_unsubscribe_url_picks_first_supported_handle() {
        // 标准写法：https 优先于 mailto（按出现顺序取第一个可执行项）。
        assert_eq!(
            extract_unsubscribe_url(
                "<https://lists.example.com/unsub?id=1>, <mailto:leave@lists.example.com>"
            ),
            Some("https://lists.example.com/unsub?id=1".to_string())
        );
        // 只有 mailto。
        assert_eq!(
            extract_unsubscribe_url("<mailto:leave@lists.example.com?subject=unsubscribe>"),
            Some("mailto:leave@lists.example.com?subject=unsubscribe".to_string())
        );
        // 无尖括号（Amazon SES 等变体）。
        assert_eq!(
            extract_unsubscribe_url("https://unsub.example.com/u?a=b"),
            Some("https://unsub.example.com/u?a=b".to_string())
        );
        // 大小写不敏感的协议头 + 空白。
        assert_eq!(
            extract_unsubscribe_url("  HTTPS://Unsub.Example.com/x  "),
            Some("HTTPS://Unsub.Example.com/x".to_string())
        );
        // 非法 / 空值不产生 URL；ftp 等不支持的协议被跳过。
        assert_eq!(extract_unsubscribe_url(""), None);
        assert_eq!(extract_unsubscribe_url("<ftp://example.com/x>"), None);
        assert_eq!(
            extract_unsubscribe_url("<ftp://example.com/x>, <mailto:leave@x.example>"),
            Some("mailto:leave@x.example".to_string())
        );
    }

    #[test]
    fn is_one_click_post_matches_rfc8058_value() {
        assert!(is_one_click_post("List-Unsubscribe=One-Click"));
        assert!(is_one_click_post(" list-unsubscribe = one-click "));
        assert!(is_one_click_post("\"List-Unsubscribe=One-Click\""));
        assert!(!is_one_click_post("List-Unsubscribe=Two-Click"));
        assert!(!is_one_click_post(""));
    }

    #[test]
    fn user_contacts_crud_and_merge_with_aggregated_contacts() {
        let mut store = CacheStore::open_in_memory().unwrap();
        store
            .sync_folders(1, &[folder("收件箱", "INBOX"), folder("已发送", "Sent")])
            .unwrap();
        let inbox_id = store.find_inbox_folder(1).unwrap().unwrap().0;
        store
            .commit_folder_headers(
                1,
                inbox_id,
                1,
                &[
                    sample_header(1, "季度会议通知"),
                    sample_header(2, "会议纪要"),
                ],
                None,
            )
            .unwrap();

        // 插入两个自定义联系人（其一与聚合联系人同邮箱）。
        let id_new = store
            .upsert_user_contact(&UserContact {
                id: 0,
                email: "custom@example.com".to_string(),
                display_name: "自定义联系人".to_string(),
                notes: "备注 A".to_string(),
            })
            .unwrap();
        assert!(id_new > 0);
        let alice_email = "alice@example.com".to_string();
        let id_alice = store
            .upsert_user_contact(&UserContact {
                id: 0,
                email: alice_email.clone(),
                display_name: "Alice 手工版".to_string(),
                notes: String::new(),
            })
            .unwrap();

        // 邮箱唯一约束：重复插入报错。
        assert!(store
            .upsert_user_contact(&UserContact {
                id: 0,
                email: alice_email.clone(),
                display_name: "重复".to_string(),
                notes: String::new(),
            })
            .is_err());

        let contacts = store.list_known_contacts("", 0, 50).unwrap();
        let custom = contacts.iter().filter(|c| c.custom).count();
        assert_eq!(custom, 2);
        // 自定义联系人排在最前。
        assert!(contacts[0].custom && contacts[1].custom);
        let merged = contacts
            .iter()
            .find(|c| c.email == alice_email)
            .expect("聚合邮箱应与自定义联系人合并");
        assert_eq!(merged.id, id_alice);
        assert_eq!(merged.name, "Alice 手工版");
        // 聚合往来计数保留。
        assert!(merged.message_count >= 2);
        let pure_custom = contacts
            .iter()
            .find(|c| c.email == "custom@example.com")
            .unwrap();
        assert_eq!(pure_custom.notes, "备注 A");
        assert_eq!(pure_custom.message_count, 0);

        // 更新 / 删除。
        let updated = store
            .upsert_user_contact(&UserContact {
                id: id_new,
                email: "custom@example.com".to_string(),
                display_name: "改名".to_string(),
                notes: String::new(),
            })
            .unwrap();
        assert_eq!(updated, id_new);
        let renamed = store.list_user_contacts().unwrap();
        assert_eq!(renamed.len(), 2);
        assert_eq!(
            renamed
                .iter()
                .find(|c| c.id == id_new)
                .unwrap()
                .display_name,
            "改名"
        );
        assert!(store.delete_user_contact(id_new).unwrap());
        assert!(!store.delete_user_contact(id_new).unwrap());

        // 批量导入：按邮箱去重。
        let (added, skipped) = store
            .import_user_contacts(&[
                UserContact {
                    id: 0,
                    email: "alice@example.com".to_string(),
                    display_name: String::new(),
                    notes: String::new(),
                },
                UserContact {
                    id: 0,
                    email: "batch1@example.com".to_string(),
                    display_name: "批量一".to_string(),
                    notes: String::new(),
                },
                UserContact {
                    id: 0,
                    email: "not-an-email".to_string(),
                    display_name: String::new(),
                    notes: String::new(),
                },
            ])
            .unwrap();
        assert_eq!(added, 1);
        assert_eq!(skipped, 2);
    }

    #[test]
    fn store_message_body_persists_unsubscribe_headers() {
        let mut store = CacheStore::open_in_memory().unwrap();
        store.sync_folders(1, &[folder("收件箱", "INBOX")]).unwrap();
        let inbox_id = store.find_inbox_folder(1).unwrap().unwrap().0;
        store
            .commit_folder_headers(1, inbox_id, 1, &[sample_header(1, "周刊")], None)
            .unwrap();
        let msg_id = store.list_messages_page(inbox_id, "", 0, 10).unwrap().items[0].id;
        store
            .store_message_body_with_html(
                msg_id,
                "正文",
                None,
                &[],
                Some("https://lists.example.com/unsub"),
                true,
            )
            .unwrap();
        let (detail, _, _) = store.get_message_detail(msg_id).unwrap().unwrap();
        assert_eq!(
            detail.unsubscribe_url.as_deref(),
            Some("https://lists.example.com/unsub")
        );
        assert!(detail.unsubscribe_one_click);
    }

    #[test]
    fn open_old_database_keeps_drafts_and_creates_user_contacts() {
        // 模拟旧版本库：只含 drafts 表（无 user_contacts / list_unsubscribe 列），
        // 打开时补表补列且历史数据保留（迁移兼容）。
        let dir = std::env::temp_dir().join(format!(
            "hylki-migrate-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("cache.db");
        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "
                CREATE TABLE drafts (
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
                INSERT INTO drafts (account_id, to_addr, subject, body_text, updated_at)
                VALUES (1, 'old@example.com', '旧草稿', '旧内容', 123);
                ",
            )
            .unwrap();
        }
        let mut store = CacheStore::open(&db_path).unwrap();
        // 旧草稿仍在（旧库无 cc/bcc 列，补列后历史数据视为空串）。
        let drafts = store.list_drafts().unwrap();
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].subject, "旧草稿");
        // 新表可用。
        let id = store
            .upsert_user_contact(&UserContact {
                id: 0,
                email: "new@example.com".to_string(),
                display_name: "新联系人".to_string(),
                notes: String::new(),
            })
            .unwrap();
        assert!(id > 0);
        drop(store);
        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(dir.join("cache.db-wal"));
        let _ = std::fs::remove_file(dir.join("cache.db-shm"));
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn unified_queries_filter_by_allowed_account_ids() {
        let mut store = CacheStore::open_in_memory().unwrap();
        store
            .sync_folders(1, &[folder("收件箱", "INBOX"), folder("已发送", "Sent")])
            .unwrap();
        store
            .sync_folders(2, &[folder("收件箱", "INBOX"), folder("已发送", "Sent")])
            .unwrap();
        let inbox_1 = store.find_inbox_folder(1).unwrap().unwrap().0;
        let inbox_2 = store.find_inbox_folder(2).unwrap().unwrap().0;
        store
            .commit_folder_headers(1, inbox_1, 1, &[sample_header(1, "账号一邮件")], None)
            .unwrap();
        store
            .commit_folder_headers(2, inbox_2, 1, &[sample_header(2, "账号二邮件")], None)
            .unwrap();

        let empty = store
            .list_unified_inbox_for_accounts(
                &[],
                "",
                QuickFilterSet::default(),
                MessageSort::DateNewest,
                0,
                50,
            )
            .unwrap();
        assert_eq!(empty.total, 0);
        assert!(empty.items.is_empty());

        let only_two = store
            .list_unified_inbox_for_accounts(
                &[2],
                "",
                QuickFilterSet::default(),
                MessageSort::DateNewest,
                0,
                50,
            )
            .unwrap();
        assert_eq!(only_two.total, 1);
        assert_eq!(only_two.items[0].account_id, 2);

        let both = store
            .list_unified_inbox_for_accounts(
                &[1, 2],
                "",
                QuickFilterSet::default(),
                MessageSort::DateNewest,
                0,
                50,
            )
            .unwrap();
        assert_eq!(both.total, 2);
    }

    #[test]
    fn wp06_tags_crud_and_apply_filter_rules() {
        let mut store = CacheStore::open_in_memory().unwrap();
        let t1 = store.upsert_tag(None, "重要", "#e01b24", Some(1)).unwrap();
        let t2 = store.upsert_tag(None, "账单", "#3584e4", Some(1)).unwrap();
        let list = store.list_tags().unwrap();
        assert_eq!(list.len(), 2);
        // 快捷键 1 被 t2 抢占后，t1 自动解绑。
        let reloaded_t1 = list.iter().find(|t| t.id == t1.id).unwrap();
        let reloaded_t2 = list.iter().find(|t| t.id == t2.id).unwrap();
        assert_eq!(reloaded_t1.shortcut, None);
        assert_eq!(reloaded_t2.shortcut, Some(1));

        assert!(store.delete_tag(t1.id).unwrap());
        assert_eq!(store.list_tags().unwrap().len(), 1);

        // 插入一封测试邮件并运行过滤规则
        store
            .conn
            .execute(
                "INSERT INTO folders (account_id, name, path) VALUES (1, '收件箱', 'INBOX')",
                [],
            )
            .unwrap();
        let inbox_id: i64 = store.conn.last_insert_rowid();
        store
            .conn
            .execute(
                "INSERT INTO folders (account_id, name, path) VALUES (1, '归档', 'Archive')",
                [],
            )
            .unwrap();
        store
            .conn
            .execute(
                "INSERT INTO messages (account_id, folder_id, uid, sender, subject, unread, flagged) VALUES (1, ?1, 101, 'billing@example.com', 'Monthly Invoice', 1, 0)",
                params![inbox_id],
            )
            .unwrap();

        let rules = vec![crate::config::FilterRule {
            id: 1,
            name: "账单自动处理".to_string(),
            account_email: String::new(),
            enabled: true,
            match_all: true,
            field: crate::config::FilterField::From,
            matcher: crate::config::FilterMatch::Contains,
            value: "billing@".to_string(),
            move_to_folder: String::new(),
            archive: true,
            star: true,
            mark_read: true,
            apply_tag: String::new(),
            forward_to: String::new(),
        }];
        let updated = store.apply_filter_rules(&rules, &[]).unwrap();
        assert_eq!(updated, 1);

        let (unread, flagged): (i64, i64) = store
            .conn
            .query_row(
                "SELECT unread, flagged FROM messages WHERE uid = 101",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(unread, 0);
        assert_eq!(flagged, 1);
    }
}

fn sanitize_attachment_filename(raw: &str) -> String {
    let base = Path::new(raw)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(raw)
        .trim();
    let cleaned: String = base
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            other => other,
        })
        .collect();
    let cleaned = cleaned.trim_matches(|c| c == '.' || c == ' ');
    if cleaned.is_empty() {
        "attachment.bin".to_string()
    } else {
        cleaned.to_string()
    }
}

fn deduplicate_filename(filename: &str, used: &mut std::collections::HashSet<String>) -> String {
    let lower = filename.to_lowercase();
    if used.insert(lower) {
        return filename.to_string();
    }
    let path = Path::new(filename);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(filename);
    let ext = path.extension().and_then(|e| e.to_str());
    for idx in 1..10_000 {
        let candidate = match ext {
            Some(e) if !e.is_empty() => format!("{stem} ({idx}).{e}"),
            _ => format!("{stem} ({idx})"),
        };
        if used.insert(candidate.to_lowercase()) {
            return candidate;
        }
    }
    filename.to_string()
}
