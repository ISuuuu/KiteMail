//! Wave 3 Group B IPC commands: contacts listing, CRUD, and vCard/CSV import/export.

use std::collections::HashSet;
use std::path::Path;

use kitemail_mail_core::cache::{CacheStore, KnownContact};
use kitemail_mail_core::config;
use kitemail_mail_core::contacts_io::{
    export_csv, export_vcards, looks_like_vcard, parse_csv_contacts, parse_vcards,
};
use kitemail_mail_core::UserContact;
use tauri::State;

use crate::commands::AppState;
use crate::dto::{
    AppMode, ContactDto, DeleteContactInput, DeleteContactOutput, ExportContactsFormatDto,
    ExportContactsInput, ExportContactsOutput, ImportContactsFormatDto, ImportContactsInput,
    ImportContactsOutput, IpcError, ListContactsInput, ListContactsOutput, MailChangedEvent,
    SaveContactInput, SaveContactOutput,
};
use crate::mail_session::{MailSession, MAX_PAGE_LIMIT, MAX_QUERY_LEN};

pub const MAX_IMPORT_CONTENT_BYTES: usize = 5 * 1024 * 1024;

fn validate_contact_email(raw: &str) -> Result<String, IpcError> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.chars().any(|c| c.is_whitespace() || c.is_control())
        || trimmed.chars().count() > 320
    {
        return Err(IpcError::invalid_input("邮箱地址格式无效", None));
    }
    let Some((local, domain)) = trimmed.split_once('@') else {
        return Err(IpcError::invalid_input("邮箱地址格式无效", None));
    };
    if local.is_empty() || domain.is_empty() || domain.contains('@') {
        return Err(IpcError::invalid_input("邮箱地址格式无效", None));
    }
    Ok(trimmed.to_string())
}

fn clean_groups(groups: Option<&[String]>) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    if let Some(items) = groups {
        for g in items {
            let trimmed = g.trim();
            if !trimmed.is_empty() && seen.insert(trimmed.to_ascii_lowercase()) {
                out.push(trimmed.to_string());
            }
        }
    }
    out
}

fn known_to_dto(c: &KnownContact) -> ContactDto {
    ContactDto {
        id: (c.custom && c.id > 0).then_some(c.id),
        name: c.name.clone(),
        email: c.email.clone(),
        notes: c.notes.clone(),
        phone: c.phone.clone(),
        organization: c.organization.clone(),
        job_title: c.job_title.clone(),
        groups: c.groups.clone(),
        is_user_contact: c.custom,
        message_count: c.message_count,
        last_contact_ts: 0,
    }
}

fn user_to_dto(c: &UserContact, message_count: usize) -> ContactDto {
    ContactDto {
        id: (c.id > 0).then_some(c.id),
        name: c.display_name.clone(),
        email: c.email.clone(),
        notes: c.notes.clone(),
        phone: c.phone.clone(),
        organization: c.organization.clone(),
        job_title: c.job_title.clone(),
        groups: c.groups.clone(),
        is_user_contact: true,
        message_count,
        last_contact_ts: 0,
    }
}

fn known_to_user_contact(c: &KnownContact) -> UserContact {
    UserContact {
        id: c.id,
        email: c.email.clone(),
        display_name: c.name.clone(),
        notes: c.notes.clone(),
        phone: c.phone.clone(),
        organization: c.organization.clone(),
        job_title: c.job_title.clone(),
        birthday: c.birthday.clone(),
        groups: c.groups.clone(),
    }
}

impl MailSession {
    fn merged_demo_contacts(&self, query: &str) -> Vec<KnownContact> {
        let demo = self.demo.lock().expect("demo mutex poisoned");
        let mut contacts = demo.store.list_known_contacts("", 0, 10_000);
        // Reset built-in demo custom marker so `demo.user_contacts` is the single source of truth.
        for c in &mut contacts {
            c.custom = false;
            c.id = 0;
            c.notes.clear();
        }
        for user in &demo.user_contacts {
            let key = user.email.trim().to_ascii_lowercase();
            if key.is_empty() {
                continue;
            }
            if let Some(existing) = contacts
                .iter_mut()
                .find(|c| c.email.eq_ignore_ascii_case(&key))
            {
                existing.id = user.id;
                existing.notes = user.notes.clone();
                existing.phone = user.phone.clone();
                existing.organization = user.organization.clone();
                existing.job_title = user.job_title.clone();
                existing.birthday = user.birthday.clone();
                existing.groups = user.groups.clone();
                existing.custom = true;
                if !user.display_name.trim().is_empty() {
                    existing.name = user.display_name.clone();
                }
            } else {
                contacts.push(KnownContact {
                    name: user.display_name.clone(),
                    email: user.email.clone(),
                    message_count: 0,
                    id: user.id,
                    notes: user.notes.clone(),
                    custom: true,
                    phone: user.phone.clone(),
                    organization: user.organization.clone(),
                    job_title: user.job_title.clone(),
                    birthday: user.birthday.clone(),
                    groups: user.groups.clone(),
                    sent_count: 0,
                });
            }
        }
        let q = query.trim().to_ascii_lowercase();
        let mut filtered: Vec<KnownContact> = contacts
            .into_iter()
            .filter(|c| {
                q.is_empty()
                    || c.name.to_ascii_lowercase().contains(&q)
                    || c.email.to_ascii_lowercase().contains(&q)
                    || c.organization.to_ascii_lowercase().contains(&q)
            })
            .collect();
        filtered.sort_by(|a, b| {
            b.custom
                .cmp(&a.custom)
                .then_with(|| b.message_count.cmp(&a.message_count))
                .then_with(|| a.email.cmp(&b.email))
        });
        filtered
    }

    pub fn list_contacts(&self, input: &ListContactsInput) -> Result<ListContactsOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        let query = input.query.as_deref().unwrap_or("").trim();
        if query.chars().count() > MAX_QUERY_LEN {
            return Err(IpcError::invalid_input(
                format!("搜索关键词长度不能超过 {MAX_QUERY_LEN} 个字符"),
                None,
            ));
        }
        let offset = input.offset.unwrap_or(0);
        let raw_limit = input.limit.unwrap_or(50);
        if raw_limit == 0 {
            return Err(IpcError::invalid_input("limit 必须大于 0", None));
        }
        let limit = raw_limit.min(MAX_PAGE_LIMIT);

        let all_items = match self.mode {
            AppMode::Demo => self.merged_demo_contacts(query),
            AppMode::Live => {
                let data_dir = self
                    .live
                    .lock()
                    .expect("live mutex poisoned")
                    .data_dir
                    .clone();
                if !config::global_db_in(&data_dir).exists() {
                    Vec::new()
                } else {
                    let cache =
                        CacheStore::open_for_data_dir_read_only(&data_dir).map_err(|e| {
                            IpcError::unavailable(format!("本地邮件缓存暂不可用: {e}"), None)
                        })?;
                    cache
                        .list_known_contacts(query, 0, 100_000)
                        .map_err(|e| IpcError::internal(format!("读取联系人失败: {e}"), None))?
                }
            }
        };

        let total = all_items.len();
        let start = offset.min(total);
        let end = start.saturating_add(limit).min(total);
        let items = all_items[start..end].iter().map(known_to_dto).collect();

        Ok(ListContactsOutput {
            items,
            total,
            has_more: end < total,
            state_version: self.state_version(),
        })
    }

    pub fn save_contact(&self, input: &SaveContactInput) -> Result<SaveContactOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        let email = validate_contact_email(&input.email)?;
        let contact = UserContact {
            id: input.id.unwrap_or(0),
            email: email.clone(),
            display_name: input.name.trim().to_string(),
            notes: input.notes.clone().unwrap_or_default(),
            phone: input.phone.as_deref().unwrap_or("").trim().to_string(),
            organization: input
                .organization
                .as_deref()
                .unwrap_or("")
                .trim()
                .to_string(),
            job_title: input.job_title.as_deref().unwrap_or("").trim().to_string(),
            birthday: String::new(),
            groups: clean_groups(input.groups.as_deref()),
        };

        let saved_dto = match self.mode {
            AppMode::Demo => {
                let mut demo = self.demo.lock().expect("demo mutex poisoned");
                if demo
                    .user_contacts
                    .iter()
                    .any(|c| c.id != contact.id && c.email.eq_ignore_ascii_case(&contact.email))
                {
                    return Err(IpcError::invalid_input(
                        format!("已存在邮箱为 {} 的联系人", contact.email),
                        None,
                    ));
                }
                let mut saved = contact;
                if saved.id == 0 {
                    saved.id = demo.next_user_contact_id;
                    demo.next_user_contact_id = demo.next_user_contact_id.saturating_add(1);
                    demo.user_contacts.push(saved.clone());
                } else if let Some(existing) =
                    demo.user_contacts.iter_mut().find(|c| c.id == saved.id)
                {
                    *existing = saved.clone();
                } else {
                    demo.next_user_contact_id = demo.next_user_contact_id.max(saved.id + 1);
                    demo.user_contacts.push(saved.clone());
                }
                user_to_dto(&saved, 0)
            }
            AppMode::Live => {
                let data_dir = self
                    .live
                    .lock()
                    .expect("live mutex poisoned")
                    .data_dir
                    .clone();
                let mut store = CacheStore::open_for_data_dir(&data_dir)
                    .map_err(|e| IpcError::internal(format!("打开本地通讯录库失败: {e}"), None))?;
                let saved_id = store.upsert_user_contact(&contact).map_err(|e| {
                    if e.contains("已存在") || e.contains("不合法") {
                        IpcError::invalid_input(e, None)
                    } else {
                        IpcError::internal(e, None)
                    }
                })?;
                store.checkpoint_and_close_all();
                let mut saved = contact;
                saved.id = saved_id;
                user_to_dto(&saved, 0)
            }
        };

        let ver = self.bump_state_version();
        self.emitter.emit_mail_changed(MailChangedEvent {
            state_version: ver,
            reason: "contactsUpdated".to_string(),
            account_id: None,
            folder_id: None,
            message_id: None,
        });

        Ok(SaveContactOutput {
            contact: saved_dto,
            state_version: ver,
        })
    }

    pub fn delete_contact(
        &self,
        input: &DeleteContactInput,
    ) -> Result<DeleteContactOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        if input.contact_id == 0 {
            return Err(IpcError::invalid_input("contactId 必须为正整数", None));
        }

        let deleted = match self.mode {
            AppMode::Demo => {
                let mut demo = self.demo.lock().expect("demo mutex poisoned");
                let before = demo.user_contacts.len();
                demo.user_contacts.retain(|c| c.id != input.contact_id);
                demo.user_contacts.len() != before
            }
            AppMode::Live => {
                let data_dir = self
                    .live
                    .lock()
                    .expect("live mutex poisoned")
                    .data_dir
                    .clone();
                if !config::global_db_in(&data_dir).exists() {
                    false
                } else {
                    let mut store =
                        CacheStore::open_for_data_dir_existing(&data_dir).map_err(|e| {
                            IpcError::internal(format!("打开本地通讯录库失败: {e}"), None)
                        })?;
                    let ok = store
                        .delete_user_contact(input.contact_id)
                        .map_err(|e| IpcError::internal(e, None))?;
                    store.checkpoint_and_close_all();
                    ok
                }
            }
        };

        let ver = if deleted {
            let v = self.bump_state_version();
            self.emitter.emit_mail_changed(MailChangedEvent {
                state_version: v,
                reason: "contactsUpdated".to_string(),
                account_id: None,
                folder_id: None,
                message_id: None,
            });
            v
        } else {
            self.state_version()
        };

        Ok(DeleteContactOutput {
            deleted,
            state_version: ver,
        })
    }

    pub fn import_contacts(
        &self,
        input: &ImportContactsInput,
    ) -> Result<ImportContactsOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        if input.content.trim().is_empty() {
            return Err(IpcError::invalid_input("导入内容不能为空", None));
        }
        if input.content.len() > MAX_IMPORT_CONTENT_BYTES {
            return Err(IpcError::invalid_input("导入文本大小超过 5 MB 上限", None));
        }
        if input.content.contains('\0') {
            return Err(IpcError::invalid_input("导入文本包含非法空字符", None));
        }

        let parsed = match input.format {
            ImportContactsFormatDto::Vcard => parse_vcards(&input.content),
            ImportContactsFormatDto::Csv => parse_csv_contacts(&input.content),
            ImportContactsFormatDto::Auto => {
                if looks_like_vcard(Path::new("import.txt"), &input.content) {
                    parse_vcards(&input.content)
                } else {
                    parse_csv_contacts(&input.content)
                }
            }
        };
        let total_parsed = parsed.len();

        let (added, skipped) = match self.mode {
            AppMode::Demo => {
                let mut demo = self.demo.lock().expect("demo mutex poisoned");
                let mut added = 0usize;
                let mut skipped = 0usize;
                for mut c in parsed {
                    let email = c.email.trim().to_string();
                    if validate_contact_email(&email).is_err()
                        || demo
                            .user_contacts
                            .iter()
                            .any(|existing| existing.email.eq_ignore_ascii_case(&email))
                    {
                        skipped += 1;
                        continue;
                    }
                    c.id = demo.next_user_contact_id;
                    c.email = email;
                    demo.next_user_contact_id = demo.next_user_contact_id.saturating_add(1);
                    demo.user_contacts.push(c);
                    added += 1;
                }
                (added, skipped)
            }
            AppMode::Live => {
                let data_dir = self
                    .live
                    .lock()
                    .expect("live mutex poisoned")
                    .data_dir
                    .clone();
                let mut store = CacheStore::open_for_data_dir(&data_dir)
                    .map_err(|e| IpcError::internal(format!("打开本地通讯录库失败: {e}"), None))?;
                let res = store
                    .import_user_contacts(&parsed)
                    .map_err(|e| IpcError::internal(e, None))?;
                store.checkpoint_and_close_all();
                res
            }
        };

        let ver = if added > 0 {
            let v = self.bump_state_version();
            self.emitter.emit_mail_changed(MailChangedEvent {
                state_version: v,
                reason: "contactsUpdated".to_string(),
                account_id: None,
                folder_id: None,
                message_id: None,
            });
            v
        } else {
            self.state_version()
        };

        Ok(ImportContactsOutput {
            added,
            skipped,
            total_parsed,
            state_version: ver,
        })
    }

    pub fn export_contacts(
        &self,
        input: &ExportContactsInput,
    ) -> Result<ExportContactsOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }

        let export_list: Vec<UserContact> = match self.mode {
            AppMode::Demo => self
                .merged_demo_contacts("")
                .iter()
                .map(known_to_user_contact)
                .collect(),
            AppMode::Live => {
                let data_dir = self
                    .live
                    .lock()
                    .expect("live mutex poisoned")
                    .data_dir
                    .clone();
                if !config::global_db_in(&data_dir).exists() {
                    Vec::new()
                } else {
                    let cache =
                        CacheStore::open_for_data_dir_read_only(&data_dir).map_err(|e| {
                            IpcError::unavailable(format!("本地邮件缓存暂不可用: {e}"), None)
                        })?;
                    cache
                        .list_known_contacts("", 0, 100_000)
                        .map_err(|e| IpcError::internal(format!("读取联系人失败: {e}"), None))?
                        .iter()
                        .map(known_to_user_contact)
                        .collect()
                }
            }
        };

        let count = export_list.len();
        let (filename, content) = match input.format {
            ExportContactsFormatDto::Vcard => {
                ("contacts.vcf".to_string(), export_vcards(&export_list))
            }
            ExportContactsFormatDto::Csv => ("contacts.csv".to_string(), export_csv(&export_list)),
        };

        Ok(ExportContactsOutput {
            format: input.format,
            filename,
            content,
            count,
        })
    }
}

#[tauri::command]
pub fn list_contacts(
    state: State<'_, AppState>,
    input: ListContactsInput,
) -> Result<ListContactsOutput, IpcError> {
    state.session.list_contacts(&input)
}

#[tauri::command]
pub fn save_contact(
    state: State<'_, AppState>,
    input: SaveContactInput,
) -> Result<SaveContactOutput, IpcError> {
    state.session.save_contact(&input)
}

#[tauri::command]
pub fn delete_contact(
    state: State<'_, AppState>,
    input: DeleteContactInput,
) -> Result<DeleteContactOutput, IpcError> {
    state.session.delete_contact(&input)
}

#[tauri::command]
pub fn import_contacts(
    state: State<'_, AppState>,
    input: ImportContactsInput,
) -> Result<ImportContactsOutput, IpcError> {
    state.session.import_contacts(&input)
}

#[tauri::command]
pub fn export_contacts(
    state: State<'_, AppState>,
    input: ExportContactsInput,
) -> Result<ExportContactsOutput, IpcError> {
    state.session.export_contacts(&input)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::UnlockAppInput;
    use crate::mail_session::tests::{make_live_test_session, make_test_session};

    #[test]
    fn demo_contacts_crud_and_vcard_csv_roundtrip_work() {
        let (session, _, dir) = make_test_session();

        let initial = session
            .list_contacts(&ListContactsInput::default())
            .expect("list initial");
        assert!(!initial.items.is_empty());
        assert!(initial.items[0].is_user_contact);
        assert_eq!(initial.items[0].id, Some(1));

        let saved = session
            .save_contact(&SaveContactInput {
                id: None,
                name: "陈思远".to_string(),
                email: "chen.sy@example.org".to_string(),
                notes: Some("核心研发".to_string()),
                phone: Some("+86-13800001111".to_string()),
                organization: Some("KiteMail".to_string()),
                job_title: Some("架构师".to_string()),
                groups: Some(vec!["研发".to_string(), "研发".to_string()]),
            })
            .expect("save contact");
        assert!(saved.contact.is_user_contact);
        assert_eq!(saved.contact.groups, vec!["研发".to_string()]);
        let saved_id = saved.contact.id.unwrap();

        let exported_vcf = session
            .export_contacts(&ExportContactsInput {
                format: ExportContactsFormatDto::Vcard,
            })
            .expect("export vcard");
        assert!(exported_vcf.content.contains("chen.sy@example.org"));
        assert!(exported_vcf.content.contains("BEGIN:VCARD"));

        let exported_csv = session
            .export_contacts(&ExportContactsInput {
                format: ExportContactsFormatDto::Csv,
            })
            .expect("export csv");
        assert!(exported_csv.content.starts_with("Name,Email,Notes\r\n"));
        assert!(exported_csv.content.contains("chen.sy@example.org"));

        // Delete saved contact, then re-import from the exported vCard!
        let del = session
            .delete_contact(&DeleteContactInput {
                contact_id: saved_id,
            })
            .expect("delete contact");
        assert!(del.deleted);

        let imported = session
            .import_contacts(&ImportContactsInput {
                format: ImportContactsFormatDto::Auto,
                content: exported_vcf.content,
            })
            .expect("import vcard");
        assert!(imported.added >= 1);

        let search = session
            .list_contacts(&ListContactsInput {
                query: Some("chen.sy".to_string()),
                offset: Some(0),
                limit: Some(10),
            })
            .expect("search imported");
        assert_eq!(search.total, 1);
        assert!(search.items[0].is_user_contact);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn import_rejects_blank_oversized_and_null_inputs_and_respects_lock() {
        let (session, _, _, _, dir) = make_live_test_session(true, Some("1234"));

        assert_eq!(
            session
                .import_contacts(&ImportContactsInput {
                    format: ImportContactsFormatDto::Auto,
                    content: "   \r\n\t ".to_string(),
                })
                .unwrap_err()
                .code,
            "INVALID_INPUT"
        );
        assert_eq!(
            session
                .import_contacts(&ImportContactsInput {
                    format: ImportContactsFormatDto::Csv,
                    content: "a\0b".to_string(),
                })
                .unwrap_err()
                .code,
            "INVALID_INPUT"
        );
        assert_eq!(
            session
                .import_contacts(&ImportContactsInput {
                    format: ImportContactsFormatDto::Csv,
                    content: "a".repeat(MAX_IMPORT_CONTENT_BYTES + 1),
                })
                .unwrap_err()
                .code,
            "INVALID_INPUT"
        );

        session.lock_app(Some("manual"));
        assert_eq!(
            session
                .list_contacts(&ListContactsInput::default())
                .unwrap_err()
                .code,
            "LOCKED"
        );
        assert_eq!(
            session
                .save_contact(&SaveContactInput {
                    id: None,
                    name: "X".to_string(),
                    email: "x@example.com".to_string(),
                    notes: None,
                    phone: None,
                    organization: None,
                    job_title: None,
                    groups: None,
                })
                .unwrap_err()
                .code,
            "LOCKED"
        );
        assert_eq!(
            session
                .delete_contact(&DeleteContactInput { contact_id: 1 })
                .unwrap_err()
                .code,
            "LOCKED"
        );
        assert_eq!(
            session
                .import_contacts(&ImportContactsInput {
                    format: ImportContactsFormatDto::Csv,
                    content: "Name,Email\nA,a@example.com".to_string(),
                })
                .unwrap_err()
                .code,
            "LOCKED"
        );
        assert_eq!(
            session
                .export_contacts(&ExportContactsInput {
                    format: ExportContactsFormatDto::Csv,
                })
                .unwrap_err()
                .code,
            "LOCKED"
        );

        session.unlock_app(&UnlockAppInput {
            passcode: "1234".to_string(),
        });

        let imported = session
            .import_contacts(&ImportContactsInput {
                format: ImportContactsFormatDto::Csv,
                content: "Name,Email,Notes\r\nAlice New,alice.new@example.com,VIP\r\n".to_string(),
            })
            .expect("live import csv");
        assert_eq!(imported.added, 1);

        let listed = session
            .list_contacts(&ListContactsInput {
                query: Some("alice.new".to_string()),
                offset: Some(0),
                limit: Some(10),
            })
            .expect("live list");
        assert_eq!(listed.total, 1);
        assert!(listed.items[0].is_user_contact);
        let id = listed.items[0].id.unwrap();

        let del = session
            .delete_contact(&DeleteContactInput { contact_id: id })
            .expect("live delete");
        assert!(del.deleted);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
