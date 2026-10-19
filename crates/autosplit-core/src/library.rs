//! Writing the splits library: ready-made splits made from a preset's splits, added to the
//! library of their game, changed or deleted. The library's file is written again each time, and
//! masks, savestates and base images no ready-made split uses anymore are deleted.
//!
//! A write happens whole or not at all. Every file is found before anything is written, new
//! files that take over an old file's name are written under another name first, and the
//! library's file is written to a temporary file and renamed. Only then do the new files take
//! their names and the old ones go. Anything written before a failure is deleted again.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::image::{BgraImage, BgraView};
use crate::imgproc::stretch_to_rgba;
use crate::preset::{
    LIBRARY_FILE, LibrarySplit, PresetError, PresetSplit, SplitLibrary, game_key, load_libraries, new_preset_folder,
    relative_text, sanitize_file_name, unique_file_name, unwritable,
};

/// The size base images are made at, which the Split Library shows them at.
pub const BASE_IMAGE_SIZE: (usize, usize) = (64, 48);

/// The start of the name a file gets while it's written, before it takes over an old file's.
const WRITING_PREFIX: &str = ".writing-";

/// An image of a ready-made split to write: a preset's split, with its mask and savestate as
/// paths that exist, and a screenshot the mask fits, at any size. Only a fade split can be without
/// a mask.
pub struct NewStep {
    pub split: PresetSplit,
    pub mask: Option<PathBuf>,
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
    let (libraries, errors) = load_libraries(directory);
    let found = libraries.into_iter().find(|loaded| game_key(&loaded.library.game) == game_key(game));
    let (folder, before, new_folder) = match found {
        Some(loaded) => (loaded.folder, loaded.library, false),
        // One that can't be read may be this game's, which a new one would be made beside
        None if !errors.is_empty() => {
            let errors: Vec<String> = errors.iter().map(ToString::to_string).collect();
            return Err(PresetError::Invalid {
                path: directory.to_owned(),
                reason: format!(
                    "a library there can't be read, and it may be this game's. Fix it first:\n\n{}",
                    errors.join("\n\n")
                ),
            });
        }
        None => {
            let folder = new_preset_folder(directory, game);
            std::fs::create_dir_all(&folder).map_err(|error| unwritable(&folder, error))?;
            (folder, SplitLibrary { game: game.to_owned(), splits: Vec::new() }, true)
        }
    };
    let written =
        store_steps(&folder, description, steps, &HashSet::new(), &HashSet::new()).and_then(|(stored, pending)| {
            let mut now = before.clone();
            now.splits.extend(stored);
            write_library(&folder, &before, &now, pending, &[])
        });
    match written {
        Ok(()) => Ok(folder),
        Err(error) => {
            // A library that was made for it goes with it
            if new_folder {
                let _ = std::fs::remove_dir_all(&folder);
            }
            Err(error)
        }
    }
}

/// Changes the `index`th ready-made split of the library in `folder`, which has to still be
/// `expected`: its description, and its images and settings when there are new `steps`. Files of
/// `in_use`, which a preset being edited still takes from the library, are never written over or
/// deleted.
pub fn change_ready_made(
    folder: &Path,
    index: usize,
    expected: &[LibrarySplit],
    description: &str,
    steps: Option<Vec<NewStep>>,
    in_use: &[PathBuf],
) -> Result<(), PresetError> {
    let before = SplitLibrary::load(&folder.join(LIBRARY_FILE))?;
    let mut chunks = expected_chunks(folder, &before, index, expected)?;
    // The files only this one uses, whose names its new files can take over
    let others: Vec<LibrarySplit> = (chunks.iter().enumerate())
        .filter(|&(other, _)| other != index)
        .flat_map(|(_, chunk)| chunk.iter().cloned())
        .collect();
    let replaced: HashSet<String> =
        library_files(&chunks[index]).difference(&library_files(&others)).cloned().collect();
    let (stored, pending) = match steps {
        Some(steps) => {
            // Masks of images given another name, which take that name too, in lowercase
            let renamed: HashSet<String> = (chunks[index].iter().zip(&steps))
                .filter(|(old, new)| old.split.name != new.split.name && replaced.contains(&old.split.mask))
                .map(|(old, _)| old.split.mask.to_lowercase())
                .collect();
            store_steps(folder, description, steps, &replaced, &renamed)?
        }
        None => (described(std::mem::take(&mut chunks[index]), description), Pending::default()),
    };
    chunks[index] = stored;
    let now = SplitLibrary { game: before.game.clone(), splits: chunks.concat() };
    write_library(folder, &before, &now, pending, in_use)
}

/// Deletes the `index`th ready-made split of the library in `folder`, which has to still be
/// `expected`, with the files only it used, unless a preset being edited still takes one of them
/// from the library, which `in_use` says.
pub fn delete_ready_made(
    folder: &Path,
    index: usize,
    expected: &[LibrarySplit],
    in_use: &[PathBuf],
) -> Result<(), PresetError> {
    let before = SplitLibrary::load(&folder.join(LIBRARY_FILE))?;
    let mut chunks = expected_chunks(folder, &before, index, expected)?;
    chunks.remove(index);
    let now = SplitLibrary { game: before.game.clone(), splits: chunks.concat() };
    write_library(folder, &before, &now, Pending::default(), in_use)
}

/// The ready-made splits of `library`, when its `index`th is still `expected`, which it may not
/// be after the library was changed meanwhile.
fn expected_chunks(
    folder: &Path,
    library: &SplitLibrary,
    index: usize,
    expected: &[LibrarySplit],
) -> Result<Vec<Vec<LibrarySplit>>, PresetError> {
    let chunks: Vec<Vec<LibrarySplit>> = library.ready_made().map(<[LibrarySplit]>::to_vec).collect();
    if chunks.get(index).is_some_and(|chunk| chunk.as_slice() == expected) {
        return Ok(chunks);
    }
    // Named by the split it's for, its last image, like Split Library names it
    let name = expected.last().map(|step| step.split.name.as_str()).unwrap_or("The ready-made split");
    Err(PresetError::Unwritable {
        path: folder.join(LIBRARY_FILE),
        reason: format!(
            "{name} changed in it since you opened it, so nothing was written. Open it again and try once more."
        ),
    })
}

/// The description on the first image, which the Split Library shows, and none on the others.
fn described(mut steps: Vec<LibrarySplit>, description: &str) -> Vec<LibrarySplit> {
    for (index, step) in steps.iter_mut().enumerate() {
        step.description = if index == 0 { description.to_owned() } else { String::new() };
    }
    steps
}

/// Files written for a ready-made split that the library's file doesn't name yet: some under
/// their own names, and some under another name until they take over an old file's.
#[derive(Default)]
struct Pending {
    made: Vec<PathBuf>,
    renames: Vec<(PathBuf, PathBuf)>,
}

impl Pending {
    /// Deletes what was written, when the library isn't written after all.
    fn discard(self) {
        for path in self.made.iter().chain(self.renames.iter().map(|(from, _)| from)) {
            let _ = std::fs::remove_file(path);
        }
    }

    /// The new files take their names, over the old ones.
    fn commit(self) -> Result<(), PresetError> {
        for (from, to) in &self.renames {
            std::fs::rename(from, to).map_err(|error| unwritable(to, error))?;
        }
        Ok(())
    }
}

/// Puts the masks, savestates and base images of `steps` in `folder`, and makes their library
/// splits. A file in the folder already, like one of a ready-made split that was inserted, is
/// used where it is. The `replaced` files, which only the ready-made split these replace used,
/// are written over once the library is, so its new files keep their names instead of getting a
/// number. The `renamed` masks, in lowercase, are of images given another name, so they're copied
/// under it instead of used where they are. When something can't be found or written, what was
/// written is deleted again.
fn store_steps(
    folder: &Path,
    description: &str,
    steps: Vec<NewStep>,
    replaced: &HashSet<String>,
    renamed: &HashSet<String>,
) -> Result<(Vec<LibrarySplit>, Pending), PresetError> {
    let root = std::fs::canonicalize(folder).map_err(|error| unwritable(folder, error))?;
    let found = |path: &Path| {
        std::fs::canonicalize(path)
            .map_err(|error| PresetError::Unreadable { path: path.to_owned(), reason: error.to_string() })
    };
    // Every file first, so a missing one changes nothing
    let mut sources = Vec::new();
    for step in &steps {
        let savestate = step.savestate.as_deref().map(found).transpose()?;
        sources.push((step.mask.as_deref().map(found).transpose()?, savestate));
    }
    let inside = |path: &Path| {
        let inside = path.strip_prefix(&root).ok().map(relative_text)?;
        (!renamed.contains(&inside.to_lowercase())).then_some(inside)
    };
    // Used where they are, so never written over
    let kept: HashSet<String> = (sources.iter())
        .flat_map(|(mask, savestate)| mask.iter().chain(savestate))
        .filter_map(|path| inside(path))
        .collect();
    // The folder's files by their names in lowercase. Names that differ only in case are one
    // file on some systems, like Windows, so a library copied there couldn't have both: a new
    // file gets a number instead.
    let existing: HashMap<String, String> = (std::fs::read_dir(&root).into_iter().flatten().flatten())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .map(|name| (name.to_lowercase(), name))
        .collect();
    let taken = |name: &str| {
        (existing.get(&name.to_lowercase())).is_some_and(|actual| !replaced.contains(actual) || kept.contains(actual))
    };
    // Names given in this write, which the folder doesn't have yet, in lowercase too
    let mut given: HashSet<String> = HashSet::new();
    let mut free = |wanted: String| {
        let name = unique_file_name(&wanted, |name| given.contains(&name.to_lowercase()) || taken(name));
        // Over a replaced file whose name differs only in case: its spelling, which a system that
        // ignores case keeps on disk anyway, so the library names the file git has everywhere
        let name = existing.get(&name.to_lowercase()).cloned().unwrap_or(name);
        given.insert(name.to_lowercase());
        name
    };
    let mut pending = Pending::default();
    // Where a file named `name` is written, which is another name while an old file has it
    let target = |pending: &mut Pending, name: &str| {
        let path = root.join(name);
        if path.exists() {
            let writing = root.join(format!("{WRITING_PREFIX}{name}"));
            pending.renames.push((writing.clone(), path));
            writing
        } else {
            pending.made.push(path.clone());
            path
        }
    };
    let mut made = Vec::new();
    let written = (|| {
        for ((index, step), (mask_source, savestate_source)) in steps.into_iter().enumerate().zip(&sources) {
            let stem = sanitize_file_name(&step.split.name, "split");
            // A file's path in the folder, copied there under `wanted` when it's somewhere else
            let mut place = |source: &Path, wanted: String| -> Result<String, PresetError> {
                if let Some(inside) = inside(source) {
                    return Ok(inside);
                }
                let name = free(wanted);
                let to = target(&mut pending, &name);
                std::fs::copy(source, &to).map_err(|error| unwritable(&to, error))?;
                Ok(name)
            };
            let mask = match mask_source {
                Some(source) => place(source, format!("{stem}.png"))?,
                None => String::new(),
            };
            // Under its own name, which says where it is in the game
            let savestate = match savestate_source {
                Some(source) => {
                    let wanted = source.file_name().map(|name| name.to_string_lossy().into_owned());
                    place(source, wanted.unwrap_or(stem.clone()))?
                }
                None => String::new(),
            };
            let base = free(format!("base_{stem}.png"));
            let png = base_image(step.screenshot.view())
                .encode_png()
                .map_err(|error| PresetError::Unwritable { path: root.join(&base), reason: error.to_string() })?;
            let to = target(&mut pending, &base);
            std::fs::write(&to, png).map_err(|error| unwritable(&to, error))?;
            made.push(LibrarySplit {
                split: PresetSplit {
                    mask,
                    savestate,
                    with_previous: index > 0 && step.split.with_previous,
                    ..step.split
                },
                description: String::new(),
                base_image: base,
                then: index > 0,
            });
        }
        Ok(())
    })();
    match written {
        Ok(()) => Ok((described(made, description), pending)),
        Err(error) => {
            pending.discard();
            Err(error)
        }
    }
}

/// Writes `now` as the library in `folder`, with the `pending` files it names, and deletes the
/// files `before` used that it doesn't. Nothing is written when that would write over or delete
/// a file of `in_use`.
fn write_library(
    folder: &Path,
    before: &SplitLibrary,
    now: &SplitLibrary,
    pending: Pending,
    in_use: &[PathBuf],
) -> Result<(), PresetError> {
    // Compared as the files they are, since names written in another case can be the same file,
    // like on Windows
    let file = |name: &String| {
        let path = folder.join(name);
        std::fs::canonicalize(&path).unwrap_or(path)
    };
    let used: HashSet<PathBuf> = library_files(&now.splits).iter().map(file).collect();
    // Only in the library's own folder
    let unused: Vec<PathBuf> = (library_files(&before.splits).iter())
        .filter(|old| !used.contains(&file(old)))
        .map(|old| folder.join(old))
        .filter(|path| path.parent().is_some_and(|parent| parent == folder) && path.is_file())
        .collect();
    let in_use: HashSet<PathBuf> = in_use.iter().filter_map(|path| std::fs::canonicalize(path).ok()).collect();
    let changed = unused.iter().chain(pending.renames.iter().map(|(_, to)| to));
    if let Some(path) =
        changed.into_iter().find(|path| std::fs::canonicalize(path).is_ok_and(|path| in_use.contains(&path)))
    {
        let path = path.clone();
        pending.discard();
        return Err(PresetError::Unwritable {
            path,
            reason: "a split of the preset being edited uses it until the preset is saved. Save the preset first"
                .to_owned(),
        });
    }
    let file = folder.join(LIBRARY_FILE);
    let writing = folder.join(format!("{WRITING_PREFIX}{LIBRARY_FILE}"));
    let text = toml::to_string(now).expect("libraries always serialize");
    if let Err(error) = std::fs::write(&writing, text).and_then(|()| std::fs::rename(&writing, &file)) {
        let _ = std::fs::remove_file(&writing);
        pending.discard();
        return Err(unwritable(&file, error));
    }
    pending.commit()?;
    for path in unused {
        let _ = std::fs::remove_file(path);
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
