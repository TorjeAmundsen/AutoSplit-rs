//! AutoSplit settings profiles (`.toml`), compatible both ways with AutoSplit's files.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Deserializer, Serialize};

use crate::commands::Command;
use crate::image::Rect;
use crate::split_image::SplitDefaults;

/// The capture region of a new profile: AutoSplit's window starts with a 640x480 region at the
/// top left.
pub const NEW_PROFILE_REGION: Region = Region { x: 0, y: 0, width: 640, height: 480 };

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Region {
    pub x: i64,
    pub y: i64,
    pub width: i64,
    pub height: i64,
}

impl Region {
    /// The 4:3 picture in the middle of a frame of this size, like a 4:3 game in a 16:9 canvas:
    /// 1440x1080 at X 240 in 1920x1080. The full height when the frame is wider, else the full
    /// width.
    pub fn centered_4_3(width: usize, height: usize) -> Self {
        let (width, height) = (width as i64, height as i64);
        if width * 3 >= height * 4 {
            // Rounded to the nearest pixel
            let inner = (height * 4 + 1) / 3;
            Self { x: (width - inner) / 2, y: 0, width: inner, height }
        } else {
            let inner = (width * 3 + 2) / 4;
            Self { x: 0, y: (height - inner) / 2, width, height: inner }
        }
    }

    /// The pixels AutoSplit compares from a frame of this size: the start is kept inside the
    /// frame, the end is clamped to it.
    pub fn rect(&self, width: usize, height: usize) -> Rect {
        let start = |position: i64, size: usize| position.clamp(0, size.saturating_sub(1) as i64) as usize;
        let (left, top) = (start(self.x, width), start(self.y, height));
        let end = |start: usize, length: i64, size: usize| {
            start.saturating_add(usize::try_from(length).unwrap_or(0)).min(size).max(start)
        };
        Rect { left, top, right: end(left, self.width, width), bottom: end(top, self.height, height) }
    }

    /// How the region fits frames of this size.
    pub fn fit(&self, width: usize, height: usize) -> RegionFit {
        if self.width <= 0 || self.height <= 0 {
            return RegionFit::Empty;
        }
        let rect = self.rect(width, height);
        let whole = rect.left as i64 == self.x
            && rect.top as i64 == self.y
            && rect.width() as i64 == self.width
            && rect.height() as i64 == self.height;
        if whole { RegionFit::Inside } else { RegionFit::Partly }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionFit {
    /// Width or height is 0, so nothing is compared.
    Empty,
    /// Only part of the region is in the frame, and only that part is compared.
    Partly,
    Inside,
}

/// How commands reach the timer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimerConnection {
    /// Pressing the profile's hotkeys, like AutoSplit.
    #[default]
    Hotkeys,
    /// LiveSplit One connecting to AutoSplit's WebSocket server.
    #[serde(rename = "livesplit_one")]
    LiveSplitOne,
    /// Sending POSIX signals to Molok. Only on Linux. Other systems keep it in the profile, but
    /// use hotkeys.
    Molok,
}

/// AutoSplit's `capture_method` value for video capture devices.
pub const VIDEO_CAPTURE_DEVICE: &str = "VIDEO_CAPTURE_DEVICE";

/// LiveSplit One's server connection has no default port, so this one is AutoSplit's choice:
/// next to LiveSplit Server's 16834.
pub const DEFAULT_LIVESPLIT_ONE_PORT: u16 = 16835;

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

fn is_default_livesplit_one_port(port: &u16) -> bool {
    *port == DEFAULT_LIVESPLIT_ONE_PORT
}

/// Levels each channel of a fade image's color may be off by default, which covers capture
/// setups whose colors are a little off, like analog ones: a GV-USB2 shows OoT's #a0a0a0 with
/// blue 11 levels low.
pub const DEFAULT_FADE_COLOR_TOLERANCE: i64 = 15;

fn is_default_fade_color_tolerance(tolerance: &i64) -> bool {
    *tolerance == DEFAULT_FADE_COLOR_TOLERANCE
}

fn is_true(value: &bool) -> bool {
    *value
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
    /// Whether pauses and delays show the seconds left. New in AutoSplit 2.3.3.
    pub display_pause_timer: bool,
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
    /// Not in AutoSplit. Only written when changed, so profiles stay as AutoSplit writes them.
    #[serde(skip_serializing_if = "is_default")]
    pub timer_connection: TimerConnection,
    /// Not in AutoSplit. Only written when changed.
    #[serde(skip_serializing_if = "is_default_livesplit_one_port")]
    pub livesplit_one_port: u16,
    /// Not in AutoSplit. Split images that start with the same number are compared for the same
    /// split. Only written when changed.
    #[serde(skip_serializing_if = "is_default")]
    pub group_split_images: bool,
    /// Not in AutoSplit. The images of a grouped split are shown in a 2x2 grid. Only written
    /// when changed.
    #[serde(skip_serializing_if = "is_default")]
    pub split_image_grid: bool,
    /// Not in AutoSplit. How many levels each channel of a fade image's color may be off in the
    /// capture. Only written when changed.
    #[serde(skip_serializing_if = "is_default_fade_color_tolerance")]
    pub fade_color_tolerance: i64,
    /// Not in AutoSplit. Fade images leave the capture's noise out of their similarity, which
    /// they tell apart from the scene in each frame. On by default, and only written when off.
    #[serde(skip_serializing_if = "is_true")]
    pub fade_ignore_noise: bool,
    /// Not in AutoSplit. Make Split Images shows the split image file names and every button,
    /// instead of only what making split images needs. Only written when changed.
    #[serde(skip_serializing_if = "is_default")]
    pub advanced_maker: bool,
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
            display_pause_timer: true,
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
            timer_connection: TimerConnection::Hotkeys,
            livesplit_one_port: DEFAULT_LIVESPLIT_ONE_PORT,
            group_split_images: false,
            split_image_grid: false,
            fade_color_tolerance: DEFAULT_FADE_COLOR_TOLERANCE,
            fade_ignore_noise: true,
            advanced_maker: false,
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
    #[error("This is a split image preset, not a settings file.")]
    SplitImagePreset,
}

/// Where a profile's `directory` is: relative folders are read from `base`, the profile's folder.
pub fn resolve_directory(base: &Path, directory: &str) -> PathBuf {
    base.join(directory)
}

/// AutoSplit's names for its capture methods, for the window capture ones a profile can have.
pub fn capture_method_name(method: &str) -> &str {
    match method {
        "WINDOWS_GRAPHICS_CAPTURE" => "Windows Graphics Capture",
        "BITBLT" => "BitBlt",
        "PRINTWINDOW_RENDERFULLCONTENT" => "Force Full Content Rendering",
        "DESKTOP_DUPLICATION" => "Direct3D Desktop Duplication",
        other => other,
    }
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
        // Any TOML reads as a profile with defaults for what it doesn't have, presets too
        if text.parse::<toml::Table>().is_ok_and(|table| table.contains_key("split")) {
            return Err(ProfileError::SplitImagePreset);
        }
        toml::from_str(text).map_err(|error| ProfileError::Invalid { reason: plain_toml_error(&error) })
    }

    /// Keys written after `[capture_region]`, which TOML puts inside that table, where they're
    /// ignored. Easy to do by adding a setting at the end of the file.
    pub fn keys_inside_capture_region(text: &str) -> Vec<String> {
        let Ok(table) = text.parse::<toml::Table>() else { return Vec::new() };
        let Some(region) = table.get("capture_region").and_then(toml::Value::as_table) else { return Vec::new() };
        region.keys().filter(|key| !["x", "y", "width", "height"].contains(&key.as_str())).cloned().collect()
    }

    /// Makes relative split image and screenshot folders relative to `base`, the profile's
    /// folder, instead of wherever AutoSplit was started from.
    pub fn resolve_directories(&mut self, base: &Path) {
        for directory in [&mut self.split_image_directory, &mut self.screenshot_directory] {
            if !directory.is_empty() {
                *directory = resolve_directory(base, directory).to_string_lossy().into_owned();
            }
        }
    }

    pub fn to_toml(&self) -> String {
        toml::to_string(self).expect("profiles always serialize")
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        std::fs::write(path, self.to_toml())
    }

    /// Whether a capture device is set, by name or by number.
    pub fn has_capture_device(&self) -> bool {
        !self.capture_device_name.is_empty() || self.capture_device_id >= 0
    }

    /// Whether the capture region is still a new profile's, or empty, so it wasn't set.
    pub fn region_untouched(&self) -> bool {
        self.capture_region == NEW_PROFILE_REGION || self.capture_region == Region::default()
    }

    /// The region to compare from OBS Virtual Camera's frames of `frame` size while the profile's
    /// region is still a new profile's, or empty: the whole frame. The camera sends OBS's canvas,
    /// usually 1920x1080, whatever the game's capture in it is, and 640x480 at the top left is
    /// only part of it. `None` for other devices, a region that was set, or a frame it fits.
    pub fn obs_full_frame_region(&self, frame: (usize, usize)) -> Option<Region> {
        let untouched = self.region_untouched();
        let obs = self.capture_device_name.to_lowercase().contains("obs virtual cam");
        let full = Region { x: 0, y: 0, width: frame.0 as i64, height: frame.1 as i64 };
        (untouched && obs && full.width > 0 && full.height > 0 && full != self.capture_region).then_some(full)
    }

    /// The window capture method the profile uses, which isn't supported, if it uses one.
    pub fn window_capture_method(&self) -> Option<&str> {
        let method = self.capture_method.as_str();
        (!method.is_empty() && method != VIDEO_CAPTURE_DEVICE).then_some(method)
    }

    pub fn split_defaults(&self) -> SplitDefaults {
        SplitDefaults {
            comparison_method: self.default_comparison_method,
            similarity_threshold: self.default_similarity_threshold,
            delay_time: self.default_delay_time,
            pause_time: self.default_pause_time,
            fps_limit: self.fps_limit,
            fade_tolerance: self.fade_color_tolerance,
            fade_ignore_noise: self.fade_ignore_noise,
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

/// TOML's message, with Rust type names put in words and a hint for Windows paths.
fn plain_toml_error(error: &toml::de::Error) -> String {
    let mut message = error
        .to_string()
        .replace("expected i64", "expected a whole number")
        .replace("expected u16", "expected a whole number from 0 to 65535")
        .replace("expected f64", "expected a number");
    if message.contains("escape") || message.contains("unicode") {
        message.push_str("\nUse / instead of \\ in paths, or put the path in 'single quotes'.");
    }
    message
}
