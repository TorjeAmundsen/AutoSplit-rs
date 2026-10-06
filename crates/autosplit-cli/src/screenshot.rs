//! Screenshots of the capture region, saved like AutoSplit's: `NNN_SplitImage.png` in the
//! screenshot folder, or the split image folder without one.

use std::io::Write;
use std::path::{Path, PathBuf};

use autosplit_core::image::BgraImage;
use autosplit_core::profile::UserProfile;

/// Saves `image` in the background, since encoding a large frame takes a while, which
/// comparisons shouldn't wait for. `done` gets the message for the log. Fails right away when
/// there's nowhere to save it.
pub fn save(
    profile: &UserProfile,
    image: Option<BgraImage>,
    done: impl FnOnce(Result<String, String>) + Send + 'static,
) -> Result<(), String> {
    let directory = if profile.screenshot_directory.is_empty() {
        &profile.split_image_directory
    } else {
        &profile.screenshot_directory
    };
    if directory.is_empty() {
        return Err("Screenshot: no screenshot or split image folder is set".into());
    }
    let directory = PathBuf::from(directory);
    if !directory.is_dir() {
        return Err(format!("Screenshot: the folder {} doesn't exist", directory.display()));
    }
    let image = image.ok_or("Screenshot: there's no capture to take a screenshot of")?;
    if image.is_empty() {
        return Err("Screenshot: the capture region is empty".into());
    }
    let (path, file) = claim_path(&directory)
        .ok_or_else(|| format!("Screenshot: couldn't create a file in {}", directory.display()))?;
    let open = profile.open_screenshot;
    std::thread::spawn(move || {
        let written = image
            .encode_png()
            .map_err(|error| error.to_string())
            .and_then(|png| (&file).write_all(&png).map_err(|error| error.to_string()));
        done(match written {
            Ok(()) => {
                if open {
                    open_file(&path);
                }
                Ok(format!("Saved screenshot {}", path.display()))
            }
            Err(error) => Err(format!("Screenshot: couldn't write {}: {error}", path.display())),
        });
    });
    Ok(())
}

/// The next free `NNN_SplitImage.png`, created so a second screenshot can't take it too.
fn claim_path(directory: &Path) -> Option<(PathBuf, std::fs::File)> {
    for index in 1.. {
        let path = directory.join(format!("{index:03}_SplitImage.png"));
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Some((path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return None,
        }
    }
    None
}

/// Opens the file (or link) with its default app, like AutoSplit's `open_file`.
pub fn open_file(path: &Path) {
    let opener = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(opener).arg(path).status();
}
