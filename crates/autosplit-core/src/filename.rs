//! Split image filename conventions, matching AutoSplit's `split_parser.py`.
//!
//! `(0.95)` threshold, `[10]` pause seconds, `#500#` delay milliseconds, `@3@` loops,
//! `^1^` comparison method, `{dbp}` flags. Values need their closing delimiter. Values that are
//! missing, invalid or out of range fall back to defaults like in AutoSplit, and
//! [`filename_warnings`] reports the invalid ones.

use std::fmt;

use crate::compare::ComparisonMethod;
use crate::split_image::SplitDefaults;

pub const DUMMY_FLAG: u32 = 1 << 0;
pub const BELOW_FLAG: u32 = 1 << 1;
pub const PAUSE_FLAG: u32 = 1 << 2;

pub const START_KEYWORD: &str = "start_auto_splitter";
pub const RESET_KEYWORD: &str = "reset";

/// Whether `keyword` appears in `filename` as a word of its own: not preceded or followed by a
/// letter. "reset_(0.96).png" and "02reset.png" have the reset keyword, "preset.png" doesn't.
pub fn has_keyword(filename: &str, keyword: &str) -> bool {
    keyword_start(filename, keyword).is_some()
}

/// Where `keyword` first appears in `filename` as a word of its own.
fn keyword_start(filename: &str, keyword: &str) -> Option<usize> {
    filename.match_indices(keyword).map(|(start, _)| start).find(|&start| {
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

/// What a part of a split image's file name does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartKind {
    /// Text without a meaning, like the split's name.
    Text,
    /// The leading digits.
    Number,
    /// `start_auto_splitter` or `reset`, the one that decides the image's type.
    Keyword,
    Threshold,
    Pause,
    Delay,
    Loops,
    Method,
    Flags,
    Extension,
}

/// A part of a split image's file name, as the parser reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamePart {
    pub kind: PartKind,
    /// As written in the file name.
    pub text: String,
    /// What it does, or why it's ignored. Empty for plain text.
    pub meaning: String,
    /// The value is ignored, see [`filename_warnings`].
    pub ignored: bool,
}

/// `filename` in parts, each value with what it does, read the way the split image loader reads
/// it: the first `(` to the next `)` and so on. The parts together are `filename`. A value inside
/// another one is left as part of the other.
pub fn name_parts(filename: &str) -> Vec<NamePart> {
    // ASCII lowercasing keeps every byte where it is
    let lower = filename.to_ascii_lowercase();
    let warnings = filename_warnings(&lower);
    let warning = |setting: Setting| {
        warnings.iter().find(|warning| match warning {
            FilenameWarning::NotANumber { setting: of, .. } | FilenameWarning::OutOfRange { setting: of, .. } => {
                *of == setting
            }
            FilenameWarning::UnknownComparisonMethod(_) => setting == Setting::ComparisonMethod,
            FilenameWarning::InvalidFlag { .. } | FilenameWarning::DummyAndPause { .. } => false,
        })
    };
    let mut spans: Vec<(usize, usize, PartKind, String, bool)> = Vec::new();
    if let Some(number) = number_prefix(&lower) {
        let meaning = format!("number {number}: the image's place in the order, and its group when grouping is on");
        spans.push((0, number.len(), PartKind::Number, meaning, false));
    }
    // The start keyword decides the type when both are there
    let keyword = [(START_KEYWORD, "start image"), (RESET_KEYWORD, "reset image")]
        .into_iter()
        .find_map(|(keyword, meaning)| Some((keyword_start(&lower, keyword)?, keyword, meaning)));
    if let Some((start, keyword, meaning)) = keyword {
        spans.push((start, start + keyword.len(), PartKind::Keyword, meaning.to_owned(), false));
    }
    let values = [
        ('(', ')', PartKind::Threshold, Setting::Threshold),
        ('[', ']', PartKind::Pause, Setting::PauseTime),
        ('#', '#', PartKind::Delay, Setting::DelayTime),
        ('@', '@', PartKind::Loops, Setting::Loops),
        ('^', '^', PartKind::Method, Setting::ComparisonMethod),
    ];
    for (open, close, kind, setting) in values {
        let Some(start) = lower.find(open) else { continue };
        let Some(length) = lower[start + 1..].find(close) else { continue };
        let end = start + 1 + length + 1;
        let raw = &lower[start + 1..end - 1];
        let (meaning, ignored) = match warning(setting) {
            Some(warning) => (format!("ignored: {warning}"), true),
            None => (value_meaning(kind, raw), false),
        };
        spans.push((start, end, kind, meaning, ignored));
    }
    if let Some(start) = lower.find('{')
        && let Some(length) = lower[start..].find('}')
    {
        let (meaning, ignored) = match parse_flags(&lower) {
            Ok(flags) => (flags_meaning(flags), false),
            Err(warning) => (format!("ignored: {warning}"), true),
        };
        spans.push((start, start + length + 1, PartKind::Flags, meaning, ignored));
    }
    if let Some(dot) = lower.rfind('.') {
        spans.push((dot, lower.len(), PartKind::Extension, String::new(), false));
    }
    spans.sort_by_key(|span| span.0);

    let mut parts = Vec::new();
    let mut at = 0;
    let text = |parts: &mut Vec<NamePart>, from: usize, to: usize| {
        if from < to {
            parts.push(NamePart {
                kind: PartKind::Text,
                text: filename[from..to].to_owned(),
                meaning: String::new(),
                ignored: false,
            });
        }
    };
    for (start, end, kind, meaning, ignored) in spans {
        if start < at {
            continue;
        }
        text(&mut parts, at, start);
        parts.push(NamePart { kind, text: filename[start..end].to_owned(), meaning, ignored });
        at = end;
    }
    text(&mut parts, at, filename.len());
    parts
}

/// What a valid value does.
fn value_meaning(kind: PartKind, raw: &str) -> String {
    match kind {
        PartKind::Threshold => format!("threshold: splits at a similarity of {raw} or more"),
        PartKind::Pause => format!("pause: stops comparing for {raw} seconds after the split"),
        PartKind::Delay => format!("delay: splits {raw} ms after the match"),
        PartKind::Loops => format!("loops: used {raw} times in a row"),
        PartKind::Method => match raw.parse().ok().and_then(ComparisonMethod::from_index) {
            Some(method) => format!("comparison method: {}", method.name()),
            None => format!("comparison method {raw}"),
        },
        _ => String::new(),
    }
}

/// What valid flags do.
fn flags_meaning(flags: u32) -> String {
    let named = [
        (DUMMY_FLAG, "dummy: moves on without splitting"),
        (BELOW_FLAG, "below: splits once the similarity drops back below the threshold"),
        (PAUSE_FLAG, "pause: pauses the timer instead of splitting"),
    ];
    let meanings: Vec<&str> = named.iter().filter(|(flag, _)| flags & flag != 0).map(|(_, meaning)| *meaning).collect();
    if meanings.is_empty() { "flags: none".into() } else { meanings.join("; ") }
}

/// The settings a split image's file name carries, from a preset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplitSpec<'a> {
    /// `reset` and `start_auto_splitter` make the reset and start images.
    pub name: &'a str,
    pub threshold: f64,
    /// Seconds, or `None` for AutoSplit's default pause time.
    pub pause: Option<f64>,
    /// Milliseconds.
    pub delay: u32,
    pub dummy: bool,
    pub below: bool,
}

/// The file name of split image `index` (from 0) of `count`, like AutoMask names them. With the
/// profile's `defaults`, it leaves out a delay the profile already has, and asks for L2 Norm,
/// which presets are made for, when the profile compares another way.
///
/// `reset_(0.96)_[0.1].png`, `05_Bottle_(0.93)_[35]_#1100#_{b}.png`: the number has as many
/// digits as `count`, so the images sort in order.
pub fn build_filename(spec: &SplitSpec, index: usize, count: usize, defaults: Option<&SplitDefaults>) -> String {
    let mut name = match spec.name {
        START_KEYWORD | RESET_KEYWORD => spec.name.to_owned(),
        _ => format!("{index:0width$}_{}", spec.name, width = count.to_string().len()),
    };
    name += &format!("_({})", spec.threshold);
    if let Some(pause) = spec.pause {
        name += &format!("_[{pause}]");
    }
    let default_delay = defaults.map_or(0, |defaults| defaults.delay_time);
    if i64::from(spec.delay) != default_delay {
        name += &format!("_#{}#", spec.delay);
    }
    if defaults.is_some_and(|defaults| defaults.comparison_method != ComparisonMethod::L2Norm.index()) {
        name += "_^0^";
    }
    let flags: String = [(spec.dummy, 'd'), (spec.below, 'b')].iter().filter(|(on, _)| *on).map(|(_, c)| c).collect();
    if !flags.is_empty() {
        name += &format!("_{{{flags}}}");
    }
    name + ".png"
}

/// Why a split's name can't make a split image's file name.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum NameError {
    #[error("The name can't be empty.")]
    Empty,
    #[error("The name can't have '{0}', which AutoSplit reads as the start of a setting in file names.")]
    SettingCharacter(char),
    #[error("The name can't have '{0}', which file names can't have.")]
    FileNameCharacter(char),
    #[error("The name can't have '{0}' in it, which would make it the {0} image. Name it exactly {0} for that.")]
    Keyword(&'static str),
}

/// Checks a split's name, the part of the file name between the number and the settings.
pub fn validate_split_name(name: &str) -> Result<(), NameError> {
    if name.trim().is_empty() {
        return Err(NameError::Empty);
    }
    if name == START_KEYWORD || name == RESET_KEYWORD {
        return Ok(());
    }
    if let Some(c) = name.chars().find(|c| "#@{}()[]^".contains(*c)) {
        return Err(NameError::SettingCharacter(c));
    }
    if let Some(c) = name.chars().find(|c| r#"/\:*?"<>|"#.contains(*c) || c.is_control()) {
        return Err(NameError::FileNameCharacter(c));
    }
    let lower = name.to_lowercase();
    // AutoSplit matches the keywords anywhere in the name, so "preset" counts too
    match [START_KEYWORD, RESET_KEYWORD].into_iter().find(|keyword| lower.contains(keyword)) {
        Some(keyword) => Err(NameError::Keyword(keyword)),
        None => Ok(()),
    }
}
