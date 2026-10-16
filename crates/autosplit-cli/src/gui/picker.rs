//! Why a file picker didn't open. rfd doesn't say: it logs the error and answers like the picker
//! was cancelled, which on Linux happens without xdg-desktop-portal or zenity. Its log is caught
//! here instead, so the window can say so.

use std::sync::{Mutex, PoisonError};

/// The last error rfd logged.
static ERROR: Mutex<Option<String>> = Mutex::new(None);

struct Errors;

impl log::Log for Errors {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Error && metadata.target().starts_with("rfd")
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            *ERROR.lock().unwrap_or_else(PoisonError::into_inner) = Some(record.args().to_string());
        }
    }

    fn flush(&self) {}
}

/// Catches rfd's errors from now on.
pub fn catch_errors() {
    if log::set_logger(&Errors).is_ok() {
        log::set_max_level(log::LevelFilter::Error);
    }
}

/// Forgets an earlier error, before a picker opens. rfd may start the picker on a thread of its
/// own before its answer is awaited, so this goes before the picker is asked for.
pub fn clear_error() {
    ERROR.lock().unwrap_or_else(PoisonError::into_inner).take();
}

/// Why the picker that just answered nothing didn't open, if it didn't.
pub fn take_error() -> Option<String> {
    let error = ERROR.lock().unwrap_or_else(PoisonError::into_inner).take()?;
    let help = if cfg!(target_os = "linux") {
        " File pickers need xdg-desktop-portal, which desktops usually have, or zenity. MANUAL.md's Linux section \
         says more."
    } else {
        ""
    };
    Some(format!("Couldn't open a file picker: {error}.{help}"))
}
