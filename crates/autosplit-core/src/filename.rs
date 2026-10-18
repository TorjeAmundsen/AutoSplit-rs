//! Split image filename conventions, matching AutoSplit's `split_parser.py`.
//!
//! `(0.95)` threshold, `[10]` pause seconds, `#500#` delay milliseconds, `@3@` loops,
//! `^1^` comparison method, `{dbp}` flags. Values need their closing delimiter. Values that are
//! missing, invalid or out of range fall back to defaults like in AutoSplit, and
//! [`filename_warnings`] reports the invalid ones.
//!
//! Not in AutoSplit: the `{f}` flag makes a fade image, which splits on a gradual fade to its
//! color, and `~1000~` is how long that fade takes in milliseconds.

use std::fmt;

use crate::compare::ComparisonMethod;
use crate::split_image::SplitDefaults;

pub const DUMMY_FLAG: u32 = 1 << 0;
pub const BELOW_FLAG: u32 = 1 << 1;
pub const PAUSE_FLAG: u32 = 1 << 2;
pub const FADE_FLAG: u32 = 1 << 3;

/// How long a fade takes when the file name doesn't say, in milliseconds.
pub const DEFAULT_FADE_TIME: i64 = 1000;
/// Shorter fades can't be told apart from a cut.
pub const MIN_FADE_TIME: i64 = 150;
/// Longer ones would keep a long history of the similarity, which no game's fade needs.
pub const MAX_FADE_TIME: i64 = 10_000;

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

/// How a value between `open` and `close` is written in a file name, read like AutoSplit 2.3.4:
/// split at every `open` and `close`, the name has to make 1 part (no value) or 3 (the value in
/// the middle). Anything else is a mistake that AutoSplit reports, and the value is ignored.
enum Delimited<'a> {
    Absent,
    /// The value, and where its delimiters are.
    Value(&'a str, usize, usize),
    /// How many times `open` and `close` appear, and where the first and last are.
    Invalid(usize, usize, usize),
}

fn delimited(filename: &str, open: char, close: char) -> Delimited<'_> {
    let at: Vec<usize> = filename.match_indices([open, close]).map(|(at, _)| at).collect();
    match at[..] {
        [] => Delimited::Absent,
        [first, second] => Delimited::Value(&filename[first + 1..second], first, second),
        [first, .., last] | [first @ last] => Delimited::Invalid(at.len(), first, last),
    }
}

/// The value between `open` and `close`, if it's written right.
fn raw_value(filename: &str, open: char, close: char) -> Option<&str> {
    match delimited(filename, open, close) {
        Delimited::Value(value, ..) => Some(value),
        Delimited::Absent | Delimited::Invalid(..) => None,
    }
}

/// Stricter than Python: no spaces around the number and no underscores in it (see
/// COMPATIBILITY.md).
fn parse_float(text: &str) -> Option<f64> {
    text.parse().ok()
}

fn parse_int(text: &str) -> Option<i64> {
    text.parse().ok()
}

fn float_value(filename: &str, open: char, close: char) -> Option<f64> {
    parse_float(raw_value(filename, open, close)?)
}

fn int_value(filename: &str, open: char, close: char) -> Option<i64> {
    parse_int(raw_value(filename, open, close)?)
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

/// `~1000~`: how long a fade image's fade takes, in milliseconds. From [`MIN_FADE_TIME`] to
/// [`MAX_FADE_TIME`].
pub fn fade_time_from_filename(filename: &str) -> Option<i64> {
    int_value(filename, '~', '~').filter(|value| (MIN_FADE_TIME..=MAX_FADE_TIME).contains(value))
}

/// `{dbpf}`: dummy, below threshold, pause and fade flags. `m` is a legacy flag that's ignored.
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
            'F' => flags |= FADE_FLAG,
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
    NotANumber {
        setting: Setting,
        value: String,
    },
    OutOfRange {
        setting: Setting,
        value: String,
    },
    /// The setting's delimiters appear `count` times, where a value needs exactly 2.
    Delimiters {
        setting: Setting,
        count: usize,
    },
    InvalidFlag {
        flags: String,
        invalid: char,
    },
    DummyAndPause {
        flags: String,
    },
    UnknownComparisonMethod(i64),
    /// A value that only fade images (`{f}`) use, on an image without it.
    FadeOnly(Setting),
    /// A value that fade images don't use.
    NotForFade(Setting),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    Threshold,
    PauseTime,
    DelayTime,
    Loops,
    ComparisonMethod,
    Flags,
    FadeTime,
}

impl Setting {
    /// The characters around the setting's value.
    pub fn delimiters(self) -> (char, char) {
        match self {
            Self::Threshold => ('(', ')'),
            Self::PauseTime => ('[', ']'),
            Self::DelayTime => ('#', '#'),
            Self::Loops => ('@', '@'),
            Self::ComparisonMethod => ('^', '^'),
            Self::Flags => ('{', '}'),
            Self::FadeTime => ('~', '~'),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Threshold => "threshold",
            Self::PauseTime => "pause time",
            Self::DelayTime => "delay time",
            Self::Loops => "loop count",
            Self::ComparisonMethod => "comparison method",
            Self::Flags => "flags",
            Self::FadeTime => "fade time",
        }
    }

    fn allowed(self) -> &'static str {
        match self {
            Self::Threshold => "between 0 and 1",
            Self::Loops => "1 or more",
            Self::PauseTime | Self::DelayTime | Self::ComparisonMethod => "0 or more",
            Self::Flags => "d, b, p, f or m",
            Self::FadeTime => "between 150 and 10000",
        }
    }

    fn fallback(self) -> &'static str {
        match self {
            Self::Loops => "the image is used once",
            Self::Flags => "the image has no flags",
            Self::FadeTime => "the default of 1000 is used",
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
            Self::Delimiters { setting, count } => match setting.delimiters() {
                (open, close) if open == close => write!(
                    f,
                    "'{open}' appears {count} times, but the {} has to be between exactly 2, so {}",
                    setting.name(),
                    setting.fallback()
                ),
                (open, close) => write!(
                    f,
                    "'{open}' and '{close}' appear {count} times, but the {} has to be between one of each, so {}",
                    setting.name(),
                    setting.fallback()
                ),
            },
            Self::InvalidFlag { flags, invalid } => write!(
                f,
                "the flags {{{flags}}} contain '{invalid}', which isn't d, b, p, f or m, so all of the image's flags are ignored"
            ),
            Self::DummyAndPause { flags } => {
                write!(f, "the flags {{{flags}}} combine dummy and pause, so all of the image's flags are ignored")
            }
            Self::UnknownComparisonMethod(index) => write!(
                f,
                "comparison method {index} doesn't exist (0 is L2 norm, 1 histograms, 2 pHash), so the image compares as 0, which only matches a threshold of 0"
            ),
            Self::FadeOnly(setting) => {
                write!(f, "the {} only applies to fade images, with the {{f}} flag, so it's ignored", setting.name())
            }
            Self::NotForFade(setting) => {
                write!(f, "fade images, with the {{f}} flag, compare by color, so the {} is ignored", setting.name())
            }
        }
    }
}

/// Values in `filename` that are ignored.
pub fn filename_warnings(filename: &str) -> Vec<FilenameWarning> {
    let flags = parse_flags(filename);
    let fade = flags.as_ref().is_ok_and(|flags| flags & FADE_FLAG != 0);
    let mut warnings = Vec::new();
    let mut check = |setting: Setting, in_range: &dyn Fn(&str) -> Option<bool>| {
        let (open, close) = setting.delimiters();
        let raw = match delimited(filename, open, close) {
            Delimited::Absent => return,
            Delimited::Invalid(count, ..) => return warnings.push(FilenameWarning::Delimiters { setting, count }),
            Delimited::Value(raw, ..) => raw,
        };
        let value = raw.to_owned();
        match in_range(raw) {
            None => warnings.push(FilenameWarning::NotANumber { setting, value }),
            Some(false) => warnings.push(FilenameWarning::OutOfRange { setting, value }),
            Some(true) => {}
        }
    };
    check(Setting::Threshold, &|text| parse_float(text).map(|value| (0.0..=1.0).contains(&value)));
    check(Setting::PauseTime, &|text| parse_float(text).map(|value| value >= 0.0));
    check(Setting::DelayTime, &|text| parse_int(text).map(|value| value >= 0));
    check(Setting::Loops, &|text| parse_int(text).map(|value| value >= 1));
    // A fade image's method is ignored, which says enough about it
    if !fade {
        check(Setting::ComparisonMethod, &|text| parse_int(text).map(|value| value >= 0));
    }
    // Without `{f}`, a lone `~` is part of the name, as it is for AutoSplit
    if fade {
        check(Setting::FadeTime, &|text| parse_int(text).map(|value| (MIN_FADE_TIME..=MAX_FADE_TIME).contains(&value)));
    }
    if let Delimited::Invalid(count, ..) = delimited(filename, '{', '}') {
        warnings.push(FilenameWarning::Delimiters { setting: Setting::Flags, count });
    }

    if fade {
        if !matches!(delimited(filename, '^', '^'), Delimited::Absent) {
            warnings.push(FilenameWarning::NotForFade(Setting::ComparisonMethod));
        }
    } else if let Some(index) =
        comparison_method_from_filename(filename).filter(|index| ComparisonMethod::from_index(*index).is_none())
    {
        warnings.push(FilenameWarning::UnknownComparisonMethod(index));
    }
    if !fade && matches!(delimited(filename, '~', '~'), Delimited::Value(..)) {
        warnings.push(FilenameWarning::FadeOnly(Setting::FadeTime));
    }
    if let Err(warning) = flags {
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
    /// `~1000~`, a fade image's fade time.
    FadeTime,
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
/// it: a `(` and a `)` around a value and so on, or everything from the first delimiter to the last
/// when they appear more often. The parts together are `filename`. A value inside another one is
/// left as part of the other.
pub fn name_parts(filename: &str) -> Vec<NamePart> {
    // ASCII lowercasing keeps every byte where it is
    let lower = filename.to_ascii_lowercase();
    let warnings = filename_warnings(&lower);
    let warning = |setting: Setting| {
        warnings.iter().find(|warning| match warning {
            FilenameWarning::NotANumber { setting: of, .. }
            | FilenameWarning::OutOfRange { setting: of, .. }
            | FilenameWarning::Delimiters { setting: of, .. } => *of == setting,
            FilenameWarning::UnknownComparisonMethod(_) => setting == Setting::ComparisonMethod,
            FilenameWarning::FadeOnly(of) | FilenameWarning::NotForFade(of) => *of == setting,
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
        (PartKind::Threshold, Setting::Threshold),
        (PartKind::Pause, Setting::PauseTime),
        (PartKind::Delay, Setting::DelayTime),
        (PartKind::Loops, Setting::Loops),
        (PartKind::Method, Setting::ComparisonMethod),
        (PartKind::Flags, Setting::Flags),
        (PartKind::FadeTime, Setting::FadeTime),
    ];
    let fade = parse_flags(&lower).is_ok_and(|flags| flags & FADE_FLAG != 0);
    for (kind, setting) in values {
        let (open, close) = setting.delimiters();
        let (start, end, raw) = match delimited(&lower, open, close) {
            Delimited::Absent => continue,
            // A lone `~` is part of the name, unless it's a fade image
            Delimited::Invalid(..) if setting == Setting::FadeTime && !fade => continue,
            Delimited::Value(raw, first, second) => (first, second + 1, raw),
            // From the first delimiter to the last, since none of it is read
            Delimited::Invalid(_, first, last) => (first, last + 1, ""),
        };
        let (meaning, ignored) = match (kind, warning(setting)) {
            (_, Some(warning)) => (format!("ignored: {warning}"), true),
            (PartKind::Flags, None) => match parse_flags(&lower) {
                Ok(flags) => (flags_meaning(flags), false),
                Err(warning) => (format!("ignored: {warning}"), true),
            },
            (_, None) => (value_meaning(kind, raw), false),
        };
        spans.push((start, end, kind, meaning, ignored));
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
        PartKind::FadeTime => format!("fade time: the fade to the color takes about {raw} ms"),
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
        (FADE_FLAG, "fade: reacts to a gradual fade to the image's color, not to a cut"),
    ];
    let meanings: Vec<&str> = named.iter().filter(|(flag, _)| flags & flag != 0).map(|(_, meaning)| *meaning).collect();
    if meanings.is_empty() { "flags: none".into() } else { meanings.join("; ") }
}

/// The settings a split image's file name carries, from a preset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplitSpec<'a> {
    /// `reset` and `start_auto_splitter` make the reset and start images.
    pub name: &'a str,
    /// `None` for the profile's default similarity threshold.
    pub threshold: Option<f64>,
    /// Seconds, or `None` for the profile's default pause time.
    pub pause: Option<f64>,
    /// Milliseconds, or `None` for the profile's default delay time.
    pub delay: Option<u32>,
    pub dummy: bool,
    pub below: bool,
    /// Milliseconds, for a fade split, `{f}`. `None` for one made from a screenshot.
    pub fade_time: Option<u32>,
}

/// The file name of split image `index` (from 0) of `count`, like AutoMask names them. Only the
/// settings the split has are in it, so the others are the profile's. With the profile's
/// `defaults`, it asks for L2 Norm, which presets are made for, when the profile compares another
/// way.
///
/// `reset_(0.96)_[0.1].png`, `05_Bottle_(0.93)_[35]_#1100#_{b}.png`: the number has as many
/// digits as `count`, so the images sort in order.
pub fn build_filename(spec: &SplitSpec, index: usize, count: usize, defaults: Option<&SplitDefaults>) -> String {
    let mut name = match spec.name {
        START_KEYWORD | RESET_KEYWORD => spec.name.to_owned(),
        _ => format!("{index:0width$}_{}", spec.name, width = count.to_string().len()),
    };
    if let Some(threshold) = spec.threshold {
        name += &format!("_({threshold})");
    }
    if let Some(pause) = spec.pause {
        name += &format!("_[{pause}]");
    }
    if let Some(delay) = spec.delay {
        name += &format!("_#{delay}#");
    }
    if let Some(fade_time) = spec.fade_time {
        name += &format!("_~{fade_time}~");
    }
    // Fade images always compare by color
    let other_method = defaults.is_some_and(|defaults| defaults.comparison_method != ComparisonMethod::L2Norm.index());
    if other_method && spec.fade_time.is_none() {
        name += "_^0^";
    }
    let flags: String = [(spec.dummy, 'd'), (spec.below, 'b'), (spec.fade_time.is_some(), 'f')]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, c)| c)
        .collect();
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
    if let Some(c) = name.chars().find(|c| "#@{}()[]^~".contains(*c)) {
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
