//! Wave 3 Group C IPC commands: full preferences, app-lock passcode management,
//! trusted remote senders allowlist, cache pruning/clearing, and update checking.

use std::sync::atomic::Ordering;
use std::time::Instant;

use kitemail_mail_core::app_lock::{
    hash_passcode, set_lock_persisted_in, validate_new_passcode, verify_passcode,
};
use kitemail_mail_core::cache::CacheStore;
use kitemail_mail_core::config::{app_storage_summary_in, normalize_app_lock_idle_minutes};
use kitemail_mail_core::{secrets, update, MailRequest};
use tauri::State;

use crate::commands::AppState;
use crate::dto::{
    AppMode, CheckAppUpdateOutput, ClearBodyCachesInput, ClearBodyCachesOutput,
    FullPreferencesPatchDto, GetFullPreferencesOutput, IpcError, LockStateEvent, MailChangedEvent,
    ManageTrustedSendersInput, ManageTrustedSendersOutput, SetAppLockPasscodeInput,
    SetAppLockPasscodeOutput, StorageStatsDto, TrustedSenderActionDto, UpdateFullPreferencesOutput,
};
use crate::mail_session::MailSession;
use crate::preferences::{to_full_preferences_dto, VALID_APP_LOCK_IDLE_MINUTES};

/// 构建版本：发布流水线注入 `KITEMAIL_VERSION`（标签去掉 v），本地构建回落到 Cargo 版本。
pub fn app_version() -> &'static str {
    option_env!("KITEMAIL_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
}

fn non_empty_opt(s: &str) -> Option<String> {
    let trimmed = s.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn compute_storage_stats_for(data_dir: &std::path::Path) -> (String, String, StorageStatsDto) {
    let summary = app_storage_summary_in(data_dir);
    let config_bytes = summary.config_stats.total_bytes;
    let mail_bytes = summary.mail_stats.total_bytes;
    let total_bytes = config_bytes.saturating_add(mail_bytes);
    let file_count = summary
        .config_stats
        .file_count
        .saturating_add(summary.mail_stats.file_count);
    (
        summary.app_data_dir.to_string_lossy().into_owned(),
        summary.mail_data_dir.to_string_lossy().into_owned(),
        StorageStatsDto {
            total_bytes,
            mail_bytes,
            config_bytes,
            file_count,
        },
    )
}

impl MailSession {
    pub fn get_full_preferences(&self) -> Result<GetFullPreferencesOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        let raw_prefs = self.prefs.snapshot_raw();
        let has_passcode = self.has_passcode();
        let lock_on_startup = self.is_lock_on_startup();
        let preferences = to_full_preferences_dto(&raw_prefs, has_passcode, lock_on_startup);
        let active_dir = match self.mode {
            AppMode::Demo => self.prefs.data_dir().to_path_buf(),
            AppMode::Live => self
                .live
                .lock()
                .expect("live mutex poisoned")
                .data_dir
                .clone(),
        };
        let (data_dir, mail_data_dir, storage_stats) = compute_storage_stats_for(&active_dir);

        Ok(GetFullPreferencesOutput {
            preferences,
            has_passcode,
            allowed_remote_senders: raw_prefs.allowed_remote_senders,
            data_dir,
            mail_data_dir,
            storage_stats,
            app_version: app_version().to_string(),
            is_portable: update::is_portable(),
            state_version: self.state_version(),
        })
    }

    pub fn update_full_preferences(
        &self,
        input: &FullPreferencesPatchDto,
    ) -> Result<UpdateFullPreferencesOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }
        let has_passcode = self.has_passcode();
        if let Some(enabled) = input.app_lock_enabled {
            if enabled && !has_passcode {
                return Err(IpcError::invalid_input(
                    "请先通过设置锁屏口令来启用应用锁",
                    None,
                ));
            }
            if !enabled && has_passcode {
                return Err(IpcError::invalid_input("关闭应用锁必须验证当前口令", None));
            }
        }
        if let Some(true) = input.app_lock_on_startup {
            if !has_passcode {
                return Err(IpcError::invalid_input(
                    "请先设置锁屏口令后再开启启动锁定",
                    None,
                ));
            }
        }

        let prev_push = self.prefs.snapshot_raw().push_enabled;
        let updated_raw = self.prefs.update_full_patch(input)?;
        if let Some(on_startup) = input.app_lock_on_startup {
            let _ = self.set_lock_on_startup(on_startup)?;
        }

        if self.mode == AppMode::Live && updated_raw.push_enabled != prev_push {
            let worker = self
                .live
                .lock()
                .expect("live mutex poisoned")
                .worker
                .clone();
            if let Some(w) = worker {
                w.send(MailRequest::SetPushEnabled {
                    enabled: updated_raw.push_enabled,
                });
            }
        }

        self.on_preferences_updated();
        let ver = self.bump_state_version();
        self.emitter.emit_mail_changed(MailChangedEvent {
            state_version: ver,
            reason: "preferencesUpdated".to_string(),
            account_id: None,
            folder_id: None,
            message_id: None,
        });

        let preferences =
            to_full_preferences_dto(&updated_raw, self.has_passcode(), self.is_lock_on_startup());
        Ok(UpdateFullPreferencesOutput {
            preferences,
            state_version: ver,
        })
    }

    pub fn set_app_lock_passcode(
        &self,
        input: &SetAppLockPasscodeInput,
    ) -> Result<SetAppLockPasscodeOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }

        if let Some(idle) = input.idle_minutes {
            if !VALID_APP_LOCK_IDLE_MINUTES.contains(&idle) {
                return Err(IpcError::invalid_input(
                    "idleMinutes 必须为 0/1/5/10/15/30/60 之一",
                    None,
                ));
            }
        }

        let (existing_hash, persist_keyring, persist_dir) = {
            let guard = self.lock_state.lock().expect("lock_state mutex poisoned");
            (
                guard.passcode_hash.clone(),
                guard.persist_keyring,
                guard.persist_dir.clone(),
            )
        };

        if let Some(ref stored) = existing_hash {
            let current = input.current_passcode.as_deref().unwrap_or("");
            if current.is_empty() || !verify_passcode(current, stored) {
                return Err(IpcError::invalid_input("当前锁屏口令不正确", None));
            }
        }

        let (next_hash, has_passcode, message) = match input.new_passcode.as_deref() {
            Some(new_pass) => {
                validate_new_passcode(new_pass).map_err(|e| IpcError::invalid_input(e, None))?;
                let hashed = hash_passcode(new_pass)
                    .map_err(|e| IpcError::internal(format!("生成口令摘要失败: {e}"), None))?;
                if persist_keyring {
                    secrets::save_app_lock_hash(&hashed)
                        .map_err(|e| IpcError::internal(format!("保存锁屏口令失败: {e}"), None))?;
                }
                let msg = if existing_hash.is_some() {
                    "锁屏口令已更新".to_string()
                } else {
                    "锁屏口令已设置".to_string()
                };
                (Some(hashed), true, msg)
            }
            None => {
                if persist_keyring {
                    secrets::delete_app_lock_hash()
                        .map_err(|e| IpcError::internal(format!("清除锁屏口令失败: {e}"), None))?;
                }
                if let Some(ref dir) = persist_dir {
                    let _ = set_lock_persisted_in(dir, false);
                    let _ = crate::preferences::set_startup_lock_enabled_in(dir, false);
                }
                (None, false, "锁屏口令已清除".to_string())
            }
        };

        {
            let mut guard = self.lock_state.lock().expect("lock_state mutex poisoned");
            guard.passcode_hash = next_hash;
            guard.failed_attempts = 0;
            guard.cooldown_until = None;
            guard.tracker.reset_after_unlock(Instant::now());
            if !has_passcode {
                guard.locked = false;
                guard.locked_reason = None;
                guard.lock_on_startup = false;
            } else if let Some(on_startup) = input.enable_on_startup {
                if let Some(ref dir) = guard.persist_dir {
                    let _ = crate::preferences::set_startup_lock_enabled_in(dir, on_startup);
                }
                guard.lock_on_startup = on_startup;
            }
        }
        if !has_passcode {
            self.locked.store(false, Ordering::SeqCst);
        }

        if let Some(idle) = input.idle_minutes {
            let normalized = normalize_app_lock_idle_minutes(idle);
            self.prefs
                .mutate_and_save(|p| {
                    if p.app_lock_idle_minutes == normalized {
                        return false;
                    }
                    p.app_lock_idle_minutes = normalized;
                    true
                })
                .map_err(|e| IpcError::internal(format!("保存锁屏超时设置失败: {e}"), None))?;
        }

        self.refresh_tray_lock_enabled();
        let ver = self.bump_state_version();
        self.emitter.emit_lock_state(LockStateEvent {
            state_version: ver,
            locked: self.is_locked(),
            has_passcode,
            locked_reason: None,
        });

        Ok(SetAppLockPasscodeOutput {
            has_passcode,
            app_lock_enabled: has_passcode,
            message,
            state_version: ver,
        })
    }

    pub fn manage_trusted_senders(
        &self,
        input: &ManageTrustedSendersInput,
    ) -> Result<ManageTrustedSendersOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }

        let mut changed = false;
        match input.action {
            TrustedSenderActionDto::List => {}
            TrustedSenderActionDto::Add => {
                let raw = input.sender.as_deref().unwrap_or("");
                let mut valid = false;
                self.prefs
                    .mutate_and_save(|prefs| {
                        let before = prefs.allowed_remote_senders.len();
                        if prefs.allow_remote_sender(raw).is_some() {
                            valid = true;
                        }
                        let diff = prefs.allowed_remote_senders.len() != before;
                        changed = diff;
                        diff
                    })
                    .map_err(|e| IpcError::internal(format!("保存白名单失败: {e}"), None))?;
                if !valid {
                    return Err(IpcError::invalid_input("发件人邮箱格式无效", None));
                }
            }
            TrustedSenderActionDto::Remove => {
                let raw = input.sender.as_deref().unwrap_or("").trim();
                if raw.is_empty() {
                    return Err(IpcError::invalid_input("sender 不能为空", None));
                }
                self.prefs
                    .mutate_and_save(|prefs| {
                        let removed = prefs.remove_remote_sender(raw);
                        changed = removed;
                        removed
                    })
                    .map_err(|e| IpcError::internal(format!("保存白名单失败: {e}"), None))?;
            }
            TrustedSenderActionDto::Clear => {
                self.prefs
                    .mutate_and_save(|prefs| {
                        if prefs.allowed_remote_senders.is_empty() {
                            return false;
                        }
                        prefs.allowed_remote_senders.clear();
                        changed = true;
                        true
                    })
                    .map_err(|e| IpcError::internal(format!("清空白名单失败: {e}"), None))?;
            }
        }

        let ver = if changed {
            self.on_preferences_updated();
            let v = self.bump_state_version();
            self.emitter.emit_mail_changed(MailChangedEvent {
                state_version: v,
                reason: "preferencesUpdated".to_string(),
                account_id: None,
                folder_id: None,
                message_id: None,
            });
            v
        } else {
            self.state_version()
        };

        let senders = self.prefs.snapshot_raw().allowed_remote_senders;
        Ok(ManageTrustedSendersOutput {
            senders,
            state_version: ver,
        })
    }

    pub fn clear_body_caches(
        &self,
        input: &ClearBodyCachesInput,
    ) -> Result<ClearBodyCachesOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }

        let keep_days = input.keep_days.unwrap_or(0);
        if keep_days > 36_500 {
            return Err(IpcError::invalid_input(
                "keepDays 超出合理范围 (0..=36500)",
                None,
            ));
        }

        let active_dir = match self.mode {
            AppMode::Demo => self.prefs.data_dir().to_path_buf(),
            AppMode::Live => {
                self.clear_live_body_cache();
                let (data_dir, worker) = {
                    let live = self.live.lock().expect("live mutex poisoned");
                    (live.data_dir.clone(), live.worker.clone())
                };
                if kitemail_mail_core::config::global_db_in(&data_dir).exists() {
                    if let Ok(mut store) = CacheStore::open_for_data_dir_existing(&data_dir) {
                        if keep_days == 0 {
                            let _ = store.clear_message_bodies();
                        } else {
                            let _ = store
                                .prune_expired_downloads(keep_days, chrono::Utc::now().timestamp());
                        }
                        store.checkpoint_and_close_all();
                    }
                }
                if let Some(w) = worker {
                    if keep_days == 0 {
                        w.send(MailRequest::ClearBodyCaches);
                    } else {
                        w.send(MailRequest::PruneExpiredDownloads { keep_days });
                    }
                }
                data_dir
            }
        };

        let (_, _, storage_stats) = compute_storage_stats_for(&active_dir);
        let ver = self.bump_state_version();
        let message = if keep_days == 0 {
            "已清理全部离线正文缓存".to_string()
        } else {
            format!("已清理 {keep_days} 天前的过期离线缓存")
        };

        Ok(ClearBodyCachesOutput {
            accepted: true,
            message,
            storage_stats,
            state_version: ver,
        })
    }

    pub fn check_app_update(&self) -> Result<CheckAppUpdateOutput, IpcError> {
        if self.is_locked() {
            return Err(IpcError::locked(None));
        }

        let current_version = app_version().to_string();
        let target = update::current_target().to_string();

        if self.mode == AppMode::Demo {
            return Ok(CheckAppUpdateOutput {
                checked: true,
                update_available: false,
                current_version: current_version.clone(),
                target,
                latest_version: Some(current_version),
                notes: None,
                pub_date: None,
                download_url: None,
                message: "演示模式不执行远程更新检查".to_string(),
            });
        }

        match update::check(&current_version) {
            Ok(Some(info)) => Ok(CheckAppUpdateOutput {
                checked: true,
                update_available: true,
                current_version,
                target: info.target,
                latest_version: Some(info.version.clone()),
                notes: non_empty_opt(&info.notes),
                pub_date: non_empty_opt(&info.pub_date),
                download_url: Some(info.url),
                message: format!("发现新版本 {}", info.version),
            }),
            Ok(None) => Ok(CheckAppUpdateOutput {
                checked: true,
                update_available: false,
                current_version: current_version.clone(),
                target,
                latest_version: Some(current_version),
                notes: None,
                pub_date: None,
                download_url: None,
                message: "当前已是最新版本".to_string(),
            }),
            Err(err) => Ok(CheckAppUpdateOutput {
                checked: false,
                update_available: false,
                current_version,
                target,
                latest_version: None,
                notes: None,
                pub_date: None,
                download_url: None,
                message: format!("检查更新失败: {err}"),
            }),
        }
    }
}

#[tauri::command]
pub fn get_full_preferences(
    state: State<'_, AppState>,
) -> Result<GetFullPreferencesOutput, IpcError> {
    state.session.get_full_preferences()
}

#[tauri::command]
pub fn update_full_preferences(
    state: State<'_, AppState>,
    input: FullPreferencesPatchDto,
) -> Result<UpdateFullPreferencesOutput, IpcError> {
    state.session.update_full_preferences(&input)
}

#[tauri::command]
pub fn set_app_lock_passcode(
    state: State<'_, AppState>,
    input: SetAppLockPasscodeInput,
) -> Result<SetAppLockPasscodeOutput, IpcError> {
    state.session.set_app_lock_passcode(&input)
}

#[tauri::command]
pub fn manage_trusted_senders(
    state: State<'_, AppState>,
    input: ManageTrustedSendersInput,
) -> Result<ManageTrustedSendersOutput, IpcError> {
    state.session.manage_trusted_senders(&input)
}

#[tauri::command]
pub fn clear_body_caches(
    state: State<'_, AppState>,
    input: ClearBodyCachesInput,
) -> Result<ClearBodyCachesOutput, IpcError> {
    state.session.clear_body_caches(&input)
}

#[tauri::command]
pub fn check_app_update(state: State<'_, AppState>) -> Result<CheckAppUpdateOutput, IpcError> {
    state.session.check_app_update()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::{ReaderPositionDto, UnlockAppInput};
    use crate::mail_session::tests::{make_live_test_session, make_test_session};
    use crate::preferences::{
        resolve_live_data_dir_with, resolve_preview_data_dir_with, verify_isolated_live_dirs,
        PreferencesStore,
    };
    use kitemail_mail_core::config::{
        load_accounts_in, resolve_data_dir, save_accounts_in, save_preferences_in, AccountConfig,
        AppPreferences,
    };
    use kitemail_mail_core::update::{TARGET_INSTALLER, TARGET_PORTABLE};

    #[test]
    fn full_preferences_boundary_validation_and_main_prefs_sync_work() {
        let (session, _, dir) = make_test_session();

        let bad_scale = session
            .update_full_preferences(&FullPreferencesPatchDto {
                ui_text_scale_percent: Some(125),
                ..Default::default()
            })
            .unwrap_err();
        assert_eq!(bad_scale.code, "INVALID_INPUT");

        let bad_mode = session
            .update_full_preferences(&FullPreferencesPatchDto {
                read_mark_mode: Some("instant".to_string()),
                ..Default::default()
            })
            .unwrap_err();
        assert_eq!(bad_mode.code, "INVALID_INPUT");

        let bad_sync = session
            .update_full_preferences(&FullPreferencesPatchDto {
                auto_sync_minutes: Some(99_999),
                ..Default::default()
            })
            .unwrap_err();
        assert_eq!(bad_sync.code, "INVALID_INPUT");

        let bad_undo = session
            .update_full_preferences(&FullPreferencesPatchDto {
                undo_send_secs: Some(999),
                ..Default::default()
            })
            .unwrap_err();
        assert_eq!(bad_undo.code, "INVALID_INPUT");

        let updated = session
            .update_full_preferences(&FullPreferencesPatchDto {
                theme_id: Some("midnight".to_string()),
                dark_mode: Some(Some(true)),
                reader_position: Some(ReaderPositionDto::Below),
                ui_text_scale_percent: Some(120),
                auto_sync_minutes: Some(10),
                push_enabled: Some(false),
                read_mark_mode: Some("delay".to_string()),
                undo_send_secs: Some(15),
                default_view: Some("last".to_string()),
                ..Default::default()
            })
            .expect("valid full preferences update");
        assert_eq!(updated.preferences.theme_id, "midnight");
        assert_eq!(updated.preferences.ui_text_scale_percent, 120);
        assert_eq!(updated.preferences.read_mark_mode, "delay");
        assert_eq!(updated.preferences.undo_send_secs, 15);

        // Must stay synchronized with MainPreferencesDto and persist to disk.
        let boot = session.bootstrap();
        assert_eq!(boot.preferences.theme_id, "midnight");
        assert_eq!(boot.preferences.dark_mode, Some(true));
        assert_eq!(boot.preferences.reader_position, ReaderPositionDto::Below);

        let reopened = PreferencesStore::open_in(dir.clone()).snapshot_dto();
        assert_eq!(reopened.theme_id, "midnight");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn passcode_set_verify_clear_and_debug_redaction_work() {
        let (session, _, dir) = make_test_session();

        let debug_str = format!(
            "{:?}",
            SetAppLockPasscodeInput {
                current_passcode: Some("old-secret-999".to_string()),
                new_passcode: Some("new-secret-888".to_string()),
                enable_on_startup: Some(true),
                idle_minutes: Some(10),
            }
        );
        assert!(!debug_str.contains("old-secret-999"));
        assert!(!debug_str.contains("new-secret-888"));
        assert!(debug_str.contains("[REDACTED]"));

        // Too short passcode rejected without leaking input.
        let too_short = session
            .set_app_lock_passcode(&SetAppLockPasscodeInput {
                current_passcode: None,
                new_passcode: Some("12".to_string()),
                enable_on_startup: None,
                idle_minutes: None,
            })
            .unwrap_err();
        assert_eq!(too_short.code, "INVALID_INPUT");

        let set_out = session
            .set_app_lock_passcode(&SetAppLockPasscodeInput {
                current_passcode: None,
                new_passcode: Some("pass-1234".to_string()),
                enable_on_startup: Some(true),
                idle_minutes: Some(10),
            })
            .expect("set initial passcode");
        assert!(set_out.has_passcode);
        assert!(set_out.app_lock_enabled);

        // Changing without valid currentPasscode must fail.
        let wrong_curr = session
            .set_app_lock_passcode(&SetAppLockPasscodeInput {
                current_passcode: Some("wrong-curr".to_string()),
                new_passcode: Some("pass-5678".to_string()),
                enable_on_startup: None,
                idle_minutes: None,
            })
            .unwrap_err();
        assert_eq!(wrong_curr.code, "INVALID_INPUT");

        // Clearing with valid currentPasscode succeeds and turns off startup lock.
        let cleared = session
            .set_app_lock_passcode(&SetAppLockPasscodeInput {
                current_passcode: Some("pass-1234".to_string()),
                new_passcode: None,
                enable_on_startup: None,
                idle_minutes: None,
            })
            .expect("clear passcode");
        assert!(!cleared.has_passcode);
        assert!(!cleared.app_lock_enabled);
        assert!(!session.is_lock_on_startup());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn trusted_senders_cache_clear_update_and_lock_guard_work() {
        let (session, _, _, worker, dir) = make_live_test_session(true, Some("1234"));

        // Trusted senders CRUD.
        let added = session
            .manage_trusted_senders(&ManageTrustedSendersInput {
                action: TrustedSenderActionDto::Add,
                sender: Some("Alice <Alice@Example.com>".to_string()),
            })
            .expect("add trusted sender");
        assert_eq!(added.senders, vec!["alice@example.com".to_string()]);

        let removed = session
            .manage_trusted_senders(&ManageTrustedSendersInput {
                action: TrustedSenderActionDto::Remove,
                sender: Some("alice@example.com".to_string()),
            })
            .expect("remove trusted sender");
        assert!(removed.senders.is_empty());

        // Clear body caches in Live mode dispatches worker request and preserves account.
        let cleared = session
            .clear_body_caches(&ClearBodyCachesInput { keep_days: Some(0) })
            .expect("clear body caches");
        assert!(cleared.accepted);
        assert!(worker
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|r| matches!(r, MailRequest::ClearBodyCaches)));

        // Lock rejects all 6 settings commands.
        session.lock_app(Some("manual"));
        assert_eq!(session.get_full_preferences().unwrap_err().code, "LOCKED");
        assert_eq!(
            session
                .update_full_preferences(&FullPreferencesPatchDto::default())
                .unwrap_err()
                .code,
            "LOCKED"
        );
        assert_eq!(
            session
                .set_app_lock_passcode(&SetAppLockPasscodeInput::default())
                .unwrap_err()
                .code,
            "LOCKED"
        );
        assert_eq!(
            session
                .manage_trusted_senders(&ManageTrustedSendersInput {
                    action: TrustedSenderActionDto::List,
                    sender: None,
                })
                .unwrap_err()
                .code,
            "LOCKED"
        );
        assert_eq!(
            session
                .clear_body_caches(&ClearBodyCachesInput::default())
                .unwrap_err()
                .code,
            "LOCKED"
        );
        assert_eq!(session.check_app_update().unwrap_err().code, "LOCKED");

        session.unlock_app(&UnlockAppInput {
            passcode: "1234".to_string(),
        });
        let full = session.get_full_preferences().expect("get full prefs");
        assert!(full.has_passcode);
        assert!(full.preferences.app_lock_enabled);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn demo_update_check_never_touches_network() {
        let (session, _, dir) = make_test_session();
        let res = session.check_app_update().expect("demo check update");
        assert!(res.checked);
        assert!(!res.update_available);
        assert_eq!(res.current_version, env!("CARGO_PKG_VERSION"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn portable_flag_and_upgrade_data_dir_compatibility_are_preserved() {
        let base = std::env::temp_dir().join(format!(
            "kitemail-p4-portable-upgrade-{}",
            uuid::Uuid::new_v4()
        ));
        let exe_dir = base.join("portable-app");
        let local_app_data = base.join("local-app-data");
        std::fs::create_dir_all(&exe_dir).unwrap();
        std::fs::create_dir_all(&local_app_data).unwrap();

        // 1. Portable flag detection & data dir resolution.
        assert!(!exe_dir.join("portable.flag").is_file());
        std::fs::write(exe_dir.join("portable.flag"), b"").unwrap();
        assert!(exe_dir.join("portable.flag").is_file());

        let prod_portable = resolve_data_dir(None, true, Some(&exe_dir), Some(&local_app_data));
        assert_eq!(prod_portable, exe_dir.join("data"));
        let preview_portable =
            resolve_preview_data_dir_with(None, None, true, Some(&exe_dir), Some(&local_app_data));
        assert_eq!(preview_portable, exe_dir.join("data-preview"));
        let live_portable =
            resolve_live_data_dir_with(None, None, true, Some(&exe_dir), Some(&local_app_data));
        assert_eq!(live_portable, prod_portable);
        let live_installed =
            resolve_live_data_dir_with(None, None, false, Some(&exe_dir), Some(&local_app_data));
        assert_eq!(live_installed, local_app_data.join("KiteMail"));
        assert!(verify_isolated_live_dirs(&preview_portable, &[prod_portable]).is_ok());

        // Target strings match mail-core installer vs portable contract.
        assert_ne!(TARGET_INSTALLER, TARGET_PORTABLE);

        // 2. Upgrade compatibility: legacy preferences.toml & accounts.toml in existing data_dir.
        let upgrade_dir = base.join("legacy-upgrade-profile");
        std::fs::create_dir_all(&upgrade_dir).unwrap();
        let legacy_prefs = AppPreferences {
            theme_id: "rose".to_string(),
            compact_list: true,
            signature_above_quote: false,
            signature_above_quote_default_applied: false,
            thread_newest_first: false,
            thread_newest_first_default_applied: false,
            allowed_remote_senders: vec!["trusted@example.com".to_string()],
            ..AppPreferences::default()
        };
        save_preferences_in(&upgrade_dir, &legacy_prefs).unwrap();
        save_accounts_in(
            &upgrade_dir,
            &[AccountConfig {
                id: 1,
                display_name: "Legacy User".to_string(),
                email: "legacy@example.com".to_string(),
                ..AccountConfig::default()
            }],
        )
        .unwrap();

        let store = PreferencesStore::open_in(upgrade_dir.clone());
        let dto = to_full_preferences_dto(&store.snapshot_raw(), false, false);
        assert_eq!(dto.theme_id, "rose");
        assert!(dto.compact_list);
        assert!(dto.signature_above_quote);
        assert!(dto.thread_newest_first);

        let loaded_accounts = load_accounts_in(&upgrade_dir).unwrap();
        assert_eq!(loaded_accounts.len(), 1);
        assert_eq!(loaded_accounts[0].email, "legacy@example.com");

        let (_, _, stats) = compute_storage_stats_for(&upgrade_dir);
        assert!(stats.config_bytes > 0);
        assert!(stats.file_count >= 2);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn build_rs_main_rs_and_capabilities_register_all_17_wave3_commands() {
        let wave3_commands = [
            "list_cached_attachments",
            "fetch_attachment_preview",
            "save_attachment",
            "save_all_attachments",
            "get_message_print_html",
            "open_message_window",
            "list_contacts",
            "save_contact",
            "delete_contact",
            "import_contacts",
            "export_contacts",
            "get_full_preferences",
            "update_full_preferences",
            "set_app_lock_passcode",
            "manage_trusted_senders",
            "clear_body_caches",
            "check_app_update",
        ];

        let build_rs = include_str!("../build.rs");
        let main_rs = include_str!("main.rs");
        let cap_json = include_str!("../capabilities/main.json");

        for cmd in wave3_commands {
            assert!(
                build_rs.contains(&format!("\"{cmd}\"")),
                "build.rs missing command {cmd}"
            );
            assert!(
                main_rs.contains(cmd),
                "main.rs invoke_handler missing command {cmd}"
            );
            let perm = format!("allow-{}", cmd.replace('_', "-"));
            assert!(
                cap_json.contains(&format!("\"{perm}\"")),
                "capabilities/main.json missing permission {perm}"
            );
        }
    }
}
