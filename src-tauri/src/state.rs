use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition};

use crate::input::CursorSink;
use crate::journal::Journal;
use crate::paths;
use crate::tunnel::Tunnel;
use crate::winapi::{self, WindowInfo};

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

pub struct AppState {
    pub app: AppHandle,
    pub journal: Arc<Journal>,
    pub tunnel: Arc<Tunnel>,
    pub hostname: String,
    key: Mutex<String>,
    port: Mutex<u16>,
    active: AtomicBool,
    stage: Mutex<Option<String>>,
    cursor: Mutex<Option<(i32, i32)>>,
    windows: Mutex<Vec<WindowInfo>>,
    windows_at: Mutex<Option<Instant>>,
    started: Instant,
    commands: AtomicU64,
    notes: AtomicU64,
    started_at: String,
}

impl AppState {
    pub fn new(app: AppHandle) -> Arc<Self> {
        let journal = Arc::new(Journal::new(paths::log_path()));
        let tunnel = Arc::new(Tunnel::new(app.clone(), Arc::clone(&journal)));
        let key = winapi::random_hex(16);
        let hostname = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "unknown-host".to_string());
        let state = Arc::new(AppState {
            app,
            journal,
            tunnel,
            hostname,
            key: Mutex::new(key),
            port: Mutex::new(0),
            active: AtomicBool::new(true),
            stage: Mutex::new(None),
            cursor: Mutex::new(None),
            windows: Mutex::new(Vec::new()),
            windows_at: Mutex::new(None),
            started: Instant::now(),
            commands: AtomicU64::new(0),
            notes: AtomicU64::new(0),
            started_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        });
        state.write_key_file();
        state
    }

    pub fn key(&self) -> String {
        lock(&self.key).clone()
    }

    pub fn rotate_key(&self) -> String {
        let fresh = winapi::random_hex(16);
        *lock(&self.key) = fresh;
        self.write_key_file();
        self.journal.push(&self.app, "key", "ключ перегенерирован — старый отозван");
        let _ = self.app.emit("state", self.snapshot());
        self.key()
    }

    pub fn set_port(&self, port: u16) {
        *lock(&self.port) = port;
        self.tunnel.set_port(port);
    }

    pub fn port(&self) -> u16 {
        *lock(&self.port)
    }

    pub fn pairing_block(&self) -> String {
        let url = self.tunnel.url().unwrap_or_else(|| "туннель ещё не поднялся".to_string());
        format!(
            "PC-BRIDGE-PAIRING v1\nurl={url}\nkey={}\ncreated={}",
            self.key(),
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
        )
    }

    pub fn write_key_file(&self) {
        let _ = std::fs::write(paths::key_path(), self.pairing_block());
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }

    pub fn set_active(&self, active: bool) {
        self.active.store(active, Ordering::Relaxed);
        self.journal.push(
            &self.app,
            "agent",
            if active { "агент возобновлён" } else { "агент поставлен на паузу" },
        );
        let _ = self.app.emit("state", self.snapshot());
    }

    /// Current stage of the work the assistant is performing (shown as a log header).
    pub fn set_stage(&self, stage: Option<String>) {
        *lock(&self.stage) = stage;
        let _ = self.app.emit("state", self.snapshot());
    }

    pub fn stage(&self) -> Option<String> {
        lock(&self.stage).clone()
    }

    pub fn bump_commands(&self) {
        self.commands.fetch_add(1, Ordering::Relaxed);
    }

    pub fn bump_notes(&self) -> u64 {
        self.notes.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn uptime_seconds(&self) -> u64 {
        self.started.elapsed().as_secs()
    }

    /// The virtual cursor is permanent: there is no toggle in the UI any more.
    pub fn overlay_enabled(&self) -> bool {
        true
    }

    pub fn show_overlay(&self, visible: bool) {
        if let Some(window) = self.app.get_webview_window("overlay") {
            if visible {
                let (x, y) = self.cursor();
                let _ = window.set_position(PhysicalPosition::new(x - 24, y - 24));
                let _ = window.show();
            } else {
                let _ = window.hide();
            }
        }
    }

    pub fn refresh_windows(&self) -> Vec<WindowInfo> {
        let snapshot = winapi::list_windows();
        *lock(&self.windows) = snapshot.clone();
        *lock(&self.windows_at) = Some(Instant::now());
        snapshot
    }

    /// Window list that is at most `max_age` old.
    ///
    /// Enumerating every top-level window costs real time on a busy desktop, and nothing
    /// in the UI needs it on a fixed schedule any more — so the list is built when
    /// somebody actually asks for it and reused for a short while after.
    pub fn windows_recent(&self, max_age: std::time::Duration) -> Vec<WindowInfo> {
        let fresh = {
            let at = lock(&self.windows_at);
            matches!(*at, Some(stamp) if stamp.elapsed() < max_age)
        };
        if fresh {
            return self.windows_snapshot();
        }
        self.refresh_windows()
    }

    pub fn windows_snapshot(&self) -> Vec<WindowInfo> {
        lock(&self.windows).clone()
    }

    pub fn snapshot(&self) -> Value {
        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "hostname": self.hostname,
            "port": self.port(),
            "tunnel_url": self.tunnel.url(),
            "pairing": self.pairing_block(),
            "active": self.is_active(),
            "stage": self.stage(),
            "overlay": self.overlay_enabled(),
            "elevated_hint": winapi::is_elevated(),
            "uptime_seconds": self.uptime_seconds(),
            "commands": self.commands.load(Ordering::Relaxed),
            "notes": self.notes.load(Ordering::Relaxed),
            "started_at": self.started_at,
            "data_dir": paths::data_dir().to_string_lossy(),
        })
    }
}

impl CursorSink for AppState {
    fn cursor(&self) -> (i32, i32) {
        let mut slot = lock(&self.cursor);
        if slot.is_none() {
            let (width, height) = winapi::screen_size();
            *slot = Some((width / 2, height / 2));
        }
        slot.unwrap_or((0, 0))
    }

    fn set_cursor(&self, x: i32, y: i32) {
        *lock(&self.cursor) = Some((x, y));
        if let Some(window) = self.app.get_webview_window("overlay") {
            let _ = window.set_position(PhysicalPosition::new(x - 24, y - 24));
            if !window.is_visible().unwrap_or(false) {
                let _ = window.show();
            }
        }
    }

    fn pulse(&self) {
        let _ = self.app.emit_to("overlay", "pulse", ());
    }
}
