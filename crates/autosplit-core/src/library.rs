//! Writing the splits library: ready-made splits made from a preset's splits, added to the
//! library of their game, changed or deleted. The library's file is written again each time, and
//! masks and base images no ready-made split uses anymore are deleted.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::image::{BgraImage, BgraView};
use crate::imgproc::stretch_to_rgba;
use crate::preset::{
    LIBRARY_FILE, LibrarySplit, PresetError, PresetSplit, SplitLibrary, game_key, load_libraries, new_preset_folder,
    sanitize_file_name, unique_file_name,
};

/// The size base images are made at, which the Split Library shows them at.
pub const BASE_IMAGE_SIZE: (usize, usize) = (64, 48);

/// An image of a ready-made split to write: a preset's split, with its mask and savestate as
/// paths that exist, and a screenshot the mask fits, at any size.
pub struct NewStep {
    pub split: PresetSplit,
    pub mask: PathBuf,
    pub savestate: Option<PathBuf>,
    pub screenshot: BgraImage,
}

/// `screenshot` at [`BASE_IMAGE_SIZE`], opaque.
pub fn base_image(screenshot: BgraView) -> BgraImage {
    let (width, height) = BASE_IMAGE_SIZE;
    let mut rgba = vec![[0; 4]; width * height];
    stretch_to_rgba(screenshot, width, height, &mut rgba);
    let bgra = rgba.iter().flat_map(|&[r, g, b, _]| [b, g, r, 255]).collect();
    BgraImage::from_raw(width, height, bgra)
}

/// Adds a ready-made split made of `steps`, in order, to the library of `game` in `directory`,
/// or a new library for it. Returns the library's folder.
pub fn add_ready_made(
    directory: &Path,
    game: &str,
    description: &str,
    steps: Vec<NewStep>,
) -> Result<PathBuf, PresetError> {
    let (libraries, _) = load_libraries(directory);
    let found = libraries.into_iter().find(|loaded| game_key(&loaded.library.game) == game_key(game));
    let (folder, before) = match found {
        Some(loaded) => (loaded.folder, loaded.library),
        None => {
            let folder = new_preset_folder(directory, game);
            std::fs::create_dir_all(&folder).map_err(|error| unwritable(&folder, error))?;
            (folder, SplitLibrary { game: game.to_owned(), splits: Vec::new() })
        }
    };
    let mut now = before.clone();
    now.splits.extend(store_steps(&folder, description, steps, &HashSet::new())?);
    write_library(&folder, &before, &now)?;
    Ok(folder)
}

/// Changes the `index`th ready-made split of the library in `folder`: its description, and its
/// images and settings when there are new `steps`.
pub fn change_ready_made(
    folder: &Path,
    index: usize,
    description: &str,
    steps: Option<Vec<NewStep>>,
) -> Result<(), PresetError> {
    let before = SplitLibrary::load(&folder.join(LIBRARY_FILE))?;
    let mut chunks: Vec<Vec<LibrarySplit>> = before.ready_made().map(<[LibrarySplit]>::to_vec).collect();
    if index >= chunks.len() {
        return Err(missing(folder, index));
    }
    // The files only this one uses, whose names its new files can take over
    let others: Vec<LibrarySplit> = (chunks.iter().enumerate())
        .filter(|&(other, _)| other != index)
        .flat_map(|(_, chunk)| chunk.iter().cloned())
        .collect();
    let replaced: HashSet<String> =
        library_files(&chunks[index]).difference(&library_files(&others)).cloned().collect();
    let chunk = &mut chunks[index];
    *chunk = match steps {
        Some(steps) => store_steps(folder, description, steps, &replaced)?,
        None => described(std::mem::take(chunk), description),
    };
    let now = SplitLibrary { game: before.game.clone(), splits: chunks.concat() };
    write_library(folder, &before, &now)
}

/// Deletes the `index`th ready-made split of the library in `folder`, with the files only it
/// used.
pub fn delete_ready_made(folder: &Path, index: usize) -> Result<(), PresetError> {
    let before = SplitLibrary::load(&folder.join(LIBRARY_FILE))?;
    let mut chunks: Vec<Vec<LibrarySplit>> = before.ready_made().map(<[LibrarySplit]>::to_vec).collect();
    if index >= chunks.len() {
        return Err(missing(folder, index));
    }
    chunks.remove(index);
    let now = SplitLibrary { game: before.game.clone(), splits: chunks.concat() };
    write_library(folder, &before, &now)
}

/// The description on the first image, which the Split Library shows, and none on the others.
fn described(mut steps: Vec<LibrarySplit>, description: &str) -> Vec<LibrarySplit> {
    for (index, step) in steps.iter_mut().enumerate() {
        step.description = if index == 0 { description.to_owned() } else { String::new() };
    }
    steps
}

/// Puts the masks, savestates and base images of `steps` in `folder`, and makes their library
/// splits. A file in the folder already, like one of a ready-made split that was inserted, is
/// used where it is. The `replaced` files, which only the ready-made split these replace used,
/// are written over, so its new files keep their names instead of getting a number.
fn store_steps(
    folder: &Path,
    description: &str,
    steps: Vec<NewStep>,
    replaced: &HashSet<String>,
) -> Result<Vec<LibrarySplit>, PresetError> {
    let root = std::fs::canonicalize(folder).map_err(|error| unwritable(folder, error))?;
    let inside = |path: &Path| -> Option<String> {
        let path = std::fs::canonicalize(path).ok()?;
        Some(path.strip_prefix(&root).ok()?.to_string_lossy().replace('\\', "/"))
    };
    // Used where they are, so never written over
    let kept: HashSet<String> = (steps.iter())
        .flat_map(|step| std::iter::once(&step.mask).chain(&step.savestate))
        .filter_map(|path| inside(path))
        .collect();
    let taken = |name: &str| root.join(name).exists() && (!replaced.contains(name) || kept.contains(name));
    // Names given in this write, which the folder doesn't have yet
    let mut given: HashSet<String> = HashSet::new();
    let free = |given: &mut HashSet<String>, wanted: String| {
        let name = unique_file_name(&wanted, |name| given.contains(name) || taken(name));
        given.insert(name.clone());
        name
    };
    // A file's path in the folder, copied there under `wanted` when it's somewhere else
    let place = |given: &mut HashSet<String>, path: &Path, wanted: String| -> Result<String, PresetError> {
        let path = std::fs::canonicalize(path)
            .map_err(|error| PresetError::Unreadable { path: path.to_owned(), reason: error.to_string() })?;
        if let Ok(inside) = path.strip_prefix(&root) {
            return Ok(inside.to_string_lossy().replace('\\', "/"));
        }
        let name = free(given, wanted);
        std::fs::copy(&path, root.join(&name)).map_err(|error| unwritable(&root.join(&name), error))?;
        Ok(name)
    };
    let mut made = Vec::new();
    for (index, step) in steps.into_iter().enumerate() {
        let stem = sanitize_file_name(&step.split.name, "split");
        let mask = place(&mut given, &step.mask, format!("{stem}.png"))?;
        // Under its own name, which says where it is in the game
        let savestate = match &step.savestate {
            Some(path) => place(
                &mut given,
                path,
                path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or(stem.clone()),
            )?,
            None => String::new(),
        };
        let base = free(&mut given, format!("base_{stem}.png"));
        let png = base_image(step.screenshot.view())
            .encode_png()
            .map_err(|error| PresetError::Unwritable { path: root.join(&base), reason: error.to_string() })?;
        std::fs::write(root.join(&base), png).map_err(|error| unwritable(&root.join(&base), error))?;
        made.push(LibrarySplit {
            split: PresetSplit { mask, savestate, with_previous: index > 0 && step.split.with_previous, ..step.split },
            description: String::new(),
            base_image: base,
            then: index > 0,
        });
    }
    Ok(described(made, description))
}

/// Writes `now` as the library in `folder`, and deletes the masks and base images `before` used
/// that it doesn't.
fn write_library(folder: &Path, before: &SplitLibrary, now: &SplitLibrary) -> Result<(), PresetError> {
    let file = folder.join(LIBRARY_FILE);
    let text = toml::to_string(now).expect("libraries always serialize");
    std::fs::write(&file, text).map_err(|error| unwritable(&file, error))?;
    let used = library_files(&now.splits);
    for old in library_files(&before.splits).difference(&used) {
        let path = folder.join(old);
        // Only in the library's own folder
        if path.parent().is_some_and(|parent| parent == folder) && path.is_file() {
            let _ = std::fs::remove_file(path);
        }
    }
    Ok(())
}

/// The masks, savestates and base images `splits` use, as they write them.
fn library_files(splits: &[LibrarySplit]) -> HashSet<String> {
    (splits.iter())
        .flat_map(|split| [&split.split.mask, &split.split.savestate, &split.base_image])
        .filter(|path| !path.is_empty())
        .cloned()
        .collect()
}

fn unwritable(path: &Path, error: std::io::Error) -> PresetError {
    PresetError::Unwritable { path: path.to_owned(), reason: error.to_string() }
}

fn missing(folder: &Path, index: usize) -> PresetError {
    PresetError::Invalid {
        path: folder.join(LIBRARY_FILE),
        reason: format!("it has no ready-made split {}", index + 1),
    }
}
