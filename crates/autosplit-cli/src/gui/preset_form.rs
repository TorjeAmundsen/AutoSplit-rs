//! A preset being edited in Make Split Images: its changes, the list operations that keep the
//! window's other per-split lists in step, and what has to be fixed before it can be saved. And
//! what Split Library shows of ready-made splits next to it: their names, settings and file
//! names, whether the preset has them already, and whether there's room for them.

use std::path::Path;

use autosplit_core::filename::{
    DEFAULT_FADE_TIME, MAX_FADE_TIME, MIN_FADE_TIME, NamePart, PartKind, RESET_KEYWORD, START_KEYWORD, name_parts,
    validate_split_name,
};
use autosplit_core::preset::{Preset, PresetSplit, game_key};
use autosplit_core::split_folder::MAX_GROUP_SIZE;
use autosplit_core::split_image::SplitDefaults;

/// The threshold a split starts with once it's made a fade split: the solid frame at the end of a
/// fade reaches it, and the frame before doesn't.
pub const FADE_THRESHOLD: f64 = 0.995;
/// The color a split starts with once it's made a fade split.
pub const FADE_COLOR: &str = "#000000";

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

    /// Where it is in `ALL`, which the Type list shows in that order.
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|&each| each == self).unwrap_or_default()
    }

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
            Self::Start => "Start Image",
            Self::Reset => "Reset Image",
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
    /// Whether it's compared by a fade to a color, or else a screenshot. A fade split starts at
    /// [`FADE_THRESHOLD`], with the color and fade time it had before, or [`FADE_COLOR`].
    Compare(bool),
    /// A fade split's color, as typed.
    Color(String),
    /// A fade split's fade time in milliseconds, rounded to whole ones.
    FadeTime(f64),
    /// `None` for the profile's default similarity threshold.
    Threshold(Option<f64>),
    /// Seconds, or `None` for the profile's default pause time.
    Pause(Option<f64>),
    /// Milliseconds, rounded to whole ones, or `None` for the profile's default delay time.
    Delay(Option<f64>),
    Dummy(bool),
    Below(bool),
    /// Compared for the same split as the split before it.
    WithPrevious(bool),
    /// Like the mask. Empty for none. The instructions stay until saving, for a savestate picked
    /// again.
    Savestate(String),
    Instructions(String),
}

/// The field of a split that a problem is about, to show it next to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Name,
    Mask,
    /// A fade split's color.
    Color,
    /// Same split as the one above.
    Group,
}

impl SplitEdit {
    /// Whether it changes other fields than its own, which the form shows again then: the type
    /// sets the name and Same split as the one above, and comparing by a fade the threshold.
    pub fn changes_other_fields(&self) -> bool {
        matches!(self, Self::Kind(_) | Self::Compare(_))
    }

    /// Whether the split's mask is another one, so what's shown of the old one goes.
    pub fn changes_mask(&self) -> bool {
        matches!(self, Self::Mask(_))
    }
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

    /// Why split `index` can't be compared with the split above, for the greyed out checkbox, or
    /// `None` when it can.
    pub fn why_not_joined(&self, index: usize) -> Option<&'static str> {
        let preset = &self.preset;
        let split = preset.splits.get(index)?;
        let previous = index.checked_sub(1).and_then(|previous| preset.splits.get(previous));
        if split.is_keyword() {
            Some("Start and reset images aren't splits, so they can't be compared with one.")
        } else if previous.is_none() {
            Some("It's the first split, so there's no split above to compare it with.")
        } else if previous.is_some_and(PresetSplit::is_keyword) {
            Some("The split above is a start or reset image, which isn't a split to compare it with.")
        } else if index - self.preset.group(index - 1).start >= MAX_GROUP_SIZE {
            Some("The splits above are already 4 compared together, the most there can be.")
        } else {
            None
        }
    }

    /// The preset as it's saved, each split like [`saved_split`] says.
    pub fn to_save(&self) -> Preset {
        Preset { splits: self.preset.splits.iter().map(saved_split).collect(), ..self.preset.clone() }
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
            SplitEdit::Compare(true) if !split.is_fade() => {
                let (color, time) = split.remembered.fade.take().unwrap_or((FADE_COLOR.to_owned(), None));
                split.fade = Some(color);
                split.fade_time = Some(time.unwrap_or(DEFAULT_FADE_TIME));
                split.remembered.threshold = Some(split.threshold);
                split.threshold = Some(FADE_THRESHOLD);
            }
            SplitEdit::Compare(false) => {
                if let Some(color) = split.fade.take() {
                    split.remembered.fade = Some((color, split.fade_time.take()));
                }
                // A fade's threshold would almost never match a screenshot
                if let Some(threshold) = split.remembered.threshold.take() {
                    split.threshold = threshold;
                }
            }
            SplitEdit::Compare(true) => {}
            SplitEdit::Color(color) if split.is_fade() => split.fade = Some(color),
            SplitEdit::Color(_) => {}
            SplitEdit::FadeTime(time) if split.is_fade() => {
                split.fade_time = Some((time.round() as u32).clamp(MIN_FADE_TIME, MAX_FADE_TIME));
            }
            SplitEdit::FadeTime(_) => {}
            // The default isn't written, so the preset keeps following it
            // Three decimals, like the spin box, for fade images that need 0.995
            SplitEdit::Threshold(threshold) => {
                split.threshold = threshold.map(|threshold| (threshold.clamp(0.0, 1.0) * 1000.0).round() / 1000.0);
            }
            SplitEdit::Pause(pause) => split.pause = pause.map(|pause| (pause.max(0.0) * 100.0).round() / 100.0),
            SplitEdit::Delay(delay) => split.delay = delay.map(|delay| delay.max(0.0).round() as u32),
            SplitEdit::Dummy(on) => split.dummy = on,
            SplitEdit::Below(on) => split.below = on,
            SplitEdit::WithPrevious(on) => split.with_previous = on,
            SplitEdit::Savestate(savestate) => split.savestate = savestate,
            SplitEdit::Instructions(text) => split.instructions = text,
        }
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
        self.drop_unused_joins();
        let at = self.place_after(selected);
        let name = self.free_split_name();
        self.preset.splits.insert(at, new_split(&name));
        (at, ListChange::Insert(at))
    }

    /// A ready-made split from the splits library in `folder`, its `steps` in order, put at
    /// `at`, a place from [`Self::place_after`], [`Self::place_before`] or [`Self::drop_place`].
    /// Their masks and savestates are the library's files, which saving copies into the preset.
    /// The first isn't compared with the split before it, which isn't the library's.
    pub fn import(&mut self, at: usize, steps: &[PresetSplit], folder: &Path) -> Vec<ListChange> {
        self.drop_unused_joins();
        let at = at.min(self.preset.splits.len());
        let in_library =
            |path: &str| if path.is_empty() { String::new() } else { folder.join(path).display().to_string() };
        let mut changes = Vec::new();
        for (step, split) in steps.iter().enumerate() {
            let split = PresetSplit {
                mask: in_library(&split.mask),
                savestate: in_library(&split.savestate),
                with_previous: step > 0 && split.with_previous,
                ..split.clone()
            };
            self.preset.splits.insert(at + step, split);
            changes.push(ListChange::Insert(at + step));
        }
        changes
    }

    /// The split to select once `count` ready-made images went in at `at`, with `selected`
    /// selected before. Above the selected split, it stays selected, so the next ones go between
    /// them. Without a selected split they went at the end, and nothing stays selected, so the
    /// next ones go at the end too. Otherwise the last image is selected, so the next ones go
    /// after it. Either way they end up in the order they're put in. With nothing selected, it's
    /// past the last split.
    pub fn selected_after_import(
        &self,
        selected: Option<usize>,
        at: usize,
        count: usize,
        keep_selected: bool,
    ) -> usize {
        match selected {
            Some(selected) if keep_selected && at <= selected => selected + count,
            None if keep_selected => self.preset.splits.len(),
            _ => (at + count).saturating_sub(1),
        }
    }

    /// Where a split added after `selected` goes: after the splits compared together with it, or
    /// at the end.
    pub fn place_after(&self, selected: Option<usize>) -> usize {
        let count = self.preset.splits.len();
        selected.filter(|&index| index < count).map_or(count, |index| self.preset.group(index).end)
    }

    /// Where a split added before `selected` goes: before the splits compared together with it,
    /// or at the end.
    pub fn place_before(&self, selected: Option<usize>) -> usize {
        let count = self.preset.splits.len();
        selected.filter(|&index| index < count).map_or(count, |index| self.preset.group(index).start)
    }

    /// Where a split dropped at `gap`, the place between two rows of the list, goes: there, or
    /// at the nearer end of splits compared together, which it can't go between.
    pub fn drop_place(&self, gap: usize) -> usize {
        let count = self.preset.splits.len();
        let gap = gap.min(count);
        if gap == count || !self.preset.joins_previous(gap) {
            return gap;
        }
        let group = self.preset.group(gap);
        if gap - group.start <= group.end - gap { group.start } else { group.end }
    }

    /// A copy of split `index` with the same mask and savestate, after the splits compared
    /// together with it. A copy of one compared with the split above is compared with them too,
    /// and a copy of the first is a split of its own.
    pub fn duplicate(&mut self, index: usize) -> Option<(usize, ListChange)> {
        self.drop_unused_joins();
        let copy = self.preset.splits.get(index)?.clone();
        let at = self.preset.group(index).end;
        self.preset.splits.insert(at, copy);
        Some((at, ListChange::Insert(at)))
    }

    /// Removes split `index`. Returns the split to select after it, the one that took its place or
    /// the one before, and 0 when none is left.
    pub fn remove(&mut self, index: usize) -> Option<(usize, ListChange)> {
        if index >= self.preset.splits.len() {
            return None;
        }
        self.drop_unused_joins();
        // The next split compared with it is the first of the ones left, not compared with the
        // split before them
        let first = !self.preset.joins_previous(index);
        self.preset.splits.remove(index);
        if let Some(next) = self.preset.splits.get_mut(index).filter(|_| first) {
            next.with_previous = false;
        }
        let left = self.preset.splits.len();
        Some((index.min(left.saturating_sub(1)), ListChange::Remove(index)))
    }

    /// Moves split `index` one up or down: inside the splits compared together with it, which
    /// stay together, or with them past the split or splits before or after them. Returns where
    /// it went.
    pub fn move_split(&mut self, index: usize, up: bool) -> Option<(usize, ListChange)> {
        if index >= self.preset.splits.len() {
            return None;
        }
        self.drop_unused_joins();
        let group = self.preset.group(index);
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
            let before = self.preset.group(group.start.checked_sub(1)?);
            (before.start, group.end, group.start - before.start, index - before.len())
        } else {
            if group.end >= self.preset.splits.len() {
                return None;
            }
            let after = self.preset.group(group.end);
            (group.start, after.end, group.len(), index + after.len())
        };
        self.preset.splits[start..end].rotate_left(left);
        Some((to, ListChange::Rotate { start, end, left }))
    }

    /// Unticks Same split as the one above where it doesn't apply, like under a start or reset
    /// image after the split above became one. Adding, removing or moving splits gives splits
    /// other neighbors, which would otherwise compare such a split with a split by surprise.
    fn drop_unused_joins(&mut self) {
        for index in 0..self.preset.splits.len() {
            if !self.preset.joins_previous(index) {
                self.preset.splits[index].with_previous = false;
            }
        }
    }

    /// What has to be fixed before saving, the preset's first, then each split's in order.
    pub fn problems(&self) -> Vec<Problem> {
        problems_of(&self.preset)
    }
}

/// What has to be fixed in `preset` before it's saved or used, the preset's first, then each
/// split's in order.
pub fn problems_of(preset: &Preset) -> Vec<Problem> {
    let whole = preset_level_problems(preset).into_iter().map(|message| Problem { split: None, field: None, message });
    let splits = preset.splits.iter().enumerate().flat_map(|(index, split)| {
        split_problems(split).into_iter().map(move |(field, message)| Problem {
            split: Some(index),
            field: Some(field),
            message,
        })
    });
    let mut problems: Vec<Problem> = whole.chain(splits).collect();
    problems.extend((group_problems(preset).into_iter()).map(|(index, field, message)| Problem {
        split: Some(index),
        field: Some(field),
        message,
    }));
    // Stable, so each split's problems stay in the order they're found
    problems.sort_by_key(|problem| problem.split.map_or(0, |index| index + 1));
    problems
}

/// What of `problems` is wrong with `field` of split `index`, one problem a line, or empty.
pub fn field_problems(problems: &[Problem], index: usize, field: Field) -> String {
    (problems.iter())
        .filter(|problem| problem.split == Some(index) && problem.field == Some(field))
        .map(|problem| problem.message.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Splits compared together: too many reset or start images, too many splits compared
/// together, splits with no split to be compared with, and splits compared together with the
/// same name. Those are the only splits that could get the same file name: the others have
/// numbers of their own, and reset and start images past the first get their place.
fn group_problems(preset: &Preset) -> Vec<(usize, Field, String)> {
    let mut problems = Vec::new();
    for (index, split) in preset.splits.iter().enumerate() {
        if preset.keyword_place(index) > Some(MAX_GROUP_SIZE) {
            let kind = kind_word(split);
            problems.push((index, Field::Name, format!("A preset can have up to {MAX_GROUP_SIZE} {kind} images.")));
        }
        // Once, on the first split past the most
        if index - preset.group(index).start == MAX_GROUP_SIZE {
            let message = format!("Up to {MAX_GROUP_SIZE} splits can be compared together.");
            problems.push((index, Field::Group, message));
        }
        if split.with_previous && !preset.joins_previous(index) {
            let message = if split.is_keyword() {
                format!("A {} image can't be compared with the split before it.", kind_word(split))
            } else {
                "There's no split before it to compare it with.".to_owned()
            };
            problems.push((index, Field::Group, message));
        }
    }
    for index in (0..preset.splits.len()).filter(|&index| preset.same_name_in_group(index).is_some()) {
        let message = "It has the same name as a split above it that it's compared together with. Give it \
                       another name.";
        problems.push((index, Field::Name, message.to_owned()));
    }
    problems
}

fn preset_level_problems(preset: &Preset) -> Vec<String> {
    let mut problems = Vec::new();
    if preset.name.trim().is_empty() {
        problems.push("The preset needs a name.".to_owned());
    }
    // It's listed under its game
    if preset.game.trim().is_empty() {
        problems.push("The preset needs a game. Pick one, or type a new one.".to_owned());
    }
    if preset.splits.is_empty() {
        problems.push("The preset needs a split. Add one.".to_owned());
    } else if preset.splits.iter().all(PresetSplit::is_keyword) {
        problems.push("The preset needs a split besides the start and reset images.".to_owned());
    }
    problems
}

/// A ready-made split's heading in Split Library, like the split list's.
pub fn library_heading(split: &PresetSplit) -> String {
    match SplitKind::of(split) {
        SplitKind::Split => split.name.clone(),
        kind => kind.label().to_owned(),
    }
}

/// Whether `search` finds a ready-made split with `texts`, its names and description: every
/// word of it is in one of them, ignoring case. An empty search finds everything.
pub fn matches_search(texts: &[&str], search: &str) -> bool {
    let texts: Vec<String> = texts.iter().map(|text| text.to_lowercase()).collect();
    search.to_lowercase().split_whitespace().all(|word| texts.iter().any(|text| text.contains(word)))
}

/// The file name a ready-made split's image gets, in its parts, with `XX` for its number,
/// which depends on where it's put. Reset and start images have none. It follows the profile's
/// `defaults`, like the split list's.
pub fn library_file_parts(split: &PresetSplit, defaults: Option<&SplitDefaults>) -> Vec<NamePart> {
    let alone = Preset { splits: vec![PresetSplit { with_previous: false, ..split.clone() }], ..Preset::default() };
    let mut parts = name_parts(&alone.file_names(defaults).swap_remove(0));
    if let Some(number) = parts.first_mut().filter(|part| part.kind == PartKind::Number) {
        number.text = "XX".to_owned();
        number.meaning = "Its number, which comes from where it's put".to_owned();
    }
    parts
}

/// The file name a ready-made split's image gets, like [`library_file_parts`] says it.
pub fn library_file_name(split: &PresetSplit, defaults: Option<&SplitDefaults>) -> String {
    library_file_parts(split, defaults).into_iter().map(|part| part.text).collect()
}

/// A ready-made split's image `step` with the settings and savestate of `from`, a split selected
/// in the preset, keeping its own name and mask.
pub fn with_settings_of(step: &PresetSplit, from: &PresetSplit) -> PresetSplit {
    PresetSplit { name: step.name.clone(), mask: step.mask.clone(), ..from.clone() }
}

/// What's in the way of a ready-made split's image `step` taking the settings of `from`, the split
/// selected in the preset at its place, if anything. It has to be the same image, inserted from
/// the library, so another split's settings aren't written over it. And an image without a mask
/// has to stay a fade split, of the whole screen.
pub fn settings_problem(step: &PresetSplit, from: &PresetSplit) -> Option<String> {
    if from.name != step.name {
        return Some(format!(
            "{} is selected where {} should be. Select the ready-made split's images in Make Split Images, from its first.",
            library_heading(from),
            library_heading(step)
        ));
    }
    if step.mask.is_empty() && !from.is_fade() {
        return Some(format!("{} has no mask, so it has to stay a fade split.", library_heading(step)));
    }
    None
}

/// What taking the settings of `from` changes about a ready-made split's image `step`, like
/// From ScrubsDummyLow: threshold default to 0.92. `savestates` are the file names of its
/// savestate and the new one, or none when it's the same file.
pub fn settings_changes(step: &PresetSplit, from: &PresetSplit, savestates: Option<(String, String)>) -> String {
    fn shown<T: ToString>(value: &Option<T>) -> String {
        value.as_ref().map_or_else(|| "default".to_owned(), T::to_string)
    }
    let mut changes = Vec::new();
    let mut compare = |what: &str, old: String, new: String| {
        if old != new {
            changes.push(format!("{what} {old} to {new}"));
        }
    };
    compare("threshold", shown(&step.threshold), shown(&from.threshold));
    compare("pause", shown(&step.pause), shown(&from.pause));
    compare("delay", shown(&step.delay), shown(&from.delay));
    compare("fade", shown(&step.fade), shown(&from.fade));
    compare("fade time", shown(&step.fade_time), shown(&from.fade_time));
    compare("dummy", step.dummy.to_string(), from.dummy.to_string());
    compare("below", step.below.to_string(), from.below.to_string());
    compare("with previous", step.with_previous.to_string(), from.with_previous.to_string());
    if let Some((old, new)) = savestates {
        compare("savestate", old, new);
    }
    if step.instructions != from.instructions {
        changes.push("instructions".to_owned());
    }
    let heading = library_heading(from);
    if changes.is_empty() {
        return format!("From {heading}: no changes");
    }
    format!("From {heading}: {}", changes.join(", "))
}

/// Where a ready-made split is in the preset already.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InPreset {
    /// The file name of the first split with its name, like the split list's.
    pub file: String,
    /// A line saying where it is, like as split 05.
    pub note: String,
}

/// Where a split named `name` is in `preset` already, with the profile's `defaults` like the
/// split list's, or none when it isn't.
pub fn in_preset(preset: &Preset, name: &str, defaults: Option<&SplitDefaults>) -> Option<InPreset> {
    let index = preset.splits.iter().position(|split| split.name == name)?;
    let file = preset.file_names(defaults).swap_remove(index);
    let note = match SplitKind::of(&preset.splits[index]) {
        SplitKind::Split => {
            let number: String = file.chars().take_while(char::is_ascii_digit).collect();
            format!("Already in the preset, as split {number}.")
        }
        _ => format!("Already in the preset, as its {} image.", kind_word(&preset.splits[index])),
    };
    Some(InPreset { file, note })
}

/// Why a ready-made split's `steps` can't be imported into `preset`: it would have more reset or
/// start images than it can.
pub fn import_blocker(preset: &Preset, steps: &[PresetSplit]) -> Option<String> {
    let step = steps.iter().filter(|step| step.is_keyword()).find(|step| {
        let named =
            |splits: &mut dyn Iterator<Item = &PresetSplit>| splits.filter(|split| split.name == step.name).count();
        named(&mut preset.splits.iter()) + named(&mut steps.iter()) > MAX_GROUP_SIZE
    })?;
    let total = (preset.splits.iter().chain(steps)).filter(|split| split.name == step.name).count();
    Some(format!("A preset can have up to {MAX_GROUP_SIZE} {} images, and this would make {total}.", kind_word(step)))
}

/// `split` as it's saved, into the preset or the split library: instructions only go with a
/// savestate, and fade splits have neither, since there's no screen to get to for a screenshot.
/// While editing they're kept, for a split switched to a fade and back.
pub fn saved_split(split: &PresetSplit) -> PresetSplit {
    let mut split = split.clone();
    if split.is_fade() {
        split.savestate.clear();
    }
    if split.savestate.is_empty() {
        split.instructions.clear();
    }
    split
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
    // A fade split without one is the whole screen
    if split.mask.is_empty() && !split.is_fade() {
        problems.push((Field::Mask, "It needs a mask. Choose one.".to_owned()));
    }
    if split.is_fade() && split.fade_color().is_none() {
        problems.push((Field::Color, "Write the color as # and 6 hex digits, like #a0a0a0.".to_owned()));
    }
    // Only a preset written by hand, since the form keeps it in range
    if split.fade_time.is_some_and(|time| !(MIN_FADE_TIME..=MAX_FADE_TIME).contains(&time)) {
        let message = format!("The fade time has to be between {MIN_FADE_TIME} and {MAX_FADE_TIME} ms.");
        problems.push((Field::Color, message));
    }
    problems
}

/// What a reset or start image is called in messages.
fn kind_word(split: &PresetSplit) -> &'static str {
    match SplitKind::of(split) {
        SplitKind::Start => "start",
        _ => "reset",
    }
}

/// A split's name as the list's heading: the reset and start images by what they are, numbered
/// from the second.
pub fn split_heading(preset: &Preset, index: usize) -> String {
    let Some(split) = preset.splits.get(index) else { return String::new() };
    let kind = match SplitKind::of(split) {
        SplitKind::Reset => "Reset Image",
        SplitKind::Start => "Start Image",
        SplitKind::Split => return split.name.clone(),
    };
    match preset.keyword_place(index) {
        Some(place) if place > 1 => format!("{kind} {place}"),
        _ => kind.into(),
    }
}

/// How many characters of a mask's and a savestate's file name fit beside their buttons in the
/// split form, about, since characters differ in width.
pub const MASK_NAME_CHARS: usize = 24;
pub const SAVESTATE_NAME_CHARS: usize = 15;
/// A fade split's mask, beside Choose... and Clear. Fewer than a savestate's, since a mask's name
/// is often wider letters.
pub const FADE_MASK_NAME_CHARS: usize = 13;

/// `name` in at most `max` characters, shortened in the middle with `...`, so the start, often a
/// number, and the end with the extension both show: `006 en...ku.gzs`.
pub fn elide_middle(name: &str, max: usize) -> String {
    let chars: Vec<char> = name.chars().collect();
    if chars.len() <= max {
        return name.to_owned();
    }
    let keep = max.saturating_sub(3);
    // The end gets the odd one, for the extension
    let tail = keep.div_ceil(2);
    let head = keep - tail;
    let start: String = chars[..head].iter().collect();
    let end: String = chars[chars.len() - tail..].iter().collect();
    format!("{start}...{end}")
}

/// A card of the Savestate Instructions window, for a savestate with instructions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstructionText {
    /// The split's place in the preset.
    pub split: usize,
    /// The split's heading, like in the split list.
    pub heading: String,
    /// The name Copy Savestates gives the savestate, which is what's loaded on the console.
    pub file: String,
    pub instructions: String,
}

/// The cards of the Savestate Instructions window, in split order, named like Copy Savestates
/// names the files, so each card goes with the savestate it's about.
pub fn instruction_cards(preset: &Preset) -> Vec<InstructionText> {
    let with_savestate = preset.splits.iter().enumerate().filter(|(_, split)| !split.savestate.is_empty());
    // savestate_copies has one name for each split with a savestate, in the same order
    (with_savestate.zip(preset.savestate_copies()))
        .filter(|((_, split), _)| !split.instructions.is_empty())
        .map(|((index, split), (_, file))| InstructionText {
            split: index,
            heading: split_heading(preset, index),
            file,
            instructions: split.instructions.clone(),
        })
        .collect()
}

/// What leaving a preset with unsaved changes asks, and its button that leaves it: for a preset
/// `saved_before`, its changes, and for a new one, the whole preset, which was never saved.
/// `name` is what the Name field has, which can be empty.
pub fn discard_question(name: &str, saved_before: bool) -> (String, &'static str) {
    let name = name.trim();
    if saved_before {
        let preset = if name.is_empty() { "this preset".to_owned() } else { name.to_owned() };
        return (format!("Discard the changes to {preset}?"), "Discard Changes");
    }
    let preset = if name.is_empty() { "the new preset".to_owned() } else { format!("the new preset {name}") };
    (format!("Discard {preset}? It hasn't been saved yet."), "Discard Preset")
}

/// The line under the editor: the first of `problems` and how many more there are, or else
/// whether the preset has changes to save.
pub fn status_line(preset: &Preset, problems: &[Problem], has_changes: bool) -> String {
    match problem_summary(preset, problems) {
        Some(summary) => summary,
        None if has_changes => "Ready to save.".to_owned(),
        None => "No changes yet.".to_owned(),
    }
}

/// The split clicking the problem line or F8 goes to: the next one after `selected` with a
/// problem, from the top again after the last. `None` without any.
pub fn next_problem_split(problems: &[Problem], selected: usize) -> Option<usize> {
    let with_problems: Vec<usize> = problems.iter().filter_map(|problem| problem.split).collect();
    with_problems.iter().find(|&&index| index > selected).or(with_problems.first()).copied()
}

/// The first of `problems`, with the split it's about, and how many more there are. `None` when
/// there are none.
pub fn problem_summary(preset: &Preset, problems: &[Problem]) -> Option<String> {
    let first = problems.first()?;
    let more = match problems.len() - 1 {
        0 => String::new(),
        1 => " And 1 more problem.".to_owned(),
        more => format!(" And {more} more problems."),
    };
    let at = first.split.map(|index| format!("{}: ", split_heading(preset, index))).unwrap_or_default();
    Some(format!("{at}{}{more}", first.message))
}

/// Every one of `problems` as a list item with the split it's about, a blank line between them so
/// a message that wraps reads as one. Empty when there are none.
pub fn problem_list(preset: &Preset, problems: &[Problem]) -> String {
    (problems.iter())
        .map(|problem| match problem.split {
            Some(index) => format!("- {}: {}", split_heading(preset, index), problem.message),
            None => format!("- {}", problem.message),
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Which of `count` splits have one of `problems`, for their names in the list.
pub fn problem_flags(problems: &[Problem], count: usize) -> Vec<bool> {
    (0..count).map(|index| problems.iter().any(|problem| problem.split == Some(index))).collect()
}

/// The name a preset named `name` is saved with, among the names of its game's other presets,
/// `taken`: its own when that's free, else for a `new` preset the first free ` (2)`, ` (3)` and so
/// on, like a copy. `None` for one saved in place under a taken name, which the Preset list
/// couldn't tell apart. Names are the same ignoring case and extra spaces, like games.
pub fn saved_preset_name(name: &str, taken: &[&str], new: bool) -> Option<String> {
    let is_taken = |name: &str| taken.iter().any(|other| game_key(other) == game_key(name));
    if !is_taken(name) {
        return Some(name.to_owned());
    }
    let base = name.trim();
    new.then(|| (2..).map(|n| format!("{base} ({n})")).find(|name| !is_taken(name)).expect("some number is free"))
}

/// The one of `games` that `game` is, typed in another case or with other spaces, like
/// `the legend of zelda:  ocarina of time`, so presets of a game stay together.
pub fn known_game<'a>(game: &str, games: &[&'a str]) -> Option<&'a str> {
    let wanted = game_key(game);
    games.iter().copied().find(|&known| !known.is_empty() && game_key(known) == wanted)
}
