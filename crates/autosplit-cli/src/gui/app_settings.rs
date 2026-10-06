//! Settings of the program on this computer rather than of a game, kept out of profiles:
//! `app.toml` in the OS's settings folder (`%APPDATA%\AutoSplit` on Windows,
//! `~/Library/Application Support/AutoSplit` on macOS, `~/.config/autosplit` on Linux).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Overrides [`AppSettings::hardware_acceleration`] for one start: `software` or `gpu`.
pub const RENDERER_VARIABLE: &str = "AUTOSPLIT_RENDERER";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    /// Draw the window with the GPU. Off falls back to drawing on the CPU.
    pub hardware_acceleration: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self { hardware_acceleration: true }
    }
}

fn path() -> Option<PathBuf> {
    let folder = if cfg!(windows) {
        PathBuf::from(std::env::var_os("APPDATA")?).join("AutoSplit")
    } else if cfg!(target_os = "macos") {
        PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support/AutoSplit")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| Some(PathBuf::from(std::env::var_os("HOME")?).join(".config")))?
            .join("autosplit")
    };
    Some(folder.join("app.toml"))
}

impl AppSettings {
    /// The saved settings, or the defaults when there are none or they can't be read.
    pub fn load() -> Self {
        path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = path().ok_or_else(|| std::io::Error::other("no settings folder"))?;
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder)?;
        }
        std::fs::write(path, toml::to_string(self).map_err(std::io::Error::other)?)
    }

    /// Whether to draw with the GPU this time: the setting, unless [`RENDERER_VARIABLE`] says
    /// otherwise.
    pub fn use_gpu(&self) -> bool {
        match std::env::var(RENDERER_VARIABLE).as_deref() {
            Ok("software") => false,
            Ok("gpu") => true,
            _ => self.hardware_acceleration,
        }
    }
}
