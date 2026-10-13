//! The texts of the main window.

use std::time::Duration;

use autosplit_core::runner::{Buttons, ImageReading, ImageRef, ResetReading, Showing, StartStatus, Status};
use autosplit_core::text::window_texts;
use test_case::test_case;

fn status() -> Status {
    Status {
        running: true,
        start_status: StartStatus::Started,
        showing: Showing::Image(ImageRef::Split(2)),
        current_filename: Some("05_bottle.png".into()),
        similarity: Some(0.4123),
        highest: Some(0.951),
        threshold: Some(0.93),
        loop_position: Some((1, 2)),
        reset_similarity: Some(ResetReading::Similarity(0.1234)),
        reset_highest: Some(0.5),
        reset_threshold: Some(0.96),
        split_position: Some((3, 12)),
        below: false,
        group: Vec::new(),
        buttons: Buttons {
            start: false,
            reset: true,
            skip_split: true,
            undo_split: true,
            next_image: true,
            previous_image: true,
        },
    }
}

const NOW: Duration = Duration::from_secs(10);

#[test_case(status(), true => ["0.412", "0.951", "0.930", "0.123", "0.500", "0.960"]; "comparing")]
#[test_case(status(), false => ["-", "0.951", "0.930", "-", "0.500", "0.960"]; "without a capture")]
#[test_case(Status { reset_similarity: Some(ResetReading::Disabled), ..status() }, true => ["0.412", "0.951", "0.930", "disabled", "0.500", "0.960"]; "auto reset off")]
#[test_case(Status { reset_similarity: Some(ResetReading::Paused), ..status() }, true => ["0.412", "0.951", "0.930", "paused", "0.500", "0.960"]; "reset paused")]
#[test_case(Status { reset_similarity: Some(ResetReading::NotApplicable), ..status() }, true => ["0.412", "0.951", "0.930", "N/A", "N/A", "N/A"]; "no reset image")]
#[test_case(Status { reset_similarity: None, ..status() }, true => ["0.412", "0.951", "0.930", "-", "-", "-"]; "before the run")]
fn similarity_table(status: Status, capture: bool) -> [String; 6] {
    // Act, with the expected value checked by test_case
    window_texts(&status, NOW, "", capture, true).table
}

#[test_case(status() => ""; "image shown")]
#[test_case(Status { showing: Showing::Nothing, ..status() } => "ready"; "idle")]
#[test_case(Status { showing: Showing::Pause { until: NOW + Duration::from_millis(9900) }, ..status() } => "None (Paused). 9.9 seconds remaining"; "pause")]
#[test_case(Status { showing: Showing::SplitDelay { until: NOW + Duration::from_millis(300) }, ..status() } => "Delayed Split: 0.3 seconds remaining"; "split delay")]
#[test_case(Status { showing: Showing::StartDelay { until: NOW + Duration::from_millis(1000) }, ..status() } => "Delayed Before Starting:\n 1.0 second remaining"; "start delay")]
fn current_message(status: Status) -> String {
    // Act, with the expected value checked by test_case
    window_texts(&status, NOW, "ready", true, true).current_message
}

#[test_case(Status { showing: Showing::Pause { until: NOW + Duration::from_millis(9900) }, ..status() } => "None (Paused)."; "pause")]
#[test_case(Status { showing: Showing::SplitDelay { until: NOW + Duration::from_millis(300) }, ..status() } => "Delayed Split:"; "split delay")]
#[test_case(Status { showing: Showing::StartDelay { until: NOW + Duration::from_millis(1000) }, ..status() } => "Delayed Before Starting:\n 1.0 second remaining"; "start delay keeps its timer")]
fn current_message_without_the_pause_timer(status: Status) -> String {
    // Act, with the expected value checked by test_case
    window_texts(&status, NOW, "ready", true, false).current_message
}

#[test_case(status() => "1/2"; "looping image")]
#[test_case(Status { loop_position: None, ..status() } => "N/A"; "no image")]
fn image_loop(status: Status) -> String {
    // Act, with the expected value checked by test_case
    window_texts(&status, NOW, "", true, true).image_loop
}

#[test_case(status() => "05_bottle.png"; "image")]
#[test_case(Status { current_filename: None, ..status() } => "-"; "none")]
#[test_case(grouped() => "Comparing against 2 images"; "several images")]
#[test_case(Status { current_filename: None, ..grouped() } => "-"; "several images during a split delay")]
fn current_file(status: Status) -> String {
    // Act, with the expected value checked by test_case
    window_texts(&status, NOW, "", true, true).current_file
}

#[test_case(StartStatus::NotFound => "not found"; "not found")]
#[test_case(StartStatus::Ready => "ready"; "ready")]
#[test_case(StartStatus::DelayingStart => "delaying start..."; "delaying")]
#[test_case(StartStatus::Started => "started"; "started")]
#[test_case(StartStatus::NotReady => "not ready"; "not ready")]
fn start_status(start_status: StartStatus) -> String {
    // Act, with the expected value checked by test_case
    window_texts(&Status { start_status, ..status() }, NOW, "", true, true).start_status
}

fn grouped() -> Status {
    let reading = |similarity, highest, threshold| ImageReading { similarity, highest, threshold };
    Status { group: vec![reading(Some(0.4123), Some(0.5), 0.93), reading(Some(0.2), Some(0.8), 0.9)], ..status() }
}

#[test_case(grouped(), true => [["0.412", "0.500", "0.930"], ["0.200", "0.800", "0.900"], ["-", "-", "-"], ["-", "-", "-"]]; "two images")]
#[test_case(grouped(), false => [["-", "0.500", "0.930"], ["-", "0.800", "0.900"], ["-", "-", "-"], ["-", "-", "-"]]; "without a capture")]
#[test_case(status(), true => [["0.412", "0.951", "0.930"], ["-", "-", "-"], ["-", "-", "-"], ["-", "-", "-"]]; "start image")]
fn group_rows(status: Status, capture: bool) -> [[String; 3]; 4] {
    // Act, with the expected value checked by test_case
    window_texts(&status, NOW, "", capture, true).group_rows
}
