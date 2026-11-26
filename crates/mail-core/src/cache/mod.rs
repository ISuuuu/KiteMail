//! Split SQLite mail cache:
//! - Global metadata (`global.db` in `app_data_dir`): `account_slots`, `tags`, `user_contacts`, `templates`, `outbox`, `drafts`
//! - Per-account storage (`<mail_data_dir>\<account_dir>\`): `mail.db` + lazy `content.db` + `attachments\<00..ff>\`
//!
//! External IDs (`folder_id`, `message_id`, `attachment_id`) encode `(slot << 24) | local_id` into `u32`
//! while keeping internal SQLite schemas inside each account folder free of any instance-specific IDs.

pub mod account_db;
pub mod global_db;
pub mod id_codec;
pub mod merge;
pub mod migrate;

#[cfg(test)]
mod tests;

use std::cell::{RefCell, RefMut};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use account_db::AccountDb;
use global_db::GlobalDb;
pub use id_codec::{decode_scoped_id, encode_scoped_id};
use merge::{
    compare_message_summaries, dedup_search_candidates, fold_contact_rows,
    parallel_account_disk_query,
};
pub use migrate::{
    copy_mail_dir_contents, dir_has_account_mail_data, inspect_mail_data_dir,
    inspect_mail_data_dir_with_app_dir, merge_imported_accounts, validate_target_mail_dir,
    DiscoveredAccountDir, MailDirInspection, MailDirSwitchMode,
};

use crate::config::{self, AccountConfig};
use crate::search_query::SearchQuery;
use crate::{
    AttachmentMeta, FolderSummary, LocalDraft, MailPage, MailTemplate, MessageDetail,
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
    /// 逗号分隔的抄送人（Cc 头 / Graph ccRecipients），无抄送时为空。
    pub cc: String,
    pub subject: String,
    pub preview: String,
    pub date_ts: i64,
    pub date_display: String,
    pub unread: bool,
    pub flagged: bool,
    pub has_attachment: bool,
}

/// Attachment payload extracted from a fetched RFC 822 / Graph message
/// (or metadata-only placeholder when the payload exceeds the auto-cache threshold).
#[derive(Clone, Debug)]
pub struct CachedAttachmentInput {
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: usize,
    pub content: Option<Vec<u8>>,
    pub remote_part: String,
    pub transfer_encoding: String,
}

impl CachedAttachmentInput {
    /// Construct an already-downloaded attachment payload.
    pub fn downloaded(
        filename: impl Into<String>,
        mime_type: impl Into<String>,
        content: Vec<u8>,
    ) -> Self {
        let size_bytes = content.len();
        Self {
            filename: filename.into(),
            mime_type: mime_type.into(),
            size_bytes,
            content: Some(content),
            remote_part: String::new(),
            transfer_encoding: String::new(),
        }
    }
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
    #[serde(default = "crate::default_true")]
    pub downloaded: bool,
}

/// Resolved attachment lookup result: either locally cached bytes or remote
/// download coordinates for an on-demand fetch.
#[derive(Clone, Debug)]
pub enum AttachmentRecord {
    Local {
        message_id: u32,
        filename: String,
        mime_type: String,
        bytes: Vec<u8>,
    },
    PendingRemote {
        message_id: u32,
        account_id: u32,
        folder_path: String,
        uid: u32,
        remote_msg_id: String,
        filename: String,
        mime_type: String,
        size_bytes: usize,
        remote_part: String,
        transfer_encoding: String,
    },
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
    /// WP-10：电话、组织、职位、生日、分组标签与发信历史统计。
    #[serde(default)]
    pub phone: String,
    #[serde(default)]
    pub organization: String,
    #[serde(default)]
    pub job_title: String,
    #[serde(default)]
    pub birthday: String,
    #[serde(default)]
    pub groups: Vec<String>,
    #[serde(default)]
    pub sent_count: usize,
}

/// SQL predicate matching every account's Sent folder by friendly name or raw path.
pub(crate) const SENT_FOLDER_MATCH: &str = "(f.name = '已发送' OR UPPER(f.path) LIKE '%SENT%')";

/// SQL predicate matching every account's Archive folder by friendly name or raw path.
pub(crate) const ARCHIVE_FOLDER_MATCH: &str = "(f.name = '归档' OR UPPER(f.path) LIKE '%ARCHIVE%')";

/// SQL predicate matching every account's Trash/Deleted folder by friendly name or raw path.
pub(crate) const TRASH_FOLDER_MATCH: &str =
    "(f.name = '已删除' OR UPPER(f.path) LIKE '%TRASH%' OR UPPER(f.path) LIKE '%DELETED%')";

/// SQL predicate matching every account's Junk/Spam folder by friendly name or raw path.
pub(crate) const JUNK_FOLDER_MATCH: &str =
    "(f.name = '垃圾邮件' OR UPPER(f.path) LIKE '%JUNK%' OR UPPER(f.path) LIKE '%SPAM%')";
pub(crate) const SPAM_FOLDER_MATCH: &str = JUNK_FOLDER_MATCH;

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

pub(crate) fn is_trash_folder(name: &str, path: &str) -> bool {
    let upper_path = path.to_ascii_uppercase();
    name == "已删除" || upper_path.contains("TRASH") || upper_path.contains("DELETED")
}

pub(crate) fn is_junk_folder(name: &str, path: &str) -> bool {
    let upper_path = path.to_ascii_uppercase();
    name == "垃圾邮件"
        || upper_path.contains("JUNK")
        || upper_path.contains("SPAM")
        || upper_path.contains("BULK")
}

/// Strip characters that Windows forbids in file names and trim surrounding whitespace / dots;
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

/// F-7：判断 `List-Unsubscribe-Post` 头是否为 RFC 8058 的 `List-Unsubscribe=One-Click`。
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

#[derive(Clone, Debug)]
enum StoreMode {
    Disk {
        app_data_dir: PathBuf,
        mail_data_dir: PathBuf,
        read_only: bool,
    },
    InMemory,
}

/// Split SQLite mail cache facade routing global tables to `global.db` and per-account mail data
/// to isolated `<mail_data_dir>/<email>/mail.db` + lazy `content.db` + sharded attachments.
pub struct CacheStore {
    mode: StoreMode,
    global: GlobalDb,
    accounts: RefCell<HashMap<u32, AccountDb>>,
    slot_to_account: RefCell<HashMap<u8, u32>>,
    account_to_slot: RefCell<HashMap<u32, (u8, String)>>,
    own_emails: Vec<String>,
}

impl CacheStore {
    /// Open or create the split cache storage rooted at `app_data_dir`, automatically resolving
    /// `mail_data_dir` via `storage.toml` and migrating any legacy `hylki-cache.db` if present.
    pub fn open_for_data_dir(app_data_dir: &Path) -> Result<Self, String> {
        let mail_data_dir = config::resolve_mail_data_dir_in(app_data_dir);
        Self::open_split_internal(app_data_dir, &mail_data_dir, None)
    }

    /// Open an already-initialized split cache storage in read-write mode (skipping migration).
    pub fn open_for_data_dir_existing(app_data_dir: &Path) -> Result<Self, String> {
        let global_path = config::global_db_in(app_data_dir);
        if !global_path.exists() {
            return Self::open_for_data_dir(app_data_dir);
        }
        let mail_data_dir = config::resolve_mail_data_dir_in(app_data_dir);
        let global = GlobalDb::open_existing(&global_path)?;
        let mut store = Self::from_parts(
            StoreMode::Disk {
                app_data_dir: app_data_dir.to_path_buf(),
                mail_data_dir,
                read_only: false,
            },
            global,
        )?;
        if let Ok(accounts) = config::load_accounts_in(app_data_dir) {
            let _ = store.sync_account_registry(&accounts);
        }
        Ok(store)
    }

    /// Open a read-only split cache connection for fast UI-thread lookups.
    pub fn open_for_data_dir_read_only(app_data_dir: &Path) -> Result<Self, String> {
        let global_path = config::global_db_in(app_data_dir);
        let mail_data_dir = config::resolve_mail_data_dir_in(app_data_dir);
        let global = GlobalDb::open_read_only(&global_path)?;
        Self::from_parts(
            StoreMode::Disk {
                app_data_dir: app_data_dir.to_path_buf(),
                mail_data_dir,
                read_only: true,
            },
            global,
        )
    }

    /// Open or create a split cache store using explicit `(app_data_dir, mail_data_dir)`.
    pub fn open_split(app_data_dir: &Path, mail_data_dir: &Path) -> Result<Self, String> {
        Self::open_split_internal(app_data_dir, mail_data_dir, None)
    }

    /// Compatibility wrapper: treats `path.parent()` as `app_data_dir` and triggers lossless
    /// migration if `path` itself is an existing legacy single-SQLite database file.
    pub fn open(path: &Path) -> Result<Self, String> {
        let app_data_dir = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let mail_data_dir = config::resolve_mail_data_dir_in(app_data_dir);
        let legacy_override = path.is_file().then_some(path);
        Self::open_split_internal(app_data_dir, &mail_data_dir, legacy_override)
    }

    /// Compatibility wrapper: opens existing split store rooted at `path.parent()`.
    pub fn open_existing(path: &Path) -> Result<Self, String> {
        let app_data_dir = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        Self::open_for_data_dir_existing(app_data_dir)
    }

    /// Compatibility wrapper: opens read-only split store rooted at `path.parent()`.
    pub fn open_read_only(path: &Path) -> Result<Self, String> {
        let app_data_dir = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        Self::open_for_data_dir_read_only(app_data_dir)
    }

    /// Open an isolated in-memory split cache database for unit tests.
    pub fn open_in_memory() -> Result<Self, String> {
        let global = GlobalDb::open_in_memory()?;
        Self::from_parts(StoreMode::InMemory, global)
    }

    fn open_split_internal(
        app_data_dir: &Path,
        mail_data_dir: &Path,
        legacy_override: Option<&Path>,
    ) -> Result<Self, String> {
        std::fs::create_dir_all(app_data_dir)
            .map_err(|e| format!("创建数据目录失败 ({}): {e}", app_data_dir.display()))?;
        std::fs::create_dir_all(mail_data_dir)
            .map_err(|e| format!("创建邮件存储目录失败 ({}): {e}", mail_data_dir.display()))?;

        let global_path = config::global_db_in(app_data_dir);
        let legacy_arg = if !global_path.exists() {
            legacy_override
        } else {
            None
        };
        let _ = migrate::migrate_legacy_cache_if_needed(app_data_dir, mail_data_dir, legacy_arg)?;

        let global = GlobalDb::open(&global_path)?;
        let mut store = Self::from_parts(
            StoreMode::Disk {
                app_data_dir: app_data_dir.to_path_buf(),
                mail_data_dir: mail_data_dir.to_path_buf(),
                read_only: false,
            },
            global,
        )?;
        if let Ok(accounts) = config::load_accounts_in(app_data_dir) {
            let _ = store.sync_account_registry(&accounts);
        }
        Ok(store)
    }

    fn from_parts(mode: StoreMode, global: GlobalDb) -> Result<Self, String> {
        let slots = global.list_account_slots()?;
        let mut slot_to_account = HashMap::new();
        let mut account_to_slot = HashMap::new();
        for (acc_id, slot, dir_name) in slots {
            slot_to_account.insert(slot, acc_id);
            account_to_slot.insert(acc_id, (slot, dir_name));
        }
        Ok(Self {
            mode,
            global,
            accounts: RefCell::new(HashMap::new()),
            slot_to_account: RefCell::new(slot_to_account),
            account_to_slot: RefCell::new(account_to_slot),
            own_emails: Vec::new(),
        })
    }

    /// Synchronize configured `accounts.toml` entries with `global.db` `account_slots`.
    pub fn sync_account_registry(&mut self, accounts: &[AccountConfig]) -> Result<(), String> {
        let read_only = matches!(
            self.mode,
            StoreMode::Disk {
                read_only: true,
                ..
            }
        );
        if read_only || matches!(self.mode, StoreMode::InMemory) {
            return Ok(());
        }
        for acc in accounts {
            let preferred = config::resolve_account_storage_dir(acc, accounts);
            let (slot, actual_dir) = self.global.ensure_account_slot(acc.id, Some(&preferred))?;
            if actual_dir != preferred && actual_dir.starts_with("account_") {
                let _ = self.global.update_account_storage_dir(acc.id, &preferred);
                self.account_to_slot
                    .borrow_mut()
                    .insert(acc.id, (slot, preferred));
            } else {
                self.account_to_slot
                    .borrow_mut()
                    .insert(acc.id, (slot, actual_dir));
            }
            self.slot_to_account.borrow_mut().insert(slot, acc.id);
        }
        Ok(())
    }

    /// Checkpoint all WAL logs and close every opened SQLite connection.
    pub fn checkpoint_and_close_all(&mut self) {
        let drained: Vec<(u32, AccountDb)> = self.accounts.borrow_mut().drain().collect();
        for (_, db) in drained {
            db.checkpoint();
            drop(db);
        }
        self.global.checkpoint();
    }

    /// Update configured account email addresses used by the `To Me` quick filter.
    pub fn set_own_emails(&mut self, emails: Vec<String>) {
        self.own_emails = emails
            .into_iter()
            .map(|e| e.trim().to_lowercase())
            .filter(|e| !e.is_empty())
            .collect();
    }

    fn refresh_slots_cache(&self) {
        if let Ok(slots) = self.global.list_account_slots() {
            let mut s2a = self.slot_to_account.borrow_mut();
            let mut a2s = self.account_to_slot.borrow_mut();
            s2a.clear();
            a2s.clear();
            for (acc_id, slot, dir_name) in slots {
                s2a.insert(slot, acc_id);
                a2s.insert(acc_id, (slot, dir_name));
            }
        }
    }

    fn resolve_preferred_dir_for_account(&self, account_id: u32) -> Option<String> {
        match &self.mode {
            StoreMode::Disk { app_data_dir, .. } => {
                let accounts = config::load_accounts_in(app_data_dir).ok()?;
                let acc = accounts.iter().find(|a| a.id == account_id)?;
                Some(config::resolve_account_storage_dir(acc, &accounts))
            }
            StoreMode::InMemory => Some(format!("account_{account_id}")),
        }
    }

    fn ensure_account_registered(&self, account_id: u32) -> Result<(u8, String), String> {
        if let Some(existing) = self.account_to_slot.borrow().get(&account_id).cloned() {
            return Ok(existing);
        }
        let preferred = self.resolve_preferred_dir_for_account(account_id);
        let (slot, dir_name) = self
            .global
            .ensure_account_slot(account_id, preferred.as_deref())?;
        self.slot_to_account.borrow_mut().insert(slot, account_id);
        self.account_to_slot
            .borrow_mut()
            .insert(account_id, (slot, dir_name.clone()));
        Ok((slot, dir_name))
    }

    fn account_id_for_slot(&self, slot: u8) -> Option<u32> {
        if let Some(&id) = self.slot_to_account.borrow().get(&slot) {
            return Some(id);
        }
        self.refresh_slots_cache();
        self.slot_to_account.borrow().get(&slot).copied()
    }

    fn has_account(&self, account_id: u32) -> bool {
        if self.accounts.borrow().contains_key(&account_id)
            || self.account_to_slot.borrow().contains_key(&account_id)
        {
            return true;
        }
        self.refresh_slots_cache();
        if self.account_to_slot.borrow().contains_key(&account_id) {
            return true;
        }
        if let StoreMode::Disk {
            app_data_dir,
            mail_data_dir,
            ..
        } = &self.mode
        {
            if let Ok(accounts) = config::load_accounts_in(app_data_dir) {
                if let Some(acc) = accounts.iter().find(|a| a.id == account_id) {
                    let dir_name = config::resolve_account_storage_dir(acc, &accounts);
                    if AccountDb::mail_db_path_in(&mail_data_dir.join(dir_name)).is_file() {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn open_or_get_account_for_write(
        &self,
        account_id: u32,
    ) -> Result<RefMut<'_, AccountDb>, String> {
        if matches!(
            self.mode,
            StoreMode::Disk {
                read_only: true,
                ..
            }
        ) {
            return Err("只读缓存连接不允许写入账号数据".to_string());
        }
        if !self.accounts.borrow().contains_key(&account_id) {
            let (slot, dir_name) = self.ensure_account_registered(account_id)?;
            let db = match &self.mode {
                StoreMode::InMemory => AccountDb::open_in_memory(account_id, slot)?,
                StoreMode::Disk { mail_data_dir, .. } => {
                    let dir = config::account_mail_dir_in(mail_data_dir, &dir_name)?;
                    AccountDb::open_in_dir(account_id, slot, &dir)?
                }
            };
            self.accounts.borrow_mut().insert(account_id, db);
        }
        let guard = self.accounts.borrow_mut();
        Ok(RefMut::map(guard, |map| map.get_mut(&account_id).unwrap()))
    }

    fn open_existing_account_for_read(
        &self,
        account_id: u32,
    ) -> Result<Option<RefMut<'_, AccountDb>>, String> {
        if self.accounts.borrow().contains_key(&account_id) {
            let guard = self.accounts.borrow_mut();
            return Ok(Some(RefMut::map(guard, |map| {
                map.get_mut(&account_id).unwrap()
            })));
        }
        if !self.has_account(account_id) {
            return Ok(None);
        }
        let Some((slot, dir_name)) = self.account_to_slot.borrow().get(&account_id).cloned() else {
            return Ok(None);
        };
        let db = match &self.mode {
            StoreMode::InMemory => return Ok(None),
            StoreMode::Disk {
                mail_data_dir,
                read_only,
                ..
            } => {
                let dir = config::account_mail_dir_in(mail_data_dir, &dir_name)?;
                if !AccountDb::mail_db_path_in(&dir).is_file() {
                    return Ok(None);
                }
                if *read_only {
                    match AccountDb::open_read_only_in_dir(account_id, slot, &dir)? {
                        Some(ro) => ro,
                        None => return Ok(None),
                    }
                } else {
                    AccountDb::open_existing_in_dir(account_id, slot, &dir)?
                }
            }
        };
        self.accounts.borrow_mut().insert(account_id, db);
        let guard = self.accounts.borrow_mut();
        Ok(Some(RefMut::map(guard, |map| {
            map.get_mut(&account_id).unwrap()
        })))
    }

    fn resolve_target_account_ids(&self, allowed: Option<&[u32]>) -> Vec<u32> {
        if let Some(ids) = allowed {
            let mut deduped = ids.to_vec();
            deduped.sort_unstable();
            deduped.dedup();
            return deduped;
        }
        self.all_known_account_ids()
    }

    fn all_known_account_ids(&self) -> Vec<u32> {
        if !matches!(self.mode, StoreMode::InMemory) {
            self.refresh_slots_cache();
        }
        let mut set: BTreeSet<u32> = self.account_to_slot.borrow().keys().copied().collect();
        set.extend(self.accounts.borrow().keys().copied());
        if let StoreMode::Disk {
            app_data_dir,
            mail_data_dir,
            ..
        } = &self.mode
        {
            if let Ok(accounts) = config::load_accounts_in(app_data_dir) {
                for acc in &accounts {
                    let dir_name = config::resolve_account_storage_dir(acc, &accounts);
                    if AccountDb::mail_db_path_in(&mail_data_dir.join(dir_name)).is_file() {
                        set.insert(acc.id);
                    }
                }
            }
        }
        set.into_iter().collect()
    }

    fn disk_query_targets(&self, account_ids: &[u32]) -> Option<Vec<(u32, u8, PathBuf)>> {
        let StoreMode::Disk { mail_data_dir, .. } = &self.mode else {
            return None;
        };
        if account_ids.len() < 2 {
            return None;
        }
        let mut out = Vec::new();
        for &acc_id in account_ids {
            if let Ok(Some(db)) = self.open_existing_account_for_read(acc_id) {
                db.checkpoint();
                let dir_name = self
                    .account_to_slot
                    .borrow()
                    .get(&acc_id)
                    .map(|(_, d)| d.clone())?;
                let dir = mail_data_dir.join(dir_name);
                if AccountDb::mail_db_path_in(&dir).is_file() {
                    out.push((acc_id, db.slot, dir));
                }
            }
        }
        (out.len() >= 2).then_some(out)
    }

    fn resolve_scoped_account(&self, external_id: u32) -> Result<Option<(u32, u32)>, String> {
        let Some((slot, local_id)) = decode_scoped_id(external_id) else {
            return Ok(None);
        };
        let Some(account_id) = self.account_id_for_slot(slot) else {
            return Ok(None);
        };
        Ok(Some((account_id, local_id)))
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
        let mut db = self.open_or_get_account_for_write(account_id)?;
        db.sync_folders_with_flags(remote_folders)
    }

    /// WP-07: Rename a cached folder's path and display name after IMAP `RENAME`.
    pub fn rename_folder_entry(
        &mut self,
        account_id: u32,
        folder_id: u32,
        old_path: &str,
        new_path: &str,
        new_name: &str,
        delimiter: Option<&str>,
    ) -> Result<Vec<FolderSummary>, String> {
        let Some((_, local_fid)) = decode_scoped_id(folder_id) else {
            return self.list_folders(account_id);
        };
        let mut db = self.open_or_get_account_for_write(account_id)?;
        db.rename_folder_entry(local_fid, old_path, new_path, new_name, delimiter)
    }

    /// WP-07: Remove a cached folder (and cascade-delete its cached messages) after IMAP `DELETE`.
    pub fn delete_folder_entry(
        &mut self,
        account_id: u32,
        folder_id: u32,
    ) -> Result<Vec<FolderSummary>, String> {
        let Some((_, local_fid)) = decode_scoped_id(folder_id) else {
            return self.list_folders(account_id);
        };
        let mut db = self.open_or_get_account_for_write(account_id)?;
        db.delete_folder_entry(local_fid)
    }

    /// List cached folders for `account_id`.
    pub fn list_folders(&self, account_id: u32) -> Result<Vec<FolderSummary>, String> {
        let Some(db) = self.open_existing_account_for_read(account_id)? else {
            return Ok(Vec::new());
        };
        db.list_folders()
    }

    /// Return `(account_id, path, has_more_remote, remote_next_link, min_cached_uid, cached_count)`
    /// for `folder_id` to drive incremental older-message fetching.
    pub fn folder_remote_cursor(
        &self,
        folder_id: u32,
    ) -> Result<Option<(u32, String, bool, String, Option<u32>, usize)>, String> {
        let Some((account_id, local_fid)) = self.resolve_scoped_account(folder_id)? else {
            return Ok(None);
        };
        let Some(db) = self.open_existing_account_for_read(account_id)? else {
            return Ok(None);
        };
        db.folder_remote_cursor(local_fid)
    }

    /// Return `(folder_id, account_id, path)` for all inbox folders that still have older remote mail.
    pub fn list_inbox_folders_with_more_remote(&self) -> Result<Vec<(u32, u32, String)>, String> {
        let mut all = Vec::new();
        for acc_id in self.all_known_account_ids() {
            if let Some(db) = self.open_existing_account_for_read(acc_id)? {
                all.extend(db.list_inbox_folders_with_more_remote()?);
            }
        }
        Ok(all)
    }

    /// 后台索引候选：尚未同步、仍有远端历史或有待索引正文的文件夹。
    pub fn list_backfill_folders(&self) -> Result<Vec<(u32, u32, String, bool)>, String> {
        let mut all = Vec::new();
        for acc_id in self.all_known_account_ids() {
            if let Some(db) = self.open_existing_account_for_read(acc_id)? {
                all.extend(db.list_backfill_folders()?);
            }
        }
        all.sort_by(|a, b| {
            let a_inbox = a.2.eq_ignore_ascii_case("INBOX");
            let b_inbox = b.2.eq_ignore_ascii_case("INBOX");
            b_inbox.cmp(&a_inbox).then_with(|| a.0.cmp(&b.0))
        });
        Ok(all)
    }

    /// 返回一批仍缺少可搜索正文的邮件，包括升级前缓存的邮件头。
    pub fn list_unindexed_messages(
        &self,
        folder_id: u32,
        limit: usize,
    ) -> Result<Vec<(u32, u32, String)>, String> {
        let Some((account_id, local_fid)) = self.resolve_scoped_account(folder_id)? else {
            return Ok(Vec::new());
        };
        let Some(db) = self.open_existing_account_for_read(account_id)? else {
            return Ok(Vec::new());
        };
        db.list_unindexed_messages(local_fid, limit)
    }

    /// 保存独立的搜索正文，空串表示已检查此邮件。
    pub fn store_index_text(&self, message_id: u32, text: &str) -> Result<(), String> {
        let Some((account_id, local_msg_id)) = self.resolve_scoped_account(message_id)? else {
            return Ok(());
        };
        let Some(db) = self.open_existing_account_for_read(account_id)? else {
            return Ok(());
        };
        db.store_index_text(local_msg_id, text)
    }

    /// Persist whether `folder_id` has more older remote messages and its Graph `@odata.nextLink`.
    pub fn set_folder_remote_cursor(
        &self,
        folder_id: u32,
        has_more_remote: bool,
        remote_next_link: &str,
    ) -> Result<(), String> {
        let Some((account_id, local_fid)) = self.resolve_scoped_account(folder_id)? else {
            return Ok(());
        };
        let Some(db) = self.open_existing_account_for_read(account_id)? else {
            return Ok(());
        };
        db.set_folder_remote_cursor(local_fid, has_more_remote, remote_next_link)
    }

    /// Look up folder path and stored UIDVALIDITY for `folder_id`.
    pub fn folder_meta(&self, folder_id: u32) -> Result<Option<(String, u32)>, String> {
        let Some((account_id, local_fid)) = self.resolve_scoped_account(folder_id)? else {
            return Ok(None);
        };
        let Some(db) = self.open_existing_account_for_read(account_id)? else {
            return Ok(None);
        };
        db.folder_meta(local_fid)
    }

    pub(crate) fn folder_cached_uids(&self, folder_id: u32) -> Result<HashSet<u32>, String> {
        let Some((account_id, local_fid)) = self.resolve_scoped_account(folder_id)? else {
            return Ok(HashSet::new());
        };
        let Some(db) = self.open_existing_account_for_read(account_id)? else {
            return Ok(HashSet::new());
        };
        db.folder_cached_uids(local_fid)
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
        let Some((_, local_fid)) = decode_scoped_id(folder_id) else {
            return Err("无效的文件夹 ID".to_string());
        };
        let mut db = self.open_or_get_account_for_write(account_id)?;
        db.commit_folder_headers_with_remote(local_fid, uid_validity, headers, prune_missing_uids)
    }

    /// Reconcile `unread` and `flagged` state for all cached messages in `folder_id`.
    pub fn reconcile_folder_flags(
        &mut self,
        folder_id: u32,
        flagged_uids: &HashSet<u32>,
        unseen_uids: &HashSet<u32>,
    ) -> Result<usize, String> {
        let Some((account_id, local_fid)) = self.resolve_scoped_account(folder_id)? else {
            return Ok(0);
        };
        let mut db = self.open_or_get_account_for_write(account_id)?;
        db.reconcile_folder_flags(local_fid, flagged_uids, unseen_uids)
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

    /// Query a page of cached message summaries for `folder_id`.
    pub fn list_messages_page_with_options(
        &self,
        folder_id: u32,
        query: &str,
        quick_filters: QuickFilterSet,
        sort: MessageSort,
        offset: usize,
        limit: usize,
    ) -> Result<MailPage, String> {
        let Some((account_id, local_fid)) = self.resolve_scoped_account(folder_id)? else {
            return Ok(MailPage {
                folder_id,
                offset,
                total: 0,
                items: Vec::new(),
            });
        };
        let Some(db) = self.open_existing_account_for_read(account_id)? else {
            return Ok(MailPage {
                folder_id,
                offset,
                total: 0,
                items: Vec::new(),
            });
        };
        let contact_emails = if quick_filters.from_contacts {
            self.global.list_user_contact_emails_lower()
        } else {
            Vec::new()
        };
        db.list_messages_page_with_options(
            local_fid,
            folder_id,
            query,
            quick_filters,
            sort,
            &self.own_emails,
            &contact_emails,
            offset,
            limit,
        )
    }

    pub fn list_unified_inbox(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<MailPage, String> {
        self.list_unified_inbox_filtered(query, MessageFilter::All, offset, limit)
    }

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
            true,
        )
    }

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
            true,
        )
    }

    pub fn list_starred_messages(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<MailPage, String> {
        self.list_starred_messages_filtered(query, MessageFilter::All, offset, limit)
    }

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
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn list_unread_messages_for_accounts(
        &self,
        allowed_account_ids: &[u32],
        query: &str,
        quick_filters: QuickFilterSet,
        sort: MessageSort,
        offset: usize,
        limit: usize,
    ) -> Result<MailPage, String> {
        let predicate =
            format!("m.unread = 1 AND NOT ({TRASH_FOLDER_MATCH} OR {JUNK_FOLDER_MATCH})");
        self.list_role_messages_with_options_for_accounts(
            &predicate,
            "未读邮件",
            Some(allowed_account_ids),
            query,
            quick_filters,
            sort,
            offset,
            limit,
            false,
        )
    }

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
            false,
        )
    }

    pub fn list_sent_messages(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<MailPage, String> {
        self.list_sent_messages_filtered(query, MessageFilter::All, offset, limit)
    }

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
            false,
        )
    }

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
            false,
        )
    }

    pub fn list_archived_messages(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<MailPage, String> {
        self.list_archived_messages_filtered(query, MessageFilter::All, offset, limit)
    }

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
            false,
        )
    }

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
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn list_junk_messages_for_accounts(
        &self,
        allowed_account_ids: &[u32],
        query: &str,
        quick_filters: QuickFilterSet,
        sort: MessageSort,
        offset: usize,
        limit: usize,
    ) -> Result<MailPage, String> {
        self.list_role_messages_with_options_for_accounts(
            JUNK_FOLDER_MATCH,
            "垃圾邮件",
            Some(allowed_account_ids),
            query,
            quick_filters,
            sort,
            offset,
            limit,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn list_trash_messages_for_accounts(
        &self,
        allowed_account_ids: &[u32],
        query: &str,
        quick_filters: QuickFilterSet,
        sort: MessageSort,
        offset: usize,
        limit: usize,
    ) -> Result<MailPage, String> {
        self.list_role_messages_with_options_for_accounts(
            TRASH_FOLDER_MATCH,
            "已删除",
            Some(allowed_account_ids),
            query,
            quick_filters,
            sort,
            offset,
            limit,
            false,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn list_snoozed_messages_for_accounts(
        &self,
        allowed_account_ids: &[u32],
        query: &str,
        quick_filters: QuickFilterSet,
        sort: MessageSort,
        offset: usize,
        limit: usize,
    ) -> Result<MailPage, String> {
        self.list_role_messages_with_options_for_accounts(
            "m.snoozed_until_ts > 0",
            "稍后处理",
            Some(allowed_account_ids),
            query,
            quick_filters,
            sort,
            offset,
            limit,
            false,
        )
    }

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
        collapse_threads: bool,
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
            collapse_threads,
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
        collapse_threads: bool,
    ) -> Result<MailPage, String> {
        let target_ids = self.resolve_target_account_ids(allowed_account_ids);
        if target_ids.is_empty() {
            return Ok(MailPage {
                folder_id: 0,
                offset,
                total: 0,
                items: Vec::new(),
            });
        }
        let contact_emails = if quick_filters.from_contacts {
            self.global.list_user_contact_emails_lower()
        } else {
            Vec::new()
        };
        let fetch_limit = offset.saturating_add(limit);

        let mut total = 0usize;
        let mut candidates: Vec<MessageSummary> = Vec::new();
        for acc_id in &target_ids {
            if let Some(db) = self.open_existing_account_for_read(*acc_id)? {
                let (acc_total, acc_items) = db.query_role_candidates(
                    predicate,
                    label,
                    query,
                    quick_filters,
                    sort,
                    &self.own_emails,
                    &contact_emails,
                    fetch_limit,
                    collapse_threads,
                )?;
                total = total.saturating_add(acc_total);
                candidates.extend(acc_items);
            }
        }

        candidates.sort_by(|a, b| compare_message_summaries(sort, a, b));
        let mut page_items: Vec<MessageSummary> =
            candidates.into_iter().skip(offset).take(limit).collect();
        self.fill_thread_counts_for_mixed(&mut page_items);

        Ok(MailPage {
            folder_id: 0,
            offset,
            total,
            items: page_items,
        })
    }

    /// Cross-account, cross-folder local search.
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
        let target_ids = self.resolve_target_account_ids(Some(allowed_account_ids));
        let parsed = SearchQuery::parse(query);
        let contact_emails = if quick_filters.from_contacts {
            self.global.list_user_contact_emails_lower()
        } else {
            Vec::new()
        };

        let own_emails = &self.own_emails;
        let all_candidates = if let Some(disk_targets) = self.disk_query_targets(&target_ids) {
            let ref_targets: Vec<(u32, u8, &Path)> = disk_targets
                .iter()
                .map(|(acc, slot, p)| (*acc, *slot, p.as_path()))
                .collect();
            let batches = parallel_account_disk_query(&ref_targets, &|db| {
                db.query_search_candidates(
                    &parsed,
                    quick_filters,
                    sort,
                    own_emails,
                    &contact_emails,
                )
            })?;
            batches
                .into_iter()
                .flat_map(|(_, rows)| rows)
                .collect::<Vec<_>>()
        } else {
            let mut combined = Vec::new();
            for acc_id in &target_ids {
                if let Some(db) = self.open_existing_account_for_read(*acc_id)? {
                    combined.extend(db.query_search_candidates(
                        &parsed,
                        quick_filters,
                        sort,
                        &self.own_emails,
                        &contact_emails,
                    )?);
                }
            }
            combined
        };

        let mut deduped = dedup_search_candidates(all_candidates);
        let total = deduped.len();
        deduped.sort_by(|a, b| compare_message_summaries(sort, a, b));
        let mut items: Vec<MessageSummary> = deduped.into_iter().skip(offset).take(limit).collect();
        self.fill_thread_counts_for_mixed(&mut items);

        Ok(MailPage {
            folder_id: 0,
            offset,
            total,
            items,
        })
    }

    fn fill_thread_counts_for_mixed(&self, items: &mut [MessageSummary]) {
        if items.is_empty() {
            return;
        }
        let mut by_account: HashMap<u32, Vec<usize>> = HashMap::new();
        for (idx, item) in items.iter().enumerate() {
            by_account.entry(item.account_id).or_default().push(idx);
        }
        for (acc_id, indices) in by_account {
            if let Ok(Some(db)) = self.open_existing_account_for_read(acc_id) {
                let mut sub: Vec<MessageSummary> =
                    indices.iter().map(|&i| items[i].clone()).collect();
                db.fill_thread_counts(&mut sub);
                for (pos, orig_idx) in indices.into_iter().enumerate() {
                    items[orig_idx].thread_count = sub[pos].thread_count;
                }
            }
        }
    }

    pub fn find_inbox_folder(&self, account_id: u32) -> Result<Option<(u32, String)>, String> {
        let Some(db) = self.open_existing_account_for_read(account_id)? else {
            return Ok(None);
        };
        db.find_inbox_folder()
    }

    pub fn list_summaries_by_uids(
        &self,
        folder_id: u32,
        uids: &[u32],
    ) -> Result<Vec<MessageSummary>, String> {
        let Some((account_id, local_fid)) = self.resolve_scoped_account(folder_id)? else {
            return Ok(Vec::new());
        };
        let Some(db) = self.open_existing_account_for_read(account_id)? else {
            return Ok(Vec::new());
        };
        db.list_summaries_by_uids(local_fid, uids)
    }

    pub fn get_message_detail(
        &self,
        message_id: u32,
    ) -> Result<Option<(MessageDetail, u32, bool)>, String> {
        self.get_message_detail_ordered(message_id, true)
    }

    pub fn get_message_detail_ordered(
        &self,
        message_id: u32,
        newest_first: bool,
    ) -> Result<Option<(MessageDetail, u32, bool)>, String> {
        let Some((account_id, local_msg_id)) = self.resolve_scoped_account(message_id)? else {
            return Ok(None);
        };
        let Some(db) = self.open_existing_account_for_read(account_id)? else {
            return Ok(None);
        };
        db.get_message_detail_ordered(local_msg_id, newest_first)
    }

    pub fn find_sent_folder(&self, account_id: u32) -> Result<Option<(u32, String)>, String> {
        let Some(db) = self.open_existing_account_for_read(account_id)? else {
            return Ok(None);
        };
        db.find_sent_folder()
    }

    pub fn list_thread_messages(
        &self,
        account_id: u32,
        message_id: u32,
        subject: &str,
        newest_first: bool,
    ) -> Result<Vec<ThreadMessage>, String> {
        let Some(db) = self.open_existing_account_for_read(account_id)? else {
            return Ok(Vec::new());
        };
        let local_anchor_id = if message_id == 0 {
            0
        } else {
            decode_scoped_id(message_id)
                .map(|(_, lid)| lid)
                .unwrap_or(0)
        };
        db.list_thread_messages(local_anchor_id, subject, newest_first)
    }

    pub fn store_message_body(
        &mut self,
        message_id: u32,
        body_text: &str,
        attachments: &[CachedAttachmentInput],
    ) -> Result<(), String> {
        self.store_message_body_with_html(
            message_id,
            body_text,
            None,
            attachments,
            None,
            false,
            None,
        )
    }

    pub fn store_message_body_with_html(
        &mut self,
        message_id: u32,
        body_text: &str,
        body_html: Option<&str>,
        attachments: &[CachedAttachmentInput],
        unsubscribe_url: Option<&str>,
        unsubscribe_one_click: bool,
        auth_results: Option<&str>,
    ) -> Result<(), String> {
        let Some((account_id, local_msg_id)) = self.resolve_scoped_account(message_id)? else {
            return Err("无法定位邮件所属账号".to_string());
        };
        let mut db = self.open_or_get_account_for_write(account_id)?;
        db.store_message_body_with_html(
            local_msg_id,
            body_text,
            body_html,
            attachments,
            unsubscribe_url,
            unsubscribe_one_click,
            auth_results,
        )
    }

    pub fn clear_message_bodies(&mut self) -> Result<usize, String> {
        let mut cleared = 0usize;
        for acc_id in self.all_known_account_ids() {
            if self.open_existing_account_for_read(acc_id)?.is_some() {
                let mut db = self.open_or_get_account_for_write(acc_id)?;
                cleared += db.clear_message_bodies()?;
            }
        }
        Ok(cleared)
    }

    pub fn prune_expired_downloads(
        &mut self,
        keep_days: u32,
        now_ts: i64,
    ) -> Result<usize, String> {
        if keep_days == 0 {
            return Ok(0);
        }
        let mut pruned = 0usize;
        for acc_id in self.all_known_account_ids() {
            if self.open_existing_account_for_read(acc_id)?.is_some() {
                let mut db = self.open_or_get_account_for_write(acc_id)?;
                pruned += db.prune_expired_downloads(keep_days, now_ts)?;
            }
        }
        Ok(pruned)
    }

    pub fn get_message_remote_id(&self, message_id: u32) -> Result<Option<String>, String> {
        let Some((account_id, local_msg_id)) = self.resolve_scoped_account(message_id)? else {
            return Ok(None);
        };
        let Some(db) = self.open_existing_account_for_read(account_id)? else {
            return Ok(None);
        };
        db.get_message_remote_id(local_msg_id)
    }

    pub fn list_cached_attachments(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<CachedAttachment>, String> {
        let fetch_limit = offset.saturating_add(limit);
        let mut combined = Vec::new();
        for acc_id in self.all_known_account_ids() {
            if let Some(db) = self.open_existing_account_for_read(acc_id)? {
                combined.extend(db.list_cached_attachments(query, fetch_limit)?);
            }
        }
        combined.sort_by(|a, b| b.date_ts.cmp(&a.date_ts).then_with(|| a.id.cmp(&b.id)));
        Ok(combined.into_iter().skip(offset).take(limit).collect())
    }

    pub fn list_known_contacts(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<KnownContact>, String> {
        let mut counts: HashMap<String, usize> = HashMap::new();
        let mut sent_counts: HashMap<String, usize> = HashMap::new();
        let mut names: HashMap<String, String> = HashMap::new();
        let mut emails: HashMap<String, String> = HashMap::new();

        let all_ids = self.all_known_account_ids();
        if let Some(disk_targets) = self.disk_query_targets(&all_ids) {
            let ref_targets: Vec<(u32, u8, &Path)> = disk_targets
                .iter()
                .map(|(acc, slot, p)| (*acc, *slot, p.as_path()))
                .collect();
            let batches = parallel_account_disk_query(&ref_targets, &|db| db.scan_contact_rows())?;
            for (_, rows) in batches {
                fold_contact_rows(rows, &mut counts, &mut sent_counts, &mut names, &mut emails);
            }
        } else {
            for acc_id in all_ids {
                if let Some(db) = self.open_existing_account_for_read(acc_id)? {
                    let rows = db.scan_contact_rows()?;
                    fold_contact_rows(rows, &mut counts, &mut sent_counts, &mut names, &mut emails);
                }
            }
        }

        let trimmed = query.trim().to_lowercase();
        let mut contacts: Vec<KnownContact> = counts
            .into_iter()
            .map(|(key, message_count)| KnownContact {
                name: names.get(&key).cloned().unwrap_or_default(),
                email: emails.get(&key).cloned().unwrap_or_else(|| key.clone()),
                message_count,
                sent_count: sent_counts.get(&key).copied().unwrap_or(0),
                ..KnownContact::default()
            })
            .collect();

        for user in self.list_user_contacts()? {
            let key = user.email.trim().to_lowercase();
            if key.is_empty() {
                continue;
            }
            if let Some(existing) = contacts.iter_mut().find(|c| c.email.to_lowercase() == key) {
                existing.id = user.id;
                existing.notes = user.notes;
                existing.phone = user.phone;
                existing.organization = user.organization;
                existing.job_title = user.job_title;
                existing.birthday = user.birthday;
                existing.groups = user.groups;
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
                    phone: user.phone,
                    organization: user.organization,
                    job_title: user.job_title,
                    birthday: user.birthday,
                    groups: user.groups,
                    sent_count: 0,
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

        contacts.sort_by(|a, b| {
            b.custom
                .cmp(&a.custom)
                .then_with(|| b.message_count.cmp(&a.message_count))
                .then_with(|| a.email.cmp(&b.email))
        });
        Ok(contacts.into_iter().skip(offset).take(limit).collect())
    }

    pub fn list_user_contacts(&self) -> Result<Vec<UserContact>, String> {
        self.global.list_user_contacts()
    }

    pub fn upsert_user_contact(&mut self, contact: &UserContact) -> Result<u32, String> {
        self.global.upsert_user_contact(contact)
    }

    pub fn delete_user_contact(&mut self, contact_id: u32) -> Result<bool, String> {
        self.global.delete_user_contact(contact_id)
    }

    pub fn list_tags(&self) -> Result<Vec<TagItem>, String> {
        self.global.list_tags()
    }

    pub fn upsert_tag(
        &mut self,
        id: Option<u32>,
        name: &str,
        color: &str,
        shortcut: Option<u8>,
    ) -> Result<TagItem, String> {
        self.global.upsert_tag(id, name, color, shortcut)
    }

    pub fn delete_tag(&mut self, tag_id: u32) -> Result<bool, String> {
        self.global.delete_tag(tag_id)
    }

    pub fn apply_filter_rules(
        &mut self,
        rules: &[config::FilterRule],
        accounts: &[AccountConfig],
    ) -> Result<usize, String> {
        let enabled_rules: Vec<&config::FilterRule> = rules
            .iter()
            .filter(|r| r.enabled && !r.value.trim().is_empty())
            .collect();
        if enabled_rules.is_empty() {
            return Ok(0);
        }
        let mut updated = 0usize;
        for acc_id in self.all_known_account_ids() {
            if self.open_existing_account_for_read(acc_id)?.is_some() {
                let acc_email = accounts
                    .iter()
                    .find(|a| a.id == acc_id)
                    .map(|a| a.email.as_str())
                    .unwrap_or("");
                let mut db = self.open_or_get_account_for_write(acc_id)?;
                updated += db.apply_filter_rules(acc_email, &enabled_rules)?;
            }
        }
        Ok(updated)
    }

    pub fn import_user_contacts(
        &mut self,
        contacts: &[UserContact],
    ) -> Result<(usize, usize), String> {
        self.global.import_user_contacts(contacts)
    }

    pub fn find_archive_folder(&self, account_id: u32) -> Result<Option<(u32, String)>, String> {
        let Some(db) = self.open_existing_account_for_read(account_id)? else {
            return Ok(None);
        };
        db.find_archive_folder()
    }

    pub fn find_trash_folder(&self, account_id: u32) -> Result<Option<(u32, String)>, String> {
        let Some(db) = self.open_existing_account_for_read(account_id)? else {
            return Ok(None);
        };
        db.find_trash_folder()
    }

    pub fn find_spam_folder(&self, account_id: u32) -> Result<Option<(u32, String)>, String> {
        let Some(db) = self.open_existing_account_for_read(account_id)? else {
            return Ok(None);
        };
        db.find_spam_folder()
    }

    pub fn remove_message(&mut self, message_id: u32) -> Result<Option<u32>, String> {
        let Some((account_id, local_msg_id)) = self.resolve_scoped_account(message_id)? else {
            return Ok(None);
        };
        if self.open_existing_account_for_read(account_id)?.is_none() {
            return Ok(None);
        }
        let mut db = self.open_or_get_account_for_write(account_id)?;
        db.remove_message(local_msg_id)
    }

    pub fn get_attachment_record(
        &self,
        attachment_id: u32,
    ) -> Result<Option<AttachmentRecord>, String> {
        let Some((account_id, local_att_id)) = self.resolve_scoped_account(attachment_id)? else {
            return Ok(None);
        };
        let Some(db) = self.open_existing_account_for_read(account_id)? else {
            return Ok(None);
        };
        db.get_attachment_record(local_att_id)
    }

    pub fn store_downloaded_attachment_bytes(
        &mut self,
        attachment_id: u32,
        bytes: &[u8],
    ) -> Result<usize, String> {
        let Some((account_id, local_att_id)) = self.resolve_scoped_account(attachment_id)? else {
            return Err("目标附件记录不存在".to_string());
        };
        let mut db = self.open_or_get_account_for_write(account_id)?;
        db.store_downloaded_attachment_bytes(local_att_id, bytes)
    }

    pub fn list_message_attachment_ids(&self, message_id: u32) -> Result<Vec<u32>, String> {
        let Some((account_id, local_msg_id)) = self.resolve_scoped_account(message_id)? else {
            return Ok(Vec::new());
        };
        let Some(db) = self.open_existing_account_for_read(account_id)? else {
            return Ok(Vec::new());
        };
        db.list_message_attachment_ids(local_msg_id)
    }

    pub fn get_attachment_bytes(
        &self,
        attachment_id: u32,
    ) -> Result<Option<(String, String, Vec<u8>)>, String> {
        let Some((account_id, local_att_id)) = self.resolve_scoped_account(attachment_id)? else {
            return Ok(None);
        };
        let Some(db) = self.open_existing_account_for_read(account_id)? else {
            return Ok(None);
        };
        db.get_attachment_bytes(local_att_id)
    }

    pub fn get_message_location(&self, message_id: u32) -> Result<Option<(String, u32)>, String> {
        let Some((account_id, local_msg_id)) = self.resolve_scoped_account(message_id)? else {
            return Ok(None);
        };
        let Some(db) = self.open_existing_account_for_read(account_id)? else {
            return Ok(None);
        };
        db.get_message_location(local_msg_id)
    }

    pub fn delete_attachment_and_update_uid(
        &self,
        message_id: u32,
        attachment_id: u32,
        new_uid: Option<u32>,
    ) -> Result<Vec<AttachmentMeta>, String> {
        let Some((account_id, local_msg_id)) = self.resolve_scoped_account(message_id)? else {
            return Ok(Vec::new());
        };
        let Some((att_acc_id, local_att_id)) = self.resolve_scoped_account(attachment_id)? else {
            return Ok(Vec::new());
        };
        if att_acc_id != account_id {
            return Err("邮件与附件所属账号不一致".to_string());
        }
        let Some(db) = self.open_existing_account_for_read(account_id)? else {
            return Ok(Vec::new());
        };
        db.delete_attachment_and_update_uid(local_msg_id, local_att_id, new_uid)
    }

    pub fn export_message_attachments(
        &self,
        message_id: u32,
        dest_dir: &Path,
    ) -> Result<Vec<PathBuf>, String> {
        let Some((account_id, local_msg_id)) = self.resolve_scoped_account(message_id)? else {
            return Ok(Vec::new());
        };
        let Some(db) = self.open_existing_account_for_read(account_id)? else {
            return Ok(Vec::new());
        };
        db.export_message_attachments(local_msg_id, dest_dir)
    }

    pub fn set_message_seen(&mut self, message_id: u32, seen: bool) -> Result<(), String> {
        let Some((account_id, local_msg_id)) = self.resolve_scoped_account(message_id)? else {
            return Ok(());
        };
        let mut db = self.open_or_get_account_for_write(account_id)?;
        db.set_message_seen(local_msg_id, seen)
    }

    pub fn set_folder_all_seen(&mut self, folder_id: u32) -> Result<(), String> {
        let Some((account_id, local_fid)) = self.resolve_scoped_account(folder_id)? else {
            return Ok(());
        };
        let mut db = self.open_or_get_account_for_write(account_id)?;
        db.set_folder_all_seen(local_fid)
    }

    pub fn clear_folder(&mut self, folder_id: u32) -> Result<(), String> {
        let Some((account_id, local_fid)) = self.resolve_scoped_account(folder_id)? else {
            return Ok(());
        };
        let mut db = self.open_or_get_account_for_write(account_id)?;
        db.clear_folder(local_fid)
    }

    pub fn folder_unread_remote_ids(&self, folder_id: u32) -> Result<Vec<(u32, String)>, String> {
        let Some((account_id, local_fid)) = self.resolve_scoped_account(folder_id)? else {
            return Ok(Vec::new());
        };
        let Some(db) = self.open_existing_account_for_read(account_id)? else {
            return Ok(Vec::new());
        };
        db.folder_unread_remote_ids(local_fid)
    }

    pub fn set_message_flagged(&mut self, message_id: u32, flagged: bool) -> Result<(), String> {
        let Some((account_id, local_msg_id)) = self.resolve_scoped_account(message_id)? else {
            return Ok(());
        };
        let mut db = self.open_or_get_account_for_write(account_id)?;
        db.set_message_flagged(local_msg_id, flagged)
    }

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
            "",
            "",
            "",
            None,
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
        from_addr: &str,
        reply_to: &str,
        body_format: &str,
        body_html: Option<&str>,
        attachment_paths: &[String],
        created_at: i64,
        send_at: Option<i64>,
        local_draft_id: Option<u32>,
    ) -> Result<u32, String> {
        self.global.enqueue_outbox_from_draft(
            account_id,
            to_addr,
            cc_addr,
            bcc_addr,
            subject,
            body_text,
            in_reply_to,
            from_addr,
            reply_to,
            body_format,
            body_html,
            attachment_paths,
            created_at,
            send_at,
            local_draft_id,
        )
    }

    pub fn update_outbox_status(
        &mut self,
        outbox_id: u32,
        status: OutboxStatus,
        last_error: &str,
    ) -> Result<(), String> {
        self.global
            .update_outbox_status(outbox_id, status, last_error)
    }

    pub fn delete_outbox(&mut self, outbox_id: u32) -> Result<(), String> {
        self.global.delete_outbox(outbox_id)
    }

    pub(crate) fn recover_interrupted_outbox(&self) -> Result<(), String> {
        self.global.recover_interrupted_outbox()
    }

    pub fn claim_outbox_for_delivery(
        &mut self,
        outbox_id: u32,
        now: i64,
    ) -> Result<Option<OutboxItem>, String> {
        self.global.claim_outbox_for_delivery(outbox_id, now)
    }

    pub fn claim_outbox_for_retry(&mut self, outbox_id: u32) -> Result<Option<OutboxItem>, String> {
        self.global.claim_outbox_for_retry(outbox_id)
    }

    pub fn list_due_queued_ids(
        &self,
        account_id: Option<u32>,
        now: i64,
    ) -> Result<Vec<(u32, u32)>, String> {
        self.global.list_due_queued_ids(account_id, now)
    }

    pub fn next_queued_send_at(&self, now: i64) -> Result<Option<i64>, String> {
        self.global.next_queued_send_at(now)
    }

    pub fn outbox_status(
        &self,
        account_id: u32,
        outbox_id: u32,
    ) -> Result<Option<OutboxStatus>, String> {
        self.global.load_outbox_status(account_id, outbox_id)
    }

    pub fn undo_queued_outbox(
        &mut self,
        account_id: u32,
        outbox_id: u32,
    ) -> Result<Option<u32>, String> {
        self.global.undo_queued_outbox(account_id, outbox_id)
    }

    pub fn delete_queued_outbox(&mut self, outbox_id: u32) -> Result<bool, String> {
        self.global.delete_queued_outbox(outbox_id)
    }

    pub fn list_outbox(&self, account_id: u32) -> Result<Vec<OutboxItem>, String> {
        self.global.list_outbox(account_id)
    }

    pub fn upsert_draft(&mut self, draft: &LocalDraft) -> Result<u32, String> {
        self.global.upsert_draft(draft)
    }

    pub fn list_drafts(&self) -> Result<Vec<LocalDraft>, String> {
        self.global.list_drafts()
    }

    pub(crate) fn referenced_attachment_paths(&self) -> Result<Vec<String>, String> {
        self.global.referenced_attachment_paths()
    }

    pub fn load_draft(&self, draft_id: u32) -> Result<Option<LocalDraft>, String> {
        self.global.load_draft(draft_id)
    }

    pub fn list_templates(&self) -> Result<Vec<MailTemplate>, String> {
        self.global.list_templates()
    }

    pub fn upsert_template(&mut self, template: &MailTemplate) -> Result<u32, String> {
        self.global.upsert_template(template)
    }

    pub fn delete_template(&mut self, template_id: u32) -> Result<(), String> {
        self.global.delete_template(template_id)
    }

    pub fn delete_draft(&mut self, draft_id: u32) -> Result<(), String> {
        self.global.delete_draft(draft_id)
    }

    /// Remove all cached folders, messages, outbox, drafts, and on-disk account folder for `account_id`.
    pub fn delete_account_data(&mut self, account_id: u32) -> Result<(), String> {
        let mut dir_to_remove: Option<PathBuf> = None;
        if let Some(db) = self.accounts.borrow_mut().remove(&account_id) {
            dir_to_remove = db.dir.clone();
            db.checkpoint();
            drop(db);
        }
        if let Some((slot, dir_name)) = self.account_to_slot.borrow_mut().remove(&account_id) {
            self.slot_to_account.borrow_mut().remove(&slot);
            if dir_to_remove.is_none() {
                if let StoreMode::Disk {
                    ref mail_data_dir, ..
                } = self.mode
                {
                    if let Ok(p) = config::account_mail_dir_in(mail_data_dir, &dir_name) {
                        dir_to_remove = Some(p);
                    }
                }
            }
        }
        let db_dir = self.global.remove_account_records(account_id)?;
        if dir_to_remove.is_none() {
            if let (Some(dir_name), StoreMode::Disk { mail_data_dir, .. }) = (db_dir, &self.mode) {
                dir_to_remove = config::account_mail_dir_in(mail_data_dir, &dir_name).ok();
            }
        }
        if let Some(dir) = dir_to_remove {
            if dir.exists() {
                let _ = std::fs::remove_dir_all(&dir);
            }
        }
        Ok(())
    }

    pub fn list_thread_summaries(
        &self,
        account_id: u32,
        head_message_id: u32,
        subject: &str,
    ) -> Result<Vec<MessageSummary>, String> {
        let Some(db) = self.open_existing_account_for_read(account_id)? else {
            return Ok(Vec::new());
        };
        let local_head_id = decode_scoped_id(head_message_id)
            .map(|(_, lid)| lid)
            .unwrap_or(0);
        db.list_thread_summaries(local_head_id, subject)
    }

    pub fn set_message_tags(&mut self, message_id: u32, tags: &[String]) -> Result<(), String> {
        let Some((account_id, local_msg_id)) = self.resolve_scoped_account(message_id)? else {
            return Ok(());
        };
        let mut db = self.open_or_get_account_for_write(account_id)?;
        db.set_message_tags(local_msg_id, tags)
    }

    pub fn snooze_message(&mut self, message_id: u32, until_ts: i64) -> Result<(), String> {
        let Some((account_id, local_msg_id)) = self.resolve_scoped_account(message_id)? else {
            return Ok(());
        };
        let mut db = self.open_or_get_account_for_write(account_id)?;
        db.snooze_message(local_msg_id, until_ts)
    }

    pub fn wake_due_snoozed(&mut self, now_ts: i64) -> Result<Vec<(u32, u32, String)>, String> {
        let mut woken = Vec::new();
        for acc_id in self.all_known_account_ids() {
            if self.open_existing_account_for_read(acc_id)?.is_some() {
                let mut db = self.open_or_get_account_for_write(acc_id)?;
                woken.extend(db.wake_due_snoozed(now_ts)?);
            }
        }
        Ok(woken)
    }

    pub fn copy_message_to_folder(
        &mut self,
        message_id: u32,
        target_account_id: u32,
        target_folder_path: &str,
    ) -> Result<(), String> {
        let Some((src_acc_id, local_msg_id)) = self.resolve_scoped_account(message_id)? else {
            return Err("未找到要复制的源邮件".to_string());
        };
        if src_acc_id == target_account_id {
            let mut db = self.open_or_get_account_for_write(src_acc_id)?;
            return db.copy_message_within_same_account(local_msg_id, target_folder_path);
        }
        let payload = {
            let Some(src_db) = self.open_existing_account_for_read(src_acc_id)? else {
                return Err("未找到源账号缓存库".to_string());
            };
            let Some(p) = src_db.export_message_for_cross_account_copy(local_msg_id)? else {
                return Err("源邮件不存在".to_string());
            };
            p
        };
        let mut dst_db = self.open_or_get_account_for_write(target_account_id)?;
        dst_db.insert_copied_message(target_folder_path, &payload)
    }

    #[cfg(test)]
    pub(crate) fn global_conn(&self) -> &Connection {
        &self.global.conn
    }

    #[cfg(test)]
    pub(crate) fn with_account_mail_conn<T>(
        &mut self,
        account_id: u32,
        f: impl FnOnce(&Connection) -> T,
    ) -> T {
        let db = self.open_or_get_account_for_write(account_id).unwrap();
        f(&db.mail_conn)
    }

    #[cfg(test)]
    pub(crate) fn account_dir_for(&self, account_id: u32) -> Option<PathBuf> {
        self.accounts
            .borrow()
            .get(&account_id)
            .and_then(|db| db.dir.clone())
    }
}

pub(crate) fn sync_message_rfc_refs(
    conn: &Connection,
    message_row_id: u32,
    message_id: &str,
    in_reply_to: &str,
    references_header: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM message_rfc_refs WHERE message_row_id = ?1",
        params![message_row_id],
    )
    .map_err(|e| format!("清理旧 RFC 会话索引失败: {e}"))?;

    let mut tokens: HashSet<String> = HashSet::new();
    for tok in extract_rfc_message_ids(message_id)
        .into_iter()
        .chain(extract_rfc_message_ids(in_reply_to))
        .chain(extract_rfc_message_ids(references_header))
    {
        tokens.insert(tok);
    }
    if tokens.is_empty() {
        return Ok(());
    }

    let mut stmt = conn
        .prepare_cached(
            "INSERT OR IGNORE INTO message_rfc_refs (message_row_id, token)
             VALUES (?1, ?2)",
        )
        .map_err(|e| format!("准备写入 RFC 会话索引失败: {e}"))?;
    for tok in tokens {
        stmt.execute(params![message_row_id, tok])
            .map_err(|e| format!("写入 RFC 会话索引失败: {e}"))?;
    }
    Ok(())
}

pub(crate) fn collect_attachment_paths<P: rusqlite::Params>(
    conn: &Connection,
    sql: &str,
    params: P,
) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| format!("准备查询附件路径失败: {e}"))?;
    let rows = stmt
        .query_map(params, |row| row.get::<_, String>(0))
        .map_err(|e| format!("读取附件路径失败: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("收集附件路径失败: {e}"))?;
    Ok(rows)
}

pub(crate) fn split_address(raw: &str) -> (String, String) {
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

pub(crate) fn sanitize_attachment_filename(raw: &str) -> String {
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

pub(crate) fn deduplicate_filename(filename: &str, used: &mut HashSet<String>) -> String {
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
