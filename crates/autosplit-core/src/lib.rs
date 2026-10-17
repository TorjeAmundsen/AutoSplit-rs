//! Platform independent core of AutoSplit: split image parsing, image comparison,
//! profiles and the split state machine. Capture, hotkeys and UI live elsewhere.

pub mod auto_controlled;
pub mod commands;
pub mod compare;
pub mod filename;
pub mod hotkeys;
pub mod image;
#[doc(hidden)]
pub mod imgproc;
pub mod library;
pub mod livesplit_one;
pub mod mask;
#[cfg(target_os = "linux")]
pub mod molok;
pub mod preset;
pub mod profile;
pub mod runner;
pub mod split_folder;
pub mod split_image;
pub mod tester;
pub mod text;
