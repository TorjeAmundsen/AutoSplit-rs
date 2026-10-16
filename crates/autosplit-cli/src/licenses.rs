//! The license and the third-party notices, built into the program so the release zip doesn't
//! need them as separate files. About > Licenses shows them.

use std::path::PathBuf;

const LICENSE: &str = include_str!("../../../LICENSE");
const THIRD_PARTY_NOTICES: &str = include_str!("../../../THIRD_PARTY_NOTICES.txt");

/// The license, then the third-party notices. With Windows line endings when `crlf`, for Notepad.
pub fn text(crlf: bool) -> String {
    let text = format!("{}\n\n{}", LICENSE.trim_end(), THIRD_PARTY_NOTICES.trim_end()).replace("\r\n", "\n") + "\n";
    if crlf { text.replace('\n', "\r\n") } else { text }
}

/// Writes the licenses to a text file in the temporary folder, to open in the default text editor.
pub fn write() -> std::io::Result<PathBuf> {
    let path = crate::platform::temp_folder().join("AutoSplit licenses.txt");
    std::fs::write(&path, text(cfg!(windows)))?;
    Ok(path)
}
