//! What the profile's hotkeys do, matching AutoSplit's `hotkeys.py`.

use std::time::Duration;

use crate::commands::{Command, TimerLink};
use crate::profile::UserProfile;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HotkeyAction {
    /// Starts the run when it isn't running. While running, the timer splits by itself.
    Split,
    Reset,
    /// Moves to the next image only.
    SkipSplit,
    /// Moves to the previous image only.
    UndoSplit,
    /// Does nothing in AutoSplit either, but is listened for so it keeps its keys.
    Pause,
    Screenshot,
    ToggleAutoResetImage,
}

impl HotkeyAction {
    pub const ALL: [HotkeyAction; 7] = [
        Self::Split,
        Self::Reset,
        Self::SkipSplit,
        Self::UndoSplit,
        Self::Pause,
        Self::Screenshot,
        Self::ToggleAutoResetImage,
    ];

    /// The hotkeys AutoSplit presses for the timer.
    pub const SENT: [HotkeyAction; 5] = [Self::Split, Self::Reset, Self::SkipSplit, Self::UndoSplit, Self::Pause];

    pub fn hotkey(self, profile: &UserProfile) -> &str {
        match self {
            Self::Split => &profile.split_hotkey,
            Self::Reset => &profile.reset_hotkey,
            Self::SkipSplit => &profile.skip_split_hotkey,
            Self::UndoSplit => &profile.undo_split_hotkey,
            Self::Pause => &profile.pause_hotkey,
            Self::Screenshot => &profile.screenshot_hotkey,
            Self::ToggleAutoResetImage => &profile.toggle_auto_reset_image_hotkey,
        }
    }

    pub fn hotkey_mut(self, profile: &mut UserProfile) -> &mut String {
        match self {
            Self::Split => &mut profile.split_hotkey,
            Self::Reset => &mut profile.reset_hotkey,
            Self::SkipSplit => &mut profile.skip_split_hotkey,
            Self::UndoSplit => &mut profile.undo_split_hotkey,
            Self::Pause => &mut profile.pause_hotkey,
            Self::Screenshot => &mut profile.screenshot_hotkey,
            Self::ToggleAutoResetImage => &mut profile.toggle_auto_reset_image_hotkey,
        }
    }

    /// Sets this action's hotkey, `""` to clear it. Like AutoSplit, a hotkey belongs to one
    /// action, so another action with the same hotkey loses it; `same` tells whether two names
    /// are the same keys.
    pub fn assign(self, profile: &mut UserProfile, hotkey: &str, same: impl Fn(&str, &str) -> bool) {
        if !hotkey.is_empty() {
            for other in Self::ALL {
                if other != self && same(other.hotkey(profile), hotkey) {
                    other.hotkey_mut(profile).clear();
                }
            }
        }
        *self.hotkey_mut(profile) = hotkey.to_owned();
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Split => "split",
            Self::Reset => "reset",
            Self::SkipSplit => "skip split",
            Self::UndoSplit => "undo split",
            Self::Pause => "pause",
            Self::Screenshot => "screenshot",
            Self::ToggleAutoResetImage => "toggle auto reset image",
        }
    }

    /// When the timer is synced, only the screenshot and auto reset hotkeys are listened for,
    /// like AutoSplit's settings window allows when auto-controlled.
    pub fn listened(self, link: TimerLink) -> bool {
        !link.timer_synced() || matches!(self, Self::Screenshot | Self::ToggleAutoResetImage)
    }
}

/// The hotkey that sends a command, like AutoSplit's `send_command`.
impl From<Command> for HotkeyAction {
    fn from(command: Command) -> Self {
        match command {
            Command::Start | Command::Split => Self::Split,
            Command::Reset => Self::Reset,
            Command::Skip => Self::SkipSplit,
            Command::Undo => Self::UndoSplit,
            Command::Pause => Self::Pause,
        }
    }
}

/// How long a hotkey AutoSplit pressed itself is ignored by its own listener.
pub const OWN_PRESS_WINDOW: Duration = Duration::from_millis(500);

/// Hotkeys AutoSplit pressed itself also reach its own hotkey listener. Ignoring them for a
/// moment keeps the reset hotkey of "start also resets" from resetting the new run, and the last
/// split from starting another one.
#[derive(Clone, Debug, Default)]
pub struct OwnPresses {
    pressed: Vec<(String, Duration)>,
}

impl OwnPresses {
    pub fn pressed(&mut self, hotkey: &str, now: Duration) {
        self.pressed.push((hotkey.to_owned(), now));
    }

    /// Whether a press of `hotkey` at `now` is AutoSplit's own.
    pub fn is_own(&mut self, hotkey: &str, now: Duration) -> bool {
        self.pressed.retain(|(_, at)| now.saturating_sub(*at) < OWN_PRESS_WINDOW);
        self.pressed.iter().any(|(pressed, _)| pressed == hotkey)
    }
}
