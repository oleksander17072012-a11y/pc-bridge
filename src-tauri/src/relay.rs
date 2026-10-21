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
    json_response(StatusCode::OK, json!({ "windows": state.windows_snapshot() }))
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
    if text.chars().count() > 2000 {
        return json_response(StatusCode::PAYLOAD_TOO_LARGE, json!({ "error": "note too long (max 2000 characters)" }));
    }
    let sequence = state.bump_notes();
    state.journal.push(&state.app, "note", &format!("[{sequence:03}] {text}"));
    json_response(StatusCode::OK, json!({ "ok": true, "sequence": sequence }))
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

    let mut answer = match kind.as_str() {
        "shell" => run_shell_job(&state, &payload).await,
        "screenshot" => run_screenshot_job().await,
        "gui" => run_gui_job(&state, &payload).await,
        other => {
            return json_response(StatusCode::BAD_REQUEST, json!({ "error": format!("unknown type: {other}") }));
        }
    };

    if payload.get("screenshot_after").and_then(|value| value.as_bool()).unwrap_or(false) {
        if let Some(extra) = run_screenshot_job().await.as_object() {
            if let Some(object) = answer.as_object_mut() {
                for (key, value) in extra {
                    if key != "exit_code" {
                        object.insert(key.clone(), value.clone());
                    }
                }
            }
        }
    }

    json_response(StatusCode::OK, answer)
}

async fn run_shell_job(state: &Arc<AppState>, payload: &Value) -> Value {
    let command = payload.get("command").and_then(|value| value.as_str()).unwrap_or("").to_string();
    if command.trim().is_empty() {
        return json!({ "error": "no command", "exit_code": 400 });
    }
    let shell_kind = payload.get("shell").and_then(|value| value.as_str()).map(str::to_string);
    let timeout = payload.get("timeout").and_then(|value| value.as_u64()).unwrap_or(110).clamp(1, 600);

    state.journal.push(&state.app, "shell", &format!("$ {}", truncate(&command, 300)));
    let state_after = Arc::clone(state);
    let command_for_thread = command.clone();
    let handle = tokio::task::spawn_blocking(move || {
        shell::run(&command_for_thread, shell_kind.as_deref(), Duration::from_secs(timeout))
    });

    match tokio::time::timeout(Duration::from_secs(timeout + 10), handle).await {
        Ok(Ok(outcome)) => {
            state_after.journal.push(
                &state_after.app,
                "shell",
                &format!("exit={} · {}", outcome.exit_code, truncate(outcome.output.trim(), 240)),
            );
            json!({ "output": outcome.output, "exit_code": outcome.exit_code, "host": state_after.hostname })
        }
        Ok(Err(error)) => json!({ "output": format!("[error] {error}"), "exit_code": 1 }),
        Err(_) => json!({ "output": "[timeout] the command did not finish in time", "exit_code": 124 }),
    }
}

async fn run_screenshot_job() -> Value {
    let handle = tokio::task::spawn_blocking(capture_payload);
    match tokio::time::timeout(Duration::from_secs(60), handle).await {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => json!({ "screenshot_error": format!("{error}"), "exit_code": 1 }),
        Err(_) => json!({ "screenshot_error": "capture timed out", "exit_code": 124 }),
    }
}

fn capture_payload() -> Value {
    match winapi::capture(1280) {
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
        std::thread::sleep(Duration::from_millis(400));
        worker.show_overlay(false);
        outcome
    });

    match tokio::time::timeout(Duration::from_secs(240), handle).await {
        Ok(Ok(outcome)) => {
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
        Ok(Err(error)) => json!({ "output": format!("[error] {error}"), "exit_code": 1 }),
        Err(_) => json!({ "output": "[timeout] gui job exceeded its budget", "exit_code": 124 }),
    }
}
