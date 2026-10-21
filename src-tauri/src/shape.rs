use tauri::WebviewWindow;

use crate::winapi;

/// Radius in CSS pixels; the stylesheet uses the same value for the shell surface.
pub const CORNER_RADIUS: f64 = 18.0;

/// Rounds the window outline.
///
/// * Windows 11 — native DWM rounding, anti-aliased, no clipping needed.
/// * Windows 10 — a rounded window region: the only reliable way there, because the
///   WebView2 surface paints an opaque rectangle and DWM has no corner preference.
///
/// The region is set one pixel wider than the CSS radius so the anti-aliased CSS edge
/// stays inside the clip instead of being cut off.
pub fn apply(window: &WebviewWindow) {
    let Ok(handle) = window.hwnd() else { return };
    let hwnd = handle.0 as isize;
    if hwnd == 0 {
        return;
    }

    if winapi::windows_build() >= 22000 {
        winapi::prefer_round_corners(hwnd);
        winapi::clear_region(hwnd);
        return;
    }

    if window.is_maximized().unwrap_or(false) {
        winapi::clear_region(hwnd);
        return;
    }

    let Ok(size) = window.inner_size() else { return };
    let scale = window.scale_factor().unwrap_or(1.0);
    let radius = (CORNER_RADIUS * scale).round().max(1.0) as i32;
    winapi::set_round_region(hwnd, size.width as i32, size.height as i32, radius);
}
