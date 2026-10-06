//! `run`'s output: an event log, with a status line kept below it in place when the output is a
//! terminal.

use std::fs::{File, OpenOptions};
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Terminals are at least this wide, and a wrapped line can't be rewritten in place.
const STATUS_MAX_WIDTH: usize = 79;
/// The log file starts over once it's this big.
const LOG_MAX_BYTES: u64 = 1_000_000;

/// Gets each log line, like the GUI's log window.
pub type Listener = Box<dyn FnMut(&str) + Send>;

pub struct Console {
    out: Option<Box<dyn Write + Send>>,
    terminal: bool,
    /// Also gets the log, when LiveSplit started AutoSplit and nothing else would show it.
    file: Option<File>,
    /// The status line currently shown, empty when none is.
    shown: String,
    listener: Option<Listener>,
    started: Instant,
}

impl Console {
    fn with(out: Option<Box<dyn Write + Send>>, terminal: bool, file: Option<File>) -> Self {
        Self { out, terminal, file, shown: String::new(), listener: None, started: Instant::now() }
    }

    /// Logs nowhere.
    pub fn silent() -> Self {
        Self::with(None, false, None)
    }

    /// Logs to `autosplit.log` (see [`Console::auto_controlled`]) and to `listener`, for the
    /// GUI, which has no terminal. Also to stderr when that's a terminal.
    pub fn with_listener(listener: Listener) -> (Self, Option<PathBuf>) {
        let (mut console, path) = Self::auto_controlled();
        console.terminal = false;
        console.listener = Some(listener);
        (console, path)
    }

    /// Logs to stdout.
    pub fn stdout() -> Self {
        Self::with(Some(Box::new(std::io::stdout())), std::io::stdout().is_terminal(), None)
    }

    /// When auto-controlled, stdout belongs to LiveSplit and LiveSplit reports each stderr line
    /// as an error, so the log goes to `autosplit.log` next to the executable (or in the temp
    /// folder when that one is read-only), and to stderr only when that's a terminal.
    pub fn auto_controlled() -> (Self, Option<PathBuf>) {
        let terminal = std::io::stderr().is_terminal();
        let opened = log_paths().into_iter().find_map(|path| open_log(&path).map(|file| (file, path)));
        let (file, path) = opened.unzip();
        let out = terminal.then(|| Box::new(std::io::stderr()) as Box<dyn Write + Send>);
        (Self::with(out, terminal, file), path)
    }

    pub fn shows_status(&self) -> bool {
        self.out.is_some() && self.terminal
    }

    /// Adds a line to the event log, with the time since launch.
    pub fn log(&mut self, message: impl std::fmt::Display) {
        let line = format!("{:>9.3}s  {message}", self.started.elapsed().as_secs_f64());
        if let Some(file) = &mut self.file {
            let _ = writeln!(file, "{line}");
        }
        if let Some(listener) = &mut self.listener {
            listener(&line);
        }
        self.clear_status();
        if let Some(out) = &mut self.out {
            let _ = writeln!(out, "{line}");
            let _ = out.flush();
        }
    }

    pub fn warn(&mut self, message: impl std::fmt::Display) {
        self.log(format_args!("warning: {message}"));
    }

    pub fn error(&mut self, message: impl std::fmt::Display) {
        self.log(format_args!("error: {message}"));
    }

    /// Replaces the status line. Nothing is shown when the output isn't a terminal.
    pub fn status(&mut self, line: &str) {
        if !self.shows_status() {
            return;
        }
        let mut width = 0;
        let line: String = line
            .chars()
            .take_while(|c| {
                width += c.width().unwrap_or(0);
                width <= STATUS_MAX_WIDTH
            })
            .collect();
        if line == self.shown {
            return;
        }
        let padding = self.shown.width().saturating_sub(line.width());
        if let Some(out) = &mut self.out {
            let _ = write!(out, "\r{line}{:padding$}", "");
            let _ = out.flush();
        }
        self.shown = line;
    }

    fn clear_status(&mut self) {
        if self.shown.is_empty() {
            return;
        }
        if let Some(out) = &mut self.out {
            let _ = write!(out, "\r{:width$}\r", "", width = self.shown.width());
        }
        self.shown.clear();
    }
}

impl Drop for Console {
    fn drop(&mut self) {
        self.clear_status();
    }
}

fn log_paths() -> Vec<PathBuf> {
    let next_to_exe = std::env::current_exe().ok().map(|exe| exe.with_file_name("autosplit.log"));
    next_to_exe.into_iter().chain([std::env::temp_dir().join("autosplit.log")]).collect()
}

/// Appends, so an AutoSplit that LiveSplit started while the previous one was still closing
/// doesn't garble its log, and starts over once the file is big.
fn open_log(path: &PathBuf) -> Option<File> {
    let big = std::fs::metadata(path).is_ok_and(|metadata| metadata.len() > LOG_MAX_BYTES);
    let mut options = OpenOptions::new();
    options.create(true);
    if big {
        options.write(true).truncate(true)
    } else {
        options.append(true)
    };
    options.open(path).ok()
}

/// The current time in UTC, like `2026-10-06 09:13:19 UTC`.
pub fn utc_now() -> String {
    let seconds = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs());
    let (days, time) = (seconds / 86_400, seconds % 86_400);
    let (year, month, day) = civil_from_days(days as i64);
    format!("{year}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC", time / 3600, time / 60 % 60, time % 60)
}

/// Howard Hinnant's `civil_from_days`: the date `days` after 1970-01-01.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_index + 2) / 5 + 1) as u32;
    let month = if month_index < 10 { month_index + 3 } else { month_index - 9 } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}
