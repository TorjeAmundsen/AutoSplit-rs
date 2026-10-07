//! Molok, a Linux timer controlled with POSIX real-time signals. It reports nothing back.

use crate::commands::Command;

/// The signal that sends `command`, as an offset from `SIGRTMIN`.
pub fn signal_for(command: Command) -> i32 {
    match command {
        Command::Start | Command::Split => 0,
        Command::Skip => 1,
        Command::Undo => 2,
        Command::Reset => 3,
        // Also resumes
        Command::Pause => 4,
    }
}
