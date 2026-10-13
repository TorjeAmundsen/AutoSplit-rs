//! Split image presets (`preset.toml`): the masks and settings of a category's split images,
//! which turn screenshots into split images named the way AutoSplit reads them. And the splits
//! library (`splits.toml`), ready-made splits to put in presets.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::filename::{RESET_KEYWORD, START_KEYWORD, SplitSpec, build_filename};
use crate::split_image::SplitDefaults;

/// The file a preset folder has its preset in.
pub const PRESET_FILE: &str = "preset.toml";
/// The file a splits library folder has its splits in.
pub const LIBRARY_FILE: &str = "splits.toml";
/// The threshold of a split that doesn't set one.
pub const DEFAULT_THRESHOLD: f64 = 0.95;

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
    /// kept.
    pub mask: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<f64>,
    /// Seconds. Without one, AutoSplit's default pause time applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pause: Option<f64>,
    /// Milliseconds.
    #[serde(default, skip_serializing_if = "is_default")]
    pub delay: u32,
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

impl PresetSplit {
    pub fn threshold(&self) -> f64 {
        self.threshold.unwrap_or(DEFAULT_THRESHOLD)
    }

    /// Whether it makes the reset or start image.
    pub fn is_keyword(&self) -> bool {
        self.name == RESET_KEYWORD || self.name == START_KEYWORD
    }

    /// What the split image's file name says.
    pub fn spec(&self) -> SplitSpec<'_> {
        SplitSpec {
            name: &self.name,
            threshold: self.threshold(),
            pause: self.pause,
            delay: self.delay,
            dummy: self.dummy,
            below: self.below,
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

    /// The split above that `index` is compared together with and has the same name as, ignoring
    /// case like Windows file names do, `Bär` like `BÄR`. They'd share a number, so their split
    /// images and savestate copies would get the same name.
    pub fn same_name_in_group(&self, index: usize) -> Option<usize> {
        let split = self.splits.get(index)?;
        let name = split.name.trim().to_lowercase();
        let start = (0..=index).rev().find(|&first| !self.joins_previous(first))?;
        (start..index).find(|&other| self.splits[other].name.trim().to_lowercase() == name)
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

/// The ready-made splits of a game.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SplitLibrary {
    #[serde(default)]
    pub game: String,
    #[serde(rename = "split")]
    pub splits: Vec<LibrarySplit>,
}

/// A ready-made split, with what it's for and a screenshot it's made for.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LibrarySplit {
    #[serde(flatten)]
    pub split: PresetSplit,
    #[serde(default)]
    pub description: String,
    /// A screenshot the mask fits, relative to the library's folder.
    #[serde(default)]
    pub base_image: String,
}

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum PresetError {
    #[error("'{}' couldn't be read: {reason}", .path.display())]
    Unreadable { path: PathBuf, reason: String },
    #[error("'{}' isn't a valid preset: {reason}", .path.display())]
    Invalid { path: PathBuf, reason: String },
    #[error("'{}' isn't a split image preset: it has no [[split]] tables.", .path.display())]
    NotAPreset { path: PathBuf },
    #[error("'{}' couldn't be written: {reason}", .path.display())]
    Unwritable { path: PathBuf, reason: String },
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

/// Reads TOML with `[[split]]` tables, which presets and libraries have and profiles don't.
fn parse_with_splits<T: serde::de::DeserializeOwned>(text: &str, path: &Path) -> Result<T, PresetError> {
    let invalid = |reason: String| PresetError::Invalid { path: path.to_owned(), reason };
    let table: toml::Table = text.parse().map_err(|error: toml::de::Error| invalid(error.message().to_owned()))?;
    if !table.get("split").is_some_and(toml::Value::is_array) {
        return Err(PresetError::NotAPreset { path: path.to_owned() });
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
        parse_with_splits(text, path)
    }

    pub fn to_toml(&self) -> String {
        toml::to_string(self).expect("presets always serialize")
    }
}

impl SplitLibrary {
    pub fn load(path: &Path) -> Result<Self, PresetError> {
        parse_with_splits(&read(path)?, path)
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
    (game.to_lowercase(), name.to_lowercase())
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
fn relative_text(path: &Path) -> String {
    path.components().map(|part| part.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/")
}

fn canonical(path: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(path).ok()
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

fn unwritable(path: &Path, error: std::io::Error) -> PresetError {
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
    let root = canonical(target).unwrap_or_else(|| target.to_owned());
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
    let Some(root) = canonical(target) else { return };
    let paths = |preset: &Preset| -> Vec<PathBuf> {
        (preset.splits.iter())
            .flat_map(|split| [&split.mask, &split.savestate])
            .filter(|path| !path.is_empty())
            .filter_map(|path| canonical(&root.join(path)))
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
