//! The parts of the `autosplit` command that run AutoSplit: capture devices, hotkeys, LiveSplit,
//! LiveSplit One and Molok, around the core's split state machine.

pub mod console;
pub mod device;
pub mod gui;
pub mod hotkeys;
pub mod licenses;
pub mod livesplit;
pub mod livesplit_one;
#[cfg(target_os = "linux")]
pub mod molok;
pub mod platform;
pub mod profile;
pub mod run;
pub mod screenshot;
pub mod session;
mod stdin;
