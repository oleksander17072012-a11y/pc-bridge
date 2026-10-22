//! Side-by-side chat panel.
//!
//! The chat is a real top-level WebView2 window, tiled next to the launcher, not an
//! iframe and not a child webview:
//!   * OAuth logins (Google / Microsoft) refuse to work inside embedded frames, but a
//!     normal window with a normal user agent is fine;
//!   * WebView2 keeps its own profile in the app data folder, so the session survives
//!     restarts — you log in once.
//!
//! The remote origin gets **no** Tauri IPC access: it is not listed in any capability,
//! so the page can only talk to the internet, never to the bridge commands.

use std::sync::Mutex;
use std::sync::OnceLock;

use tauri::{Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::winapi;

pub const CHAT_LABEL: &str = "chat";
const CHAT_URL: &str = "https://arena.ai/";
const LAUNCHER_WIDTH: i32 = 1000;
const LAUNCHER_HEIGHT: i32 = 800;

/// Geometry of the launcher before it was stretched, so closing the chat can restore it.
struct Geometry {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

fn saved() -> &'static Mutex<Option<Geometry>> {
    static SAVED: OnceLock<Mutex<Option<Geometry>>> = OnceLock::new();
    SAVED.get_or_init(|| Mutex::new(None))
}

pub fn is_open(app: &tauri::AppHandle) -> bool {
    app.get_webview_window(CHAT_LABEL).is_some()
}

/// Stretches the launcher to the left half of the screen and opens the chat on the right.
pub fn open(app: &tauri::AppHandle, launcher: &WebviewWindow) -> Result<(), String> {
    if let Some(existing) = app.get_webview_window(CHAT_LABEL) {
        let _ = existing.show();
        let _ = existing.unminimize();
        let _ = existing.set_focus();
        return Ok(());
    }

    let (screen_width, screen_height) = winapi::screen_size();
    if screen_width <= 0 || screen_height <= 0 {
        return Err("не удалось определить размер экрана".into());
    }

    // Remember where the launcher was, so it can return there later.
    let position = launcher.outer_position().unwrap_or(PhysicalPosition::new(0, 0));
    let size = launcher.outer_size().unwrap_or(PhysicalSize::new(LAUNCHER_WIDTH as u32, LAUNCHER_HEIGHT as u32));
    *saved().lock().unwrap_or_else(|error| error.into_inner()) = Some(Geometry {
        x: position.x,
        y: position.y,
        width: size.width as i32,
        height: size.height as i32,
    });

    let half = screen_width / 2;
    let top = (screen_height - LAUNCHER_HEIGHT).max(0) / 2;
    let height = LAUNCHER_HEIGHT.min(screen_height - top);

    let _ = launcher.set_position(PhysicalPosition::new(0, top));
    let _ = launcher.set_size(PhysicalSize::new(half as u32, height as u32));

    let url = CHAT_URL.parse().map_err(|error| format!("некорректный адрес чата: {error}"))?;
    let chat = WebviewWindowBuilder::new(app, CHAT_LABEL, WebviewUrl::External(url))
        .title("Arena.ai — Agent Mode")
        .inner_size(half as f64, height as f64)
        .position(half as f64, top as f64)
        .min_inner_size(420.0, 500.0)
        .build()
        .map_err(|error| format!("не удалось открыть окно чата: {error}"))?;

    // Closing the chat restores the launcher to its original size and position.
    let app_handle = app.clone();
    chat.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::Destroyed) {
            if let Some(launcher) = app_handle.get_webview_window("main") {
                let mut slot = saved().lock().unwrap_or_else(|error| error.into_inner());
                if let Some(geometry) = slot.take() {
                    let _ = launcher.set_size(PhysicalSize::new(geometry.width as u32, geometry.height as u32));
                    let _ = launcher.set_position(PhysicalPosition::new(geometry.x, geometry.y));
                }
            }
        }
    });

    Ok(())
}

pub fn close(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window(CHAT_LABEL) {
        let _ = window.close();
    }
}
