//! LiveSplit One's server protocol.

use autosplit_core::commands::{Command, ControlRequest, TimerReport};
use autosplit_core::livesplit_one::{EchoFilter, Message, Received, describe_error, parse_event, parse_message};
use test_case::test_case;

#[test_case(r#"{"event":"Splitted"}"# => Some(Message::Event("Splitted".into())); "event")]
#[test_case(r#"{"success":null}"# => Some(Message::Success); "success")]
#[test_case(r#"{"success":{"state":"Running","index":2}}"# => Some(Message::Success); "success with a value")]
#[test_case(r#"{"error":{"code":"NoRunInProgress"}}"# => Some(Message::Error("NoRunInProgress".into())); "error")]
#[test_case(r#"{"error":{"code":"InvalidCommand","message":"unknown variant"}}"# => Some(Message::Error("InvalidCommand".into())); "error with a message")]
#[test_case("not json" => None; "not json")]
#[test_case(r#"{"something":"else"}"# => None; "unknown message")]
fn parses_messages(text: &str) -> Option<Message> {
    // Act, with the expected value checked by test_case
    parse_message(text)
}

#[test_case("Started" => TimerReport { command: Some(Command::Start), request: Some(ControlRequest::Start) }; "started starts")]
#[test_case("Splitted" => TimerReport { command: Some(Command::Split), request: Some(ControlRequest::Skip) }; "split moves to the next split")]
#[test_case("Finished" => TimerReport { command: Some(Command::Split), request: Some(ControlRequest::Skip) }; "the last split moves past the end")]
#[test_case("SplitSkipped" => TimerReport { command: Some(Command::Skip), request: Some(ControlRequest::Skip) }; "skip moves to the next split")]
#[test_case("SplitUndone" => TimerReport { command: Some(Command::Undo), request: Some(ControlRequest::Undo) }; "undo goes back")]
#[test_case("Reset" => TimerReport { command: Some(Command::Reset), request: Some(ControlRequest::Reset) }; "reset resets")]
#[test_case("Paused" => TimerReport::default(); "pause is ignored")]
fn maps_events_like_livesplit_lines(event: &str) -> TimerReport {
    // Act, with the expected value checked by test_case
    parse_event(event)
}

#[test]
fn event_without_a_pending_command_is_the_runners() {
    // Arrange
    let mut filter = EchoFilter::default();

    // Act
    let received = filter.receive(Message::Event("Splitted".into()));

    // Assert
    assert_eq!(received, Received::RunnerEvent("Splitted".into()));
}

#[test]
fn event_before_a_commands_answer_is_an_echo() {
    // Arrange
    let mut filter = EchoFilter::default();
    filter.sent(Command::Split);

    // Act
    let received = filter.receive(Message::Event("Splitted".into()));

    // Assert
    assert_eq!(received, Received::Nothing);
}

#[test_case(Message::Success; "success")]
#[test_case(Message::Error("RunFinished".into()); "error")]
fn event_after_the_answer_is_the_runners(answer: Message) {
    // Arrange
    let mut filter = EchoFilter::default();
    filter.sent(Command::Split);
    filter.receive(Message::Event("Splitted".into()));
    filter.receive(answer);

    // Act
    let received = filter.receive(Message::Event("SplitUndone".into()));

    // Assert
    assert_eq!(received, Received::RunnerEvent("SplitUndone".into()));
}

#[test]
fn every_command_needs_its_own_answer() {
    // Arrange
    let mut filter = EchoFilter::default();
    filter.sent(Command::Reset);
    filter.sent(Command::Start);
    filter.receive(Message::Success);

    // Act
    let received = filter.receive(Message::Event("Started".into()));

    // Assert
    assert_eq!(received, Received::Nothing);
}

#[test]
fn cleared_filter_forgets_unanswered_commands() {
    // Arrange
    let mut filter = EchoFilter::default();
    filter.sent(Command::Split);
    filter.clear();

    // Act
    let received = filter.receive(Message::Event("Reset".into()));

    // Assert
    assert_eq!(received, Received::RunnerEvent("Reset".into()));
}

#[test]
fn failed_command_is_reported_with_its_command() {
    // Arrange
    let mut filter = EchoFilter::default();
    filter.sent(Command::Split);

    // Act
    let received = filter.receive(Message::Error("NoRunInProgress".into()));

    // Assert
    assert_eq!(received, Received::Failed { command: Command::Split, code: "NoRunInProgress".into() });
}

#[test]
fn reset_of_a_stopped_timer_is_not_a_failure() {
    // Arrange
    let mut filter = EchoFilter::default();
    filter.sent(Command::Reset);

    // Act
    let received = filter.receive(Message::Error("NoRunInProgress".into()));

    // Assert
    assert_eq!(received, Received::Nothing);
}

#[test_case("NoRunInProgress" => "the timer isn't running"; "known code")]
#[test_case("SomethingNew" => "error SomethingNew"; "unknown code")]
fn describes_errors(code: &str) -> String {
    // Act, with the expected value checked by test_case
    describe_error(code)
}

#[test_case(None => true; "no origin like the desktop app")]
#[test_case(Some("https://one.livesplit.org") => true; "web app")]
#[test_case(Some("http://one.livesplit.org") => false; "web app without https")]
#[test_case(Some("tauri://localhost") => true; "desktop app")]
#[test_case(Some("https://tauri.localhost") => true; "desktop app on windows")]
#[test_case(Some("http://localhost:5173") => true; "local development server")]
#[test_case(Some("http://[::1]:8080") => true; "ipv6 localhost")]
#[test_case(Some("https://evil.example") => false; "other web page")]
#[test_case(Some("https://one.livesplit.org.evil.example") => false; "look-alike host")]
#[test_case(Some("null") => false; "opaque origin")]
fn allows_origins(origin: Option<&str>) -> bool {
    // Act, with the expected value checked by test_case
    autosplit_core::livesplit_one::origin_allowed(origin)
}
