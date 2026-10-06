//! The `--auto-controlled` protocol of the LiveSplit AutoSplit Integration component, matching
//! AutoSplit's `AutoControlledThread.py`. LiveSplit writes requests to stdin, and AutoSplit
//! prints commands to stdout (see [`crate::commands::outputs_for`]).

use crate::commands::{Command, ControlRequest, TimerReport};

pub fn parse_control_line(line: &str) -> TimerReport {
    let request = match line {
        "kill" => Some(ControlRequest::Kill),
        "start" => Some(ControlRequest::Start),
        "split" | "skip" => Some(ControlRequest::Skip),
        "undo" => Some(ControlRequest::Undo),
        "reset" => Some(ControlRequest::Reset),
        // Any separator is allowed between "settings" and the path
        _ if line.starts_with("settings") => Some(ControlRequest::LoadSettings(line.chars().skip(9).collect())),
        _ => None,
    };
    TimerReport { command: Command::from_name(line), request }
}

/// Decodes a line from LiveSplit. LiveSplit writes in the system's ANSI code page (it never sets
/// an encoding for AutoSplit's stdin), so anything that isn't UTF-8 is decoded with `fallback`.
pub fn decode_line(bytes: &[u8], fallback: impl FnOnce(&[u8]) -> String) -> String {
    let bytes = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    let bytes = bytes.strip_suffix(b"\r").unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(line) => line.to_owned(),
        Err(_) => fallback(bytes),
    }
}

/// The first two lines printed when auto-controlled, which LiveSplit reads before anything
/// else: the version, then the process ID.
pub fn handshake_lines(version: &str) -> [String; 2] {
    [version.to_owned(), std::process::id().to_string()]
}
