//! Long-running commands, detached from the HTTP request.
//!
//! Why this exists: Cloudflare closes a proxied request after ~100 seconds (524). A build
//! that takes two minutes therefore cannot be a synchronous `/cmd` — the answer is lost
//! even though the command ran. So a command can be started in the background and its
//! output collected afterwards, one short request at a time.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

use serde_json::{json, Value};

#[derive(Clone)]
struct Entry {
    status: &'static str, // "running" | "done"
    started: Instant,
    finished_ms: Option<u64>,
    output: String,
    exit_code: i32,
}

pub struct Jobs {
    entries: Mutex<HashMap<String, Entry>>,
}

impl Jobs {
    pub fn new() -> Self {
        Jobs { entries: Mutex::new(HashMap::new()) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Entry>> {
        self.entries.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// Registers a job and returns its id.
    pub fn start(&self) -> String {
        let id = crate::winapi::random_hex(8);
        // Keep the registry small: drop the ten oldest finished jobs when it grows.
        {
            let mut entries = self.lock();
            if entries.len() > 60 {
                let mut finished: Vec<(String, Instant)> = entries
                    .iter()
                    .filter(|(_, entry)| entry.status == "done")
                    .map(|(id, entry)| (id.clone(), entry.started))
                    .collect();
                finished.sort_by_key(|(_, started)| *started);
                for (id, _) in finished.into_iter().take(10) {
                    entries.remove(&id);
                }
            }
            entries.insert(
                id.clone(),
                Entry {
                    status: "running",
                    started: Instant::now(),
                    finished_ms: None,
                    output: String::new(),
                    exit_code: -1,
                },
            );
        }
        id
    }

    pub fn finish(&self, id: &str, output: String, exit_code: i32) {
        let mut entries = self.lock();
        if let Some(entry) = entries.get_mut(id) {
            entry.status = "done";
            entry.finished_ms = Some(entry.started.elapsed().as_millis() as u64);
            entry.output = output;
            entry.exit_code = exit_code;
        }
    }

    /// Current state of a job, or `None` when the id is unknown.
    pub fn get(&self, id: &str) -> Option<Value> {
        let entries = self.lock();
        let entry = entries.get(id)?;
        Some(json!({
            "job_id": id,
            "status": entry.status,
            "took_ms": entry.started.elapsed().as_millis() as u64,
            "finished_ms": entry.finished_ms,
            "output": entry.output,
            "exit_code": entry.exit_code,
        }))
    }

    pub fn ids(&self) -> Vec<Value> {
        let entries = self.lock();
        entries
            .iter()
            .map(|(id, entry)| {
                json!({
                    "job_id": id,
                    "status": entry.status,
                    "took_ms": entry.started.elapsed().as_millis() as u64,
                    "exit_code": entry.exit_code,
                })
            })
            .collect()
    }
}
