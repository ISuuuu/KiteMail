//! Main window geometry restoration/persistence and logical-to-physical DPI coordinate math.

use kitemail_mail_core::config::AppPreferences;

use crate::dto::RectDto;
use crate::preferences::PreferencesStore;

pub const DEFAULT_WINDOW_WIDTH: f32 = 1280.0;
pub const DEFAULT_WINDOW_HEIGHT: f32 = 800.0;
pub const MIN_WINDOW_WIDTH: f32 = 900.0;
pub const MIN_WINDOW_HEIGHT: f32 = 600.0;
pub const MAX_WINDOW_WIDTH: f32 = 7680.0;
pub const MAX_WINDOW_HEIGHT: f32 = 4320.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PhysicalRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl PhysicalRect {
    pub fn width(self) -> i32 {
        self.right - self.left
    }

    pub fn height(self) -> i32 {
        self.bottom - self.top
    }
}

/// Resolve startup window logical size from persisted preferences, clamping outliers.
pub fn resolve_window_size(prefs: &AppPreferences) -> (f32, f32) {
    match (prefs.window_width, prefs.window_height) {
        (Some(w), Some(h)) if w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0 => (
            w.clamp(MIN_WINDOW_WIDTH, MAX_WINDOW_WIDTH),
            h.clamp(MIN_WINDOW_HEIGHT, MAX_WINDOW_HEIGHT),
        ),
        _ => (DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT),
    }
}

/// Capture window geometry without letting maximized or minimized bounds overwrite normal dimensions.
pub fn capture_window_geometry(
    prefs: &mut AppPreferences,
    logical_width: f32,
    logical_height: f32,
    maximized: bool,
    minimized: bool,
) -> bool {
    if minimized {
        return false;
    }
    let mut changed = false;
    if prefs.window_maximized != maximized {
        prefs.window_maximized = maximized;
        changed = true;
    }
    // Never overwrite the restored normal window dimensions while maximized.
    if !maximized
        && logical_width.is_finite()
        && logical_height.is_finite()
        && logical_width >= 320.0
        && logical_height >= 240.0
    {
        let w = logical_width.clamp(MIN_WINDOW_WIDTH, MAX_WINDOW_WIDTH);
        let h = logical_height.clamp(MIN_WINDOW_HEIGHT, MAX_WINDOW_HEIGHT);
        if prefs.window_width != Some(w) || prefs.window_height != Some(h) {
            prefs.window_width = Some(w);
            prefs.window_height = Some(h);
            changed = true;
        }
    }
    changed
}

/// Normalize an effective DPI scale factor (`window.scale_factor()` or frontend `devicePixelRatio`).
pub fn resolve_effective_dpr(frontend_dpr: Option<f64>, window_dpr: f64) -> f64 {
    if let Some(dpr) = frontend_dpr {
        if dpr.is_finite() && (0.5..=8.0).contains(&dpr) {
            return dpr;
        }
    }
    if window_dpr.is_finite() && (0.5..=8.0).contains(&window_dpr) {
        window_dpr
    } else {
        1.0
    }
}

/// Convert a CSS logical-pixel rectangle into client-relative physical pixels.
pub fn logical_rect_to_physical(rect: RectDto, dpr: f64) -> PhysicalRect {
    let scale = resolve_effective_dpr(Some(dpr), 1.0);
    let left = (rect.x * scale).round() as i32;
    let top = (rect.y * scale).round() as i32;
    let right = ((rect.x + rect.width.max(0.0)) * scale).round() as i32;
    let bottom = ((rect.y + rect.height.max(0.0)) * scale).round() as i32;
    PhysicalRect {
        left,
        top,
        right: right.max(left),
        bottom: bottom.max(top),
    }
}

/// Compute `(clip_window_rect, controller_bounds_relative_to_clip)` matching `webview_host::clipped_rects`.
pub fn compute_clipped_rects(
    content: PhysicalRect,
    clip_viewport: PhysicalRect,
) -> Option<(PhysicalRect, PhysicalRect)> {
    let clip = PhysicalRect {
        left: content.left.max(clip_viewport.left),
        top: content.top.max(clip_viewport.top),
        right: content.right.min(clip_viewport.right),
        bottom: content.bottom.min(clip_viewport.bottom),
    };
    if clip.right <= clip.left
        || clip.bottom <= clip.top
        || content.right <= content.left
        || content.bottom <= content.top
    {
        return None;
    }
    let bounds = PhysicalRect {
        left: content.left - clip.left,
        top: content.top - clip.top,
        right: content.right - clip.left,
        bottom: content.bottom - clip.top,
    };
    Some((clip, bounds))
}

/// Restore initial main window bounds, maximized flag, and fullscreen flag.
pub fn restore_main_window_geometry(window: &tauri::WebviewWindow, prefs: &AppPreferences) {
    let (width, height) = resolve_window_size(prefs);
    let _ = window.set_size(tauri::LogicalSize::new(f64::from(width), f64::from(height)));
    if prefs.window_maximized {
        let _ = window.maximize();
    }
    if prefs.start_fullscreen {
        let _ = window.set_fullscreen(true);
    }
}

/// Capture and persist current main window geometry from Tauri window state.
pub fn save_main_window_geometry(window: &tauri::WebviewWindow, prefs_store: &PreferencesStore) {
    let minimized = window.is_minimized().unwrap_or(false);
    if minimized {
        return;
    }
    let maximized = window.is_maximized().unwrap_or(false);
    let scale = window.scale_factor().unwrap_or(1.0);
    let (logical_w, logical_h) = match window.inner_size() {
        Ok(size) => {
            let logical = size.to_logical::<f32>(scale);
            (logical.width, logical.height)
        }
        Err(_) => return,
    };
    let _ = prefs_store.mutate_and_save(|prefs| {
        capture_window_geometry(prefs, logical_w, logical_h, maximized, false)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_window_size_uses_defaults_and_clamps() {
        let defaults = AppPreferences::default();
        assert_eq!(
            resolve_window_size(&defaults),
            (DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT)
        );

        let tiny = AppPreferences {
            window_width: Some(300.0),
            window_height: Some(200.0),
            ..Default::default()
        };
        assert_eq!(
            resolve_window_size(&tiny),
            (MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT)
        );

        let custom = AppPreferences {
            window_width: Some(1440.0),
            window_height: Some(900.0),
            ..Default::default()
        };
        assert_eq!(resolve_window_size(&custom), (1440.0, 900.0));

        let nan = AppPreferences {
            window_width: Some(f32::NAN),
            window_height: Some(800.0),
            ..Default::default()
        };
        assert_eq!(
            resolve_window_size(&nan),
            (DEFAULT_WINDOW_WIDTH, DEFAULT_WINDOW_HEIGHT)
        );
    }

    #[test]
    fn capture_geometry_preserves_normal_size_across_maximize_and_minimize() {
        let mut prefs = AppPreferences::default();

        assert!(capture_window_geometry(
            &mut prefs, 1360.0, 840.0, false, false
        ));
        assert_eq!(prefs.window_width, Some(1360.0));
        assert_eq!(prefs.window_height, Some(840.0));
        assert!(!prefs.window_maximized);

        // Transitioning to maximized updates flag but keeps 1360x840 restore dimensions.
        assert!(capture_window_geometry(
            &mut prefs, 1920.0, 1080.0, true, false
        ));
        assert!(prefs.window_maximized);
        assert_eq!(prefs.window_width, Some(1360.0));
        assert_eq!(prefs.window_height, Some(840.0));

        // Minimizing must be ignored completely.
        assert!(!capture_window_geometry(&mut prefs, 0.0, 0.0, false, true));
        assert!(prefs.window_maximized);
        assert_eq!(prefs.window_width, Some(1360.0));
        assert_eq!(prefs.window_height, Some(840.0));
    }

    #[test]
    fn logical_to_physical_dpi_and_clipping_match_expected_pixels() {
        let rect = RectDto {
            x: 600.0,
            y: 48.0,
            width: 680.0,
            height: 752.0,
        };
        let phys_100 = logical_rect_to_physical(rect, 1.0);
        assert_eq!(
            phys_100,
            PhysicalRect {
                left: 600,
                top: 48,
                right: 1280,
                bottom: 800
            }
        );

        let phys_125 = logical_rect_to_physical(rect, 1.25);
        assert_eq!(
            phys_125,
            PhysicalRect {
                left: 750,
                top: 60,
                right: 1600,
                bottom: 1000
            }
        );

        let phys_150 = logical_rect_to_physical(rect, 1.5);
        assert_eq!(
            phys_150,
            PhysicalRect {
                left: 900,
                top: 72,
                right: 1920,
                bottom: 1200
            }
        );

        let clip_vp = PhysicalRect {
            left: 950,
            top: 72,
            right: 1920,
            bottom: 1200,
        };
        let (clip, bounds) =
            compute_clipped_rects(phys_150, clip_vp).expect("non-empty intersection");
        assert_eq!(clip, clip_vp);
        assert_eq!(
            bounds,
            PhysicalRect {
                left: -50,
                top: 0,
                right: 970,
                bottom: 1128
            }
        );

        // Disjoint rects produce None so native host hides the child HWND.
        let disjoint = PhysicalRect {
            left: 0,
            top: 0,
            right: 100,
            bottom: 100,
        };
        assert!(compute_clipped_rects(phys_150, disjoint).is_none());
    }
}
