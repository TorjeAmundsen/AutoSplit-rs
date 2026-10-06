//! LiveSplit's AutoSplit Integration component, which starts `autosplit --auto-controlled` and
//! talks to it over stdin and stdout.

use std::io::Write;
use std::path::Path;
use std::sync::mpsc;

use anyhow::Result;
use autosplit_core::auto_controlled::handshake_lines;

use crate::console::{Console, utc_now};
use crate::session::Event;
use crate::stdin;

/// Prints the `--auto-controlled` handshake, and forwards LiveSplit's lines to the session.
pub fn connect(events: &mpsc::Sender<Event>) -> Result<()> {
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
pub fn log_started(console: &mut Console, log_path: Option<&Path>) {
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
