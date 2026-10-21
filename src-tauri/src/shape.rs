use tauri::WebviewWindow;

use crate::winapi;

/// CSS shell radius (16 px) plus one pixel, so the system clip never eats the
/// anti-aliased CSS edge but still removes any stray native border pixels.
const CORNER_RADIUS: f64 = 17.0;

/// Enforces the rounded window outline at the OS level.
///
/// * Windows 11 — DWM rounds the window natively and anti-aliased.
/// * Windows 10 — a rounded window region clips the rectangle, otherwise the window
///   background behind the CSS corners shows up as square dark blocks.
///
/// With no blur effect involved there is nothing opaque covering the corners, so the
/// CSS `border-radius` supplies the smooth edge and the region only trims the rest.
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
