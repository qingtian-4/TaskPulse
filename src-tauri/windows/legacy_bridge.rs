//! Compatibility entry for installed Hook and MCP configurations saved before
//! the Windows app rename. It cannot open the GUI or run arbitrary commands.
#![windows_subsystem = "windows"]

use std::ffi::OsString;
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};

fn allowed(args: &[OsString]) -> bool {
    let arg = |index: usize| args.get(index).and_then(|s| s.to_str());
    let value = |index: usize| {
        arg(index).is_some_and(|s| {
            !s.is_empty() && !s.starts_with('-') && !s.chars().any(char::is_control)
        })
    };
    match (arg(0), args.len()) {
        (Some("--claude-event" | "--integration-validate"), 1) => true,
        (Some("--claude-event"), 2) => arg(1) == Some("--taskpulse-hook"),
        (Some("--integration-mcp" | "--integration-diagnose"), 3) => {
            arg(1) == Some("--source") && value(2)
        }
        (Some("--integration-event" | "--integration-host-event"), 5) => {
            arg(1) == Some("--source") && value(2) && arg(3) == Some("--adapter") && value(4)
        }
        _ => false,
    }
}

fn main() {
    let args = std::env::args_os().skip(1).take(6).collect::<Vec<_>>();
    if !allowed(&args) {
        std::process::exit(2);
    }
    let status = std::env::current_exe()
        .and_then(|exe| {
            Command::new(exe.with_file_name("TaskPulse.exe"))
                .args(args)
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit())
                .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
                .status()
        })
        .ok()
        .and_then(|status| status.code())
        .unwrap_or(1);
    std::process::exit(status);
}
