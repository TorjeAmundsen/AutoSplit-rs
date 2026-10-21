//! Presets shared as `.zip` files: a preset's `preset.toml` with the masks and savestates it
//! uses, and nothing else from its folder. Importing reads the whole archive first and checks
//! it, so a preset someone sent can't write outside its own folder, use files from elsewhere on
//! this computer, or fill the disk.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::image::BgraImage;
use crate::preset::{
    PRESET_FILE, Preset, PresetError, SAVESTATE_NOTE, SAVESTATE_NOTE_FILE, SAVESTATES_FOLDER, leave_for_deleting,
    move_aside, new_preset_folder, remove_leftovers, unreadable, unwritable, write_through_temporary,
};

/// The most files an archive can have, far more than any preset uses.
pub const MAX_FILES: usize = 4096;
/// The most an archive's files can take unpacked, far more than any preset uses.
const MAX_SIZE: u64 = 256 * 1024 * 1024;

/// The start of the hidden name a preset's folder has while it's imported.
const IMPORTING_PREFIX: &str = ".importing-";
/// How old a folder an import left has to be before another import deletes it, so one still
/// being written, like by another AutoSplit, stays.
const LEFTOVER_AGE: Duration = Duration::from_secs(60 * 60);
/// The start of the hidden name a replaced preset's folder has until the import is in its place.
/// Unlike what deleting leaves, these aren't deleted later, since one can be all that's left of
/// a preset when putting it back failed.
const REPLACED_PREFIX: &str = ".replaced-";

/// A preset read from an archive and checked, with the files it uses, by their paths in it.
#[derive(Debug)]
pub struct PresetArchive {
    pub preset: Preset,
    files: Vec<(String, Vec<u8>)>,
}

/// `path` as a preset writes it: relative parts joined by `/`, or `None` when it leaves the
/// folder or names a drive, like `../x.png`, `/x.png` or `C:x.png`.
fn inside(path: &str) -> Option<String> {
    let mut parts = Vec::new();
    for part in path.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => return None,
            part if part.contains(':') => return None,
            part => parts.push(part),
        }
    }
    // Absolute paths start with a separator, which leaves an empty first part
    (!parts.is_empty() && !path.starts_with(['/', '\\'])).then(|| parts.join("/"))
}

/// The files a preset uses, each once however its path is written, like `a.png` and `./a.png`,
/// as [`inside`] writes them, each with the first split that uses it. Or why not, when a split
/// uses a path outside the preset's folder.
fn used_files(preset: &Preset) -> Result<Vec<(usize, String)>, String> {
    let mut used: Vec<(usize, String)> = Vec::new();
    for (index, split) in preset.splits.iter().enumerate() {
        for path in [&split.mask, &split.savestate].into_iter().filter(|path| !path.is_empty()) {
            let outside =
                || format!("split {} ({}) uses {path}, which is outside the preset's folder", index + 1, split.name);
            let name = inside(path).ok_or_else(outside)?;
            if !used.iter().any(|(_, seen)| *seen == name) {
                used.push((index, name));
            }
        }
    }
    Ok(used)
}

/// Writes the preset in `folder` to the archive `archive`, replacing it: `preset.toml` and the
/// masks and savestates it uses. A file it uses that's missing, or outside its folder, is an
/// error, since the archive would be no use without it, and so is a preset too big to import.
pub fn export_preset(folder: &Path, archive: &Path) -> Result<(), PresetError> {
    let preset_file = folder.join(PRESET_FILE);
    let text = std::fs::read(&preset_file).map_err(|error| unreadable(&preset_file, error))?;
    let preset = Preset::parse(&String::from_utf8_lossy(&text), &preset_file)?;
    let used = used_files(&preset).map_err(|reason| PresetError::Invalid { path: preset_file.clone(), reason })?;
    let too_big = |reason: String| PresetError::Unwritable { path: archive.to_owned(), reason };
    if used.len() + 1 > MAX_FILES {
        return Err(too_big(format!("the preset has more than {MAX_FILES} files, more than importing takes")));
    }
    let mut total = text.len() as u64;
    let mut files = vec![(PRESET_FILE.to_owned(), text)];
    for (_, name) in used {
        let file = folder.join(&name);
        let contents = std::fs::read(&file).map_err(|error| unreadable(&file, error))?;
        total += contents.len() as u64;
        if total > MAX_SIZE {
            let size = MAX_SIZE / 1024 / 1024;
            return Err(too_big(format!("the preset's files take more than {size} MB, more than importing takes")));
        }
        files.push((name, contents));
    }
    write_through_temporary(archive, |temporary| write_zip(temporary, &files))
        .map_err(|error| unwritable(archive, error))
}

fn write_zip(path: &Path, files: &[(String, Vec<u8>)]) -> std::io::Result<()> {
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path)?);
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, contents) in files {
        zip.start_file(name.as_str(), options)?;
        zip.write_all(contents)?;
    }
    zip.finish()?.sync_all()
}

/// Reads and checks the preset in the archive `archive`. Its `preset.toml` is at the top, or in
/// one folder, as zipping a preset's folder makes it. Every mask and savestate it uses has to be
/// in the archive, and masks have to be PNG images. Other files are left out. Errors name the
/// archive by its file name, which says which one without the folders it was picked from.
pub fn read_archive(archive: &Path) -> Result<PresetArchive, PresetError> {
    let shown = archive.file_name().map_or_else(|| archive.to_owned(), PathBuf::from);
    let invalid = |reason: String| PresetError::Invalid { path: shown.clone(), reason };
    let file = std::fs::File::open(archive).map_err(|error| unreadable(&shown, error))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| invalid("it isn't a zip file".to_owned()))?;
    if zip.len() > MAX_FILES {
        return Err(invalid(format!("it has more than {MAX_FILES} files")));
    }
    let damaged = || invalid("it's damaged, so its files can't be read".to_owned());
    let mut files: HashMap<String, Vec<u8>> = HashMap::new();
    let mut total = 0u64;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|_| damaged())?;
        if entry.is_dir() {
            continue;
        }
        let Some(name) = inside(entry.name()) else {
            return Err(invalid(format!("it has {}, which would be outside the preset's folder", entry.name())));
        };
        // Read no more than what's left, whatever the archive says the file's size is
        let left = MAX_SIZE - total;
        let mut contents = Vec::new();
        (&mut entry).take(left + 1).read_to_end(&mut contents).map_err(|_| damaged())?;
        total += contents.len() as u64;
        if total > MAX_SIZE {
            return Err(invalid(format!("its files take more than {} MB unpacked", MAX_SIZE / 1024 / 1024)));
        }
        files.insert(name, contents);
    }
    let roots: Vec<&str> = (files.keys())
        .filter_map(|name| name.strip_suffix(PRESET_FILE))
        .filter(|folder| folder.is_empty() || (folder.matches('/').count() == 1 && folder.ends_with('/')))
        .collect();
    let root = match roots.as_slice() {
        [] => return Err(invalid("it has no preset in it".to_owned())),
        [root] => root.to_string(),
        // The top one, when one is in a folder too
        roots if roots.contains(&"") => String::new(),
        _ => return Err(invalid("it has several presets, and a file for importing can only have one".to_owned())),
    };
    let text = String::from_utf8_lossy(&files[&format!("{root}{PRESET_FILE}")]).into_owned();
    let mut preset = Preset::parse(&text, &shown)?;
    let used = used_files(&preset).map_err(invalid)?;
    let masks: HashSet<String> = preset.splits.iter().filter_map(|split| inside(&split.mask)).collect();
    let mut kept = Vec::new();
    for (index, name) in used {
        let split = &preset.splits[index].name;
        let Some(contents) = files.remove(&format!("{root}{name}")) else {
            return Err(invalid(format!("split {} ({split}) uses {name}, which isn't in the file", index + 1)));
        };
        if masks.contains(&name) && BgraImage::decode_png(&contents).is_err() {
            return Err(invalid(format!("the mask {name} of split {} ({split}) isn't a PNG image", index + 1)));
        }
        kept.push((name, contents));
    }
    // Written like the files are, so the preset finds them on every system
    for split in &mut preset.splits {
        for field in [&mut split.mask, &mut split.savestate] {
            if let Some(name) = inside(field) {
                *field = name;
            }
        }
    }
    Ok(PresetArchive { preset, files: kept })
}

impl PresetArchive {
    /// Puts the preset in a new folder in `presets`, named after it, or in place of the preset
    /// folder `replace`. It's written to a hidden folder first, which isn't a preset, so a failed
    /// import leaves nothing. A replaced preset is only deleted once the new one is ready, and
    /// stays when it can't be, like when a file in it is open. Returns the preset's folder.
    pub fn install(&self, presets: &Path, replace: Option<&Path>) -> Result<PathBuf, PresetError> {
        remove_leftovers(presets, IMPORTING_PREFIX, LEFTOVER_AGE);
        std::fs::create_dir_all(presets).map_err(|error| unwritable(presets, error))?;
        // Made here rather than found free, so two imports at once never share one
        let mut staging = PathBuf::new();
        for n in 1.. {
            staging = presets.join(format!("{IMPORTING_PREFIX}{n}"));
            match std::fs::create_dir(&staging) {
                Ok(()) => break,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(unwritable(&staging, error)),
            }
        }
        if let Err(error) = self.write(&staging) {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error);
        }
        let (target, moved) = match replace {
            Some(folder) => (folder.to_owned(), swap(folder, &staging)),
            None => {
                let target = new_preset_folder(presets, &self.preset.name);
                let moved = std::fs::rename(&staging, &target).map_err(|error| unwritable(&target, error));
                (target, moved)
            }
        };
        if moved.is_err() {
            let _ = std::fs::remove_dir_all(&staging);
        }
        moved.map(|()| target)
    }

    /// Writes the preset's files, the savestate note when it has savestates, and `preset.toml`
    /// into the empty folder `folder`. Each file is new: two names this system takes for one
    /// file, like `M.png` and `m.png` on Windows, are an error rather than one file written over
    /// the other.
    fn write(&self, folder: &Path) -> Result<(), PresetError> {
        let toml = self.preset.to_toml();
        let note = format!("{SAVESTATES_FOLDER}/{SAVESTATE_NOTE_FILE}");
        let has_savestates = (self.files.iter()).any(|(name, _)| name.split('/').next() == Some(SAVESTATES_FOLDER));
        let note = has_savestates.then_some((note.as_str(), SAVESTATE_NOTE.as_bytes()));
        let files = (self.files.iter().map(|(name, contents)| (name.as_str(), contents.as_slice())))
            .chain(note)
            .chain([(PRESET_FILE, toml.as_bytes())]);
        for (name, contents) in files {
            let path = folder.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|error| unwritable(parent, error))?;
            }
            let written = (std::fs::OpenOptions::new().write(true).create_new(true).open(&path))
                .and_then(|mut file| file.write_all(contents));
            match written {
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let reason = "the preset has another file with the same name on this system, like M.png and m.png \
                                  on Windows"
                        .to_owned();
                    return Err(PresetError::Unwritable { path: name.into(), reason });
                }
                written => written.map_err(|error| unwritable(&path, error))?,
            }
        }
        Ok(())
    }
}

/// Puts `staging` in place of the preset folder `folder`. The old folder is moved aside first,
/// and back when the new one can't take its place. When it can't go back either, it's kept
/// aside, and the error says where.
fn swap(folder: &Path, staging: &Path) -> Result<(), PresetError> {
    let aside = move_aside(folder, REPLACED_PREFIX)?;
    if let Err(error) = std::fs::rename(staging, folder) {
        if let Err(back) = std::fs::rename(&aside, folder) {
            let name = folder.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
            let reason = format!(
                "{error}. The old preset couldn't be put back ({back}), so it's kept in {}. Rename that folder to {name} to get it back",
                aside.display()
            );
            return Err(PresetError::Unwritable { path: folder.to_owned(), reason });
        }
        return Err(unwritable(folder, error));
    }
    // Not a preset anymore
    if std::fs::remove_dir_all(&aside).is_err() {
        leave_for_deleting(&aside);
    }
    Ok(())
}
