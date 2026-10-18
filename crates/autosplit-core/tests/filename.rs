//! Filename values AutoSplit ignores are reported as warnings. The values themselves are
//! checked against AutoSplit by the golden tests.

use autosplit_core::filename::{
    FilenameWarning, NameError, RESET_KEYWORD, START_KEYWORD, Setting, SplitSpec, build_filename,
    delay_time_from_filename, fade_time_from_filename, filename_warnings, has_keyword, name_parts,
    threshold_from_filename, validate_split_name,
};
use autosplit_core::split_image::SplitDefaults;
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
#[test_case("x_[ten].png" => not_a_number(Setting::PauseTime, "ten"); "pause that isn't a number")]
#[test_case("x_( 0.95 ).png" => not_a_number(Setting::Threshold, " 0.95 "); "threshold with spaces")]
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
#[test_case("boss #2.png" => vec![FilenameWarning::Delimiters { setting: Setting::DelayTime, count: 1 }]; "unclosed delimiter")]
#[test_case("x_(0.5)_(0.7).png" => vec![FilenameWarning::Delimiters { setting: Setting::Threshold, count: 4 }]; "two thresholds")]
#[test_case("x_{b}_{d}.png" => vec![FilenameWarning::Delimiters { setting: Setting::Flags, count: 4 }]; "two flag groups")]
#[test_case("x_(0.9a)_{x}.png" => vec![
    FilenameWarning::NotANumber { setting: Setting::Threshold, value: "0.9a".into() },
    FilenameWarning::InvalidFlag { flags: "x".into(), invalid: 'x' },
]; "several problems")]
#[test_case("x_~500~_{f}.png" => Vec::<FilenameWarning>::new(); "fade image with a fade time")]
#[test_case("x_~100~_{f}.png" => out_of_range(Setting::FadeTime, "100"); "fade time too short to tell from a cut")]
#[test_case("x_~3600000~_{f}.png" => out_of_range(Setting::FadeTime, "3600000"); "fade time too long")]
#[test_case("x_~fast~_{f}.png" => not_a_number(Setting::FadeTime, "fast"); "fade time that isn't a number")]
#[test_case("x_~1000~.png" => vec![FilenameWarning::FadeOnly(Setting::FadeTime)]; "fade time without the fade flag")]
#[test_case("boss~2.png" => Vec::<FilenameWarning>::new(); "lone tilde in a name")]
#[test_case("x_^1^_{f}.png" => vec![FilenameWarning::NotForFade(Setting::ComparisonMethod)]; "method on a fade image")]
#[test_case("x_^9^_{f}.png" => vec![FilenameWarning::NotForFade(Setting::ComparisonMethod)]; "unknown method on a fade image")]
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

#[test_case("x_#5#_#6#.png"; "two delays")]
#[test_case("x_#5.png"; "one delimiter")]
fn delay_with_the_wrong_number_of_delimiters_falls_back_to_the_default(filename: &str) {
    // Act
    let delay = delay_time_from_filename(filename);

    // Assert
    assert_eq!(delay, None);
}

#[test_case(FilenameWarning::Delimiters { setting: Setting::Threshold, count: 4 } => "'(' and ')' appear 4 times, but the threshold has to be between one of each, so the profile's default is used"; "pair")]
#[test_case(FilenameWarning::Delimiters { setting: Setting::DelayTime, count: 3 } => "'#' appears 3 times, but the delay time has to be between exactly 2, so the profile's default is used"; "same character")]
fn delimiter_warning_message(warning: FilenameWarning) -> String {
    // Act, with the expected value checked by test_case
    warning.to_string()
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

/// The parts of `filename` as `kind:text`, with `!` for ignored values.
fn parts(filename: &str) -> Vec<String> {
    let mark = |ignored: bool| if ignored { "!" } else { "" };
    name_parts(filename).iter().map(|part| format!("{:?}{}:{}", part.kind, mark(part.ignored), part.text)).collect()
}

#[test]
fn name_parts_follow_the_parser() {
    // Act
    let parts = parts("05_Bottle_(0.93)_[35]_#1100#_{b}.png");

    // Assert
    assert_eq!(
        parts,
        [
            "Number:05",
            "Text:_Bottle_",
            "Threshold:(0.93)",
            "Text:_",
            "Pause:[35]",
            "Text:_",
            "Delay:#1100#",
            "Text:_",
            "Flags:{b}",
            "Extension:.png",
        ]
    );
}

#[test_case("Reset_(1.5).png" => vec!["Keyword:Reset", "Text:_", "Threshold!:(1.5)", "Extension:.png"]; "out of range threshold")]
#[test_case("x_{dp}.png" => vec!["Text:x_", "Flags!:{dp}", "Extension:.png"]; "dummy and pause flags")]
#[test_case("start_auto_splitter_reset.png" => vec!["Keyword:start_auto_splitter", "Text:_reset", "Extension:.png"]; "start keyword wins")]
#[test_case("x_(0.5.png" => vec!["Text:x_", "Threshold!:(", "Text:0.5", "Extension:.png"]; "unclosed value")]
#[test_case("x_(0.5)_(0.7).png" => vec!["Text:x_", "Threshold!:(0.5)_(0.7)", "Extension:.png"]; "two values")]
#[test_case("x_~500~_{f}.png" => vec!["Text:x_", "FadeTime:~500~", "Text:_", "Flags:{f}", "Extension:.png"]; "fade time")]
#[test_case("x_~500~.png" => vec!["Text:x_", "FadeTime!:~500~", "Extension:.png"]; "fade time without the fade flag")]
#[test_case("boss~2.png" => vec!["Text:boss~2", "Extension:.png"]; "lone tilde")]
fn name_parts_mark_what_the_parser_ignores(filename: &str) -> Vec<String> {
    // Act, with the expected value checked by test_case
    parts(filename)
}

#[test]
fn name_parts_keep_the_whole_name() {
    // Arrange
    let filename = "01_ÆØÅ_(0.9)_^7^_@2@_{x}.PNG";

    // Act
    let joined: String = name_parts(filename).into_iter().map(|part| part.text).collect();

    // Assert
    assert_eq!(joined, filename);
}

const SPEC: SplitSpec = SplitSpec {
    name: "Bottle",
    threshold: Some(0.93),
    pause: None,
    delay: None,
    dummy: false,
    below: false,
    fade_time: None,
};

fn defaults(delay_time: i64, comparison_method: i64) -> SplitDefaults {
    SplitDefaults {
        comparison_method,
        similarity_threshold: 0.9,
        delay_time,
        pause_time: 10.0,
        fps_limit: 60,
        fade_tolerance: 10,
    }
}

#[test_case(SPEC, 5, 20, None => "05_Bottle_(0.93).png"; "padded to the count's digits")]
#[test_case(SPEC, 5, 9, None => "5_Bottle_(0.93).png"; "one digit")]
#[test_case(SplitSpec { name: "reset", pause: Some(0.1), ..SPEC }, 0, 20, None => "reset_(0.93)_[0.1].png"; "reset image without a number")]
#[test_case(SplitSpec { name: "start_auto_splitter", pause: Some(10.0), below: true, ..SPEC }, 1, 20, None => "start_auto_splitter_(0.93)_[10]_{b}.png"; "start image")]
#[test_case(SplitSpec { pause: Some(35.0), delay: Some(1100), below: true, ..SPEC }, 5, 20, None => "05_Bottle_(0.93)_[35]_#1100#_{b}.png"; "every setting")]
#[test_case(SplitSpec { threshold: None, ..SPEC }, 5, 20, None => "05_Bottle.png"; "the profile's threshold")]
#[test_case(SplitSpec { dummy: true, below: true, ..SPEC }, 5, 20, None => "05_Bottle_(0.93)_{db}.png"; "flags in one group")]
#[test_case(SPEC, 5, 20, Some(defaults(500, 0)) => "05_Bottle_(0.93).png"; "the profile's delay")]
#[test_case(SplitSpec { delay: Some(500), ..SPEC }, 5, 20, Some(defaults(500, 0)) => "05_Bottle_(0.93)_#500#.png"; "its own delay, like the profile's")]
#[test_case(SplitSpec { delay: Some(0), ..SPEC }, 5, 20, Some(defaults(500, 0)) => "05_Bottle_(0.93)_#0#.png"; "no delay against the profile's")]
#[test_case(SPEC, 5, 20, Some(defaults(0, 2)) => "05_Bottle_(0.93)_^0^.png"; "l2 norm against the profile's method")]
#[test_case(SplitSpec { threshold: Some(0.995), fade_time: Some(1000), ..SPEC }, 5, 20, None => "05_Bottle_(0.995)_~1000~_{f}.png"; "fade split")]
#[test_case(SplitSpec { fade_time: Some(350), dummy: true, ..SPEC }, 5, 20, Some(defaults(0, 2)) => "05_Bottle_(0.93)_~350~_{df}.png"; "fade split, which never asks for a method")]
fn builds_file_names_like_automask(
    spec: SplitSpec,
    index: usize,
    count: usize,
    defaults: Option<SplitDefaults>,
) -> String {
    // Act, with the expected value checked by test_case
    build_filename(&spec, index, count, defaults.as_ref())
}

#[test_case("Bottle" => Ok(()); "plain")]
#[test_case("Full white fade" => Ok(()); "spaces")]
#[test_case("reset" => Ok(()); "reset image")]
#[test_case(" " => Err(NameError::Empty); "empty")]
#[test_case("Boss (2)" => Err(NameError::SettingCharacter('(')); "setting character")]
#[test_case("A/B" => Err(NameError::FileNameCharacter('/')); "file name character")]
#[test_case("Reset Room" => Err(NameError::Keyword(RESET_KEYWORD)); "reset keyword")]
#[test_case("Preset" => Err(NameError::Keyword(RESET_KEYWORD)); "keyword inside a word")]
#[test_case("Fade~1" => Err(NameError::SettingCharacter('~')); "fade time character")]
fn split_names(name: &str) -> Result<(), NameError> {
    // Act, with the expected value checked by test_case
    validate_split_name(name)
}

#[test_case("x_~500~_{f}.png" => Some(500); "fade time")]
#[test_case("x_~100~_{f}.png" => None; "too short")]
#[test_case("x_~10001~_{f}.png" => None; "too long")]
#[test_case("x_~10000~_{f}.png" => Some(10000); "the longest")]
#[test_case("x_{f}.png" => None; "not set")]
fn fade_time(filename: &str) -> Option<i64> {
    // Act, with the expected value checked by test_case
    fade_time_from_filename(filename)
}

#[test]
fn fade_split_file_names_read_back_as_fade_images() {
    // Arrange
    let spec = SplitSpec { threshold: Some(0.995), fade_time: Some(350), below: true, ..SPEC };
    let name = build_filename(&spec, 5, 20, None);

    // Act
    let read = (filename_warnings(&name), fade_time_from_filename(&name), threshold_from_filename(&name));

    // Assert
    assert_eq!(read, (vec![], Some(350), Some(0.995)));
}
