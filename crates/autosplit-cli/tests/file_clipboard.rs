//! The formats Copy Savestates puts on the clipboard on Linux.
#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};

use autosplit::file_clipboard::{Contents, file_uri};

#[test]
fn paths_become_file_uris() {
    assert_eq!(
        file_uri(Path::new("/tmp/AutoSplit savestates/01_Deku #1.gzs")),
        "file:///tmp/AutoSplit%20savestates/01_Deku%20%231.gzs"
    );
}

#[test]
fn each_format_lists_the_files() {
    // Act
    let contents = Contents::new(&[PathBuf::from("/tmp/a.gzs"), PathBuf::from("/tmp/b.gzs")]);

    // Assert
    assert_eq!(contents.uri_list, b"file:///tmp/a.gzs\r\nfile:///tmp/b.gzs\r\n");
    assert_eq!(contents.gnome, b"copy\nfile:///tmp/a.gzs\nfile:///tmp/b.gzs");
    assert_eq!(contents.text, b"x-special/nautilus-clipboard\ncopy\nfile:///tmp/a.gzs\nfile:///tmp/b.gzs\n");
}
