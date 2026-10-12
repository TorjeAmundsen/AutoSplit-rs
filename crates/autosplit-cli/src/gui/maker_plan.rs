//! What saving a preset's split images does to the split image folder, worked out before any
//! file is written, so it can be asked about first.

use std::path::{Path, PathBuf};

use autosplit_core::filename::{RESET_KEYWORD, START_KEYWORD, has_keyword, number_prefix};

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

/// Writing `names` into a folder that has `existing` files.
pub fn plan_writes(names: &[String], existing: &[PathBuf]) -> WritePlan {
    let file_name = |path: &PathBuf| path.file_name().map(|name| name.to_string_lossy().into_owned());
    let same_name = |name: &str, other: &str| name.eq_ignore_ascii_case(other);
    let keys: Vec<SplitKey> = names.iter().filter_map(|name| split_key(name)).collect();
    let mut plan = WritePlan::default();
    for path in existing.iter().filter(|path| is_png(path)) {
        let Some(existing_name) = file_name(path) else { continue };
        if names.iter().any(|name| same_name(name, &existing_name)) {
            plan.overwrites.push(existing_name);
        } else if split_key(&existing_name).is_some_and(|key| keys.contains(&key)) {
            plan.stale.push(path.clone());
        }
    }
    plan
}
