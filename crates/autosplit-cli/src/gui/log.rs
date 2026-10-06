//! The log window's lines.

use std::collections::VecDeque;

/// Lines kept for the log window. autosplit.log has them all.
pub const LOG_LINES: usize = 2000;

#[derive(Default)]
pub struct LogLines {
    lines: VecDeque<String>,
}

impl LogLines {
    pub fn push(&mut self, line: String) {
        self.lines.push_back(line);
        while self.lines.len() > LOG_LINES {
            self.lines.pop_front();
        }
    }

    pub fn text(&self) -> String {
        let lines: Vec<&str> = self.lines.iter().map(String::as_str).collect();
        lines.join("\n")
    }
}
