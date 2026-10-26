//! The LiveSplit `--auto-controlled` protocol, matching AutoSplit's `AutoControlledThread.py`.

use autosplit_core::auto_controlled::{decode_line, handshake_lines, parse_control_line};
use autosplit_core::commands::Command;
use autosplit_core::commands::ControlRequest;
use autosplit_core::profile::UserProfile;
use test_case::test_case;

#[test_case("split" => Some(ControlRequest::Skip); "split moves to the next image")]
#[test_case("skip" => Some(ControlRequest::Skip); "skip moves to the next image")]
#[test_case("start" => Some(ControlRequest::Start); "start")]
#[test_case("undo" => Some(ControlRequest::Undo); "undo")]
#[test_case("reset" => Some(ControlRequest::Reset); "reset")]
#[test_case("kill" => Some(ControlRequest::Kill); "kill")]
#[test_case("pause" => None; "pause is ignored like in AutoSplit")]
#[test_case("settings|C:/splits/oot.toml" => Some(ControlRequest::LoadSettings("C:/splits/oot.toml".into())); "settings loads the path after any separator")]
#[test_case("settings" => Some(ControlRequest::LoadSettings(String::new())); "settings without a path")]
fn request(line: &str) -> Option<ControlRequest> {
    // Act, with the expected value checked by test_case
    parse_control_line(line).request
}

#[test_case("split", vec![Command::Split] => true; "line in screenshot on")]
#[test_case("split", vec![Command::Reset] => false; "line not in screenshot on")]
#[test_case("settings", vec![Command::Split] => false; "line that isn't a command")]
fn wants_screenshot(line: &str, screenshot_on: Vec<Command>) -> bool {
    // Arrange
    let profile = UserProfile { screenshot_on, ..UserProfile::default() };

    // Act
    let line = parse_control_line(line);

    // Assert, with the expected value checked by test_case
    line.wants_screenshot(&profile)
}

#[test]
fn handshake_is_the_version_then_the_process_id() {
    // Act
    let lines = handshake_lines("2.3.2");

    // Assert
    assert_eq!(lines, ["2.3.2".to_owned(), std::process::id().to_string()]);
}

#[test_case(b"split\n" => "split"; "newline is dropped")]
#[test_case(b"split\r\n" => "split"; "windows line ending is dropped")]
#[test_case("settings|C:/Users/Jos\u{e9}/a.toml\r\n".as_bytes() => "settings|C:/Users/Jos\u{e9}/a.toml"; "utf-8 is read as utf-8")]
#[test_case(b"settings|C:/Users/Jos\xe9/a.toml\r\n" => "settings|C:/Users/Jos\u{e9}/a.toml"; "other bytes go to the fallback")]
fn decodes_lines(bytes: &[u8]) -> String {
    // Arrange
    let latin1 = |bytes: &[u8]| bytes.iter().map(|&byte| char::from(byte)).collect();

    // Act, with the expected value checked by test_case
    decode_line(bytes, latin1)
}
