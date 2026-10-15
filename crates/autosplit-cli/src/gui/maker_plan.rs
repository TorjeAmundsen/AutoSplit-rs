//! What saving a preset's split images does to the split image folder: worked out before any
//! file is written, so it can be asked about first, and the screenshots moved out after.

use std::path::{Path, PathBuf};

use autosplit_core::filename::{PartKind, RESET_KEYWORD, START_KEYWORD, has_keyword, name_parts, number_prefix};
use autosplit_core::preset::{LoadedPreset, game_key};
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

/// The screenshots to move out of the split image folder after a save: those of the split images
/// `done`, but not one a split image that `failed` is still made from, which saving again needs,
/// and not one of the preset's split images, in `keep`, picked as a screenshot.
pub fn screenshots_to_move(done: &[PathBuf], failed: &[PathBuf], keep: &[PathBuf]) -> Vec<PathBuf> {
    let failed: Vec<PathBuf> = failed.iter().map(|path| canonical(path)).collect();
    let keep: Vec<PathBuf> = keep.iter().map(|path| canonical(path)).collect();
    let mut moving: Vec<PathBuf> = Vec::new();
    for path in done {
        let real = canonical(path);
        if !failed.contains(&real) && !keep.contains(&real) && !moving.iter().any(|path| canonical(path) == real) {
            moving.push(path.clone());
        }
    }
    moving
}

/// Moves screenshots that are in the split image folder into its `base` folder, where the
/// split image loader doesn't look. A screenshot whose name is taken there gets a free one, like
/// `001_SplitImage (1).png`, since screenshot numbers are used again once theirs moved. Returns
/// where each went, also when one couldn't be moved, with why.
pub fn move_to_base(folder: &Path, sources: &[PathBuf]) -> (Vec<(PathBuf, PathBuf)>, Option<String>) {
    let mut moves = Vec::new();
    let Ok(real_folder) = std::fs::canonicalize(folder) else { return (moves, None) };
    for source in sources {
        let in_folder = source.parent().map(canonical).as_ref() == Some(&real_folder);
        let (Some(parent), Some(name)) = (source.parent(), file_name_text(source)) else { continue };
        if !in_folder || !source.is_file() {
            continue;
        }
        let base = parent.join("base");
        let target = base.join(autosplit_core::preset::unique_file_name(&name, |name| base.join(name).exists()));
        let result = std::fs::create_dir_all(&base).and_then(|()| std::fs::rename(source, &target));
        if let Err(error) = result {
            return (moves, Some(format!("Couldn't move {} to the base folder: {error}", source.display())));
        }
        moves.push((source.clone(), target));
    }
    (moves, None)
}

/// Whether each split's saved split image, in `saved`, is in `folder` under the split's file
/// name in `names`. The saved paths are kept when one isn't, so a name that changes and changes
/// back, like while editing the preset, counts again.
pub fn saves_in_place(saved: &[Option<PathBuf>], names: &[String], folder: Option<&Path>) -> Vec<bool> {
    let folder = folder.map(canonical);
    (saved.iter().enumerate())
        .map(|(index, saved)| {
            saved.as_ref().is_some_and(|path| {
                path.is_file()
                    && path.parent().map(canonical) == folder
                    && path
                        .file_name()
                        .is_some_and(|name| names.get(index).is_some_and(|wanted| name == wanted.as_str()))
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
}
