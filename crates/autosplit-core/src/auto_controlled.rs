//! The `--auto-controlled` protocol of the LiveSplit AutoSplit Integration component, matching
//! AutoSplit's `AutoControlledThread.py`. LiveSplit writes requests to stdin, and AutoSplit
//! prints commands to stdout (see [`crate::commands::outputs_for`]).

use crate::commands::Command;
use crate::profile::UserProfile;

/// A request read from stdin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ControlRequest {
    Start,
    /// LiveSplit sends "split" and "skip" when the user splits or skips manually.
    /// Both move AutoSplit to the next split, like the "Skip Split" button.
    Skip,
    Undo,
    Reset,
    /// Load this profile. Empty when nothing followed "settings".
    LoadSettings(String),
    /// Close AutoSplit. Used in development.
    Kill,
}

/// What a stdin line asks for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ControlLine {
    /// The line read as a command name, for the profile's "screenshot on" option.
    pub command: Option<Command>,
    pub request: Option<ControlRequest>,
}

impl ControlLine {
    /// AutoSplit takes a screenshot when the line names a command in `screenshot_on`, on top of
    /// handling the request.
    pub fn wants_screenshot(&self, profile: &UserProfile) -> bool {
        self.command.is_some_and(|command| profile.screenshot_on.contains(&command))
    }
}

pub fn parse_control_line(line: &str) -> ControlLine {
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
    ControlLine { command: Command::from_name(line), request }
}

/// The first two lines printed when auto-controlled, which LiveSplit reads before anything
/// else: the version, then the process ID.
pub fn handshake_lines(version: &str) -> [String; 2] {
    [version.to_owned(), std::process::id().to_string()]
}
