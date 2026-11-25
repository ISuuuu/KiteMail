//! Per-account storage engine:
//! - `<account_dir>/mail.db`: hot index & headers (`folders`, `messages`, `attachments` metadata, `message_rfc_refs`, FTS5)
//! - `<account_dir>/content.db`: cold message bodies (`message_bodies`), opened lazily on demand
//! - `<account_dir>/attachments/<00..ff>/<local_att_id>.bin`: 256-bucket sharded attachment blobs
//!
//! Zero external `account_id` or `slot` values are stored inside `mail.db` or `content.db`,
//! allowing installed and portable Hylki instances to mount the exact same account folder seamlessly.

use std::cell::{RefCell, RefMut};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};

use super::global_db::ensure_column;
use super::id_codec::{decode_scoped_id, encode_scoped_id};
use super::{
    collect_attachment_paths, deduplicate_filename, extract_rfc_message_ids, is_junk_folder,
    is_trash_folder, normalize_thread_subject, sanitize_attachment_filename, sync_message_rfc_refs,
    AttachmentRecord, CachedAttachment, CachedAttachmentInput, CachedHeaderInput, RemoteFolder,
    ARCHIVE_FOLDER_MATCH, JUNK_FOLDER_MATCH, SENT_FOLDER_MATCH, SPAM_FOLDER_MATCH,
    TRASH_FOLDER_MATCH,
};
use crate::search_query::SearchQuery;
use crate::{
    start_of_today_ts, AttachmentMeta, FolderSummary, MailPage, MessageDetail, MessageSort,
    MessageSummary, QuickFilterSet, ThreadMessage,
};

pub(crate) struct AccountDb {
    pub(crate) account_id: u32,
    pub(crate) slot: u8,
    pub(crate) dir: Option<PathBuf>,
    pub(crate) attachments_dir: Option<PathBuf>,
    pub(crate) mail_conn: Connection,
    content_conn: RefCell<Option<Connection>>,
    read_only: bool,
}

/// Extracted local message row used both for direct single-account pages and cross-account merges.
#[derive(Clone, Debug)]
pub(crate) struct MergedMessageCandidate {
    pub(crate) summary: MessageSummary,
    pub(crate) rfc_message_id: String,
    /// `0` Inbox, `1` normal folder, `2` Archive, `3` Trash/Junk (mirrors SQLite dedup priority).
    pub(crate) folder_priority: u8,
}

const MAX_THREAD_BFS_HOPS: usize = 8;
const MAX_THREAD_BFS_ROWS: usize = 200;
const MAX_THREAD_SUBJECT_ROWS: usize = 60;
const MAX_THREAD_MESSAGES: usize = 40;

struct AnchorThreadMeta {
    id: u32,
    folder_id: u32,
    rfc_message_id: String,
    in_reply_to: String,
    references_header: String,
    sender: String,
    date_ts: i64,
    thread_subject: String,
}

#[derive(Clone)]
struct ThreadCountRow {
    id: u32,
    folder_id: u32,
    rfc_message_id: String,
    sender: String,
    date_ts: i64,
}

fn collect_anchor_rfc_seeds(
    rfc_message_id: &str,
    in_reply_to: &str,
    references_header: &str,
) -> (HashSet<String>, bool) {
    let mut seed_ids: HashSet<String> = HashSet::new();
    for tok in extract_rfc_message_ids(rfc_message_id) {
        seed_ids.insert(tok);
    }
    let irt_ids = extract_rfc_message_ids(in_reply_to);
    let ref_ids = extract_rfc_message_ids(references_header);
    let anchor_has_reply_headers = !irt_ids.is_empty() || !ref_ids.is_empty();
    for tok in irt_ids.into_iter().chain(ref_ids) {
        seed_ids.insert(tok);
    }
    (seed_ids, anchor_has_reply_headers)
}

fn count_distinct_thread_rows(
    rows: &[ThreadCountRow],
    extra_anchor: Option<&ThreadCountRow>,
    local_anchor_id: u32,
    anchor_in_trash_or_junk: bool,
    trash_or_junk_folders: &HashMap<u32, bool>,
) -> u32 {
    let mut distinct: u32 = 0;
    let mut seen_keys: HashSet<(String, String, i64)> = HashSet::new();
    for row in rows.iter().chain(extra_anchor) {
        if !anchor_in_trash_or_junk
            && !(local_anchor_id != 0 && row.id == local_anchor_id)
            && trash_or_junk_folders
                .get(&row.folder_id)
                .copied()
                .unwrap_or(false)
        {
            continue;
        }
        let norm_mid = extract_rfc_message_ids(&row.rfc_message_id)
            .into_iter()
            .next()
            .unwrap_or_default();
        if !norm_mid.is_empty() && !seen_keys.insert((norm_mid, row.sender.clone(), row.date_ts)) {
            continue;
        }
        distinct += 1;
        if distinct as usize >= MAX_THREAD_MESSAGES {
            return MAX_THREAD_MESSAGES as u32;
        }
    }
    distinct.max(1)
}

impl AccountDb {
    pub(crate) fn mail_db_path_in(dir: &Path) -> PathBuf {
        dir.join("mail.db")
    }

    pub(crate) fn content_db_path_in(dir: &Path) -> PathBuf {
        dir.join("content.db")
    }

    pub(crate) fn open_in_dir(account_id: u32, slot: u8, dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("创建账号邮件目录失败 ({}): {e}", dir.display()))?;
        let mail_path = Self::mail_db_path_in(dir);
        let mail_conn = Connection::open(&mail_path)
            .map_err(|e| format!("打开账号索引库失败 ({}): {e}", mail_path.display()))?;
        let db = Self {
            account_id,
            slot,
            dir: Some(dir.to_path_buf()),
            attachments_dir: Some(dir.join("attachments")),
            mail_conn,
            content_conn: RefCell::new(None),
            read_only: false,
        };
        db.configure_and_migrate_mail(true)?;
        Ok(db)
    }

    pub(crate) fn open_existing_in_dir(
        account_id: u32,
        slot: u8,
        dir: &Path,
    ) -> Result<Self, String> {
        let mail_path = Self::mail_db_path_in(dir);
        if !mail_path.exists() {
            return Self::open_in_dir(account_id, slot, dir);
        }
        let flags =
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let mail_conn = Connection::open_with_flags(&mail_path, flags)
            .map_err(|e| format!("打开已有账号索引库失败 ({}): {e}", mail_path.display()))?;
        mail_conn
            .execute_batch(
                "PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL; PRAGMA busy_timeout = 5000;",
            )
            .map_err(|e| format!("配置已有账号索引库失败: {e}"))?;
        Ok(Self {
            account_id,
            slot,
            dir: Some(dir.to_path_buf()),
            attachments_dir: Some(dir.join("attachments")),
            mail_conn,
            content_conn: RefCell::new(None),
            read_only: false,
        })
    }

    pub(crate) fn open_read_only_in_dir(
        account_id: u32,
        slot: u8,
        dir: &Path,
    ) -> Result<Option<Self>, String> {
        let mail_path = Self::mail_db_path_in(dir);
        if !mail_path.exists() {
            return Ok(None);
        }
        let flags =
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let mail_conn = Connection::open_with_flags(&mail_path, flags).map_err(|e| {
            format!(
                "以只读方式打开账号索引库失败 ({}): {e}",
                mail_path.display()
            )
        })?;
        mail_conn
            .execute_batch("PRAGMA busy_timeout = 2000; PRAGMA query_only = ON;")
            .map_err(|e| format!("配置只读账号索引库失败: {e}"))?;
        Ok(Some(Self {
            account_id,
            slot,
            dir: Some(dir.to_path_buf()),
            attachments_dir: Some(dir.join("attachments")),
            mail_conn,
            content_conn: RefCell::new(None),
            read_only: true,
        }))
    }

    pub(crate) fn open_in_memory(account_id: u32, slot: u8) -> Result<Self, String> {
        let mail_conn = Connection::open_in_memory()
            .map_err(|e| format!("打开账号内存索引库失败 ({account_id}): {e}"))?;
        let db = Self {
            account_id,
            slot,
            dir: None,
            attachments_dir: None,
            mail_conn,
            content_conn: RefCell::new(None),
            read_only: false,
        };
        db.configure_and_migrate_mail(false)?;
        Ok(db)
    }

    pub(crate) fn checkpoint(&self) {
        if !self.read_only {
            let _ = self
                .mail_conn
                .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
            if let Some(ref conn) = *self.content_conn.borrow() {
                let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
            }
        }
    }

    fn configure_and_migrate_mail(&self, enable_wal: bool) -> Result<(), String> {
        if enable_wal {
            self.mail_conn
                .execute_batch("PRAGMA journal_mode = WAL;")
                .map_err(|e| format!("启用账号索引库 WAL 失败: {e}"))?;
        }
        self.mail_conn
            .execute_batch(
                "
                PRAGMA foreign_keys = ON;
                PRAGMA synchronous = NORMAL;
                PRAGMA busy_timeout = 5000;

                CREATE TABLE IF NOT EXISTS folders (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL,
                    path TEXT NOT NULL UNIQUE,
                    uid_validity INTEGER NOT NULL DEFAULT 0,
                    unread_count INTEGER NOT NULL DEFAULT 0,
                    total_count INTEGER NOT NULL DEFAULT 0,
                    is_inbox INTEGER NOT NULL DEFAULT 0,
                    has_more_remote INTEGER NOT NULL DEFAULT 0,
                    remote_next_link TEXT NOT NULL DEFAULT ''
                );

                CREATE TABLE IF NOT EXISTS messages (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
                    uid INTEGER NOT NULL,
                    remote_id TEXT NOT NULL DEFAULT '',
                    message_id TEXT NOT NULL DEFAULT '',
                    in_reply_to TEXT NOT NULL DEFAULT '',
                    references_header TEXT NOT NULL DEFAULT '',
                    sender TEXT NOT NULL,
                    recipients TEXT NOT NULL DEFAULT '',
                    cc TEXT NOT NULL DEFAULT '',
                    subject TEXT NOT NULL,
                    thread_subject TEXT NOT NULL DEFAULT '',
                    preview TEXT NOT NULL DEFAULT '',
                    date_ts INTEGER NOT NULL DEFAULT 0,
                    date_display TEXT NOT NULL DEFAULT '',
                    unread INTEGER NOT NULL DEFAULT 1,
                    flagged INTEGER NOT NULL DEFAULT 0,
                    has_attachment INTEGER NOT NULL DEFAULT 0,
                    body_loaded INTEGER NOT NULL DEFAULT 0,
                    body_cached_at INTEGER NOT NULL DEFAULT 0,
                    index_text TEXT,
                    list_unsubscribe TEXT NOT NULL DEFAULT '',
                    list_unsubscribe_post INTEGER NOT NULL DEFAULT 0,
                    auth_results TEXT NOT NULL DEFAULT '',
                    importance INTEGER NOT NULL DEFAULT 0,
                    tags TEXT NOT NULL DEFAULT '',
                    answered INTEGER NOT NULL DEFAULT 0,
                    forwarded INTEGER NOT NULL DEFAULT 0,
                    snoozed_until_ts INTEGER NOT NULL DEFAULT 0,
                    UNIQUE(folder_id, uid)
                );

                CREATE INDEX IF NOT EXISTS idx_messages_folder_date
                    ON messages(folder_id, date_ts DESC, uid DESC);
                CREATE INDEX IF NOT EXISTS idx_messages_msgid
                    ON messages(message_id);
                CREATE INDEX IF NOT EXISTS idx_messages_thread_subj
                    ON messages(thread_subject, date_ts DESC, uid DESC);

                CREATE TABLE IF NOT EXISTS attachments (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    message_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
                    filename TEXT NOT NULL,
                    mime_type TEXT NOT NULL,
                    size_bytes INTEGER NOT NULL,
                    content BLOB NOT NULL DEFAULT X'',
                    storage_path TEXT NOT NULL DEFAULT '',
                    downloaded INTEGER NOT NULL DEFAULT 1,
                    remote_part TEXT NOT NULL DEFAULT '',
                    transfer_encoding TEXT NOT NULL DEFAULT ''
                );

                CREATE TABLE IF NOT EXISTS message_rfc_refs (
                    message_row_id INTEGER NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
                    token TEXT NOT NULL,
                    PRIMARY KEY (token, message_row_id)
                );
                CREATE INDEX IF NOT EXISTS idx_message_rfc_refs_row
                    ON message_rfc_refs(message_row_id);
                ",
            )
            .map_err(|e| format!("初始化账号索引库表结构失败: {e}"))?;

        for (table, col, def) in [
            ("folders", "is_inbox", "INTEGER NOT NULL DEFAULT 0"),
            ("folders", "has_more_remote", "INTEGER NOT NULL DEFAULT 0"),
            ("folders", "remote_next_link", "TEXT NOT NULL DEFAULT ''"),
            ("messages", "cc", "TEXT NOT NULL DEFAULT ''"),
            ("messages", "body_loaded", "INTEGER NOT NULL DEFAULT 0"),
            ("messages", "body_cached_at", "INTEGER NOT NULL DEFAULT 0"),
            ("messages", "index_text", "TEXT"),
            ("messages", "list_unsubscribe", "TEXT NOT NULL DEFAULT ''"),
            (
                "messages",
                "list_unsubscribe_post",
                "INTEGER NOT NULL DEFAULT 0",
            ),
            ("messages", "auth_results", "TEXT NOT NULL DEFAULT ''"),
            ("messages", "importance", "INTEGER NOT NULL DEFAULT 0"),
            ("messages", "tags", "TEXT NOT NULL DEFAULT ''"),
            ("messages", "answered", "INTEGER NOT NULL DEFAULT 0"),
            ("messages", "forwarded", "INTEGER NOT NULL DEFAULT 0"),
            ("messages", "snoozed_until_ts", "INTEGER NOT NULL DEFAULT 0"),
            ("messages", "thread_subject", "TEXT NOT NULL DEFAULT ''"),
            ("attachments", "storage_path", "TEXT NOT NULL DEFAULT ''"),
            ("attachments", "downloaded", "INTEGER NOT NULL DEFAULT 1"),
            ("attachments", "remote_part", "TEXT NOT NULL DEFAULT ''"),
            (
                "attachments",
                "transfer_encoding",
                "TEXT NOT NULL DEFAULT ''",
            ),
        ] {
            ensure_column(&self.mail_conn, table, col, def)?;
        }

        self.mail_conn
            .execute_batch(
                "
                CREATE INDEX IF NOT EXISTS idx_messages_needing_index
                    ON messages(folder_id, id) WHERE body_loaded = 0 AND index_text IS NULL;

                CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
                    subject,
                    sender,
                    recipients,
                    preview,
                    content='messages',
                    content_rowid='id',
                    tokenize='unicode61 remove_diacritics 2'
                );

                CREATE TRIGGER IF NOT EXISTS messages_ai AFTER INSERT ON messages BEGIN
                    INSERT INTO messages_fts(rowid, subject, sender, recipients, preview)
                    VALUES (new.id, new.subject, new.sender, new.recipients, new.preview);
                END;

                CREATE TRIGGER IF NOT EXISTS messages_ad AFTER DELETE ON messages BEGIN
                    INSERT INTO messages_fts(messages_fts, rowid, subject, sender, recipients, preview)
                    VALUES ('delete', old.id, old.subject, old.sender, old.recipients, old.preview);
                END;

                CREATE TRIGGER IF NOT EXISTS messages_au
                AFTER UPDATE OF subject, sender, recipients, preview ON messages BEGIN
                    INSERT INTO messages_fts(messages_fts, rowid, subject, sender, recipients, preview)
                    VALUES ('delete', old.id, old.subject, old.sender, old.recipients, old.preview);
                    INSERT INTO messages_fts(rowid, subject, sender, recipients, preview)
                    VALUES (new.id, new.subject, new.sender, new.recipients, new.preview);
                END;

                CREATE VIRTUAL TABLE IF NOT EXISTS messages_index_fts USING fts5(
                    index_text,
                    content='',
                    contentless_delete=1,
                    tokenize='unicode61 remove_diacritics 2'
                );

                CREATE TRIGGER IF NOT EXISTS messages_index_ad AFTER DELETE ON messages BEGIN
                    DELETE FROM messages_index_fts WHERE rowid = old.id;
                END;
                ",
            )
            .map_err(|e| format!("初始化账号 FTS5 索引失败: {e}"))?;

        let schema_version: i32 = self
            .mail_conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap_or(0);
        if schema_version < 1 {
            self.mail_conn
                .execute(
                    "INSERT INTO messages_fts(messages_fts) VALUES('rebuild')",
                    [],
                )
                .map_err(|e| format!("重建账号 FTS5 索引失败: {e}"))?;
            self.mail_conn
                .execute("PRAGMA user_version = 1", [])
                .map_err(|e| format!("更新账号索引库版本失败: {e}"))?;
        }
        Ok(())
    }

    pub(crate) fn ensure_content_conn(&self) -> Result<Option<RefMut<'_, Connection>>, String> {
        if self.content_conn.borrow().is_none() {
            let conn = match (&self.dir, self.read_only) {
                (None, true) => return Ok(None),
                (None, false) => {
                    let c = Connection::open_in_memory()
                        .map_err(|e| format!("打开内存正文库失败: {e}"))?;
                    Self::configure_content_db(&c, false)?;
                    c
                }
                (Some(dir), true) => {
                    let path = Self::content_db_path_in(dir);
                    if !path.exists() {
                        return Ok(None);
                    }
                    let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                        | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
                    let c = Connection::open_with_flags(&path, flags)
                        .map_err(|e| format!("只读打开正文库失败 ({}): {e}", path.display()))?;
                    c.execute_batch("PRAGMA busy_timeout = 2000; PRAGMA query_only = ON;")
                        .map_err(|e| format!("配置只读正文库失败: {e}"))?;
                    c
                }
                (Some(dir), false) => {
                    std::fs::create_dir_all(dir)
                        .map_err(|e| format!("创建正文库目录失败 ({}): {e}", dir.display()))?;
                    let path = Self::content_db_path_in(dir);
                    let c = Connection::open(&path)
                        .map_err(|e| format!("打开正文库失败 ({}): {e}", path.display()))?;
                    Self::configure_content_db(&c, true)?;
                    c
                }
            };
            *self.content_conn.borrow_mut() = Some(c_from(conn));
        }
        let guard = self.content_conn.borrow_mut();
        Ok(Some(RefMut::map(guard, |opt| opt.as_mut().unwrap())))
    }

    /// Open existing `content.db` without creating an empty database file when absent.
    /// Respects `self.read_only` (read-write when writable, query-only when read-only).
    fn open_existing_content_conn(&self) -> Result<Option<RefMut<'_, Connection>>, String> {
        if self.content_conn.borrow().is_some() {
            let guard = self.content_conn.borrow_mut();
            return Ok(Some(RefMut::map(guard, |opt| opt.as_mut().unwrap())));
        }
        let Some(ref dir) = self.dir else {
            return Ok(None);
        };
        let path = Self::content_db_path_in(dir);
        if !path.exists() {
            return Ok(None);
        }
        self.ensure_content_conn()
    }

    fn configure_content_db(conn: &Connection, enable_wal: bool) -> Result<(), String> {
        if enable_wal {
            conn.execute_batch("PRAGMA journal_mode = WAL;")
                .map_err(|e| format!("启用正文库 WAL 失败: {e}"))?;
        }
        conn.execute_batch(
            "
            PRAGMA synchronous = NORMAL;
            PRAGMA busy_timeout = 5000;
            CREATE TABLE IF NOT EXISTS message_bodies (
                message_id INTEGER PRIMARY KEY,
                body_text TEXT NOT NULL DEFAULT '',
                body_html TEXT
            );
            ",
        )
        .map_err(|e| format!("初始化正文库表结构失败: {e}"))?;
        Ok(())
    }

    fn encode(&self, local_id: u32) -> Result<u32, String> {
        encode_scoped_id(self.slot, local_id)
    }

    pub(crate) fn sync_folders_with_flags(
        &mut self,
        remote_folders: &[(RemoteFolder, bool)],
    ) -> Result<Vec<FolderSummary>, String> {
        if remote_folders.is_empty() {
            return self.list_folders();
        }
        let tx = self
            .mail_conn
            .transaction()
            .map_err(|e| format!("开启文件夹同步事务失败: {e}"))?;

        for (folder, is_inbox) in remote_folders {
            tx.execute(
                "
                INSERT INTO folders (name, path, unread_count, total_count, is_inbox)
                VALUES (?1, ?2, ?3, ?4, ?5)
                ON CONFLICT(path) DO UPDATE SET
                    name = excluded.name,
                    unread_count = excluded.unread_count,
                    total_count = excluded.total_count,
                    is_inbox = excluded.is_inbox
                ",
                params![
                    folder.name,
                    folder.path,
                    folder.unread_count,
                    folder.total_count as i64,
                    *is_inbox as i32
                ],
            )
            .map_err(|e| format!("写入文件夹失败 ({}): {e}", folder.path))?;
        }

        let live_paths: HashSet<&str> = remote_folders
            .iter()
            .map(|(f, _)| f.path.as_str())
            .collect();
        let stale_ids: Vec<u32> = {
            let mut stmt = tx
                .prepare("SELECT id, path FROM folders")
                .map_err(|e| format!("查询本地文件夹失败: {e}"))?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|e| format!("遍历本地文件夹失败: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("收集本地文件夹失败: {e}"))?;
            rows.into_iter()
                .filter_map(|(id, path)| (!live_paths.contains(path.as_str())).then_some(id))
                .collect()
        };

        let mut deleted_msg_ids: Vec<u32> = Vec::new();
        let mut old_attachment_paths: Vec<String> = Vec::new();
        for stale_id in stale_ids {
            let mut stmt = tx
                .prepare("SELECT id FROM messages WHERE folder_id = ?1")
                .map_err(|e| format!("查询待清理文件夹邮件失败: {e}"))?;
            let ids = stmt
                .query_map(params![stale_id], |r| r.get::<_, u32>(0))
                .map_err(|e| format!("遍历待清理文件夹邮件失败: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("收集待清理文件夹邮件失败: {e}"))?;
            drop(stmt);
            deleted_msg_ids.extend(ids);
            old_attachment_paths.extend(collect_attachment_paths(
                &tx,
                "SELECT a.storage_path FROM attachments a
                 JOIN messages m ON m.id = a.message_id
                 WHERE m.folder_id = ?1 AND a.storage_path != ''",
                params![stale_id],
            )?);
            tx.execute("DELETE FROM folders WHERE id = ?1", params![stale_id])
                .map_err(|e| format!("清理已删除远端文件夹缓存失败: {e}"))?;
        }

        tx.commit()
            .map_err(|e| format!("提交文件夹事务失败: {e}"))?;

        if !deleted_msg_ids.is_empty() {
            self.delete_content_bodies_for_ids(&deleted_msg_ids);
        }
        if !old_attachment_paths.is_empty() {
            delete_attachment_files(self.attachments_dir.as_deref(), &old_attachment_paths);
        }
        self.list_folders()
    }

    pub(crate) fn rename_folder_entry(
        &mut self,
        local_folder_id: u32,
        old_path: &str,
        new_path: &str,
        new_name: &str,
        delimiter: Option<&str>,
    ) -> Result<Vec<FolderSummary>, String> {
        let tx = self
            .mail_conn
            .transaction()
            .map_err(|e| format!("创建文件夹事务失败: {e}"))?;
        if let Some(delimiter) = delimiter.filter(|d| !d.is_empty()) {
            let prefix = format!("{old_path}{delimiter}");
            let descendants = {
                let mut stmt = tx
                    .prepare("SELECT id, path FROM folders")
                    .map_err(|e| format!("查询文件夹失败: {e}"))?;
                let rows = stmt
                    .query_map([], |row| {
                        Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?))
                    })
                    .map_err(|e| format!("查询文件夹失败: {e}"))?;
                rows.collect::<Result<Vec<_>, _>>()
                    .map_err(|e| format!("读取文件夹失败: {e}"))?
            };
            for (id, path) in descendants {
                if let Some(suffix) = path.strip_prefix(&prefix) {
                    tx.execute(
                        "UPDATE folders SET path = ?1 WHERE id = ?2",
                        params![format!("{new_path}{delimiter}{suffix}"), id],
                    )
                    .map_err(|e| format!("更新子文件夹失败: {e}"))?;
                }
            }
        }
        tx.execute(
            "UPDATE folders SET path = ?1, name = ?2 WHERE id = ?3",
            params![new_path, new_name, local_folder_id],
        )
        .map_err(|e| format!("更新文件夹名称失败: {e}"))?;
        tx.commit()
            .map_err(|e| format!("提交文件夹事务失败: {e}"))?;
        self.list_folders()
    }

    pub(crate) fn delete_folder_entry(
        &mut self,
        local_folder_id: u32,
    ) -> Result<Vec<FolderSummary>, String> {
        let tx = self
            .mail_conn
            .transaction()
            .map_err(|e| format!("开启删除文件夹缓存事务失败: {e}"))?;
        let deleted_msg_ids: Vec<u32> = {
            let mut stmt = tx
                .prepare("SELECT id FROM messages WHERE folder_id = ?1")
                .map_err(|e| format!("查询待删邮件失败: {e}"))?;
            let collected = stmt
                .query_map(params![local_folder_id], |r| r.get::<_, u32>(0))
                .map_err(|e| format!("遍历待删邮件失败: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("收集待删邮件失败: {e}"))?;
            collected
        };
        let old_paths = collect_attachment_paths(
            &tx,
            "SELECT a.storage_path FROM attachments a
             JOIN messages m ON m.id = a.message_id
             WHERE m.folder_id = ?1 AND a.storage_path != ''",
            params![local_folder_id],
        )?;
        tx.execute(
            "DELETE FROM folders WHERE id = ?1",
            params![local_folder_id],
        )
        .map_err(|e| format!("删除本地文件夹缓存失败: {e}"))?;
        tx.commit()
            .map_err(|e| format!("提交删除文件夹缓存事务失败: {e}"))?;

        self.delete_content_bodies_for_ids(&deleted_msg_ids);
        delete_attachment_files(self.attachments_dir.as_deref(), &old_paths);
        self.list_folders()
    }

    pub(crate) fn list_folders(&self) -> Result<Vec<FolderSummary>, String> {
        let mut stmt = self
            .mail_conn
            .prepare(
                "
                SELECT id, name, path, unread_count, total_count, has_more_remote
                FROM folders
                ORDER BY CASE WHEN UPPER(path) = 'INBOX' OR is_inbox = 1 THEN 0 ELSE 1 END, id ASC
                ",
            )
            .map_err(|e| format!("查询文件夹失败: {e}"))?;

        let raw_rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, u32>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, u32>(3)?,
                    row.get::<_, i64>(4)?.max(0) as usize,
                    row.get::<_, i32>(5)? != 0,
                ))
            })
            .map_err(|e| format!("读取文件夹列表失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析文件夹行失败: {e}"))?;

        let mut out = Vec::with_capacity(raw_rows.len());
        for (local_id, name, path, unread_count, total_count, has_more_remote) in raw_rows {
            out.push(FolderSummary {
                id: self.encode(local_id)?,
                account_id: self.account_id,
                name,
                path,
                unread_count,
                total_count,
                has_more_remote,
            });
        }
        Ok(out)
    }

    pub(crate) fn folder_remote_cursor(
        &self,
        local_folder_id: u32,
    ) -> Result<Option<(u32, String, bool, String, Option<u32>, usize)>, String> {
        let row = self
            .mail_conn
            .query_row(
                "
                SELECT
                    f.path,
                    f.has_more_remote,
                    f.remote_next_link,
                    (SELECT MIN(m.uid) FROM messages m WHERE m.folder_id = f.id),
                    (SELECT COUNT(*) FROM messages m WHERE m.folder_id = f.id)
                FROM folders f
                WHERE f.id = ?1
                ",
                params![local_folder_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i32>(1)? != 0,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<u32>>(3)?,
                        row.get::<_, i64>(4)?.max(0) as usize,
                    ))
                },
            )
            .optional()
            .map_err(|e| format!("读取文件夹增量游标失败: {e}"))?;
        Ok(row.map(|(path, has_more, next_link, min_uid, count)| {
            (self.account_id, path, has_more, next_link, min_uid, count)
        }))
    }

    pub(crate) fn list_inbox_folders_with_more_remote(
        &self,
    ) -> Result<Vec<(u32, u32, String)>, String> {
        let mut stmt = self
            .mail_conn
            .prepare(
                "
                SELECT id, path
                FROM folders
                WHERE (UPPER(path) = 'INBOX' OR is_inbox = 1) AND has_more_remote = 1
                ORDER BY id ASC
                ",
            )
            .map_err(|e| format!("查询待加载历史邮件的收件箱失败: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("遍历待加载历史邮件的收件箱失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析待加载历史邮件的收件箱失败: {e}"))?;
        let mut out = Vec::with_capacity(rows.len());
        for (local_fid, path) in rows {
            out.push((self.encode(local_fid)?, self.account_id, path));
        }
        Ok(out)
    }

    pub(crate) fn list_backfill_folders(&self) -> Result<Vec<(u32, u32, String, bool)>, String> {
        let mut stmt = self
            .mail_conn
            .prepare(&format!(
                "SELECT f.id, f.path, f.uid_validity = 0
                 FROM folders f
                 WHERE (
                     f.has_more_remote = 1 OR f.uid_validity = 0 OR EXISTS (
                         SELECT 1 FROM messages m
                         WHERE m.folder_id = f.id AND m.body_loaded = 0 AND m.index_text IS NULL
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
                Ok((
                    row.get::<_, u32>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, bool>(2)?,
                ))
            })
            .map_err(|e| format!("读取后台索引文件夹失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析后台索引文件夹失败: {e}"))?;
        let mut out = Vec::with_capacity(rows.len());
        for (local_fid, path, needs_sync) in rows {
            out.push((self.encode(local_fid)?, self.account_id, path, needs_sync));
        }
        Ok(out)
    }

    pub(crate) fn list_unindexed_messages(
        &self,
        local_folder_id: u32,
        limit: usize,
    ) -> Result<Vec<(u32, u32, String)>, String> {
        let mut stmt = self
            .mail_conn
            .prepare(
                "SELECT id, uid, remote_id FROM messages
                 WHERE folder_id = ?1 AND body_loaded = 0 AND index_text IS NULL
                 ORDER BY id LIMIT ?2",
            )
            .map_err(|e| format!("查询待索引邮件失败: {e}"))?;
        let rows = stmt
            .query_map(params![local_folder_id, limit as i64], |row| {
                Ok((
                    row.get::<_, u32>(0)?,
                    row.get::<_, u32>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| format!("读取待索引邮件失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析待索引邮件失败: {e}"))?;
        let mut out = Vec::with_capacity(rows.len());
        for (local_msg_id, uid, remote_id) in rows {
            out.push((self.encode(local_msg_id)?, uid, remote_id));
        }
        Ok(out)
    }

    pub(crate) fn store_index_text(&self, local_msg_id: u32, text: &str) -> Result<(), String> {
        let updated = self
            .mail_conn
            .execute(
                "UPDATE messages SET index_text = ?1
                 WHERE id = ?2 AND body_loaded = 0 AND index_text IS NULL",
                params![text, local_msg_id],
            )
            .map_err(|e| format!("写入邮件搜索正文失败: {e}"))?;
        if updated > 0 && !text.is_empty() {
            let _ = self.mail_conn.execute(
                "DELETE FROM messages_index_fts WHERE rowid = ?1",
                params![local_msg_id],
            );
            self.mail_conn
                .execute(
                    "INSERT INTO messages_index_fts(rowid, index_text) VALUES (?1, ?2)",
                    params![local_msg_id, text],
                )
                .map_err(|e| format!("更新 FTS 搜索正文失败: {e}"))?;
        }
        Ok(())
    }

    pub(crate) fn set_folder_remote_cursor(
        &self,
        local_folder_id: u32,
        has_more_remote: bool,
        remote_next_link: &str,
    ) -> Result<(), String> {
        self.mail_conn
            .execute(
                "
                UPDATE folders
                SET has_more_remote = ?1, remote_next_link = ?2
                WHERE id = ?3
                ",
                params![has_more_remote as i32, remote_next_link, local_folder_id],
            )
            .map_err(|e| format!("更新文件夹远程游标失败: {e}"))?;
        Ok(())
    }

    pub(crate) fn folder_meta(
        &self,
        local_folder_id: u32,
    ) -> Result<Option<(String, u32)>, String> {
        self.mail_conn
            .query_row(
                "SELECT path, uid_validity FROM folders WHERE id = ?1",
                params![local_folder_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?)),
            )
            .optional()
            .map_err(|e| format!("读取文件夹元数据失败: {e}"))
    }

    pub(crate) fn commit_folder_headers_with_remote(
        &mut self,
        local_folder_id: u32,
        uid_validity: u32,
        headers: &[(CachedHeaderInput, String)],
        prune_missing_uids: Option<&[u32]>,
    ) -> Result<(), String> {
        let tx = self
            .mail_conn
            .transaction()
            .map_err(|e| format!("开启邮件同步事务失败: {e}"))?;

        let mut deleted_msg_ids: Vec<u32> = Vec::new();
        let mut old_attachment_paths: Vec<String> = Vec::new();
        let previous_validity: u32 = tx
            .query_row(
                "SELECT uid_validity FROM folders WHERE id = ?1",
                params![local_folder_id],
                |row| row.get(0),
            )
            .map_err(|e| format!("读取文件夹 UIDVALIDITY 失败: {e}"))?;

        if previous_validity != 0 && previous_validity != uid_validity {
            let mut stmt = tx
                .prepare("SELECT id FROM messages WHERE folder_id = ?1")
                .map_err(|e| format!("读取旧纪元邮件失败: {e}"))?;
            let ids = stmt
                .query_map(params![local_folder_id], |r| r.get::<_, u32>(0))
                .map_err(|e| format!("遍历旧纪元邮件失败: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("收集旧纪元邮件失败: {e}"))?;
            deleted_msg_ids.extend(ids);
            drop(stmt);

            old_attachment_paths.extend(collect_attachment_paths(
                &tx,
                "SELECT a.storage_path FROM attachments a
                 JOIN messages m ON m.id = a.message_id
                 WHERE m.folder_id = ?1 AND a.storage_path != ''",
                params![local_folder_id],
            )?);
            tx.execute(
                "DELETE FROM messages WHERE folder_id = ?1",
                params![local_folder_id],
            )
            .map_err(|e| format!("UIDVALIDITY 变化后清理旧消息失败: {e}"))?;
        }

        if let Some(live_uids) = prune_missing_uids {
            let mut stmt = tx
                .prepare("SELECT id, uid FROM messages WHERE folder_id = ?1")
                .map_err(|e| format!("读取现有 UID 失败: {e}"))?;
            let existing: Vec<(u32, u32)> = stmt
                .query_map(params![local_folder_id], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })
                .map_err(|e| format!("遍历现有 UID 失败: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("收集现有 UID 失败: {e}"))?;
            drop(stmt);

            let live_set: HashSet<u32> = live_uids.iter().copied().collect();
            for (msg_id, uid) in existing {
                if !live_set.contains(&uid) {
                    deleted_msg_ids.push(msg_id);
                    old_attachment_paths.extend(collect_attachment_paths(
                        &tx,
                        "SELECT storage_path FROM attachments WHERE message_id = ?1 AND storage_path != ''",
                        params![msg_id],
                    )?);
                    tx.execute("DELETE FROM messages WHERE id = ?1", params![msg_id])
                        .map_err(|e| format!("清理服务端已删除邮件失败: {e}"))?;
                }
            }
        }

        for (item, remote_id) in headers {
            let thread_subj = normalize_thread_subject(&item.subject);
            tx.execute(
                "
                INSERT INTO messages (
                    folder_id, uid, remote_id, message_id, in_reply_to, references_header,
                    sender, recipients, cc, subject, thread_subject, preview, date_ts, date_display,
                    unread, flagged, has_attachment
                )
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
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
                    cc = excluded.cc,
                    subject = excluded.subject,
                    thread_subject = excluded.thread_subject,
                    preview = CASE
                        WHEN messages.body_loaded = 0 THEN excluded.preview
                        ELSE messages.preview
                    END,
                    date_ts = excluded.date_ts,
                    date_display = excluded.date_display,
                    unread = excluded.unread,
                    flagged = excluded.flagged,
                    has_attachment = excluded.has_attachment
                ",
                params![
                    local_folder_id,
                    item.uid,
                    remote_id,
                    item.message_id,
                    item.in_reply_to,
                    item.references_header,
                    item.sender,
                    item.recipients,
                    item.cc,
                    item.subject,
                    thread_subj,
                    item.preview,
                    item.date_ts,
                    item.date_display,
                    item.unread as i32,
                    item.flagged as i32,
                    item.has_attachment as i32,
                ],
            )
            .map_err(|e| format!("写入邮件头失败 (UID {}): {e}", item.uid))?;

            let (msg_row_id, stored_mid, stored_irt, stored_refs): (u32, String, String, String) =
                tx.query_row(
                    "SELECT id, message_id, in_reply_to, references_header
                     FROM messages WHERE folder_id = ?1 AND uid = ?2",
                    params![local_folder_id, item.uid],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .map_err(|e| format!("读取写入后的邮件索引失败: {e}"))?;
            sync_message_rfc_refs(&tx, msg_row_id, &stored_mid, &stored_irt, &stored_refs)?;
        }

        let unread_count: u32 = tx
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE folder_id = ?1 AND unread = 1",
                params![local_folder_id],
                |row| row.get(0),
            )
            .map_err(|e| format!("统计未读数失败: {e}"))?;
        let total_count: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE folder_id = ?1",
                params![local_folder_id],
                |row| row.get(0),
            )
            .map_err(|e| format!("统计总数失败: {e}"))?;

        tx.execute(
            "
            UPDATE folders
            SET uid_validity = ?1, unread_count = ?2, total_count = ?3
            WHERE id = ?4
            ",
            params![uid_validity, unread_count, total_count, local_folder_id],
        )
        .map_err(|e| format!("更新文件夹同步进度失败: {e}"))?;

        tx.commit()
            .map_err(|e| format!("提交邮件同步事务失败: {e}"))?;

        self.delete_content_bodies_for_ids(&deleted_msg_ids);
        delete_attachment_files(self.attachments_dir.as_deref(), &old_attachment_paths);
        Ok(())
    }

    pub(crate) fn reconcile_folder_flags(
        &mut self,
        local_folder_id: u32,
        flagged_uids: &HashSet<u32>,
        unseen_uids: &HashSet<u32>,
    ) -> Result<usize, String> {
        let tx = self
            .mail_conn
            .transaction()
            .map_err(|e| format!("开启状态位对账事务失败: {e}"))?;

        let rows: Vec<(u32, u32, bool, bool)> = {
            let mut stmt = tx
                .prepare("SELECT id, uid, unread, flagged FROM messages WHERE folder_id = ?1")
                .map_err(|e| format!("读取本地状态位失败: {e}"))?;
            let collected = stmt
                .query_map(params![local_folder_id], |row| {
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
                    params![local_folder_id],
                    |row| row.get(0),
                )
                .map_err(|e| format!("统计未读数失败: {e}"))?;
            tx.execute(
                "UPDATE folders SET unread_count = ?1 WHERE id = ?2",
                params![unread_count, local_folder_id],
            )
            .map_err(|e| format!("更新文件夹未读数失败: {e}"))?;
        }

        tx.commit()
            .map_err(|e| format!("提交状态位对账事务失败: {e}"))?;
        Ok(updated)
    }

    pub(crate) fn fts5_term(term: &str) -> Option<String> {
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

    pub(crate) fn sort_sql_prefixed(sort: MessageSort) -> &'static str {
        match sort {
            MessageSort::DateNewest => "m.date_ts DESC, m.uid DESC",
            MessageSort::DateOldest => "m.date_ts ASC, m.uid ASC",
            MessageSort::SenderAsc => "LOWER(m.sender) ASC, m.date_ts DESC, m.uid DESC",
            MessageSort::SenderDesc => "LOWER(m.sender) DESC, m.date_ts DESC, m.uid DESC",
            MessageSort::SubjectAsc => "LOWER(m.subject) ASC, m.date_ts DESC, m.uid DESC",
            MessageSort::SubjectDesc => "LOWER(m.subject) DESC, m.date_ts DESC, m.uid DESC",
            MessageSort::UnreadFirst => "m.unread DESC, m.date_ts DESC, m.uid DESC",
            MessageSort::StarredFirst => "m.flagged DESC, m.date_ts DESC, m.uid DESC",
            MessageSort::AccountAsc | MessageSort::AccountDesc => "m.date_ts DESC, m.uid DESC",
            MessageSort::AttachmentFirst => "m.has_attachment DESC, m.date_ts DESC, m.uid DESC",
            MessageSort::ImportanceFirst => "m.importance DESC, m.date_ts DESC, m.uid DESC",
            MessageSort::TagsAsc => "LOWER(m.tags) ASC, m.date_ts DESC, m.uid DESC",
        }
    }

    pub(crate) fn sort_sql_bare(sort: MessageSort) -> &'static str {
        match sort {
            MessageSort::DateNewest => "date_ts DESC, uid DESC",
            MessageSort::DateOldest => "date_ts ASC, uid ASC",
            MessageSort::SenderAsc => "LOWER(sender) ASC, date_ts DESC, uid DESC",
            MessageSort::SenderDesc => "LOWER(sender) DESC, date_ts DESC, uid DESC",
            MessageSort::SubjectAsc => "LOWER(subject) ASC, date_ts DESC, uid DESC",
            MessageSort::SubjectDesc => "LOWER(subject) DESC, date_ts DESC, uid DESC",
            MessageSort::UnreadFirst => "unread DESC, date_ts DESC, uid DESC",
            MessageSort::StarredFirst => "flagged DESC, date_ts DESC, uid DESC",
            MessageSort::AccountAsc | MessageSort::AccountDesc => "date_ts DESC, uid DESC",
            MessageSort::AttachmentFirst => "has_attachment DESC, date_ts DESC, uid DESC",
            MessageSort::ImportanceFirst => "importance DESC, date_ts DESC, uid DESC",
            MessageSort::TagsAsc => "LOWER(tags) ASC, date_ts DESC, uid DESC",
        }
    }

    /// Find local `message_id`s in `content.db` matching `LOWER(body_text) LIKE %term%`
    /// (only invoked for non-ASCII / punctuation LIKE search terms).
    fn find_content_body_hits(&self, term_lower: &str) -> Result<Vec<u32>, String> {
        let Some(conn) = self.open_existing_content_conn()? else {
            return Ok(Vec::new());
        };
        let pattern = format!("%{term_lower}%");
        let mut stmt = conn
            .prepare("SELECT message_id FROM message_bodies WHERE LOWER(body_text) LIKE ?1")
            .map_err(|e| format!("查询正文检索条件失败: {e}"))?;
        let rows = stmt
            .query_map(params![pattern], |row| row.get::<_, u32>(0))
            .map_err(|e| format!("遍历正文检索结果失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("收集正文检索结果失败: {e}"))?;
        Ok(rows)
    }

    pub(crate) fn append_search_and_filter_clauses(
        &self,
        parsed: &SearchQuery,
        quick_filters: QuickFilterSet,
        today_start_ts: i64,
        own_emails: &[String],
        contact_emails_lower: &[String],
        clauses: &mut Vec<String>,
        args: &mut Vec<rusqlite::types::Value>,
    ) -> Result<(), String> {
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
            if contact_emails_lower.is_empty() {
                clauses.push("1 = 0".to_string());
            } else {
                let parts: Vec<&str> = contact_emails_lower
                    .iter()
                    .map(|_| "LOWER(m.sender) LIKE ?")
                    .collect();
                clauses.push(format!("({})", parts.join(" OR ")));
                for email in contact_emails_lower {
                    args.push(format!("%{email}%").into());
                }
            }
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
        for val in &parsed.tags {
            clauses.push(
                "(',' || REPLACE(LOWER(COALESCE(m.tags, '')), ', ', ',') || ',') LIKE ?"
                    .to_string(),
            );
            args.push(format!("%,{val},%").into());
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
                let body_hits = self.find_content_body_hits(&term.to_lowercase())?;
                if body_hits.is_empty() {
                    clauses.push(
                        "(LOWER(m.subject) LIKE ? OR LOWER(m.sender) LIKE ? OR LOWER(m.recipients) LIKE ? OR LOWER(m.preview) LIKE ? OR LOWER(COALESCE(m.index_text, '')) LIKE ?)"
                            .to_string(),
                    );
                } else {
                    let id_list = body_hits
                        .into_iter()
                        .map(|id| id.to_string())
                        .collect::<Vec<_>>()
                        .join(",");
                    clauses.push(format!(
                        "(LOWER(m.subject) LIKE ? OR LOWER(m.sender) LIKE ? OR LOWER(m.recipients) LIKE ? OR LOWER(m.preview) LIKE ? OR LOWER(COALESCE(m.index_text, '')) LIKE ? OR m.id IN ({id_list}))"
                    ));
                }
                let pat = format!("%{term}%");
                for _ in 0..5 {
                    args.push(pat.clone().into());
                }
            }
        }
        Ok(())
    }

    pub(crate) fn list_messages_page_with_options(
        &self,
        local_folder_id: u32,
        external_folder_id: u32,
        query: &str,
        quick_filters: QuickFilterSet,
        sort: MessageSort,
        own_emails: &[String],
        contact_emails_lower: &[String],
        offset: usize,
        limit: usize,
    ) -> Result<MailPage, String> {
        let parsed = SearchQuery::parse(query);
        let mut clauses = vec!["m.folder_id = ?".to_string()];
        let mut args: Vec<rusqlite::types::Value> = vec![i64::from(local_folder_id).into()];
        self.append_search_and_filter_clauses(
            &parsed,
            quick_filters,
            start_of_today_ts(),
            own_emails,
            contact_emails_lower,
            &mut clauses,
            &mut args,
        )?;
        let where_sql = clauses.join(" AND ");
        let order_sql = Self::sort_sql_prefixed(sort);

        let total: i64 = self
            .mail_conn
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
            .mail_conn
            .prepare(&format!(
                "
                SELECT m.id, m.folder_id, f.path, m.uid, m.sender, m.subject,
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
            .query_map(rusqlite::params_from_iter(page_args.iter()), |row| {
                self.map_local_summary_row(row)
            })
            .map_err(|e| format!("执行分页查询失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析分页结果失败: {e}"))?;
        self.fill_thread_counts(&mut items);

        Ok(MailPage {
            folder_id: external_folder_id,
            offset,
            total: total.max(0) as usize,
            items,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn query_role_candidates(
        &self,
        predicate: &str,
        label: &str,
        query: &str,
        quick_filters: QuickFilterSet,
        sort: MessageSort,
        own_emails: &[String],
        contact_emails_lower: &[String],
        fetch_limit: usize,
    ) -> Result<(usize, Vec<MessageSummary>), String> {
        let parsed = SearchQuery::parse(query);
        let mut clauses = vec![predicate.to_string()];
        let mut args: Vec<rusqlite::types::Value> = Vec::new();
        self.append_search_and_filter_clauses(
            &parsed,
            quick_filters,
            start_of_today_ts(),
            own_emails,
            contact_emails_lower,
            &mut clauses,
            &mut args,
        )?;
        let where_sql = clauses.join(" AND ");
        let order_sql = Self::sort_sql_prefixed(sort);

        let total: i64 = self
            .mail_conn
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

        if fetch_limit == 0 || total <= 0 {
            return Ok((total.max(0) as usize, Vec::new()));
        }

        let mut page_args = args;
        page_args.push((fetch_limit as i64).into());

        let mut stmt = self
            .mail_conn
            .prepare(&format!(
                "
                SELECT m.id, m.folder_id, f.path, m.uid, m.sender, m.subject,
                       m.preview, m.date_ts, m.date_display, m.unread, m.flagged, m.has_attachment,
                       m.recipients, m.importance, m.tags, m.answered, m.forwarded, m.snoozed_until_ts
                FROM messages m
                JOIN folders f ON f.id = m.folder_id
                WHERE {where_sql}
                ORDER BY {order_sql}
                LIMIT ?
                "
            ))
            .map_err(|e| format!("准备{label}分页查询失败: {e}"))?;

        let items = stmt
            .query_map(rusqlite::params_from_iter(page_args.iter()), |row| {
                self.map_local_summary_row(row)
            })
            .map_err(|e| format!("执行{label}分页查询失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析{label}结果失败: {e}"))?;

        Ok((total.max(0) as usize, items))
    }

    pub(crate) fn query_search_candidates(
        &self,
        parsed: &SearchQuery,
        quick_filters: QuickFilterSet,
        sort: MessageSort,
        own_emails: &[String],
        contact_emails_lower: &[String],
    ) -> Result<Vec<MergedMessageCandidate>, String> {
        let mut clauses: Vec<String> = Vec::new();
        let mut args: Vec<rusqlite::types::Value> = Vec::new();
        if parsed.folder.is_empty() {
            clauses.push(format!("NOT ({TRASH_FOLDER_MATCH} OR {JUNK_FOLDER_MATCH})"));
        }
        self.append_search_and_filter_clauses(
            parsed,
            quick_filters,
            start_of_today_ts(),
            own_emails,
            contact_emails_lower,
            &mut clauses,
            &mut args,
        )?;
        let where_sql = if clauses.is_empty() {
            "1 = 1".to_string()
        } else {
            clauses.join(" AND ")
        };
        let order_sql = Self::sort_sql_bare(sort);
        let sql = format!(
            "
            WITH ranked AS (
                SELECT m.id, m.folder_id, f.path AS folder_path, m.uid, m.sender, m.subject,
                       m.preview, m.date_ts, m.date_display, m.unread, m.flagged, m.has_attachment,
                       m.recipients, m.importance, m.tags, m.answered, m.forwarded, m.snoozed_until_ts,
                       m.message_id,
                       CASE
                           WHEN UPPER(f.path) = 'INBOX' OR f.is_inbox = 1 THEN 0
                           WHEN {TRASH_FOLDER_MATCH} OR {JUNK_FOLDER_MATCH} THEN 3
                           WHEN {ARCHIVE_FOLDER_MATCH} THEN 2
                           ELSE 1
                       END AS folder_prio,
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
            SELECT id, folder_id, folder_path, uid, sender, subject,
                   preview, date_ts, date_display, unread, flagged, has_attachment,
                   recipients, importance, tags, answered, forwarded, snoozed_until_ts,
                   message_id, folder_prio
            FROM ranked
            WHERE rn = 1
            ORDER BY {order_sql}
            "
        );
        let mut stmt = self
            .mail_conn
            .prepare(&sql)
            .map_err(|e| format!("准备跨文件夹搜索查询失败: {e}"))?;

        let rows = stmt
            .query_map(rusqlite::params_from_iter(args.iter()), |row| {
                let summary = self.map_local_summary_row(row)?;
                let rfc_message_id: String = row.get(18)?;
                let folder_prio: i64 = row.get(19)?;
                Ok(MergedMessageCandidate {
                    summary,
                    rfc_message_id,
                    folder_priority: folder_prio.clamp(0, 3) as u8,
                })
            })
            .map_err(|e| format!("执行跨文件夹搜索查询失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析跨文件夹搜索结果失败: {e}"))?;
        Ok(rows)
    }

    fn load_trash_or_junk_folders(&self) -> Result<HashMap<u32, bool>, String> {
        let mut stmt = self
            .mail_conn
            .prepare("SELECT id, name, path FROM folders")
            .map_err(|e| format!("查询文件夹属性失败: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                let id: u32 = row.get(0)?;
                let name: String = row.get(1)?;
                let path: String = row.get(2)?;
                Ok((
                    id,
                    is_trash_folder(&name, &path) || is_junk_folder(&name, &path),
                ))
            })
            .map_err(|e| format!("读取文件夹属性失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析文件夹属性失败: {e}"))?;
        Ok(rows.into_iter().collect())
    }

    fn load_thread_count_rows(&self, local_ids: &[u32]) -> Result<Vec<ThreadCountRow>, String> {
        let mut out = Vec::new();
        for chunk in local_ids.chunks(200) {
            let placeholders = vec!["?"; chunk.len()].join(", ");
            let sql = format!(
                "SELECT id, folder_id, message_id, sender, date_ts
                 FROM messages
                 WHERE id IN ({placeholders})"
            );
            let args: Vec<rusqlite::types::Value> =
                chunk.iter().map(|&id| i64::from(id).into()).collect();
            let mut stmt = self
                .mail_conn
                .prepare(&sql)
                .map_err(|e| format!("准备会话计数行查询失败: {e}"))?;
            let rows = stmt
                .query_map(rusqlite::params_from_iter(args.iter()), |row| {
                    Ok(ThreadCountRow {
                        id: row.get(0)?,
                        folder_id: row.get(1)?,
                        rfc_message_id: row.get(2)?,
                        sender: row.get(3)?,
                        date_ts: row.get(4)?,
                    })
                })
                .map_err(|e| format!("读取会话计数行失败: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("解析会话计数行失败: {e}"))?;
            out.extend(rows);
        }
        Ok(out)
    }

    fn bfs_rfc_thread_ids(
        &self,
        local_anchor_id: u32,
        anchor_exists: bool,
        seed_ids: HashSet<String>,
    ) -> Result<(HashSet<u32>, bool), String> {
        let mut matched_ids: HashSet<u32> = HashSet::new();
        let mut complete = true;
        if !seed_ids.is_empty() {
            let mut frontier: Vec<String> = seed_ids.iter().cloned().collect();
            let mut seen_tokens: HashSet<String> = seed_ids;

            for hop in 0..MAX_THREAD_BFS_HOPS {
                if frontier.is_empty() || matched_ids.len() >= MAX_THREAD_BFS_ROWS {
                    if matched_ids.len() >= MAX_THREAD_BFS_ROWS {
                        complete = false;
                    }
                    break;
                }
                let mut new_rows: Vec<u32> = Vec::new();
                for chunk in frontier.chunks(200) {
                    let placeholders = vec!["?"; chunk.len()].join(", ");
                    let sql = format!(
                        "SELECT DISTINCT message_row_id
                         FROM message_rfc_refs
                         WHERE token IN ({placeholders})"
                    );
                    let args: Vec<rusqlite::types::Value> =
                        chunk.iter().map(|tok| tok.clone().into()).collect();
                    let mut stmt = self
                        .mail_conn
                        .prepare(&sql)
                        .map_err(|e| format!("准备 RFC 会话索引查询失败: {e}"))?;
                    let rows = stmt
                        .query_map(rusqlite::params_from_iter(args.iter()), |row| {
                            row.get::<_, u32>(0)
                        })
                        .map_err(|e| format!("执行 RFC 会话索引查询失败: {e}"))?;
                    for row_res in rows {
                        let rid = row_res.map_err(|e| format!("解析 RFC 会话索引失败: {e}"))?;
                        if matched_ids.insert(rid) {
                            new_rows.push(rid);
                            if matched_ids.len() >= MAX_THREAD_BFS_ROWS {
                                complete = false;
                                break;
                            }
                        }
                    }
                    if matched_ids.len() >= MAX_THREAD_BFS_ROWS {
                        complete = false;
                        break;
                    }
                }
                if new_rows.is_empty() || matched_ids.len() >= MAX_THREAD_BFS_ROWS {
                    if matched_ids.len() >= MAX_THREAD_BFS_ROWS {
                        complete = false;
                    }
                    break;
                }
                let mut next_frontier: Vec<String> = Vec::new();
                for chunk in new_rows.chunks(200) {
                    let placeholders = vec!["?"; chunk.len()].join(", ");
                    let sql = format!(
                        "SELECT token FROM message_rfc_refs WHERE message_row_id IN ({placeholders})"
                    );
                    let args: Vec<rusqlite::types::Value> =
                        chunk.iter().map(|&id| i64::from(id).into()).collect();
                    let mut stmt = self
                        .mail_conn
                        .prepare(&sql)
                        .map_err(|e| format!("准备 RFC 会话关联查询失败: {e}"))?;
                    let rows = stmt
                        .query_map(rusqlite::params_from_iter(args.iter()), |row| {
                            row.get::<_, String>(0)
                        })
                        .map_err(|e| format!("执行 RFC 会话关联查询失败: {e}"))?;
                    for tok_res in rows {
                        let tok = tok_res.map_err(|e| format!("解析 RFC 会话关联失败: {e}"))?;
                        if seen_tokens.insert(tok.clone()) {
                            next_frontier.push(tok);
                        }
                    }
                }
                frontier = next_frontier;
                if hop + 1 == MAX_THREAD_BFS_HOPS && !frontier.is_empty() {
                    complete = false;
                }
            }
            if local_anchor_id != 0 && anchor_exists {
                matched_ids.insert(local_anchor_id);
            }
        }
        Ok((matched_ids, complete))
    }

    pub(crate) fn fill_thread_counts(&self, items: &mut [MessageSummary]) {
        if items.is_empty() {
            return;
        }
        for m in items.iter_mut() {
            m.thread_count = 1;
        }

        let item_local_ids: Vec<Option<u32>> = items
            .iter()
            .map(|m| decode_scoped_id(m.id).map(|(_, lid)| lid))
            .collect();
        let mut local_ids: Vec<u32> = item_local_ids.iter().flatten().copied().collect();
        if local_ids.is_empty() {
            return;
        }
        local_ids.sort_unstable();
        local_ids.dedup();

        let Ok(trash_or_junk_folders) = self.load_trash_or_junk_folders() else {
            return;
        };

        let mut anchors: HashMap<u32, AnchorThreadMeta> = HashMap::with_capacity(local_ids.len());
        for chunk in local_ids.chunks(200) {
            let placeholders = vec!["?"; chunk.len()].join(", ");
            let sql = format!(
                "SELECT id, folder_id, message_id, in_reply_to, references_header, sender, date_ts, thread_subject
                 FROM messages
                 WHERE id IN ({placeholders})"
            );
            let args: Vec<rusqlite::types::Value> =
                chunk.iter().map(|&id| i64::from(id).into()).collect();
            let Ok(mut stmt) = self.mail_conn.prepare(&sql) else {
                continue;
            };
            let Ok(rows) = stmt
                .query_map(rusqlite::params_from_iter(args.iter()), |row| {
                    Ok(AnchorThreadMeta {
                        id: row.get(0)?,
                        folder_id: row.get(1)?,
                        rfc_message_id: row.get(2)?,
                        in_reply_to: row.get(3)?,
                        references_header: row.get(4)?,
                        sender: row.get(5)?,
                        date_ts: row.get(6)?,
                        thread_subject: row.get(7)?,
                    })
                })
                .map(|iter| iter.collect::<Result<Vec<_>, _>>())
            else {
                continue;
            };
            if let Ok(rows) = rows {
                for meta in rows {
                    anchors.insert(meta.id, meta);
                }
            }
        }
        if anchors.is_empty() {
            return;
        }

        let mut anchor_seeds: HashMap<u32, (HashSet<String>, bool)> =
            HashMap::with_capacity(anchors.len());
        let mut all_tokens_set: HashSet<String> = HashSet::new();
        for (&local_id, meta) in &anchors {
            let (seeds, has_reply) = collect_anchor_rfc_seeds(
                &meta.rfc_message_id,
                &meta.in_reply_to,
                &meta.references_header,
            );
            for tok in &seeds {
                all_tokens_set.insert(tok.clone());
            }
            anchor_seeds.insert(local_id, (seeds, has_reply));
        }

        let mut token_to_rows: HashMap<String, Vec<u32>> = HashMap::new();
        if !all_tokens_set.is_empty() {
            let all_tokens: Vec<String> = all_tokens_set.into_iter().collect();
            for chunk in all_tokens.chunks(200) {
                let placeholders = vec!["?"; chunk.len()].join(", ");
                let sql = format!(
                    "SELECT token, message_row_id
                     FROM message_rfc_refs
                     WHERE token IN ({placeholders})"
                );
                let args: Vec<rusqlite::types::Value> =
                    chunk.iter().map(|tok| tok.clone().into()).collect();
                let Ok(mut stmt) = self.mail_conn.prepare(&sql) else {
                    continue;
                };
                let Ok(rows) = stmt
                    .query_map(rusqlite::params_from_iter(args.iter()), |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
                    })
                    .map(|iter| iter.collect::<Result<Vec<_>, _>>())
                else {
                    continue;
                };
                if let Ok(rows) = rows {
                    for (tok, rid) in rows {
                        token_to_rows.entry(tok).or_default().push(rid);
                    }
                }
            }
        }

        let mut rfc_count_cache: HashMap<(u32, bool), u32> = HashMap::new();
        let mut subject_fallback_items: HashMap<String, Vec<(usize, u32, bool)>> = HashMap::new();

        for (idx, item) in items.iter_mut().enumerate() {
            let Some(local_id) = item_local_ids[idx] else {
                continue;
            };
            let Some(meta) = anchors.get(&local_id) else {
                continue;
            };
            let anchor_in_trash_or_junk = trash_or_junk_folders
                .get(&meta.folder_id)
                .copied()
                .unwrap_or(false);
            let Some((seeds, anchor_has_reply_headers)) = anchor_seeds.get(&local_id) else {
                continue;
            };
            let has_rfc_peer = seeds.iter().any(|tok| {
                token_to_rows
                    .get(tok)
                    .is_some_and(|rows| rows.iter().any(|&rid| rid != local_id))
            });

            if has_rfc_peer {
                if let Some(&cached) = rfc_count_cache.get(&(local_id, anchor_in_trash_or_junk)) {
                    item.thread_count = cached;
                    continue;
                }
                if let Ok((matched_ids, complete)) =
                    self.bfs_rfc_thread_ids(local_id, true, seeds.clone())
                {
                    let ids_vec: Vec<u32> = matched_ids.into_iter().collect();
                    if let Ok(rows) = self.load_thread_count_rows(&ids_vec) {
                        let count = count_distinct_thread_rows(
                            &rows,
                            None,
                            local_id,
                            anchor_in_trash_or_junk,
                            &trash_or_junk_folders,
                        );
                        item.thread_count = count;
                        rfc_count_cache.insert((local_id, anchor_in_trash_or_junk), count);
                        if complete {
                            for r in &rows {
                                let r_in_trash_or_junk = trash_or_junk_folders
                                    .get(&r.folder_id)
                                    .copied()
                                    .unwrap_or(false);
                                if r_in_trash_or_junk == anchor_in_trash_or_junk {
                                    rfc_count_cache.insert((r.id, anchor_in_trash_or_junk), count);
                                }
                            }
                        }
                    }
                }
            } else if *anchor_has_reply_headers {
                item.thread_count = 1;
            } else {
                let key = normalize_thread_subject(&item.subject);
                if key.is_empty() {
                    continue;
                }
                subject_fallback_items.entry(key).or_default().push((
                    idx,
                    local_id,
                    anchor_in_trash_or_junk,
                ));
            }
        }

        if subject_fallback_items.is_empty() {
            return;
        }

        let keys: Vec<String> = subject_fallback_items.keys().cloned().collect();
        let mut multi_keys: HashSet<String> = HashSet::new();
        for chunk in keys.chunks(200) {
            let placeholders = vec!["?"; chunk.len()].join(", ");
            let sql = format!(
                "SELECT thread_subject, COUNT(*)
                 FROM messages
                 WHERE thread_subject IN ({placeholders})
                 GROUP BY thread_subject"
            );
            let args: Vec<rusqlite::types::Value> =
                chunk.iter().map(|k| k.clone().into()).collect();
            let Ok(mut stmt) = self.mail_conn.prepare(&sql) else {
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
                for (key, count) in rows {
                    if count > 1 {
                        multi_keys.insert(key);
                    }
                }
            }
        }

        let Ok(mut subj_stmt) = self.mail_conn.prepare(&format!(
            "SELECT id, folder_id, message_id, sender, date_ts
             FROM messages
             WHERE thread_subject = ?1
             ORDER BY date_ts DESC, uid DESC
             LIMIT {MAX_THREAD_SUBJECT_ROWS}"
        )) else {
            return;
        };

        for (key, entries) in subject_fallback_items {
            if !multi_keys.contains(&key) {
                continue;
            }
            let Ok(rows) = subj_stmt
                .query_map(params![&key], |row| {
                    Ok(ThreadCountRow {
                        id: row.get(0)?,
                        folder_id: row.get(1)?,
                        rfc_message_id: row.get(2)?,
                        sender: row.get(3)?,
                        date_ts: row.get(4)?,
                    })
                })
                .map(|iter| iter.collect::<Result<Vec<_>, _>>())
            else {
                continue;
            };
            let Ok(rows) = rows else {
                continue;
            };
            let mut key_cache: [Option<u32>; 2] = [None, None];
            for (idx, local_id, anchor_in_trash_or_junk) in entries {
                let anchor_in_rows = rows.iter().any(|r| r.id == local_id);
                if anchor_in_rows {
                    let cache_slot = anchor_in_trash_or_junk as usize;
                    let count = match key_cache[cache_slot] {
                        Some(c) => c,
                        None => {
                            let c = count_distinct_thread_rows(
                                &rows,
                                None,
                                local_id,
                                anchor_in_trash_or_junk,
                                &trash_or_junk_folders,
                            );
                            key_cache[cache_slot] = Some(c);
                            c
                        }
                    };
                    items[idx].thread_count = count;
                } else {
                    let extra_anchor = anchors.get(&local_id).and_then(|meta| {
                        (meta.thread_subject == key).then(|| ThreadCountRow {
                            id: meta.id,
                            folder_id: meta.folder_id,
                            rfc_message_id: meta.rfc_message_id.clone(),
                            sender: meta.sender.clone(),
                            date_ts: meta.date_ts,
                        })
                    });
                    items[idx].thread_count = count_distinct_thread_rows(
                        &rows,
                        extra_anchor.as_ref(),
                        local_id,
                        anchor_in_trash_or_junk,
                        &trash_or_junk_folders,
                    );
                }
            }
        }
    }

    pub(crate) fn list_summaries_by_uids(
        &self,
        local_folder_id: u32,
        uids: &[u32],
    ) -> Result<Vec<MessageSummary>, String> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        let marks = vec!["?"; uids.len()].join(", ");
        let mut args: Vec<rusqlite::types::Value> = vec![i64::from(local_folder_id).into()];
        args.extend(uids.iter().map(|u| i64::from(*u).into()));
        let mut stmt = self
            .mail_conn
            .prepare(&format!(
                "
                SELECT m.id, m.folder_id, f.path, m.uid, m.sender, m.subject,
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
            .query_map(rusqlite::params_from_iter(args.iter()), |row| {
                self.map_local_summary_row(row)
            })
            .map_err(|e| format!("执行按 UID 查询邮件摘要失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析按 UID 查询邮件摘要失败: {e}"))?;
        self.fill_thread_counts(&mut rows);
        Ok(rows)
    }

    fn load_content_bodies_map(
        &self,
        local_msg_ids: &[u32],
    ) -> Result<HashMap<u32, (String, Option<String>)>, String> {
        let mut out = HashMap::new();
        if local_msg_ids.is_empty() {
            return Ok(out);
        }
        let Some(conn) = self.open_existing_content_conn()? else {
            return Ok(out);
        };
        for chunk in local_msg_ids.chunks(200) {
            let marks = vec!["?"; chunk.len()].join(", ");
            let sql = format!(
                "SELECT message_id, body_text, body_html FROM message_bodies WHERE message_id IN ({marks})"
            );
            let args: Vec<rusqlite::types::Value> =
                chunk.iter().map(|id| i64::from(*id).into()).collect();
            let mut stmt = conn
                .prepare(&sql)
                .map_err(|e| format!("查询正文内容失败: {e}"))?;
            let rows = stmt
                .query_map(rusqlite::params_from_iter(args.iter()), |row| {
                    Ok((
                        row.get::<_, u32>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                    ))
                })
                .map_err(|e| format!("读取正文内容失败: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("解析正文内容失败: {e}"))?;
            for (mid, text, html) in rows {
                out.insert(mid, (text, html));
            }
        }
        Ok(out)
    }

    pub(crate) fn get_message_detail_ordered(
        &self,
        local_msg_id: u32,
        newest_first: bool,
    ) -> Result<Option<(MessageDetail, u32, bool)>, String> {
        let row = self
            .mail_conn
            .query_row(
                "
                SELECT m.id, m.folder_id, m.uid, m.message_id, m.sender, m.recipients,
                       m.subject, m.date_ts, m.date_display, m.body_loaded, m.flagged, f.path,
                       m.list_unsubscribe, m.list_unsubscribe_post, m.auth_results, m.cc
                FROM messages m
                JOIN folders f ON f.id = m.folder_id
                WHERE m.id = ?1
                ",
                params![local_msg_id],
                |row| {
                    let unsub: String = row.get(12)?;
                    let auth_results: String = row.get(14)?;
                    Ok((
                        row.get::<_, u32>(0)?,
                        row.get::<_, u32>(1)?,
                        row.get::<_, u32>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, i32>(9)? != 0,
                        row.get::<_, i32>(10)? != 0,
                        row.get::<_, String>(11)?,
                        (!unsub.trim().is_empty()).then_some(unsub),
                        row.get::<_, i32>(13)? != 0,
                        (!auth_results.trim().is_empty()).then_some(auth_results),
                        row.get::<_, String>(15)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| format!("查询邮件详情失败: {e}"))?;

        let Some((
            local_id,
            local_folder_id,
            uid,
            rfc_message_id,
            sender,
            recipients,
            subject,
            date_ts,
            date_display,
            body_loaded,
            flagged,
            folder_path,
            unsubscribe_url,
            unsubscribe_one_click,
            auth_results,
            cc,
        )) = row
        else {
            return Ok(None);
        };

        let (body_text, body_html) = if body_loaded {
            let map = self.load_content_bodies_map(&[local_id])?;
            map.get(&local_id).cloned().unwrap_or_default()
        } else {
            (String::new(), None)
        };

        let attachments = self.list_attachments(local_id)?;
        let thread_messages = self
            .list_thread_messages(local_id, &subject, newest_first)
            .unwrap_or_default();
        let ext_id = self.encode(local_id)?;
        let ext_folder_id = self.encode(local_folder_id)?;
        let detail = MessageDetail {
            id: ext_id,
            account_id: self.account_id,
            folder_id: ext_folder_id,
            folder_path,
            uid,
            rfc_message_id,
            sender,
            recipients,
            cc,
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
            auth_results,
        };
        Ok(Some((detail, uid, body_loaded)))
    }

    pub(crate) fn find_role_folder(
        &self,
        predicate: &str,
    ) -> Result<Option<(u32, String)>, String> {
        let row = self
            .mail_conn
            .query_row(
                &format!(
                    "
                    SELECT f.id, f.path FROM folders f
                    WHERE {predicate}
                    ORDER BY f.id ASC LIMIT 1
                    "
                ),
                [],
                |row| Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|e| format!("查找角色文件夹失败: {e}"))?;
        match row {
            Some((local_fid, path)) => Ok(Some((self.encode(local_fid)?, path))),
            None => Ok(None),
        }
    }

    pub(crate) fn find_inbox_folder(&self) -> Result<Option<(u32, String)>, String> {
        self.find_role_folder("(UPPER(f.path) = 'INBOX' OR f.is_inbox = 1)")
    }

    pub(crate) fn find_sent_folder(&self) -> Result<Option<(u32, String)>, String> {
        self.find_role_folder(SENT_FOLDER_MATCH)
    }

    pub(crate) fn find_archive_folder(&self) -> Result<Option<(u32, String)>, String> {
        self.find_role_folder(ARCHIVE_FOLDER_MATCH)
    }

    pub(crate) fn find_trash_folder(&self) -> Result<Option<(u32, String)>, String> {
        self.find_role_folder(TRASH_FOLDER_MATCH)
    }

    pub(crate) fn find_spam_folder(&self) -> Result<Option<(u32, String)>, String> {
        self.find_role_folder(SPAM_FOLDER_MATCH)
    }

    pub(crate) fn list_thread_messages(
        &self,
        local_anchor_id: u32,
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
            cc: String,
            subject: String,
            date_ts: i64,
            date_display: String,
            body_loaded: bool,
            preview: String,
            unread: bool,
            flagged: bool,
        }

        let anchor_info: Option<(u32, String, String, String)> = if local_anchor_id != 0 {
            self.mail_conn
                .query_row(
                    "
                    SELECT folder_id, message_id, in_reply_to, references_header
                    FROM messages
                    WHERE id = ?1
                    ",
                    params![local_anchor_id],
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
        let (seed_ids, anchor_has_reply_headers) = match anchor_info {
            Some((_, ref mid, ref irt, ref refs)) => collect_anchor_rfc_seeds(mid, irt, refs),
            None => (HashSet::new(), false),
        };

        let map_raw_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<RawRow> {
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
                body_loaded: row.get::<_, i32>(13)? != 0,
                preview: row.get(14)?,
                unread: row.get::<_, i32>(15)? != 0,
                flagged: row.get::<_, i32>(16)? != 0,
                cc: row.get(17)?,
            })
        };

        let (matched_ids, _) =
            self.bfs_rfc_thread_ids(local_anchor_id, anchor_info.is_some(), seed_ids)?;

        let use_rfc =
            matched_ids.len() >= 2 || (anchor_has_reply_headers && !matched_ids.is_empty());

        let mut raw_rows: Vec<RawRow> = Vec::new();
        if use_rfc {
            let ids_vec: Vec<u32> = matched_ids.into_iter().collect();
            for chunk in ids_vec.chunks(200) {
                let placeholders = vec!["?"; chunk.len()].join(", ");
                let sql = format!(
                    "
                    SELECT m.id, m.folder_id, f.name, f.path, m.uid, m.message_id,
                           m.in_reply_to, m.references_header, m.sender, m.recipients,
                           m.subject, m.date_ts, m.date_display, m.body_loaded,
                           m.preview, m.unread, m.flagged, m.cc
                    FROM messages m
                    JOIN folders f ON f.id = m.folder_id
                    WHERE m.id IN ({placeholders})
                    "
                );
                let args: Vec<rusqlite::types::Value> =
                    chunk.iter().map(|&id| i64::from(id).into()).collect();
                let mut stmt = self
                    .mail_conn
                    .prepare(&sql)
                    .map_err(|e| format!("准备 RFC 会话详情查询失败: {e}"))?;
                let rows = stmt
                    .query_map(rusqlite::params_from_iter(args.iter()), map_raw_row)
                    .map_err(|e| format!("读取 RFC 会话行失败: {e}"))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| format!("解析 RFC 会话行失败: {e}"))?;
                for row in rows {
                    let _ = (&row.in_reply_to, &row.references_header);
                    raw_rows.push(row);
                }
            }
        } else {
            let key = normalize_thread_subject(subject);
            if key.is_empty() {
                return Ok(Vec::new());
            }
            let order_sql = if newest_first {
                "m.date_ts DESC, m.uid DESC"
            } else {
                "m.date_ts ASC, m.uid ASC"
            };
            let mut stmt = self
                .mail_conn
                .prepare(&format!(
                    "
                    SELECT m.id, m.folder_id, f.name, f.path, m.uid, m.message_id,
                           m.in_reply_to, m.references_header, m.sender, m.recipients,
                           m.subject, m.date_ts, m.date_display, m.body_loaded,
                           m.preview, m.unread, m.flagged, m.cc
                    FROM messages m
                    JOIN folders f ON f.id = m.folder_id
                    WHERE m.thread_subject = ?1
                    ORDER BY {order_sql}
                    LIMIT {MAX_THREAD_SUBJECT_ROWS}
                    "
                ))
                .map_err(|e| format!("准备会话查询失败: {e}"))?;

            let rows = stmt
                .query_map(params![key], map_raw_row)
                .map_err(|e| format!("执行会话查询失败: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("解析会话结果失败: {e}"))?;

            for row in rows {
                let _ = (&row.in_reply_to, &row.references_header);
                raw_rows.push(row);
            }
            if local_anchor_id != 0 && !raw_rows.iter().any(|r| r.id == local_anchor_id) {
                if let Some(anchor_row) = self
                    .mail_conn
                    .query_row(
                        "
                        SELECT m.id, m.folder_id, f.name, f.path, m.uid, m.message_id,
                               m.in_reply_to, m.references_header, m.sender, m.recipients,
                               m.subject, m.date_ts, m.date_display, m.body_loaded,
                               m.preview, m.unread, m.flagged, m.cc
                        FROM messages m
                        JOIN folders f ON f.id = m.folder_id
                        WHERE m.id = ?1 AND m.thread_subject = ?2
                        ",
                        params![local_anchor_id, key],
                        map_raw_row,
                    )
                    .optional()
                    .map_err(|e| format!("补查会话锚点邮件失败: {e}"))?
                {
                    let _ = (&anchor_row.in_reply_to, &anchor_row.references_header);
                    raw_rows.push(anchor_row);
                }
            }
        }

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
                (local_anchor_id != 0 && r.id == local_anchor_id)
                    || (!is_trash_folder(&r.folder_name, &r.folder_path)
                        && !is_junk_folder(&r.folder_name, &r.folder_path))
            });
        }

        let copy_priority = |r: &RawRow| -> u8 {
            if local_anchor_id != 0 && r.id == local_anchor_id {
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
        if deduped.len() > MAX_THREAD_MESSAGES {
            if local_anchor_id != 0 {
                if let Some(anchor_pos) = deduped.iter().position(|r| r.id == local_anchor_id) {
                    if anchor_pos >= MAX_THREAD_MESSAGES {
                        let anchor = deduped.remove(anchor_pos);
                        deduped.truncate(MAX_THREAD_MESSAGES - 1);
                        deduped.push(anchor);
                    } else {
                        deduped.truncate(MAX_THREAD_MESSAGES);
                    }
                } else {
                    deduped.truncate(MAX_THREAD_MESSAGES);
                }
            } else {
                deduped.truncate(MAX_THREAD_MESSAGES);
            }
        }

        let body_ids: Vec<u32> = deduped
            .iter()
            .filter(|r| r.body_loaded)
            .map(|r| r.id)
            .collect();
        let bodies_map = self.load_content_bodies_map(&body_ids)?;

        let mut out = Vec::with_capacity(deduped.len());
        for row in deduped {
            let (body_text, body_html) = if row.body_loaded {
                bodies_map
                    .get(&row.id)
                    .cloned()
                    .unwrap_or_else(|| (row.preview.clone(), None))
            } else {
                (row.preview, None)
            };
            let attachments = self.list_attachments(row.id).unwrap_or_default();
            out.push(ThreadMessage {
                id: self.encode(row.id)?,
                rfc_message_id: row.rfc_message_id,
                subject: row.subject,
                sender: row.sender,
                recipients: row.recipients,
                cc: row.cc,
                date_ts: row.date_ts,
                date_display: row.date_display,
                body_text,
                body_html,
                body_loaded: row.body_loaded,
                unread: row.unread,
                flagged: row.flagged,
                attachments,
                uid: row.uid,
                folder_path: row.folder_path,
            });
        }
        Ok(out)
    }

    pub(crate) fn list_attachments(
        &self,
        local_msg_id: u32,
    ) -> Result<Vec<AttachmentMeta>, String> {
        let mut stmt = self
            .mail_conn
            .prepare(
                "
                SELECT id, filename, mime_type, size_bytes, downloaded
                FROM attachments
                WHERE message_id = ?1
                ORDER BY id ASC
                ",
            )
            .map_err(|e| format!("查询附件列表失败: {e}"))?;

        let raw_rows = stmt
            .query_map(params![local_msg_id], |row| {
                Ok((
                    row.get::<_, u32>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?.max(0) as usize,
                    row.get::<_, i32>(4)? != 0,
                ))
            })
            .map_err(|e| format!("读取附件记录失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析附件记录失败: {e}"))?;
        let mut out = Vec::with_capacity(raw_rows.len());
        for (local_att_id, filename, mime_type, size_bytes, downloaded) in raw_rows {
            out.push(AttachmentMeta {
                id: self.encode(local_att_id)?,
                filename,
                mime_type,
                size_bytes,
                downloaded,
            });
        }
        Ok(out)
    }

    pub(crate) fn store_message_body_with_html(
        &mut self,
        local_msg_id: u32,
        body_text: &str,
        body_html: Option<&str>,
        attachments: &[CachedAttachmentInput],
        unsubscribe_url: Option<&str>,
        unsubscribe_one_click: bool,
        auth_results: Option<&str>,
    ) -> Result<(), String> {
        // Step 1: commit body into lazy content.db first so mail.db never flags body_loaded=1 before body is durable.
        {
            let content_conn = self
                .ensure_content_conn()?
                .ok_or_else(|| "无法以可写方式打开正文库".to_string())?;
            content_conn
                .execute(
                    "
                    INSERT INTO message_bodies (message_id, body_text, body_html)
                    VALUES (?1, ?2, ?3)
                    ON CONFLICT(message_id) DO UPDATE SET
                        body_text = excluded.body_text,
                        body_html = excluded.body_html
                    ",
                    params![local_msg_id, body_text, body_html],
                )
                .map_err(|e| format!("写入邮件正文库失败: {e}"))?;
        }

        // Step 2: update hot index metadata, FTS5 index, and sharded attachments in mail.db.
        let now_ts = chrono::Utc::now().timestamp();
        let tx = self
            .mail_conn
            .transaction()
            .map_err(|e| format!("开启正文缓存事务失败: {e}"))?;

        tx.execute(
            "
            UPDATE messages
            SET body_loaded = 1,
                body_cached_at = ?1,
                index_text = NULL,
                has_attachment = CASE WHEN ?2 > 0 THEN 1 ELSE has_attachment END,
                list_unsubscribe = ?4,
                list_unsubscribe_post = ?5,
                auth_results = ?6
            WHERE id = ?3
            ",
            params![
                now_ts,
                attachments.len() as i64,
                local_msg_id,
                unsubscribe_url.unwrap_or_default(),
                unsubscribe_one_click as i64,
                auth_results.unwrap_or_default(),
            ],
        )
        .map_err(|e| format!("保存邮件正文索引失败: {e}"))?;

        tx.execute(
            "DELETE FROM messages_index_fts WHERE rowid = ?1",
            params![local_msg_id],
        )
        .map_err(|e| format!("清理旧搜索正文索引失败: {e}"))?;
        if !body_text.is_empty() {
            tx.execute(
                "INSERT INTO messages_index_fts(rowid, index_text) VALUES (?1, ?2)",
                params![local_msg_id, body_text],
            )
            .map_err(|e| format!("写入正文 FTS5 索引失败: {e}"))?;
        }

        let old_paths = collect_attachment_paths(
            &tx,
            "SELECT storage_path FROM attachments WHERE message_id = ?1 AND storage_path != ''",
            params![local_msg_id],
        )?;

        tx.execute(
            "DELETE FROM attachments WHERE message_id = ?1",
            params![local_msg_id],
        )
        .map_err(|e| format!("清理旧附件缓存失败: {e}"))?;

        let mut written_files: Vec<String> = Vec::new();
        for att in attachments {
            let downloaded = att.content.is_some();
            let stored_size = att.content.as_ref().map_or(att.size_bytes, Vec::len);
            let initial_blob: &[u8] = match (self.attachments_dir.as_ref(), att.content.as_deref())
            {
                (None, Some(bytes)) => bytes,
                _ => &[],
            };
            tx.execute(
                "
                INSERT INTO attachments (
                    message_id, filename, mime_type, size_bytes, content,
                    storage_path, downloaded, remote_part, transfer_encoding
                )
                VALUES (?1, ?2, ?3, ?4, ?5, '', ?6, ?7, ?8)
                ",
                params![
                    local_msg_id,
                    att.filename,
                    att.mime_type,
                    stored_size as i64,
                    initial_blob,
                    downloaded as i32,
                    att.remote_part,
                    att.transfer_encoding,
                ],
            )
            .map_err(|e| format!("保存附件失败 ({}): {e}", att.filename))?;

            if let (Some(dir), Some(bytes)) =
                (self.attachments_dir.as_deref(), att.content.as_deref())
            {
                let att_id = u32::try_from(tx.last_insert_rowid())
                    .map_err(|_| "附件行 ID 超出范围".to_string())?;
                let rel_path = match write_attachment_file(dir, att_id, bytes) {
                    Ok(p) => p,
                    Err(err) => {
                        delete_attachment_files(Some(dir), &written_files);
                        return Err(err);
                    }
                };
                if let Err(err) = tx.execute(
                    "UPDATE attachments SET storage_path = ?1, content = X'' WHERE id = ?2",
                    params![rel_path, att_id],
                ) {
                    delete_attachment_files(Some(dir), &[rel_path]);
                    delete_attachment_files(Some(dir), &written_files);
                    return Err(format!("更新附件磁盘路径失败: {err}"));
                }
                written_files.push(rel_path);
            }
        }

        if let Err(err) = tx.commit() {
            delete_attachment_files(self.attachments_dir.as_deref(), &written_files);
            return Err(format!("提交正文缓存事务失败: {err}"));
        }
        delete_attachment_files(self.attachments_dir.as_deref(), &old_paths);
        Ok(())
    }

    pub(crate) fn clear_message_bodies(&mut self) -> Result<usize, String> {
        let cleared: usize = self
            .mail_conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE body_loaded != 0 OR (index_text IS NOT NULL AND index_text != '')",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n.max(0) as usize)
            .map_err(|e| format!("统计已缓存正文数失败: {e}"))?;

        let tx = self
            .mail_conn
            .transaction()
            .map_err(|e| format!("开启正文缓存清理事务失败: {e}"))?;
        let old_paths = collect_attachment_paths(
            &tx,
            "SELECT storage_path FROM attachments WHERE storage_path != ''",
            [],
        )?;
        tx.execute(
            "UPDATE messages SET body_loaded = 0, body_cached_at = 0, index_text = NULL
             WHERE body_loaded != 0 OR index_text IS NOT NULL",
            [],
        )
        .map_err(|e| format!("清空邮件正文标记失败: {e}"))?;
        tx.execute(
            "INSERT INTO messages_index_fts(messages_index_fts) VALUES('delete-all')",
            [],
        )
        .map_err(|e| format!("清空正文 FTS5 索引失败: {e}"))?;
        tx.execute("DELETE FROM attachments", [])
            .map_err(|e| format!("清空附件缓存失败: {e}"))?;
        tx.commit()
            .map_err(|e| format!("提交正文缓存清理事务失败: {e}"))?;

        if let Some(content_conn) = self.open_existing_content_conn()? {
            let _ = content_conn.execute("DELETE FROM message_bodies", []);
            let _ = content_conn.execute_batch("VACUUM");
        }
        delete_attachment_files(self.attachments_dir.as_deref(), &old_paths);
        let _ = self.mail_conn.execute_batch("VACUUM");
        Ok(cleared)
    }

    pub(crate) fn prune_expired_downloads(
        &mut self,
        keep_days: u32,
        now_ts: i64,
    ) -> Result<usize, String> {
        if keep_days == 0 {
            return Ok(0);
        }
        let cutoff = now_ts.saturating_sub(i64::from(keep_days) * 86_400);
        let tx = self
            .mail_conn
            .transaction()
            .map_err(|e| format!("开启过期缓存清理事务失败: {e}"))?;

        let target_ids: Vec<u32> = {
            let mut stmt = tx
                .prepare(
                    "SELECT id FROM messages
                     WHERE date_ts > 0
                       AND date_ts < ?1
                       AND (
                           body_loaded != 0
                           OR EXISTS (SELECT 1 FROM attachments a WHERE a.message_id = messages.id)
                       )",
                )
                .map_err(|e| format!("查询过期缓存邮件失败: {e}"))?;
            let collected = stmt
                .query_map(params![cutoff], |r| r.get::<_, u32>(0))
                .map_err(|e| format!("遍历过期缓存邮件失败: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("收集过期缓存邮件失败: {e}"))?;
            collected
        };

        let old_paths = collect_attachment_paths(
            &tx,
            "SELECT a.storage_path
             FROM attachments a
             JOIN messages m ON m.id = a.message_id
             WHERE m.date_ts > 0 AND m.date_ts < ?1 AND a.storage_path != ''",
            params![cutoff],
        )?;

        let pruned = tx
            .execute(
                "UPDATE messages
                 SET body_loaded = 0,
                     body_cached_at = 0,
                     index_text = COALESCE(index_text, '')
                 WHERE date_ts > 0
                   AND date_ts < ?1
                   AND (
                       body_loaded != 0
                       OR EXISTS (SELECT 1 FROM attachments a WHERE a.message_id = messages.id)
                   )",
                params![cutoff],
            )
            .map_err(|e| format!("清理过期正文缓存失败: {e}"))?;

        for msg_id in &target_ids {
            let _ = tx.execute(
                "DELETE FROM messages_index_fts WHERE rowid = ?1",
                params![msg_id],
            );
        }

        tx.execute(
            "DELETE FROM attachments
             WHERE message_id IN (
                 SELECT id FROM messages WHERE date_ts > 0 AND date_ts < ?1
             )",
            params![cutoff],
        )
        .map_err(|e| format!("清理过期附件缓存失败: {e}"))?;

        tx.commit()
            .map_err(|e| format!("提交过期缓存清理事务失败: {e}"))?;

        self.delete_content_bodies_for_ids(&target_ids);
        delete_attachment_files(self.attachments_dir.as_deref(), &old_paths);
        Ok(pruned)
    }

    fn delete_content_bodies_for_ids(&self, local_msg_ids: &[u32]) {
        if local_msg_ids.is_empty() || self.read_only {
            return;
        }
        let Ok(Some(conn)) = self.open_existing_content_conn() else {
            return;
        };
        for chunk in local_msg_ids.chunks(200) {
            let marks = vec!["?"; chunk.len()].join(", ");
            let args: Vec<rusqlite::types::Value> =
                chunk.iter().map(|id| i64::from(*id).into()).collect();
            let _ = conn.execute(
                &format!("DELETE FROM message_bodies WHERE message_id IN ({marks})"),
                rusqlite::params_from_iter(args.iter()),
            );
        }
    }

    pub(crate) fn get_message_remote_id(
        &self,
        local_msg_id: u32,
    ) -> Result<Option<String>, String> {
        self.mail_conn
            .query_row(
                "SELECT remote_id FROM messages WHERE id = ?1",
                params![local_msg_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map(|opt| opt.filter(|s| !s.is_empty()))
            .map_err(|e| format!("读取邮件远端 ID 失败: {e}"))
    }

    pub(crate) fn list_cached_attachments(
        &self,
        query: &str,
        fetch_limit: usize,
    ) -> Result<Vec<CachedAttachment>, String> {
        let trimmed = query.trim();
        let raw_rows = if trimmed.is_empty() {
            let mut stmt = self
                .mail_conn
                .prepare(
                    "
                    SELECT a.id, a.message_id, a.filename, a.mime_type, a.size_bytes,
                           m.sender, m.subject, m.date_display, m.date_ts, a.downloaded
                    FROM attachments a
                    JOIN messages m ON m.id = a.message_id
                    ORDER BY m.date_ts DESC, a.id ASC
                    LIMIT ?1
                    ",
                )
                .map_err(|e| format!("准备附件总览查询失败: {e}"))?;
            let rows = stmt
                .query_map(params![fetch_limit as i64], |row| {
                    Ok((
                        row.get::<_, u32>(0)?,
                        row.get::<_, u32>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?.max(0) as usize,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, i32>(9)? != 0,
                    ))
                })
                .map_err(|e| format!("执行附件总览查询失败: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("解析附件总览结果失败: {e}"))?;
            rows
        } else {
            let pattern = format!("%{trimmed}%");
            let mut stmt = self
                .mail_conn
                .prepare(
                    "
                    SELECT a.id, a.message_id, a.filename, a.mime_type, a.size_bytes,
                           m.sender, m.subject, m.date_display, m.date_ts, a.downloaded
                    FROM attachments a
                    JOIN messages m ON m.id = a.message_id
                    WHERE (a.filename LIKE ?1 OR m.sender LIKE ?1 OR m.subject LIKE ?1)
                    ORDER BY m.date_ts DESC, a.id ASC
                    LIMIT ?2
                    ",
                )
                .map_err(|e| format!("准备附件总览搜索查询失败: {e}"))?;
            let rows = stmt
                .query_map(params![pattern, fetch_limit as i64], |row| {
                    Ok((
                        row.get::<_, u32>(0)?,
                        row.get::<_, u32>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?.max(0) as usize,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, i32>(9)? != 0,
                    ))
                })
                .map_err(|e| format!("执行附件总览搜索查询失败: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("解析附件总览搜索结果失败: {e}"))?;
            rows
        };

        let mut out = Vec::with_capacity(raw_rows.len());
        for (
            local_att_id,
            local_msg_id,
            filename,
            mime_type,
            size_bytes,
            sender,
            subject,
            date_display,
            date_ts,
            downloaded,
        ) in raw_rows
        {
            out.push(CachedAttachment {
                id: self.encode(local_att_id)?,
                message_id: self.encode(local_msg_id)?,
                account_id: self.account_id,
                filename,
                mime_type,
                size_bytes,
                sender,
                subject,
                date_display,
                date_ts,
                downloaded,
            });
        }
        Ok(out)
    }

    pub(crate) fn scan_contact_rows(&self) -> Result<Vec<(String, String, bool)>, String> {
        let mut stmt = self
            .mail_conn
            .prepare(&format!(
                "
                SELECT m.sender, m.recipients, CASE WHEN {SENT_FOLDER_MATCH} THEN 1 ELSE 0 END AS is_sent
                FROM messages m
                LEFT JOIN folders f ON f.id = m.folder_id
                "
            ))
            .map_err(|e| format!("查询联系人来源失败: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)? != 0,
                ))
            })
            .map_err(|e| format!("读取联系人来源失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析联系人来源失败: {e}"))?;
        Ok(rows)
    }

    pub(crate) fn apply_filter_rules(
        &mut self,
        account_email: &str,
        rules: &[&crate::config::FilterRule],
    ) -> Result<usize, String> {
        use crate::config::{FilterField, FilterMatch};

        let matching_rules: Vec<&&crate::config::FilterRule> = rules
            .iter()
            .filter(|r| {
                r.account_email.trim().is_empty()
                    || r.account_email.trim().eq_ignore_ascii_case(account_email)
            })
            .collect();
        if matching_rules.is_empty() {
            return Ok(0);
        }

        let needs_body = matching_rules
            .iter()
            .any(|r| matches!(r.field, FilterField::Body));

        let rows: Vec<(u32, i64, String, String, String, String, bool, bool, bool)> = {
            let mut stmt = self
                .mail_conn
                .prepare(
                    "
                    SELECT m.id, m.folder_id, m.sender, m.recipients, m.subject,
                           m.preview, m.unread, m.flagged, m.body_loaded
                    FROM messages m
                    ",
                )
                .map_err(|e| format!("读取缓存邮件失败: {e}"))?;
            let collected = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, u32>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, i64>(6)? != 0,
                        row.get::<_, i64>(7)? != 0,
                        row.get::<_, i64>(8)? != 0,
                    ))
                })
                .map_err(|e| format!("遍历缓存邮件失败: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("解析邮件行失败: {e}"))?;
            collected
        };

        let bodies_map = if needs_body {
            let ids: Vec<u32> = rows.iter().filter(|r| r.8).map(|r| r.0).collect();
            self.load_content_bodies_map(&ids)?
        } else {
            HashMap::new()
        };

        let mut updated = 0usize;
        for (msg_id, folder_id, sender, recipients, subject, preview, unread, flagged, _) in rows {
            let body_for_rule = bodies_map
                .get(&msg_id)
                .map(|(t, _)| t.as_str())
                .unwrap_or(preview.as_str());
            for rule in &matching_rules {
                let haystack = match rule.field {
                    FilterField::From => sender.as_str(),
                    FilterField::To => recipients.as_str(),
                    FilterField::Subject => subject.as_str(),
                    FilterField::Body => body_for_rule,
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
                    self.mail_conn
                        .execute(
                            "UPDATE messages SET unread = 0 WHERE id = ?1",
                            params![msg_id],
                        )
                        .map_err(|e| format!("过滤器标记已读失败: {e}"))?;
                    changed = true;
                }
                if rule.star && !flagged {
                    self.mail_conn
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
                        .mail_conn
                        .query_row(
                            "SELECT id FROM folders WHERE (LOWER(path) = LOWER(?1) OR LOWER(name) = LOWER(?1)) LIMIT 1",
                            params![target],
                            |r| r.get::<_, i64>(0),
                        )
                        .optional()
                        .map_err(|e| format!("查询目标文件夹失败: {e}"))?
                    {
                        if dest_id != folder_id {
                            let _ = self.mail_conn.execute(
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

    pub(crate) fn remove_message(&mut self, local_msg_id: u32) -> Result<Option<u32>, String> {
        let tx = self
            .mail_conn
            .transaction()
            .map_err(|e| format!("开启邮件清理事务失败: {e}"))?;
        let local_folder_id: Option<u32> = tx
            .query_row(
                "SELECT folder_id FROM messages WHERE id = ?1",
                params![local_msg_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| format!("查询邮件所在文件夹失败: {e}"))?;
        let Some(local_folder_id) = local_folder_id else {
            return Ok(None);
        };
        let old_paths = collect_attachment_paths(
            &tx,
            "SELECT storage_path FROM attachments WHERE message_id = ?1 AND storage_path != ''",
            params![local_msg_id],
        )?;
        tx.execute("DELETE FROM messages WHERE id = ?1", params![local_msg_id])
            .map_err(|e| format!("移除缓存邮件失败: {e}"))?;
        tx.execute(
            "
            UPDATE folders
            SET unread_count = (SELECT COUNT(*) FROM messages WHERE folder_id = ?1 AND unread = 1),
                total_count = (SELECT COUNT(*) FROM messages WHERE folder_id = ?1)
            WHERE id = ?1
            ",
            params![local_folder_id],
        )
        .map_err(|e| format!("更新文件夹计数失败: {e}"))?;
        tx.commit()
            .map_err(|e| format!("提交邮件清理事务失败: {e}"))?;

        self.delete_content_bodies_for_ids(&[local_msg_id]);
        delete_attachment_files(self.attachments_dir.as_deref(), &old_paths);
        Ok(Some(self.encode(local_folder_id)?))
    }

    pub(crate) fn get_attachment_record(
        &self,
        local_att_id: u32,
    ) -> Result<Option<AttachmentRecord>, String> {
        let row = self
            .mail_conn
            .query_row(
                "
                SELECT a.message_id, f.path, m.uid, m.remote_id,
                       a.filename, a.mime_type, a.size_bytes, a.content,
                       a.storage_path, a.downloaded, a.remote_part, a.transfer_encoding
                FROM attachments a
                JOIN messages m ON m.id = a.message_id
                JOIN folders f ON f.id = m.folder_id
                WHERE a.id = ?1
                ",
                params![local_att_id],
                |row| {
                    Ok((
                        row.get::<_, u32>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, u32>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, i64>(6)?.max(0) as usize,
                        row.get::<_, Vec<u8>>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, i32>(9)? != 0,
                        row.get::<_, String>(10)?,
                        row.get::<_, String>(11)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| format!("读取附件记录失败: {e}"))?;

        let Some((
            local_msg_id,
            folder_path,
            uid,
            remote_msg_id,
            filename,
            mime_type,
            size_bytes,
            content,
            storage_path,
            downloaded,
            remote_part,
            transfer_encoding,
        )) = row
        else {
            return Ok(None);
        };

        let ext_msg_id = self.encode(local_msg_id)?;

        if downloaded {
            if !storage_path.is_empty() {
                if let (Some(dir), Some(resolved)) = (
                    self.attachments_dir.as_deref(),
                    resolve_attachment_disk_path(self.attachments_dir.as_deref(), &storage_path),
                ) {
                    let _ = dir;
                    if let Ok(bytes) = std::fs::read(resolved) {
                        return Ok(Some(AttachmentRecord::Local {
                            message_id: ext_msg_id,
                            filename,
                            mime_type,
                            bytes,
                        }));
                    }
                }
            } else {
                return Ok(Some(AttachmentRecord::Local {
                    message_id: ext_msg_id,
                    filename,
                    mime_type,
                    bytes: content,
                }));
            }
        }

        Ok(Some(AttachmentRecord::PendingRemote {
            message_id: ext_msg_id,
            account_id: self.account_id,
            folder_path,
            uid,
            remote_msg_id,
            filename,
            mime_type,
            size_bytes,
            remote_part,
            transfer_encoding,
        }))
    }

    pub(crate) fn store_downloaded_attachment_bytes(
        &mut self,
        local_att_id: u32,
        bytes: &[u8],
    ) -> Result<usize, String> {
        let tx = self
            .mail_conn
            .transaction()
            .map_err(|e| format!("开启附件落盘事务失败: {e}"))?;
        let old_path: Option<String> = tx
            .query_row(
                "SELECT storage_path FROM attachments WHERE id = ?1",
                params![local_att_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| format!("查询目标附件失败: {e}"))?;
        let Some(old_path) = old_path else {
            return Err("目标附件记录不存在".to_string());
        };

        let mut new_path = String::new();
        let blob: &[u8] = if let Some(dir) = self.attachments_dir.as_deref() {
            new_path = write_attachment_file(dir, local_att_id, bytes)?;
            &[]
        } else {
            bytes
        };

        if let Err(err) = tx.execute(
            "
            UPDATE attachments
            SET size_bytes = ?1,
                content = ?2,
                storage_path = ?3,
                downloaded = 1
            WHERE id = ?4
            ",
            params![bytes.len() as i64, blob, new_path, local_att_id],
        ) {
            if !new_path.is_empty() && new_path != old_path {
                delete_attachment_files(self.attachments_dir.as_deref(), &[new_path]);
            }
            return Err(format!("更新已下载附件状态失败: {err}"));
        }

        if let Err(err) = tx.commit() {
            if !new_path.is_empty() && new_path != old_path {
                delete_attachment_files(self.attachments_dir.as_deref(), &[new_path]);
            }
            return Err(format!("提交附件落盘事务失败: {err}"));
        }

        if !old_path.is_empty() && old_path != new_path {
            delete_attachment_files(self.attachments_dir.as_deref(), &[old_path]);
        }
        Ok(bytes.len())
    }

    pub(crate) fn list_message_attachment_ids(
        &self,
        local_msg_id: u32,
    ) -> Result<Vec<u32>, String> {
        let mut stmt = self
            .mail_conn
            .prepare("SELECT id FROM attachments WHERE message_id = ?1 ORDER BY id ASC")
            .map_err(|e| format!("查询邮件附件 ID 列表失败: {e}"))?;
        let rows = stmt
            .query_map(params![local_msg_id], |row| row.get::<_, u32>(0))
            .map_err(|e| format!("读取邮件附件 ID 列表失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析邮件附件 ID 列表失败: {e}"))?;
        let mut out = Vec::with_capacity(rows.len());
        for local_id in rows {
            out.push(self.encode(local_id)?);
        }
        Ok(out)
    }

    pub(crate) fn get_attachment_bytes(
        &self,
        local_att_id: u32,
    ) -> Result<Option<(String, String, Vec<u8>)>, String> {
        match self.get_attachment_record(local_att_id)? {
            Some(AttachmentRecord::Local {
                filename,
                mime_type,
                bytes,
                ..
            }) => Ok(Some((filename, mime_type, bytes))),
            _ => Ok(None),
        }
    }

    pub(crate) fn get_message_location(
        &self,
        local_msg_id: u32,
    ) -> Result<Option<(String, u32)>, String> {
        self.mail_conn
            .query_row(
                "SELECT f.path, m.uid FROM messages m JOIN folders f ON f.id = m.folder_id WHERE m.id = ?1",
                params![local_msg_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?)),
            )
            .optional()
            .map_err(|e| format!("查询邮件位置失败: {e}"))
    }

    pub(crate) fn delete_attachment_and_update_uid(
        &self,
        local_msg_id: u32,
        local_att_id: u32,
        new_uid: Option<u32>,
    ) -> Result<Vec<AttachmentMeta>, String> {
        let tx = self
            .mail_conn
            .unchecked_transaction()
            .map_err(|e| format!("开启删除本地附件事务失败: {e}"))?;
        let old_paths = collect_attachment_paths(
            &tx,
            "SELECT storage_path FROM attachments WHERE id = ?1 AND message_id = ?2 AND storage_path != ''",
            params![local_att_id, local_msg_id],
        )?;
        tx.execute(
            "DELETE FROM attachments WHERE id = ?1 AND message_id = ?2",
            params![local_att_id, local_msg_id],
        )
        .map_err(|e| format!("删除本地附件缓存失败: {e}"))?;
        let remaining_count: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM attachments WHERE message_id = ?1",
                params![local_msg_id],
                |row| row.get(0),
            )
            .unwrap_or(0);
        let has_att = if remaining_count == 0 { 0i64 } else { 1i64 };
        if let Some(uid) = new_uid {
            tx.execute(
                "UPDATE messages SET has_attachment = ?1, uid = ?2 WHERE id = ?3",
                params![has_att, uid, local_msg_id],
            )
            .map_err(|e| format!("更新邮件附件状态失败: {e}"))?;
        } else {
            tx.execute(
                "UPDATE messages SET has_attachment = ?1 WHERE id = ?2",
                params![has_att, local_msg_id],
            )
            .map_err(|e| format!("更新邮件附件状态失败: {e}"))?;
        }
        tx.commit()
            .map_err(|e| format!("提交删除本地附件事务失败: {e}"))?;
        delete_attachment_files(self.attachments_dir.as_deref(), &old_paths);
        self.list_attachments(local_msg_id)
    }

    pub(crate) fn export_message_attachments(
        &self,
        local_msg_id: u32,
        dest_dir: &Path,
    ) -> Result<Vec<PathBuf>, String> {
        let ext_att_ids = self.list_message_attachment_ids(local_msg_id)?;
        let mut rows = Vec::with_capacity(ext_att_ids.len());
        for ext_att_id in ext_att_ids {
            if let Some((_, local_att_id)) = decode_scoped_id(ext_att_id) {
                if let Some((filename, _mime, bytes)) = self.get_attachment_bytes(local_att_id)? {
                    rows.push((filename, bytes));
                }
            }
        }

        if rows.is_empty() {
            return Ok(Vec::new());
        }
        std::fs::create_dir_all(dest_dir)
            .map_err(|e| format!("创建转发附件临时目录失败 ({}): {e}", dest_dir.display()))?;

        let mut used_names: HashSet<String> = HashSet::new();
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

    pub(crate) fn set_message_seen(&mut self, local_msg_id: u32, seen: bool) -> Result<(), String> {
        let tx = self
            .mail_conn
            .transaction()
            .map_err(|e| format!("开启已读状态事务失败: {e}"))?;
        let folder_id: Option<u32> = tx
            .query_row(
                "SELECT folder_id FROM messages WHERE id = ?1",
                params![local_msg_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| format!("查询邮件所在文件夹失败: {e}"))?;
        let Some(folder_id) = folder_id else {
            return Ok(());
        };
        tx.execute(
            "UPDATE messages SET unread = ?1 WHERE id = ?2",
            params![(!seen) as i32, local_msg_id],
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

    pub(crate) fn set_folder_all_seen(&mut self, local_folder_id: u32) -> Result<(), String> {
        self.mail_conn
            .execute(
                "UPDATE messages SET unread = 0 WHERE folder_id = ?1",
                params![local_folder_id],
            )
            .map_err(|e| format!("批量更新已读状态失败: {e}"))?;
        self.mail_conn
            .execute(
                "
                UPDATE folders
                SET unread_count = (SELECT COUNT(*) FROM messages WHERE folder_id = ?1 AND unread = 1)
                WHERE id = ?1
                ",
                params![local_folder_id],
            )
            .map_err(|e| format!("更新文件夹未读数失败: {e}"))?;
        Ok(())
    }

    pub(crate) fn clear_folder(&mut self, local_folder_id: u32) -> Result<(), String> {
        let tx = self
            .mail_conn
            .transaction()
            .map_err(|e| format!("开启清空文件夹事务失败: {e}"))?;
        let deleted_msg_ids: Vec<u32> = {
            let mut stmt = tx
                .prepare("SELECT id FROM messages WHERE folder_id = ?1")
                .map_err(|e| format!("查询待清空邮件失败: {e}"))?;
            let collected = stmt
                .query_map(params![local_folder_id], |r| r.get::<_, u32>(0))
                .map_err(|e| format!("遍历待清空邮件失败: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("收集待清空邮件失败: {e}"))?;
            collected
        };
        let old_paths = collect_attachment_paths(
            &tx,
            "SELECT a.storage_path FROM attachments a
             JOIN messages m ON m.id = a.message_id
             WHERE m.folder_id = ?1 AND a.storage_path != ''",
            params![local_folder_id],
        )?;
        tx.execute(
            "DELETE FROM messages WHERE folder_id = ?1",
            params![local_folder_id],
        )
        .map_err(|e| format!("清空文件夹缓存失败: {e}"))?;
        tx.execute(
            "
            UPDATE folders
            SET unread_count = (SELECT COUNT(*) FROM messages WHERE folder_id = ?1 AND unread = 1),
                total_count = (SELECT COUNT(*) FROM messages WHERE folder_id = ?1)
            WHERE id = ?1
            ",
            params![local_folder_id],
        )
        .map_err(|e| format!("更新文件夹计数失败: {e}"))?;
        tx.commit()
            .map_err(|e| format!("提交清空文件夹事务失败: {e}"))?;

        self.delete_content_bodies_for_ids(&deleted_msg_ids);
        delete_attachment_files(self.attachments_dir.as_deref(), &old_paths);
        Ok(())
    }

    pub(crate) fn folder_unread_remote_ids(
        &self,
        local_folder_id: u32,
    ) -> Result<Vec<(u32, String)>, String> {
        let mut stmt = self
            .mail_conn
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
            .query_map(params![local_folder_id], |row| {
                Ok((row.get::<_, u32>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("读取文件夹未读邮件失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析文件夹未读邮件失败: {e}"))?;
        let mut out = Vec::with_capacity(rows.len());
        for (local_id, remote_id) in rows {
            out.push((self.encode(local_id)?, remote_id));
        }
        Ok(out)
    }

    pub(crate) fn set_message_flagged(
        &mut self,
        local_msg_id: u32,
        flagged: bool,
    ) -> Result<(), String> {
        self.mail_conn
            .execute(
                "UPDATE messages SET flagged = ?1 WHERE id = ?2",
                params![flagged as i32, local_msg_id],
            )
            .map_err(|e| format!("更新星标状态失败: {e}"))?;
        Ok(())
    }

    pub(crate) fn list_thread_summaries(
        &self,
        local_head_id: u32,
        subject: &str,
    ) -> Result<Vec<MessageSummary>, String> {
        let thread = self.list_thread_messages(local_head_id, subject, false)?;
        let mut local_ids: Vec<u32> = Vec::new();
        for m in thread {
            if let Some((_, lid)) = decode_scoped_id(m.id) {
                if lid != local_head_id {
                    local_ids.push(lid);
                }
            }
        }
        if local_ids.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = vec!["?"; local_ids.len()].join(", ");
        let args: Vec<rusqlite::types::Value> =
            local_ids.iter().map(|&id| i64::from(id).into()).collect();
        let mut stmt = self
            .mail_conn
            .prepare(&format!(
                "
                SELECT m.id, m.folder_id, f.path, m.uid, m.sender, m.subject,
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
            .query_map(rusqlite::params_from_iter(args.iter()), |row| {
                self.map_local_summary_row(row)
            })
            .map_err(|e| format!("执行会话子邮件查询失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析会话子邮件失败: {e}"))?;
        Ok(rows)
    }

    pub(crate) fn set_message_tags(
        &mut self,
        local_msg_id: u32,
        tags: &[String],
    ) -> Result<(), String> {
        let joined = tags
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(",");
        self.mail_conn
            .execute(
                "UPDATE messages SET tags = ?1 WHERE id = ?2",
                params![joined, local_msg_id],
            )
            .map_err(|e| format!("更新邮件标签失败: {e}"))?;
        Ok(())
    }

    pub(crate) fn snooze_message(
        &mut self,
        local_msg_id: u32,
        until_ts: i64,
    ) -> Result<(), String> {
        self.mail_conn
            .execute(
                "UPDATE messages SET snoozed_until_ts = ?1 WHERE id = ?2",
                params![until_ts, local_msg_id],
            )
            .map_err(|e| format!("更新稍后提醒失败: {e}"))?;
        Ok(())
    }

    pub(crate) fn wake_due_snoozed(
        &mut self,
        now_ts: i64,
    ) -> Result<Vec<(u32, u32, String)>, String> {
        let mut stmt = self
            .mail_conn
            .prepare(
                "SELECT id, subject FROM messages
                 WHERE snoozed_until_ts > 0 AND snoozed_until_ts <= ?1",
            )
            .map_err(|e| format!("查询到期稍后提醒失败: {e}"))?;
        let due_raw: Vec<(u32, String)> = stmt
            .query_map(params![now_ts], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| format!("读取到期稍后提醒失败: {e}"))?
            .filter_map(Result::ok)
            .collect();
        drop(stmt);
        if !due_raw.is_empty() {
            self.mail_conn
                .execute(
                    "UPDATE messages SET snoozed_until_ts = 0, unread = 1
                     WHERE snoozed_until_ts > 0 AND snoozed_until_ts <= ?1",
                    params![now_ts],
                )
                .map_err(|e| format!("唤醒稍后提醒邮件失败: {e}"))?;
        }
        let mut out = Vec::with_capacity(due_raw.len());
        for (local_id, subject) in due_raw {
            out.push((self.encode(local_id)?, self.account_id, subject));
        }
        Ok(out)
    }

    pub(crate) fn copy_message_within_same_account(
        &mut self,
        local_msg_id: u32,
        target_folder_path: &str,
    ) -> Result<(), String> {
        let tx = self
            .mail_conn
            .transaction()
            .map_err(|e| format!("开启复制邮件事务失败: {e}"))?;
        let target_folder_id: u32 = tx
            .query_row(
                "SELECT id FROM folders WHERE path = ?1",
                params![target_folder_path],
                |row| row.get(0),
            )
            .map_err(|e| format!("未找到目标文件夹 {target_folder_path}: {e}"))?;
        let next_uid: u32 = tx
            .query_row(
                "SELECT COALESCE(MAX(uid), 0) + 1 FROM messages WHERE folder_id = ?1",
                params![target_folder_id],
                |row| row.get(0),
            )
            .unwrap_or(1);
        let inserted = tx
            .execute(
                "
                INSERT INTO messages (
                    folder_id, uid, remote_id, message_id, in_reply_to, references_header,
                    sender, recipients, cc, subject, thread_subject, preview, date_ts, date_display,
                    unread, flagged, has_attachment, body_loaded, body_cached_at, index_text,
                    list_unsubscribe, list_unsubscribe_post, auth_results,
                    importance, tags, answered, forwarded, snoozed_until_ts
                )
                SELECT
                    ?1, ?2, '', message_id, in_reply_to, references_header,
                    sender, recipients, cc, subject, thread_subject, preview, date_ts, date_display,
                    unread, flagged, has_attachment, body_loaded, body_cached_at, index_text,
                    list_unsubscribe, list_unsubscribe_post, auth_results,
                    importance, tags, answered, forwarded, snoozed_until_ts
                FROM messages
                WHERE id = ?3
                ",
                params![target_folder_id, next_uid, local_msg_id],
            )
            .map_err(|e| format!("复制邮件到本地缓存失败: {e}"))?;

        let new_local_id = if inserted > 0 {
            let new_row_id = u32::try_from(tx.last_insert_rowid())
                .map_err(|_| "复制邮件行 ID 超出范围".to_string())?;
            let (msg_id, irt, refs, index_text): (String, String, String, Option<String>) = tx
                .query_row(
                    "SELECT message_id, in_reply_to, references_header, index_text FROM messages WHERE id = ?1",
                    params![new_row_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .map_err(|e| format!("读取复制邮件索引失败: {e}"))?;
            sync_message_rfc_refs(&tx, new_row_id, &msg_id, &irt, &refs)?;
            if let Some(txt) = index_text.filter(|s| !s.is_empty()) {
                let _ = tx.execute(
                    "INSERT INTO messages_index_fts(rowid, index_text) VALUES (?1, ?2)",
                    params![new_row_id, txt],
                );
            }
            Some(new_row_id)
        } else {
            None
        };
        tx.commit()
            .map_err(|e| format!("提交复制邮件事务失败: {e}"))?;

        if let Some(new_id) = new_local_id {
            if let Ok(map) = self.load_content_bodies_map(&[local_msg_id]) {
                if let Some((text, html)) = map.get(&local_msg_id) {
                    if let Ok(Some(c)) = self.ensure_content_conn() {
                        let _ = c.execute(
                            "INSERT OR REPLACE INTO message_bodies (message_id, body_text, body_html) VALUES (?1, ?2, ?3)",
                            params![new_id, text, html],
                        );
                    }
                    if !text.is_empty() {
                        let _ = self.mail_conn.execute(
                            "INSERT INTO messages_index_fts(rowid, index_text) VALUES (?1, ?2)",
                            params![new_id, text],
                        );
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn export_message_for_cross_account_copy(
        &self,
        local_msg_id: u32,
    ) -> Result<Option<CopiedMessagePayload>, String> {
        let row = self
            .mail_conn
            .query_row(
                "
                SELECT message_id, in_reply_to, references_header, sender, recipients, cc,
                       subject, thread_subject, preview, date_ts, date_display,
                       unread, flagged, has_attachment, body_loaded, body_cached_at, index_text,
                       list_unsubscribe, list_unsubscribe_post, auth_results,
                       importance, tags, answered, forwarded, snoozed_until_ts
                FROM messages WHERE id = ?1
                ",
                params![local_msg_id],
                |r| {
                    Ok(CopiedMessagePayload {
                        message_id: r.get(0)?,
                        in_reply_to: r.get(1)?,
                        references_header: r.get(2)?,
                        sender: r.get(3)?,
                        recipients: r.get(4)?,
                        cc: r.get(5)?,
                        subject: r.get(6)?,
                        thread_subject: r.get(7)?,
                        preview: r.get(8)?,
                        date_ts: r.get(9)?,
                        date_display: r.get(10)?,
                        unread: r.get(11)?,
                        flagged: r.get(12)?,
                        has_attachment: r.get(13)?,
                        body_loaded: r.get(14)?,
                        body_cached_at: r.get(15)?,
                        index_text: r.get(16)?,
                        list_unsubscribe: r.get(17)?,
                        list_unsubscribe_post: r.get(18)?,
                        auth_results: r.get(19)?,
                        importance: r.get(20)?,
                        tags: r.get(21)?,
                        answered: r.get(22)?,
                        forwarded: r.get(23)?,
                        snoozed_until_ts: r.get(24)?,
                        body: None,
                    })
                },
            )
            .optional()
            .map_err(|e| format!("读取跨账号待复制邮件失败: {e}"))?;

        let Some(mut payload) = row else {
            return Ok(None);
        };
        if payload.body_loaded != 0 {
            let bodies = self.load_content_bodies_map(&[local_msg_id])?;
            payload.body = bodies.get(&local_msg_id).cloned();
        }
        Ok(Some(payload))
    }

    pub(crate) fn insert_copied_message(
        &mut self,
        target_folder_path: &str,
        payload: &CopiedMessagePayload,
    ) -> Result<(), String> {
        let tx = self
            .mail_conn
            .transaction()
            .map_err(|e| format!("开启跨账号复制事务失败: {e}"))?;
        let target_folder_id: u32 = tx
            .query_row(
                "SELECT id FROM folders WHERE path = ?1",
                params![target_folder_path],
                |row| row.get(0),
            )
            .map_err(|e| format!("未找到目标文件夹 {target_folder_path}: {e}"))?;
        let next_uid: u32 = tx
            .query_row(
                "SELECT COALESCE(MAX(uid), 0) + 1 FROM messages WHERE folder_id = ?1",
                params![target_folder_id],
                |row| row.get(0),
            )
            .unwrap_or(1);

        tx.execute(
            "
            INSERT INTO messages (
                folder_id, uid, remote_id, message_id, in_reply_to, references_header,
                sender, recipients, cc, subject, thread_subject, preview, date_ts, date_display,
                unread, flagged, has_attachment, body_loaded, body_cached_at, index_text,
                list_unsubscribe, list_unsubscribe_post, auth_results,
                importance, tags, answered, forwarded, snoozed_until_ts
            )
            VALUES (
                ?1, ?2, '', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27
            )
            ",
            params![
                target_folder_id,
                next_uid,
                payload.message_id,
                payload.in_reply_to,
                payload.references_header,
                payload.sender,
                payload.recipients,
                payload.cc,
                payload.subject,
                payload.thread_subject,
                payload.preview,
                payload.date_ts,
                payload.date_display,
                payload.unread,
                payload.flagged,
                payload.has_attachment,
                payload.body_loaded,
                payload.body_cached_at,
                payload.index_text,
                payload.list_unsubscribe,
                payload.list_unsubscribe_post,
                payload.auth_results,
                payload.importance,
                payload.tags,
                payload.answered,
                payload.forwarded,
                payload.snoozed_until_ts,
            ],
        )
        .map_err(|e| format!("写入跨账号复制邮件失败: {e}"))?;
        let new_row_id = u32::try_from(tx.last_insert_rowid())
            .map_err(|_| "复制邮件行 ID 超出范围".to_string())?;
        sync_message_rfc_refs(
            &tx,
            new_row_id,
            &payload.message_id,
            &payload.in_reply_to,
            &payload.references_header,
        )?;
        let fts_text = payload
            .body
            .as_ref()
            .map(|(t, _)| t.as_str())
            .or(payload.index_text.as_deref())
            .unwrap_or("");
        if !fts_text.is_empty() {
            let _ = tx.execute(
                "INSERT INTO messages_index_fts(rowid, index_text) VALUES (?1, ?2)",
                params![new_row_id, fts_text],
            );
        }
        tx.commit()
            .map_err(|e| format!("提交跨账号复制事务失败: {e}"))?;

        if let Some((ref text, ref html)) = payload.body {
            if let Ok(Some(c)) = self.ensure_content_conn() {
                let _ = c.execute(
                    "INSERT OR REPLACE INTO message_bodies (message_id, body_text, body_html) VALUES (?1, ?2, ?3)",
                    params![new_row_id, text, html],
                );
            }
        }
        Ok(())
    }

    fn map_local_summary_row(&self, row: &rusqlite::Row<'_>) -> rusqlite::Result<MessageSummary> {
        let local_id: u32 = row.get(0)?;
        let local_folder_id: u32 = row.get(1)?;
        let tags_raw: String = row.get(14)?;
        let tags = tags_raw
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let ext_id = encode_scoped_id(self.slot, local_id).map_err(|e| {
            rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(e)))
        })?;
        let ext_folder_id = encode_scoped_id(self.slot, local_folder_id).map_err(|e| {
            rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(e)))
        })?;
        Ok(MessageSummary {
            id: ext_id,
            account_id: self.account_id,
            folder_id: ext_folder_id,
            folder_path: row.get(2)?,
            uid: row.get(3)?,
            sender: row.get(4)?,
            subject: row.get(5)?,
            preview: row.get(6)?,
            date_ts: row.get(7)?,
            date_display: row.get(8)?,
            unread: row.get::<_, i32>(9)? != 0,
            flagged: row.get::<_, i32>(10)? != 0,
            has_attachment: row.get::<_, i32>(11)? != 0,
            recipients: row.get(12)?,
            thread_count: 1,
            importance: row.get::<_, i32>(13)?.clamp(-1, 1) as i8,
            tags,
            answered: row.get::<_, i32>(15)? != 0,
            forwarded: row.get::<_, i32>(16)? != 0,
            snoozed_until_ts: row.get(17)?,
        })
    }
}

#[inline]
fn c_from(c: Connection) -> Connection {
    c
}

pub(crate) struct CopiedMessagePayload {
    message_id: String,
    in_reply_to: String,
    references_header: String,
    sender: String,
    recipients: String,
    cc: String,
    subject: String,
    thread_subject: String,
    preview: String,
    date_ts: i64,
    date_display: String,
    unread: i32,
    flagged: i32,
    has_attachment: i32,
    body_loaded: i32,
    body_cached_at: i64,
    index_text: Option<String>,
    list_unsubscribe: String,
    list_unsubscribe_post: i32,
    auth_results: String,
    importance: i32,
    tags: String,
    answered: i32,
    forwarded: i32,
    snoozed_until_ts: i64,
    body: Option<(String, Option<String>)>,
}

/// Format relative path inside account's `attachments/` using 256 two-hex-digit buckets.
pub(crate) fn sharded_attachment_rel_path(local_att_id: u32) -> String {
    format!("{:02x}/{local_att_id}.bin", local_att_id & 0xff)
}

/// Safely resolve either sharded (`<xx>/<id>.bin`) or flat (`<id>.bin`) relative attachment path.
pub(crate) fn resolve_attachment_disk_path(
    attachments_dir: Option<&Path>,
    storage_path: &str,
) -> Option<PathBuf> {
    let dir = attachments_dir?;
    let trimmed = storage_path.trim().trim_start_matches(['/', '\\']);
    if trimmed.is_empty() || trimmed.contains('\0') {
        return None;
    }
    let rel = Path::new(trimmed);
    let mut comps = rel.components();
    let valid = match (comps.next(), comps.next(), comps.next()) {
        (Some(std::path::Component::Normal(_)), None, None) => true,
        (Some(std::path::Component::Normal(_)), Some(std::path::Component::Normal(_)), None) => {
            true
        }
        _ => false,
    };
    valid.then(|| dir.join(rel))
}

pub(crate) fn delete_attachment_files(attachments_dir: Option<&Path>, paths: &[String]) {
    for rel in paths {
        if let Some(full) = resolve_attachment_disk_path(attachments_dir, rel) {
            let _ = std::fs::remove_file(full);
        }
    }
}

pub(crate) fn write_attachment_file(
    dir: &Path,
    local_att_id: u32,
    bytes: &[u8],
) -> Result<String, String> {
    let bucket_dir = dir.join(format!("{:02x}", local_att_id & 0xff));
    std::fs::create_dir_all(&bucket_dir)
        .map_err(|e| format!("创建附件分桶目录失败 ({}): {e}", bucket_dir.display()))?;
    let rel = sharded_attachment_rel_path(local_att_id);
    let target = dir.join(&rel);
    let tmp = bucket_dir.join(format!("{local_att_id}.bin.tmp"));
    std::fs::write(&tmp, bytes)
        .map_err(|e| format!("写入临时附件文件失败 ({}): {e}", tmp.display()))?;
    if target.exists() {
        let _ = std::fs::remove_file(&target);
    }
    if let Err(err) = std::fs::rename(&tmp, &target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("提交附件文件失败 ({}): {err}", target.display()));
    }
    Ok(rel)
}
