//! Filename values AutoSplit ignores are reported as warnings. The values themselves are
//! checked against AutoSplit by the golden tests.

use autosplit_core::filename::{
    FilenameWarning, RESET_KEYWORD, START_KEYWORD, Setting, delay_time_from_filename, filename_warnings, has_keyword,
    threshold_from_filename,
};
use test_case::test_case;

fn not_a_number(setting: Setting, value: &str) -> Vec<FilenameWarning> {
    vec![FilenameWarning::NotANumber { setting, value: value.into() }]
}

fn out_of_range(setting: Setting, value: &str) -> Vec<FilenameWarning> {
    vec![FilenameWarning::OutOfRange { setting, value: value.into() }]
}

#[test_case("01_(0.9)_[5]_#1000#_@3@_^2^_{b}.png" => Vec::<FilenameWarning>::new(); "valid values")]
#[test_case("split.png" => Vec::<FilenameWarning>::new(); "no values")]
#[test_case("x_(0.9a).png" => not_a_number(Setting::Threshold, "0.9a"); "threshold that isn't a number")]
#[test_case("x_( 0.95 ).png" => not_a_number(Setting::Threshold, " 0.95 "); "threshold with spaces")]
#[test_case("x_[ten].png" => not_a_number(Setting::PauseTime, "ten"); "pause that isn't a number")]
#[test_case("x_#1_000#.png" => not_a_number(Setting::DelayTime, "1_000"); "delay with an underscore")]
#[test_case("x_#1.5#.png" => not_a_number(Setting::DelayTime, "1.5"); "fractional delay")]
#[test_case("x_@two@.png" => not_a_number(Setting::Loops, "two"); "loops that isn't a number")]
#[test_case("x_^x^.png" => not_a_number(Setting::ComparisonMethod, "x"); "method that isn't a number")]
#[test_case("x_(1.5).png" => out_of_range(Setting::Threshold, "1.5"); "threshold above 1")]
#[test_case("x_(nan).png" => out_of_range(Setting::Threshold, "nan"); "nan threshold")]
#[test_case("x_[-2].png" => out_of_range(Setting::PauseTime, "-2"); "negative pause")]
#[test_case("x_#-5#.png" => out_of_range(Setting::DelayTime, "-5"); "negative delay")]
#[test_case("x_@0@.png" => out_of_range(Setting::Loops, "0"); "zero loops")]
#[test_case("x_^-1^.png" => out_of_range(Setting::ComparisonMethod, "-1"); "negative method")]
#[test_case("x_^5^.png" => vec![FilenameWarning::UnknownComparisonMethod(5)]; "unknown method")]
#[test_case("x_{dx}.png" => vec![FilenameWarning::InvalidFlag { flags: "dx".into(), invalid: 'x' }]; "invalid flag")]
#[test_case("x_{dp}.png" => vec![FilenameWarning::DummyAndPause { flags: "dp".into() }]; "dummy and pause")]
#[test_case("boss #2.png" => Vec::<FilenameWarning>::new(); "unclosed delimiter")]
#[test_case("x_(0.9a)_{x}.png" => vec![
    FilenameWarning::NotANumber { setting: Setting::Threshold, value: "0.9a".into() },
    FilenameWarning::InvalidFlag { flags: "x".into(), invalid: 'x' },
]; "several problems")]
fn warnings(filename: &str) -> Vec<FilenameWarning> {
    // Act, with the expected value checked by test_case
    filename_warnings(filename)
}

#[test]
fn threshold_with_spaces_falls_back_to_the_default() {
    // Act
    let threshold = threshold_from_filename("x_( 0.95 ).png");

    // Assert
    assert_eq!(threshold, None);
}

#[test]
fn delay_with_an_underscore_falls_back_to_the_default() {
    // Act
    let delay = delay_time_from_filename("x_#1_000#.png");

    // Assert
    assert_eq!(delay, None);
}

#[test]
fn warning_message_names_the_value_and_the_fallback() {
    // Arrange
    let warning = FilenameWarning::OutOfRange { setting: Setting::Threshold, value: "1.5".into() };

    // Act
    let message = warning.to_string();

    // Assert
    assert_eq!(message, "the threshold 1.5 isn't between 0 and 1, so the profile's default is used");
}

#[test_case("reset.png", RESET_KEYWORD => true; "reset alone")]
#[test_case("reset_(0.96)_[0.1].png", RESET_KEYWORD => true; "reset with values")]
#[test_case("03_reset_room.png", RESET_KEYWORD => true; "reset between underscores")]
#[test_case("02reset.png", RESET_KEYWORD => true; "digits before reset")]
#[test_case("reset2.png", RESET_KEYWORD => true; "digits after reset")]
#[test_case("preset_menu.png", RESET_KEYWORD => false; "letters before reset")]
#[test_case("resets.png", RESET_KEYWORD => false; "letters after reset")]
#[test_case("preset_reset.png", RESET_KEYWORD => true; "a later whole word counts")]
#[test_case("start_auto_splitter_(0.9).png", START_KEYWORD => true; "start keyword")]
#[test_case("xstart_auto_splitter.png", START_KEYWORD => false; "start keyword inside a word")]
fn keyword(filename: &str, keyword: &str) -> bool {
    // Act, with the expected value checked by test_case
    has_keyword(filename, keyword)
}

#[test_case("x_(0.5.png"; "unclosed threshold")]
#[test_case("x_(0.5"; "unclosed threshold at the end")]
fn unclosed_threshold_is_ignored(filename: &str) {
    // Act
    let threshold = threshold_from_filename(filename);

    // Assert
    assert_eq!(threshold, None);
}
