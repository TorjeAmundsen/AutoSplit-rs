//! LiveSplit's lines on stdin, with `--auto-controlled`.

use std::io::BufRead;

use autosplit_core::auto_controlled::decode_line;

use crate::platform;

/// Reads lines on a thread of its own: `line` gets each one until it returns false, `closed`
/// runs when LiveSplit closes stdin.
pub fn read_lines(mut line: impl FnMut(String) -> bool + Send + 'static, closed: impl FnOnce() + Send + 'static) {
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        let mut bytes = Vec::new();
        loop {
            bytes.clear();
            match stdin.read_until(b'\n', &mut bytes) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if !line(decode_line(&bytes, platform::decode_ansi)) {
                        return;
                    }
                }
            }
        }
        closed();
    });
}
