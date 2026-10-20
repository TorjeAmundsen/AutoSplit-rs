//! Small helpers for the files the windows show and compare.

use std::path::Path;

pub use autosplit_core::preset::canonical;

pub fn is_png(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
}

/// A path's last part as text, or `None` for a path without one, like `..`.
pub fn file_name_text(path: &Path) -> Option<String> {
    path.file_name().map(|name| name.to_string_lossy().into_owned())
}
