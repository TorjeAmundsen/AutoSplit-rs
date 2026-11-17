//! The parts of the `autosplit` command that run AutoSplit: capture devices, hotkeys, LiveSplit,
//! LiveSplit One and Molok, around the core's split state machine.

/// The name of the key shortcuts are held with, Command on macOS and Control elsewhere, for
/// messages that name a shortcut, like `concat!(primary_key!(), "+,")`.
#[cfg(target_os = "macos")]
macro_rules! primary_key {
    () => {
        "Cmd"
    };
}
#[cfg(not(target_os = "macos"))]
macro_rules! primary_key {
    () => {
        "Ctrl"
    };
}

pub mod console;
pub mod device;
#[cfg(target_os = "linux")]
pub mod file_clipboard;
pub mod gui;
pub mod hotkeys;
pub mod licenses;
pub mod livesplit;
pub mod livesplit_one;
#[cfg(target_os = "linux")]
pub mod molok;
pub mod platform;
pub mod profile;
pub mod recording;
pub mod run;
pub mod screenshot;
pub mod session;
mod stdin;

/// The version the window title, the log, `--version` and LiveSplit see. Dev builds from main set
/// `AUTOSPLIT_VERSION` when they're built, like `0.4.0-dev.12+abc1234`, so they don't pass for
/// the release Cargo.toml names.
pub const VERSION: &str = match option_env!("AUTOSPLIT_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};
