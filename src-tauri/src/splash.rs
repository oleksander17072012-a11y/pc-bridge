//! Startup window: a small rounded card that reports progress, then grows into the
//! full launcher.
//!
//! The window is created at the splash size and stays that way while the relay, the
//! tunnel and the key are being prepared. The renderer calls `expand_window` once it has
//! something to show, and the window animates to the launcher size around its own
//! monitor centre.
//!
//! Smoothness rules learned the hard way:
//!   * every frame is ONE `SetWindowPos` (move + resize together) — separate messages
//!     repaint in between and the picture stutters;
//!   * the rounded region is recomputed on every frame, so the corners stay round while
//!     the window grows instead of flickering back to square;
//!   * frames are paced by rendezvous: the next frame is only queued once the previous
//!     one has been applied, so the queue never floods and the animation stays in sync
//!     with the compositor. The result is "as many frames as the screen can show"
//!     inside a short, fixed time budget.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tauri::{Manager, WebviewWindow};

use crate::shape;
use crate::winapi;

/// Logical size of the launcher once startup is done (matches the stylesheet).
pub const FULL_WIDTH: f64 = 940.0;
pub const FULL_HEIGHT: f64 = 760.0;

/// How long the growth takes. Short on purpose: a long animation on a 3440px screen
/// spends most of its time in WebView2 re-layout and reads as slow, not smooth.
const DURATION: Duration = Duration::from_millis(340);
/// Never wait longer than this for a frame to be applied.
const FRAME_TIMEOUT: Duration = Duration::from_millis(120);

static EXPANDED: AtomicBool = AtomicBool::new(false);

fn ease_out(progress: f64) -> f64 {
    let inverse = 1.0 - progress;
    1.0 - inverse * inverse * inverse
}

fn mix(from: i32, to: i32, progress: f64) -> i32 {
    (from as f64 + (to - from) as f64 * progress).round() as i32
}

/// The monitor the window currently sits on, as (x, y, width, height) in physical pixels.
fn monitor_bounds(window: &WebviewWindow) -> (i32, i32, i32, i32) {
    if let Ok(Some(monitor)) = window.current_monitor() {
        let origin = monitor.position();
        let size = monitor.size();
        if size.width > 0 && size.height > 0 {
            return (origin.x, origin.y, size.width as i32, size.height as i32);
        }
    }
    let (width, height) = winapi::screen_size();
    (0, 0, width, height)
}

/// Grows the window from its current geometry to the launcher size, centred on the
/// monitor it lives on, and returns when the last frame has been applied.
pub fn expand(window: &WebviewWindow) -> Result<(), String> {
    if EXPANDED.swap(true, Ordering::SeqCst) {
        return Ok(());
    }

    // The caller has a loaded page, so the window is ready to be seen.
    let _ = window.show();

    let scale = window.scale_factor().unwrap_or(1.0);
    let (origin_x, origin_y, screen_width, screen_height) = monitor_bounds(window);

    let target_width = ((FULL_WIDTH * scale).round() as i32).clamp(320, screen_width);
    let target_height = ((FULL_HEIGHT * scale).round() as i32).clamp(240, screen_height);
    let target_x = origin_x + (screen_width - target_width) / 2;
    let target_y = origin_y + (screen_height - target_height) / 2;

    let start = window.outer_position().unwrap_or(tauri::PhysicalPosition::new(target_x, target_y));
    let from = window.outer_size().unwrap_or(tauri::PhysicalSize::new(
        target_width as u32,
        target_height as u32,
    ));

    let Ok(handle) = window.hwnd() else {
        // Not on Windows (or the window is gone): jump straight to the final size.
        let _ = window.set_size(tauri::PhysicalSize::new(target_width as u32, target_height as u32));
        let _ = window.set_position(tauri::PhysicalPosition::new(target_x, target_y));
        return Ok(());
    };
    let hwnd = handle.0 as isize;
    let radius = (shape::CORNER_RADIUS * scale).round().max(1.0) as i32;

    // Round the starting geometry first: the first animation frame must not be square.
    let pre_rounded = window.clone();
    let pre = (from.width as i32, from.height as i32);
    let _ = window.run_on_main_thread(move || {
        if let Ok(handle) = pre_rounded.hwnd() {
            let hwnd = handle.0 as isize;
            winapi::set_rect_rounded(hwnd, start.x, start.y, pre.0, pre.1, radius);
        }
    });

    let started = Instant::now();
    loop {
        let elapsed = started.elapsed().as_secs_f64();
        let progress = (elapsed / DURATION.as_secs_f64()).min(1.0);
        let eased = ease_out(progress);

        let width = mix(from.width as i32, target_width, eased).max(160);
        let height = mix(from.height as i32, target_height, eased).max(120);
        let x = mix(start.x, target_x, eased);
        let y = mix(start.y, target_y, eased);

        // Rendezvous: apply this frame on the main thread, then continue. Without the
        // ack the frames pile up in the queue and the window jumps in bursts.
        let (done, applied) = std::sync::mpsc::channel();
        let target = window.clone();
        let queued = window.run_on_main_thread(move || {
            winapi::set_rect_rounded(hwnd, x, y, width, height, radius);
            // Keeps the geometry Tauri reports in sync with the native window.
            let _ = target.set_size(tauri::PhysicalSize::new(width as u32, height as u32));
            let _ = done.send(());
        });
        if queued.is_err() {
            break;
        }
        let _ = applied.recv_timeout(FRAME_TIMEOUT);

        if progress >= 1.0 {
            break;
        }
    }

    let _ = window.set_size(tauri::PhysicalSize::new(target_width as u32, target_height as u32));
    let _ = window.set_position(tauri::PhysicalPosition::new(target_x, target_y));

    // Round the final outline (Windows 11 rounds natively; Windows 10 needs the region).
    let rounded = window.clone();
    let _ = window.run_on_main_thread(move || shape::apply(&rounded));
    let _ = window.set_focus();
    Ok(())
}

/// Rounds the outline right now — used for the small startup window, so it never shows
/// square corners while the webview is still painting.
pub fn round_now(window: &WebviewWindow) {
    let rounded = window.clone();
    let _ = window.run_on_main_thread(move || shape::apply(&rounded));
}

/// True once the launcher has reached its full size.
pub fn is_expanded() -> bool {
    EXPANDED.load(Ordering::SeqCst)
}

/// Shows the window again without growing it (used by the tray menu).
pub fn restore(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
