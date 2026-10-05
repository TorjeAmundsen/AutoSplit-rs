//! Number formatting for the UI.

use autosplit_core::text::{decimal, seconds_remaining_text};
use test_case::test_case;

#[test_case(0.9, "0.900"; "pads to three decimals")]
#[test_case(1.0, "1.000"; "one")]
#[test_case(0.0, "0.000"; "zero")]
#[test_case(0.9999, "0.999"; "truncates instead of rounding")]
#[test_case(0.29, "0.290"; "no float rounding step down like python's int of 0.29 times 100")]
#[test_case(0.12345, "0.123"; "drops extra decimals")]
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
#[test_case(0.0, "0.0 seconds remaining"; "plural at zero")]
fn seconds_remaining(seconds: f64, expected: &str) {
    // Act
    let text = seconds_remaining_text(seconds);

    // Assert
    assert_eq!(text, expected);
}
