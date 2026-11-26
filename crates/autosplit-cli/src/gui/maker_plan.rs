//! What saving a preset's split images does to the split image folder: worked out before any
//! file is written, so it can be asked about first, and the screenshots moved out after.

use std::path::{Path, PathBuf};

use autosplit_core::filename::{PartKind, RESET_KEYWORD, START_KEYWORD, has_keyword, name_parts, number_prefix};
use autosplit_core::preset::{LoadedPreset, game_key};
use autosplit_core::split_folder::reads_as_png;
use same_file::is_same_file;

use super::paths::{canonical, file_name_text, is_png};
use crate::screenshot::is_screenshot_name;

/// Whether the file `name` looks like a split image made already, which masking again would ruin:
/// it has a number, the reset or start keyword, or a setting. Names with a screenshot's
/// `NNN_SplitImage` in them don't count, copies of screenshots too.
pub fn looks_made(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let screenshot =
        lower.match_indices("_splitimage").any(|(at, _)| lower[..at].ends_with(|c: char| c.is_ascii_digit()));
    !screenshot && name_parts(name).iter().any(|part| !matches!(part.kind, PartKind::Text | PartKind::Extension))
}

/// The files a save replaces, and the older split images of the same splits it leaves behind.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WritePlan {
    /// File names in the folder that are written again.
    pub overwrites: Vec<String>,
    /// Images of the same splits under other names, from before a setting changed the name. The
    /// folder would use them too.
    pub stale: Vec<PathBuf>,
}

/// Which split a file name is for: its number, or the start or reset keyword.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SplitKey {
    Number(u64),
    Start,
    Reset,
}

fn split_key(name: &str) -> Option<SplitKey> {
    let name = name.to_lowercase();
    if has_keyword(&name, START_KEYWORD) {
        return Some(SplitKey::Start);
    }
    if has_keyword(&name, RESET_KEYWORD) {
        return Some(SplitKey::Reset);
    }
    // By value, so "5_Boss" and "05_Boss" are the same split, like when the preset grows
    number_prefix(&name)?.parse().ok().map(SplitKey::Number)
}

/// Whether writing the file `name` into `folder` writes over `path`: when it has that name, or
/// when the file system treats both names as one file. Windows and macOS do for names that only
/// differ in letter case, like `Bossé` and `BOSSÉ`, and macOS also for two Unicode forms of `é`.
/// Linux usually doesn't, so there they're two files, which both count for the split.
pub fn writes_over(folder: &Path, name: &str, path: &Path) -> bool {
    file_name_text(path).as_deref() == Some(name) || is_same_file(folder.join(name), path).unwrap_or(false)
}

/// Writing `names` into a folder that has `existing` files, where `writes_over` says whether
/// writing a name writes over an existing file, like [`writes_over`] does. The split images of the
/// preset, all its file names in `preset`, are never stale, like the ones compared for the same
/// split. `sources`, the files the split images are made from, aren't either, and neither are
/// screenshots (`NNN_SplitImage.png`): their numbers count from 1 where split images count from
/// 0, so they'd look like the next split's.
pub fn plan_writes(
    names: &[String],
    preset: &[String],
    existing: &[PathBuf],
    sources: &[PathBuf],
    writes_over: impl Fn(&str, &Path) -> bool,
) -> WritePlan {
    let keys: Vec<SplitKey> = names.iter().filter_map(|name| split_key(name)).collect();
    let mut plan = WritePlan::default();
    for path in existing.iter().filter(|path| is_png(path)) {
        let Some(existing_name) = file_name_text(path) else { continue };
        if names.iter().any(|name| writes_over(name, path)) {
            plan.overwrites.push(existing_name);
        } else if !sources.contains(path)
            && !preset.iter().any(|name| writes_over(name, path))
            && !is_screenshot_name(&existing_name)
            && split_key(&existing_name).is_some_and(|key| keys.contains(&key))
        {
            plan.stale.push(path.clone());
        }
    }
    plan
}

/// The images a save of every split moves into a backup folder: the files among a folder's
/// `existing` ones that a folder load reads as split images, whatever their extension, and that
/// aren't one of the preset's file names in `preset`. `writes_over` tells
/// whether a name is that file, as for [`plan_writes`]. All the preset's names count, so splits
/// that are off or have no screenshot yet keep theirs. Not `sources` or screenshots
/// (`NNN_SplitImage.png`) either, which aren't split images.
pub fn old_images(
    preset: &[String],
    existing: &[PathBuf],
    sources: &[PathBuf],
    writes_over: impl Fn(&str, &Path) -> bool,
) -> Vec<PathBuf> {
    (existing.iter())
        .filter(|path| !sources.contains(path) && reads_as_png(path))
        .filter(|path| file_name_text(path).is_some_and(|name| !is_screenshot_name(&name)))
        .filter(|path| !preset.iter().any(|name| writes_over(name, path)))
        .cloned()
        .collect()
}

/// The `old` images to move into a backup folder, without those of the splits in `kept`, each its
/// file name and its name, which aren't written: one may only have an image under an older name,
/// like after the preset changed its threshold, and would be left without. A numbered split's are
/// found by number and name, like [`images_of_splits`] finds them, so another split's image with
/// its number still moves. A reset or start split keeps every reset or start image.
pub fn images_to_back_up(old: &[PathBuf], kept: &[(&str, &str)]) -> Vec<PathBuf> {
    let keys: Vec<SplitKey> = kept.iter().filter_map(|(file, _)| split_key(file)).collect();
    let numbered: Vec<(usize, &str)> = (kept.iter())
        .filter_map(|&(file, name)| match split_key(file)? {
            SplitKey::Number(number) => Some((usize::try_from(number).ok()?, name)),
            SplitKey::Start | SplitKey::Reset => None,
        })
        .collect();
    let of_numbered = images_of_splits(old, &numbered);
    let of_keyword = |path: &Path| {
        let key = file_name_text(path).and_then(|name| split_key(&name));
        key.is_some_and(|key| matches!(key, SplitKey::Start | SplitKey::Reset) && keys.contains(&key))
    };
    (old.iter()).filter(|path| !of_numbered.contains(path) && !of_keyword(path)).cloned().collect()
}

/// The reset and start images among the `old` ones when the preset, all its file names in
/// `preset`, has none of that kind: likely the user's own, and moving them turns off resetting or
/// starting by itself.
pub fn lone_keyword_images(old: &[PathBuf], preset: &[String]) -> Vec<PathBuf> {
    let keys: Vec<SplitKey> = preset.iter().filter_map(|name| split_key(name)).collect();
    (old.iter())
        .filter(|path| {
            let key = file_name_text(path).and_then(|name| split_key(&name));
            key.is_some_and(|key| matches!(key, SplitKey::Start | SplitKey::Reset) && !keys.contains(&key))
        })
        .cloned()
        .collect()
}

/// Moves `images` into a new folder in the split image `folder`, where the split image loader
/// doesn't look, named for the time `now` like `backup_2026-11-26_143005_UTC`, or `backup_... (1)`
/// when that's taken. Images that are gone already are left out. Returns the folder and how many
/// moved, also when one couldn't be, with why, like [`move_to_base`]. No folder is made when
/// there's nothing to move.
pub fn move_to_backup(folder: &Path, images: &[PathBuf], now: &str) -> (PathBuf, usize, Option<String>) {
    let name = autosplit_core::preset::unique_file_name(&format!("backup_{now}"), |name| folder.join(name).exists());
    let backup = folder.join(name);
    let (moves, error) = move_into(&backup, images, &backup.display().to_string());
    (backup, moves.len(), error)
}

/// The `stale` images to delete once the split images `written` are: only those of a split whose
/// new image was written, so a split whose image failed keeps its old one. Never one of the
/// images written, in case the file system treats its old name as the new one.
pub fn stale_to_delete(stale: &[PathBuf], written: &[PathBuf]) -> Vec<PathBuf> {
    let keys: Vec<SplitKey> =
        written.iter().filter_map(|path| file_name_text(path)).filter_map(|name| split_key(&name)).collect();
    (stale.iter())
        .filter(|path| file_name_text(path).and_then(|name| split_key(&name)).is_some_and(|key| keys.contains(&key)))
        .filter(|path| !written.iter().any(|written| is_same_file(written, path).unwrap_or(false)))
        .cloned()
        .collect()
}

/// The split images among a folder's `existing` files of `splits`, each its number and name,
/// like the splits just turned off: PNGs named for one of them, its number, by value, so
/// `5_Boss` and `05_Boss` are both 5, then `_` and its name, followed by the extension or by
/// `_` and a setting, like `_(0.9)`. The name is checked too, since an image from an older version of the
/// preset can have the number of another split. Not reset or start images, and not screenshots
/// (`NNN_SplitImage`), whose numbers count from 1.
pub fn images_of_splits(existing: &[PathBuf], splits: &[(usize, &str)]) -> Vec<PathBuf> {
    let of_split = |file: &str, number: usize, name: &str| {
        let Some(SplitKey::Number(found)) = split_key(file) else { return false };
        let digits = number_prefix(file).map_or(0, str::len);
        let rest = file[digits..].strip_prefix('_').and_then(|rest| rest.strip_prefix(name));
        // The extension, or a setting, not more of a longer name like Boss_2's
        let ends = |rest: &str| {
            rest.starts_with('.')
                || rest.strip_prefix('_').is_some_and(|setting| setting.starts_with(['(', '[', '#', '@', '{', '^']))
        };
        found == number as u64 && rest.is_some_and(ends)
    };
    (existing.iter())
        .filter(|path| is_png(path))
        .filter(|path| {
            file_name_text(path).is_some_and(|file| {
                !is_screenshot_name(&file) && splits.iter().any(|&(number, name)| of_split(&file, number, name))
            })
        })
        .cloned()
        .collect()
}

/// What the window says about a preset's screenshots: `with` of its splits have one, and
/// `without` have to be grabbed, fade splits counted in neither, since they're made from their
/// color. None for a preset without screenshots, which is grabbed like always. Short, since it
/// shares its line with the grouping warning.
pub fn screenshots_status(with: usize, without: usize) -> Option<String> {
    let splits = match with {
        0 => return None,
        1 => "1 split has".to_owned(),
        with => format!("{with} splits have"),
    };
    Some(match without {
        0 => format!("{splits} a screenshot. Click Save All."),
        without => format!("{splits} a screenshot for Save All, {without} left to grab."),
    })
}

/// Where a split's row is in the split list, which hides some rows, like splits that are off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowPlace {
    pub hidden: bool,
    /// Its place among the shown rows, from 0. For a hidden row, the place the next shown one has.
    pub place: usize,
    /// The shown rows above and below it, which Up and Down go to.
    pub above: Option<usize>,
    pub below: Option<usize>,
}

/// Each row's place in the split list, where `shown` says which rows are shown.
pub fn row_places(shown: &[bool]) -> Vec<RowPlace> {
    let mut places = Vec::with_capacity(shown.len());
    let (mut place, mut above) = (0, None);
    for (index, &is_shown) in shown.iter().enumerate() {
        places.push(RowPlace { hidden: !is_shown, place, above, below: None });
        if is_shown {
            (place, above) = (place + 1, Some(index));
        }
    }
    let mut below = None;
    for (index, &is_shown) in shown.iter().enumerate().rev() {
        places[index].below = below;
        if is_shown {
            below = Some(index);
        }
    }
    places
}

/// What the strip under the split list says when not choosing: how many of the splits the runner
/// can turn on or off are `off`, and how many splits that are off still have their image in the
/// split image folder, `kept`, which AutoSplit compares anyway.
pub fn choice_summary(off: usize, kept: usize) -> String {
    let off = match off {
        0 => return "Every split is on.".to_owned(),
        1 => "1 split is off".to_owned(),
        off => format!("{off} splits are off"),
    };
    match kept {
        0 => format!("{off}."),
        kept => format!("{off}, {kept} still compared."),
    }
}

/// What the window says after `picked` screenshots were set in order, from the first, for the
/// `count` splits that take one: past `fades` fade splits, made from their color, and `off`
/// splits that are off, which get no split image.
pub fn screenshots_set_status(picked: usize, count: usize, fades: usize, off: usize) -> String {
    // For each split, and for the splits
    let (each, all) = match (fades > 0, off > 0) {
        (false, false) => ("", ""),
        (true, false) => (" that isn't a fade split", " that aren't fade splits"),
        (false, true) => (" that's on", " that are on"),
        (true, true) => (" that's on and isn't a fade split", " that are on and aren't fade splits"),
    };
    let save = "Save All writes their split images.";
    match picked {
        _ if picked == count => format!("{picked} screenshots set, one for each split{each}. {save}"),
        _ if all.is_empty() => format!("{picked} screenshots for {count} splits, set in order from the first. {save}"),
        _ => format!("{picked} screenshots for the {count} splits{all}, set in order from the first. {save}"),
    }
}

/// The split to select after grabbing split `after`, where `skipped` says which splits aren't
/// grabbed, like ones made from their preset's screenshot: the next one that is, so playing
/// through and pressing the screenshot hotkey never replaces one. None when no split after it is
/// left.
pub fn next_to_grab(skipped: &[bool], after: usize) -> Option<usize> {
    (after + 1..skipped.len()).find(|&index| !skipped[index])
}

/// Where a split image saved from its preset's screenshot at `saved_from` is from once the
/// preset is saved again: `new`, the screenshot's file now, when it was saved from `old`, the
/// screenshot just before the save, which saving at most copied. None when it's from another
/// screenshot than the preset has now, so it isn't the split's image anymore, even when the new
/// screenshot's file gets the old one's name.
pub fn saved_from_after_preset_save(saved_from: &Path, old: Option<&Path>, new: Option<&Path>) -> Option<PathBuf> {
    new.filter(|_| old == Some(saved_from)).map(Path::to_owned)
}

/// The screenshots to move out of the split image folder after a save: those of the split images
/// `done`, but not one a split image that `failed` is still made from, which saving again needs,
/// and not one of the preset's split images, in `keep`, picked as a screenshot.
pub fn screenshots_to_move(done: &[PathBuf], failed: &[PathBuf], keep: &[PathBuf]) -> Vec<PathBuf> {
    let failed: Vec<PathBuf> = failed.iter().map(|path| canonical(path)).collect();
    let keep: Vec<PathBuf> = keep.iter().map(|path| canonical(path)).collect();
    let mut moving: Vec<PathBuf> = Vec::new();
    // The canonical paths of those moving, to move each file once
    let mut seen: Vec<PathBuf> = Vec::new();
    for path in done {
        let real = canonical(path);
        if !failed.contains(&real) && !keep.contains(&real) && !seen.contains(&real) {
            moving.push(path.clone());
            seen.push(real);
        }
    }
    moving
}

/// Moves screenshots that are in the split image folder into its `base` folder, where the
/// split image loader doesn't look. A screenshot whose name is taken there gets a free one, like
/// `001_SplitImage (1).png`, since screenshot numbers are used again once theirs moved. Returns
/// where each went, also when one couldn't be moved, with why.
pub fn move_to_base(folder: &Path, sources: &[PathBuf]) -> (Vec<(PathBuf, PathBuf)>, Option<String>) {
    let Ok(real_folder) = std::fs::canonicalize(folder) else { return (Vec::new(), None) };
    let in_folder: Vec<PathBuf> = (sources.iter())
        .filter(|source| source.parent().map(canonical).as_ref() == Some(&real_folder))
        .cloned()
        .collect();
    move_into(&folder.join("base"), &in_folder, "the base folder")
}

/// Moves `files` into the folder `into`, made once one moves, each under its name or a free one
/// like `name (1).png`. Files that are gone already are left out. Returns where each went, and
/// when one couldn't be moved, why, saying it was moved to `place`, and that the files after it
/// stay where they are.
fn move_into(into: &Path, files: &[PathBuf], place: &str) -> (Vec<(PathBuf, PathBuf)>, Option<String>) {
    let mut moves = Vec::new();
    let files: Vec<&PathBuf> = files.iter().filter(|file| file.is_file()).collect();
    for (at, file) in files.iter().enumerate() {
        let Some(name) = file_name_text(file) else { continue };
        let target = into.join(autosplit_core::preset::unique_file_name(&name, |name| into.join(name).exists()));
        let result = std::fs::create_dir_all(into).and_then(|()| std::fs::rename(file, &target));
        if let Err(error) = result {
            let rest = if at + 1 < files.len() { ". The files after it are still where they were." } else { "" };
            return (moves, Some(format!("Couldn't move {} to {place}: {error}{rest}", file.display())));
        }
        moves.push(((*file).clone(), target));
    }
    (moves, None)
}

/// Whether each split's saved split image, in `saved`, is in `folder` under the split's file
/// name in `names`. The saved paths are kept when one isn't, so a name that changes and changes
/// back, like while editing the preset, counts again.
/// It runs on every change while a preset is edited, so each folder is made canonical once.
pub fn saves_in_place(saved: &[Option<PathBuf>], names: &[String], folder: Option<&Path>) -> Vec<bool> {
    let folder = folder.map(canonical);
    // Each folder a saved split image is in, and whether it's `folder`
    let mut parents: Vec<(PathBuf, bool)> = Vec::new();
    let mut in_folder = |parent: &Path| -> bool {
        if let Some((_, found)) = parents.iter().find(|(known, _)| known == parent) {
            return *found;
        }
        let found = Some(canonical(parent)) == folder;
        parents.push((parent.to_owned(), found));
        found
    };
    (saved.iter().enumerate())
        .map(|(index, saved)| {
            saved.as_ref().is_some_and(|path| {
                let named =
                    path.file_name().is_some_and(|name| names.get(index).is_some_and(|wanted| name == wanted.as_str()));
                named && path.parent().is_some_and(&mut in_folder) && path.is_file()
            })
        })
        .collect()
}

/// Where a deleted preset was: its game, its place among the game's presets, and the game's place
/// among the games, all from 0.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeletedPreset {
    pub game: String,
    pub place: usize,
    pub game_place: usize,
}

/// The preset to show after `deleted` is gone, by index into the presets left, whose games are
/// `games` in list order: the one that took its place in its game, or the game's last, like
/// deleting from a list. When the game has none left, the first preset of the game that took the
/// game's place, or of the last game.
pub fn preset_after_delete(games: &[&str], deleted: &DeletedPreset) -> usize {
    let same: Vec<usize> = (0..games.len()).filter(|&index| games[index] == deleted.game).collect();
    if let Some(&index) = same.get(deleted.place).or(same.last()) {
        return index;
    }
    let mut firsts: Vec<usize> = Vec::new();
    for index in 0..games.len() {
        if index == 0 || games[index] != games[index - 1] {
            firsts.push(index);
        }
    }
    firsts.get(deleted.game_place).or(firsts.last()).copied().unwrap_or(0)
}

/// The presets as Make Split Images lists them, sorted by game and then name, with what the window
/// asks of them. Games are the same however they're spelled, by `game_key`.
#[derive(Clone, Debug, Default)]
pub struct PresetList(Vec<LoadedPreset>);

impl PresetList {
    /// `presets` sorted like `load_presets` sorts them.
    pub fn new(presets: Vec<LoadedPreset>) -> Self {
        Self(presets)
    }

    pub fn get(&self, index: usize) -> Option<&LoadedPreset> {
        self.0.get(index)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &LoadedPreset> {
        self.0.iter()
    }

    /// The games, in the order they're listed, each as its first preset spells it.
    pub fn games(&self) -> Vec<&str> {
        let mut games: Vec<&str> = self.0.iter().map(|loaded| loaded.preset.game.as_str()).collect();
        // Sorted by game, so the presets of a game are next to each other
        games.dedup_by(|game, before| game_key(game) == game_key(before));
        games
    }

    /// Where `game` is among the games.
    pub fn game_index(&self, game: &str) -> Option<usize> {
        let key = game_key(game);
        self.games().iter().position(|&each| game_key(each) == key)
    }

    /// The presets of `game`, by index.
    pub fn presets_of(&self, game: &str) -> Vec<usize> {
        let key = game_key(game);
        (0..self.0.len()).filter(|&index| game_key(&self.0[index].preset.game) == key).collect()
    }

    /// The preset in `folder`, however its path is written.
    pub fn find(&self, folder: &Path) -> Option<usize> {
        let folder = canonical(folder);
        self.0.iter().position(|loaded| canonical(&loaded.folder) == folder)
    }

    /// Where preset `index` is, to show its neighbor once it's deleted.
    pub fn place(&self, index: usize) -> DeletedPreset {
        let game = self.0.get(index).map(|loaded| loaded.preset.game.clone()).unwrap_or_default();
        DeletedPreset {
            place: self.presets_of(&game).iter().position(|&each| each == index).unwrap_or(0),
            game_place: self.game_index(&game).unwrap_or(0),
            game,
        }
    }

    /// The preset to show now that `deleted` is gone from the presets, as `preset_after_delete`
    /// picks it.
    pub fn after_delete(&self, deleted: &DeletedPreset) -> usize {
        let games: Vec<String> = self.0.iter().map(|loaded| game_key(&loaded.preset.game)).collect();
        let games: Vec<&str> = games.iter().map(String::as_str).collect();
        preset_after_delete(&games, &DeletedPreset { game: game_key(&deleted.game), ..deleted.clone() })
    }

    /// The names of `game`'s presets, which another of its presets can't have, besides the one in
    /// `except`, which is being saved again.
    pub fn taken_names(&self, game: &str, except: Option<&Path>) -> Vec<&str> {
        let (key, except) = (game_key(game), except.map(canonical));
        (self.0.iter())
            .filter(|loaded| game_key(&loaded.preset.game) == key)
            .filter(|loaded| except.as_ref().is_none_or(|except| canonical(&loaded.folder) != *except))
            .map(|loaded| loaded.preset.name.as_str())
            .collect()
    }

    /// `game`'s preset named `name`, if it has one. Games and names are the same ignoring case
    /// and extra spaces, like the lists tell them apart.
    pub fn named(&self, game: &str, name: &str) -> Option<&LoadedPreset> {
        let (game, name) = (game_key(game), game_key(name));
        (self.0.iter()).find(|loaded| game_key(&loaded.preset.game) == game && game_key(&loaded.preset.name) == name)
    }
}
