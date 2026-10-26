//! Number formatting for the UI.

use autosplit_core::commands::Command;
use autosplit_core::text::{Compared, decimal, seconds_remaining_text};
use test_case::test_case;

#[test_case(0.9, "0.900"; "pads to three decimals")]
#[test_case(1.0, "1.000"; "one")]
#[test_case(0.9999, "0.999"; "truncates instead of rounding")]
#[test_case(0.29, "0.290"; "no float rounding step down like python's int of 0.29 times 100")]
#[test_case(-0.0001, "0.000"; "small negatives truncate to zero")]
#[test_case(-0.25, "-0.250"; "negatives")]
#[test_case(1e-7, "0.000"; "tiny values")]
fn decimal_truncates_to_three_decimals(value: f64, expected: &str) {
    // Act
    let text = decimal(value);

    // Assert
    assert_eq!(text, expected);
}

#[test_case(1.0, "1.0 second remaining"; "singular at one second")]
#[test_case(1.04, "1.0 second remaining"; "singular when it shows as one second")]
#[test_case(0.5, "0.5 seconds remaining"; "plural below one second")]
#[test_case(2.04, "2.0 seconds remaining"; "plural above one second")]
fn seconds_remaining(seconds: f64, expected: &str) {
    // Act
    let text = seconds_remaining_text(seconds);

    // Assert
    assert_eq!(text, expected);
}

fn compared(below: bool) -> Compared {
    Compared { filename: "05_bottle.png".into(), threshold: Some(0.93), below }
}

#[test_case(Some(compared(false)), Command::Split => " (05_bottle.png at 0.951 / 0.930)"; "split")]
#[test_case(Some(compared(true)), Command::Pause => " (05_bottle.png dropped to 0.951, below 0.930)"; "below flag pause")]
#[test_case(Some(compared(false)), Command::Reset => ""; "reset has no image")]
#[test_case(None, Command::Split => ""; "nothing compared")]
fn split_details(compared: Option<Compared>, command: Command) -> String {
    // Act, with the expected value checked by test_case
    Compared::details(compared.as_ref(), command, Some(0.951))
}
