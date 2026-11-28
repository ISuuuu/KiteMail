//! Isolated preview profile directory and whitelisted preference persistence.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use kitemail_mail_core::config::{
    default_mail_data_dir_in, load_preferences_in, normalize_app_lock_idle_minutes,
    resolve_data_dir, resolve_mail_data_dir_in, save_preferences_in, AppPreferences,
    UI_TEXT_SCALE_STEPS,
};

use crate::dto::{
    FullPreferencesDto, FullPreferencesPatchDto, IpcError, MainPreferencesDto,
    MainPreferencesPatch, MainViewDto, ReaderPositionDto,
};

pub const VALID_THEME_IDS: &[&str] = &[
    "system", "default", "midnight", "tidal", "rose", "earth", "forest",
];

pub const SIDEBAR_MIN_WIDTH: f32 = 180.0;
pub const SIDEBAR_DEFAULT_WIDTH: f32 = 255.0;
pub const SIDEBAR_MAX_WIDTH: f32 = 280.0;

pub const LIST_MIN_WIDTH: f32 = 280.0;
pub const LIST_DEFAULT_WIDTH: f32 = 350.0;
pub const LIST_MAX_WIDTH: f32 = 7680.0;

pub const LIST_BELOW_MIN_HEIGHT: f32 = 240.0;
pub const LIST_BELOW_DEFAULT_HEIGHT: f32 = 360.0;
pub const LIST_BELOW_MAX_HEIGHT: f32 = 4320.0;

pub const READER_ZOOM_MIN: u32 = 50;
pub const READER_ZOOM_MAX: u32 = 250;
pub const AUTO_SYNC_MAX_MINUTES: u32 = 1440;
pub const UNDO_SEND_MAX_SECS: u32 = 60;
pub const TEXT_PREF_MAX_CHARS: usize = 2000;

pub const VALID_READ_MARK_MODES: &[&str] = &["shown", "delay", "manual"];
pub const VALID_DEFAULT_VIEWS: &[&str] = &["unified", "last"];
pub const VALID_APP_LOCK_IDLE_MINUTES: &[u32] = &[0, 1, 5, 10, 15, 30, 60];

const PREVIEW_DATA_DIR_NAME: &str = "KiteMailPreview";
const STARTUP_LOCK_FLAG_FILE: &str = "app_lock_startup.flag";

pub fn is_startup_lock_enabled_in(data_dir: &Path) -> bool {
    data_dir.join(STARTUP_LOCK_FLAG_FILE).is_file()
}

pub fn set_startup_lock_enabled_in(data_dir: &Path, enabled: bool) -> Result<(), String> {
    let path = data_dir.join(STARTUP_LOCK_FLAG_FILE);
    if enabled {
        std::fs::create_dir_all(data_dir)
            .map_err(|e| format!("创建配置目录失败 ({}): {e}", data_dir.display()))?;
        std::fs::write(&path, b"startup_lock=true\n")
            .map_err(|e| format!("写入启动锁配置失败 ({}): {e}", path.display()))
    } else {
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("清除启动锁配置失败 ({}): {e}", path.display())),
        }
    }
}

/// Resolve the isolated preview data directory, keeping production `%LOCALAPPDATA%/KiteMail` untouched.
pub fn resolve_preview_data_dir_with(
    preview_env: Option<&str>,
    mail_env: Option<&str>,
    is_portable: bool,
    exe_dir: Option<&Path>,
    local_app_data: Option<&Path>,
) -> PathBuf {
    if let Some(raw) = preview_env.map(str::trim).filter(|s| !s.is_empty()) {
        return PathBuf::from(raw);
    }
    if let Some(raw) = mail_env.map(str::trim).filter(|s| !s.is_empty()) {
        return PathBuf::from(raw).join("preview");
    }
    if is_portable {
        if let Some(dir) = exe_dir {
            return dir.join("data-preview");
        }
    }
    local_app_data
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(PREVIEW_DATA_DIR_NAME)
}

/// Resolve the formal Live-mode data directory:
/// `KITEMAIL_DATA_DIR` > `KITEMAIL_PREVIEW_DATA_DIR` > portable `<exe_dir>\data` > `%LOCALAPPDATA%\KiteMail`.
pub fn resolve_live_data_dir_with(
    mail_env: Option<&str>,
    preview_env: Option<&str>,
    is_portable: bool,
    exe_dir: Option<&Path>,
    local_app_data: Option<&Path>,
) -> PathBuf {
    if let Some(raw) = mail_env.map(str::trim).filter(|s| !s.is_empty()) {
        return PathBuf::from(raw);
    }
    if let Some(raw) = preview_env.map(str::trim).filter(|s| !s.is_empty()) {
        return PathBuf::from(raw);
    }
    resolve_data_dir(None, is_portable, exe_dir, local_app_data)
}

#[allow(dead_code)]
pub fn production_default_data_dir_with(
    is_portable: bool,
    exe_dir: Option<&Path>,
    local_app_data: Option<&Path>,
) -> PathBuf {
    resolve_data_dir(None, is_portable, exe_dir, local_app_data)
}

fn normalized_path_key(path: &Path) -> String {
    let resolved = if path.exists() {
        std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
    } else {
        let mut cursor = Some(path);
        let mut suffix: Vec<std::ffi::OsString> = Vec::new();
        let mut base = None;
        while let Some(cur) = cursor {
            if cur.exists() {
                if let Ok(canon) = std::fs::canonicalize(cur) {
                    base = Some(canon);
                    break;
                }
            }
            if let Some(name) = cur.file_name() {
                suffix.push(name.to_os_string());
                cursor = cur.parent();
            } else {
                break;
            }
        }
        if let Some(mut rebuilt) = base {
            for comp in suffix.into_iter().rev() {
                rebuilt.push(comp);
            }
            rebuilt
        } else {
            path.to_path_buf()
        }
    };
    let s = resolved.to_string_lossy();
    let stripped = s.strip_prefix(r"\\?\").unwrap_or(&s);
    stripped
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase()
}

#[allow(dead_code)]
/// Verify that neither `data_dir` nor its resolved `mail_data_dir` points to any production default path.
pub fn verify_isolated_live_dirs(
    data_dir: &Path,
    prod_data_dirs: &[PathBuf],
) -> Result<PathBuf, String> {
    let data_key = normalized_path_key(data_dir);
    let mail_dir = resolve_mail_data_dir_in(data_dir);
    let mail_key = normalized_path_key(&mail_dir);

    for prod in prod_data_dirs {
        let prod_key = normalized_path_key(prod);
        let prod_mail_key = normalized_path_key(&default_mail_data_dir_in(prod));
        if data_key == prod_key || data_key == prod_mail_key {
            return Err(format!(
                "Live 预览模式禁止直接使用正式默认资料目录: {}",
                data_dir.display()
            ));
        }
        if mail_key == prod_key || mail_key == prod_mail_key {
            return Err(format!(
                "Live 预览模式禁止将实际邮件目录指向正式默认目录: {}",
                mail_dir.display()
            ));
        }
    }
    Ok(mail_dir)
}

/// Resolve the active Live-mode data directory (preserving `%LOCALAPPDATA%\KiteMail` / `<exe_dir>\data`
/// as well as custom `KITEMAIL_DATA_DIR` or explicit `KITEMAIL_PREVIEW_DATA_DIR`).
pub fn resolve_and_verify_live_data_dir() -> Result<(PathBuf, PathBuf), String> {
    let mail_env = std::env::var("KITEMAIL_DATA_DIR").ok();
    let preview_env = std::env::var("KITEMAIL_PREVIEW_DATA_DIR").ok();
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    let local_app_data = dirs::data_local_dir();
    let is_portable = kitemail_mail_core::update::is_portable();

    let data_dir = resolve_live_data_dir_with(
        mail_env.as_deref(),
        preview_env.as_deref(),
        is_portable,
        exe_dir.as_deref(),
        local_app_data.as_deref(),
    );

    let mail_dir = resolve_mail_data_dir_in(&data_dir);
    Ok((data_dir, mail_dir))
}

/// Active preview data directory for `preferences.toml` and isolated WebView2 profiles.
pub fn preview_data_dir() -> PathBuf {
    let preview_env = std::env::var("KITEMAIL_PREVIEW_DATA_DIR").ok();
    let mail_env = std::env::var("KITEMAIL_DATA_DIR").ok();
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    let local_app_data = dirs::data_local_dir();
    resolve_preview_data_dir_with(
        preview_env.as_deref(),
        mail_env.as_deref(),
        kitemail_mail_core::update::is_portable(),
        exe_dir.as_deref(),
        local_app_data.as_deref(),
    )
}

/// Isolated WebView2 user data directory for the embedded mail reader.
pub fn reader_webview2_user_data_dir(base_dir: &Path) -> PathBuf {
    base_dir.join("WebView2-reader")
}

/// Normalize an `AppPreferences` loaded from disk so corrupted/legacy values fall back safely.
pub fn normalize_loaded_preferences(prefs: &mut AppPreferences) {
    if !VALID_THEME_IDS.contains(&prefs.theme_id.as_str()) {
        prefs.theme_id = "system".to_string();
    }
    prefs.reader_position = ReaderPositionDto::from_pref_str(&prefs.reader_position)
        .as_str()
        .to_string();
    prefs.sidebar_width = prefs.sidebar_width.map(|w| {
        if w.is_finite() {
            w.clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH)
        } else {
            SIDEBAR_DEFAULT_WIDTH
        }
    });
    prefs.list_width = prefs.list_width.map(|w| {
        if w.is_finite() {
            w.clamp(LIST_MIN_WIDTH, LIST_MAX_WIDTH)
        } else {
            LIST_DEFAULT_WIDTH
        }
    });
    prefs.list_below_height = prefs.list_below_height.map(|h| {
        if h.is_finite() {
            h.clamp(LIST_BELOW_MIN_HEIGHT, LIST_BELOW_MAX_HEIGHT)
        } else {
            LIST_BELOW_DEFAULT_HEIGHT
        }
    });
    prefs.reader_zoom_percent = prefs
        .reader_zoom_percent
        .clamp(READER_ZOOM_MIN, READER_ZOOM_MAX);
}

pub fn to_main_preferences_dto(prefs: &AppPreferences) -> MainPreferencesDto {
    let theme_id = if VALID_THEME_IDS.contains(&prefs.theme_id.as_str()) {
        prefs.theme_id.clone()
    } else {
        "system".to_string()
    };
    let sidebar_width = prefs
        .sidebar_width
        .filter(|w| w.is_finite())
        .map(|w| w.clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH))
        .unwrap_or(SIDEBAR_DEFAULT_WIDTH);
    let list_width = prefs
        .list_width
        .filter(|w| w.is_finite())
        .map(|w| w.clamp(LIST_MIN_WIDTH, LIST_MAX_WIDTH))
        .unwrap_or(LIST_DEFAULT_WIDTH);
    let list_below_height = prefs
        .list_below_height
        .filter(|h| h.is_finite())
        .map(|h| h.clamp(LIST_BELOW_MIN_HEIGHT, LIST_BELOW_MAX_HEIGHT))
        .unwrap_or(LIST_BELOW_DEFAULT_HEIGHT);

    MainPreferencesDto {
        theme_id,
        dark_mode: prefs.dark_mode,
        reader_position: ReaderPositionDto::from_pref_str(&prefs.reader_position),
        sidebar_width,
        list_width,
        list_below_height,
        sidebar_collapsed: prefs.sidebar_collapsed,
        compact_list: prefs.compact_list,
        show_avatars: prefs.show_avatars,
        list_group_by_date: prefs.list_group_by_date,
        reader_zoom_percent: prefs
            .reader_zoom_percent
            .clamp(READER_ZOOM_MIN, READER_ZOOM_MAX),
    }
}

/// Validate and merge a whitelisted `MainPreferencesPatch` onto `prefs`.
pub fn apply_preferences_patch(
    prefs: &mut AppPreferences,
    patch: &MainPreferencesPatch,
) -> Result<(), IpcError> {
    if let Some(theme_id) = patch.theme_id.as_deref() {
        if !VALID_THEME_IDS.contains(&theme_id) {
            return Err(IpcError::invalid_input(
                format!("不支持的主题标识: {theme_id}"),
                None,
            ));
        }
    }
    if let Some(width) = patch.sidebar_width {
        if !width.is_finite() || !(SIDEBAR_MIN_WIDTH..=SIDEBAR_MAX_WIDTH).contains(&width) {
            return Err(IpcError::invalid_input(
                format!("sidebarWidth 超出允许范围 ({SIDEBAR_MIN_WIDTH}..={SIDEBAR_MAX_WIDTH})"),
                None,
            ));
        }
    }
    if let Some(width) = patch.list_width {
        if !width.is_finite() || !(LIST_MIN_WIDTH..=LIST_MAX_WIDTH).contains(&width) {
            return Err(IpcError::invalid_input(
                format!("listWidth 超出允许范围 (>={LIST_MIN_WIDTH})"),
                None,
            ));
        }
    }
    if let Some(height) = patch.list_below_height {
        if !height.is_finite() || !(LIST_BELOW_MIN_HEIGHT..=LIST_BELOW_MAX_HEIGHT).contains(&height)
        {
            return Err(IpcError::invalid_input(
                format!("listBelowHeight 超出允许范围 (>={LIST_BELOW_MIN_HEIGHT})"),
                None,
            ));
        }
    }
    if let Some(zoom) = patch.reader_zoom_percent {
        if !(READER_ZOOM_MIN..=READER_ZOOM_MAX).contains(&zoom) {
            return Err(IpcError::invalid_input(
                format!("readerZoomPercent 超出允许范围 ({READER_ZOOM_MIN}..={READER_ZOOM_MAX})"),
                None,
            ));
        }
    }

    if let Some(theme_id) = &patch.theme_id {
        prefs.theme_id = theme_id.clone();
    }
    if let Some(dark_mode) = patch.dark_mode {
        prefs.dark_mode = dark_mode;
    }
    if let Some(pos) = patch.reader_position {
        prefs.reader_position = pos.as_str().to_string();
    }
    if let Some(width) = patch.sidebar_width {
        prefs.sidebar_width = Some(width);
    }
    if let Some(width) = patch.list_width {
        prefs.list_width = Some(width);
    }
    if let Some(height) = patch.list_below_height {
        prefs.list_below_height = Some(height);
    }
    if let Some(collapsed) = patch.sidebar_collapsed {
        prefs.sidebar_collapsed = collapsed;
    }
    if let Some(compact) = patch.compact_list {
        prefs.compact_list = compact;
    }
    if let Some(show_avatars) = patch.show_avatars {
        prefs.show_avatars = show_avatars;
    }
    if let Some(group_by_date) = patch.list_group_by_date {
        prefs.list_group_by_date = group_by_date;
    }
    if let Some(zoom) = patch.reader_zoom_percent {
        prefs.reader_zoom_percent = zoom;
    }
    Ok(())
}

pub fn to_full_preferences_dto(
    prefs: &AppPreferences,
    app_lock_enabled: bool,
    app_lock_on_startup: bool,
) -> FullPreferencesDto {
    let main = to_main_preferences_dto(prefs);
    let read_mark_mode = if VALID_READ_MARK_MODES.contains(&prefs.read_mark_mode.as_str()) {
        prefs.read_mark_mode.clone()
    } else {
        "shown".to_string()
    };
    let default_view = if VALID_DEFAULT_VIEWS.contains(&prefs.default_view.as_str()) {
        prefs.default_view.clone()
    } else {
        "unified".to_string()
    };
    let ui_text_scale_percent = if UI_TEXT_SCALE_STEPS.contains(&prefs.ui_text_scale_percent) {
        prefs.ui_text_scale_percent
    } else {
        100
    };
    FullPreferencesDto {
        theme_id: main.theme_id,
        dark_mode: main.dark_mode,
        reader_position: main.reader_position,
        compact_list: main.compact_list,
        show_avatars: main.show_avatars,
        list_group_by_date: main.list_group_by_date,
        reader_zoom_percent: main.reader_zoom_percent,
        ui_text_scale_percent,
        auto_sync_minutes: prefs.auto_sync_minutes.min(AUTO_SYNC_MAX_MINUTES),
        push_enabled: prefs.push_enabled,
        desktop_notifications: prefs.desktop_notifications,
        notification_show_content: prefs.notification_show_content,
        close_to_minimize: prefs.close_to_minimize,
        minimize_to_tray: prefs.minimize_to_tray,
        check_updates: prefs.check_updates,
        block_remote_images: prefs.block_remote_images,
        show_remote_banner: prefs.show_remote_banner,
        clean_links: prefs.clean_links,
        read_mark_mode,
        thread_newest_first: prefs.thread_newest_first,
        undo_send_secs: prefs.undo_send_secs.min(UNDO_SEND_MAX_SECS),
        signature_above_quote: prefs.signature_above_quote,
        signature_dashes: prefs.signature_dashes,
        attachment_reminder_keywords: prefs.attachment_reminder_keywords.clone(),
        reply_header_template: prefs.reply_header_template.clone(),
        single_key_shortcuts: prefs.single_key_shortcuts,
        default_view,
        app_lock_enabled,
        app_lock_idle_minutes: normalize_app_lock_idle_minutes(prefs.app_lock_idle_minutes),
        app_lock_on_startup: app_lock_enabled && app_lock_on_startup,
    }
}

/// Validate and merge a whitelisted `FullPreferencesPatchDto` onto `prefs`.
pub fn apply_full_preferences_patch(
    prefs: &mut AppPreferences,
    patch: &FullPreferencesPatchDto,
) -> Result<(), IpcError> {
    if let Some(theme_id) = patch.theme_id.as_deref() {
        if !VALID_THEME_IDS.contains(&theme_id) {
            return Err(IpcError::invalid_input(
                format!("不支持的主题标识: {theme_id}"),
                None,
            ));
        }
    }
    if let Some(zoom) = patch.reader_zoom_percent {
        if !(READER_ZOOM_MIN..=READER_ZOOM_MAX).contains(&zoom) {
            return Err(IpcError::invalid_input(
                format!("readerZoomPercent 超出允许范围 ({READER_ZOOM_MIN}..={READER_ZOOM_MAX})"),
                None,
            ));
        }
    }
    if let Some(scale) = patch.ui_text_scale_percent {
        if !UI_TEXT_SCALE_STEPS.contains(&scale) {
            return Err(IpcError::invalid_input(
                "uiTextScalePercent 必须为 90/100/110/120/135/150 之一",
                None,
            ));
        }
    }
    if let Some(mins) = patch.auto_sync_minutes {
        if mins > AUTO_SYNC_MAX_MINUTES {
            return Err(IpcError::invalid_input(
                format!("autoSyncMinutes 超出允许范围 (0..={AUTO_SYNC_MAX_MINUTES})"),
                None,
            ));
        }
    }
    if let Some(mode) = patch.read_mark_mode.as_deref() {
        if !VALID_READ_MARK_MODES.contains(&mode) {
            return Err(IpcError::invalid_input(
                format!("不支持的已读标记策略: {mode}"),
                None,
            ));
        }
    }
    if let Some(secs) = patch.undo_send_secs {
        if secs > UNDO_SEND_MAX_SECS {
            return Err(IpcError::invalid_input(
                format!("undoSendSecs 超出允许范围 (0..={UNDO_SEND_MAX_SECS})"),
                None,
            ));
        }
    }
    if let Some(view) = patch.default_view.as_deref() {
        if !VALID_DEFAULT_VIEWS.contains(&view) {
            return Err(IpcError::invalid_input(
                format!("不支持的启动视图: {view}"),
                None,
            ));
        }
    }
    if let Some(idle) = patch.app_lock_idle_minutes {
        if !VALID_APP_LOCK_IDLE_MINUTES.contains(&idle) {
            return Err(IpcError::invalid_input(
                "appLockIdleMinutes 必须为 0/1/5/10/15/30/60 之一",
                None,
            ));
        }
    }
    if let Some(keywords) = patch.attachment_reminder_keywords.as_deref() {
        if keywords.contains('\0') || keywords.chars().count() > TEXT_PREF_MAX_CHARS {
            return Err(IpcError::invalid_input(
                "attachmentReminderKeywords 长度超限或包含非法字符",
                None,
            ));
        }
    }
    if let Some(tpl) = patch.reply_header_template.as_deref() {
        if tpl.contains('\0') || tpl.chars().count() > TEXT_PREF_MAX_CHARS {
            return Err(IpcError::invalid_input(
                "replyHeaderTemplate 长度超限或包含非法字符",
                None,
            ));
        }
    }

    if let Some(theme_id) = &patch.theme_id {
        prefs.theme_id = theme_id.clone();
    }
    if let Some(dark_mode) = patch.dark_mode {
        prefs.dark_mode = dark_mode;
    }
    if let Some(pos) = patch.reader_position {
        prefs.reader_position = pos.as_str().to_string();
    }
    if let Some(compact) = patch.compact_list {
        prefs.compact_list = compact;
    }
    if let Some(show_avatars) = patch.show_avatars {
        prefs.show_avatars = show_avatars;
    }
    if let Some(group_by_date) = patch.list_group_by_date {
        prefs.list_group_by_date = group_by_date;
    }
    if let Some(zoom) = patch.reader_zoom_percent {
        prefs.reader_zoom_percent = zoom;
    }
    if let Some(scale) = patch.ui_text_scale_percent {
        prefs.ui_text_scale_percent = scale;
    }
    if let Some(mins) = patch.auto_sync_minutes {
        prefs.auto_sync_minutes = mins;
    }
    if let Some(push) = patch.push_enabled {
        prefs.push_enabled = push;
    }
    if let Some(notif) = patch.desktop_notifications {
        prefs.desktop_notifications = notif;
    }
    if let Some(show_content) = patch.notification_show_content {
        prefs.notification_show_content = show_content;
    }
    if let Some(close_min) = patch.close_to_minimize {
        prefs.close_to_minimize = close_min;
    }
    if let Some(min_tray) = patch.minimize_to_tray {
        prefs.minimize_to_tray = min_tray;
    }
    if let Some(check_upd) = patch.check_updates {
        prefs.check_updates = check_upd;
    }
    if let Some(block_remote) = patch.block_remote_images {
        prefs.block_remote_images = block_remote;
    }
    if let Some(show_banner) = patch.show_remote_banner {
        prefs.show_remote_banner = show_banner;
    }
    if let Some(clean_links) = patch.clean_links {
        prefs.clean_links = clean_links;
    }
    if let Some(mode) = &patch.read_mark_mode {
        prefs.read_mark_mode = mode.clone();
    }
    if let Some(newest_first) = patch.thread_newest_first {
        prefs.thread_newest_first = newest_first;
    }
    if let Some(undo_secs) = patch.undo_send_secs {
        prefs.undo_send_secs = undo_secs;
    }
    if let Some(sig_above) = patch.signature_above_quote {
        prefs.signature_above_quote = sig_above;
    }
    if let Some(sig_dashes) = patch.signature_dashes {
        prefs.signature_dashes = sig_dashes;
    }
    if let Some(keywords) = &patch.attachment_reminder_keywords {
        prefs.attachment_reminder_keywords = keywords.clone();
    }
    if let Some(tpl) = &patch.reply_header_template {
        prefs.reply_header_template = tpl.clone();
    }
    if let Some(shortcuts) = patch.single_key_shortcuts {
        prefs.single_key_shortcuts = shortcuts;
    }
    if let Some(view) = &patch.default_view {
        prefs.default_view = view.clone();
    }
    if let Some(idle) = patch.app_lock_idle_minutes {
        prefs.app_lock_idle_minutes = idle;
    }
    Ok(())
}

/// Thread-safe preferences store serialized under a single mutex for atomic merges.
pub struct PreferencesStore {
    data_dir: PathBuf,
    prefs: Mutex<AppPreferences>,
}

impl PreferencesStore {
    pub fn open_in(data_dir: PathBuf) -> Self {
        let mut prefs = load_preferences_in(&data_dir).unwrap_or_default();
        normalize_loaded_preferences(&mut prefs);
        Self {
            data_dir,
            prefs: Mutex::new(prefs),
        }
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn update_full_patch(
        &self,
        patch: &FullPreferencesPatchDto,
    ) -> Result<AppPreferences, IpcError> {
        let mut guard = self.prefs.lock().expect("preferences mutex poisoned");
        let mut candidate = guard.clone();
        apply_full_preferences_patch(&mut candidate, patch)?;
        save_preferences_in(&self.data_dir, &candidate)
            .map_err(|e| IpcError::internal(format!("保存偏好设置失败: {e}"), None))?;
        *guard = candidate.clone();
        Ok(candidate)
    }

    pub fn snapshot_raw(&self) -> AppPreferences {
        self.prefs
            .lock()
            .expect("preferences mutex poisoned")
            .clone()
    }

    pub fn reload_from_disk(&self) -> AppPreferences {
        let mut prefs = load_preferences_in(&self.data_dir).unwrap_or_default();
        normalize_loaded_preferences(&mut prefs);
        let mut guard = self.prefs.lock().expect("preferences mutex poisoned");
        *guard = prefs.clone();
        prefs
    }

    pub fn snapshot_dto(&self) -> MainPreferencesDto {
        let guard = self.prefs.lock().expect("preferences mutex poisoned");
        to_main_preferences_dto(&guard)
    }

    pub fn update_patch(
        &self,
        patch: &MainPreferencesPatch,
    ) -> Result<MainPreferencesDto, IpcError> {
        let mut guard = self.prefs.lock().expect("preferences mutex poisoned");
        let mut candidate = guard.clone();
        apply_preferences_patch(&mut candidate, patch)?;
        save_preferences_in(&self.data_dir, &candidate)
            .map_err(|e| IpcError::internal(format!("保存偏好设置失败: {e}"), None))?;
        *guard = candidate;
        Ok(to_main_preferences_dto(&guard))
    }

    pub fn mutate_and_save<F>(&self, f: F) -> Result<bool, String>
    where
        F: FnOnce(&mut AppPreferences) -> bool,
    {
        let mut guard = self.prefs.lock().expect("preferences mutex poisoned");
        let mut candidate = guard.clone();
        if !f(&mut candidate) {
            return Ok(false);
        }
        save_preferences_in(&self.data_dir, &candidate)?;
        *guard = candidate;
        Ok(true)
    }

    pub fn update_last_location(&self, view: MainViewDto) {
        let (next_last_view, next_last_folder) = match view {
            MainViewDto::UnifiedInbox => (Some("unified".to_string()), None),
            MainViewDto::Unread => (Some("unread".to_string()), None),
            MainViewDto::Starred => (Some("starred".to_string()), None),
            MainViewDto::Sent => (Some("sent".to_string()), None),
            MainViewDto::Drafts => (Some("drafts".to_string()), None),
            MainViewDto::Archive => (Some("archive".to_string()), None),
            MainViewDto::Junk => (Some("junk".to_string()), None),
            MainViewDto::Trash => (Some("trash".to_string()), None),
            MainViewDto::Snoozed => (Some("snoozed".to_string()), None),
            MainViewDto::Filters | MainViewDto::Tags => (Some("unified".to_string()), None),
            MainViewDto::AccountFolder {
                account_id,
                folder_id,
            } => (None, Some((account_id, folder_id))),
        };
        let _ = self.mutate_and_save(|prefs| {
            if prefs.last_view == next_last_view && prefs.last_folder == next_last_folder {
                return false;
            }
            prefs.last_view = next_last_view;
            prefs.last_folder = next_last_folder;
            true
        });
    }
}

pub fn view_from_last_str(last: Option<&str>) -> Option<MainViewDto> {
    match last? {
        "unified" => Some(MainViewDto::UnifiedInbox),
        "unread" => Some(MainViewDto::Unread),
        "starred" => Some(MainViewDto::Starred),
        "sent" => Some(MainViewDto::Sent),
        "drafts" => Some(MainViewDto::Drafts),
        "archive" => Some(MainViewDto::Archive),
        "junk" => Some(MainViewDto::Junk),
        "trash" => Some(MainViewDto::Trash),
        "snoozed" => Some(MainViewDto::Snoozed),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kitemail-tauri-prefs-{label}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn preview_data_dir_is_isolated_from_production_kitemail_dir() {
        let local = PathBuf::from(r"C:\Users\tester\AppData\Local");
        let dir = resolve_preview_data_dir_with(None, None, false, None, Some(&local));
        assert_eq!(dir, local.join("KiteMailPreview"));
        assert_ne!(dir, local.join("KiteMail"));

        let env_dir = resolve_preview_data_dir_with(
            Some(r"D:\CustomPreview"),
            Some(r"D:\CustomMail"),
            false,
            None,
            Some(&local),
        );
        assert_eq!(env_dir, PathBuf::from(r"D:\CustomPreview"));

        let mail_env_dir =
            resolve_preview_data_dir_with(None, Some(r"D:\TestProfile"), false, None, Some(&local));
        assert_eq!(
            mail_env_dir,
            PathBuf::from(r"D:\TestProfile").join("preview")
        );
    }

    #[test]
    fn patch_validation_rejects_out_of_range_and_unknown_values() {
        let dir = temp_dir("validation");
        let store = PreferencesStore::open_in(dir.clone());

        let bad_theme = store.update_patch(&MainPreferencesPatch {
            theme_id: Some("neon".to_string()),
            ..Default::default()
        });
        assert_eq!(bad_theme.unwrap_err().code, "INVALID_INPUT");

        let bad_sidebar_low = store.update_patch(&MainPreferencesPatch {
            sidebar_width: Some(179.9),
            ..Default::default()
        });
        assert_eq!(bad_sidebar_low.unwrap_err().code, "INVALID_INPUT");

        let bad_sidebar_high = store.update_patch(&MainPreferencesPatch {
            sidebar_width: Some(281.0),
            ..Default::default()
        });
        assert_eq!(bad_sidebar_high.unwrap_err().code, "INVALID_INPUT");

        let bad_list_width = store.update_patch(&MainPreferencesPatch {
            list_width: Some(279.0),
            ..Default::default()
        });
        assert_eq!(bad_list_width.unwrap_err().code, "INVALID_INPUT");

        let bad_below_height = store.update_patch(&MainPreferencesPatch {
            list_below_height: Some(239.0),
            ..Default::default()
        });
        assert_eq!(bad_below_height.unwrap_err().code, "INVALID_INPUT");

        let bad_zoom = store.update_patch(&MainPreferencesPatch {
            reader_zoom_percent: Some(49),
            ..Default::default()
        });
        assert_eq!(bad_zoom.unwrap_err().code, "INVALID_INPUT");

        let bad_nan = store.update_patch(&MainPreferencesPatch {
            list_width: Some(f32::NAN),
            ..Default::default()
        });
        assert_eq!(bad_nan.unwrap_err().code, "INVALID_INPUT");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn valid_patch_merges_and_persists_serially() {
        let dir = temp_dir("persist");
        let store = PreferencesStore::open_in(dir.clone());

        let updated = store
            .update_patch(&MainPreferencesPatch {
                theme_id: Some("midnight".to_string()),
                dark_mode: Some(Some(true)),
                reader_position: Some(ReaderPositionDto::Below),
                sidebar_width: Some(220.0),
                list_width: Some(420.0),
                list_below_height: Some(300.0),
                reader_zoom_percent: Some(125),
                ..Default::default()
            })
            .expect("valid patch should persist");

        assert_eq!(updated.theme_id, "midnight");
        assert_eq!(updated.dark_mode, Some(true));
        assert_eq!(updated.reader_position, ReaderPositionDto::Below);
        assert_eq!(updated.sidebar_width, 220.0);
        assert_eq!(updated.list_width, 420.0);
        assert_eq!(updated.list_below_height, 300.0);
        assert_eq!(updated.reader_zoom_percent, 125);

        // Partial update keeps previously persisted fields and allows explicit darkMode = null.
        let follow_sys = store
            .update_patch(&MainPreferencesPatch {
                dark_mode: Some(None),
                ..Default::default()
            })
            .expect("clear dark mode");
        assert_eq!(follow_sys.dark_mode, None);
        assert_eq!(follow_sys.theme_id, "midnight");
        assert_eq!(follow_sys.sidebar_width, 220.0);

        let reopened = PreferencesStore::open_in(dir.clone()).snapshot_dto();
        assert_eq!(reopened, follow_sys);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn live_data_dir_prefers_env_falls_back_to_production_and_checks_isolation_helper() {
        let local = temp_dir("prod-root");
        let prod_dir = production_default_data_dir_with(false, None, Some(&local));
        assert_eq!(prod_dir, local.join("KiteMail"));

        let fallback = resolve_live_data_dir_with(None, None, false, None, Some(&local));
        assert_eq!(fallback, prod_dir);

        let portable =
            resolve_live_data_dir_with(None, None, true, Some(&local.join("app")), Some(&local));
        assert_eq!(portable, local.join("app").join("data"));

        let preview_override =
            resolve_live_data_dir_with(None, Some(r"D:\OtherPreview"), false, None, Some(&local));
        assert_eq!(preview_override, PathBuf::from(r"D:\OtherPreview"));
        assert!(
            verify_isolated_live_dirs(&preview_override, std::slice::from_ref(&prod_dir)).is_ok()
        );

        let env_custom = temp_dir("live-env");
        let resolved_env = resolve_live_data_dir_with(
            Some(env_custom.to_str().unwrap()),
            Some(r"D:\OtherPreview"),
            false,
            None,
            Some(&local),
        );
        assert_eq!(resolved_env, env_custom);
        assert!(verify_isolated_live_dirs(&resolved_env, std::slice::from_ref(&prod_dir)).is_ok());

        // Reject data_dir pointing directly to production default directory when explicit isolation helper is called.
        let direct_prod = verify_isolated_live_dirs(&prod_dir, std::slice::from_ref(&prod_dir));
        assert!(direct_prod.is_err());

        // Reject isolated data_dir whose storage.toml points mail_data_dir to production mail dir!
        let sneaky_dir = temp_dir("sneaky-storage");
        let prod_mail_dir = default_mail_data_dir_in(&prod_dir);
        kitemail_mail_core::config::save_storage_config_in(
            &sneaky_dir,
            &kitemail_mail_core::config::StorageConfig {
                mail_data_dir: Some(prod_mail_dir.to_string_lossy().into_owned()),
            },
        )
        .expect("write storage.toml");
        let sneaky_check = verify_isolated_live_dirs(&sneaky_dir, std::slice::from_ref(&prod_dir));
        assert!(sneaky_check.is_err());

        let _ = std::fs::remove_dir_all(&local);
        let _ = std::fs::remove_dir_all(&env_custom);
        let _ = std::fs::remove_dir_all(&sneaky_dir);
    }
}
