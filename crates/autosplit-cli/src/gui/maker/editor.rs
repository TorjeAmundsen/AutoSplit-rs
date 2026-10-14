//! Editing a preset in Make Split Images: the split list and the selected split's form edit a
//! draft, which the output preview shows masked on the capture as it changes. Saving writes it
//! with its files into its folder, or a new one.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use autosplit_core::mask::Mask;
use autosplit_core::preset::{Preset, delete_preset, game_key, new_preset_folder, save_preset};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::super::Gui;
use super::super::maker_plan::{DeletedPreset, preset_after_delete};
use super::super::maker_plan::{canonical, file_name_text};
use super::super::messages::{Answer, Message};
use super::super::preset_form::{Draft, Field, ListChange, SplitEdit, SplitKind, field_value};
use super::super::preset_form::{known_game, problem_flags, problem_list, problem_summary};
use super::super::recorder::key_character;
use super::super::ui::{SplitField, SplitForm};
use super::{Kept, game_label, presets_folder};
use autosplit_input::Modifiers;
use slint::winit_030::winit::event::KeyEvent;
use slint::winit_030::winit::keyboard::{Key, NamedKey};

/// A preset being edited, with what the window had before, to go back to on Cancel.
/// A split that was removed, to bring back with Ctrl+Z while nothing else changed.
struct Removal {
    before: Preset,
    after: Preset,
    kept: Kept,
}

pub(super) struct Editing {
    pub(super) draft: Draft,
    /// The folder the preset was read from, which its masks' relative paths are in. None for a
    /// new preset.
    pub(super) source: Option<PathBuf>,
    before: Kept,
    /// Which time editing started, from `Maker::edits`.
    id: u64,
    /// The splits removed, the last one last.
    removed: Vec<Removal>,
    /// How many times splits were added, removed or moved, so a file picked for a split while
    /// they were, where the file dialog doesn't block the window, isn't given to another.
    reorders: u64,
}

/// Whether saving copies a mask or savestate into the preset's folder: it was picked from
/// somewhere else. Paths in the preset are relative, and picked ones absolute.
fn copied_in(source: Option<&Path>, path: &str) -> bool {
    let path = Path::new(path);
    path.is_absolute() && source.is_none_or(|source| !canonical(path).starts_with(canonical(source)))
}

/// A mask or savestate's full path, for its tooltip, or empty without one.
fn shown_path(maker: &super::Maker, path: &str) -> String {
    if path.is_empty() { String::new() } else { maker.file(path).display().to_string() }
}

impl Gui {
    pub(super) fn connect_editor(&self) {
        let weak = self.me.clone();
        let maker = &self.windows.maker;
        on!(weak, maker, on_edit_preset, () gui => gui.start_editing(false));
        on!(weak, maker, on_new_preset, () gui => gui.start_editing(true));
        on!(weak, maker, on_game_edited, (text) gui => gui.edit_preset(|preset| preset.game = text.to_string()));
        on!(weak, maker, on_edit_game_chosen, (index) gui => gui.edit_game_chosen(index as usize));
        on!(weak, maker, on_pick_game, () gui => gui.pick_game());
        on!(weak, maker, on_name_edited, (text) gui => gui.edit_preset(|preset| preset.name = text.to_string()));
        let kinds: Vec<SharedString> = SplitKind::ALL.iter().map(|kind| kind.label().into()).collect();
        maker.set_kinds(ModelRc::new(VecModel::from(kinds)));
        on!(weak, maker, on_split_edited, (field) gui => gui.split_field_edited(field));
        on!(weak, maker, on_choose_mask, () gui => gui.choose_split_file(false));
        on!(weak, maker, on_choose_savestate, () gui => gui.choose_split_file(true));
        on!(weak, maker, on_clear_savestate, () gui => gui.edit_split(SplitEdit::Savestate(String::new())));
        on!(weak, maker, on_add_split, () gui => {
            let selected = gui.selected_split();
            gui.change_splits(|draft| Some(draft.add(selected)));
        });
        on!(weak, maker, on_duplicate_split, () gui => {
            let selected = gui.selected_split();
            gui.change_splits(|draft| draft.duplicate(selected?));
        });
        on!(weak, maker, on_remove_split, () gui => gui.remove_split());
        on!(weak, maker, on_move_split, (up) gui => {
            let selected = gui.selected_split();
            gui.change_splits(|draft| draft.move_split(selected?, up));
        });
        on!(weak, maker, on_save_preset, () gui => gui.save_draft(false));
        on!(weak, maker, on_save_preset_as_new, () gui => gui.save_draft(true));
        on!(weak, maker, on_cancel_edit, () gui => gui.unless_discarded(|gui| gui.cancel_editing()));
        on!(weak, maker, on_show_problems, () gui => gui.show_problems());
        on!(weak, maker, on_delete_preset, () gui => gui.ask_delete_preset());
    }

    /// Every problem of the preset being edited, in a message box.
    fn show_problems(&self) {
        let maker = self.maker.borrow();
        let Some(edit) = &maker.edit else { return };
        let list = problem_list(&edit.draft.preset, &edit.draft.problems());
        drop(maker);
        if !list.is_empty() {
            self.show_message(Message::warning(format!("Fix these before saving:\n\n{list}")));
        }
    }

    /// Escape cancels editing, Ctrl+S saves, Ctrl+Z brings back a removed split and Alt+Up and
    /// Alt+Down move the selected split, like the buttons. Returns whether the key was one of them.
    pub(in crate::gui) fn editor_shortcut(self: &Rc<Self>, event: &KeyEvent, held: Modifiers) -> bool {
        if !self.editing() {
            return false;
        }
        let character = key_character(event).map(|c| c.to_ascii_lowercase());
        match (&event.logical_key, held) {
            (Key::Named(NamedKey::ArrowUp | NamedKey::ArrowDown), Modifiers::ALT) => {
                let up = event.logical_key == Key::Named(NamedKey::ArrowUp);
                let selected = self.selected_split();
                self.change_splits(|draft| draft.move_split(selected?, up));
            }
            _ if event.repeat => return false,
            // An open dropdown list closes first
            (Key::Named(NamedKey::Escape), held) if held.is_empty() && !self.windows.maker.get_list_showing() => {
                self.unless_discarded(|gui| gui.cancel_editing());
            }
            // Otherwise a text field's own undo
            (_, Modifiers::CONTROL) if character == Some('z') => return self.undo_remove(),
            (_, Modifiers::CONTROL) if character == Some('s') => {
                if self.windows.maker.get_can_save_preset() {
                    self.save_draft(false);
                }
            }
            _ => return false,
        }
        true
    }

    pub(super) fn editing(&self) -> bool {
        self.maker.borrow().edit.is_some()
    }

    /// Whether leaving edit mode would lose changes.
    pub(super) fn has_unsaved_preset(&self) -> bool {
        self.maker.borrow().edit.as_ref().is_some_and(|edit| edit.draft.has_changes())
    }

    /// Edits the shown preset, or a new one for its game.
    fn start_editing(&self, new: bool) {
        let mut maker = self.maker.borrow_mut();
        if maker.edit.is_some() || maker.working > 0 {
            return;
        }
        let loaded = maker.loaded().cloned();
        let before = maker.keep();
        let (draft, source) = match (new, loaded) {
            (false, Some(loaded)) => (Draft::edit(loaded.preset), Some(loaded.folder)),
            (false, None) => return,
            (true, loaded) => (Draft::new(&loaded.map(|loaded| loaded.preset.game).unwrap_or_default()), None),
        };
        if new {
            maker.start_over(draft.preset.splits.len());
        }
        let (game, name) = (draft.preset.game.clone(), draft.preset.name.clone());
        maker.edits += 1;
        let id = maker.edits;
        maker.edit = Some(Editing { draft, source, before, id, removed: Vec::new(), reorders: 0 });
        let selected = maker.selected;
        drop(maker);
        self.show_edit_games(&game);
        let window = &self.windows.maker;
        window.set_edit_name(name.into());
        window.set_editing(true);
        window.set_status(SharedString::new());
        self.choose_split(selected);
    }

    /// Lists the games to pick for the preset being edited, `game` picked, or typed as a new game
    /// when no preset has it.
    fn show_edit_games(&self, game: &str) {
        let maker = self.maker.borrow();
        let mut games: Vec<SharedString> = maker.games().into_iter().map(game_label).collect();
        let index = maker.game_index(game);
        drop(maker);
        games.push("New game...".into());
        let window = &self.windows.maker;
        window.set_edit_games(ModelRc::new(VecModel::from(games)));
        window.set_edit_game(game.into());
        window.set_edit_game_index(index.unwrap_or(0) as i32);
        window.set_new_game(index.is_none());
    }

    /// Back from typing a new game to the list: the game typed when it has presets, else the
    /// chosen preset's, else the first. The list still has New game... picked, which is no game.
    fn pick_game(&self) {
        let window = &self.windows.maker;
        let maker = self.maker.borrow();
        let index = (maker.game_index(&window.get_edit_game()))
            .or_else(|| maker.loaded().and_then(|loaded| maker.game_index(&loaded.preset.game)))
            .unwrap_or(0);
        drop(maker);
        window.set_new_game(false);
        window.set_edit_game_index(index as i32);
        self.edit_game_chosen(index);
    }

    /// A game was picked for the preset, or New game..., the last, to type one.
    fn edit_game_chosen(&self, index: usize) {
        let game = self.maker.borrow().games().get(index).map(|&game| game.to_owned());
        let window = &self.windows.maker;
        match game {
            Some(game) => {
                window.set_edit_game(game.as_str().into());
                self.edit_preset(|preset| preset.game = game);
            }
            None => {
                window.set_edit_game(SharedString::new());
                window.set_new_game(true);
                self.edit_preset(|preset| preset.game.clear());
            }
        }
    }

    /// Leaves edit mode, the one way out of it. The splits' grabs and screenshots stay with the
    /// splits as they were edited. Returns what editing had.
    fn leave_editing(&self) -> Option<Editing> {
        let mut maker = self.maker.borrow_mut();
        let edit = maker.edit.take()?;
        maker.live_mask = None;
        drop(maker);
        self.windows.maker.set_editing(false);
        Some(edit)
    }

    /// Cancels editing: the window goes back to how it was before.
    pub(super) fn cancel_editing(&self) {
        let Some(edit) = self.leave_editing() else { return };
        let mut maker = self.maker.borrow_mut();
        maker.restore(edit.before);
        let (preset, selected) = (maker.preset, maker.selected);
        drop(maker);
        self.show_preset(preset);
        self.choose_split(selected);
    }

    /// Does `then` once there are no changes to lose, asking first when there are.
    pub(in crate::gui) fn unless_discarded(self: &Rc<Self>, then: impl FnOnce(&Rc<Self>) + 'static) {
        if !self.has_unsaved_preset() {
            return then(self);
        }
        let name = self.maker.borrow().edit.as_ref().map(|edit| edit.draft.preset.name.clone()).unwrap_or_default();
        let weak = Rc::downgrade(self);
        let labels = ["Discard Changes", "", "Keep Editing"];
        // Enter keeps editing: losing the changes is never the default
        self.show_message(Message::choice(format!("Discard the changes to {name}?"), labels, 2, move |answer| {
            if let (Some(gui), Answer::Yes) = (weak.upgrade(), answer) {
                then(&gui);
            }
        }));
    }

    fn selected_split(&self) -> Option<usize> {
        let maker = self.maker.borrow();
        let count = maker.edit.as_ref().map_or(0, |edit| edit.draft.preset.splits.len());
        (maker.selected < count).then_some(maker.selected)
    }

    fn edit_preset(&self, change: impl FnOnce(&mut Preset)) {
        if let Some(edit) = self.maker.borrow_mut().edit.as_mut() {
            change(&mut edit.draft.preset);
        }
        self.show_maker();
    }

    /// An edit of the selected split's field. The field keeps what's typed, so only what shows
    /// it elsewhere changes.
    fn edit_split(&self, edit: SplitEdit) {
        let Some(index) = self.selected_split() else { return };
        self.edit_split_at(index, edit);
    }

    /// A field of the split form was edited: its value goes into the selected split.
    fn split_field_edited(&self, field: SplitField) {
        let window = &self.windows.maker;
        let edit = match field {
            SplitField::Name => SplitEdit::Name(window.get_split_name().into()),
            SplitField::Kind => match SplitKind::ALL.get(window.get_split_kind() as usize) {
                Some(&kind) => SplitEdit::Kind(kind),
                None => return,
            },
            SplitField::Threshold => {
                SplitEdit::Threshold(window.get_has_threshold().then(|| field_value(window.get_threshold())))
            }
            SplitField::Delay => SplitEdit::Delay(window.get_has_delay().then(|| field_value(window.get_delay()))),
            SplitField::Pause => SplitEdit::Pause(window.get_has_pause().then(|| field_value(window.get_pause()))),
            SplitField::Dummy => SplitEdit::Dummy(window.get_dummy()),
            SplitField::Below => SplitEdit::Below(window.get_below()),
            SplitField::WithPrevious => SplitEdit::WithPrevious(window.get_with_previous()),
            SplitField::Instructions => SplitEdit::Instructions(window.get_instructions().into()),
        };
        self.edit_split(edit);
    }

    /// An edit of split `index`'s field.
    fn edit_split_at(&self, index: usize, edit: SplitEdit) {
        let (refill, mask_changed) = (edit.changes_other_fields(), edit.changes_mask());
        if let Some(editing) = self.maker.borrow_mut().edit.as_mut() {
            editing.draft.edit_split(index, edit);
        }
        if mask_changed {
            self.maker.borrow_mut().live_mask = None;
        }
        if refill {
            self.show_form();
        }
        self.show_maker();
    }

    /// Changes the splits' order and keeps the grabs, screenshots and pictures with their
    /// splits. `change` returns the split to select and the change made.
    fn change_splits(&self, change: impl FnOnce(&mut Draft) -> Option<(usize, ListChange)>) {
        let mut maker = self.maker.borrow_mut();
        let Some(edit) = maker.edit.as_mut() else { return };
        let Some((select, change)) = change(&mut edit.draft) else { return };
        edit.reorders += 1;
        change.mirror(&mut maker.splits);
        drop(maker);
        self.choose_split(select);
    }

    /// Removes the selected split, keeping it and what it had to bring back.
    fn remove_split(&self) {
        let Some(selected) = self.selected_split() else { return };
        let maker = self.maker.borrow();
        let Some(edit) = &maker.edit else { return };
        let (before, kept) = (edit.draft.preset.clone(), maker.keep());
        drop(maker);
        self.change_splits(|draft| draft.remove(selected));
        if let Some(edit) = self.maker.borrow_mut().edit.as_mut() {
            let after = edit.draft.preset.clone();
            edit.removed.push(Removal { before, after, kept });
        }
    }

    /// Brings back the split removed last, with its grab or screenshot, when nothing changed
    /// since. Returns whether it did.
    fn undo_remove(&self) -> bool {
        let mut maker = self.maker.borrow_mut();
        let Some(edit) = maker.edit.as_mut() else { return false };
        if edit.removed.last().is_none_or(|removal| removal.after != edit.draft.preset) {
            // Other changes came after, which bringing it back like this would undo too
            edit.removed.clear();
            return false;
        }
        let Some(removal) = edit.removed.pop() else { return false };
        edit.draft.preset = removal.before;
        edit.reorders += 1;
        maker.restore(removal.kept);
        maker.live_mask = None;
        let selected = maker.selected;
        drop(maker);
        self.windows.maker.set_selected(selected as i32);
        self.show_form();
        self.show_maker();
        true
    }

    /// Puts the selected split's fields in the form, when another split is selected.
    pub(super) fn show_form(&self) {
        let profile = &self.state.borrow().document.profile;
        let defaults = (profile.default_similarity_threshold, profile.default_delay_time, profile.default_pause_time);
        let (threshold_default, delay_default, pause_default) = defaults;
        let maker = self.maker.borrow();
        let Some(edit) = &maker.edit else { return };
        let Some(split) = edit.draft.preset.splits.get(maker.selected).cloned() else { return };
        drop(maker);
        let window = &self.windows.maker;
        window.set_form_generation(window.get_form_generation().wrapping_add(1));
        window.set_split_name(split.name.as_str().into());
        let kind = SplitKind::of(&split);
        window.set_split_kind(SplitKind::ALL.iter().position(|&each| each == kind).unwrap_or_default() as i32);
        // Unticked, they show the profile's defaults, which ticking or editing starts from
        window.set_has_threshold(split.threshold.is_some());
        window.set_threshold(split.threshold.unwrap_or(threshold_default) as f32);
        window.set_has_delay(split.delay.is_some());
        window.set_delay(split.delay.map_or(delay_default as f32, |delay| delay as f32));
        window.set_has_pause(split.pause.is_some());
        window.set_pause(split.pause.unwrap_or(pause_default) as f32);
        window.set_dummy(split.dummy);
        window.set_below(split.below);
        window.set_with_previous(split.with_previous);
        window.set_instructions(split.instructions.as_str().into());
    }

    /// What editing shows besides the fields: the form's problems, and whether the preset can be
    /// saved.
    pub(super) fn show_editing(&self) -> Vec<bool> {
        let grouping = self.state.borrow().document.profile.group_split_images;
        let maker = self.maker.borrow();
        let Some(edit) = &maker.edit else { return Vec::new() };
        let problems = edit.draft.problems();
        let selected = maker.selected;
        let splits = &edit.draft.preset.splits;
        let window = &self.windows.maker;
        window.set_has_split(selected < splits.len());
        if let Some(split) = splits.get(selected) {
            window.set_form(SplitForm {
                name_problem: edit.draft.field_problems(selected, Field::Name).into(),
                mask_problem: edit.draft.field_problems(selected, Field::Mask).into(),
                group_problem: edit.draft.field_problems(selected, Field::Group).into(),
                mask: file_name_text(Path::new(&split.mask)).unwrap_or_default().into(),
                savestate: file_name_text(Path::new(&split.savestate)).unwrap_or_default().into(),
                mask_path: shown_path(&maker, &split.mask).into(),
                savestate_path: shown_path(&maker, &split.savestate).into(),
                mask_copied: copied_in(edit.source.as_deref(), &split.mask),
                savestate_copied: copied_in(edit.source.as_deref(), &split.savestate),
                grouping_off: !grouping
                    && (edit.draft.preset.joins_previous(selected)
                        || edit.draft.preset.keyword_place(selected).is_some_and(|place| place > 1)),
                can_join: edit.draft.why_not_joined(selected).is_none(),
                why_not_joined: edit.draft.why_not_joined(selected).unwrap_or_default().into(),
            });
            // Also when typing the name made it a start or reset image
            let kind = SplitKind::of(split);
            window.set_split_kind(SplitKind::ALL.iter().position(|&each| each == kind).unwrap_or_default() as i32);
        }
        let summary = match problem_summary(&edit.draft.preset, &problems) {
            Some(summary) => summary,
            None if edit.draft.has_changes() => "Ready to save.".to_owned(),
            None => "No changes yet.".to_owned(),
        };
        window.set_problems(summary.into());
        window.set_problem_list(problem_list(&edit.draft.preset, &problems).into());
        // The next split with a problem after the selected one, from the top again after the last
        let with_problems: Vec<usize> = problems.iter().filter_map(|problem| problem.split).collect();
        let next = with_problems.iter().find(|&&index| index > selected).or(with_problems.first());
        window.set_problem_split(next.map_or(-1, |&index| index as i32));
        window.set_has_problems(!problems.is_empty());
        window.set_can_save_preset(problems.is_empty() && edit.draft.can_save());
        window.set_can_delete_preset(edit.source.is_some());
        problem_flags(&problems, splits.len())
    }

    /// Asks for a mask or savestate for the selected split. A mask has to read as one.
    fn choose_split_file(self: &Rc<Self>, savestate: bool) {
        let maker = self.maker.borrow();
        let Some(edit) = &maker.edit else { return };
        // The split the file is for, while this same editing goes on and the splits stay in place
        let (id, reorders, index) = (edit.id, edit.reorders, maker.selected);
        // Without one yet, the preset's own folder
        let current = edit.draft.preset.splits.get(maker.selected).and_then(|split| {
            let path = if savestate { &split.savestate } else { &split.mask };
            (!path.is_empty()).then(|| maker.file(path))
        });
        let folder = current
            .and_then(|path| path.parent().map(Path::to_owned))
            .filter(|folder| folder.is_dir())
            .or_else(|| edit.source.clone())
            .unwrap_or_else(presets_folder);
        let extensions = edit.draft.savestate_extensions();
        drop(maker);
        let title = if savestate { "Select Savestate" } else { "Select Mask" };
        let mut picker = self.file_dialog_over(title, self.windows.maker.window()).set_directory(folder);
        if !savestate {
            picker = picker.add_filter("PNG image", &["png"]);
        } else if !extensions.is_empty() {
            // Any game's practice tool has its own kind, so the ones the preset has come first,
            // and any file can still be picked
            let patterns: Vec<String> = extensions.iter().map(|extension| format!("*.{extension}")).collect();
            picker = picker
                .add_filter(format!("Savestates ({})", patterns.join(", ")), &extensions)
                .add_filter("All files", &["*"]);
        }
        self.when_picked(picker.pick_file(), move |gui, path| {
            let (same, in_place) = match &gui.maker.borrow().edit {
                Some(edit) => (edit.id == id, edit.reorders == reorders),
                None => (false, false),
            };
            if !same {
                return;
            }
            if !in_place {
                let what = if savestate { "savestate" } else { "mask" };
                let text =
                    format!("The splits changed while the {what} was picked, so it wasn't used. Choose it again.");
                return gui.show_message(Message::warning(text));
            }
            if !savestate && let Err(error) = Mask::open(&path) {
                let text = format!("{} can't be a mask: {error}", path.display());
                return gui.show_message(Message::error(text));
            }
            let path = path.display().to_string();
            gui.edit_split_at(index, if savestate { SplitEdit::Savestate(path) } else { SplitEdit::Mask(path) });
        });
    }

    /// Saves the draft into its folder, or a new one, then shows it as saved.
    fn save_draft(self: &Rc<Self>, as_new: bool) {
        let maker = self.maker.borrow();
        let Some(edit) = &maker.edit else { return };
        let problems = edit.draft.problems();
        if !problems.is_empty() {
            let text = format!("Fix these before saving:\n\n{}", problem_list(&edit.draft.preset, &problems));
            drop(maker);
            return self.show_message(Message::error(text));
        }
        let (source, mut preset) = (edit.source.clone(), edit.draft.to_save());
        // A new game typed like one that has presets is that game
        if let Some(game) = known_game(&preset.game, &maker.games()) {
            preset.game = game.to_owned();
        }
        let in_place = source.as_ref().filter(|_| !as_new);
        // The other presets of the game, which the Preset list tells apart by name
        let taken: Vec<String> = (maker.presets.iter())
            .filter(|loaded| in_place.is_none_or(|folder| canonical(&loaded.folder) != canonical(folder)))
            .filter(|loaded| game_key(&loaded.preset.game) == game_key(&preset.game))
            .map(|loaded| loaded.preset.name.trim().to_lowercase())
            .collect();
        drop(maker);
        let is_taken = |name: &str| taken.contains(&name.trim().to_lowercase());
        if is_taken(&preset.name) {
            if in_place.is_some() {
                let text =
                    format!("Another preset for {} is named {}. Give this one another name.", preset.game, preset.name);
                return self.show_message(Message::error(text));
            }
            // A copy, named like one
            let base = preset.name.trim().to_owned();
            preset.name =
                (2..).map(|n| format!("{base} ({n})")).find(|name| !is_taken(name)).expect("some number is free");
        }
        let target = match in_place {
            Some(folder) => folder.clone(),
            None => new_preset_folder(&presets_folder(), &preset.name),
        };
        if let Err(error) = save_preset(source.as_deref(), &target, &preset) {
            return self.show_message(Message::error(format!("Couldn't save the preset. {error}")));
        }
        self.leave_editing();
        self.reload_presets(Some(&target), None);
        self.windows.maker.set_status(format!("Saved the preset {}.", preset.name).into());
    }

    /// Asks to delete the edited preset's folder, then deletes it.
    fn ask_delete_preset(self: &Rc<Self>) {
        let maker = self.maker.borrow();
        let Some(Editing { source: Some(folder), .. }) = &maker.edit else { return };
        let folder = folder.clone();
        let name = maker.loaded().map(|loaded| loaded.preset.name.clone()).unwrap_or_default();
        let game = maker.loaded().map(|loaded| loaded.preset.game.clone()).unwrap_or_default();
        let deleted = DeletedPreset {
            place: maker.presets_of(&game).iter().position(|&index| index == maker.preset).unwrap_or(0),
            game_place: maker.game_index(&game).unwrap_or(0),
            game,
        };
        drop(maker);
        let text = format!(
            "Delete the preset {name}? Its folder is deleted with its masks and savestates, which can't be undone:\n\n{}",
            folder.display()
        );
        let weak = Rc::downgrade(self);
        self.show_message(Message::choice(text, ["Delete Preset", "", "Cancel"], 2, move |answer| {
            let (Some(gui), Answer::Yes) = (weak.upgrade(), answer) else { return };
            if let Err(error) = delete_preset(&folder) {
                return gui.show_message(Message::error(format!("Couldn't delete the preset. {error}")));
            }
            gui.leave_editing();
            gui.reload_presets(None, Some(&deleted));
            gui.windows.maker.set_status(format!("Deleted the preset {name}.").into());
        }));
    }

    /// Reads the presets again after a save or delete, and shows the one in `folder`, keeping
    /// the splits' grabs, or else the one next to a `deleted` one, or else the first.
    fn reload_presets(&self, folder: Option<&Path>, deleted: Option<&DeletedPreset>) {
        let errors = self.read_presets();
        let mut maker = self.maker.borrow_mut();
        let found = folder.and_then(|folder| {
            let folder = canonical(folder);
            maker.presets.iter().position(|loaded| canonical(&loaded.folder) == folder)
        });
        maker.masks.clear();
        maker.live_mask = None;
        let games: Vec<String> = maker.presets.iter().map(|loaded| game_key(&loaded.preset.game)).collect();
        let games: Vec<&str> = games.iter().map(String::as_str).collect();
        let deleted = deleted.map(|deleted| DeletedPreset { game: game_key(&deleted.game), ..deleted.clone() });
        let next = deleted.map_or(0, |deleted| preset_after_delete(&games, &deleted));
        drop(maker);
        match found {
            Some(index) => {
                self.maker.borrow_mut().generation += 1;
                self.show_preset(index);
            }
            None => self.choose_preset(next),
        }
        self.show_preset_errors(errors);
    }
}
