//! Timer commands and how they are sent, matching AutoSplit's `hotkeys.send_command`, plus
//! LiveSplit One and Molok.

use serde::{Deserialize, Serialize};

use crate::hotkeys::HotkeyAction;
use crate::livesplit_one;
#[cfg(target_os = "linux")]
use crate::molok;
use crate::profile::{TimerConnection, UserProfile};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Command {
    Split,
    Start,
    Pause,
    Reset,
    Skip,
    Undo,
}

impl Command {
    pub const ALL: [Command; 6] = [Self::Split, Self::Start, Self::Pause, Self::Reset, Self::Skip, Self::Undo];

    pub fn name(self) -> &'static str {
        match self {
            Self::Split => "split",
            Self::Start => "start",
            Self::Pause => "pause",
            Self::Reset => "reset",
            Self::Skip => "skip",
            Self::Undo => "undo",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|command| command.name() == name)
    }
}

/// What the timer (or LiveSplit's stdin) asks AutoSplit to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ControlRequest {
    Start,
    /// The timer split or skipped. Both move AutoSplit to the next split, like the "Skip Split"
    /// button.
    Skip,
    Undo,
    Reset,
    /// Load this profile. Empty when nothing followed "settings".
    LoadSettings(String),
    /// Close AutoSplit. Used in development.
    Kill,
}

/// Something the timer reported: a line from LiveSplit or an event from LiveSplit One.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TimerReport {
    /// The report read as a command name, for the profile's "screenshot on" option.
    pub command: Option<Command>,
    pub request: Option<ControlRequest>,
}

impl TimerReport {
    /// AutoSplit takes a screenshot when the report names a command in `screenshot_on`, on top
    /// of handling the request.
    pub fn wants_screenshot(&self, profile: &UserProfile) -> bool {
        self.command.is_some_and(|command| profile.screenshot_on.contains(&command))
    }
}

/// One thing to do to deliver a command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Output {
    /// Take a screenshot of the capture region (the profile's "screenshot on" option).
    Screenshot,
    /// Press this hotkey, in AutoSplit's hotkey name format.
    PressHotkey(String),
    /// Print this line to stdout for LiveSplit.
    PrintLine(&'static str),
    /// Send this message to LiveSplit One.
    LiveSplitOne(&'static str),
    /// Send Molok the signal `SIGRTMIN` plus this.
    #[cfg(target_os = "linux")]
    Molok(i32),
}

/// Where commands go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimerLink {
    Hotkeys,
    /// Started by LiveSplit's AutoSplit Integration component with `--auto-controlled`.
    AutoControlled,
    LiveSplitOne,
    #[cfg(target_os = "linux")]
    Molok,
}

impl From<TimerConnection> for TimerLink {
    fn from(connection: TimerConnection) -> Self {
        match connection {
            TimerConnection::Hotkeys => Self::Hotkeys,
            TimerConnection::LiveSplitOne => Self::LiveSplitOne,
            #[cfg(target_os = "linux")]
            TimerConnection::Molok => Self::Molok,
            // Molok only runs on Linux, so a profile from there uses hotkeys elsewhere
            #[cfg(not(target_os = "linux"))]
            TimerConnection::Molok => Self::Hotkeys,
        }
    }
}

impl TimerLink {
    /// The timer tells AutoSplit about its splits, skips, undos and resets, so AutoSplit follows
    /// it instead of needing hotkeys.
    pub fn timer_synced(self) -> bool {
        matches!(self, Self::AutoControlled | Self::LiveSplitOne)
    }

    /// The timer only gets commands through the profile's hotkeys, so the ones a split image
    /// folder uses must be set.
    pub fn needs_hotkeys(self) -> bool {
        self == Self::Hotkeys
    }
}

/// `send_command`: what to do for `command`, in order.
pub fn outputs_for(command: Command, profile: &UserProfile, link: TimerLink) -> Vec<Output> {
    let mut outputs = Vec::new();
    if profile.screenshot_on.contains(&command) {
        outputs.push(Output::Screenshot);
    }
    // A synced timer did the skip or undo itself
    if link.timer_synced() && matches!(command, Command::Skip | Command::Undo) {
        return outputs;
    }
    let sent: &[Command] = if command == Command::Start && profile.start_also_resets {
        &[Command::Reset, Command::Start]
    } else {
        &[command]
    };
    match link {
        TimerLink::AutoControlled => outputs.extend(sent.iter().map(|command| Output::PrintLine(command.name()))),
        TimerLink::LiveSplitOne => {
            outputs.extend(sent.iter().map(|&command| Output::LiveSplitOne(livesplit_one::message_for(command))))
        }
        #[cfg(target_os = "linux")]
        TimerLink::Molok => outputs.extend(sent.iter().map(|&command| Output::Molok(molok::signal_for(command)))),
        TimerLink::Hotkeys => {
            // Only the reset hotkey for "start also resets": the first matching case in AutoSplit
            let hotkey = HotkeyAction::from(sent[0]).hotkey(profile);
            if !hotkey.is_empty() {
                outputs.push(Output::PressHotkey(hotkey.to_owned()));
            }
        }
    }
    outputs
}
