//! Split image presets (`preset.toml`): the masks and settings of a category's split images,
//! which turn screenshots into split images named the way AutoSplit reads them. And the splits
//! library (`splits.toml`), ready-made splits to put in presets.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::filename::{SplitSpec, build_filename};
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
    /// `reset` and `start_auto_splitter` make the reset and start images.
    pub name: String,
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
    /// A savestate for the emulator, relative to the preset's folder, to get to the split's
    /// screen for its screenshot.
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
    /// The file names of the preset's split images, in order. With the profile's `defaults`,
    /// see [`build_filename`].
    pub fn file_names(&self, defaults: Option<&SplitDefaults>) -> Vec<String> {
        let count = self.splits.len();
        self.splits
            .iter()
            .enumerate()
            .map(|(index, split)| build_filename(&split.spec(), index, count, defaults))
            .collect()
    }

    /// The savestates to copy for the emulator, each with the name it's copied as: the split's
    /// number and name like its split image, `_SEE_INSTRUCTIONS` when it has instructions, and
    /// the savestate's extension.
    pub fn savestate_copies(&self) -> Vec<(&str, String)> {
        let width = self.splits.len().to_string().len();
        let name = |index: usize, split: &PresetSplit| {
            let extension = Path::new(&split.savestate).extension().map(|extension| extension.to_string_lossy());
            let see = if split.instructions.is_empty() { "" } else { "_SEE_INSTRUCTIONS" };
            let extension = extension.map_or(String::new(), |extension| format!(".{extension}"));
            format!("{index:0width$}_{}{see}{extension}", split.name)
        };
        self.splits
            .iter()
            .enumerate()
            .filter(|(_, split)| !split.savestate.is_empty())
            .map(|(index, split)| (split.savestate.as_str(), name(index, split)))
            .collect()
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

/// The folders in `directory` that have `file`, by name, or none when it can't be read.
fn folders_with(directory: &Path, file: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(directory) else { return Vec::new() };
    let mut folders: Vec<PathBuf> =
        entries.filter_map(|entry| Some(entry.ok()?.path())).filter(|path| path.join(file).is_file()).collect();
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
