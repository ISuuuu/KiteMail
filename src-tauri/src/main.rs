//! KiteMail Tauri 2 desktop preview entry point.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod accounts;
mod attachments;
mod commands;
mod compose;
mod contacts;
mod dto;
mod mail_session;
mod preferences;
mod reader;
mod settings;
mod single_instance;
mod tray;
mod win_idle;
mod window_lifecycle;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tauri::{Manager, WindowEvent};

use crate::commands::{perform_clean_quit, AppState};
use crate::dto::{AppMode, RefreshMailInput, MAIN_WINDOW_LABEL};
use crate::mail_session::{MailSession, TauriMainEventEmitter};
use crate::preferences::{
    preview_data_dir, reader_webview2_user_data_dir, resolve_and_verify_live_data_dir,
    PreferencesStore,
};
use crate::reader::ReaderManager;
use crate::tray::{TrayCommand, TrayHandle};

const TRAY_ICON_PNG: &[u8] = include_bytes!("../icons/icon.png");

#[cfg(windows)]
fn show_startup_error_dialog(title: &str, message: &str) {
    use windows::core::PCWSTR;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

    let wide_title: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    let wide_msg: Vec<u16> = message.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let _ = MessageBoxW(
            None,
            PCWSTR(wide_msg.as_ptr()),
            PCWSTR(wide_title.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}

#[cfg(not(windows))]
fn show_startup_error_dialog(_title: &str, _message: &str) {}

fn parse_launch_mode<I, S>(args: I, demo_env: Option<&str>) -> AppMode
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    if args.into_iter().any(|arg| arg.as_ref() == "--demo") {
        return AppMode::Demo;
    }
    if demo_env.is_some_and(|v| matches!(v.trim(), "1" | "true" | "TRUE")) {
        return AppMode::Demo;
    }
    AppMode::Live
}

fn main() {
    let mode = parse_launch_mode(
        std::env::args(),
        std::env::var("KITEMAIL_DEMO").ok().as_deref(),
    );

    let Some(_single_instance_guard) = single_instance::acquire_or_activate() else {
        return;
    };

    let data_dir = match mode {
        AppMode::Demo => preview_data_dir(),
        AppMode::Live => match resolve_and_verify_live_data_dir() {
            Ok((dir, _mail_dir)) => {
                // Ensure `kitemail_mail_core::secrets` / `config::data_dir()` also stay isolated
                // inside `dir` instead of falling back to production `%LOCALAPPDATA%\KiteMail`.
                if std::env::var_os("KITEMAIL_DATA_DIR").is_none_or(|v| v.is_empty()) {
                    std::env::set_var("KITEMAIL_DATA_DIR", &dir);
                }
                dir
            }
            Err(err) => {
                eprintln!("{err}");
                show_startup_error_dialog("KiteMail 启动失败", &err);
                return;
            }
        },
    };

    let Some(_data_dir_guard) = single_instance::acquire_data_dir_lock(&data_dir) else {
        let msg = format!(
            "资料目录 {} 正被另一个 KiteMail 进程占用，禁止并发写入。",
            data_dir.display()
        );
        eprintln!("{msg}");
        show_startup_error_dialog("KiteMail 启动失败", &msg);
        return;
    };

    let prefs_store = Arc::new(PreferencesStore::open_in(data_dir.clone()));
    let tray = TrayHandle::start(TRAY_ICON_PNG).map(Arc::new);
    let quitting = Arc::new(AtomicBool::new(false));

    let tray_for_setup = tray.clone();
    let quitting_for_setup = Arc::clone(&quitting);
    let data_dir_for_setup = data_dir.clone();

    tauri::Builder::default()
        .setup(move |app| {
            let handle = app.handle().clone();

            #[cfg(windows)]
            let reader_sink: Arc<dyn crate::reader::ReaderPlatformSink> =
                Arc::new(crate::reader::platform_win::TauriWindowsReaderSink::new(
                    handle.clone(),
                    reader_webview2_user_data_dir(&data_dir_for_setup),
                ));
            #[cfg(not(windows))]
            let reader_sink: Arc<dyn crate::reader::ReaderPlatformSink> =
                Arc::new(crate::reader::NullReaderSink);

            let reader = Arc::new(ReaderManager::new(reader_sink));
            let emitter = Arc::new(TauriMainEventEmitter::new(handle.clone()));
            let session = match mode {
                AppMode::Demo => {
                    let s = Arc::new(MailSession::new(
                        AppMode::Demo,
                        prefs_store.clone(),
                        reader.clone(),
                        emitter,
                    ));
                    s.bind_self(&s);
                    s
                }
                AppMode::Live => MailSession::new_live(
                    data_dir_for_setup.clone(),
                    prefs_store.clone(),
                    reader.clone(),
                    emitter,
                ),
            };
            {
                let handler: Arc<dyn crate::reader::ThreadActionHandler> = session.clone();
                reader.set_action_handler(Arc::downgrade(&handler));
            }

            if let Some(ref t) = tray_for_setup {
                session.attach_tray(Arc::clone(t));
            }

            app.manage(AppState {
                session: Arc::clone(&session),
                prefs: prefs_store.clone(),
                reader: reader.clone(),
                tray: tray_for_setup.clone(),
                quitting: Arc::clone(&quitting_for_setup),
            });

            if let Some(main_win) = app.get_webview_window(MAIN_WINDOW_LABEL) {
                #[cfg(windows)]
                if let Some(ref t) = tray_for_setup {
                    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
                    if let Ok(raw) = HasWindowHandle::window_handle(&main_win) {
                        if let RawWindowHandle::Win32(win32) = raw.as_raw() {
                            t.set_main_hwnd(win32.hwnd.get());
                        }
                    }
                }

                let raw_prefs = prefs_store.snapshot_raw();
                window_lifecycle::restore_main_window_geometry(&main_win, &raw_prefs);

                let win_for_events = main_win.clone();
                let prefs_for_events = prefs_store.clone();
                let reader_for_events = reader.clone();
                let session_for_events = Arc::clone(&session);
                let tray_for_events = tray_for_setup.clone();
                let quitting_for_events = Arc::clone(&quitting_for_setup);
                let handle_for_events = handle.clone();

                main_win.on_window_event(move |event| match event {
                    WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                        window_lifecycle::save_main_window_geometry(
                            &win_for_events,
                            &prefs_for_events,
                        );
                    }
                    WindowEvent::CloseRequested { api, .. } => {
                        window_lifecycle::save_main_window_geometry(
                            &win_for_events,
                            &prefs_for_events,
                        );
                        let close_to_tray = !quitting_for_events.load(Ordering::SeqCst)
                            && prefs_for_events.snapshot_raw().close_to_minimize
                            && tray_for_events.is_some();
                        if close_to_tray {
                            api.prevent_close();
                            if let Some(ref t) = tray_for_events {
                                if !t.hide_main_window() {
                                    let _ = win_for_events.hide();
                                }
                            }
                        } else {
                            perform_clean_quit(
                                &handle_for_events,
                                &session_for_events,
                                &prefs_for_events,
                                &reader_for_events,
                                tray_for_events.as_deref(),
                                &quitting_for_events,
                            );
                        }
                    }
                    WindowEvent::Destroyed => {
                        perform_clean_quit(
                            &handle_for_events,
                            &session_for_events,
                            &prefs_for_events,
                            &reader_for_events,
                            tray_for_events.as_deref(),
                            &quitting_for_events,
                        );
                    }
                    _ => {}
                });
            }

            // Spawn background 1s monitor for tray actions, minimize_to_tray, and auto-lock.
            let handle_for_tick = handle.clone();
            let session_for_tick = Arc::clone(&session);
            let prefs_for_tick = prefs_store.clone();
            let reader_for_tick = reader.clone();
            let tray_for_tick = tray_for_setup.clone();
            let quitting_for_tick = Arc::clone(&quitting_for_setup);
            let _ = std::thread::Builder::new()
                .name("kitemail-tauri-tray-lock-tick".to_string())
                .spawn(move || {
                    while !quitting_for_tick.load(Ordering::SeqCst) {
                        std::thread::sleep(Duration::from_secs(1));
                        if quitting_for_tick.load(Ordering::SeqCst) {
                            break;
                        }
                        if let Some(ref t) = tray_for_tick {
                            t.set_lock_enabled(session_for_tick.has_passcode());
                            if prefs_for_tick.snapshot_raw().minimize_to_tray
                                && t.is_main_window_minimized()
                            {
                                t.hide_main_window();
                            }
                            for cmd in t.drain_commands() {
                                match cmd {
                                    TrayCommand::ShowWindow => {
                                        t.show_main_window();
                                        if let Some(win) =
                                            handle_for_tick.get_webview_window(MAIN_WINDOW_LABEL)
                                        {
                                            let _ = win.show();
                                            let _ = win.unminimize();
                                            let _ = win.set_focus();
                                        }
                                    }
                                    TrayCommand::SyncAll => {
                                        if !session_for_tick.is_locked() {
                                            let _ =
                                                session_for_tick.refresh_mail(&RefreshMailInput {
                                                    view: session_for_tick.current_view(),
                                                });
                                        }
                                    }
                                    TrayCommand::LockNow => {
                                        session_for_tick.lock_app(Some("manual"));
                                    }
                                    TrayCommand::SystemLockOrSuspend => {
                                        if prefs_for_tick.snapshot_raw().app_lock_on_system_lock {
                                            session_for_tick.lock_app(Some("systemLock"));
                                        }
                                    }
                                    TrayCommand::Quit => {
                                        perform_clean_quit(
                                            &handle_for_tick,
                                            &session_for_tick,
                                            &prefs_for_tick,
                                            &reader_for_tick,
                                            Some(t),
                                            &quitting_for_tick,
                                        );
                                        handle_for_tick.exit(0);
                                        return;
                                    }
                                }
                            }
                        }
                        session_for_tick.auto_lock_tick(
                            win_idle::is_current_process_foreground(),
                            win_idle::system_idle_duration(),
                        );
                    }
                });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::main_bootstrap,
            commands::list_messages,
            commands::open_message,
            commands::set_reader_layout,
            commands::update_main_preferences,
            commands::refresh_mail,
            commands::unlock_app,
            commands::lock_app,
            commands::set_message_seen,
            commands::set_message_flagged,
            commands::allow_remote_images,
            commands::load_older_messages,
            commands::archive_message,
            commands::delete_message,
            commands::mark_message_spam,
            commands::move_message,
            commands::undo_message_move,
            commands::quit_app,
            compose::list_drafts,
            compose::save_draft,
            compose::delete_draft,
            compose::send_mail,
            compose::undo_send_mail,
            compose::list_outbox,
            compose::retry_outbox_item,
            compose::delete_outbox_item,
            accounts::list_accounts,
            accounts::verify_account,
            accounts::save_account,
            accounts::delete_account,
            accounts::set_account_enabled,
            accounts::start_microsoft_oauth,
            attachments::list_cached_attachments,
            attachments::fetch_attachment_preview,
            attachments::save_attachment,
            attachments::save_all_attachments,
            attachments::get_message_print_html,
            attachments::open_message_window,
            contacts::list_contacts,
            contacts::save_contact,
            contacts::delete_contact,
            contacts::import_contacts,
            contacts::export_contacts,
            settings::get_full_preferences,
            settings::update_full_preferences,
            settings::set_app_lock_passcode,
            settings::manage_trusted_senders,
            settings::clear_body_caches,
            settings::check_app_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running KiteMail Tauri preview");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_mode_parses_demo_flag_and_env() {
        assert_eq!(parse_launch_mode(["kitemail.exe"], None), AppMode::Live);
        assert_eq!(
            parse_launch_mode(["kitemail.exe", "--demo"], None),
            AppMode::Demo
        );
        assert_eq!(
            parse_launch_mode(["kitemail.exe"], Some("1")),
            AppMode::Demo
        );
    }
}
