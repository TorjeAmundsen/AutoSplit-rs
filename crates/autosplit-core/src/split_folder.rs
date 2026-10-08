//! Reading and validating a split image folder, matching AutoSplit's `split_parser.py`.

use std::ops::Range;
use std::path::{Path, PathBuf};

use crate::filename::{RESET_KEYWORD, START_KEYWORD, number_prefix};
use crate::profile::UserProfile;
use crate::split_image::{ImageType, LoadError, LoadWarning, SplitImage};

/// The images of a valid split image folder.
#[derive(Clone, Debug, Default)]
pub struct SplitSet {
    pub start: Option<SplitImage>,
    pub reset: Option<SplitImage>,
    /// In file name order, or by number then name when grouped.
    pub splits: Vec<SplitImage>,
    /// Ranges of `splits` compared for the same split. One image each, unless the profile
    /// groups split images by number.
    pub groups: Vec<Range<usize>>,
}

/// How many split images can be compared for the same split.
pub const MAX_GROUP_SIZE: usize = 4;

/// What validation needs to know besides the images.
#[derive(Clone, Copy, Debug, Default)]
pub struct ValidationContext {
    pub split_hotkey_set: bool,
    pub reset_hotkey_set: bool,
    pub pause_hotkey_set: bool,
    /// The timer only gets commands through hotkeys. LiveSplit, LiveSplit One and Molok don't need
    /// them.
    pub hotkeys_needed: bool,
    /// Split images that start with the same number are compared for the same split.
    pub group_split_images: bool,
}

impl ValidationContext {
    pub fn from_profile(profile: &UserProfile, hotkeys_needed: bool) -> Self {
        Self {
            split_hotkey_set: !profile.split_hotkey.is_empty(),
            reset_hotkey_set: !profile.reset_hotkey.is_empty(),
            pause_hotkey_set: !profile.pause_hotkey.is_empty(),
            hotkeys_needed,
            group_split_images: profile.group_split_images,
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
    #[error("'{0}' doesn't start with a number. Grouping split images by number needs one on every split image.")]
    NoNumberPrefix(String),
    #[error(
        "More than {MAX_GROUP_SIZE} split images start with the number {0}. Up to {MAX_GROUP_SIZE} can be compared for the same split."
    )]
    GroupTooLarge(String),
    #[error(
        "The split images that start with the number {0} have different loop counts. Images compared for the same split need the same @loops@."
    )]
    GroupLoopsDiffer(String),
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
pub fn load_profile_folder(profile: &UserProfile, hotkeys_needed: bool) -> FolderLoad {
    load_split_folder(
        Path::new(&profile.split_image_directory),
        &ValidationContext::from_profile(profile, hotkeys_needed),
    )
}

/// Loads the profile's folder to start a run, which also needs a split hotkey when the timer
/// only gets hotkeys (the start of AutoSplit's `__auto_splitter`).
pub fn load_profile_folder_to_start(profile: &UserProfile, hotkeys_needed: bool) -> FolderLoad {
    if profile.split_hotkey.is_empty() && hotkeys_needed {
        return FolderLoad {
            result: Err(ValidationError::NoSplitHotkey),
            load_errors: Vec::new(),
            warnings: Vec::new(),
        };
    }
    load_profile_folder(profile, hotkeys_needed)
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
    let set = SplitSet { start, reset, splits: images, groups: Vec::new() };
    FolderLoad { result: validate(set, context), load_errors, warnings }
}

fn validate(mut set: SplitSet, context: &ValidationContext) -> Result<SplitSet, ValidationError> {
    if set.splits.is_empty() {
        return Err(ValidationError::NoSplitImage);
    }
    if let Some(error) = set.start.iter().chain(&set.reset).find_map(SplitImage::load_error) {
        return Err(ValidationError::InvalidImage(error.clone()));
    }
    if set.start.is_some() && !context.split_hotkey_set && context.hotkeys_needed {
        return Err(ValidationError::StartWithoutSplitHotkey);
    }
    if set.reset.is_some() && !context.reset_hotkey_set && context.hotkeys_needed {
        return Err(ValidationError::ResetWithoutResetHotkey);
    }
    for image in &set.splits {
        if let Some(error) = image.load_error() {
            return Err(ValidationError::InvalidImage(error.clone()));
        }
        if !context.pause_hotkey_set && image.is_pause() && context.hotkeys_needed {
            return Err(ValidationError::PauseWithoutPauseHotkey(image.filename.clone()));
        }
        match image.image_type {
            ImageType::Reset => return Err(ValidationError::MultipleKeywordImages(RESET_KEYWORD)),
            ImageType::Start => return Err(ValidationError::MultipleKeywordImages(START_KEYWORD)),
            ImageType::Split => {}
        }
    }
    set.groups = group_splits(&mut set.splits, context.group_split_images)?;
    Ok(set)
}

/// The ranges of `splits` compared for the same split. Grouping by number sorts `splits` by
/// number, then name: "1_a.png" and "01_b.png" are both 1.
fn group_splits(splits: &mut [SplitImage], by_number: bool) -> Result<Vec<Range<usize>>, ValidationError> {
    if !by_number {
        return Ok((0..splits.len()).map(|index| index..index + 1).collect());
    }
    if let Some(image) = splits.iter().find(|image| number_prefix(&image.filename).is_none()) {
        return Err(ValidationError::NoNumberPrefix(image.filename.clone()));
    }
    fn digits(image: &SplitImage) -> &str {
        number_prefix(&image.filename).unwrap_or_default()
    }
    fn number(image: &SplitImage) -> (usize, &str) {
        let significant = digits(image).trim_start_matches('0');
        (significant.len(), significant)
    }
    // Stable, so images with the same number stay in name order
    splits.sort_by(|a, b| number(a).cmp(&number(b)));

    let mut groups = Vec::new();
    for group in splits.chunk_by(|a, b| number(a) == number(b)) {
        let first = &group[0];
        if group.len() > MAX_GROUP_SIZE {
            return Err(ValidationError::GroupTooLarge(digits(first).to_owned()));
        }
        if group.iter().any(|image| image.loops != first.loops) {
            return Err(ValidationError::GroupLoopsDiffer(digits(first).to_owned()));
        }
        let start = groups.last().map_or(0, |previous: &Range<usize>| previous.end);
        groups.push(start..start + group.len());
    }
    Ok(groups)
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
