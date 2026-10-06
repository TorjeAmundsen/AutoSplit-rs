//! What hotkeys do and when AutoSplit's own presses are ignored.

use std::time::Duration;

use autosplit_core::commands::{Command, TimerLink};
use autosplit_core::hotkeys::{HotkeyAction, OwnPresses};
use test_case::test_case;

#[test_case(HotkeyAction::Split, TimerLink::Hotkeys => true; "split with hotkeys")]
#[test_case(HotkeyAction::Split, TimerLink::AutoControlled => false; "split when auto controlled")]
#[test_case(HotkeyAction::Reset, TimerLink::LiveSplitOne => false; "reset with livesplit one")]
#[test_case(HotkeyAction::Screenshot, TimerLink::AutoControlled => true; "screenshot when auto controlled")]
#[test_case(HotkeyAction::ToggleAutoResetImage, TimerLink::LiveSplitOne => true; "toggle auto reset with livesplit one")]
fn listened(action: HotkeyAction, link: TimerLink) -> bool {
    // Act, with the expected value checked by test_case
    action.listened(link)
}

#[test_case("num 1", Duration::from_millis(100) => true; "same hotkey right after")]
#[test_case("num 1", Duration::from_millis(600) => false; "same hotkey later")]
#[test_case("num 3", Duration::from_millis(100) => false; "other hotkey")]
fn own_presses(hotkey: &str, after: Duration) -> bool {
    // Arrange
    let mut presses = OwnPresses::default();
    presses.pressed("num 1", Duration::from_secs(1));

    // Act, with the expected value checked by test_case
    presses.is_own(hotkey, Duration::from_secs(1) + after)
}

#[test_case(Command::Start => HotkeyAction::Split; "start presses split")]
#[test_case(Command::Split => HotkeyAction::Split; "split")]
#[test_case(Command::Reset => HotkeyAction::Reset; "reset")]
#[test_case(Command::Skip => HotkeyAction::SkipSplit; "skip")]
#[test_case(Command::Undo => HotkeyAction::UndoSplit; "undo")]
#[test_case(Command::Pause => HotkeyAction::Pause; "pause")]
fn command_hotkeys(command: Command) -> HotkeyAction {
    // Act, with the expected value checked by test_case
    HotkeyAction::from(command)
}
