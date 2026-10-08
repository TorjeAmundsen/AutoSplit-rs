//! The run's one-line status.

use std::time::Duration;

use autosplit_core::runner::{Buttons, ImageRef, ResetReading, Showing, StartStatus, Status};
use autosplit_core::text::status_line;
use test_case::test_case;

fn status() -> Status {
    Status {
        running: true,
        start_status: StartStatus::NotReady,
        showing: Showing::Image(ImageRef::Split(2)),
        current_filename: Some("05_bottle.png".into()),
        similarity: Some(0.4123),
        highest: Some(0.951),
        threshold: Some(0.93),
        loop_position: Some((1, 1)),
        reset_similarity: None,
        reset_highest: None,
        reset_threshold: None,
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

#[test_case(status() => "split 3/12  0.412 / 0.930 (best 0.951)  05_bottle.png"; "split image")]
#[test_case(Status { loop_position: Some((1, 2)), ..status() } => "split 3/12  0.412 / 0.930 (best 0.951)  loop 1/2  05_bottle.png"; "looped split image")]
#[test_case(Status { reset_similarity: Some(ResetReading::Similarity(0.1234)), reset_threshold: Some(0.96), ..status() } => "split 3/12  0.412 / 0.930 (best 0.951)  reset 0.123 / 0.960  05_bottle.png"; "with the reset image")]
#[test_case(Status { showing: Showing::Image(ImageRef::Start), split_position: None, current_filename: Some("start.png".into()), ..status() } => "waiting for start  0.412 / 0.930 (best 0.951)  start.png"; "start image")]
#[test_case(Status { showing: Showing::Pause { until: NOW + Duration::from_millis(9900) }, ..status() } => "comparing again in 9.9s"; "pause")]
#[test_case(Status { showing: Showing::SplitDelay { until: NOW + Duration::from_millis(300) }, ..status() } => "splitting in 0.3s"; "split delay")]
#[test_case(Status { showing: Showing::StartDelay { until: NOW + Duration::from_millis(300) }, ..status() } => "starting in 0.3s"; "start delay")]
#[test_case(Status { showing: Showing::Nothing, ..status() } => "ready: press num 1 to start"; "idle")]
fn shows_the_status(status: Status) -> String {
    // Act, with the expected value checked by test_case
    status_line(&status, NOW, "ready: press num 1 to start", true)
}

#[test]
fn similarity_without_a_capture_is_a_dash() {
    // Act
    let line = status_line(&status(), NOW, "", false);

    // Assert
    assert_eq!(line, "split 3/12  - / 0.930 (best 0.951)  05_bottle.png");
}
