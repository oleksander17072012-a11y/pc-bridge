use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

pub struct Outcome {
    pub output: String,
    pub exit_code: i32,
    /// Set when the command matched the watchlist: it still ran, but the user must see it.
    pub warning: Option<String>,
}

pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Commands that get a loud line in the journal when they run.
///
/// This is a **watchlist, not a blocklist**: nothing is refused here, because the user
/// asked for full access to their own machine. What it does is make an irreversible
/// command impossible to miss in the launcher's log.
const WATCHLIST: &[&str] = &[
    // boot and disk
    "format ",
    "diskpart",
    "bcdedit",
    "vssadmin delete",
    "wbadmin delete",
    "cipher /w:",
    "clear-disk",
    // system files and protection (allowed, but never silently)
    "c:\\windows",
    "system32",
    "program files",
    "set-mppreference",
    "netsh advfirewall",
    "sc delete",
    "sc stop",
    "reg add",
    "reg delete",
    "icacls",
    // accounts and autostart
    "net user",
    "net localgroup",
    "schtasks /create",
    "schtasks /delete",
    "new-itemproperty",
];

/// `timeout: None` means "wait as long as the command needs".
/// Returns the watchlist entry a command matches, if any.
pub fn watchlist_hit(command: &str) -> Option<&'static str> {
    let lowered = command.to_lowercase();
    WATCHLIST.iter().find(|entry| lowered.contains(*entry)).copied()
}

/// Above this the command line gets close to the Windows limit (32 767 bytes), which
/// surfaces as `os error 206`. Such commands are written to a temporary script instead.
const SCRIPT_THRESHOLD: usize = 6000;

pub fn run(command: &str, shell: Option<&str>, timeout: Option<Duration>) -> Outcome {
    let warning = watchlist_hit(command).map(|hit| {
        format!(
            "внимание: команда содержит «{}» — необратимое действие, выполнено по настройке",
            hit.trim()
        )
    });

    // A very long one-liner would hit the Windows command-line limit; run it from a file.
    let temp_script = if command.len() > SCRIPT_THRESHOLD {
        write_script(command, shell).ok()
    } else {
        None
    };

    let mut process = match (shell, &temp_script) {
        (Some("powershell") | Some("pwsh"), Some(path)) => {
            let mut command_line = Command::new("powershell");
            command_line.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"]);
            command_line.arg(path);
            command_line
        }
        (_, Some(path)) => {
            let mut command_line = Command::new("cmd.exe");
            command_line.args(["/c"]);
            command_line.arg(path);
            command_line
        }
        (Some("powershell") | Some("pwsh"), None) => {
            let mut command_line = Command::new("powershell");
            let wrapped = crate::encoding::powershell_wrapper(command);
            command_line.args(["-NoProfile", "-NonInteractive", "-Command", &wrapped]);
            command_line
        }
        (_, None) => {
            let mut command_line = Command::new("cmd.exe");
            let wrapped = crate::encoding::cmd_wrapper(command);
            command_line.args(["/c", &wrapped]);
            command_line
        }
    };

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        process.creation_flags(CREATE_NO_WINDOW);
    }

    process.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());

    let child = match process.spawn() {
        Ok(child) => child,
        Err(error) => {
            return Outcome { output: format!("[error] {error}"), exit_code: 1, warning };
        }
    };

    let pid = child.id();
    let script_path = temp_script.clone();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(child.wait_with_output());
    });

    // No truncation: the caller gets everything the command printed.
    let received = match timeout {
        Some(limit) => receiver.recv_timeout(limit),
        None => receiver.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected),
    };

    match received {
        Ok(Ok(output)) => {
            // Never lossy: a Cyrillic path must survive the trip through the console.
            let mut text = crate::encoding::decode(&output.stdout);
            text.push_str(&crate::encoding::decode(&output.stderr));
            if let Some(path) = &script_path {
                let _ = std::fs::remove_file(path);
            }
            Outcome { output: text, exit_code: output.status.code().unwrap_or(-1), warning }
        }
        Ok(Err(error)) => Outcome { output: format!("[error] {error}"), exit_code: 1, warning },
        Err(mpsc::RecvTimeoutError::Disconnected) => Outcome {
            output: "[error] the command produced no result".into(),
            exit_code: 1,
            warning,
        },
        Err(_) => {
            // Best effort: the reader thread owns the handle, so kill by PID.
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                let _ = Command::new("taskkill")
                    .args(["/F", "/T", "/PID", &pid.to_string()])
                    .creation_flags(CREATE_NO_WINDOW)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
            let seconds = timeout.map(|limit| limit.as_secs()).unwrap_or(0);
            Outcome {
                output: format!("[timeout after {seconds}s — command killed]"),
                exit_code: 124,
                warning,
            }
        }
    }
}

/// Writes a long command to a temporary script and returns its path.
fn write_script(command: &str, shell: Option<&str>) -> Result<std::path::PathBuf, std::io::Error> {
    let powershell = matches!(shell, Some("powershell") | Some("pwsh"));
    let extension = if powershell { "ps1" } else { "bat" };
    let mut path = std::env::temp_dir();
    path.push(format!("pcbridge-{}.{}", crate::winapi::random_hex(6), extension));
    let body = if powershell {
        format!("$ErrorActionPreference='Continue'\n{command}\n")
    } else {
        format!("@echo off\r\nchcp 65001>nul\r\n{command}\r\n")
    };
    std::fs::write(&path, body)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::{run, watchlist_hit};
    use std::time::Duration;

    #[test]
    fn dangerous_commands_are_watched_but_not_blocked() {
        // Full access was requested, so nothing is refused. The guard is visibility:
        // the command runs and the launcher shows it in red.
        assert!(watchlist_hit("format C: /q").is_some());
        assert!(watchlist_hit("DiskPart").is_some());
        assert!(watchlist_hit("dir C:\\Users").is_none());
    }

    #[test]
    fn runs_a_command_and_reports_output() {
        let outcome = run("echo pc-bridge-ok", None, Some(Duration::from_secs(20)));
        assert_eq!(outcome.exit_code, 0);
        assert!(outcome.output.contains("pc-bridge-ok"));
    }
}
