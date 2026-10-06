//! Utilities for spawning commands in new terminal windows.

use crate::error::Result;
use std::process::{Command, Stdio};

/// Terminal emulators to try, in order.
const TERMINAL_EMULATORS: &[(&str, &[&str])] = &[
    ("gnome-terminal", &["--", "bash", "-c"]),
    ("xterm", &["-e", "bash", "-c"]),
    ("konsole", &["--nofork", "-e", "bash", "-c"]),
    ("alacritty", &["-e", "bash", "-c"]),
    ("xfce4-terminal", &["-e", "bash", "-c"]),
];

/// Spawn a command in a new terminal window.
pub fn spawn_in_terminal(exe_path: &std::path::Path, args: &[String]) -> Result<()> {
    let cmd_str = build_command_string(exe_path, args)?;

    for (term_name, term_args) in TERMINAL_EMULATORS.iter() {
        let mut cmd = Command::new(term_name);
        cmd.args(*term_args)
            .arg(&cmd_str)
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        if cmd.spawn().is_ok() {
            return Ok(());
        }
    }

    Err(crate::error::anyhow!("No terminal emulator found"))
}

/// Build a shell command string with proper escaping and a pause at the end.
fn build_command_string(exe_path: &std::path::Path, args: &[String]) -> Result<String> {
    let exe_str = exe_path.to_string_lossy().to_string();
    let args_str = args.iter()
        .map(|s| format!("'{}'", s.replace("'", "'\\''")))
        .collect::<Vec<_>>()
        .join(" ");

    Ok(format!(
        "{} {}; echo; read -p 'Press Enter to close this window...'",
        exe_str, args_str
    ))
}
