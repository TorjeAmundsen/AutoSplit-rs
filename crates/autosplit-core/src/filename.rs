//! Split image filename conventions, matching AutoSplit's `split_parser.py`.
//!
//! `(0.95)` threshold, `[10]` pause seconds, `#500#` delay milliseconds, `@3@` loops,
//! `^1^` comparison method, `{dbp}` flags. Values need their closing delimiter. Values that are
//! missing, invalid or out of range fall back to defaults like in AutoSplit, and
//! [`filename_warnings`] reports the invalid ones.

use std::fmt;

use crate::compare::ComparisonMethod;

pub const DUMMY_FLAG: u32 = 1 << 0;
pub const BELOW_FLAG: u32 = 1 << 1;
pub const PAUSE_FLAG: u32 = 1 << 2;

pub const START_KEYWORD: &str = "start_auto_splitter";
pub const RESET_KEYWORD: &str = "reset";

/// Whether `keyword` appears in `filename` as a word of its own: not preceded or followed by a
/// letter. "reset_(0.96).png" and "02reset.png" have the reset keyword, "preset.png" doesn't.
pub fn has_keyword(filename: &str, keyword: &str) -> bool {
    filename.match_indices(keyword).any(|(start, _)| {
        let before = filename[..start].chars().next_back();
        let after = filename[start + keyword.len()..].chars().next();
        !before.is_some_and(char::is_alphabetic) && !after.is_some_and(char::is_alphabetic)
    })
}

/// The digits `filename` starts with, like "01" in "01_boss.png". Split images with the same
/// number are compared for the same split when the profile groups them.
pub fn number_prefix(filename: &str) -> Option<&str> {
    let digits = filename.len() - filename.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    (digits > 0).then(|| &filename[..digits])
}

/// The text between the first `open` and the next `close`.
fn raw_value(filename: &str, open: char, close: char) -> Option<&str> {
    let (_, after) = filename.split_once(open)?;
    Some(after.split_once(close)?.0)
}

fn float_value(filename: &str, open: char, close: char) -> Option<f64> {
    raw_value(filename, open, close)?.parse().ok()
}

fn int_value(filename: &str, open: char, close: char) -> Option<i64> {
    raw_value(filename, open, close)?.parse().ok()
}

/// `(0.95)`: similarity threshold, only valid within [0, 1].
pub fn threshold_from_filename(filename: &str) -> Option<f64> {
    float_value(filename, '(', ')').filter(|value| (0.0..=1.0).contains(value))
}

/// `[10]`: pause time in seconds after the split.
pub fn pause_from_filename(filename: &str) -> Option<f64> {
    float_value(filename, '[', ']').filter(|value| *value >= 0.0)
}

/// `#500#`: delay in milliseconds before the split is sent.
pub fn delay_time_from_filename(filename: &str) -> Option<i64> {
    int_value(filename, '#', '#').filter(|value| *value >= 0)
}

/// `@3@`: how many times the image is used in a row. At least 1.
pub fn loop_from_filename(filename: &str) -> i64 {
    int_value(filename, '@', '@').unwrap_or(1).max(1)
}

/// `^1^`: comparison method index.
pub fn comparison_method_from_filename(filename: &str) -> Option<i64> {
    int_value(filename, '^', '^').filter(|value| *value >= 0)
}

/// `{dbp}`: dummy, below threshold and pause flags. `m` is a legacy flag that's ignored.
/// Any other character, or dummy combined with pause, invalidates all flags.
pub fn flags_from_filename(filename: &str) -> u32 {
    parse_flags(filename).unwrap_or(0)
}

fn parse_flags(filename: &str) -> Result<u32, FilenameWarning> {
    let Some(raw) = raw_value(filename, '{', '}') else {
        return Ok(0);
    };
    let mut flags = 0;
    for c in raw.chars() {
        match c.to_ascii_uppercase() {
            'D' => flags |= DUMMY_FLAG,
            'B' => flags |= BELOW_FLAG,
            'P' => flags |= PAUSE_FLAG,
            'M' => {}
            _ => return Err(FilenameWarning::InvalidFlag { flags: raw.to_owned(), invalid: c }),
        }
    }
    if flags & DUMMY_FLAG != 0 && flags & PAUSE_FLAG != 0 {
        return Err(FilenameWarning::DummyAndPause { flags: raw.to_owned() });
    }
    Ok(flags)
}

/// A filename value that's ignored, the way AutoSplit silently ignores it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FilenameWarning {
    NotANumber { setting: Setting, value: String },
    OutOfRange { setting: Setting, value: String },
    InvalidFlag { flags: String, invalid: char },
    DummyAndPause { flags: String },
    UnknownComparisonMethod(i64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    Threshold,
    PauseTime,
    DelayTime,
    Loops,
    ComparisonMethod,
}

impl Setting {
    fn name(self) -> &'static str {
        match self {
            Self::Threshold => "threshold",
            Self::PauseTime => "pause time",
            Self::DelayTime => "delay time",
            Self::Loops => "loop count",
            Self::ComparisonMethod => "comparison method",
        }
    }

    fn allowed(self) -> &'static str {
        match self {
            Self::Threshold => "between 0 and 1",
            Self::Loops => "1 or more",
            Self::PauseTime | Self::DelayTime | Self::ComparisonMethod => "0 or more",
        }
    }

    fn fallback(self) -> &'static str {
        match self {
            Self::Loops => "the image is used once",
            _ => "the profile's default is used",
        }
    }
}

impl fmt::Display for FilenameWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotANumber { setting, value } => {
                write!(f, "the {} '{value}' isn't a number, so {}", setting.name(), setting.fallback())
            }
            Self::OutOfRange { setting, value } => {
                write!(f, "the {} {value} isn't {}, so {}", setting.name(), setting.allowed(), setting.fallback())
            }
            Self::InvalidFlag { flags, invalid } => write!(
                f,
                "the flags {{{flags}}} contain '{invalid}', which isn't d, b, p or m, so all of the image's flags are ignored"
            ),
            Self::DummyAndPause { flags } => {
                write!(f, "the flags {{{flags}}} combine dummy and pause, so all of the image's flags are ignored")
            }
            Self::UnknownComparisonMethod(index) => write!(
                f,
                "comparison method {index} doesn't exist (0 is L2 norm, 1 histograms, 2 pHash), so the image never matches"
            ),
        }
    }
}

/// Values in `filename` that are ignored.
pub fn filename_warnings(filename: &str) -> Vec<FilenameWarning> {
    let mut warnings = Vec::new();
    let mut check = |setting: Setting, open, close, in_range: &dyn Fn(&str) -> Option<bool>| {
        let Some(raw) = raw_value(filename, open, close) else {
            return;
        };
        let value = raw.to_owned();
        match in_range(raw) {
            None => warnings.push(FilenameWarning::NotANumber { setting, value }),
            Some(false) => warnings.push(FilenameWarning::OutOfRange { setting, value }),
            Some(true) => {}
        }
    };
    let float = |text: &str| text.parse::<f64>().ok();
    let int = |text: &str| text.parse::<i64>().ok();
    check(Setting::Threshold, '(', ')', &|text| float(text).map(|value| (0.0..=1.0).contains(&value)));
    check(Setting::PauseTime, '[', ']', &|text| float(text).map(|value| value >= 0.0));
    check(Setting::DelayTime, '#', '#', &|text| int(text).map(|value| value >= 0));
    check(Setting::Loops, '@', '@', &|text| int(text).map(|value| value >= 1));
    check(Setting::ComparisonMethod, '^', '^', &|text| int(text).map(|value| value >= 0));

    if let Some(index) =
        comparison_method_from_filename(filename).filter(|index| ComparisonMethod::from_index(*index).is_none())
    {
        warnings.push(FilenameWarning::UnknownComparisonMethod(index));
    }
    if let Err(warning) = parse_flags(filename) {
        warnings.push(warning);
    }
    warnings
}
