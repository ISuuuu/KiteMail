//! Global SQLite store (`global.db`) shared across all accounts:
//! `account_slots`, `store_meta`, `tags`, `user_contacts`, `templates`, `outbox`, `drafts`.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

use super::id_codec::MAX_SLOT;
use super::TagItem;
use crate::{LocalDraft, MailTemplate, OutboxItem, OutboxStatus, UserContact};

pub(crate) struct GlobalDb {
    pub(crate) conn: Connection,
    read_only: bool,
}

impl GlobalDb {
    pub(crate) fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("创建全局数据库目录失败 ({}): {e}", parent.display()))?;
        }
        let conn = Connection::open(path)
            .map_err(|e| format!("打开全局数据库失败 ({}): {e}", path.display()))?;
        let db = Self {
            conn,
            read_only: false,
        };
        db.configure_and_migrate(true)?;
        Ok(db)
    }

    pub(crate) fn open_existing(path: &Path) -> Result<Self, String> {
        let flags =
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(path, flags)
            .map_err(|e| format!("打开已有全局数据库失败 ({}): {e}", path.display()))?;
        conn.execute_batch(
            "PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL; PRAGMA busy_timeout = 5000;",
        )
        .map_err(|e| format!("配置已有全局数据库失败: {e}"))?;
        Ok(Self {
            conn,
            read_only: false,
        })
    }

    pub(crate) fn open_read_only(path: &Path) -> Result<Self, String> {
        let flags =
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(path, flags)
            .map_err(|e| format!("以只读方式打开全局数据库失败 ({}): {e}", path.display()))?;
        conn.execute_batch("PRAGMA busy_timeout = 2000; PRAGMA query_only = ON;")
            .map_err(|e| format!("配置只读全局数据库失败: {e}"))?;
        Ok(Self {
            conn,
            read_only: true,
        })
    }

    pub(crate) fn open_in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| format!("打开内存全局库失败: {e}"))?;
        let db = Self {
            conn,
            read_only: false,
        };
        db.configure_and_migrate(false)?;
        Ok(db)
    }

    fn configure_and_migrate(&self, enable_wal: bool) -> Result<(), String> {
        if enable_wal {
            self.conn
                .execute_batch("PRAGMA journal_mode = WAL;")
                .map_err(|e| format!("启用全局库 WAL 模式失败: {e}"))?;
        }
        self.conn
            .execute_batch(
                "
                PRAGMA foreign_keys = ON;
                PRAGMA synchronous = NORMAL;
                PRAGMA busy_timeout = 5000;

                CREATE TABLE IF NOT EXISTS store_meta (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS account_slots (
                    account_id INTEGER PRIMARY KEY,
                    slot INTEGER NOT NULL UNIQUE CHECK(slot BETWEEN 1 AND 254),
                    storage_dir TEXT NOT NULL DEFAULT ''
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

                CREATE TABLE IF NOT EXISTS user_contacts (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    email TEXT NOT NULL UNIQUE,
                    display_name TEXT NOT NULL DEFAULT '',
                    notes TEXT NOT NULL DEFAULT '',
                    phone TEXT NOT NULL DEFAULT '',
                    organization TEXT NOT NULL DEFAULT '',
                    job_title TEXT NOT NULL DEFAULT '',
                    birthday TEXT NOT NULL DEFAULT '',
                    groups_json TEXT NOT NULL DEFAULT '[]',
                    created_at INTEGER NOT NULL DEFAULT 0,
                    updated_at INTEGER NOT NULL DEFAULT 0
                );

                CREATE TABLE IF NOT EXISTS tags (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL UNIQUE,
                    color TEXT NOT NULL DEFAULT '#3584e4',
                    shortcut INTEGER
                );

                CREATE TABLE IF NOT EXISTS templates (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL,
                    to_addr TEXT NOT NULL DEFAULT '',
                    cc_addr TEXT NOT NULL DEFAULT '',
                    bcc_addr TEXT NOT NULL DEFAULT '',
                    subject TEXT NOT NULL DEFAULT '',
                    body_text TEXT NOT NULL DEFAULT '',
                    body_format TEXT NOT NULL DEFAULT '',
                    attachment_paths_json TEXT NOT NULL DEFAULT '[]',
                    created_at INTEGER NOT NULL
                );
                ",
            )
            .map_err(|e| format!("初始化全局库表结构失败: {e}"))?;

        for (table, col, def) in [
            ("outbox", "cc_addr", "TEXT NOT NULL DEFAULT ''"),
            ("outbox", "bcc_addr", "TEXT NOT NULL DEFAULT ''"),
            ("outbox", "send_at", "INTEGER"),
            ("outbox", "from_addr", "TEXT NOT NULL DEFAULT ''"),
            ("outbox", "reply_to", "TEXT NOT NULL DEFAULT ''"),
            ("outbox", "body_html", "TEXT"),
            ("drafts", "cc_addr", "TEXT NOT NULL DEFAULT ''"),
            ("drafts", "bcc_addr", "TEXT NOT NULL DEFAULT ''"),
            ("drafts", "from_addr", "TEXT NOT NULL DEFAULT ''"),
            ("drafts", "reply_to", "TEXT NOT NULL DEFAULT ''"),
            ("drafts", "body_format", "TEXT NOT NULL DEFAULT ''"),
            ("user_contacts", "phone", "TEXT NOT NULL DEFAULT ''"),
            ("user_contacts", "organization", "TEXT NOT NULL DEFAULT ''"),
            ("user_contacts", "job_title", "TEXT NOT NULL DEFAULT ''"),
            ("user_contacts", "birthday", "TEXT NOT NULL DEFAULT ''"),
            ("user_contacts", "groups_json", "TEXT NOT NULL DEFAULT '[]'"),
        ] {
            ensure_column(&self.conn, table, col, def)?;
        }

        Ok(())
    }

    pub(crate) fn checkpoint(&self) {
        if !self.read_only {
            let _ = self.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
        }
    }

    pub(crate) fn get_meta(&self, key: &str) -> Result<Option<String>, String> {
        self.conn
            .query_row(
                "SELECT value FROM store_meta WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| format!("读取全局元数据失败 ({key}): {e}"))
    }

    pub(crate) fn set_meta(&self, key: &str, value: &str) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO store_meta (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map_err(|e| format!("写入全局元数据失败 ({key}): {e}"))?;
        Ok(())
    }

    pub(crate) fn list_account_slots(&self) -> Result<Vec<(u32, u8, String)>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT account_id, slot, storage_dir FROM account_slots ORDER BY account_id ASC",
            )
            .map_err(|e| format!("查询账号槽位列表失败: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, u32>(0)?,
                    row.get::<_, u8>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| format!("读取账号槽位列表失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析账号槽位列表失败: {e}"))?;
        Ok(rows)
    }

    pub(crate) fn ensure_account_slot(
        &self,
        account_id: u32,
        preferred_dir: Option<&str>,
    ) -> Result<(u8, String), String> {
        if let Some((slot, existing_dir)) = self
            .conn
            .query_row(
                "SELECT slot, storage_dir FROM account_slots WHERE account_id = ?1",
                params![account_id],
                |r| Ok((r.get::<_, u8>(0)?, r.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|e| format!("查询账号槽位失败: {e}"))?
        {
            let dir = match preferred_dir.map(str::trim).filter(|s| !s.is_empty()) {
                Some(pref)
                    if existing_dir.is_empty()
                        || (existing_dir != pref
                            && existing_dir == format!("account_{account_id}")) =>
                {
                    if !self.read_only {
                        let _ = self.conn.execute(
                            "UPDATE account_slots SET storage_dir = ?1 WHERE account_id = ?2",
                            params![pref, account_id],
                        );
                    }
                    pref.to_string()
                }
                _ if !existing_dir.is_empty() => existing_dir,
                _ => preferred_dir
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("account_{account_id}")),
            };
            return Ok((slot, dir));
        }

        if self.read_only {
            return Err(format!("只读模式下找不到账号 #{account_id} 的存储槽位"));
        }

        let used_slots: Vec<u8> = {
            let mut stmt = self
                .conn
                .prepare("SELECT slot FROM account_slots ORDER BY slot ASC")
                .map_err(|e| format!("读取已占槽位失败: {e}"))?;
            let collected = stmt
                .query_map([], |r| r.get::<_, u8>(0))
                .map_err(|e| format!("遍历已占槽位失败: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("收集已占槽位失败: {e}"))?;
            collected
        };
        let next_slot = (1..=MAX_SLOT)
            .find(|s| !used_slots.contains(s))
            .ok_or_else(|| format!("本地账号槽位已满（最多支持 {MAX_SLOT} 个账号）"))?;
        let dir = preferred_dir
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("account_{account_id}"));

        self.conn
            .execute(
                "INSERT INTO account_slots (account_id, slot, storage_dir) VALUES (?1, ?2, ?3)",
                params![account_id, next_slot, dir],
            )
            .map_err(|e| format!("分配账号存储槽位失败: {e}"))?;

        Ok((next_slot, dir))
    }

    pub(crate) fn update_account_storage_dir(
        &self,
        account_id: u32,
        storage_dir: &str,
    ) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE account_slots SET storage_dir = ?1 WHERE account_id = ?2",
                params![storage_dir, account_id],
            )
            .map_err(|e| format!("更新账号存储目录失败: {e}"))?;
        Ok(())
    }

    pub(crate) fn remove_account_records(
        &mut self,
        account_id: u32,
    ) -> Result<Option<String>, String> {
        let tx = self
            .conn
            .transaction()
            .map_err(|e| format!("开启全局账号清理事务失败: {e}"))?;
        let storage_dir: Option<String> = tx
            .query_row(
                "SELECT storage_dir FROM account_slots WHERE account_id = ?1",
                params![account_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| format!("查询账号存储目录失败: {e}"))?;
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
        tx.execute(
            "DELETE FROM account_slots WHERE account_id = ?1",
            params![account_id],
        )
        .map_err(|e| format!("清理账号槽位失败: {e}"))?;
        tx.commit()
            .map_err(|e| format!("提交全局账号清理事务失败: {e}"))?;
        Ok(storage_dir.filter(|s| !s.trim().is_empty()))
    }

    pub(crate) fn list_user_contacts(&self) -> Result<Vec<UserContact>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "
                SELECT id, email, display_name, notes, phone, organization, job_title, birthday, groups_json
                FROM user_contacts
                ORDER BY id ASC
                ",
            )
            .map_err(|e| format!("查询用户联系人失败: {e}"))?;
        let rows = stmt
            .query_map([], |row| {
                let groups_json: String = row.get(8)?;
                let groups: Vec<String> = serde_json::from_str(&groups_json).unwrap_or_default();
                Ok(UserContact {
                    id: row.get::<_, i64>(0)? as u32,
                    email: row.get(1)?,
                    display_name: row.get(2)?,
                    notes: row.get(3)?,
                    phone: row.get(4)?,
                    organization: row.get(5)?,
                    job_title: row.get(6)?,
                    birthday: row.get(7)?,
                    groups,
                })
            })
            .map_err(|e| format!("读取用户联系人失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析用户联系人失败: {e}"))?;
        Ok(rows)
    }

    pub(crate) fn list_user_contact_emails_lower(&self) -> Vec<String> {
        let Ok(mut stmt) = self
            .conn
            .prepare("SELECT LOWER(TRIM(email)) FROM user_contacts WHERE TRIM(email) != ''")
        else {
            return Vec::new();
        };
        let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) else {
            return Vec::new();
        };
        rows.filter_map(Result::ok).collect()
    }

    pub(crate) fn upsert_user_contact(&mut self, contact: &UserContact) -> Result<u32, String> {
        let email = contact.email.trim().to_string();
        if !email.contains('@') {
            return Err("邮箱地址不合法".to_string());
        }
        let now = chrono::Utc::now().timestamp();
        let display_name = contact.display_name.trim().to_string();
        let notes = contact.notes.clone();
        let phone = contact.phone.trim().to_string();
        let organization = contact.organization.trim().to_string();
        let job_title = contact.job_title.trim().to_string();
        let birthday = contact.birthday.trim().to_string();
        let groups_json =
            serde_json::to_string(&contact.groups).unwrap_or_else(|_| "[]".to_string());
        if contact.id == 0 {
            self.conn
                .execute(
                    "
                    INSERT INTO user_contacts (
                        email, display_name, notes, phone, organization, job_title, birthday, groups_json,
                        created_at, updated_at
                    )
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
                    ",
                    params![
                        email,
                        display_name,
                        notes,
                        phone,
                        organization,
                        job_title,
                        birthday,
                        groups_json,
                        now
                    ],
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
                    SET email = ?1, display_name = ?2, notes = ?3,
                        phone = ?4, organization = ?5, job_title = ?6, birthday = ?7, groups_json = ?8,
                        updated_at = ?9
                    WHERE id = ?10
                    ",
                    params![
                        email,
                        display_name,
                        notes,
                        phone,
                        organization,
                        job_title,
                        birthday,
                        groups_json,
                        now,
                        contact.id
                    ],
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

    pub(crate) fn delete_user_contact(&mut self, contact_id: u32) -> Result<bool, String> {
        let affected = self
            .conn
            .execute(
                "DELETE FROM user_contacts WHERE id = ?1",
                params![contact_id],
            )
            .map_err(|e| format!("删除联系人失败: {e}"))?;
        Ok(affected > 0)
    }

    pub(crate) fn import_user_contacts(
        &mut self,
        contacts: &[UserContact],
    ) -> Result<(usize, usize), String> {
        let now = chrono::Utc::now().timestamp();
        let mut added = 0usize;
        let mut skipped = 0usize;
        for contact in contacts {
            let email = contact.email.trim();
            if !email.contains('@') {
                skipped += 1;
                continue;
            }
            let groups_json =
                serde_json::to_string(&contact.groups).unwrap_or_else(|_| "[]".to_string());
            let affected = self
                .conn
                .execute(
                    "
                    INSERT OR IGNORE INTO user_contacts (
                        email, display_name, notes, phone, organization, job_title, birthday, groups_json,
                        created_at, updated_at
                    )
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
                    ",
                    params![
                        email,
                        contact.display_name.trim(),
                        contact.notes,
                        contact.phone.trim(),
                        contact.organization.trim(),
                        contact.job_title.trim(),
                        contact.birthday.trim(),
                        groups_json,
                        now
                    ],
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

    pub(crate) fn list_tags(&self) -> Result<Vec<TagItem>, String> {
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

    pub(crate) fn upsert_tag(
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

    pub(crate) fn delete_tag(&mut self, tag_id: u32) -> Result<bool, String> {
        let affected = self
            .conn
            .execute("DELETE FROM tags WHERE id = ?1", params![tag_id])
            .map_err(|e| format!("删除标签失败: {e}"))?;
        Ok(affected > 0)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn enqueue_outbox_from_draft(
        &mut self,
        account_id: u32,
        to_addr: &str,
        cc_addr: &str,
        bcc_addr: &str,
        subject: &str,
        body_text: &str,
        in_reply_to: &str,
        from_addr: &str,
        reply_to: &str,
        body_html: Option<&str>,
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
                from_addr, reply_to, body_html, attachment_paths_json, status, last_error,
                created_at, send_at
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'queued', '', ?12, ?13)
            ",
            params![
                account_id,
                to_addr,
                cc_addr,
                bcc_addr,
                subject,
                body_text,
                in_reply_to,
                from_addr,
                reply_to,
                body_html,
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

    pub(crate) fn update_outbox_status(
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

    pub(crate) fn delete_outbox(&mut self, outbox_id: u32) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM outbox WHERE id = ?1", params![outbox_id])
            .map_err(|e| format!("删除发件箱记录失败: {e}"))?;
        Ok(())
    }

    pub(crate) fn claim_outbox_for_delivery(
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

    pub(crate) fn claim_outbox_for_retry(
        &mut self,
        outbox_id: u32,
    ) -> Result<Option<OutboxItem>, String> {
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

    fn load_outbox_row(&self, outbox_id: u32) -> Result<Option<OutboxItem>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "
                SELECT id, account_id, to_addr, cc_addr, bcc_addr, subject, body_text,
                       in_reply_to, from_addr, reply_to, attachment_paths_json, status, last_error,
                       body_html
                FROM outbox
                WHERE id = ?1
                ",
            )
            .map_err(|e| format!("查询发件箱失败: {e}"))?;
        let rows = stmt
            .query_map(params![outbox_id], |row| {
                let paths_json: String = row.get(10)?;
                let attachment_paths: Vec<String> =
                    serde_json::from_str(&paths_json).unwrap_or_default();
                let status_str: String = row.get(11)?;
                Ok(OutboxItem {
                    id: row.get(0)?,
                    account_id: row.get(1)?,
                    to_addr: row.get(2)?,
                    cc_addr: row.get(3)?,
                    bcc_addr: row.get(4)?,
                    subject: row.get(5)?,
                    body_text: row.get(6)?,
                    in_reply_to: row.get(7)?,
                    from_addr: row.get(8)?,
                    reply_to: row.get(9)?,
                    attachment_paths,
                    status: OutboxStatus::from_str(&status_str),
                    last_error: row.get(12)?,
                    body_html: row.get(13)?,
                })
            })
            .map_err(|e| format!("读取发件箱失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析发件箱行失败: {e}"))?;
        Ok(rows.into_iter().next())
    }

    pub(crate) fn list_due_queued_ids(
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

    pub(crate) fn next_queued_send_at(&self, now: i64) -> Result<Option<i64>, String> {
        self.conn
            .query_row(
                "
                SELECT MIN(send_at)
                FROM outbox
                WHERE status = 'queued' AND send_at IS NOT NULL AND send_at > ?1
                ",
                params![now],
                |row| row.get::<_, Option<i64>>(0),
            )
            .map_err(|e| format!("查询下一个定时发送时间失败: {e}"))
    }

    pub(crate) fn expire_queued_outbox(&mut self) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE outbox SET send_at = NULL WHERE status = 'queued' AND send_at IS NOT NULL",
                [],
            )
            .map_err(|e| format!("准备退出发件队列失败: {e}"))?;
        Ok(())
    }

    pub(crate) fn undo_queued_outbox(
        &mut self,
        account_id: u32,
        outbox_id: u32,
    ) -> Result<Option<u32>, String> {
        let tx = self
            .conn
            .transaction()
            .map_err(|e| format!("撤销发送事务失败: {e}"))?;
        let inserted = tx
            .execute(
                "INSERT INTO drafts (account_id, to_addr, cc_addr, bcc_addr, subject, body_text, in_reply_to, from_addr, reply_to, body_format, attachment_paths_json, updated_at)
                 SELECT account_id, to_addr, cc_addr, bcc_addr, subject, body_text, in_reply_to, from_addr, reply_to, '', attachment_paths_json, ?3
                 FROM outbox WHERE id = ?1 AND account_id = ?2 AND status = 'queued'",
                params![outbox_id, account_id, chrono::Utc::now().timestamp()],
            )
            .map_err(|e| format!("恢复草稿失败: {e}"))?;
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

    pub(crate) fn delete_queued_outbox(&mut self, outbox_id: u32) -> Result<bool, String> {
        let deleted = self
            .conn
            .execute(
                "DELETE FROM outbox WHERE id = ?1 AND status = 'queued'",
                params![outbox_id],
            )
            .map_err(|e| format!("删除发件箱记录失败: {e}"))?;
        Ok(deleted > 0)
    }

    pub(crate) fn list_outbox(&self, account_id: u32) -> Result<Vec<OutboxItem>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "
                SELECT id, account_id, to_addr, cc_addr, bcc_addr, subject, body_text,
                       in_reply_to, from_addr, reply_to, attachment_paths_json, status, last_error,
                       body_html
                FROM outbox
                WHERE account_id = ?1
                ORDER BY id DESC
                ",
            )
            .map_err(|e| format!("查询发件箱失败: {e}"))?;

        let rows = stmt
            .query_map(params![account_id], |row| {
                let paths_json: String = row.get(10)?;
                let attachment_paths: Vec<String> =
                    serde_json::from_str(&paths_json).unwrap_or_default();
                let status_str: String = row.get(11)?;
                Ok(OutboxItem {
                    id: row.get(0)?,
                    account_id: row.get(1)?,
                    to_addr: row.get(2)?,
                    cc_addr: row.get(3)?,
                    bcc_addr: row.get(4)?,
                    subject: row.get(5)?,
                    body_text: row.get(6)?,
                    in_reply_to: row.get(7)?,
                    from_addr: row.get(8)?,
                    reply_to: row.get(9)?,
                    attachment_paths,
                    status: OutboxStatus::from_str(&status_str),
                    last_error: row.get(12)?,
                    body_html: row.get(13)?,
                })
            })
            .map_err(|e| format!("读取发件箱失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析发件箱行失败: {e}"))?;
        Ok(rows)
    }

    pub(crate) fn upsert_draft(&mut self, draft: &LocalDraft) -> Result<u32, String> {
        let paths_json = serde_json::to_string(&draft.attachment_paths)
            .map_err(|e| format!("序列化草稿附件路径失败: {e}"))?;
        if draft.id == 0 {
            self.conn
                .execute(
                    "
                    INSERT INTO drafts (
                        account_id, to_addr, cc_addr, bcc_addr, subject, body_text,
                        in_reply_to, from_addr, reply_to, body_format,
                        attachment_paths_json, updated_at
                    )
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
                    ",
                    params![
                        draft.account_id,
                        draft.to_addr,
                        draft.cc_addr,
                        draft.bcc_addr,
                        draft.subject,
                        draft.body_text,
                        draft.in_reply_to,
                        draft.from_addr,
                        draft.reply_to,
                        draft.body_format,
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
                        from_addr = ?8, reply_to = ?9, body_format = ?10,
                        attachment_paths_json = ?11, updated_at = ?12
                    WHERE id = ?13
                    ",
                    params![
                        draft.account_id,
                        draft.to_addr,
                        draft.cc_addr,
                        draft.bcc_addr,
                        draft.subject,
                        draft.body_text,
                        draft.in_reply_to,
                        draft.from_addr,
                        draft.reply_to,
                        draft.body_format,
                        paths_json,
                        draft.updated_at,
                        draft.id
                    ],
                )
                .map_err(|e| format!("更新草稿失败: {e}"))?;
            Ok(draft.id)
        }
    }

    pub(crate) fn list_drafts(&self) -> Result<Vec<LocalDraft>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "
                SELECT id, account_id, to_addr, cc_addr, bcc_addr, subject, body_text,
                       in_reply_to, from_addr, reply_to, attachment_paths_json, updated_at,
                       body_format
                FROM drafts
                ORDER BY updated_at DESC, id DESC
                ",
            )
            .map_err(|e| format!("查询本地草稿失败: {e}"))?;

        let rows = stmt
            .query_map([], |row| {
                let paths_json: String = row.get(10)?;
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
                    from_addr: row.get(8)?,
                    reply_to: row.get(9)?,
                    attachment_paths,
                    updated_at: row.get(11)?,
                    body_format: row.get(12)?,
                })
            })
            .map_err(|e| format!("读取本地草稿失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析本地草稿失败: {e}"))?;
        Ok(rows)
    }

    pub(crate) fn list_templates(&self) -> Result<Vec<MailTemplate>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "
                SELECT id, name, to_addr, cc_addr, bcc_addr, subject, body_text,
                       body_format, attachment_paths_json, created_at
                FROM templates
                ORDER BY created_at DESC, id DESC
                ",
            )
            .map_err(|e| format!("查询写信模板失败: {e}"))?;

        let rows = stmt
            .query_map([], |row| {
                let paths_json: String = row.get(8)?;
                let attachment_paths: Vec<String> =
                    serde_json::from_str(&paths_json).unwrap_or_default();
                Ok(MailTemplate {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    to_addr: row.get(2)?,
                    cc_addr: row.get(3)?,
                    bcc_addr: row.get(4)?,
                    subject: row.get(5)?,
                    body_text: row.get(6)?,
                    body_format: row.get(7)?,
                    attachment_paths,
                    created_at: row.get(9)?,
                })
            })
            .map_err(|e| format!("读取写信模板失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析写信模板失败: {e}"))?;
        Ok(rows)
    }

    pub(crate) fn upsert_template(&mut self, template: &MailTemplate) -> Result<u32, String> {
        let paths_json =
            serde_json::to_string(&template.attachment_paths).unwrap_or_else(|_| "[]".to_string());
        if template.id == 0 {
            self.conn
                .execute(
                    "
                    INSERT INTO templates (
                        name, to_addr, cc_addr, bcc_addr, subject, body_text,
                        body_format, attachment_paths_json, created_at
                    )
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                    ",
                    params![
                        template.name,
                        template.to_addr,
                        template.cc_addr,
                        template.bcc_addr,
                        template.subject,
                        template.body_text,
                        template.body_format,
                        paths_json,
                        template.created_at
                    ],
                )
                .map_err(|e| format!("保存写信模板失败: {e}"))?;
            return Ok(self.conn.last_insert_rowid() as u32);
        }
        self.conn
            .execute(
                "
                UPDATE templates
                SET name = ?2, to_addr = ?3, cc_addr = ?4, bcc_addr = ?5, subject = ?6,
                    body_text = ?7, body_format = ?8, attachment_paths_json = ?9,
                    created_at = ?10
                WHERE id = ?1
                ",
                params![
                    template.id,
                    template.name,
                    template.to_addr,
                    template.cc_addr,
                    template.bcc_addr,
                    template.subject,
                    template.body_text,
                    template.body_format,
                    paths_json,
                    template.created_at
                ],
            )
            .map_err(|e| format!("更新写信模板失败: {e}"))?;
        Ok(template.id)
    }

    pub(crate) fn delete_template(&mut self, template_id: u32) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM templates WHERE id = ?1", params![template_id])
            .map_err(|e| format!("删除写信模板失败: {e}"))?;
        Ok(())
    }

    pub(crate) fn delete_draft(&mut self, draft_id: u32) -> Result<(), String> {
        self.conn
            .execute("DELETE FROM drafts WHERE id = ?1", params![draft_id])
            .map_err(|e| format!("删除草稿失败: {e}"))?;
        Ok(())
    }
}

pub(crate) fn ensure_column(
    conn: &Connection,
    table: &str,
    col: &str,
    def: &str,
) -> Result<(), String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|e| format!("读取表结构失败: {e}"))?;
    let exists = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| format!("遍历表字段失败: {e}"))?
        .filter_map(Result::ok)
        .any(|name| name == col);
    if !exists {
        conn.execute(&format!("ALTER TABLE {table} ADD COLUMN {col} {def}"), [])
            .map_err(|e| format!("迁移字段 {table}.{col} 失败: {e}"))?;
    }
    Ok(())
}
