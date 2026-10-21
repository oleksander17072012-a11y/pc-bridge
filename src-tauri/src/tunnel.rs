//! Cloudflare quick tunnel: downloads `cloudflared` once, verifies its Authenticode
//! signature, keeps the process alive and reports the public URL to the UI.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use tauri::{AppHandle, Emitter};

use crate::journal::Journal;
use crate::paths;
use crate::shell::CREATE_NO_WINDOW;

const DOWNLOAD_URL: &str =
    "https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-windows-amd64.exe";

struct Inner {
    child: Option<Child>,
    url: Option<String>,
}

pub struct Tunnel {
    app: AppHandle,
    journal: Arc<Journal>,
    inner: Arc<Mutex<Inner>>,
    binary: Mutex<Option<PathBuf>>,
    port: Mutex<u16>,
    stopping: AtomicBool,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

impl Tunnel {
    pub fn new(app: AppHandle, journal: Arc<Journal>) -> Self {
        Tunnel {
            app,
            journal,
            inner: Arc::new(Mutex::new(Inner { child: None, url: None })),
            binary: Mutex::new(None),
            port: Mutex::new(0),
            stopping: AtomicBool::new(false),
        }
    }

    pub fn set_port(&self, port: u16) {
        *lock(&self.port) = port;
    }

    pub fn url(&self) -> Option<String> {
        lock(&self.inner).url.clone()
    }

    /// Restarts the tunnel from scratch (the "Обновить туннель" button).
    pub fn restart(self: &Arc<Self>) {
        {
            let mut inner = lock(&self.inner);
            if let Some(mut child) = inner.child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
            inner.url = None;
        }
        let _ = self.app.emit("tunnel", serde_json::json!({ "url": null }));
        self.journal.push(&self.app, "tunnel", "перезапуск туннеля по запросу");
        self.stopping.store(false, Ordering::Relaxed);
        self.start();
    }

    pub fn start(self: &Arc<Self>) {
        if self.stopping.load(Ordering::Relaxed) {
            return;
        }
        let this = Arc::clone(self);
        thread::spawn(move || this.supervise());
    }

    pub fn stop(&self) {
        self.stopping.store(true, Ordering::Relaxed);
        if let Some(mut child) = lock(&self.inner).child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn supervise(self: Arc<Self>) {
        let mut warned = false;
        while !self.stopping.load(Ordering::Relaxed) {
            if self.binary_path().is_none() {
                match download_binary() {
                    Ok(path) => {
                        *lock(&self.binary) = Some(path);
                        self.journal.push(&self.app, "tunnel", "cloudflared скачан, подпись проверена");
                    }
                    Err(error) => {
                        if !warned {
                            self.journal.push(&self.app, "tunnel", &format!("cloudflared недоступен: {error}"));
                            warned = true;
                        }
                        thread::sleep(Duration::from_secs(15));
                        continue;
                    }
                }
            }

            if self.spawn().is_err() {
                thread::sleep(Duration::from_secs(10));
                continue;
            }

            loop {
                thread::sleep(Duration::from_secs(2));
                if self.stopping.load(Ordering::Relaxed) {
                    return;
                }
                let exited = {
                    let mut inner = lock(&self.inner);
                    match inner.child.as_mut() {
                        Some(child) => matches!(child.try_wait(), Ok(Some(_)) | Err(_)),
                        None => true,
                    }
                };
                if exited {
                    {
                        let mut inner = lock(&self.inner);
                        inner.child = None;
                        inner.url = None;
                    }
                    let _ = self.app.emit("tunnel", serde_json::json!({ "url": null }));
                    self.journal.push(&self.app, "tunnel", "туннель упал — перезапускаю");
                    break;
                }
            }
        }
    }

    fn binary_path(&self) -> Option<PathBuf> {
        lock(&self.binary).clone().filter(|path| path.exists())
    }

    fn spawn(&self) -> Result<(), String> {
        let binary = self.binary_path().ok_or_else(|| "cloudflared не установлен".to_string())?;
        let port = *lock(&self.port);
        let mut command = Command::new(&binary);
        command
            .arg("tunnel")
            .arg("--url")
            .arg(format!("http://127.0.0.1:{port}"))
            .arg("--no-autoupdate")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = command.spawn().map_err(|error| format!("не удалось запустить cloudflared: {error}"))?;
        if let Some(stdout) = child.stdout.take() {
            self.read_lines(stdout);
        }
        if let Some(stderr) = child.stderr.take() {
            self.read_lines(stderr);
        }
        lock(&self.inner).child = Some(child);
        self.journal.push(&self.app, "tunnel", "запускаю туннель cloudflared");
        Ok(())
    }

    /// Reads a `cloudflared` pipe and publishes the first public URL it announces.
    fn read_lines<R: std::io::Read + Send + 'static>(&self, stream: R) {
        let app = self.app.clone();
        let journal = Arc::clone(&self.journal);
        let inner = Arc::clone(&self.inner);
        thread::spawn(move || {
            let reader = BufReader::new(stream);
            for line in reader.lines().map_while(Result::ok) {
                let Some(url) = extract_url(&line) else { continue };
                let changed = {
                    let mut inner = lock(&inner);
                    if inner.url.as_deref() == Some(url.as_str()) {
                        false
                    } else {
                        inner.url = Some(url.clone());
                        true
                    }
                };
                if changed {
                    journal.push(&app, "tunnel", &format!("туннель готов: {url}"));
                    let _ = app.emit("tunnel", serde_json::json!({ "url": url }));
                }
            }
        });
    }
}

fn extract_url(line: &str) -> Option<String> {
    let mut rest = line;
    while let Some(index) = rest.find("https://") {
        let tail = &rest[index..];
        let end = tail
            .find(|character: char| {
                !(character.is_ascii_alphanumeric()
                    || character == '-'
                    || character == '.'
                    || character == ':'
                    || character == '/')
            })
            .unwrap_or(tail.len());
        let candidate = &tail[..end];
        if candidate.contains(".trycloudflare.com") {
            return Some(candidate.trim_end_matches(':').to_string());
        }
        rest = &tail[end..];
    }
    None
}

fn download_binary() -> Result<PathBuf, String> {
    let path = paths::bin_dir().join("cloudflared.exe");
    if path.exists() {
        return Ok(path);
    }

    let mut curl = Command::new("curl.exe");
    curl.args(["-sSL", "--fail", "-o"])
        .arg(&path)
        .arg(DOWNLOAD_URL)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        curl.creation_flags(CREATE_NO_WINDOW);
    }
    let status = curl.status().map_err(|error| format!("curl не запустился: {error}"))?;
    if !status.success() {
        let _ = std::fs::remove_file(&path);
        return Err(format!("загрузка не удалась (код {:?})", status.code()));
    }

    if let Err(error) = verify_signature(&path) {
        let _ = std::fs::remove_file(&path);
        return Err(format!("подпись cloudflared не подтверждена: {error}"));
    }
    Ok(path)
}

/// Authenticode check: the downloaded executable must carry a valid signature issued
/// to Cloudflare. This closes the usual "download latest, trust it blindly" gap.
fn verify_signature(path: &Path) -> Result<(), String> {
    let escaped = path.to_string_lossy().replace('\'', "''");
    let script = format!(
        "$s = Get-AuthenticodeSignature -FilePath '{escaped}'; \
         if ($s.Status -ne 'Valid') {{ exit 2 }}; \
         if ($s.SignerCertificate.Subject -notmatch 'Cloudflare') {{ exit 3 }}; exit 0"
    );
    let mut powershell = Command::new("powershell");
    powershell
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        powershell.creation_flags(CREATE_NO_WINDOW);
    }
    match powershell.status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("Get-AuthenticodeSignature вернул код {:?}", status.code())),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::extract_url;

    #[test]
    fn parses_quick_tunnel_url() {
        let line = "2026-10-21T10:00:00Z INF |  https://involvement-blues-amounts-morgan.trycloudflare.com  |";
        assert_eq!(
            extract_url(line).as_deref(),
            Some("https://involvement-blues-amounts-morgan.trycloudflare.com")
        );
    }

    #[test]
    fn ignores_unrelated_links() {
        assert_eq!(extract_url("download from https://github.com/cloudflare/cloudflared"), None);
        assert_eq!(extract_url("no links here"), None);
    }
}
