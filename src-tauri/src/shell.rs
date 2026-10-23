use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

pub struct Outcome {
    pub output: String,
    pub exit_code: i32,
}

pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A last line of defence, not a sandbox. Anything that matters should be confirmed
/// by the user in the launcher window.
const DENYLIST: [&str; 12] = [
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

pub fn run(command: &str, shell: Option<&str>, timeout: Duration) -> Outcome {
    let lowered = command.to_lowercase();
    for blocked in DENYLIST {
        if lowered.contains(blocked) {
            return Outcome {
                output: format!("[BLOCKED by safety denylist] command contains '{}'", blocked.trim()),
                exit_code: 126,
            };
        }
    }

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
            return Outcome { output: format!("[error] {error}"), exit_code: 1 };
        }
    };

    let pid = child.id();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(child.wait_with_output());
    });

    match receiver.recv_timeout(timeout) {
        Ok(Ok(output)) => {
            // Never lossy: a Cyrillic path must survive the trip through the console.
            let mut text = crate::encoding::decode(&output.stdout);
            text.push_str(&crate::encoding::decode(&output.stderr));
            let text: String = text.chars().take(20_000).collect();
            Outcome { output: text, exit_code: output.status.code().unwrap_or(-1) }
        }
        Ok(Err(error)) => Outcome { output: format!("[error] {error}"), exit_code: 1 },
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
            Outcome {
                output: format!("[timeout after {}s — command killed]", timeout.as_secs()),
                exit_code: 124,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::run;
    use std::time::Duration;

    #[test]
    fn denylist_blocks_destructive_commands() {
        let outcome = run("format C: /q", None, Duration::from_secs(1));
        assert_eq!(outcome.exit_code, 126);
        assert!(outcome.output.contains("BLOCKED"));
    }

    #[test]
    fn runs_a_command_and_reports_output() {
        let outcome = run("echo pc-bridge-ok", None, Duration::from_secs(20));
        assert_eq!(outcome.exit_code, 0);
        assert!(outcome.output.contains("pc-bridge-ok"));
    }
}
