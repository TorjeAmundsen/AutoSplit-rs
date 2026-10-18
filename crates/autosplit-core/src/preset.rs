//! Split image presets (`preset.toml`): the masks and settings of a category's split images,
//! which turn screenshots into split images named the way AutoSplit reads them. And the splits
//! library (`splits.toml`), ready-made splits to put in presets.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::filename::{DEFAULT_FADE_TIME, RESET_KEYWORD, START_KEYWORD, SplitSpec, build_filename};
use crate::split_image::SplitDefaults;

/// The file a preset folder has its preset in.
pub const PRESET_FILE: &str = "preset.toml";
/// The file a splits library folder has its splits in.
pub const LIBRARY_FILE: &str = "splits.toml";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Preset {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub game: String,
    #[serde(rename = "split")]
    pub splits: Vec<PresetSplit>,
}

/// One split image of a preset.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PresetSplit {
    /// `reset` and `start_auto_splitter` make the reset and start images. A preset can have up
    /// to [`MAX_GROUP_SIZE`](crate::split_folder::MAX_GROUP_SIZE) of each, which the split image folder compares like a group.
    pub name: String,
    /// Compared for the same split as the split before it, so its split image gets that split's
    /// number. Needs the profile to group split images by number. Only for splits that aren't
    /// reset or start images, after one that isn't either.
    #[serde(default, skip_serializing_if = "is_default")]
    pub with_previous: bool,
    /// The mask's PNG, relative to the preset's folder. Where its alpha is 255, the screenshot is
    /// kept, or a fade split's color. A fade split without one is the whole screen.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mask: String,
    /// `#rrggbb`: the color a fade split splits on a fade to, instead of a screenshot. Kept as
    /// written, which can be wrong while it's being typed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade: Option<String>,
    /// How long a fade split's fade takes, in milliseconds. Always written for fade splits, so a
    /// later default doesn't change them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_time: Option<u32>,
    /// While editing, what switching between a screenshot and a fade brings back. Never saved.
    #[serde(skip)]
    pub remembered: Remembered,
    /// Without one, the profile's default similarity threshold applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
    /// Seconds. Without one, the profile's default pause time applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pause: Option<f64>,
    /// Milliseconds. Without one, the profile's default delay time applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delay: Option<u32>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub dummy: bool,
    /// Splits once the similarity drops back below the threshold, `{b}`.
    #[serde(default, skip_serializing_if = "is_default")]
    pub below: bool,
    /// A savestate for a practice tool that loads them on the console, like gz for Ocarina of
    /// Time, relative to the preset's folder, to get to the split's screen for its screenshot.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub savestate: String,
    /// How to use the savestate.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub instructions: String,
}

/// What a split being edited had before it switched between a screenshot and a fade, so
/// switching back brings it back. It isn't part of what the split is: splits that differ only in
/// it are the same, so switching there and back is no change.
#[derive(Clone, Debug, Default)]
pub struct Remembered {
    /// The threshold before it became a fade split.
    pub threshold: Option<Option<f64>>,
    /// The color and fade time before it became a screenshot split.
    pub fade: Option<(String, Option<u32>)>,
}

impl PartialEq for Remembered {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl PresetSplit {
    /// Whether it makes the reset or start image.
    pub fn is_keyword(&self) -> bool {
        self.name == RESET_KEYWORD || self.name == START_KEYWORD
    }

    /// Whether it splits on a fade to a color, `{f}`, instead of a screenshot.
    pub fn is_fade(&self) -> bool {
        self.fade.is_some()
    }

    /// A fade split's color, once it's written right.
    pub fn fade_color(&self) -> Option<[u8; 3]> {
        parse_color(self.fade.as_deref()?)
    }

    /// What the split image's file name says.
    pub fn spec(&self) -> SplitSpec<'_> {
        SplitSpec {
            name: &self.name,
            threshold: self.threshold,
            pause: self.pause,
            delay: self.delay,
            dummy: self.dummy,
            below: self.below,
            fade_time: self.is_fade().then(|| self.fade_time.unwrap_or(DEFAULT_FADE_TIME)),
        }
    }
}

impl Preset {
    /// Whether the folder its split images go in has to be read with split images grouped by
    /// number: it has splits compared together, or several reset or start images.
    pub fn needs_grouping(&self) -> bool {
        (0..self.splits.len()).any(|index| self.joins_previous(index) || self.keyword_place(index) > Some(1))
    }

    /// Whether split `index` is compared for the same split as the one before it: it asks to
    /// be, and can be.
    pub fn joins_previous(&self, index: usize) -> bool {
        self.splits.get(index).is_some_and(|split| split.with_previous) && self.can_join_previous(index)
    }

    /// Whether split `index` can be compared for the same split as the one before it: neither
    /// it nor the one before is a reset or start image.
    pub fn can_join_previous(&self, index: usize) -> bool {
        let Some(split) = self.splits.get(index) else { return false };
        let previous = index.checked_sub(1).and_then(|previous| self.splits.get(previous));
        !split.is_keyword() && previous.is_some_and(|previous| !previous.is_keyword())
    }

    /// Which reset or start image of its kind split `index` is, from 1, or `None` for a split
    /// that's neither.
    pub fn keyword_place(&self, index: usize) -> Option<usize> {
        let split = self.splits.get(index).filter(|split| split.is_keyword())?;
        Some(self.splits[..=index].iter().filter(|other| other.name == split.name).count())
    }

    /// Each split's number: its place in the list, where a split compared with the one before it
    /// has that one's number, and the second reset or start image and on the first one's. With the count of
    /// numbers, which sets how many digits they get.
    pub fn numbers(&self) -> (Vec<usize>, usize) {
        let mut numbers = Vec::with_capacity(self.splits.len());
        let mut next = 0;
        for (index, split) in self.splits.iter().enumerate() {
            let first_of_kind = || self.splits.iter().position(|other| other.name == split.name);
            let number = match self.keyword_place(index) {
                _ if self.joins_previous(index) => next - 1,
                Some(place) if place > 1 => first_of_kind().map_or(next, |first| numbers[first]),
                _ => {
                    next += 1;
                    next - 1
                }
            };
            numbers.push(number);
        }
        (numbers, next)
    }

    /// The file names of the preset's split images, in order. With the profile's `defaults`,
    /// see [`build_filename`]. A second reset or start image and on get their place after the
    /// keyword, like `reset_2_(0.95).png`, since they'd have the same name otherwise.
    pub fn file_names(&self, defaults: Option<&SplitDefaults>) -> Vec<String> {
        let (numbers, count) = self.numbers();
        (self.splits.iter().enumerate())
            .map(|(index, split)| {
                let name = build_filename(&split.spec(), numbers[index], count, defaults);
                match self.keyword_place(index) {
                    Some(place) if place > 1 => {
                        let rest = &name[split.name.len()..];
                        format!("{}_{place}{rest}", split.name)
                    }
                    _ => name,
                }
            })
            .collect()
    }

    /// The savestates to copy for the practice tool, each with the name it's copied as: the split's
    /// number and name like its split image, `_SEE_INSTRUCTIONS` when it has instructions, and
    /// the savestate's extension, and the place of a reset or start image past the first. Splits
    /// compared together share a number, so they need their own names: the editor doesn't save
    /// a preset without, and Make Split Images does nothing with one until it's fixed.
    pub fn savestate_copies(&self) -> Vec<(&str, String)> {
        let (numbers, count) = self.numbers();
        let width = count.to_string().len();
        let name = |index: usize, split: &PresetSplit| {
            let extension = Path::new(&split.savestate).extension().map(|extension| extension.to_string_lossy());
            let see = if split.instructions.is_empty() { "" } else { "_SEE_INSTRUCTIONS" };
            let extension = extension.map_or(String::new(), |extension| format!(".{extension}"));
            let number = numbers[index];
            let place = match self.keyword_place(index) {
                Some(place) if place > 1 => format!("_{place}"),
                _ => String::new(),
            };
            format!("{number:0width$}_{}{place}{see}{extension}", split.name)
        };
        self.splits
            .iter()
            .enumerate()
            .filter(|(_, split)| !split.savestate.is_empty())
            .map(|(index, split)| (split.savestate.as_str(), name(index, split)))
            .collect()
    }

    /// The split above that `index` is compared together with and has the same name as, the way
    /// Windows compares file names: ignoring case, `Bär` like `BÄR`, and dots and spaces at the
    /// end, which it drops, `Boss.` like `Boss`. They'd share a number, so their split images and
    /// savestate copies would get the same name.
    pub fn same_name_in_group(&self, index: usize) -> Option<usize> {
        let plain = |name: &str| name.trim().trim_end_matches(['.', ' ']).to_lowercase();
        let name = plain(&self.splits.get(index)?.name);
        (self.group(index).start..index).find(|&other| plain(&self.splits[other].name) == name)
    }

    /// The splits compared together with split `index`, itself too, as the range of their
    /// indexes: from the split before the ones that join it, through the last that joins.
    pub fn group(&self, index: usize) -> std::ops::Range<usize> {
        let count = self.splits.len();
        if index >= count {
            return count..count;
        }
        let start = (0..=index).rev().find(|&first| !self.joins_previous(first)).unwrap_or(0);
        let end = (index + 1..count).find(|&next| !self.joins_previous(next)).unwrap_or(count);
        start..end
    }

    /// The file name of a profile made for this preset: the game's initials, then the preset's
    /// name in lowercase words joined by `_`, like `tlozoot_no_wrong_warp.toml`.
    pub fn profile_file_name(&self) -> String {
        let initials: String = (self.game.split_whitespace())
            .filter_map(|word| word.chars().find(|c| c.is_alphanumeric()))
            .flat_map(char::to_lowercase)
            .collect();
        let name = self.name.to_lowercase();
        let words: Vec<&str> = name.split(|c: char| !c.is_alphanumeric()).filter(|word| !word.is_empty()).collect();
        let stem = match (initials.as_str(), words.join("_")) {
            ("", words) if words.is_empty() => "settings".to_owned(),
            ("", words) => words,
            (initials, words) if words.is_empty() => initials.to_owned(),
            (initials, words) => format!("{initials}_{words}"),
        };
        format!("{stem}.toml")
    }
}

/// The ready-made splits of a game. A key it doesn't have is an error, like in a preset: a
/// misspelled `game` would otherwise leave it without one, which no preset is for.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SplitLibrary {
    pub game: String,
    #[serde(rename = "split")]
    pub splits: Vec<LibrarySplit>,
}

/// A ready-made split, with what it's for and a screenshot it's made for.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LibrarySplit {
    #[serde(flatten)]
    pub split: PresetSplit,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// A screenshot the mask fits, relative to the library's folder.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub base_image: String,
    /// The next image of the ready-made split above, like the boss's flash after a dummy for its
    /// intro text. They're imported together, in order.
    #[serde(default, skip_serializing_if = "is_default")]
    pub then: bool,
}

/// The keys a preset's `[[split]]` table can have, which are [`PresetSplit`]'s fields.
pub const SPLIT_KEYS: &[&str] = &[
    "name",
    "with_previous",
    "mask",
    "fade",
    "fade_time",
    "threshold",
    "pause",
    "delay",
    "dummy",
    "below",
    "savestate",
    "instructions",
];

/// The keys a splits library's `[[split]]` table can have besides [`SPLIT_KEYS`].
const LIBRARY_SPLIT_KEYS: &[&str] = &["description", "base_image", "then"];

/// `#rrggbb` as RGB, in either case. The `#` is needed.
pub fn parse_color(text: &str) -> Option<[u8; 3]> {
    // Only hex digits: `from_str_radix` would take a `+` too
    let hex =
        text.trim().strip_prefix('#').filter(|hex| hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()))?;
    let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum PresetError {
    #[error("'{}' couldn't be read: {reason}", .path.display())]
    Unreadable { path: PathBuf, reason: String },
    #[error("'{}' isn't a valid {}: {reason}", .path.display(), kind_of(.path))]
    Invalid { path: PathBuf, reason: String },
    #[error("'{}' isn't a {}: it has no [[split]] tables.", .path.display(), kind_of(.path))]
    NotAPreset { path: PathBuf },
    #[error("'{}' couldn't be written: {reason}", .path.display())]
    Unwritable { path: PathBuf, reason: String },
}

/// What a file of splits is called in its errors: a split library's `splits.toml`, or a preset.
fn kind_of(path: &Path) -> &'static str {
    if path.file_name().is_some_and(|name| name == LIBRARY_FILE) { "split library" } else { "split image preset" }
}

/// A preset and the folder its masks and savestates are in.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadedPreset {
    pub folder: PathBuf,
    pub preset: Preset,
}

impl LoadedPreset {
    /// A path in the preset, like a mask or savestate.
    pub fn file(&self, relative: &str) -> PathBuf {
        self.folder.join(relative)
    }
}

/// A splits library and the folder its masks and base images are in.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadedLibrary {
    pub folder: PathBuf,
    pub library: SplitLibrary,
}

/// Reads TOML with `[[split]]` tables, which presets and libraries have and profiles don't. A
/// split's key that isn't in `split_keys` is an error, like a misspelled `treshold`: it would
/// otherwise be left out without a word, and gone from the file once the preset is saved.
fn parse_with_splits<T: serde::de::DeserializeOwned>(
    text: &str,
    path: &Path,
    split_keys: &[&[&str]],
) -> Result<T, PresetError> {
    let invalid = |reason: String| PresetError::Invalid { path: path.to_owned(), reason };
    let table: toml::Table = text.parse().map_err(|error: toml::de::Error| invalid(error.message().to_owned()))?;
    let Some(splits) = table.get("split").and_then(toml::Value::as_array) else {
        return Err(PresetError::NotAPreset { path: path.to_owned() });
    };
    let known = |key: &str| split_keys.iter().any(|keys| keys.contains(&key));
    for (index, split) in splits.iter().enumerate() {
        let Some(split) = split.as_table() else { continue };
        if let Some(key) = split.keys().find(|key| !known(key)) {
            let name = split.get("name").and_then(toml::Value::as_str).unwrap_or_default();
            let keys = split_keys.concat().join(", ");
            return Err(invalid(format!("split {} ({name}) has `{key}`, which isn't one of {keys}", index + 1)));
        }
    }
    toml::from_str(text).map_err(|error| invalid(error.message().to_owned()))
}

fn read(path: &Path) -> Result<String, PresetError> {
    std::fs::read_to_string(path)
        .map_err(|error| PresetError::Unreadable { path: path.to_owned(), reason: error.to_string() })
}

impl Preset {
    pub fn load(path: &Path) -> Result<Self, PresetError> {
        Self::parse(&read(path)?, path)
    }

    /// `path` is only for the errors.
    pub fn parse(text: &str, path: &Path) -> Result<Self, PresetError> {
        parse_with_splits(text, path, &[SPLIT_KEYS])
    }

    pub fn to_toml(&self) -> String {
        toml::to_string(self).expect("presets always serialize")
    }
}

impl SplitLibrary {
    pub fn load(path: &Path) -> Result<Self, PresetError> {
        Self::parse(&read(path)?, path)
    }

    /// `path` is only for the errors.
    pub fn parse(text: &str, path: &Path) -> Result<Self, PresetError> {
        let library: Self = parse_with_splits(text, path, &[SPLIT_KEYS, LIBRARY_SPLIT_KEYS])?;
        if let Some(first) = library.splits.first().filter(|first| first.then) {
            let reason = format!("split 1 ({}) has `then`, but there's no split above it to follow", first.split.name);
            return Err(PresetError::Invalid { path: path.to_owned(), reason });
        }
        Ok(library)
    }

    /// The ready-made splits: each split with the ones that follow it with `then`.
    pub fn ready_made(&self) -> impl Iterator<Item = &[LibrarySplit]> {
        self.splits.chunk_by(|_, next| next.then)
    }
}

/// The folders in `directory` that have `file`, by name, or none when it can't be read. Hidden
/// folders are left out, like what's left of a deleted preset.
fn folders_with(directory: &Path, file: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(directory) else { return Vec::new() };
    let mut folders: Vec<PathBuf> = entries
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|path| !path.file_name().is_some_and(|name| name.to_string_lossy().starts_with('.')))
        .filter(|path| path.join(file).is_file())
        .collect();
    folders.sort();
    folders
}

/// Sorted by game, then name, ignoring case, like they're listed.
fn by_game_and_name(game: &str, name: &str) -> (String, String) {
    (game_key(game), name.to_lowercase())
}

/// What tells games apart: their names ignoring case and extra spaces, so
/// `the legend of zelda:  ocarina of time` is the same game as `The Legend of Zelda: Ocarina of
/// Time`, and their presets are listed together.
pub fn game_key(game: &str) -> String {
    game.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// The presets in `directory`'s folders, sorted by game and name, and the ones that couldn't be
/// loaded.
pub fn load_presets(directory: &Path) -> (Vec<LoadedPreset>, Vec<PresetError>) {
    let mut presets = Vec::new();
    let mut errors = Vec::new();
    for folder in folders_with(directory, PRESET_FILE) {
        match Preset::load(&folder.join(PRESET_FILE)) {
            Ok(preset) => presets.push(LoadedPreset { folder, preset }),
            Err(error) => errors.push(error),
        }
    }
    presets.sort_by_key(|loaded| by_game_and_name(&loaded.preset.game, &loaded.preset.name));
    (presets, errors)
}

/// The splits libraries in `directory`'s folders, sorted by game, and the ones that couldn't be
/// loaded.
pub fn load_libraries(directory: &Path) -> (Vec<LoadedLibrary>, Vec<PresetError>) {
    let mut libraries = Vec::new();
    let mut errors = Vec::new();
    for folder in folders_with(directory, LIBRARY_FILE) {
        match SplitLibrary::load(&folder.join(LIBRARY_FILE)) {
            Ok(library) => libraries.push(LoadedLibrary { folder, library }),
            Err(error) => errors.push(error),
        }
    }
    libraries.sort_by_key(|loaded| loaded.library.game.to_lowercase());
    (libraries, errors)
}

/// The folder in a preset's folder that its savestates go in.
pub const SAVESTATES_FOLDER: &str = "savestates";

/// The note in a preset's savestate folder that says not to copy them from there.
pub const SAVESTATE_NOTE_FILE: &str = "DO_NOT_COPY.txt";

const SAVESTATE_NOTE: &str = "Don't copy the savestates from this folder yourself.

Presets reuse savestates for several splits, and their file names aren't in the order of the splits.
Use Copy Savestates in Make Split Images instead. It copies each split's savestate, numbered and
named like its split image.
";

/// Windows' names for devices, which no file or folder can have, with any extension.
const RESERVED_NAMES: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2",
    "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// `name` as a file or folder name that works on every system: words joined by `_`, without the
/// characters Windows doesn't allow, trailing dots or its device names. `fallback` when nothing's
/// left.
pub fn sanitize_file_name(name: &str, fallback: &str) -> String {
    let kept: String = name
        .chars()
        .map(|c| if c.is_whitespace() { '_' } else { c })
        .filter(|c| !r#"<>:"/\|?*"#.contains(*c) && !c.is_control())
        .collect();
    let trimmed = kept.trim_matches(|c| c == '_' || c == '.');
    if trimmed.is_empty() {
        return fallback.to_owned();
    }
    // Windows reads the part before the first dot as the device, so the mark goes there
    let (stem, rest) = trimmed.split_at(trimmed.find('.').unwrap_or(trimmed.len()));
    if RESERVED_NAMES.contains(&stem.to_lowercase().as_str()) { format!("{stem}_{rest}") } else { trimmed.to_owned() }
}

/// `file`, or `name (1).ext`, `name (2).ext` and so on when `taken` says it's in use.
pub fn unique_file_name(file: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(file) {
        return file.to_owned();
    }
    let path = Path::new(file);
    let stem = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
    let extension = path.extension().map(|extension| format!(".{}", extension.to_string_lossy())).unwrap_or_default();
    (1..).map(|n| format!("{stem} ({n}){extension}")).find(|name| !taken(name)).expect("some number is free")
}

/// A new folder in `presets` for a preset named `name`, which doesn't exist yet.
pub fn new_preset_folder(presets: &Path, name: &str) -> PathBuf {
    let base = sanitize_file_name(name, "Preset");
    let free = |folder: &str| !presets.join(folder).exists();
    (1..)
        .map(|n| if n == 1 { base.clone() } else { format!("{base}_{n}") })
        .find(|folder| free(folder))
        .map(|folder| presets.join(folder))
        .expect("some number is free")
}

/// A path inside a preset's folder, as presets write it: relative, with `/`.
pub(crate) fn relative_text(path: &Path) -> String {
    path.components().map(|part| part.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/")
}

/// Finds a mask or savestate: relative to `from`, or a picked file's absolute path.
fn find(from: &Path, path: &str) -> Result<PathBuf, PresetError> {
    let picked = Path::new(path);
    let file = if picked.is_absolute() { picked.to_owned() } else { from.join(picked) };
    std::fs::canonicalize(&file).map_err(|error| {
        let reason = match error.kind() {
            std::io::ErrorKind::NotFound => "the file doesn't exist".to_owned(),
            _ => error.to_string(),
        };
        PresetError::Unreadable { path: file, reason }
    })
}

/// Saves `preset` into the preset folder `target`, making it when needed. Its masks and
/// savestates are paths relative to `source`, the folder it was read from, or to `target` when
/// there's none, or absolute paths of files picked for it. Files outside `target` are copied in:
/// masks named after their split, savestates into `savestates` under their own names, each file
/// once, and never over a file that's there. Every file is found before anything is written, and
/// when copying or writing `preset.toml` fails, what this save made is removed again, so a failed
/// save leaves the folder as it was. Afterwards, the masks and savestates the preset used before
/// and doesn't now are deleted, and the savestate folder gets [`SAVESTATE_NOTE_FILE`]. Returns the preset as written, with relative paths.
pub fn save_preset(source: Option<&Path>, target: &Path, preset: &Preset) -> Result<Preset, PresetError> {
    let from = source.unwrap_or(target);
    let mut found = Vec::new();
    for split in &preset.splits {
        for path in [&split.mask, &split.savestate] {
            found.push(if path.is_empty() { None } else { Some(find(from, path)?) });
        }
    }
    let new_folder = !target.exists();
    std::fs::create_dir_all(target).map_err(|error| unwritable(target, error))?;
    let before = Preset::load(&target.join(PRESET_FILE)).ok();
    let mut made = Vec::new();
    match write_preset(target, preset, found, &mut made) {
        Ok(saved) => {
            remove_unused_files(target, before.as_ref(), &saved);
            let note = target.join(SAVESTATES_FOLDER).join(SAVESTATE_NOTE_FILE);
            if note.parent().is_some_and(Path::is_dir) && !note.exists() {
                // Only a note: the preset is saved without it
                let _ = std::fs::write(note, SAVESTATE_NOTE);
            }
            Ok(saved)
        }
        Err(error) => {
            if new_folder {
                let _ = std::fs::remove_dir_all(target);
            } else {
                // Files first, then the folders they were in
                for path in made.iter().rev() {
                    let _ = if path.is_dir() { std::fs::remove_dir(path) } else { std::fs::remove_file(path) };
                }
            }
            Err(error)
        }
    }
}

/// The start of the hidden name a preset's folder gets while it's deleted.
const DELETING_PREFIX: &str = ".deleting-";

/// Deletes the preset folder `folder`. It's first renamed to a hidden name next to it, which
/// fails without changing anything when a file in it is open, as on Windows. Once renamed it's
/// no longer a preset, so a delete that stops partway leaves a hidden folder instead of half a
/// preset. Those are deleted again here, the next time.
pub fn delete_preset(folder: &Path) -> Result<(), PresetError> {
    let parent = folder.parent().unwrap_or(Path::new("."));
    if let Ok(entries) = std::fs::read_dir(parent) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with(DELETING_PREFIX) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
    let name = folder.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let hidden = (1..)
        .map(|n| parent.join(format!("{DELETING_PREFIX}{name}-{n}")))
        .find(|path| !path.exists())
        .expect("some number is free");
    std::fs::rename(folder, &hidden).map_err(|error| PresetError::Unwritable {
        path: folder.to_owned(),
        reason: format!("{error}. Close any program that has a file in it open, then try again"),
    })?;
    // Not a preset anymore, whether or not all of it goes now
    let _ = std::fs::remove_dir_all(&hidden);
    Ok(())
}

pub(crate) fn unwritable(path: &Path, error: std::io::Error) -> PresetError {
    PresetError::Unwritable { path: path.to_owned(), reason: error.to_string() }
}

/// Copies `found`, each split's mask and savestate in turn, into `target` when they're outside
/// it, and writes `preset.toml`. Records the files and folders it makes in `made`.
fn write_preset(
    target: &Path,
    preset: &Preset,
    found: Vec<Option<PathBuf>>,
    made: &mut Vec<PathBuf>,
) -> Result<Preset, PresetError> {
    let root = std::fs::canonicalize(target).unwrap_or_else(|_| target.to_owned());
    // Names given to copies in this save, which the folder doesn't have yet
    let mut given: HashSet<String> = HashSet::new();
    let mut copied: HashMap<PathBuf, String> = HashMap::new();
    let mut saved = preset.clone();
    let mut found = found.into_iter();
    for split in &mut saved.splits {
        let name = split.name.clone();
        for (field, savestate) in [(&mut split.mask, false), (&mut split.savestate, true)] {
            let Some(file) = found.next().flatten() else { continue };
            if let Ok(inside) = file.strip_prefix(&root) {
                *field = relative_text(inside);
                continue;
            }
            if let Some(copy) = copied.get(&file) {
                *field = copy.clone();
                continue;
            }
            let (folder, wanted) = if savestate {
                let original = file.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
                (target.join(SAVESTATES_FOLDER), sanitize_file_name(&original, "savestate"))
            } else {
                (target.to_owned(), format!("{}.png", sanitize_file_name(&name, "mask")))
            };
            if !folder.exists() {
                std::fs::create_dir_all(&folder).map_err(|error| unwritable(&folder, error))?;
                made.push(folder.clone());
            }
            let prefix = if savestate { format!("{SAVESTATES_FOLDER}/") } else { String::new() };
            let free = unique_file_name(&wanted, |candidate| {
                folder.join(candidate).exists() || given.contains(&format!("{prefix}{candidate}").to_lowercase())
            });
            let copy = folder.join(&free);
            std::fs::copy(&file, &copy).map_err(|error| unwritable(&copy, error))?;
            made.push(copy);
            let relative = format!("{prefix}{free}");
            given.insert(relative.to_lowercase());
            copied.insert(file, relative.clone());
            *field = relative;
        }
    }
    let preset_file = target.join(PRESET_FILE);
    let temporary = target.join(format!("{PRESET_FILE}.tmp"));
    let written = std::fs::write(&temporary, saved.to_toml()).and_then(|()| std::fs::rename(&temporary, &preset_file));
    if let Err(error) = written {
        let _ = std::fs::remove_file(&temporary);
        return Err(unwritable(&preset_file, error));
    }
    Ok(saved)
}

/// Deletes the masks and savestates `before` used that `now` doesn't, compared as the files they
/// are, however their paths are written. Files the preset never used aren't its to delete.
fn remove_unused_files(target: &Path, before: Option<&Preset>, now: &Preset) {
    let Ok(root) = std::fs::canonicalize(target) else { return };
    let paths = |preset: &Preset| -> Vec<PathBuf> {
        (preset.splits.iter())
            .flat_map(|split| [&split.mask, &split.savestate])
            .filter(|path| !path.is_empty())
            // Missing files are left out: there's nothing to delete
            .filter_map(|path| std::fs::canonicalize(root.join(path)).ok())
            .collect()
    };
    let used: HashSet<PathBuf> = paths(now).into_iter().collect();
    // Only in the preset's own folder, whatever the old file says
    for old in before.map(paths).unwrap_or_default() {
        if old.starts_with(&root) && !used.contains(&old) && old.is_file() {
            let _ = std::fs::remove_file(old);
        }
    }
}
