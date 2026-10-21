use tauri::WebviewWindow;

use crate::winapi;

const CORNER_RADIUS: f64 = 16.0;

/// Keeps the visible window outline in sync with the CSS shell.
///
/// * Windows 11 — native DWM rounding, anti-aliased, nothing to clip.
/// * Windows 10 with blur — a rounded window region, otherwise the blur paints over
///   the corners and they look square.
/// * Windows 10 without blur — no region at all, so the transparent webview shows
///   perfect anti-aliased CSS corners.
pub fn apply(window: &WebviewWindow, blur: bool) {
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

    let maximized = window.is_maximized().unwrap_or(false);
    if maximized || !blur {
        winapi::clear_region(hwnd);
        return;
    }

    let Ok(size) = window.inner_size() else { return };
    let scale = window.scale_factor().unwrap_or(1.0);
    let radius = (CORNER_RADIUS * scale).round().max(1.0) as i32;
    winapi::set_round_region(hwnd, size.width as i32, size.height as i32, radius);
}
