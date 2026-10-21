//! Presets shared as `.zip` files: a preset's `preset.toml` with the masks and savestates it
//! uses, and nothing else from its folder. Importing reads the whole archive first and checks
//! it, so a preset someone sent can't write outside its own folder, use files from elsewhere on
//! this computer, or fill the disk.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::preset::{
    DELETING_PREFIX, PRESET_FILE, Preset, PresetError, SAVESTATE_NOTE, SAVESTATE_NOTE_FILE, SAVESTATES_FOLDER,
    new_preset_folder, unwritable,
};

/// The most files an archive can have, far more than any preset uses.
pub const MAX_FILES: usize = 4096;
/// The most an archive's files can take unpacked, far more than any preset uses.
pub const MAX_SIZE: u64 = 256 * 1024 * 1024;

/// The start of the hidden name a preset's folder has while it's imported.
const IMPORTING_PREFIX: &str = ".importing-";

/// A preset read from an archive and checked, with the files it uses, by their paths in it.
#[derive(Clone, Debug, PartialEq)]
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

/// The files a preset uses, each once, as they're written in it.
fn used_files(preset: &Preset) -> Vec<(usize, &str)> {
    let mut used: Vec<(usize, &str)> = Vec::new();
    for (index, split) in preset.splits.iter().enumerate() {
        for path in [&split.mask, &split.savestate] {
            if !path.is_empty() && !used.iter().any(|(_, seen)| seen == path) {
                used.push((index, path));
            }
        }
    }
    used
}

/// Writes the preset in `folder` to the archive `archive`, replacing it: `preset.toml` and the
/// masks and savestates it uses. A file it uses that's missing, or outside its folder, is an
/// error, since the archive would be no use without it.
pub fn export_preset(folder: &Path, archive: &Path) -> Result<(), PresetError> {
    let preset_file = folder.join(PRESET_FILE);
    let text = std::fs::read(&preset_file)
        .map_err(|error| PresetError::Unreadable { path: preset_file.clone(), reason: error.to_string() })?;
    let preset = Preset::parse(&String::from_utf8_lossy(&text), &preset_file)?;
    let mut files = vec![(PRESET_FILE.to_owned(), text)];
    for (index, path) in used_files(&preset) {
        let Some(name) = inside(path) else {
            let split = &preset.splits[index].name;
            let reason = format!("split {} ({split}) uses {path}, which isn't in the preset's folder", index + 1);
            return Err(PresetError::Invalid { path: preset_file, reason });
        };
        let file = folder.join(&name);
        let contents = std::fs::read(&file)
            .map_err(|error| PresetError::Unreadable { path: file.clone(), reason: error.to_string() })?;
        files.push((name, contents));
    }
    let file_name = archive.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let temporary = archive.with_file_name(format!("{file_name}.tmp"));
    let written = write_zip(&temporary, &files).and_then(|()| std::fs::rename(&temporary, archive));
    if let Err(error) = written {
        let _ = std::fs::remove_file(&temporary);
        return Err(unwritable(archive, error));
    }
    Ok(())
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
/// in the archive, and masks have to be PNG images. Other files are left out.
pub fn read_archive(archive: &Path) -> Result<PresetArchive, PresetError> {
    let invalid = |reason: String| PresetError::Invalid { path: archive.to_owned(), reason };
    let file = std::fs::File::open(archive)
        .map_err(|error| PresetError::Unreadable { path: archive.to_owned(), reason: error.to_string() })?;
    let mut zip = zip::ZipArchive::new(file).map_err(|error| invalid(format!("it isn't a zip file ({error})")))?;
    if zip.len() > MAX_FILES {
        return Err(invalid(format!("it has more than {MAX_FILES} files")));
    }
    let mut files: HashMap<String, Vec<u8>> = HashMap::new();
    let mut total = 0u64;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|error| invalid(error.to_string()))?;
        if entry.is_dir() {
            continue;
        }
        let Some(name) = inside(entry.name()) else {
            return Err(invalid(format!("it has {}, which would be outside the preset's folder", entry.name())));
        };
        // Read no more than what's left, whatever the archive says the file's size is
        let left = MAX_SIZE - total;
        let mut contents = Vec::new();
        (&mut entry).take(left + 1).read_to_end(&mut contents).map_err(|error| invalid(format!("{name}: {error}")))?;
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
        [] => return Err(invalid(format!("it has no {PRESET_FILE}"))),
        [root] => root.to_string(),
        // The top one, when one is in a folder too
        roots => match roots.iter().find(|root| root.is_empty()) {
            Some(_) => String::new(),
            None => return Err(invalid("it has several presets. Import them one at a time.".to_owned())),
        },
    };
    let text = String::from_utf8_lossy(&files[&format!("{root}{PRESET_FILE}")]).into_owned();
    let mut preset = Preset::parse(&text, archive)?;
    let mut used = Vec::new();
    let paths: Vec<(usize, String)> = used_files(&preset).into_iter().map(|(index, path)| (index, path.to_owned())).collect();
    for (index, path) in paths {
        let split = &preset.splits[index].name;
        let Some(name) = inside(&path) else {
            return Err(invalid(format!("split {} ({split}) uses {path}, which is outside the preset's folder", index + 1)));
        };
        let Some(contents) = files.remove(&format!("{root}{name}")) else {
            return Err(invalid(format!("split {} ({split}) uses {name}, which isn't in it", index + 1)));
        };
        let is_mask = preset.splits.iter().any(|split| split.mask == path);
        if is_mask && ::image::load_from_memory_with_format(&contents, ::image::ImageFormat::Png).is_err() {
            return Err(invalid(format!("the mask {name} of split {} ({split}) isn't a PNG image", index + 1)));
        }
        // Written like the file is, so the preset finds it on every system
        for split in &mut preset.splits {
            for field in [&mut split.mask, &mut split.savestate] {
                if *field == path {
                    field.clone_from(&name);
                }
            }
        }
        used.push((name, contents));
    }
    Ok(PresetArchive { preset, files: used })
}

impl PresetArchive {
    /// Puts the preset in a new folder in `presets`, named after it, or in place of the preset
    /// folder `replace`. It's written to a hidden folder first, which isn't a preset, so a failed
    /// import leaves nothing. A replaced preset is only deleted once the new one is ready, and
    /// stays when it can't be, like when a file in it is open. Returns the preset's folder.
    pub fn install(&self, presets: &Path, replace: Option<&Path>) -> Result<PathBuf, PresetError> {
        remove_leftovers(presets, IMPORTING_PREFIX);
        let staging = (1..)
            .map(|n| presets.join(format!("{IMPORTING_PREFIX}{n}")))
            .find(|path| !path.exists())
            .expect("some number is free");
        if let Err(error) = self.write(&staging) {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(error);
        }
        let target = match replace {
            Some(folder) => folder.to_owned(),
            None => new_preset_folder(presets, &self.preset.name),
        };
        let moved = match replace {
            Some(folder) => swap(folder, &staging),
            None => std::fs::rename(&staging, &target).map_err(|error| unwritable(&target, error)),
        };
        if moved.is_err() {
            let _ = std::fs::remove_dir_all(&staging);
        }
        moved.map(|()| target)
    }

    fn write(&self, folder: &Path) -> Result<(), PresetError> {
        std::fs::create_dir_all(folder).map_err(|error| unwritable(folder, error))?;
        for (name, contents) in &self.files {
            let path = folder.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|error| unwritable(parent, error))?;
            }
            std::fs::write(&path, contents).map_err(|error| unwritable(&path, error))?;
        }
        let savestates = folder.join(SAVESTATES_FOLDER);
        if savestates.is_dir() {
            let note = savestates.join(SAVESTATE_NOTE_FILE);
            std::fs::write(&note, SAVESTATE_NOTE).map_err(|error| unwritable(&note, error))?;
        }
        let preset_file = folder.join(PRESET_FILE);
        std::fs::write(&preset_file, self.preset.to_toml()).map_err(|error| unwritable(&preset_file, error))
    }
}

/// Puts `staging` in place of the preset folder `folder`. The old folder is moved aside first,
/// and back when the new one can't take its place.
fn swap(folder: &Path, staging: &Path) -> Result<(), PresetError> {
    let parent = folder.parent().unwrap_or(Path::new("."));
    remove_leftovers(parent, DELETING_PREFIX);
    let name = folder.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let aside = (1..)
        .map(|n| parent.join(format!("{DELETING_PREFIX}{name}-{n}")))
        .find(|path| !path.exists())
        .expect("some number is free");
    std::fs::rename(folder, &aside).map_err(|error| PresetError::Unwritable {
        path: folder.to_owned(),
        reason: format!("{error}. Close any program that has a file in it open, then try again"),
    })?;
    if let Err(error) = std::fs::rename(staging, folder) {
        let _ = std::fs::rename(&aside, folder);
        return Err(unwritable(folder, error));
    }
    // Not a preset anymore, whether or not all of it goes now
    let _ = std::fs::remove_dir_all(&aside);
    Ok(())
}

/// Deletes the hidden folders in `folder` that an import or delete that stopped partway left.
fn remove_leftovers(folder: &Path, prefix: &str) {
    let Ok(entries) = std::fs::read_dir(folder) else { return };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with(prefix) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}
