//! Platform independent core of AutoSplit: split image parsing, image comparison,
//! profiles and the split state machine. Capture, hotkeys and UI live elsewhere.

pub mod auto_controlled;
pub mod commands;
pub mod compare;
pub mod filename;
pub mod image;
#[doc(hidden)]
pub mod imgproc;
pub mod profile;
pub mod runner;
pub mod split_folder;
pub mod split_image;
pub mod text;
