//! Loading a profile for the commands that take one.

use std::path::Path;

use anyhow::{Context, Result, bail};
use autosplit_core::profile::UserProfile;

/// Loads a profile as it is in its file, with warnings about keys in the wrong place.
pub fn load_raw(path: &Path) -> Result<(UserProfile, Vec<String>)> {
    if !path.exists() {
        bail!("The profile {} doesn't exist.", path.display());
    }
    if path.is_dir() {
        bail!("{} is a folder. Give the profile's .toml file.", path.display());
    }
    let profile = UserProfile::load(path).with_context(|| format!("Couldn't load the profile {}", path.display()))?;
    let misplaced = std::fs::read_to_string(path)
        .map(|text| UserProfile::keys_inside_capture_region(&text))
        .unwrap_or_default()
        .into_iter()
        .map(|key| {
            format!("'{key}' is inside [capture_region], where it's ignored. Move it above the [capture_region] line.")
        })
        .collect();
    Ok((profile, misplaced))
}

/// Loads a profile, with its folders resolved from the profile's folder and `split_dir`
/// replacing its split image folder, and warnings about keys in the wrong place.
pub fn load(path: &Path, split_dir: Option<&Path>) -> Result<(UserProfile, Vec<String>)> {
    let (mut profile, warnings) = load_raw(path)?;
    profile.resolve_directories(path.parent().unwrap_or(Path::new(".")));
    if let Some(split_dir) = split_dir {
        profile.split_image_directory = split_dir.to_string_lossy().into_owned();
    }
    Ok((profile, warnings))
}
