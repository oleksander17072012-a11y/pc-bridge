use tauri::WebviewWindow;

use crate::winapi;

/// The window outline.
///
/// The window is frameless and fixed-size on purpose: without `WS_THICKFRAME` Windows
/// stops drawing its own 1 px outline and resize shadow around the window, which used
/// to show up as light "walls" along the edges. With no DWM effect and a transparent
/// webview, the rounded corners are drawn by CSS and are properly anti-aliased.
///
/// Windows 11 additionally gets DWM rounding so the corners match the system style.
pub fn apply(window: &WebviewWindow) {
    let Ok(handle) = window.hwnd() else { return };
    let hwnd = handle.0 as isize;
    if hwnd == 0 {
        return;
    }

    if winapi::windows_build() >= 22000 {
        winapi::prefer_round_corners(hwnd);
    }
    // No window region: it is rectangular by nature and produced hard, unrounded
    // corner pixels, so CSS owns the shape.
}
