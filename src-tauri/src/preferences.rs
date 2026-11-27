//! Isolated preview profile directory and whitelisted preference persistence.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use kitemail_mail_core::config::{load_preferences_in, save_preferences_in, AppPreferences};

use crate::dto::{IpcError, MainPreferencesDto, MainPreferencesPatch, ReaderPositionDto};

pub const VALID_THEME_IDS: &[&str] = &[
    "system", "default", "midnight", "tidal", "rose", "earth", "forest",
];

pub const SIDEBAR_MIN_WIDTH: f32 = 180.0;
pub const SIDEBAR_DEFAULT_WIDTH: f32 = 240.0;
pub const SIDEBAR_MAX_WIDTH: f32 = 280.0;

pub const LIST_MIN_WIDTH: f32 = 280.0;
pub const LIST_DEFAULT_WIDTH: f32 = 360.0;
pub const LIST_MAX_WIDTH: f32 = 7680.0;

pub const LIST_BELOW_MIN_HEIGHT: f32 = 240.0;
pub const LIST_BELOW_DEFAULT_HEIGHT: f32 = 360.0;
pub const LIST_BELOW_MAX_HEIGHT: f32 = 4320.0;

pub const READER_ZOOM_MIN: u32 = 50;
pub const READER_ZOOM_MAX: u32 = 250;

const PREVIEW_DATA_DIR_NAME: &str = "KiteMailPreview";

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

    #[allow(dead_code)]
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn snapshot_raw(&self) -> AppPreferences {
        self.prefs
            .lock()
            .expect("preferences mutex poisoned")
            .clone()
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
}
