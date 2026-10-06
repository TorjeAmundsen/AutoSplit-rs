//! LiveSplit One's server protocol (livesplit-core's `networking::server_protocol`). LiveSplit
//! One connects to AutoSplit as a WebSocket client. AutoSplit sends it commands, and it answers
//! each one and reports every change to its timer as an event.
//!
//! Events caused by AutoSplit's own commands arrive before that command's answer, which is how
//! they're told apart from what the runner did in LiveSplit One.

use std::collections::VecDeque;

use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::commands::{Command, ControlRequest, TimerReport};

/// The message that sends `command`.
pub fn message_for(command: Command) -> &'static str {
    match command {
        Command::Split => r#"{"command":"split"}"#,
        Command::Start => r#"{"command":"start"}"#,
        // LiveSplit's pause also resumes, and starts a stopped timer
        Command::Pause => r#"{"command":"togglePauseOrStart"}"#,
        // Saved without asking, like LiveSplit resets for AutoSplit. Asking would leave the
        // timer running, out of sync, until the runner answers.
        Command::Reset => r#"{"command":"reset","saveAttempt":true}"#,
        Command::Skip => r#"{"command":"skipSplit"}"#,
        Command::Undo => r#"{"command":"undoSplit"}"#,
    }
}

/// A message from LiveSplit One.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    /// Something changed in LiveSplit One's timer.
    Event(String),
    /// A command was done.
    Success,
    /// A command couldn't be done, with LiveSplit One's error code, like `NoRunInProgress`.
    Error(String),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum Raw {
    Event(String),
    Success(IgnoredAny),
    Error(RawError),
}

#[derive(Deserialize)]
struct RawError {
    code: String,
}

/// `None` for anything that isn't a message LiveSplit One sends.
pub fn parse_message(text: &str) -> Option<Message> {
    Some(match serde_json::from_str::<Raw>(text).ok()? {
        Raw::Event(event) => Message::Event(event),
        Raw::Success(IgnoredAny) => Message::Success,
        Raw::Error(error) => Message::Error(error.code),
    })
}

/// What a LiveSplit One event asks of AutoSplit, the same as the matching line from LiveSplit
/// in `--auto-controlled` mode. Pauses and other changes don't concern AutoSplit.
pub fn parse_event(event: &str) -> TimerReport {
    let (command, request) = match event {
        "Started" => (Command::Start, ControlRequest::Start),
        // The last split is reported as "Finished"
        "Splitted" | "Finished" => (Command::Split, ControlRequest::Skip),
        "SplitSkipped" => (Command::Skip, ControlRequest::Skip),
        "SplitUndone" => (Command::Undo, ControlRequest::Undo),
        "Reset" => (Command::Reset, ControlRequest::Reset),
        _ => return TimerReport::default(),
    };
    TimerReport { command: Some(command), request: Some(request) }
}

/// An event in words, for the log.
pub fn describe_event(event: &str) -> &str {
    match event {
        "Started" => "started",
        "Splitted" => "split",
        "Finished" => "split the last split",
        "SplitSkipped" => "skipped a split",
        "SplitUndone" => "undid a split",
        "Reset" => "reset",
        other => other,
    }
}

/// Why LiveSplit One didn't do a command, in words.
pub fn describe_error(code: &str) -> String {
    match code {
        "NoRunInProgress" => "the timer isn't running".into(),
        "RunAlreadyInProgress" => "the timer is already running".into(),
        "RunFinished" => "the run is already finished".into(),
        "CantSkipLastSplit" => "the last split can't be skipped".into(),
        "CantUndoFirstSplit" => "there's no split to undo".into(),
        "NegativeTime" => "the timer is below zero".into(),
        "TimerPaused" => "the timer is paused".into(),
        "RunnerDecidedAgainstReset" => "the reset was cancelled".into(),
        "Busy" => "LiveSplit One is busy".into(),
        other => format!("error {other}"),
    }
}

/// What a received message means for AutoSplit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Received {
    /// The runner changed the timer in LiveSplit One.
    RunnerEvent(String),
    /// LiveSplit One didn't do a command AutoSplit sent.
    Failed { command: Command, code: String },
    /// An echo of AutoSplit's own command, or an answer that needs no action.
    Nothing,
}

/// Tells LiveSplit One's own changes apart from echoes of AutoSplit's commands.
#[derive(Clone, Debug, Default)]
pub struct EchoFilter {
    /// Commands sent and not answered yet, oldest first.
    unanswered: VecDeque<Command>,
}

impl EchoFilter {
    pub fn sent(&mut self, command: Command) {
        self.unanswered.push_back(command);
    }

    /// The connection was lost or replaced, so no answers are coming.
    pub fn clear(&mut self) {
        self.unanswered.clear();
    }

    pub fn receive(&mut self, message: Message) -> Received {
        match message {
            Message::Event(event) if self.unanswered.is_empty() => Received::RunnerEvent(event),
            Message::Event(_) => Received::Nothing,
            Message::Success => {
                self.unanswered.pop_front();
                Received::Nothing
            }
            Message::Error(code) => match self.unanswered.pop_front() {
                // "Start also resets" resets first, which a stopped timer refuses
                Some(Command::Reset) if code == "NoRunInProgress" => Received::Nothing,
                Some(command) => Received::Failed { command, code },
                None => Received::Nothing,
            },
        }
    }
}

/// Whether a WebSocket connection with this `Origin` header may connect: LiveSplit One's web
/// app, its desktop app (no origin, or a localhost origin) and pages served from this computer.
/// Other web pages open in the browser could otherwise drive AutoSplit.
pub fn origin_allowed(origin: Option<&str>) -> bool {
    let Some(origin) = origin else { return true };
    let Some((scheme, rest)) = origin.split_once("://") else { return false };
    let host = match rest.strip_prefix('[') {
        Some(ipv6) => ipv6.split(']').next().unwrap_or(""),
        None => rest.split([':', '/']).next().unwrap_or(""),
    };
    match host {
        "one.livesplit.org" => scheme == "https",
        "localhost" | "127.0.0.1" | "::1" | "tauri.localhost" => true,
        _ => false,
    }
}
