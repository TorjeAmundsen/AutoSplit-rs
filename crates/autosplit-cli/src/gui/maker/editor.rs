//! Editing a preset in Make Split Images: the split list and the selected split's form edit a
//! draft, which the output preview shows masked on the capture as it changes. Saving writes it
//! with its files into its folder, or a new one.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use autosplit_core::mask::Mask;
use autosplit_core::preset::{Preset, delete_preset, new_preset_folder, save_preset};
use slint::{ComponentHandle, SharedString};

use super::super::messages::{Answer, Message};
use super::super::preset_form::{Draft, ListChange, Problem, SplitEdit, field_value};
use super::super::ui::SplitForm;
use super::super::{Gui, file_name_parts};
use super::{Kept, canonical, presets_folder, problem_summary};

/// A preset being edited, with what the window had before, to go back to on Cancel.
pub(super) struct Editing {
    pub(super) draft: Draft,
    /// The folder the preset was read from, which its masks' relative paths are in. None for a
    /// new preset.
    pub(super) source: Option<PathBuf>,
    before: Kept,
    /// Which time editing started, from `Maker::edits`.
    id: u64,
}

/// What `name` shows of a mask or savestate path: its file name.
fn file_label(path: &str) -> String {
    Path::new(path).file_name().map_or_else(String::new, |name| name.to_string_lossy().into_owned())
}

impl Gui {
    pub(super) fn connect_editor(&self) {
        let weak = self.me.clone();
        let maker = &self.windows.maker;
        on!(weak, maker, on_edit_preset, () gui => gui.start_editing(false));
        on!(weak, maker, on_new_preset, () gui => gui.start_editing(true));
        on!(weak, maker, on_game_edited, (text) gui => gui.edit_preset(|preset| preset.game = text.to_string()));
        on!(weak, maker, on_name_edited, (text) gui => gui.edit_preset(|preset| preset.name = text.to_string()));
        on!(weak, maker, on_split_name_edited, (text) gui => gui.edit_split(SplitEdit::Name(text.into())));
        on!(weak, maker, on_threshold_edited, (value) gui => gui.edit_split(SplitEdit::Threshold(field_value(value))));
        on!(weak, maker, on_delay_edited, (value) gui => gui.edit_split(SplitEdit::Delay(value.max(0.0).round() as u32)));
        on!(weak, maker, on_pause_edited, (on, value) gui => {
            gui.edit_split(SplitEdit::Pause(on.then_some(field_value(value))));
        });
        on!(weak, maker, on_dummy_edited, (on) gui => gui.edit_split(SplitEdit::Dummy(on)));
        on!(weak, maker, on_below_edited, (on) gui => gui.edit_split(SplitEdit::Below(on)));
        on!(weak, maker, on_with_previous_edited, (on) gui => gui.edit_split(SplitEdit::WithPrevious(on)));
        on!(weak, maker, on_instructions_edited, (text) gui => gui.edit_split(SplitEdit::Instructions(text.into())));
        on!(weak, maker, on_choose_mask, () gui => gui.choose_split_file(false));
        on!(weak, maker, on_choose_savestate, () gui => gui.choose_split_file(true));
        on!(weak, maker, on_clear_savestate, () gui => {
            gui.edit_split(SplitEdit::Savestate(String::new()));
            // Its instructions went with it
            gui.show_form();
        });
        on!(weak, maker, on_add_split, () gui => {
            let selected = gui.selected_split();
            gui.change_splits(|draft| Some(draft.add(selected)));
        });
        on!(weak, maker, on_duplicate_split, () gui => {
            let selected = gui.selected_split();
            gui.change_splits(|draft| draft.duplicate(selected?));
        });
        on!(weak, maker, on_remove_split, () gui => {
            let selected = gui.selected_split();
            gui.change_splits(|draft| {
                let (next, change) = draft.remove(selected?)?;
                Some((next.unwrap_or(0), change))
            });
        });
        on!(weak, maker, on_move_split, (up) gui => {
            let selected = gui.selected_split();
            gui.change_splits(|draft| draft.move_split(selected?, up));
        });
        on!(weak, maker, on_save_preset, () gui => gui.save_draft(false));
        on!(weak, maker, on_save_preset_as_new, () gui => gui.save_draft(true));
        on!(weak, maker, on_cancel_edit, () gui => gui.unless_discarded(|gui| gui.stop_editing(true)));
        on!(weak, maker, on_delete_preset, () gui => gui.ask_delete_preset());
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
            let count = draft.preset.splits.len();
            (maker.sources, maker.saved, maker.thumbs, maker.selected) =
                (vec![None; count], vec![None; count], vec![None; count], 0);
        }
        let (game, name) = (draft.preset.game.clone(), draft.preset.name.clone());
        maker.edits += 1;
        let id = maker.edits;
        maker.edit = Some(Editing { draft, source, before, id });
        let selected = maker.selected;
        drop(maker);
        let window = &self.windows.maker;
        window.set_edit_game(game.into());
        window.set_edit_name(name.into());
        window.set_editing(true);
        window.set_status(SharedString::new());
        self.choose_split(selected);
    }

    /// Leaves edit mode, back to how the window was before when `restore`. Otherwise the splits'
    /// grabs and screenshots stay with the splits as they were edited.
    pub(super) fn stop_editing(&self, restore: bool) {
        let mut maker = self.maker.borrow_mut();
        let Some(edit) = maker.edit.take() else { return };
        if restore {
            maker.restore(edit.before);
        }
        maker.live_mask = None;
        let (preset, selected) = (maker.preset, maker.selected);
        drop(maker);
        self.windows.maker.set_editing(false);
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

    /// An edit of split `index`'s field.
    fn edit_split_at(&self, index: usize, edit: SplitEdit) {
        let mask_changed = matches!(edit, SplitEdit::Mask(_));
        if let Some(editing) = self.maker.borrow_mut().edit.as_mut() {
            editing.draft.edit_split(index, edit);
        }
        if mask_changed {
            self.maker.borrow_mut().live_mask = None;
        }
        self.show_maker();
    }

    /// Changes the splits' order and keeps the grabs, screenshots and pictures with their
    /// splits. `change` returns the split to select and the change made.
    fn change_splits(&self, change: impl FnOnce(&mut Draft) -> Option<(usize, ListChange)>) {
        let mut maker = self.maker.borrow_mut();
        let Some(edit) = maker.edit.as_mut() else { return };
        let Some((select, change)) = change(&mut edit.draft) else { return };
        change.mirror(&mut maker.sources);
        change.mirror(&mut maker.saved);
        change.mirror(&mut maker.thumbs);
        drop(maker);
        self.choose_split(select);
    }

    /// Puts the selected split's fields in the form, when another split is selected.
    pub(super) fn show_form(&self) {
        let pause_default = self.state.borrow().document.profile.default_pause_time;
        let maker = self.maker.borrow();
        let Some(edit) = &maker.edit else { return };
        let Some(split) = edit.draft.preset.splits.get(maker.selected).cloned() else { return };
        drop(maker);
        let window = &self.windows.maker;
        window.set_form_generation(window.get_form_generation().wrapping_add(1));
        window.set_split_name(split.name.as_str().into());
        window.set_threshold(split.threshold() as f32);
        window.set_delay(split.delay as f32);
        window.set_has_pause(split.pause.is_some());
        // Turning the pause on starts from the profile's default
        window.set_pause(split.pause.unwrap_or(pause_default) as f32);
        window.set_dummy(split.dummy);
        window.set_below(split.below);
        window.set_with_previous(split.with_previous);
        window.set_instructions(split.instructions.as_str().into());
    }

    /// What editing shows besides the fields: the form's file name and problem, and whether the
    /// preset can be saved. `names` are the splits' file names.
    pub(super) fn show_editing(&self, names: &[String]) -> Vec<bool> {
        let maker = self.maker.borrow();
        let Some(edit) = &maker.edit else { return Vec::new() };
        let problems = edit.draft.problems();
        let selected = maker.selected;
        let splits = &edit.draft.preset.splits;
        let window = &self.windows.maker;
        window.set_has_split(selected < splits.len());
        if let Some(split) = splits.get(selected) {
            let file = names.get(selected).cloned().unwrap_or_default();
            let problem = problems.iter().find(|problem| problem.split == Some(selected));
            window.set_form(SplitForm {
                parts: file_name_parts(&file),
                file: file.into(),
                problem: problem.map(|problem| problem.message.as_str()).unwrap_or_default().into(),
                mask: file_label(&split.mask).into(),
                savestate: file_label(&split.savestate).into(),
                can_join: edit.draft.preset.can_join_previous(selected),
                name_error: edit.draft.name_has_problem(selected),
            });
        }
        let summary = match problem_summary(&edit.draft.preset, &problems) {
            Some(summary) => summary,
            None if edit.draft.has_changes() => "Ready to save.".to_owned(),
            None => "No changes yet.".to_owned(),
        };
        window.set_problems(summary.into());
        window.set_has_problems(!problems.is_empty());
        window.set_can_save_preset(problems.is_empty() && edit.draft.has_changes());
        window.set_can_delete_preset(edit.source.is_some());
        (0..splits.len()).map(|index| problems.iter().any(|problem: &Problem| problem.split == Some(index))).collect()
    }

    /// Asks for a mask or savestate for the selected split. A mask has to read as one.
    fn choose_split_file(self: &Rc<Self>, savestate: bool) {
        let maker = self.maker.borrow();
        let Some(edit) = &maker.edit else { return };
        // The split the file is for, while this same editing goes on
        let (id, index) = (edit.id, maker.selected);
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
        drop(maker);
        let (title, dialog) =
            if savestate { ("Select Savestate", None) } else { ("Select Mask", Some(("PNG image", ["png"]))) };
        let mut picker = self.file_dialog_over(title, self.windows.maker.window()).set_directory(folder);
        if let Some((name, extensions)) = dialog {
            picker = picker.add_filter(name, &extensions);
        }
        self.when_picked(picker.pick_file(), move |gui, path| {
            let same = gui.maker.borrow().edit.as_ref().is_some_and(|edit| edit.id == id);
            if !same {
                return;
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
            let list: Vec<&str> = problems.iter().map(|problem| problem.message.as_str()).collect();
            let text = format!("Fix these before saving:\n\n{}", list.join("\n"));
            drop(maker);
            return self.show_message(Message::error(text));
        }
        let (source, preset) = (edit.source.clone(), edit.draft.preset.clone());
        drop(maker);
        let target = match &source {
            Some(folder) if !as_new => folder.clone(),
            _ => new_preset_folder(&presets_folder(), &preset.name),
        };
        if let Err(error) = save_preset(source.as_deref(), &target, &preset) {
            return self.show_message(Message::error(format!("Couldn't save the preset. {error}")));
        }
        self.maker.borrow_mut().edit = None;
        self.windows.maker.set_editing(false);
        self.reload_presets(Some(&target));
        self.windows.maker.set_status(format!("Saved the preset {}.", preset.name).into());
    }

    /// Asks to delete the edited preset's folder, then deletes it.
    fn ask_delete_preset(self: &Rc<Self>) {
        let maker = self.maker.borrow();
        let Some(Editing { source: Some(folder), .. }) = &maker.edit else { return };
        let folder = folder.clone();
        let name = maker.loaded().map(|loaded| loaded.preset.name.clone()).unwrap_or_default();
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
            gui.maker.borrow_mut().edit = None;
            gui.windows.maker.set_editing(false);
            gui.reload_presets(None);
            gui.windows.maker.set_status(format!("Deleted the preset {name}.").into());
        }));
    }

    /// Reads the presets again after a save or delete, and shows the one in `folder`, keeping
    /// the splits' grabs, or else the first.
    fn reload_presets(&self, folder: Option<&Path>) {
        let errors = self.read_presets();
        let mut maker = self.maker.borrow_mut();
        let found = folder.and_then(|folder| {
            let folder = canonical(folder);
            maker.presets.iter().position(|loaded| canonical(&loaded.folder) == folder)
        });
        maker.masks.clear();
        maker.live_mask = None;
        drop(maker);
        match found {
            Some(index) => {
                self.maker.borrow_mut().generation += 1;
                self.show_preset(index);
            }
            None => self.choose_preset(0),
        }
        self.show_preset_errors(errors);
    }
}
