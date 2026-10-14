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
    /// The splits from `start` to before `end` turned left by `left`, like a group of splits
    /// moved past the split or group before it.
    Rotate {
        start: usize,
        end: usize,
        left: usize,
    },
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
            Self::Rotate { start, end, left } if start + left <= end && end <= items.len() => {
                items[start..end].rotate_left(left);
            }
            Self::Rotate { .. } => {}
        }
    }
}

/// What a split is, which AutoSplit reads from its name: a split, or the image that starts or
/// resets the run, named exactly `start_auto_splitter` or `reset`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitKind {
    Split,
    Start,
    Reset,
}

impl SplitKind {
    /// In the order the form lists them.
    pub const ALL: [Self; 3] = [Self::Split, Self::Start, Self::Reset];

    pub fn of(split: &PresetSplit) -> Self {
        match split.name.as_str() {
            START_KEYWORD => Self::Start,
            RESET_KEYWORD => Self::Reset,
            _ => Self::Split,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Split => "Split",
            Self::Start => "Start image",
            Self::Reset => "Reset image",
        }
    }
}

/// A field of a split, as the form has it.
#[derive(Clone, Debug, PartialEq)]
pub enum SplitEdit {
    /// Sets the name for a start or reset image. A split gets a new name of its own.
    Kind(SplitKind),
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

/// The field of a split that a problem is about, to show it next to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Name,
    Mask,
    /// Same split as the one above.
    Group,
}

/// Something to fix before saving, of one split or of the preset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub split: Option<usize>,
    /// The split's field it's about. None for the preset's.
    pub field: Option<Field>,
    pub message: String,
}

/// The preset being edited, and the one it started as, to tell whether it changed.
#[derive(Clone, Debug, PartialEq)]
pub struct Draft {
    pub preset: Preset,
    original: Preset,
    /// Not saved anywhere yet, so it can be saved unchanged.
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

    /// Whether it changed since editing started, which closing asks about.
    pub fn has_changes(&self) -> bool {
        self.preset != self.original
    }

    /// Whether Save does something: it changed, or it's new.
    pub fn can_save(&self) -> bool {
        self.new || self.has_changes()
    }

    /// Puts an edit of split `index` into the preset.
    pub fn edit_split(&mut self, index: usize, edit: SplitEdit) {
        if let SplitEdit::Kind(kind) = edit {
            return self.set_kind(index, kind);
        }
        let Some(split) = self.preset.splits.get_mut(index) else { return };
        match edit {
            SplitEdit::Kind(_) => {}
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

    /// The splits compared together with split `index`, itself too, as the range of their indexes.
    fn group(&self, index: usize) -> std::ops::Range<usize> {
        let start = (0..=index).rev().find(|&first| !self.preset.joins_previous(first)).unwrap_or(0);
        let end = (index + 1..self.preset.splits.len()).find(|&next| !self.preset.joins_previous(next));
        start..end.unwrap_or(self.preset.splits.len())
    }

    /// Makes split `index` a split, start image or reset image. Those can't be compared with the
    /// split above.
    fn set_kind(&mut self, index: usize, kind: SplitKind) {
        let Some(split) = self.preset.splits.get(index) else { return };
        if SplitKind::of(split) == kind {
            return;
        }
        let name = match kind {
            SplitKind::Start => START_KEYWORD.to_owned(),
            SplitKind::Reset => RESET_KEYWORD.to_owned(),
            SplitKind::Split => self.free_split_name(),
        };
        let split = &mut self.preset.splits[index];
        split.name = name;
        if kind != SplitKind::Split {
            split.with_previous = false;
        }
    }

    /// The extensions of the preset's savestates, in lowercase, each once in the order they come,
    /// for picking another savestate of the game's practice tool.
    pub fn savestate_extensions(&self) -> Vec<String> {
        let mut extensions: Vec<String> = Vec::new();
        for split in &self.preset.splits {
            let extension = std::path::Path::new(&split.savestate).extension();
            let Some(extension) = extension.map(|extension| extension.to_string_lossy().to_lowercase()) else {
                continue;
            };
            if !extensions.contains(&extension) {
                extensions.push(extension);
            }
        }
        extensions
    }

    /// `Split1`, `Split2` and so on, whichever no split has.
    fn free_split_name(&self) -> String {
        (1..)
            .map(|n| format!("Split{n}"))
            .find(|name| self.preset.splits.iter().all(|split| split.name != *name))
            .expect("some number is free")
    }

    /// A new split after `selected` and the splits compared together with it, or at the end.
    /// Returns where it is.
    pub fn add(&mut self, selected: Option<usize>) -> (usize, ListChange) {
        let count = self.preset.splits.len();
        let at = selected.filter(|&index| index < count).map_or(count, |index| self.group(index).end);
        let name = self.free_split_name();
        self.preset.splits.insert(at, new_split(&name));
        (at, ListChange::Insert(at))
    }

    /// A copy of split `index` with the same mask and savestate, after the splits compared
    /// together with it. A copy of one compared with the split above is compared with them too,
    /// and a copy of the first is a split of its own.
    pub fn duplicate(&mut self, index: usize) -> Option<(usize, ListChange)> {
        let copy = self.preset.splits.get(index)?.clone();
        let at = self.group(index).end;
        self.preset.splits.insert(at, copy);
        Some((at, ListChange::Insert(at)))
    }

    /// Removes split `index`. Returns the split to select after it, if any is left.
    pub fn remove(&mut self, index: usize) -> Option<(Option<usize>, ListChange)> {
        if index >= self.preset.splits.len() {
            return None;
        }
        // The next split compared with it is the first of the ones left, not compared with the
        // split before them
        let first = !self.preset.joins_previous(index);
        self.preset.splits.remove(index);
        if let Some(next) = self.preset.splits.get_mut(index).filter(|_| first) {
            next.with_previous = false;
        }
        let left = self.preset.splits.len();
        Some(((left > 0).then(|| index.min(left - 1)), ListChange::Remove(index)))
    }

    /// Moves split `index` one up or down: inside the splits compared together with it, which
    /// stay together, or with them past the split or splits before or after them. Returns where
    /// it went.
    pub fn move_split(&mut self, index: usize, up: bool) -> Option<(usize, ListChange)> {
        if index >= self.preset.splits.len() {
            return None;
        }
        let group = self.group(index);
        let inside = if up { index > group.start } else { index + 1 < group.end };
        if inside {
            let to = if up { index - 1 } else { index + 1 };
            let splits = &mut self.preset.splits;
            splits.swap(index, to);
            // Which one is compared with the split above stays with the place
            let (a, b) = (splits[index].with_previous, splits[to].with_previous);
            (splits[index].with_previous, splits[to].with_previous) = (b, a);
            return Some((to, ListChange::Swap(index, to)));
        }
        let (start, end, left, to) = if up {
            let before = self.group(group.start.checked_sub(1)?);
            (before.start, group.end, group.start - before.start, index - before.len())
        } else {
            if group.end >= self.preset.splits.len() {
                return None;
            }
            let after = self.group(group.end);
            (group.start, after.end, group.len(), index + after.len())
        };
        self.preset.splits[start..end].rotate_left(left);
        Some((to, ListChange::Rotate { start, end, left }))
    }

    /// What has to be fixed before saving, the preset's first, then each split's in order.
    pub fn problems(&self) -> Vec<Problem> {
        let preset = self.preset_problems().into_iter().map(|message| Problem { split: None, field: None, message });
        let splits = self.preset.splits.iter().enumerate().flat_map(|(index, split)| {
            split_problems(split).into_iter().map(move |(field, message)| Problem {
                split: Some(index),
                field: Some(field),
                message,
            })
        });
        let mut problems: Vec<Problem> = preset.chain(splits).collect();
        problems.extend((self.group_problems().into_iter()).map(|(index, field, message)| Problem {
            split: Some(index),
            field: Some(field),
            message,
        }));
        // Stable, so each split's problems stay in the order they're found
        problems.sort_by_key(|problem| problem.split.map_or(0, |index| index + 1));
        problems
    }

    /// What's wrong with `field` of split `index`, one problem a line, or empty.
    pub fn field_problems(&self, index: usize, field: Field) -> String {
        (self.problems().into_iter())
            .filter(|problem| problem.split == Some(index) && problem.field == Some(field))
            .map(|problem| problem.message)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Splits compared together: too many reset or start images, too many splits compared
    /// together, splits with no split to be compared with, and splits compared together with the
    /// same name. Those are the only splits that could get the same file name: the others have
    /// numbers of their own, and reset and start images past the first get their place.
    fn group_problems(&self) -> Vec<(usize, Field, String)> {
        let preset = &self.preset;
        let mut problems = Vec::new();
        let mut group_size = 0;
        for (index, split) in preset.splits.iter().enumerate() {
            if preset.keyword_place(index) > Some(MAX_GROUP_SIZE) {
                let name = &split.name;
                problems.push((index, Field::Name, format!("A preset can have up to {MAX_GROUP_SIZE} {name} images.")));
            }
            group_size = if preset.joins_previous(index) { group_size + 1 } else { 1 };
            if group_size == MAX_GROUP_SIZE + 1 {
                let message = format!("Up to {MAX_GROUP_SIZE} splits can be compared together.");
                problems.push((index, Field::Group, message));
            }
            if split.with_previous && !preset.joins_previous(index) {
                let message = if split.is_keyword() {
                    format!("A {} image can't be compared with the split before it.", split.name)
                } else {
                    "There's no split before it to compare it with.".to_owned()
                };
                problems.push((index, Field::Group, message));
            }
        }
        for index in (0..preset.splits.len()).filter(|&index| preset.same_name_in_group(index).is_some()) {
            let message = "It has the same name as a split it's compared together with. Give it another name.";
            problems.push((index, Field::Name, message.to_owned()));
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

fn split_problems(split: &PresetSplit) -> Vec<(Field, String)> {
    let mut problems = Vec::new();
    if let Err(error) = validate_split_name(&split.name) {
        problems.push((Field::Name, error.to_string()));
    }
    if split.mask.is_empty() {
        problems.push((Field::Mask, "It needs a mask. Choose one.".to_owned()));
    }
    problems
}
