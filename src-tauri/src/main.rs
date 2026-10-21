#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(windows))]
compile_error!("PC-Bridge is a Windows application: it relies on user32/gdi32 for window enumeration, background input and screen capture.");

mod b64;
mod commands;
mod input;
mod journal;
mod paths;
mod prompt;
mod relay;
mod shape;
mod shell;
mod state;
mod tunnel;
mod winapi;

use std::sync::Arc;
use std::time::Duration;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Emitter, Manager, RunEvent};

use crate::state::AppState;

fn main() {
    let app = tauri::Builder::default()
        .setup(|app| {
            let handle = app.handle().clone();
            let state = AppState::new(handle.clone());

            // 1. Loopback relay: an ephemeral port keeps parallel instances from clashing.
            let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
                .map_err(|error| format!("не удалось занять локальный порт: {error}"))?;
            let port = listener
                .local_addr()
                .map_err(|error| error.to_string())?
                .port();
            state.set_port(port);
            state
                .journal
                .push(&handle, "relay", &format!("релей слушает 127.0.0.1:{port}"));
            relay::serve(Arc::clone(&state), listener);

            // 2. Rounded outline (see shape.rs). No blur effects and no resize frame:
            //    DWM effects painted the full window rectangle (square corners, drag lag)
            //    and the thick frame drew a light outline along the edges.
            if let Some(main) = app.get_webview_window("main") {
                // Give the window a moment to reach its final size, then clip it.
                let target = main.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(250));
                    let clone = target.clone();
                    let _ = target.run_on_main_thread(move || shape::apply(&clone));
                });
            }

            // 3. The overlay cursor must never steal hit-testing from real windows.
            if let Some(overlay) = app.get_webview_window("overlay") {
                let _ = overlay.set_ignore_cursor_events(true);
            }

            // 4. Public tunnel.
            state.tunnel.start();

            // 5. Keep the window cache and the pairing file fresh.
            {
                let worker = Arc::clone(&state);
                std::thread::spawn(move || {
                    let mut last_url: Option<String> = None;
                    loop {
                        std::thread::sleep(Duration::from_secs(3));
                        let _ = worker.refresh_windows();
                        let url = worker.tunnel.url();
                        if url != last_url {
                            last_url = url;
                            worker.write_key_file();
                            let _ = worker.app.emit("state", worker.snapshot());
                        }
                    }
                });
            }

            // 6. Closing the window hides it to the tray; the agent keeps running.
            if let Some(window) = app.get_webview_window("main") {
                let hide_handle = handle.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        if let Some(main) = hide_handle.get_webview_window("main") {
                            let _ = main.hide();
                        }
                    }
                });
            }

            // 7. Tray menu.
            let show_item = MenuItem::with_id(app, "show", "Показать окно", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Выход", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_item, &quit_item])?;
            let mut tray = TrayIconBuilder::with_id("main-tray")
                .menu(&menu)
                .tooltip("PC-Bridge — мост к вашему ПК")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;

            app.manage(Arc::clone(&state));
            state.journal.push(&handle, "agent", "PC-Bridge запущен");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::snapshot,
            commands::journal_tail,
            commands::set_active,
            commands::rotate_key,
            commands::restart_tunnel,
            commands::refresh_windows,
            commands::activate_window,
            commands::close_window,
            commands::screenshot,
            commands::pairing_bundle,
            commands::open_data_dir,
            commands::quit,
        ])
        .build(tauri::generate_context!())
        .expect("не удалось собрать приложение PC-Bridge");

    app.run(|handle, event| {
        if let RunEvent::ExitRequested { .. } = event {
            if let Some(state) = handle.try_state::<Arc<AppState>>() {
                state.tunnel.stop();
            }
        }
    });
}
