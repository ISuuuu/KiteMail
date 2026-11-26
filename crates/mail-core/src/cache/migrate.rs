//! Legacy `hylki-cache.db` lossless migration into `global.db` + per-account DBs,
//! plus target `mail_data_dir` inspection, validation, and verified directory copy.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};

use super::account_db::{resolve_attachment_disk_path, write_attachment_file, AccountDb};
use super::global_db::GlobalDb;
use crate::config::{self, AccountConfig};

/// Mode chosen when changing `mail_data_dir`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MailDirSwitchMode {
    /// Point `storage.toml` at target directory and mount its existing per-account caches directly.
    ReuseExisting,
    /// Copy current mail directories into target directory, verify, and then switch.
    MigrateCurrent,
}

/// One existing per-account folder discovered while inspecting a directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredAccountDir {
    pub dir_name: String,
    pub total_bytes: u64,
    pub matched_email: Option<String>,
}

/// Summary returned after inspecting a candidate `mail_data_dir`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MailDirInspection {
    pub target_dir: PathBuf,
    pub existing_accounts: Vec<DiscoveredAccountDir>,
    pub total_bytes: u64,
    pub has_any_entries: bool,
    /// Whether the currently active `mail_data_dir` has any `<account>\mail.db` data.
    pub current_has_mail_data: bool,
    /// Accounts discovered in an external config directory associated with `target_dir`
    /// (e.g. `<target_dir>\..\accounts.toml`) that are not yet configured locally.
    pub importable_accounts: Vec<AccountConfig>,
    /// External config directory where `importable_accounts` were discovered, if any.
    pub importable_config_dir: Option<PathBuf>,
}

/// Return whether `dir` contains, or may contain, account mail data.
pub fn dir_has_account_mail_data(dir: &Path) -> bool {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return false,
        Err(_) => return true,
    };
    for entry in entries {
        let Ok(entry) = entry else { return true };
        // Inspection errors may indicate data that migration must not silently skip.
        if account_entry_has_mail_data(&entry).unwrap_or(true) {
            return true;
        }
    }
    false
}

/// Inspect a candidate `mail_data_dir` for existing `<account>\mail.db` folders.
pub fn inspect_mail_data_dir(target_dir: &Path, accounts: &[AccountConfig]) -> MailDirInspection {
    inspect_mail_data_dir_inner(None, target_dir, accounts)
}

/// Inspect a candidate `mail_data_dir` with full context of `app_data_dir`:
/// checks whether the current mail directory has data, and discovers any unconfigured
/// accounts from an external config directory associated with `target_dir`.
pub fn inspect_mail_data_dir_with_app_dir(
    app_data_dir: &Path,
    target_dir: &Path,
    accounts: &[AccountConfig],
) -> MailDirInspection {
    inspect_mail_data_dir_inner(Some(app_data_dir), target_dir, accounts)
}

fn inspect_mail_data_dir_inner(
    app_data_dir: Option<&Path>,
    target_dir: &Path,
    accounts: &[AccountConfig],
) -> MailDirInspection {
    let mut existing_accounts = Vec::new();
    let mut total_bytes = 0u64;
    let mut has_any_entries = false;

    if let Ok(entries) = fs::read_dir(target_dir) {
        for entry in entries.flatten() {
            has_any_entries = true;
            let path = entry.path();
            if entry.file_type().is_ok_and(|ft| ft.is_dir()) {
                let dir_name = entry.file_name().to_string_lossy().into_owned();
                if AccountDb::mail_db_path_in(&path).is_file() {
                    let stats = config::storage_stats_in(&path);
                    total_bytes = total_bytes.saturating_add(stats.total_bytes);
                    let matched_email = accounts
                        .iter()
                        .find(|acc| {
                            config::resolve_account_storage_dir(acc, accounts)
                                .eq_ignore_ascii_case(&dir_name)
                                || config::normalize_email_storage_dir(&acc.email)
                                    .eq_ignore_ascii_case(&dir_name)
                        })
                        .map(|acc| acc.email.clone());
                    existing_accounts.push(DiscoveredAccountDir {
                        dir_name,
                        total_bytes: stats.total_bytes,
                        matched_email,
                    });
                }
            }
        }
    }
    existing_accounts.sort_by(|a, b| a.dir_name.cmp(&b.dir_name));

    let current_has_mail_data = app_data_dir
        .map(|dir| dir_has_account_mail_data(&config::resolve_mail_data_dir_in(dir)))
        .unwrap_or(false);
    let (importable_config_dir, importable_accounts) =
        discover_importable_accounts(app_data_dir, target_dir, accounts);

    MailDirInspection {
        target_dir: target_dir.to_path_buf(),
        existing_accounts,
        total_bytes,
        has_any_entries,
        current_has_mail_data,
        importable_accounts,
        importable_config_dir,
    }
}

fn discover_importable_accounts(
    current_app_dir: Option<&Path>,
    target_dir: &Path,
    current_accounts: &[AccountConfig],
) -> (Option<PathBuf>, Vec<AccountConfig>) {
    let Some(parent) = target_dir.parent() else {
        return (None, Vec::new());
    };
    if !config::accounts_file_in(parent).is_file() {
        return (None, Vec::new());
    }
    if let Some(cur_app) = current_app_dir {
        if config::normalized_path_key(cur_app) == config::normalized_path_key(parent) {
            return (None, Vec::new());
        }
    }
    let target_key = config::normalized_path_key(target_dir);
    let parent_mail_key = config::normalized_path_key(&config::resolve_mail_data_dir_in(parent));
    let parent_default_key = config::normalized_path_key(&config::default_mail_data_dir_in(parent));
    if target_key != parent_mail_key && target_key != parent_default_key {
        return (None, Vec::new());
    }

    let Ok(source_accounts) = config::load_accounts_in(parent) else {
        return (None, Vec::new());
    };
    let mut importable = Vec::new();
    for acc in &source_accounts {
        let email = acc.email.trim();
        if email.is_empty()
            || current_accounts
                .iter()
                .any(|cur| cur.email.trim().eq_ignore_ascii_case(email))
            || importable
                .iter()
                .any(|seen: &AccountConfig| seen.email.trim().eq_ignore_ascii_case(email))
        {
            continue;
        }
        let mut cloned = acc.clone();
        cloned.avatar = None;
        let resolved = config::resolve_account_storage_dir(&cloned, &source_accounts);
        let legacy_slot_dir = format!("account_{}", cloned.id);
        if AccountDb::mail_db_path_in(&target_dir.join(&resolved)).is_file() {
            cloned.storage_dir = Some(resolved);
        } else if AccountDb::mail_db_path_in(&target_dir.join(&legacy_slot_dir)).is_file() {
            cloned.storage_dir = Some(legacy_slot_dir);
        }
        importable.push(cloned);
    }

    if importable.is_empty() {
        (None, Vec::new())
    } else {
        (Some(parent.to_path_buf()), importable)
    }
}

/// Merge `imported` accounts into `existing`, skipping empty or already-present emails
/// (case-insensitive) and assigning fresh unique account IDs.
pub fn merge_imported_accounts(
    existing: &[AccountConfig],
    imported: &[AccountConfig],
) -> (Vec<AccountConfig>, usize) {
    let mut merged = existing.to_vec();
    let mut added = 0usize;
    for acc in imported {
        let email = acc.email.trim();
        if email.is_empty()
            || merged
                .iter()
                .any(|cur| cur.email.trim().eq_ignore_ascii_case(email))
        {
            continue;
        }
        let mut next = acc.clone();
        next.id = config::next_account_id(&merged);
        next.avatar = None;
        merged.push(next);
        added += 1;
    }
    (merged, added)
}

/// Validate switching `mail_data_dir` from `current_dir` to `target_dir`:
/// rejects empty paths, same path, parent/child overlap, and unwritable paths.
pub fn validate_target_mail_dir(current_dir: &Path, target_dir: &Path) -> Result<(), String> {
    if target_dir.as_os_str().is_empty() {
        return Err("目标邮件存储目录不能为空".to_string());
    }

    let cur_key = config::normalized_path_key(current_dir);
    let dst_key = config::normalized_path_key(target_dir);
    if !cur_key.is_empty() && !dst_key.is_empty() {
        if cur_key == dst_key {
            return Err("目标目录与当前邮件存储目录相同".to_string());
        }
        let cur_prefix = format!("{cur_key}\\");
        if dst_key.starts_with(&cur_prefix) {
            return Err("不能将目标目录设在当前邮件目录内部".to_string());
        }
        let dst_prefix = format!("{dst_key}\\");
        if cur_key.starts_with(&dst_prefix) {
            return Err("不能将目标目录设为当前邮件目录的父目录".to_string());
        }
    }

    fs::create_dir_all(target_dir)
        .map_err(|e| format!("无法创建目标邮件目录 ({}): {e}", target_dir.display()))?;

    static NEXT_PROBE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let (probe, file) = loop {
        let probe = target_dir.join(format!(
            ".kitemail-write-probe-{}-{}.tmp",
            std::process::id(),
            NEXT_PROBE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)
        {
            Ok(file) => break (probe, file),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("目标目录不可写 ({}): {e}", target_dir.display())),
        }
    };
    drop(file);
    let _ = fs::remove_file(&probe);
    Ok(())
}

/// Copy valid per-account folders (`mail.db` present) from `src_dir` to `dst_dir`
/// (skipping transient `.db-wal` and `.db-shm` files) and verify copied file count and bytes.
pub fn copy_mail_dir_contents(src_dir: &Path, dst_dir: &Path) -> Result<(usize, u64), String> {
    let mut migration = MailDirMigration::copy(src_dir, dst_dir)?;
    let stats = migration.stats;
    migration.commit();
    Ok(stats)
}

/// Owns only paths created by this migration until the caller commits the switch.
pub(crate) struct MailDirMigration {
    staging: PathBuf,
    published: Vec<PathBuf>,
    stats: (usize, u64),
    committed: bool,
}

impl MailDirMigration {
    pub(crate) fn copy(src_dir: &Path, dst_dir: &Path) -> Result<Self, String> {
        fs::create_dir_all(dst_dir)
            .map_err(|e| format!("创建目标邮件目录失败 ({}): {e}", dst_dir.display()))?;
        let dst_dir = fs::canonicalize(dst_dir)
            .map_err(|e| format!("读取目标邮件目录失败 ({}): {e}", dst_dir.display()))?;
        let entries = match fs::read_dir(src_dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !src_dir.exists() => {
                return Ok(Self {
                    staging: PathBuf::new(),
                    published: Vec::new(),
                    stats: (0, 0),
                    committed: false,
                });
            }
            Err(e) => return Err(format!("读取源目录失败 ({}): {e}", src_dir.display())),
        };
        let mut accounts = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| format!("读取源目录项失败: {e}"))?;
            let path = entry.path();
            if account_entry_has_mail_data(&entry)? {
                let target = dst_dir.join(entry.file_name());
                ensure_target_free(&target)?;
                accounts.push((path, entry.file_name()));
            }
        }
        let staging = loop {
            static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = dst_dir.join(format!(".kitemail-migration-{}-{id}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => break path,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(format!("创建迁移暂存目录失败 ({}): {e}", path.display())),
            }
        };
        let mut migration = Self {
            staging,
            published: Vec::new(),
            stats: (0, 0),
            committed: false,
        };
        for (source, name) in &accounts {
            let (files, bytes) =
                copy_dir_recursive_filtered(source, &migration.staging.join(name))?;
            migration.stats.0 += files;
            migration.stats.1 += bytes;
        }
        for (_, name) in &accounts {
            let target = dst_dir.join(name);
            ensure_target_free(&target)?;
            if let Err(e) = publish_directory(&migration.staging.join(name), &target) {
                ensure_target_free(&target)?;
                return Err(format!("发布迁移目录失败 ({}): {e}", target.display()));
            }
            migration.published.push(target);
        }
        Ok(migration)
    }

    pub(crate) fn stats(&self) -> (usize, u64) {
        self.stats
    }

    pub(crate) fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for MailDirMigration {
    fn drop(&mut self) {
        if !self.committed {
            for path in &self.published {
                let _ = fs::remove_dir_all(path);
            }
        }
        if !self.staging.as_os_str().is_empty() {
            let _ = fs::remove_dir_all(&self.staging);
        }
    }
}

#[cfg(windows)]
fn publish_directory(source: &Path, target: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let target: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    // MoveFileW fails if the destination exists, including an empty directory.
    if unsafe {
        windows_sys::Win32::Storage::FileSystem::MoveFileW(source.as_ptr(), target.as_ptr())
    } == 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn publish_directory(source: &Path, target: &Path) -> std::io::Result<()> {
    // Reserve the destination atomically, then publish into the directory we own.
    fs::create_dir(target)?;
    match fs::rename(source, target) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = fs::remove_dir(target);
            Err(e)
        }
    }
}

fn ensure_target_free(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(format!(
            "目标已存在同名账号路径 ({})，请直接使用已有邮件数据（复用）或选择其他目录",
            path.display()
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("检查目标账号路径失败 ({}): {e}", path.display())),
    }
}

fn account_entry_has_mail_data(entry: &fs::DirEntry) -> Result<bool, String> {
    let path = entry.path();
    let meta = entry
        .metadata()
        .map_err(|e| format!("读取文件属性失败 ({}): {e}", path.display()))?;
    if metadata_is_reparse_point(&meta) {
        return Err(format!(
            "源邮件目录包含符号链接或重解析点，无法安全迁移 ({})",
            path.display()
        ));
    }
    if !meta.is_dir() {
        return Ok(false);
    }
    let db_path = AccountDb::mail_db_path_in(&path);
    let meta = match fs::symlink_metadata(&db_path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => {
            return Err(format!(
                "读取邮件数据库属性失败 ({}): {e}",
                db_path.display()
            ))
        }
    };
    if metadata_is_reparse_point(&meta) {
        return Err(format!(
            "源邮件目录包含符号链接或重解析点，无法安全迁移 ({})",
            db_path.display()
        ));
    }
    if !meta.is_file() {
        return Err(format!("邮件数据库不是普通文件 ({})", db_path.display()));
    }
    Ok(true)
}

fn is_reparse_point(path: &Path) -> Result<bool, String> {
    let meta = fs::symlink_metadata(path)
        .map_err(|e| format!("读取文件属性失败 ({}): {e}", path.display()))?;
    Ok(metadata_is_reparse_point(&meta))
}

fn metadata_is_reparse_point(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_type().is_symlink() || meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}

fn copy_dir_recursive_filtered(src: &Path, dst: &Path) -> Result<(usize, u64), String> {
    fs::create_dir_all(dst).map_err(|e| format!("创建目标子目录失败 ({}): {e}", dst.display()))?;
    let mut count = 0usize;
    let mut bytes = 0u64;
    let entries =
        fs::read_dir(src).map_err(|e| format!("读取源目录失败 ({}): {e}", src.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("读取目录项失败: {e}"))?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        let src_path = entry.path();
        let dst_path = dst.join(&name);
        let ft = entry
            .file_type()
            .map_err(|e| format!("读取文件类型失败 ({}): {e}", src_path.display()))?;
        if ft.is_symlink() || is_reparse_point(&src_path)? {
            return Err(format!(
                "源邮件目录包含符号链接或重解析点，无法安全迁移 ({})",
                src_path.display()
            ));
        }
        if ft.is_dir() {
            let (sub_c, sub_b) = copy_dir_recursive_filtered(&src_path, &dst_path)?;
            count += sub_c;
            bytes += sub_b;
        } else if ft.is_file() {
            if name_str.ends_with("-wal")
                || name_str.ends_with("-shm")
                || name_str.ends_with(".tmp")
            {
                continue;
            }
            let expected_len = fs::metadata(&src_path)
                .map_err(|e| format!("读取源文件元信息失败 ({}): {e}", src_path.display()))?
                .len();
            let written = fs::copy(&src_path, &dst_path).map_err(|e| {
                format!(
                    "复制文件失败 ({} -> {}): {e}",
                    src_path.display(),
                    dst_path.display()
                )
            })?;
            if written != expected_len {
                return Err(format!(
                    "校验复制文件长度不一致 ({}): 期望 {expected_len}, 实际 {written}",
                    dst_path.display()
                ));
            }
            count += 1;
            bytes += written;
        }
    }
    Ok((count, bytes))
}

/// Automatically migrate a legacy single SQLite cache (`hylki-cache.db` + flat `attachments/`)
/// into `global.db` + per-account `mail.db` / `content.db` / sharded attachments.
pub(crate) fn migrate_legacy_cache_if_needed(
    app_data_dir: &Path,
    mail_data_dir: &Path,
    legacy_db_path_override: Option<&Path>,
) -> Result<bool, String> {
    let default_legacy = config::cache_db_in(app_data_dir);
    let legacy_db = legacy_db_path_override.unwrap_or(&default_legacy);
    if !legacy_db.is_file() {
        return Ok(false);
    }

    let global_path = config::global_db_in(app_data_dir);
    let mut global = GlobalDb::open(&global_path)?;
    if global.get_meta("legacy_migrated")?.as_deref() == Some("1") {
        return Ok(false);
    }

    let legacy_attachments_dir = legacy_db.parent().map(|p| p.join("attachments"));
    let accounts_cfg = config::load_accounts_in(app_data_dir).unwrap_or_default();

    {
        let flags =
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let old = Connection::open_with_flags(legacy_db, flags).map_err(|e| {
            format!(
                "打开旧缓存数据库进行迁移失败 ({}): {e}",
                legacy_db.display()
            )
        })?;
        let _ = old.execute_batch("PRAGMA busy_timeout = 3000; PRAGMA query_only = ON;");

        migrate_global_tables(&old, &mut global)?;

        let old_account_ids = collect_legacy_account_ids(&old)?;
        for acc_id in old_account_ids {
            if acc_id == 0 {
                continue;
            }
            let dir_name = accounts_cfg
                .iter()
                .find(|a| a.id == acc_id)
                .map(|a| config::resolve_account_storage_dir(a, &accounts_cfg))
                .unwrap_or_else(|| format!("account_{acc_id}"));
            let (_slot, registered_dir) = global.ensure_account_slot(acc_id, Some(&dir_name))?;
            let account_dir = config::account_mail_dir_in(mail_data_dir, &registered_dir)?;
            let (slot, _) = global.ensure_account_slot(acc_id, Some(&registered_dir))?;
            migrate_one_account(
                &old,
                acc_id,
                slot,
                &account_dir,
                legacy_attachments_dir.as_deref(),
            )?;
        }
    }

    global.set_meta("legacy_migrated", "1")?;
    global.checkpoint();
    drop(global);

    backup_legacy_files(legacy_db, legacy_attachments_dir.as_deref());
    Ok(true)
}

fn table_exists(conn: &Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
        params![table],
        |_| Ok(()),
    )
    .optional()
    .ok()
    .flatten()
    .is_some()
}

fn has_column(conn: &Connection, table: &str, col: &str) -> bool {
    let Ok(mut stmt) = conn.prepare(&format!("PRAGMA table_info({table})")) else {
        return false;
    };
    let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(1)) else {
        return false;
    };
    let found = rows.filter_map(Result::ok).any(|n| n == col);
    found
}

fn opt_col_expr(conn: &Connection, table: &str, col: &str, fallback_sql: &str) -> String {
    if has_column(conn, table, col) {
        col.to_string()
    } else {
        format!("{fallback_sql} AS {col}")
    }
}

fn migrate_global_tables(old: &Connection, global: &mut GlobalDb) -> Result<(), String> {
    let tx = global
        .conn
        .transaction()
        .map_err(|e| format!("开启全局数据迁移事务失败: {e}"))?;

    if table_exists(old, "tags") {
        let mut stmt = old
            .prepare("SELECT id, name, color, shortcut FROM tags")
            .map_err(|e| format!("读取旧标签失败: {e}"))?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<i64>>(3)?,
                ))
            })
            .map_err(|e| format!("遍历旧标签失败: {e}"))?;
        for row in rows {
            let (id, name, color, shortcut) = row.map_err(|e| format!("解析旧标签失败: {e}"))?;
            tx.execute(
                "INSERT OR REPLACE INTO tags (id, name, color, shortcut) VALUES (?1, ?2, ?3, ?4)",
                params![id, name, color, shortcut],
            )
            .map_err(|e| format!("迁移标签失败: {e}"))?;
        }
    }

    if table_exists(old, "user_contacts") {
        let phone = opt_col_expr(old, "user_contacts", "phone", "''");
        let org = opt_col_expr(old, "user_contacts", "organization", "''");
        let title = opt_col_expr(old, "user_contacts", "job_title", "''");
        let bday = opt_col_expr(old, "user_contacts", "birthday", "''");
        let groups = opt_col_expr(old, "user_contacts", "groups_json", "'[]'");
        let created = opt_col_expr(old, "user_contacts", "created_at", "0");
        let updated = opt_col_expr(old, "user_contacts", "updated_at", "0");
        let mut stmt = old
            .prepare(&format!(
                "SELECT id, email, display_name, notes, {phone}, {org}, {title}, {bday}, {groups}, {created}, {updated} FROM user_contacts"
            ))
            .map_err(|e| format!("读取旧联系人失败: {e}"))?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, String>(8)?,
                    r.get::<_, i64>(9)?,
                    r.get::<_, i64>(10)?,
                ))
            })
            .map_err(|e| format!("遍历旧联系人失败: {e}"))?;
        for row in rows {
            let (id, email, name, notes, phone, org, title, bday, groups, created, updated) =
                row.map_err(|e| format!("解析旧联系人失败: {e}"))?;
            tx.execute(
                "INSERT OR REPLACE INTO user_contacts (id, email, display_name, notes, phone, organization, job_title, birthday, groups_json, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![id, email, name, notes, phone, org, title, bday, groups, created, updated],
            )
            .map_err(|e| format!("迁移联系人失败: {e}"))?;
        }
    }

    if table_exists(old, "templates") {
        let mut stmt = old
            .prepare("SELECT id, name, to_addr, cc_addr, bcc_addr, subject, body_text, body_format, attachment_paths_json, created_at FROM templates")
            .map_err(|e| format!("读取旧模板失败: {e}"))?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, String>(8)?,
                    r.get::<_, i64>(9)?,
                ))
            })
            .map_err(|e| format!("遍历旧模板失败: {e}"))?;
        for row in rows {
            let (
                id,
                name,
                to_addr,
                cc_addr,
                bcc_addr,
                subject,
                body_text,
                body_format,
                paths,
                created_at,
            ) = row.map_err(|e| format!("解析旧模板失败: {e}"))?;
            tx.execute(
                "INSERT OR REPLACE INTO templates (id, name, to_addr, cc_addr, bcc_addr, subject, body_text, body_format, attachment_paths_json, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![id, name, to_addr, cc_addr, bcc_addr, subject, body_text, body_format, paths, created_at],
            )
            .map_err(|e| format!("迁移模板失败: {e}"))?;
        }
    }

    if table_exists(old, "outbox") {
        let cc = opt_col_expr(old, "outbox", "cc_addr", "''");
        let bcc = opt_col_expr(old, "outbox", "bcc_addr", "''");
        let send_at = opt_col_expr(old, "outbox", "send_at", "NULL");
        let from_addr = opt_col_expr(old, "outbox", "from_addr", "''");
        let reply_to = opt_col_expr(old, "outbox", "reply_to", "''");
        let body_format = opt_col_expr(old, "outbox", "body_format", "''");
        let body_html = opt_col_expr(old, "outbox", "body_html", "NULL");
        let mut stmt = old
            .prepare(&format!(
                "SELECT id, account_id, to_addr, {cc}, {bcc}, subject, body_text, in_reply_to, {from_addr}, {reply_to}, {body_format}, {body_html}, attachment_paths_json, status, last_error, created_at, {send_at} FROM outbox"
            ))
            .map_err(|e| format!("读取旧发件箱失败: {e}"))?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, u32>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, String>(8)?,
                    r.get::<_, String>(9)?,
                    r.get::<_, String>(10)?,
                    r.get::<_, Option<String>>(11)?,
                    r.get::<_, String>(12)?,
                    r.get::<_, String>(13)?,
                    r.get::<_, String>(14)?,
                    r.get::<_, i64>(15)?,
                    r.get::<_, Option<i64>>(16)?,
                ))
            })
            .map_err(|e| format!("遍历旧发件箱失败: {e}"))?;
        for row in rows {
            let (
                id,
                acc,
                to_a,
                cc_a,
                bcc_a,
                subj,
                body,
                irt,
                from_a,
                rto,
                fmt,
                html,
                paths,
                st,
                err,
                created,
                send_at,
            ) = row.map_err(|e| format!("解析旧发件箱失败: {e}"))?;
            tx.execute(
                "INSERT OR REPLACE INTO outbox (id, account_id, to_addr, cc_addr, bcc_addr, subject, body_text, in_reply_to, from_addr, reply_to, body_format, body_html, attachment_paths_json, status, last_error, created_at, send_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
                params![id, acc, to_a, cc_a, bcc_a, subj, body, irt, from_a, rto, fmt, html, paths, st, err, created, send_at],
            )
            .map_err(|e| format!("迁移发件箱失败: {e}"))?;
        }
    }

    if table_exists(old, "drafts") {
        let cc = opt_col_expr(old, "drafts", "cc_addr", "''");
        let bcc = opt_col_expr(old, "drafts", "bcc_addr", "''");
        let from_addr = opt_col_expr(old, "drafts", "from_addr", "''");
        let reply_to = opt_col_expr(old, "drafts", "reply_to", "''");
        let body_fmt = opt_col_expr(old, "drafts", "body_format", "''");
        let mut stmt = old
            .prepare(&format!(
                "SELECT id, account_id, to_addr, {cc}, {bcc}, subject, body_text, in_reply_to, {from_addr}, {reply_to}, {body_fmt}, attachment_paths_json, updated_at FROM drafts"
            ))
            .map_err(|e| format!("读取旧草稿失败: {e}"))?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, u32>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, String>(8)?,
                    r.get::<_, String>(9)?,
                    r.get::<_, String>(10)?,
                    r.get::<_, String>(11)?,
                    r.get::<_, i64>(12)?,
                ))
            })
            .map_err(|e| format!("遍历旧草稿失败: {e}"))?;
        for row in rows {
            let (id, acc, to_a, cc_a, bcc_a, subj, body, irt, from_a, rto, fmt, paths, upd) =
                row.map_err(|e| format!("解析旧草稿失败: {e}"))?;
            tx.execute(
                "INSERT OR REPLACE INTO drafts (id, account_id, to_addr, cc_addr, bcc_addr, subject, body_text, in_reply_to, from_addr, reply_to, body_format, attachment_paths_json, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![id, acc, to_a, cc_a, bcc_a, subj, body, irt, from_a, rto, fmt, paths, upd],
            )
            .map_err(|e| format!("迁移草稿失败: {e}"))?;
        }
    }

    tx.commit()
        .map_err(|e| format!("提交全局数据迁移事务失败: {e}"))
}

fn collect_legacy_account_ids(old: &Connection) -> Result<Vec<u32>, String> {
    let mut ids = std::collections::BTreeSet::new();
    for table in ["folders", "messages"] {
        if table_exists(old, table) && has_column(old, table, "account_id") {
            let mut stmt = old
                .prepare(&format!("SELECT DISTINCT account_id FROM {table}"))
                .map_err(|e| format!("读取旧账号列表失败: {e}"))?;
            let rows = stmt
                .query_map([], |r| r.get::<_, u32>(0))
                .map_err(|e| format!("遍历旧账号列表失败: {e}"))?;
            for id in rows.flatten() {
                ids.insert(id);
            }
        }
    }
    Ok(ids.into_iter().collect())
}

fn migrate_one_account(
    old: &Connection,
    account_id: u32,
    slot: u8,
    account_dir: &Path,
    legacy_attachments_dir: Option<&Path>,
) -> Result<(), String> {
    let mut acc_db = AccountDb::open_in_dir(account_id, slot, account_dir)?;

    let mut folder_map: HashMap<u32, u32> = HashMap::new();
    if table_exists(old, "folders") {
        let is_inbox = opt_col_expr(old, "folders", "is_inbox", "0");
        let has_more = opt_col_expr(old, "folders", "has_more_remote", "0");
        let next_link = opt_col_expr(old, "folders", "remote_next_link", "''");
        let mut stmt = old
            .prepare(&format!(
                "SELECT id, name, path, uid_validity, unread_count, total_count, {is_inbox}, {has_more}, {next_link}
                 FROM folders WHERE account_id = ?1 ORDER BY id ASC"
            ))
            .map_err(|e| format!("读取旧文件夹失败: {e}"))?;
        let rows = stmt
            .query_map(params![account_id], |r| {
                Ok((
                    r.get::<_, u32>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, u32>(3)?,
                    r.get::<_, u32>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, i32>(6)?,
                    r.get::<_, i32>(7)?,
                    r.get::<_, String>(8)?,
                ))
            })
            .map_err(|e| format!("遍历旧文件夹失败: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("解析旧文件夹失败: {e}"))?;

        let tx = acc_db
            .mail_conn
            .transaction()
            .map_err(|e| format!("开启账号文件夹迁移事务失败: {e}"))?;
        for (old_fid, name, path, uid_val, unread, total, is_inbox, has_more, next_link) in rows {
            tx.execute(
                "INSERT INTO folders (name, path, uid_validity, unread_count, total_count, is_inbox, has_more_remote, remote_next_link)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(path) DO UPDATE SET
                     name = excluded.name,
                     uid_validity = excluded.uid_validity,
                     unread_count = excluded.unread_count,
                     total_count = excluded.total_count,
                     is_inbox = excluded.is_inbox,
                     has_more_remote = excluded.has_more_remote,
                     remote_next_link = excluded.remote_next_link",
                params![name, path, uid_val, unread, total, is_inbox, has_more, next_link],
            )
            .map_err(|e| format!("迁移文件夹失败 ({path}): {e}"))?;
            let new_fid: u32 = tx
                .query_row(
                    "SELECT id FROM folders WHERE path = ?1",
                    params![path],
                    |r| r.get(0),
                )
                .map_err(|e| format!("查询迁移后文件夹 ID 失败: {e}"))?;
            folder_map.insert(old_fid, new_fid);
        }
        tx.commit()
            .map_err(|e| format!("提交账号文件夹迁移事务失败: {e}"))?;
    }

    let mut msg_map: HashMap<u32, u32> = HashMap::new();
    let mut body_rows: Vec<(u32, String, Option<String>)> = Vec::new();
    if table_exists(old, "messages") {
        let remote_id = opt_col_expr(old, "messages", "remote_id", "''");
        let mid = opt_col_expr(old, "messages", "message_id", "''");
        let irt = opt_col_expr(old, "messages", "in_reply_to", "''");
        let refs = opt_col_expr(old, "messages", "references_header", "''");
        let cc = opt_col_expr(old, "messages", "cc", "''");
        let thread_subj = opt_col_expr(old, "messages", "thread_subject", "''");
        let body_text = opt_col_expr(old, "messages", "body_text", "NULL");
        let body_html = opt_col_expr(old, "messages", "body_html", "NULL");
        let index_text = opt_col_expr(old, "messages", "index_text", "NULL");
        let unsub = opt_col_expr(old, "messages", "list_unsubscribe", "''");
        let unsub_post = opt_col_expr(old, "messages", "list_unsubscribe_post", "0");
        let auth_res = opt_col_expr(old, "messages", "auth_results", "''");
        let imp = opt_col_expr(old, "messages", "importance", "0");
        let tags = opt_col_expr(old, "messages", "tags", "''");
        let ans = opt_col_expr(old, "messages", "answered", "0");
        let fwd = opt_col_expr(old, "messages", "forwarded", "0");
        let snooze = opt_col_expr(old, "messages", "snoozed_until_ts", "0");

        let sql = format!(
            "SELECT id, folder_id, uid, {remote_id}, {mid}, {irt}, {refs},
                    sender, recipients, {cc}, subject, {thread_subj}, preview, date_ts, date_display,
                    unread, flagged, has_attachment, {body_text}, {body_html}, {index_text},
                    {unsub}, {unsub_post}, {auth_res}, {imp}, {tags}, {ans}, {fwd}, {snooze}
             FROM messages WHERE account_id = ?1 ORDER BY id ASC"
        );
        let mut stmt = old
            .prepare(&sql)
            .map_err(|e| format!("读取旧邮件失败: {e}"))?;
        let rows = stmt
            .query_map(params![account_id], |r| {
                Ok((
                    r.get::<_, u32>(0)?,
                    r.get::<_, u32>(1)?,
                    r.get::<_, u32>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, String>(7)?,
                    r.get::<_, String>(8)?,
                    r.get::<_, String>(9)?,
                    r.get::<_, String>(10)?,
                    r.get::<_, String>(11)?,
                    r.get::<_, String>(12)?,
                    r.get::<_, i64>(13)?,
                    r.get::<_, String>(14)?,
                    r.get::<_, i32>(15)?,
                    r.get::<_, i32>(16)?,
                    r.get::<_, i32>(17)?,
                    r.get::<_, Option<String>>(18)?,
                    r.get::<_, Option<String>>(19)?,
                    r.get::<_, Option<String>>(20)?,
                    r.get::<_, String>(21)?,
                    r.get::<_, i32>(22)?,
                    r.get::<_, String>(23)?,
                    r.get::<_, i32>(24)?,
                    r.get::<_, String>(25)?,
                    r.get::<_, i32>(26)?,
                    r.get::<_, i32>(27)?,
                    r.get::<_, i64>(28)?,
                ))
            })
            .map_err(|e| format!("遍历旧邮件失败: {e}"))?;

        let tx = acc_db
            .mail_conn
            .transaction()
            .map_err(|e| format!("开启账号邮件迁移事务失败: {e}"))?;
        for row in rows {
            let (
                old_msg_id,
                old_fid,
                uid,
                remote_id,
                mid,
                irt,
                refs,
                sender,
                recipients,
                cc,
                subject,
                mut thread_subj,
                preview,
                date_ts,
                date_display,
                unread,
                flagged,
                has_attachment,
                body_text_opt,
                body_html_opt,
                index_text_opt,
                unsub,
                unsub_post,
                auth_res,
                imp,
                tags,
                ans,
                fwd,
                snooze,
            ) = row.map_err(|e| format!("解析旧邮件行失败: {e}"))?;

            let Some(&new_fid) = folder_map.get(&old_fid) else {
                continue;
            };
            if thread_subj.is_empty() {
                thread_subj = super::normalize_thread_subject(&subject);
            }
            let body_loaded = body_text_opt.is_some();

            tx.execute(
                "INSERT INTO messages (
                    folder_id, uid, remote_id, message_id, in_reply_to, references_header,
                    sender, recipients, cc, subject, thread_subject, preview, date_ts, date_display,
                    unread, flagged, has_attachment, body_loaded, body_cached_at, index_text,
                    list_unsubscribe, list_unsubscribe_post, auth_results,
                    importance, tags, answered, forwarded, snoozed_until_ts
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                    ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28
                 )
                 ON CONFLICT(folder_id, uid) DO UPDATE SET
                    subject = excluded.subject,
                    body_loaded = excluded.body_loaded",
                params![
                    new_fid,
                    uid,
                    remote_id,
                    mid,
                    irt,
                    refs,
                    sender,
                    recipients,
                    cc,
                    subject,
                    thread_subj,
                    preview,
                    date_ts,
                    date_display,
                    unread,
                    flagged,
                    has_attachment,
                    body_loaded as i32,
                    if body_loaded { date_ts.max(0) } else { 0 },
                    index_text_opt,
                    unsub,
                    unsub_post,
                    auth_res,
                    imp,
                    tags,
                    ans,
                    fwd,
                    snooze,
                ],
            )
            .map_err(|e| format!("迁移邮件失败 (UID {uid}): {e}"))?;

            let new_msg_id: u32 = tx
                .query_row(
                    "SELECT id FROM messages WHERE folder_id = ?1 AND uid = ?2",
                    params![new_fid, uid],
                    |r| r.get(0),
                )
                .map_err(|e| format!("读取迁移后邮件 ID 失败: {e}"))?;

            super::sync_message_rfc_refs(&tx, new_msg_id, &mid, &irt, &refs)?;

            let fts_text = body_text_opt
                .as_deref()
                .or(index_text_opt.as_deref())
                .unwrap_or("");
            if !fts_text.is_empty() {
                let _ = tx.execute(
                    "DELETE FROM messages_index_fts WHERE rowid = ?1",
                    params![new_msg_id],
                );
                let _ = tx.execute(
                    "INSERT INTO messages_index_fts(rowid, index_text) VALUES (?1, ?2)",
                    params![new_msg_id, fts_text],
                );
            }

            if let Some(body_text) = body_text_opt {
                body_rows.push((new_msg_id, body_text, body_html_opt));
            }
            msg_map.insert(old_msg_id, new_msg_id);
        }
        tx.commit()
            .map_err(|e| format!("提交账号邮件迁移事务失败: {e}"))?;
    }

    if !body_rows.is_empty() {
        let content_conn = acc_db
            .ensure_content_conn()?
            .ok_or_else(|| "无法创建账号正文库".to_string())?;
        let tx = content_conn
            .unchecked_transaction()
            .map_err(|e| format!("开启正文库迁移事务失败: {e}"))?;
        for (new_msg_id, body_text, body_html) in body_rows {
            tx.execute(
                "INSERT OR REPLACE INTO message_bodies (message_id, body_text, body_html) VALUES (?1, ?2, ?3)",
                params![new_msg_id, body_text, body_html],
            )
            .map_err(|e| format!("迁移正文内容失败: {e}"))?;
        }
        tx.commit()
            .map_err(|e| format!("提交正文库迁移事务失败: {e}"))?;
    }

    if table_exists(old, "attachments") && !msg_map.is_empty() {
        let storage_path = opt_col_expr(old, "attachments", "storage_path", "''");
        let downloaded = opt_col_expr(old, "attachments", "downloaded", "1");
        let remote_part = opt_col_expr(old, "attachments", "remote_part", "''");
        let xfer_enc = opt_col_expr(old, "attachments", "transfer_encoding", "''");
        let sql = format!(
            "SELECT a.message_id, a.filename, a.mime_type, a.size_bytes, a.content,
                    {storage_path}, {downloaded}, {remote_part}, {xfer_enc}
             FROM attachments a
             JOIN messages m ON m.id = a.message_id
             WHERE m.account_id = ?1 ORDER BY a.id ASC"
        );
        let mut stmt = old
            .prepare(&sql)
            .map_err(|e| format!("读取旧附件记录失败: {e}"))?;
        let rows = stmt
            .query_map(params![account_id], |r| {
                Ok((
                    r.get::<_, u32>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, Vec<u8>>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, i32>(6)? != 0,
                    r.get::<_, String>(7)?,
                    r.get::<_, String>(8)?,
                ))
            })
            .map_err(|e| format!("遍历旧附件记录失败: {e}"))?;

        let tx = acc_db
            .mail_conn
            .transaction()
            .map_err(|e| format!("开启附件迁移事务失败: {e}"))?;
        for row in rows {
            let (
                old_msg_id,
                filename,
                mime_type,
                size_bytes,
                blob_content,
                old_storage_path,
                downloaded,
                remote_part,
                xfer_enc,
            ) = row.map_err(|e| format!("解析旧附件记录失败: {e}"))?;

            let Some(&new_msg_id) = msg_map.get(&old_msg_id) else {
                continue;
            };
            let bytes_opt: Option<Vec<u8>> = if !downloaded {
                None
            } else if !old_storage_path.is_empty() {
                resolve_attachment_disk_path(legacy_attachments_dir, &old_storage_path)
                    .and_then(|p| fs::read(p).ok())
            } else {
                Some(blob_content)
            };

            tx.execute(
                "INSERT INTO attachments (
                    message_id, filename, mime_type, size_bytes, content,
                    storage_path, downloaded, remote_part, transfer_encoding
                 ) VALUES (?1, ?2, ?3, ?4, X'', '', ?5, ?6, ?7)",
                params![
                    new_msg_id,
                    filename,
                    mime_type,
                    size_bytes,
                    bytes_opt.is_some() as i32,
                    remote_part,
                    xfer_enc,
                ],
            )
            .map_err(|e| format!("写入迁移附件失败: {e}"))?;

            if let (Some(dir), Some(bytes)) =
                (acc_db.attachments_dir.as_deref(), bytes_opt.as_deref())
            {
                let new_att_id = u32::try_from(tx.last_insert_rowid())
                    .map_err(|_| "迁移附件 ID 越界".to_string())?;
                let rel = write_attachment_file(dir, new_att_id, bytes)?;
                tx.execute(
                    "UPDATE attachments SET storage_path = ?1 WHERE id = ?2",
                    params![rel, new_att_id],
                )
                .map_err(|e| format!("更新迁移附件路径失败: {e}"))?;
            }
        }
        tx.commit()
            .map_err(|e| format!("提交附件迁移事务失败: {e}"))?;
    }

    let _ = acc_db.mail_conn.execute(
        "INSERT INTO messages_fts(messages_fts) VALUES('rebuild')",
        [],
    );
    acc_db.checkpoint();
    Ok(())
}

fn backup_legacy_files(legacy_db: &Path, legacy_attachments_dir: Option<&Path>) {
    let bak_path = |p: &Path| {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("legacy");
        p.with_file_name(format!("{name}.migrated.bak"))
    };
    let rename_if_exists = |p: &Path| {
        if p.exists() {
            let dst = bak_path(p);
            if !dst.exists() {
                let _ = fs::rename(p, dst);
            }
        }
    };
    rename_if_exists(legacy_db);
    let wal = PathBuf::from(format!("{}-wal", legacy_db.display()));
    let shm = PathBuf::from(format!("{}-shm", legacy_db.display()));
    rename_if_exists(&wal);
    rename_if_exists(&shm);
    if let Some(att_dir) = legacy_attachments_dir {
        if att_dir.is_dir() {
            rename_if_exists(att_dir);
        }
    }
}
