//! `autosplit run` and `--auto-controlled`'s headless start: a [`Session`] shown with a status
//! line and an event log.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::Result;
use autosplit_core::auto_controlled::handshake_lines;
use autosplit_core::commands::TimerLink;
use autosplit_core::profile::UserProfile;

use crate::console::{Console, utc_now};
use crate::session::{Event, Session, View};
use crate::{platform, profile, stdin};

/// How often the status line is redrawn.
const STATUS_INTERVAL: Duration = Duration::from_millis(100);

/// The status line below the event log, when the output is a terminal.
struct StatusLine;

impl View for StatusLine {
    fn interval(&self, _profile: &UserProfile) -> Option<Duration> {
        Some(STATUS_INTERVAL)
    }

    fn update(&mut self, session: &Session, console: &mut Console) {
        if console.shows_status() {
            console.status(&session.status_line());
        }
    }
}

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
    let mut session = Session::new(profile, link, Console::stdout(), Box::new(StatusLine), events);
    session.console().log(format_args!("Loaded {}", profile_path.display()));
    for warning in warnings {
        session.console().warn(warning);
    }
    if link == TimerLink::LiveSplitOne {
        let port = session.profile().livesplit_one_port;
        session.listen_for_livesplit_one(port)?;
    } else {
        session.start_sending_hotkeys();
    }
    session.start();
    session.run(&receiver);
    Ok(())
}

/// Prints the `--auto-controlled` handshake, and forwards LiveSplit's lines to the session.
pub fn connect_to_livesplit(events: &mpsc::Sender<Event>) -> Result<()> {
    let mut stdout = std::io::stdout().lock();
    for line in handshake_lines(env!("CARGO_PKG_VERSION")) {
        writeln!(stdout, "{line}")?;
    }
    stdout.flush()?;
    drop(stdout);
    let (line_events, closed_events) = (events.clone(), events.clone());
    stdin::read_lines(
        move |line| line_events.send(Event::Stdin(line)).is_ok(),
        move || {
            let _ = closed_events.send(Event::StdinClosed);
        },
    );
    Ok(())
}

/// The first lines of the log when LiveSplit started `autosplit`.
pub fn log_started_by_livesplit(console: &mut Console, log_path: Option<&Path>) {
    console.log(format_args!(
        "AutoSplit {} started by LiveSplit on {}. Waiting for it to send the profile set in the AutoSplit \
         Integration component's settings.",
        env!("CARGO_PKG_VERSION"),
        utc_now()
    ));
    if let Some(log_path) = log_path {
        console.log(format_args!("Logging to {}", log_path.display()));
    }
}

/// `autosplit --auto-controlled` without a window.
pub fn auto_controlled() -> Result<()> {
    platform::leave_own_console();
    let (events, receiver) = mpsc::channel();
    connect_to_livesplit(&events)?;
    let (mut console, log_path) = Console::auto_controlled();
    log_started_by_livesplit(&mut console, log_path.as_deref());
    let mut session =
        Session::new(UserProfile::default(), TimerLink::AutoControlled, console, Box::new(StatusLine), events);
    session.wait_for_profile();
    session.start();
    session.run(&receiver);
    Ok(())
}
