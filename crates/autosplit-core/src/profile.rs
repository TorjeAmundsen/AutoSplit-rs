//! AutoSplit settings profiles (`.toml`), compatible both ways with AutoSplit's files.

use std::path::Path;

use serde::{Deserialize, Deserializer, Serialize};

use crate::commands::Command;
use crate::split_image::SplitDefaults;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Region {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

/// A settings profile. Field order is the key order AutoSplit writes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UserProfile {
    pub split_hotkey: String,
    pub reset_hotkey: String,
    pub undo_split_hotkey: String,
    pub skip_split_hotkey: String,
    pub pause_hotkey: String,
    pub screenshot_hotkey: String,
    pub toggle_auto_reset_image_hotkey: String,
    pub fps_limit: i64,
    pub live_capture_region: bool,
    /// AutoSplit's capture method name, like `WINDOWS_GRAPHICS_CAPTURE`. Kept as text so that
    /// a profile made on another OS keeps its value when saved here.
    pub capture_method: String,
    pub capture_device_id: i64,
    pub capture_device_name: String,
    pub default_comparison_method: i64,
    pub default_similarity_threshold: f64,
    /// Milliseconds
    pub default_delay_time: i64,
    /// Seconds
    pub default_pause_time: f64,
    pub loop_splits: bool,
    pub start_also_resets: bool,
    pub enable_auto_reset: bool,
    pub split_image_directory: String,
    pub screenshot_directory: String,
    pub open_screenshot: bool,
    #[serde(deserialize_with = "deserialize_screenshot_on")]
    pub screenshot_on: Vec<Command>,
    pub captured_window_title: String,
    pub capture_region: Region,
    /// Keys this version doesn't know, kept so saving doesn't drop them.
    #[serde(flatten)]
    pub unknown: toml::Table,
}

/// The values AutoSplit's settings window starts with. They're also used for keys missing
/// from a loaded file.
impl Default for UserProfile {
    fn default() -> Self {
        Self {
            split_hotkey: String::new(),
            reset_hotkey: String::new(),
            undo_split_hotkey: String::new(),
            skip_split_hotkey: String::new(),
            pause_hotkey: String::new(),
            screenshot_hotkey: String::new(),
            toggle_auto_reset_image_hotkey: String::new(),
            fps_limit: 60,
            live_capture_region: true,
            capture_method: String::new(),
            capture_device_id: -1,
            capture_device_name: String::new(),
            default_comparison_method: 0,
            default_similarity_threshold: 0.9,
            default_delay_time: 0,
            default_pause_time: 10.0,
            loop_splits: false,
            start_also_resets: false,
            enable_auto_reset: true,
            split_image_directory: String::new(),
            screenshot_directory: String::new(),
            open_screenshot: true,
            screenshot_on: Vec::new(),
            captured_window_title: String::new(),
            capture_region: Region { x: 0, y: 0, width: 0, height: 0 },
            unknown: toml::Table::new(),
        }
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ProfileError {
    #[error("This is a settings file from AutoSplit before v2.0 (.pkl). Only .toml settings files are supported.")]
    OldVersion,
    #[error("The settings file couldn't be read: {reason}")]
    Unreadable { reason: String },
    #[error("The settings file isn't valid: {reason}")]
    Invalid { reason: String },
}

impl UserProfile {
    pub fn load(path: &Path) -> Result<Self, ProfileError> {
        if path.extension().is_some_and(|extension| extension == "pkl") {
            return Err(ProfileError::OldVersion);
        }
        let text =
            std::fs::read_to_string(path).map_err(|error| ProfileError::Unreadable { reason: error.to_string() })?;
        Self::parse(&text)
    }

    pub fn parse(text: &str) -> Result<Self, ProfileError> {
        toml::from_str(text).map_err(|error| ProfileError::Invalid { reason: error.to_string() })
    }

    pub fn to_toml(&self) -> String {
        toml::to_string(self).expect("profiles always serialize")
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        std::fs::write(path, self.to_toml())
    }

    pub fn split_defaults(&self) -> SplitDefaults {
        SplitDefaults {
            comparison_method: self.default_comparison_method,
            similarity_threshold: self.default_similarity_threshold,
            delay_time: self.default_delay_time,
            pause_time: self.default_pause_time,
            fps_limit: self.fps_limit,
        }
    }
}

/// AutoSplit can save this list as booleans (`[false, false, ...]`) when the settings window was
/// never opened. Only command names mean something, so anything else is dropped, as are
/// duplicates.
fn deserialize_screenshot_on<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<Command>, D::Error> {
    let values = Vec::<toml::Value>::deserialize(deserializer)?;
    let mut commands = Vec::new();
    for command in values.iter().filter_map(|value| value.as_str().and_then(Command::from_name)) {
        if !commands.contains(&command) {
            commands.push(command);
        }
    }
    Ok(commands)
}
