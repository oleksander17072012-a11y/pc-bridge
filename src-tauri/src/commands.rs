use std::sync::Arc;

use serde_json::Value;
use tauri::State;

use crate::b64;
use crate::journal::Entry;
use crate::paths;
use crate::prompt;
use crate::state::AppState;
use crate::winapi::{self, WindowInfo};

type Shared<'a> = State<'a, Arc<AppState>>;

#[tauri::command]
pub fn snapshot(state: Shared<'_>) -> Value {
    state.snapshot()
}

#[tauri::command]
pub fn journal_tail(state: Shared<'_>, limit: Option<usize>) -> Vec<Entry> {
    state.journal.recent(limit.unwrap_or(300))
}

#[tauri::command]
pub fn set_active(state: Shared<'_>, active: bool) {
    state.set_active(active);
}

#[tauri::command]
pub fn rotate_key(state: Shared<'_>) -> String {
    state.rotate_key();
    state.pairing_block()
}

/// Pairing block followed by the assistant prompt, so one click fills the clipboard
/// with everything that has to be pasted into a chat.
#[tauri::command]
pub fn pairing_bundle(state: Shared<'_>) -> String {
    format!("{}\n\n---\n\n{}", state.pairing_block(), prompt::PAIRING_PROMPT)
}

/// Native window move: the title bar starts the system drag loop, so the window
/// follows the pointer even though it has no decorations.
#[tauri::command]
pub fn begin_drag(window: tauri::WebviewWindow) {
    let Ok(handle) = window.hwnd() else { return };
    let hwnd = handle.0 as isize;
    let clone = window.clone();
    let _ = clone.run_on_main_thread(move || winapi::begin_window_drag(hwnd));
}

#[tauri::command]
pub fn restart_tunnel(state: Shared<'_>) {
    state.tunnel.restart();
}

#[tauri::command]
pub fn refresh_windows(state: Shared<'_>) -> Vec<WindowInfo> {
    state.refresh_windows()
}

#[tauri::command]
pub fn activate_window(_state: Shared<'_>, hwnd: i64) -> bool {
    winapi::activate_window(hwnd as isize)
}

#[tauri::command]
pub fn close_window(_state: Shared<'_>, hwnd: i64) -> bool {
    winapi::close_window(hwnd as isize)
}

#[tauri::command]
pub async fn screenshot(_state: Shared<'_>) -> Result<String, String> {
    let shot = tokio::task::spawn_blocking(|| winapi::capture(1440))
        .await
        .map_err(|error| error.to_string())??;
    Ok(b64::encode(&shot.jpeg))
}

#[tauri::command]
pub fn open_data_dir() -> Result<(), String> {
    std::process::Command::new("explorer")
        .arg(paths::data_dir())
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn quit(state: Shared<'_>) {
    state.journal.push(&state.app, "agent", "закрытие из окна лаунчера");
    state.tunnel.stop();
    state.app.exit(0);
}
