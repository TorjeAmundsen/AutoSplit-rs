//! The parts of the `autosplit` command that run AutoSplit: capture devices, hotkeys, LiveSplit
//! and LiveSplit One, around the core's split state machine.

pub mod console;
pub mod device;
pub mod gui;
pub mod hotkeys;
pub mod livesplit_one;
pub mod platform;
pub mod profile;
pub mod run;
pub mod screenshot;
pub mod session;
mod stdin;
