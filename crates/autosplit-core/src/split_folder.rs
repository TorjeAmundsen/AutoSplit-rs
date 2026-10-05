//! Reading and validating a split image folder, matching AutoSplit's `split_parser.py`.

use std::path::{Path, PathBuf};

use crate::filename::{RESET_KEYWORD, START_KEYWORD};
use crate::profile::UserProfile;
use crate::split_image::{ImageType, LoadError, LoadWarning, SplitImage};

/// The images of a valid split image folder.
#[derive(Clone, Debug, Default)]
pub struct SplitSet {
    pub start: Option<SplitImage>,
    pub reset: Option<SplitImage>,
    /// In file name order.
    pub splits: Vec<SplitImage>,
}

/// What validation needs to know besides the images.
#[derive(Clone, Copy, Debug, Default)]
pub struct ValidationContext {
    pub split_hotkey_set: bool,
    pub reset_hotkey_set: bool,
    pub pause_hotkey_set: bool,
    /// Controlled by LiveSplit, which handles the timer itself so no hotkeys are needed.
    pub auto_controlled: bool,
}

impl ValidationContext {
    pub fn from_profile(profile: &UserProfile, auto_controlled: bool) -> Self {
        Self {
            split_hotkey_set: !profile.split_hotkey.is_empty(),
            reset_hotkey_set: !profile.reset_hotkey.is_empty(),
            pause_hotkey_set: !profile.pause_hotkey.is_empty(),
            auto_controlled,
        }
    }
}

/// Why a folder can't be used.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ValidationError {
    #[error("No split hotkey is set, so the timer can't be started.")]
    NoSplitHotkey,
    #[error("No split image folder is selected.")]
    NoSplitImageDirectory,
    #[error("The split image folder '{}' doesn't exist or can't be read.", .0.display())]
    InvalidDirectory(PathBuf),
    #[error("The split image folder has no split images.")]
    NoSplitImage,
    #[error("The folder has a start image, but no split hotkey is set to start the timer with.")]
    StartWithoutSplitHotkey,
    #[error("The folder has a reset image, but no reset hotkey is set.")]
    ResetWithoutResetHotkey,
    #[error(transparent)]
    InvalidImage(LoadError),
    #[error("'{0}' has the pause flag {{p}}, but no pause hotkey is set.")]
    PauseWithoutPauseHotkey(String),
    #[error("There can only be one image with the keyword '{0}'.")]
    MultipleKeywordImages(&'static str),
}

/// The outcome of loading a folder. Load errors are reported even when the folder is valid,
/// the way AutoSplit shows a message box per broken file while parsing.
#[derive(Debug)]
pub struct FolderLoad {
    pub result: Result<SplitSet, ValidationError>,
    pub load_errors: Vec<LoadError>,
    /// Filename values that are ignored. They don't stop the folder from being used.
    pub warnings: Vec<LoadWarning>,
}

/// Loads the profile's folder, e.g. to look for the start image.
pub fn load_profile_folder(profile: &UserProfile, auto_controlled: bool) -> FolderLoad {
    load_split_folder(
        Path::new(&profile.split_image_directory),
        &ValidationContext::from_profile(profile, auto_controlled),
    )
}

/// Loads the profile's folder to start a run, which also needs a split hotkey unless LiveSplit
/// controls the timer (the start of AutoSplit's `__auto_splitter`).
pub fn load_profile_folder_to_start(profile: &UserProfile, auto_controlled: bool) -> FolderLoad {
    if profile.split_hotkey.is_empty() && !auto_controlled {
        return FolderLoad {
            result: Err(ValidationError::NoSplitHotkey),
            load_errors: Vec::new(),
            warnings: Vec::new(),
        };
    }
    load_profile_folder(profile, auto_controlled)
}

/// `validate_before_parsing` (minus the capture region check) and `parse_and_validate_images`.
pub fn load_split_folder(directory: &Path, context: &ValidationContext) -> FolderLoad {
    let fail = |error| FolderLoad { result: Err(error), load_errors: Vec::new(), warnings: Vec::new() };
    if directory.as_os_str().is_empty() {
        return fail(ValidationError::NoSplitImageDirectory);
    }
    let Some(paths) = user_files(directory) else {
        return fail(ValidationError::InvalidDirectory(directory.to_owned()));
    };

    let mut images: Vec<SplitImage> = paths.iter().map(|path| SplitImage::load(path)).collect();
    let load_errors = images.iter().filter_map(|image| image.load_error().cloned()).collect();
    let warnings = images
        .iter()
        .flat_map(|image| {
            image.warnings.iter().map(|warning| LoadWarning { path: image.path.clone(), warning: warning.clone() })
        })
        .collect();

    let mut take_first =
        |image_type| images.iter().position(|image| image.image_type == image_type).map(|index| images.remove(index));
    let start = take_first(ImageType::Start);
    let reset = take_first(ImageType::Reset);
    let set = SplitSet { start, reset, splits: images };
    FolderLoad { result: validate(set, context), load_errors, warnings }
}

fn validate(set: SplitSet, context: &ValidationContext) -> Result<SplitSet, ValidationError> {
    if set.splits.is_empty() {
        return Err(ValidationError::NoSplitImage);
    }
    if let Some(error) = set.start.iter().chain(&set.reset).find_map(SplitImage::load_error) {
        return Err(ValidationError::InvalidImage(error.clone()));
    }
    if set.start.is_some() && !context.split_hotkey_set && !context.auto_controlled {
        return Err(ValidationError::StartWithoutSplitHotkey);
    }
    if set.reset.is_some() && !context.reset_hotkey_set && !context.auto_controlled {
        return Err(ValidationError::ResetWithoutResetHotkey);
    }
    for image in &set.splits {
        if let Some(error) = image.load_error() {
            return Err(ValidationError::InvalidImage(error.clone()));
        }
        if !context.pause_hotkey_set && image.is_pause() && !context.auto_controlled {
            return Err(ValidationError::PauseWithoutPauseHotkey(image.filename.clone()));
        }
        match image.image_type {
            ImageType::Reset => return Err(ValidationError::MultipleKeywordImages(RESET_KEYWORD)),
            ImageType::Start => return Err(ValidationError::MultipleKeywordImages(START_KEYWORD)),
            ImageType::Split => {}
        }
    }
    Ok(set)
}

/// Files in the folder, sorted by name. Hidden files, system files and folders are skipped.
fn user_files(directory: &Path) -> Option<Vec<PathBuf>> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(directory)
        .ok()?
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|path| is_user_file(path))
        .collect();
    paths.sort();
    Some(paths)
}

fn is_user_file(path: &Path) -> bool {
    let hidden_name = path.file_name().is_some_and(|name| name.to_string_lossy().starts_with('.'));
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if metadata.is_dir() || hidden_name {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
        const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
        return metadata.file_attributes() & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM) == 0;
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt;
        const UF_HIDDEN: u32 = 0x8000;
        return metadata.st_flags() & UF_HIDDEN == 0;
    }
    #[allow(unreachable_code)]
    true
}
