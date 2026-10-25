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
const WATCHLIST: [&str; 12] = [
    "format ",
    "rd /s /q c:",
    "del /s /q c:",
    "shutdown /s",
    "shutdown /r",
    "diskpart",
    "bcdedit",
    "cipher /w:",
    "vssadmin delete",
    "wbadmin delete",
    "net user administrator",
    "reg delete hklm",
];

/// `timeout: None` means "wait as long as the command needs".
/// Returns the watchlist entry a command matches, if any.
pub fn watchlist_hit(command: &str) -> Option<&'static str> {
    let lowered = command.to_lowercase();
    WATCHLIST.iter().find(|entry| lowered.contains(*entry)).copied()
}

pub fn run(command: &str, shell: Option<&str>, timeout: Option<Duration>) -> Outcome {
    let warning = watchlist_hit(command).map(|hit| {
        format!(
            "внимание: команда содержит «{}» — необратимое действие, выполнено по настройке",
            hit.trim()
        )
    });

    let mut process = match shell {
        Some("powershell") | Some("pwsh") => {
            let mut command_line = Command::new("powershell");
            let wrapped = crate::encoding::powershell_wrapper(command);
            command_line.args(["-NoProfile", "-NonInteractive", "-Command", &wrapped]);
            command_line
        }
        _ => {
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
