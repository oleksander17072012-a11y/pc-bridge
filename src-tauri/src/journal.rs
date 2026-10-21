use std::collections::VecDeque;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

const MAX_ENTRIES: usize = 2000;

#[derive(Clone, Serialize)]
pub struct Entry {
    pub seq: u64,
    pub time: String,
    pub kind: String,
    pub text: String,
}

/// Append-only chronological audit trail: mirrored into the UI, to a file on disk,
/// and to the assistant through `GET /journal`.
pub struct Journal {
    path: PathBuf,
    entries: Mutex<VecDeque<Entry>>,
    sequence: Mutex<u64>,
}

impl Journal {
    pub fn new(path: PathBuf) -> Self {
        Journal { path, entries: Mutex::new(VecDeque::new()), sequence: Mutex::new(0) }
    }

    pub fn push(&self, app: &AppHandle, kind: &str, text: &str) -> Entry {
        let time = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let clean: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
        let clean = clean.trim().to_string();

        let seq = {
            let mut sequence = self.sequence.lock().unwrap_or_else(|error| error.into_inner());
            *sequence += 1;
            *sequence
        };

        let entry = Entry { seq, time: time.clone(), kind: kind.to_string(), text: clean.clone() };

        {
            let mut entries = self.entries.lock().unwrap_or_else(|error| error.into_inner());
            entries.push_back(entry.clone());
            while entries.len() > MAX_ENTRIES {
                entries.pop_front();
            }
        }

        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&self.path) {
            let _ = writeln!(file, "[{time}] [{kind}] {clean}");
            let _ = file.flush();
        }

        let _ = app.emit("journal", &entry);
        entry
    }

    pub fn recent(&self, limit: usize) -> Vec<Entry> {
        let entries = self.entries.lock().unwrap_or_else(|error| error.into_inner());
        let skip = entries.len().saturating_sub(limit);
        entries.iter().skip(skip).cloned().collect()
    }
}
