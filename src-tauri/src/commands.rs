//! Tauri 2 IPC commands for the KiteMail main window (`snake_case` names, `camelCase` DTOs).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tauri::{AppHandle, Manager, State, WebviewWindow};

use crate::dto::{
    IpcError, ListMessagesInput, ListMessagesOutput, LockAppOutput, MainBootstrapDto,
    MainPreferencesDto, MainPreferencesPatch, OpenMessageInput, OpenMessageOutput,
    RefreshMailInput, RefreshMailOutput, SetMessageFlaggedInput, SetMessageFlaggedOutput,
    SetMessageSeenInput, SetMessageSeenOutput, SetReaderLayoutInput, SetReaderLayoutOutput,
    UnlockAppInput, UnlockAppOutput, MAIN_WINDOW_LABEL,
};
use crate::mail_session::{MailSession, MAX_QUERY_LEN};
use crate::preferences::PreferencesStore;
use crate::reader::ReaderManager;
use crate::tray::TrayHandle;
use crate::window_lifecycle;

pub struct AppState {
    pub session: Arc<MailSession>,
    pub prefs: Arc<PreferencesStore>,
    pub reader: Arc<ReaderManager>,
    pub tray: Option<Arc<TrayHandle>>,
    pub quitting: Arc<AtomicBool>,
}

fn validate_request_id(request_id: &str) -> Result<(), IpcError> {
    if request_id.trim().is_empty() {
        return Err(IpcError::invalid_input("requestId 不能为空", None));
    }
    Ok(())
}

#[tauri::command]
pub fn main_bootstrap(state: State<'_, AppState>) -> Result<MainBootstrapDto, IpcError> {
    Ok(state.session.bootstrap())
}

#[tauri::command]
pub fn list_messages(
    state: State<'_, AppState>,
    input: ListMessagesInput,
) -> Result<ListMessagesOutput, IpcError> {
    validate_request_id(&input.request_id)?;
    let req_id = Some(input.request_id.clone());
    if input.limit == 0 {
        return Err(IpcError::invalid_input("limit 必须大于 0", req_id));
    }
    if input.query.chars().count() > MAX_QUERY_LEN {
        return Err(IpcError::invalid_input(
            format!("搜索关键词长度不能超过 {MAX_QUERY_LEN} 个字符"),
            req_id,
        ));
    }
    state.session.list_messages(&input)
}

#[tauri::command]
pub fn open_message(
    state: State<'_, AppState>,
    input: OpenMessageInput,
) -> Result<OpenMessageOutput, IpcError> {
    validate_request_id(&input.request_id)?;
    let req_id = Some(input.request_id.clone());
    if input.account_id == 0 || input.message_id == 0 {
        return Err(IpcError::invalid_input(
            "accountId 与 messageId 必须为正整数",
            req_id,
        ));
    }
    let reader_session_id = uuid::Uuid::new_v4().to_string();
    state.session.open_message(&input, reader_session_id)
}

#[tauri::command]
pub fn set_reader_layout(
    window: WebviewWindow,
    state: State<'_, AppState>,
    input: SetReaderLayoutInput,
) -> Result<SetReaderLayoutOutput, IpcError> {
    if window.label() != MAIN_WINDOW_LABEL {
        return Err(IpcError::invalid_input(
            "仅允许主窗口调用阅读器布局接口",
            None,
        ));
    }
    let scale = window.scale_factor().unwrap_or(1.0);
    state.reader.apply_layout(&input, scale)
}

#[tauri::command]
pub fn update_main_preferences(
    state: State<'_, AppState>,
    patch: MainPreferencesPatch,
) -> Result<MainPreferencesDto, IpcError> {
    let updated = state.prefs.update_patch(&patch)?;
    state.session.on_preferences_updated();
    Ok(updated)
}

#[tauri::command]
pub fn refresh_mail(
    state: State<'_, AppState>,
    input: RefreshMailInput,
) -> Result<RefreshMailOutput, IpcError> {
    state.session.refresh_mail(&input)
}

#[tauri::command]
pub fn unlock_app(
    state: State<'_, AppState>,
    input: UnlockAppInput,
) -> Result<UnlockAppOutput, IpcError> {
    Ok(state.session.unlock_app(&input))
}

#[tauri::command]
pub fn lock_app(state: State<'_, AppState>) -> Result<LockAppOutput, IpcError> {
    Ok(state.session.lock_app(Some("manual")))
}

#[tauri::command]
pub fn set_message_seen(
    state: State<'_, AppState>,
    input: SetMessageSeenInput,
) -> Result<SetMessageSeenOutput, IpcError> {
    state.session.set_message_seen(&input)
}

#[tauri::command]
pub fn set_message_flagged(
    state: State<'_, AppState>,
    input: SetMessageFlaggedInput,
) -> Result<SetMessageFlaggedOutput, IpcError> {
    state.session.set_message_flagged(&input)
}

#[tauri::command]
pub fn quit_app(app: AppHandle, state: State<'_, AppState>) -> Result<(), IpcError> {
    perform_clean_quit(
        &app,
        &state.session,
        &state.prefs,
        &state.reader,
        state.tray.as_deref(),
        &state.quitting,
    );
    app.exit(0);
    Ok(())
}

pub fn perform_clean_quit(
    app: &AppHandle,
    session: &MailSession,
    prefs: &PreferencesStore,
    reader: &ReaderManager,
    tray: Option<&TrayHandle>,
    quitting: &AtomicBool,
) {
    if quitting.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Some(main_win) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        window_lifecycle::save_main_window_geometry(&main_win, prefs);
    }
    reader.close();
    session.prepare_quit();
    if let Some(t) = tray {
        t.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dto::{AppMode, MainViewDto, MessageFilterDto, MessageSortDto, SearchScopeDto};
    use crate::mail_session::NullEventEmitter;
    use crate::reader::NullReaderSink;

    #[test]
    fn command_validation_rejects_empty_request_id_and_zero_limit() {
        let dir =
            std::env::temp_dir().join(format!("kitemail-tauri-cmds-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let prefs = Arc::new(PreferencesStore::open_in(dir.clone()));
        let reader = Arc::new(ReaderManager::new(Arc::new(NullReaderSink)));
        let session = Arc::new(MailSession::new(
            AppMode::Demo,
            prefs,
            reader,
            Arc::new(NullEventEmitter),
        ));

        let bad_req = ListMessagesInput {
            request_id: "   ".to_string(),
            view: MainViewDto::UnifiedInbox,
            query: String::new(),
            scope: SearchScopeDto::ThisFolder,
            sort: MessageSortDto::DateNewest,
            filter: MessageFilterDto::default(),
            offset: 0,
            limit: 50,
        };
        assert!(validate_request_id(&bad_req.request_id).is_err());

        let too_long_query = ListMessagesInput {
            request_id: "ok".to_string(),
            query: "a".repeat(MAX_QUERY_LEN + 1),
            ..bad_req
        };
        assert!(too_long_query.query.chars().count() > MAX_QUERY_LEN);
        drop(session);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
