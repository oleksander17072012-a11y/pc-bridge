//! Loopback HTTP API. This is the only thing the Cloudflare tunnel exposes, so every
//! endpoint except `/health` requires `Authorization: Bearer <pairing key>`.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::b64;
use crate::input;
use crate::shell;
use crate::state::AppState;
use crate::winapi;

fn json_response(status: StatusCode, value: Value) -> Response {
    (status, Json(value)).into_response()
}

fn unauthorized() -> Response {
    json_response(StatusCode::FORBIDDEN, json!({ "error": "bad token" }))
}

fn authorized(state: &AppState, headers: &HeaderMap) -> bool {
    let expected = format!("Bearer {}", state.key());
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .map(|value| value == expected)
        .unwrap_or(false)
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    text.chars().take(limit).collect::<String>() + "…"
}

pub fn serve(state: Arc<AppState>, listener: std::net::TcpListener) {
    tauri::async_runtime::spawn(async move {
        let _ = listener.set_nonblocking(true);
        let listener = match tokio::net::TcpListener::from_std(listener) {
            Ok(listener) => listener,
            Err(error) => {
                state.journal.push(&state.app, "relay", &format!("не удалось слушать порт: {error}"));
                return;
            }
        };
        let router = Router::new()
            .route("/health", get(health))
            .route("/status", get(status))
            .route("/poll", get(poll))
            .route("/windows", get(windows))
            .route("/journal", get(journal_tail))
            .route("/board", get(board))
            .route("/ping", get(ping))
            .route("/note", post(note))
            .route("/result", post(result))
            .route("/cmd", post(cmd))
            .with_state(state);
        if let Err(error) = axum::serve(listener, router).await {
            eprintln!("relay stopped: {error}");
        }
    });
}

async fn health() -> Json<Value> {
    Json(json!({ "ok": true, "service": "pc-bridge", "version": env!("CARGO_PKG_VERSION") }))
}

async fn status(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if !authorized(&state, &headers) {
        return unauthorized();
    }
    json_response(
        StatusCode::OK,
        json!({
            "ok": true,
            "active": state.is_active(),
            "hostname": state.hostname,
            "tunnel_url": state.tunnel.url(),
            "uptime_seconds": state.uptime_seconds(),
            "commands": state.snapshot().get("commands").cloned().unwrap_or(json!(0)),
            "notes": state.snapshot().get("notes").cloned().unwrap_or(json!(0)),
        }),
    )
}

async fn poll(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if !authorized(&state, &headers) {
        return unauthorized();
    }
    // Kept for compatibility with the old polling protocol: execution is synchronous
    // now, so there is never a queued job to hand out.
    json_response(
        StatusCode::OK,
        json!({ "job": null, "hostname": state.hostname, "active": state.is_active() }),
    )
}

async fn windows(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if !authorized(&state, &headers) {
        return unauthorized();
    }
    json_response(
        StatusCode::OK,
        json!({ "windows": state.windows_recent(std::time::Duration::from_millis(700)) }),
    )
}

/// Tiny authenticated endpoint: lets the agent measure the tunnel latency without
/// pulling any state, and confirm the key still works.
async fn ping(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if !authorized(&state, &headers) {
        return unauthorized();
    }
    json_response(
        StatusCode::OK,
        json!({
            "ok": true,
            "ms_since_start": state.uptime_seconds() * 1000,
            "version": env!("CARGO_PKG_VERSION"),
        }),
    )
}

/// Everything the agent usually asks for in three separate requests, in one.
async fn board(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if !authorized(&state, &headers) {
        return unauthorized();
    }
    json_response(
        StatusCode::OK,
        json!({
            "status": {
                "active": state.is_active(),
                "hostname": state.hostname,
                "tunnel_url": state.tunnel.url(),
                "uptime_seconds": state.uptime_seconds(),
                "stage": state.stage(),
                "version": env!("CARGO_PKG_VERSION"),
            },
            "journal": state.journal.recent(30),
            "windows": state.windows_recent(std::time::Duration::from_millis(700)),
            "active_window": winapi::active_window(),
        }),
    )
}

async fn journal_tail(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if !authorized(&state, &headers) {
        return unauthorized();
    }
    json_response(StatusCode::OK, json!({ "entries": state.journal.recent(100) }))
}

async fn note(State(state): State<Arc<AppState>>, headers: HeaderMap, body: Bytes) -> Response {
    if !authorized(&state, &headers) {
        return unauthorized();
    }
    let payload: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
    let text = payload.get("text").and_then(|value| value.as_str()).unwrap_or("").trim().to_string();
    if text.is_empty() {
        return json_response(StatusCode::BAD_REQUEST, json!({ "error": "text is required" }));
    }

    // Notes double as a structured work log: "stage" starts a phase, "step" narrates one
    // action, "result"/"error" closes it. Anything else is a plain note.
    let kind = match payload.get("kind").and_then(|value| value.as_str()).unwrap_or("note") {
        "stage" => "stage",
        "step" => "step",
        "result" => "result",
        "error" => "error",
        _ => "note",
    };

    let sequence = state.bump_notes();
    if kind == "stage" {
        state.set_stage(Some(text.clone()));
    }
    let line = if kind == "stage" {
        text.clone()
    } else {
        format!("{sequence:02} · {text}")
    };
    state.journal.push(&state.app, kind, &line);
    json_response(StatusCode::OK, json!({ "ok": true, "sequence": sequence, "kind": kind }))
}

async fn result(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if !authorized(&state, &headers) {
        return unauthorized();
    }
    json_response(StatusCode::OK, json!({ "ok": true }))
}

async fn cmd(State(state): State<Arc<AppState>>, headers: HeaderMap, body: Bytes) -> Response {
    if !authorized(&state, &headers) {
        return unauthorized();
    }
    if !state.is_active() {
        return json_response(StatusCode::CONFLICT, json!({ "error": "agent is paused in the launcher" }));
    }

    let payload: Value = serde_json::from_slice(&body).unwrap_or_else(|_| json!({}));
    let kind = payload.get("type").and_then(|value| value.as_str()).unwrap_or("shell").to_string();
    state.bump_commands();
    let started = std::time::Instant::now();

    // A journal line can travel with the command: one request instead of two.
    if let Some(text) = payload.get("note").and_then(|value| value.as_str()) {
        let text = text.trim();
        if !text.is_empty() {
            let note_kind = payload.get("note_kind").and_then(|value| value.as_str()).unwrap_or("note");
            state.journal.push(&state.app, note_kind, text);
        }
    }

    let mut answer = match kind.as_str() {
        "shell" => run_shell_job(&state, &payload).await,
        "screenshot" => {
            let (width, quality) = shot_params(&payload);
            run_screenshot_job_with(width, quality).await
        }
        // Several shell commands in one round trip: each request pays for a TLS handshake
        // and a trip through the tunnel, so batching is the cheapest speed-up there is.
        "batch" => run_batch_job(&state, &payload).await,
        "gui" => run_gui_job(&state, &payload).await,
        other => {
            return json_response(StatusCode::BAD_REQUEST, json!({ "error": format!("unknown type: {other}") }));
        }
    };

    if payload.get("screenshot_after").and_then(|value| value.as_bool()).unwrap_or(false) {
        let (width, quality) = shot_params(&payload);
        if let Some(extra) = run_screenshot_job_with(width, quality).await.as_object() {
            if let Some(object) = answer.as_object_mut() {
                for (key, value) in extra {
                    if key != "exit_code" {
                        object.insert(key.clone(), value.clone());
                    }
                }
            }
        }
    }

    // The agent can now tell bridge time from its own thinking time.
    if let Some(object) = answer.as_object_mut() {
        object.insert("took_ms".into(), json!(started.elapsed().as_millis() as u64));
    }
    json_response(StatusCode::OK, answer)
}

async fn run_shell_job(state: &Arc<AppState>, payload: &Value) -> Value {
    let command = payload.get("command").and_then(|value| value.as_str()).unwrap_or("").to_string();
    if command.trim().is_empty() {
        return json!({ "error": "no command", "exit_code": 400 });
    }
    let shell_kind = payload.get("shell").and_then(|value| value.as_str()).map(str::to_string);
    // `timeout` is optional and honoured as given; without it the command may run for as
    // long as it needs.
    let timeout = payload.get("timeout").and_then(|value| value.as_u64());

    state.journal.push(&state.app, "shell", &format!("$ {}", truncate(&command, 300)));
    let state_after = Arc::clone(state);
    let command_for_thread = command.clone();
    let handle = tokio::task::spawn_blocking(move || {
        shell::run(
            &command_for_thread,
            shell_kind.as_deref(),
            timeout.map(Duration::from_secs),
        )
    });

    // The HTTP request waits without a ceiling; only the command's own timeout applies.
    match handle.await {
        Ok(outcome) => {
            state_after.journal.push(
                &state_after.app,
                "shell",
                &format!("exit={} · {}", outcome.exit_code, truncate(outcome.output.trim(), 240)),
            );
            json!({ "output": outcome.output, "exit_code": outcome.exit_code, "host": state_after.hostname })
        }
        Err(error) => json!({ "output": format!("[error] {error}"), "exit_code": 1 }),
    }
}

async fn run_screenshot_job() -> Value {
    run_screenshot_job_with(SHOT_WIDTH, SHOT_QUALITY).await
}

/// Screenshot with explicit geometry, so the agent can ask for a smaller or a sharper
/// picture depending on what it needs.
async fn run_screenshot_job_with(width: u32, quality: u8) -> Value {
    let handle = tokio::task::spawn_blocking(move || capture_payload(width, quality));
    match handle.await {
        Ok(value) => value,
        Err(error) => json!({ "screenshot_error": format!("{error}"), "exit_code": 1 }),
    }
}

/// Screenshot geometry from the request body, with sane defaults.
fn shot_params(payload: &Value) -> (u32, u8) {
    let width = payload
        .get("width")
        .and_then(|value| value.as_u64())
        .unwrap_or(SHOT_WIDTH as u64)
        .clamp(320, 3840) as u32;
    let quality = payload
        .get("quality")
        .and_then(|value| value.as_u64())
        .unwrap_or(SHOT_QUALITY as u64)
        .clamp(30, 95) as u8;
    (width, quality)
}

/// Runs up to 20 shell commands one after another inside a single request.
async fn run_batch_job(state: &Arc<AppState>, payload: &Value) -> Value {
    let commands: Vec<String> = payload
        .get("commands")
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(|text| text.to_string()))
                .collect()
        })
        .unwrap_or_default();
    if commands.is_empty() {
        return json!({ "error": "batch needs a non-empty 'commands' list", "exit_code": 400 });
    }

    let mut results = Vec::with_capacity(commands.len());
    let mut last_code = 0;
    for command in &commands {
        let step = json!({ "command": command });
        let answer = run_shell_job(state, &step).await;
        last_code = answer.get("exit_code").and_then(|value| value.as_i64()).unwrap_or(0);
        results.push(json!({
            "command": command,
            "output": answer.get("output").cloned().unwrap_or(Value::Null),
            "exit_code": last_code,
        }));
    }

    json!({ "results": results, "count": results.len(), "exit_code": last_code })
}

/// Width and quality of the screenshots the agent receives.
///
/// 1100 px at quality 65 is readable for UI work and roughly four times smaller than a
/// full-resolution capture, which matters most: the picture travels through a public
/// tunnel on every request.
const SHOT_WIDTH: u32 = 1100;
const SHOT_QUALITY: u8 = 65;

fn capture_payload(width: u32, quality: u8) -> Value {
    match winapi::capture(width, quality) {
        Ok(shot) => {
            let active = winapi::active_window();
            json!({
                "image_base64": b64::encode(&shot.jpeg),
                "screen_width": shot.width,
                "screen_height": shot.height,
                "active_window": { "title": active.title, "process": active.process, "hwnd": active.hwnd },
                "exit_code": 0
            })
        }
        Err(error) => json!({ "screenshot_error": error, "exit_code": 1 }),
    }
}

async fn run_gui_job(state: &Arc<AppState>, payload: &Value) -> Value {
    let actions = payload.get("actions").and_then(|value| value.as_array()).cloned().unwrap_or_default();
    if actions.is_empty() {
        return json!({ "output": "[error] gui job needs a non-empty 'actions' list", "exit_code": 400 });
    }
    state
        .journal
        .push(&state.app, "gui", &format!("{} действие(й); системный курсор не перемещается", actions.len()));

    let worker = Arc::clone(state);
    let handle = tokio::task::spawn_blocking(move || {
        worker.show_overlay(true);
        let outcome = input::run(&*worker, &actions);
        // 220 ms is enough for the cursor ring to be seen to settle; the answer does not
        // wait for the overlay itself, only for this short beat.
        std::thread::sleep(Duration::from_millis(220));
        worker.show_overlay(false);
        outcome
    });

    match handle.await {
        Ok(outcome) => {
            state.journal.push(
                &state.app,
                "gui",
                &format!("exit={} · {}", outcome.exit_code, truncate(&outcome.output, 200)),
            );
            json!({
                "output": outcome.output,
                "exit_code": outcome.exit_code,
                "actions_done": outcome.actions_done,
                "host": state.hostname
            })
        }
        Err(error) => json!({ "output": format!("[error] {error}"), "exit_code": 1 }),
    }
}
