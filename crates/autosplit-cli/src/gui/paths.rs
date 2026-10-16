//! Small helpers for the files the windows show and compare.

use std::path::{Path, PathBuf};

pub fn is_png(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
}

/// A path's last part as text, or `None` for a path without one, like `..`.
pub fn file_name_text(path: &Path) -> Option<String> {
    path.file_name().map(|name| name.to_string_lossy().into_owned())
}

/// The path with links and `..` resolved, to compare paths written in different ways, or the
/// path itself when it doesn't exist.
pub fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned())
}
