//! `autosplit run` and `--auto-controlled` without a window: a [`Session`] shown with a status
//! line and an event log.

use std::path::{Path, PathBuf};
use std::sync::mpsc;

use anyhow::Result;
use autosplit_core::commands::TimerLink;
use autosplit_core::profile::UserProfile;

use crate::console::Console;
use crate::session::{Event, Session};
use crate::{livesplit, platform, profile};

/// `autosplit run <profile>`.
pub fn run(profile_path: &Path, split_dir: Option<PathBuf>) -> Result<()> {
    let (profile, warnings) = profile::load(profile_path, split_dir.as_deref())?;
    let link = TimerLink::from(profile.timer_connection);
    let (events, receiver) = mpsc::channel();
    let stop_events = events.clone();
    // Without a handler, Ctrl+C would leave the status line behind
    let _ = ctrlc::set_handler(move || {
        let _ = stop_events.send(Event::Stop);
    });
    let mut session = Session::new(profile, link, Console::stdout(), None, events);
    session.console().log(format_args!("Loaded {}", profile_path.display()));
    for warning in warnings {
        session.console().warn(warning);
    }
    session.connect_timer()?;
    session.start();
    session.run(&receiver);
    Ok(())
}

/// `autosplit --auto-controlled` without a window, because it couldn't open: `no_window` says why.
pub fn auto_controlled(no_window: Option<anyhow::Error>) -> Result<()> {
    platform::leave_own_console();
    let (events, receiver) = mpsc::channel();
    let (mut console, log_path) = Console::auto_controlled();
    livesplit::log_started(&mut console, log_path.as_deref());
    if let Some(error) = no_window {
        console.warn(format_args!("Couldn't open the window, so running without it: {error:#}"));
    }
    let mut session = Session::new(UserProfile::default(), TimerLink::AutoControlled, console, None, events);
    session.connect_timer()?;
    session.start();
    session.run(&receiver);
    Ok(())
}
