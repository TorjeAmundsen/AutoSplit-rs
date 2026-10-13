//! A preset being edited in Make Split Images: its changes, the list operations that keep the
//! window's other per-split lists in step, and what has to be fixed before it can be saved.

use autosplit_core::filename::{RESET_KEYWORD, START_KEYWORD, validate_split_name};
use autosplit_core::preset::{DEFAULT_THRESHOLD, Preset, PresetSplit};
use autosplit_core::split_folder::MAX_GROUP_SIZE;

/// A number field's value as the number it shows: `1.3` stays 1.3 instead of becoming
/// 1.2999999523162842, which the field's 32-bit float would otherwise give file names.
pub fn field_value(value: f32) -> f64 {
    value.to_string().parse().unwrap_or(f64::from(value))
}

/// A change to the order of the splits, to make in lists that go with them too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListChange {
    /// A split was put at this index.
    Insert(usize),
    Remove(usize),
    Swap(usize, usize),
}

impl ListChange {
    /// Makes the change in `items`, one per split. Inserted splits get the default.
    pub fn mirror<T: Default>(self, items: &mut Vec<T>) {
        match self {
            Self::Insert(index) => items.insert(index.min(items.len()), T::default()),
            Self::Remove(index) if index < items.len() => {
                items.remove(index);
            }
            Self::Remove(_) => {}
            Self::Swap(a, b) if a.max(b) < items.len() => items.swap(a, b),
            Self::Swap(..) => {}
        }
    }
}

/// A field of a split, as the form has it.
#[derive(Clone, Debug, PartialEq)]
pub enum SplitEdit {
    Name(String),
    /// A path relative to the preset's folder, or a picked file's absolute path.
    Mask(String),
    Threshold(f64),
    /// Seconds, or `None` for AutoSplit's default pause time.
    Pause(Option<f64>),
    /// Milliseconds.
    Delay(u32),
    Dummy(bool),
    Below(bool),
    /// Compared for the same split as the split before it.
    WithPrevious(bool),
    /// Like the mask. Empty for none, which also clears the instructions.
    Savestate(String),
    Instructions(String),
}

/// Something to fix before saving, of one split or of the preset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub split: Option<usize>,
    pub message: String,
}

/// The preset being edited, and the one it started as, to tell whether it changed.
#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub preset: Preset,
    original: Preset,
    /// Not saved anywhere yet, so it counts as changed.
    new: bool,
}

impl Draft {
    /// Editing a saved preset.
    pub fn edit(preset: Preset) -> Self {
        Self { original: preset.clone(), preset, new: false }
    }

    /// A new preset for `game`, with one split to start from.
    pub fn new(game: &str) -> Self {
        let preset = Preset { name: "New Preset".into(), game: game.into(), splits: vec![new_split("Split1")] };
        Self { original: preset.clone(), preset, new: true }
    }

    pub fn has_changes(&self) -> bool {
        self.new || self.preset != self.original
    }

    /// Puts an edit of split `index` into the preset.
    pub fn edit_split(&mut self, index: usize, edit: SplitEdit) {
        let Some(split) = self.preset.splits.get_mut(index) else { return };
        match edit {
            SplitEdit::Name(name) => split.name = name,
            SplitEdit::Mask(mask) => split.mask = mask,
            // The default isn't written, so the preset keeps following it
            SplitEdit::Threshold(threshold) => {
                let threshold = (threshold.clamp(0.0, 1.0) * 100.0).round() / 100.0;
                split.threshold = (threshold != DEFAULT_THRESHOLD).then_some(threshold);
            }
            SplitEdit::Pause(pause) => split.pause = pause.map(|pause| pause.max(0.0)),
            SplitEdit::Delay(delay) => split.delay = delay,
            SplitEdit::Dummy(on) => split.dummy = on,
            SplitEdit::Below(on) => split.below = on,
            SplitEdit::WithPrevious(on) => split.with_previous = on,
            SplitEdit::Savestate(savestate) => {
                if savestate.is_empty() {
                    split.instructions.clear();
                }
                split.savestate = savestate;
            }
            SplitEdit::Instructions(text) => split.instructions = text,
        }
    }

    /// A new split after `selected`, or at the end. Returns where it is.
    pub fn add(&mut self, selected: Option<usize>) -> (usize, ListChange) {
        let at = selected.map_or(self.preset.splits.len(), |index| (index + 1).min(self.preset.splits.len()));
        let name = (1..)
            .map(|n| format!("Split{n}"))
            .find(|name| self.preset.splits.iter().all(|split| split.name != *name))
            .expect("some number is free");
        self.preset.splits.insert(at, new_split(&name));
        (at, ListChange::Insert(at))
    }

    /// A copy of split `index` after it, with the same mask and savestate.
    pub fn duplicate(&mut self, index: usize) -> Option<(usize, ListChange)> {
        let copy = self.preset.splits.get(index)?.clone();
        self.preset.splits.insert(index + 1, copy);
        Some((index + 1, ListChange::Insert(index + 1)))
    }

    /// Removes split `index`. Returns the split to select after it, if any is left.
    pub fn remove(&mut self, index: usize) -> Option<(Option<usize>, ListChange)> {
        if index >= self.preset.splits.len() {
            return None;
        }
        self.preset.splits.remove(index);
        let left = self.preset.splits.len();
        Some(((left > 0).then(|| index.min(left - 1)), ListChange::Remove(index)))
    }

    /// Moves split `index` one up or down. Returns where it went.
    pub fn move_split(&mut self, index: usize, up: bool) -> Option<(usize, ListChange)> {
        let to = if up { index.checked_sub(1)? } else { index + 1 };
        if to >= self.preset.splits.len() || index >= self.preset.splits.len() {
            return None;
        }
        self.preset.splits.swap(index, to);
        Some((to, ListChange::Swap(index, to)))
    }

    /// What has to be fixed before saving, the preset's first, then each split's in order.
    pub fn problems(&self) -> Vec<Problem> {
        let preset = self.preset_problems().into_iter().map(|message| Problem { split: None, message });
        let splits = self.preset.splits.iter().enumerate().flat_map(|(index, split)| {
            split_problems(split).into_iter().map(move |message| Problem { split: Some(index), message })
        });
        let mut problems: Vec<Problem> = preset.chain(splits).collect();
        problems
            .extend(self.group_problems().into_iter().map(|(index, message)| Problem { split: Some(index), message }));
        // Stable, so each split's problems stay in the order they're found
        problems.sort_by_key(|problem| problem.split.map_or(0, |index| index + 1));
        problems
    }

    /// Whether the split's name has to change: it isn't a valid name, or a split it's compared
    /// together with has it.
    pub fn name_has_problem(&self, index: usize) -> bool {
        let Some(split) = self.preset.splits.get(index) else { return false };
        validate_split_name(&split.name).is_err() || self.preset.same_name_in_group(index).is_some()
    }

    /// Splits compared together, or with the same file name: too many reset or start images,
    /// too many splits compared together, splits with no split to be compared with, splits
    /// compared together with the same name, and split images that would be written over each
    /// other.
    fn group_problems(&self) -> Vec<(usize, String)> {
        let preset = &self.preset;
        let mut problems = Vec::new();
        let mut group_size = 0;
        for (index, split) in preset.splits.iter().enumerate() {
            if preset.keyword_place(index) > Some(MAX_GROUP_SIZE) {
                let name = &split.name;
                problems.push((index, format!("A preset can have up to {MAX_GROUP_SIZE} {name} images.")));
            }
            group_size = if preset.joins_previous(index) { group_size + 1 } else { 1 };
            if group_size == MAX_GROUP_SIZE + 1 {
                let message = format!("Up to {MAX_GROUP_SIZE} splits can be compared together.");
                problems.push((index, message));
            }
            if split.with_previous && !preset.joins_previous(index) {
                let message = if split.is_keyword() {
                    format!("A {} image can't be compared with the split before it.", split.name)
                } else {
                    "There's no split before it to compare it with.".to_owned()
                };
                problems.push((index, message));
            }
        }
        let same_name: Vec<usize> =
            (0..preset.splits.len()).filter(|&index| preset.same_name_in_group(index).is_some()).collect();
        for &index in &same_name {
            let message = "It has the same name as a split it's compared together with. Give it another name.";
            problems.push((index, message.to_owned()));
        }
        let names = preset.file_names(None);
        for (index, name) in names.iter().enumerate() {
            if same_name.contains(&index) {
                continue;
            }
            if let Some(first) = names[..index].iter().position(|other| other.eq_ignore_ascii_case(name)) {
                let other = &preset.splits[first].name;
                problems.push((
                    index,
                    format!("It has the same file name as {other} above. Change its name or a setting."),
                ));
            }
        }
        problems
    }

    fn preset_problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if self.preset.name.trim().is_empty() {
            problems.push("The preset needs a name.".to_owned());
        }
        if self.preset.splits.is_empty() {
            problems.push("The preset needs a split. Add one.".to_owned());
        } else if self.preset.splits.iter().all(PresetSplit::is_keyword) {
            problems.push(format!("The preset needs a split besides the {START_KEYWORD} and {RESET_KEYWORD} images."));
        }
        problems
    }
}

/// A split with nothing set, which the default threshold applies to.
fn new_split(name: &str) -> PresetSplit {
    PresetSplit { name: name.into(), ..PresetSplit::default() }
}

fn split_problems(split: &PresetSplit) -> Vec<String> {
    let mut problems = Vec::new();
    if let Err(error) = validate_split_name(&split.name) {
        problems.push(error.to_string());
    }
    if split.mask.is_empty() {
        problems.push("It needs a mask. Choose one.".to_owned());
    }
    problems
}
