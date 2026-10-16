//! Sending commands to the timer, matching AutoSplit's `send_command`.

use autosplit_core::commands::{Command, Output, TimerLink, outputs_for};
use autosplit_core::profile::TimerConnection;
use autosplit_core::profile::UserProfile;
use test_case::test_case;

fn profile() -> UserProfile {
    UserProfile {
        split_hotkey: "num 1".into(),
        reset_hotkey: "num 3".into(),
        undo_split_hotkey: "num 8".into(),
        skip_split_hotkey: "num 2".into(),
        pause_hotkey: "num 5".into(),
        ..UserProfile::default()
    }
}

fn press(hotkey: &str) -> Vec<Output> {
    vec![Output::PressHotkey(hotkey.into())]
}

#[test_case(Command::Split, profile() => press("num 1"); "split presses the split hotkey")]
#[test_case(Command::Start, profile() => press("num 1"); "start presses the split hotkey")]
#[test_case(Command::Start, UserProfile { start_also_resets: true, ..profile() } => vec![Output::PressHotkey("num 3".into()), Output::PressHotkey("num 1".into())]; "start presses the reset hotkey then the split hotkey when start also resets")]
#[test_case(Command::Reset, profile() => press("num 3"); "reset presses the reset hotkey")]
#[test_case(Command::Pause, profile() => press("num 5"); "pause presses the pause hotkey")]
#[test_case(Command::Skip, profile() => press("num 2"); "skip presses the skip hotkey")]
#[test_case(Command::Undo, profile() => press("num 8"); "undo presses the undo hotkey")]
#[test_case(Command::Undo, UserProfile { undo_split_hotkey: String::new(), ..profile() } => Vec::<Output>::new(); "unset hotkey presses nothing")]
#[test_case(Command::Split, UserProfile { screenshot_on: vec![Command::Split], ..profile() } => vec![Output::Screenshot, Output::PressHotkey("num 1".into())]; "screenshot on takes a screenshot before sending")]
fn hotkey_outputs(command: Command, profile: UserProfile) -> Vec<Output> {
    // Act, with the expected value checked by test_case
    outputs_for(command, &profile, TimerLink::Hotkeys)
}

#[test_case(Command::Split, profile() => vec![Output::PrintLine("split")]; "prints the command")]
#[test_case(Command::Skip, profile() => Vec::<Output>::new(); "skip isn't printed back")]
#[test_case(Command::Undo, UserProfile { screenshot_on: vec![Command::Undo], ..profile() } => vec![Output::Screenshot]; "undo still takes its screenshot")]
#[test_case(Command::Start, UserProfile { start_also_resets: true, ..profile() } => vec![Output::PrintLine("reset"), Output::PrintLine("start")]; "start prints reset first when start also resets")]
fn auto_controlled_outputs(command: Command, profile: UserProfile) -> Vec<Output> {
    // Act, with the expected value checked by test_case
    outputs_for(command, &profile, TimerLink::AutoControlled)
}

#[test_case(Command::Split, profile() => vec![Output::LiveSplitOne(r#"{"command":"split"}"#)]; "split sends split")]
#[test_case(Command::Start, profile() => vec![Output::LiveSplitOne(r#"{"command":"start"}"#)]; "start sends start")]
#[test_case(Command::Start, UserProfile { start_also_resets: true, ..profile() } => vec![Output::LiveSplitOne(r#"{"command":"reset","saveAttempt":true}"#), Output::LiveSplitOne(r#"{"command":"start"}"#)]; "start sends reset first when start also resets")]
#[test_case(Command::Pause, profile() => vec![Output::LiveSplitOne(r#"{"command":"togglePauseOrStart"}"#)]; "pause toggles pause like LiveSplit")]
#[test_case(Command::Reset, profile() => vec![Output::LiveSplitOne(r#"{"command":"reset","saveAttempt":true}"#)]; "reset sends reset")]
#[test_case(Command::Skip, profile() => Vec::<Output>::new(); "skip isn't sent back")]
#[test_case(Command::Skip, UserProfile { screenshot_on: vec![Command::Skip], ..profile() } => vec![Output::Screenshot]; "skip still takes its screenshot")]
#[test_case(Command::Reset, UserProfile { screenshot_on: vec![Command::Reset], ..profile() } => vec![Output::Screenshot, Output::LiveSplitOne(r#"{"command":"reset","saveAttempt":true}"#)]; "screenshot on takes a screenshot before sending to LiveSplit One")]
fn livesplit_one_outputs(command: Command, profile: UserProfile) -> Vec<Output> {
    // Act, with the expected value checked by test_case
    outputs_for(command, &profile, TimerLink::LiveSplitOne)
}

#[cfg(target_os = "linux")]
#[test_case(Command::Split, profile() => vec![Output::Molok(0)]; "split sends SIGRTMIN")]
#[test_case(Command::Start, profile() => vec![Output::Molok(0)]; "start sends SIGRTMIN")]
#[test_case(Command::Start, UserProfile { start_also_resets: true, ..profile() } => vec![Output::Molok(3), Output::Molok(0)]; "start sends reset first when start also resets")]
#[test_case(Command::Skip, profile() => vec![Output::Molok(1)]; "skip sends SIGRTMIN+1")]
#[test_case(Command::Undo, profile() => vec![Output::Molok(2)]; "undo sends SIGRTMIN+2")]
#[test_case(Command::Reset, profile() => vec![Output::Molok(3)]; "reset sends SIGRTMIN+3")]
#[test_case(Command::Pause, profile() => vec![Output::Molok(4)]; "pause sends SIGRTMIN+4")]
#[test_case(Command::Split, UserProfile { split_hotkey: String::new(), ..profile() } => vec![Output::Molok(0)]; "needs no hotkey")]
#[test_case(Command::Reset, UserProfile { screenshot_on: vec![Command::Reset], ..profile() } => vec![Output::Screenshot, Output::Molok(3)]; "screenshot on takes a screenshot before signaling")]
fn molok_outputs(command: Command, profile: UserProfile) -> Vec<Output> {
    // Act, with the expected value checked by test_case
    outputs_for(command, &profile, TimerLink::Molok)
}

#[test_case(TimerConnection::Hotkeys => TimerLink::Hotkeys; "hotkeys")]
#[test_case(TimerConnection::LiveSplitOne => TimerLink::LiveSplitOne; "livesplit one")]
#[cfg_attr(target_os = "linux", test_case(TimerConnection::Molok => TimerLink::Molok; "molok"))]
#[cfg_attr(not(target_os = "linux"), test_case(TimerConnection::Molok => TimerLink::Hotkeys; "molok uses hotkeys off linux"))]
fn timer_link_from_profile(connection: TimerConnection) -> TimerLink {
    // Act, with the expected value checked by test_case
    TimerLink::from(connection)
}
