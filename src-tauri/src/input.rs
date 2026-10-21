//! Background GUI driver.
//!
//! Every action is delivered with `PostMessageW` to the window under the *virtual*
//! cursor, so the physical mouse and keyboard of the user are never touched. The
//! window is resolved with `WindowFromPoint`, exactly like the Python prototype.

use std::thread::sleep;
use std::time::Duration;

use serde_json::Value;

use crate::winapi;

/// Abstraction over the launcher state so the driver stays testable and does not
/// depend on Tauri types directly.
pub trait CursorSink {
    fn cursor(&self) -> (i32, i32);
    fn set_cursor(&self, x: i32, y: i32);
    fn pulse(&self);
}

pub struct Outcome {
    pub output: String,
    pub exit_code: i32,
    pub actions_done: usize,
}

fn number(value: Option<&Value>, fallback: i64) -> i64 {
    match value {
        Some(Value::Number(number)) => number.as_i64().or_else(|| number.as_f64().map(|f| f as i64)).unwrap_or(fallback),
        Some(Value::String(text)) => text.trim().parse().unwrap_or(fallback),
        _ => fallback,
    }
}

fn number_f(value: Option<&Value>, fallback: f64) -> f64 {
    match value {
        Some(Value::Number(number)) => number.as_f64().unwrap_or(fallback),
        Some(Value::String(text)) => text.trim().parse().unwrap_or(fallback),
        _ => fallback,
    }
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(|item| item.as_str()).unwrap_or("")
}

fn action_name(value: &Value) -> String {
    for key in ["action", "type", "name", "op"] {
        if let Some(name) = value.get(key).and_then(|item| item.as_str()) {
            return name.to_string();
        }
    }
    String::new()
}

fn target(x: i32, y: i32) -> Result<isize, String> {
    let hwnd = winapi::hwnd_at(x, y);
    if hwnd == 0 {
        return Err("под виртуальным курсором нет видимого окна".into());
    }
    Ok(hwnd)
}

fn move_to(sink: &dyn CursorSink, x: i32, y: i32, duration: f64) {
    let (start_x, start_y) = sink.cursor();
    let duration = duration.clamp(0.0, 4.0);
    let steps = ((duration * 30.0) as i32).clamp(1, 50);
    for step in 1..=steps {
        let ratio = step as f64 / steps as f64;
        let px = (start_x as f64 + (x - start_x) as f64 * ratio).round() as i32;
        let py = (start_y as f64 + (y - start_y) as f64 * ratio).round() as i32;
        sink.set_cursor(px, py);
        let hwnd = winapi::hwnd_at(px, py);
        if hwnd != 0 {
            let _ = winapi::post_mouse(hwnd, winapi::WM_MOUSEMOVE, 0, px, py, false);
        }
        if duration > 0.0 {
            sleep(Duration::from_secs_f64(duration / steps as f64));
        }
    }
}

fn click(sink: &dyn CursorSink, x: i32, y: i32, button: &str, double: bool) -> Result<(), String> {
    sink.set_cursor(x, y);
    let hwnd = target(x, y)?;
    let spec = winapi::mouse_spec(button)
        .ok_or_else(|| format!("неподдерживаемая кнопка мыши: {button}"))?;
    let _ = winapi::post_mouse(hwnd, winapi::WM_MOUSEMOVE, 0, x, y, false);
    winapi::mouse_down(hwnd, spec, x, y)?;
    sink.pulse();
    sleep(Duration::from_millis(60));
    if double && matches!(button, "left" | "primary") {
        winapi::mouse_up(hwnd, spec, x, y);
        sleep(Duration::from_millis(80));
        winapi::double_click_marker(hwnd, spec, x, y);
        sink.pulse();
        sleep(Duration::from_millis(50));
        winapi::mouse_up(hwnd, spec, x, y);
    } else {
        winapi::mouse_up(hwnd, spec, x, y);
    }
    Ok(())
}

fn press_key(hwnd: isize, key: &str) -> Result<(), String> {
    let (vk, modifiers) = winapi::key_codes(key).ok_or_else(|| format!("неподдерживаемая клавиша: {key}"))?;
    let mut held: Vec<u16> = Vec::new();
    for modifier in &modifiers {
        winapi::post_key(hwnd, *modifier, true);
        held.push(*modifier);
    }
    winapi::post_key(hwnd, vk, true);
    held.push(vk);
    for code in held.iter().rev() {
        winapi::post_key(hwnd, *code, false);
    }
    Ok(())
}

fn hotkey(hwnd: isize, keys: &[Value]) -> Result<(), String> {
    if keys.is_empty() {
        return Err("hotkey требует непустой список keys".into());
    }
    let mut explicit: Vec<u16> = Vec::new();
    let mut implicit: Vec<u16> = Vec::new();
    for item in keys {
        let name = item.as_str().unwrap_or("");
        let (vk, modifiers) = winapi::key_codes(name).ok_or_else(|| format!("неподдерживаемая клавиша: {name}"))?;
        explicit.push(vk);
        implicit.extend(modifiers);
    }
    let modifier_set = [winapi::VK_SHIFT, winapi::VK_CONTROL, winapi::VK_MENU, winapi::VK_LWIN, winapi::VK_RWIN];
    let mut sequence: Vec<u16> = Vec::new();
    for code in explicit.iter().filter(|code| modifier_set.contains(code)) {
        if !sequence.contains(code) {
            sequence.push(*code);
        }
    }
    for code in implicit {
        if !sequence.contains(&code) {
            sequence.push(code);
        }
    }
    for code in &explicit {
        if !sequence.contains(code) {
            sequence.push(*code);
        }
    }
    for code in &sequence {
        winapi::post_key(hwnd, *code, true);
    }
    sleep(Duration::from_millis(40));
    for code in sequence.iter().rev() {
        winapi::post_key(hwnd, *code, false);
    }
    Ok(())
}

fn typewrite(hwnd: isize, value: &str, interval: f64) -> Result<(), String> {
    let interval = interval.clamp(0.0, 1.0);
    for character in value.chars() {
        match character {
            '\n' => press_key(hwnd, "enter")?,
            '\t' => press_key(hwnd, "tab")?,
            '\r' => {}
            _ => {
                let mut buffer = [0u16; 2];
                for unit in character.encode_utf16(&mut buffer) {
                    if !winapi::post_char(hwnd, *unit) {
                        return Err("не удалось отправить текст в окно".into());
                    }
                }
            }
        }
        if interval > 0.0 {
            sleep(Duration::from_secs_f64(interval));
        }
    }
    Ok(())
}

fn drag(sink: &dyn CursorSink, end_x: i32, end_y: i32, duration: f64, button: &str) -> Result<(), String> {
    let (start_x, start_y) = sink.cursor();
    let hwnd = target(start_x, start_y)?;
    let spec = winapi::mouse_spec(button).ok_or_else(|| format!("неподдерживаемая кнопка перетаскивания: {button}"))?;
    let _ = winapi::post_mouse(hwnd, winapi::WM_MOUSEMOVE, 0, start_x, start_y, false);
    winapi::mouse_down(hwnd, spec, start_x, start_y)?;
    sink.pulse();
    let duration = duration.clamp(0.0, 4.0);
    let steps = ((duration * 30.0) as i32).clamp(1, 50);
    for step in 1..=steps {
        let ratio = step as f64 / steps as f64;
        let px = (start_x as f64 + (end_x - start_x) as f64 * ratio).round() as i32;
        let py = (start_y as f64 + (end_y - start_y) as f64 * ratio).round() as i32;
        sink.set_cursor(px, py);
        let _ = winapi::post_mouse(hwnd, winapi::WM_MOUSEMOVE, spec.2, px, py, false);
        if duration > 0.0 {
            sleep(Duration::from_secs_f64(duration / steps as f64));
        }
    }
    sink.set_cursor(end_x, end_y);
    winapi::mouse_up(hwnd, spec, end_x, end_y);
    Ok(())
}

/// Runs a list of GUI actions. Mirrors the Python driver's semantics and exit codes:
/// `0` ok, `95` the target did not accept background input, `130` failsafe abort,
/// `400` malformed request.
pub fn run(sink: &dyn CursorSink, actions: &[Value]) -> Outcome {
    if actions.is_empty() {
        return Outcome {
            output: "[error] gui job needs a non-empty 'actions' list".into(),
            exit_code: 400,
            actions_done: 0,
        };
    }

    let mut done = 0usize;
    for action in actions {
        let (cursor_x, cursor_y) = sink.cursor();
        if cursor_x < 10 && cursor_y < 10 {
            return Outcome {
                output: "[aborted] failsafe: виртуальный курсор в левом верхнем углу".into(),
                exit_code: 130,
                actions_done: done,
            };
        }

        let name = action_name(action);
        let result: Result<(), String> = match name.as_str() {
            "moveTo" => {
                move_to(sink, number(action.get("x"), cursor_x as i64) as i32, number(action.get("y"), cursor_y as i64) as i32, number_f(action.get("duration"), 0.3));
                Ok(())
            }
            "click" => click(
                sink,
                number(action.get("x"), cursor_x as i64) as i32,
                number(action.get("y"), cursor_y as i64) as i32,
                if text(action, "button").is_empty() { "left" } else { text(action, "button") },
                false,
            ),
            "doubleClick" => click(
                sink,
                number(action.get("x"), cursor_x as i64) as i32,
                number(action.get("y"), cursor_y as i64) as i32,
                "left",
                true,
            ),
            "rightClick" => click(
                sink,
                number(action.get("x"), cursor_x as i64) as i32,
                number(action.get("y"), cursor_y as i64) as i32,
                "right",
                false,
            ),
            "typewrite" => target(cursor_x, cursor_y)
                .and_then(|hwnd| typewrite(hwnd, text(action, "text"), number_f(action.get("interval"), 0.02))),
            "hotkey" => {
                let keys: Vec<Value> = action
                    .get("keys")
                    .and_then(|value| value.as_array())
                    .cloned()
                    .unwrap_or_default();
                target(cursor_x, cursor_y).and_then(|hwnd| hotkey(hwnd, &keys))
            }
            "press" => target(cursor_x, cursor_y).and_then(|hwnd| press_key(hwnd, text(action, "key"))),
            "scroll" => {
                let x = number(action.get("x"), cursor_x as i64) as i32;
                let y = number(action.get("y"), cursor_y as i64) as i32;
                move_to(sink, x, y, 0.0);
                target(x, y).and_then(|hwnd| winapi::post_wheel_root(hwnd, x, y, number(action.get("clicks"), 0) as i32))
            }
            "dragTo" => drag(
                sink,
                number(action.get("x"), cursor_x as i64) as i32,
                number(action.get("y"), cursor_y as i64) as i32,
                number_f(action.get("duration"), 0.5),
                if text(action, "button").is_empty() { "left" } else { text(action, "button") },
            ),
            "wait" => {
                sleep(Duration::from_secs_f64(number_f(action.get("seconds"), 1.0).max(0.0)));
                Ok(())
            }
            other => {
                return Outcome {
                    output: format!("[error] unknown action '{other}' (step {done})"),
                    exit_code: 400,
                    actions_done: done,
                }
            }
        };

        match result {
            Ok(()) => {
                done += 1;
                sleep(Duration::from_millis(50));
            }
            Err(message) => {
                return Outcome {
                    output: format!("[unsupported/background input] {message}"),
                    exit_code: 95,
                    actions_done: done,
                }
            }
        }
    }

    Outcome {
        output: format!("ok: {done} background gui action(s); system cursor untouched"),
        exit_code: 0,
        actions_done: done,
    }
}
