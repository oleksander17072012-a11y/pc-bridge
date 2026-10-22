//! Side-by-side chat panel.
//!
//! Arena sits behind a Cloudflare challenge, and embedded webviews are frequently
//! rejected by it (the page then stays blank). So the chat can be opened two ways:
//!   * a real browser window (Edge/Chrome) tiled next to the launcher — the reliable
//!     path, the challenge passes and the account session persists in the browser;
//!   * an in-app WebView2 window with its own persistent profile, for those cases when
//!     the challenge does render inside the app.
//!
//! In both cases the launcher is stretched to the left half of the screen first and
//! restored to its previous geometry when the chat window is closed.
//!
//! The remote origin gets no Tauri IPC access: it is not referenced by any capability.

use std::process::Command;
use std::sync::{Mutex, OnceLock};

use tauri::{Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::shell::CREATE_NO_WINDOW;
use crate::winapi;

pub const CHAT_LABEL: &str = "chat";
const CHAT_URL: &str = "https://arena.ai/";
const LAUNCHER_HEIGHT: i32 = 800;
const LAUNCHER_WIDTH: i32 = 1000;

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

fn remember(launcher: &WebviewWindow) {
    let position = launcher.outer_position().unwrap_or(PhysicalPosition::new(0, 0));
    let size = launcher
        .outer_size()
        .unwrap_or(PhysicalSize::new(LAUNCHER_WIDTH as u32, LAUNCHER_HEIGHT as u32));
    *saved().lock().unwrap_or_else(|error| error.into_inner()) = Some(Geometry {
        x: position.x,
        y: position.y,
        width: size.width as i32,
        height: size.height as i32,
    });
}

/// Screen geometry in **physical** pixels, which is what `set_position`/`set_size` use.
fn layout() -> (i32, i32, i32, i32, i32, i32) {
    let (screen_width, screen_height) = winapi::screen_size();
    let half = (screen_width / 2).max(480);
    let height = LAUNCHER_HEIGHT.min(screen_height);
    let top = ((screen_height - height) / 2).max(0);
    let right_width = (screen_width - half).max(480);
    (half, height, top, half, right_width, top)
}

fn stretch_launcher(launcher: &WebviewWindow) -> (i32, i32, i32, i32) {
    let (left_width, height, top, right_x, right_width, _) = layout();
    let _ = launcher.set_position(PhysicalPosition::new(0, top));
    let _ = launcher.set_size(PhysicalSize::new(left_width as u32, height as u32));
    // The rounded region is not scaled by the system, so re-clip for the new size.
    let clone = launcher.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(180));
        let inner = clone.clone();
        let _ = clone.run_on_main_thread(move || crate::shape::apply(&inner));
    });
    (right_x, right_width, height, top)
}

/// Opens the chat in the default browser, tiled to the right half.
pub fn open_in_browser(app: &tauri::AppHandle, launcher: &WebviewWindow) -> Result<(), String> {
    remember(launcher);
    let (x, width, height, top) = stretch_launcher(launcher);
    let geometry = format!("--window-position={x},{top} --window-size={width},{height}");

    // Prefer Edge, then Chrome; fall back to the shell handler.
    let candidates = [
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe",
    ];
    for candidate in candidates {
        if !std::path::Path::new(candidate).exists() {
            continue;
        }
        let mut command = Command::new(candidate);
        command.arg("--new-window").args(geometry.split(' ')).arg(CHAT_URL);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        if command.spawn().is_ok() {
            return Ok(());
        }
    }

    let mut fallback = Command::new("cmd");
    fallback.args(["/c", "start", "", CHAT_URL]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        fallback.creation_flags(CREATE_NO_WINDOW);
    }
    fallback.spawn().map(|_| ()).map_err(|error| error.to_string())
}

/// Opens the chat as an in-app window with its own persistent WebView2 profile.
pub fn open(app: &tauri::AppHandle, launcher: &WebviewWindow) -> Result<(), String> {
    if let Some(existing) = app.get_webview_window(CHAT_LABEL) {
        let _ = existing.show();
        let _ = existing.unminimize();
        let _ = existing.set_focus();
        return Ok(());
    }

    remember(launcher);
    let (x, width, height, top) = stretch_launcher(launcher);

    let url = CHAT_URL.parse().map_err(|error| format!("некорректный адрес чата: {error}"))?;
    let chat = WebviewWindowBuilder::new(app, CHAT_LABEL, WebviewUrl::External(url))
        .title("Arena.ai — Agent Mode")
        .inner_size(900.0, 800.0)
        .min_inner_size(420.0, 500.0)
        .build()
        .map_err(|error| format!("не удалось открыть окно чата: {error}"))?;

    // Builder sizes are logical, so set the physical geometry explicitly afterwards.
    let _ = chat.set_position(PhysicalPosition::new(x, top));
    let _ = chat.set_size(PhysicalSize::new(width as u32, height as u32));

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
