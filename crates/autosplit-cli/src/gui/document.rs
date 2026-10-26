//! The profile being edited in the window, and the file it's saved in.

use std::path::{Path, PathBuf};

use autosplit_core::profile::{self, NEW_PROFILE_REGION, UserProfile, VIDEO_CAPTURE_DEVICE};

pub struct Document {
    /// As in its file: folders aren't resolved, so saving writes them back as they were.
    pub profile: UserProfile,
    pub path: Option<PathBuf>,
    /// As last loaded or saved, to tell whether there are changes to save.
    saved: UserProfile,
}

impl Document {
    pub fn new(profile: UserProfile, path: Option<PathBuf>) -> Self {
        Self { saved: profile.clone(), profile, path }
    }

    /// AutoSplit's settings for a new profile: its window starts with a 640x480 region at the
    /// top left, from a video capture device.
    pub fn untitled() -> Self {
        Self::new(
            UserProfile {
                capture_method: VIDEO_CAPTURE_DEVICE.into(),
                capture_region: NEW_PROFILE_REGION,
                ..UserProfile::default()
            },
            None,
        )
    }

    pub fn has_changes(&self) -> bool {
        self.profile != self.saved
    }

    /// The file's name, like AutoSplit's save prompt names it.
    pub fn name(&self) -> String {
        self.path
            .as_ref()
            .and_then(|path| path.file_name())
            .map_or_else(|| "Untitled".to_owned(), |name| name.to_string_lossy().into_owned())
    }

    /// Where relative folders are read from: the file's folder, or `fallback` for a new profile.
    pub fn folder(&self, fallback: &Path) -> PathBuf {
        self.path.as_deref().and_then(Path::parent).unwrap_or(fallback).to_owned()
    }

    /// Where one of the profile's folders is, the way the session reads it.
    pub fn resolve(&self, fallback: &Path, directory: &str) -> PathBuf {
        profile::resolve_directory(&self.folder(fallback), directory)
    }

    /// The profile to write to a file. Only video capture devices can be picked, so a profile
    /// without a capture method gets that one, which AutoSplit would otherwise read as none.
    pub fn to_save(&self) -> UserProfile {
        let mut profile = self.profile.clone();
        if profile.capture_method.is_empty() {
            profile.capture_method = VIDEO_CAPTURE_DEVICE.into();
        }
        profile
    }

    /// `profile` was written to `path`.
    pub fn saved(&mut self, path: PathBuf, profile: UserProfile) {
        self.profile = profile.clone();
        self.saved = profile;
        self.path = Some(path);
    }
}

/// AutoSplit's `load_settings_on_open`: the only `.toml` in `folder`, the executable's, with
/// warnings about keys in the wrong place.
pub fn profile_on_open(folder: &Path) -> Result<(PathBuf, UserProfile, Vec<String>), String> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(folder)
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default();
    found.retain(|path| path.extension().is_some_and(|extension| extension == "toml"));
    match found.as_slice() {
        [] => Err("No settings file found. One can be loaded on open if placed in the same folder as the AutoSplit \
                   executable."
            .into()),
        [path] => crate::profile::load_raw(path)
            .map(|(profile, warnings)| (path.clone(), profile, warnings))
            .map_err(|error| format!("{error:#}")),
        _ => Err("Too many settings files found. Only one can be loaded on open if placed in the same folder as the \
                  AutoSplit executable."
            .into()),
    }
}

/// How a picked folder is stored: with forward slashes, like Qt's folder dialog gives them to
/// AutoSplit.
pub fn folder_text(path: &Path) -> String {
    let text = path.display().to_string().replace('\\', "/");
    match text.trim_end_matches('/') {
        "" => text,
        trimmed => trimmed.to_owned(),
    }
}
