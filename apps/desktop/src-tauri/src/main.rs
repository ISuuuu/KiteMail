//! KiteMail Tauri 2 desktop preview entry point.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod dto;
mod mail_session;
mod preferences;
mod reader;
mod window_lifecycle;

use std::sync::Arc;

use tauri::{Manager, WindowEvent};

use crate::commands::AppState;
use crate::dto::{AppMode, MAIN_WINDOW_LABEL};
use crate::mail_session::{MailSession, TauriMainEventEmitter};
use crate::preferences::{preview_data_dir, reader_webview2_user_data_dir, PreferencesStore};
use crate::reader::ReaderManager;

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
    let data_dir = preview_data_dir();
    let prefs_store = Arc::new(PreferencesStore::open_in(data_dir.clone()));

    tauri::Builder::default()
        .setup(move |app| {
            let handle = app.handle().clone();

            #[cfg(windows)]
            let reader_sink: Arc<dyn crate::reader::ReaderPlatformSink> =
                Arc::new(crate::reader::platform_win::TauriWindowsReaderSink::new(
                    handle.clone(),
                    reader_webview2_user_data_dir(&data_dir),
                ));
            #[cfg(not(windows))]
            let reader_sink: Arc<dyn crate::reader::ReaderPlatformSink> =
                Arc::new(crate::reader::NullReaderSink);

            let reader = Arc::new(ReaderManager::new(reader_sink));
            let emitter = Arc::new(TauriMainEventEmitter::new(handle.clone()));
            let session = Arc::new(MailSession::new(
                mode,
                prefs_store.clone(),
                reader.clone(),
                emitter,
            ));

            app.manage(AppState {
                session,
                prefs: prefs_store.clone(),
                reader: reader.clone(),
            });

            if let Some(main_win) = app.get_webview_window(MAIN_WINDOW_LABEL) {
                let raw_prefs = prefs_store.snapshot_raw();
                window_lifecycle::restore_main_window_geometry(&main_win, &raw_prefs);

                let win_for_events = main_win.clone();
                let prefs_for_events = prefs_store.clone();
                let reader_for_events = reader;
                main_win.on_window_event(move |event| match event {
                    WindowEvent::Resized(_) | WindowEvent::ScaleFactorChanged { .. } => {
                        window_lifecycle::save_main_window_geometry(
                            &win_for_events,
                            &prefs_for_events,
                        );
                    }
                    WindowEvent::CloseRequested { .. } => {
                        window_lifecycle::save_main_window_geometry(
                            &win_for_events,
                            &prefs_for_events,
                        );
                        reader_for_events.close();
                    }
                    WindowEvent::Destroyed => {
                        reader_for_events.close();
                    }
                    _ => {}
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::main_bootstrap,
            commands::list_messages,
            commands::open_message,
            commands::set_reader_layout,
            commands::update_main_preferences,
            commands::refresh_mail,
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
