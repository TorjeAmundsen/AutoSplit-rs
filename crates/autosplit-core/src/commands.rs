//! Timer commands and how they are sent, matching AutoSplit's `hotkeys.send_command`.

use serde::{Deserialize, Serialize};

use crate::profile::UserProfile;

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

/// One thing to do to deliver a command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Output {
    /// Take a screenshot of the capture region (the profile's "screenshot on" option).
    Screenshot,
    /// Press this hotkey, in AutoSplit's hotkey name format.
    PressHotkey(String),
    /// Print this line to stdout for LiveSplit.
    PrintLine(&'static str),
}

/// `send_command`: what to do for `command`, in order.
pub fn outputs_for(command: Command, profile: &UserProfile, auto_controlled: bool) -> Vec<Output> {
    let mut outputs = Vec::new();
    if profile.screenshot_on.contains(&command) {
        outputs.push(Output::Screenshot);
    }
    if auto_controlled {
        if command == Command::Start && profile.start_also_resets {
            outputs.push(Output::PrintLine(Command::Reset.name()));
        }
        outputs.push(Output::PrintLine(command.name()));
        return outputs;
    }
    let hotkey = match command {
        Command::Start if profile.start_also_resets => &profile.reset_hotkey,
        Command::Reset => &profile.reset_hotkey,
        Command::Start | Command::Split => &profile.split_hotkey,
        Command::Pause => &profile.pause_hotkey,
        Command::Skip => &profile.skip_split_hotkey,
        Command::Undo => &profile.undo_split_hotkey,
    };
    if !hotkey.is_empty() {
        outputs.push(Output::PressHotkey(hotkey.clone()));
    }
    outputs
}
