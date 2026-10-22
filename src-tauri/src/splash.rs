//! Startup window: a small rounded card that reports progress, then grows into the
//! full launcher.
//!
//! The window is created at `SPLASH_WIDTH`x`SPLASH_HEIGHT` and stays that way while the
//! relay, the tunnel and the key are being prepared. The renderer calls `expand_window`
//! once it has something to show, and the window animates to the full size around its
//! own centre. Animating keeps the process responsive: every frame is a plain
//! `set_size`/`set_position`, no blocking calls on the event loop.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

use crate::shape;
use crate::winapi;

/// Logical size of the launcher once startup is done (matches the stylesheet).
pub const FULL_WIDTH: f64 = 940.0;
pub const FULL_HEIGHT: f64 = 760.0;

const FRAMES: u32 = 26;
const FRAME_MS: u64 = 15;

static EXPANDED: AtomicBool = AtomicBool::new(false);

fn ease_out(progress: f64) -> f64 {
    let inverse = 1.0 - progress;
    1.0 - inverse * inverse * inverse
}

fn mix(from: i32, to: i32, progress: f64) -> i32 {
    (from as f64 + (to - from) as f64 * progress).round() as i32
}

/// Grows the window from its current geometry to the launcher size, centred on screen.
pub fn expand(window: &WebviewWindow) -> Result<(), String> {
    if EXPANDED.swap(true, Ordering::SeqCst) {
        return Ok(());
    }

    let scale = window.scale_factor().unwrap_or(1.0);
    let (screen_width, screen_height) = winapi::screen_size();

    let target_width = ((FULL_WIDTH * scale).round() as i32).min(screen_width).max(320);
    let target_height = ((FULL_HEIGHT * scale).round() as i32).min(screen_height).max(240);
    let target_x = ((screen_width - target_width) / 2).max(0);
    let target_y = ((screen_height - target_height) / 2).max(0);

    let from_position = window.outer_position().unwrap_or(PhysicalPosition::new(target_x, target_y));
    let from_size = window
        .outer_size()
        .unwrap_or(PhysicalSize::new(target_width as u32, target_height as u32));

    // A rounded region is cut for the old size; it would clip the window while it grows.
    let unclipped = window.clone();
    let _ = window.run_on_main_thread(move || shape::clear(&unclipped));

    for frame in 1..=FRAMES {
        let progress = ease_out(frame as f64 / FRAMES as f64);
        let width = mix(from_size.width as i32, target_width, progress).max(160) as u32;
        let height = mix(from_size.height as i32, target_height, progress).max(120) as u32;
        let x = mix(from_position.x, target_x, progress);
        let y = mix(from_position.y, target_y, progress);
        let _ = window.set_size(PhysicalSize::new(width, height));
        let _ = window.set_position(PhysicalPosition::new(x, y));
        std::thread::sleep(std::time::Duration::from_millis(FRAME_MS));
    }

    let _ = window.set_size(PhysicalSize::new(target_width as u32, target_height as u32));
    let _ = window.set_position(PhysicalPosition::new(target_x, target_y));

    // Round the new outline, then let the renderer fade the splash away.
    let rounded = window.clone();
    let _ = window.run_on_main_thread(move || shape::apply(&rounded));
    let _ = window.set_focus();
    Ok(())
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
