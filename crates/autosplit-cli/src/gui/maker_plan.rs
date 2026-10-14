//! What saving a preset's split images does to the split image folder, worked out before any
//! file is written, so it can be asked about first.

use std::path::{Path, PathBuf};

use autosplit_core::filename::{PartKind, RESET_KEYWORD, START_KEYWORD, has_keyword, name_parts, number_prefix};

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

fn is_png(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
}

/// Writing `names` into a folder that has `existing` files. The split images of the preset, all
/// its file names in `preset`, are never stale, like the ones compared for the same split.
/// `sources`, the files the split images are made from, aren't either, and neither are
/// screenshots (`NNN_SplitImage.png`): their numbers count from 1 where split images count from
/// 0, so they'd look like the next split's.
pub fn plan_writes(names: &[String], preset: &[String], existing: &[PathBuf], sources: &[PathBuf]) -> WritePlan {
    let file_name = |path: &PathBuf| path.file_name().map(|name| name.to_string_lossy().into_owned());
    let same_name = |name: &str, other: &str| name.eq_ignore_ascii_case(other);
    let keys: Vec<SplitKey> = names.iter().filter_map(|name| split_key(name)).collect();
    let mut plan = WritePlan::default();
    for path in existing.iter().filter(|path| is_png(path)) {
        let Some(existing_name) = file_name(path) else { continue };
        if names.iter().any(|name| same_name(name, &existing_name)) {
            plan.overwrites.push(existing_name);
        } else if !sources.contains(path)
            && !preset.iter().any(|name| same_name(name, &existing_name))
            && !is_screenshot_name(&existing_name)
            && split_key(&existing_name).is_some_and(|key| keys.contains(&key))
        {
            plan.stale.push(path.clone());
        }
    }
    plan
}

/// Whether each split's saved split image, in `saved`, is in `folder` under the split's file
/// name in `names`. The saved paths are kept when one isn't, so a name that changes and changes
/// back, like while editing the preset, counts again.
pub fn saves_in_place(saved: &[Option<PathBuf>], names: &[String], folder: Option<&Path>) -> Vec<bool> {
    let canonical = |path: &Path| std::fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
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
