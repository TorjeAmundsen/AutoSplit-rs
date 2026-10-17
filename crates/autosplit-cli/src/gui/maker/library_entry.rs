//! Add to Library and Edit Library Entry, in developer builds: ready-made splits made from the
//! splits selected in the preset being edited, written into the splits library next to the
//! program, and changed or deleted from Split Library.

use autosplit_core::library::{NewStep, add_ready_made, base_image, change_ready_made, delete_ready_made};
use autosplit_core::preset::PresetSplit;

use super::super::library_file_parts;
use super::super::preset_form::{library_file_name, library_heading};
use super::super::ui::EntryStep;
use super::library::splits_folder;
use super::*;

/// What the entry window is for, while it's open.
pub(super) struct Entry {
    /// The ready-made split being edited, by its library's folder and its place there, and its
    /// place among the game's, or none when one is added.
    editing: Option<(PathBuf, usize, usize)>,
}

/// An image of the ready-made split as it would be written, with where its base image is from.
struct Planned {
    step: NewStep,
    source: String,
}

/// The live capture's latest frame as an image, for a base image without a grab or screenshot.
fn frame_image(pixels: &Pixels) -> BgraImage {
    let bgra = pixels.as_bytes().as_chunks::<4>().0.iter().flat_map(|&[r, g, b, _]| [b, g, r, 255]).collect();
    BgraImage::from_raw(pixels.width() as usize, pixels.height() as usize, bgra)
}

/// A base image as a picture.
fn base_picture(image: &BgraImage) -> slint::Image {
    slint::Image::from_rgba8(rgba(base_image(image.view()).view()))
}

impl Gui {
    pub(super) fn connect_library_entry(&self) {
        let weak = self.me.clone();
        let maker = &self.windows.maker;
        maker.set_developer(cfg!(debug_assertions));
        on!(weak, maker, on_add_to_library, () gui => gui.open_entry(None));
        let library = &self.windows.library;
        on!(weak, library, on_edit_entry, (index) gui => gui.open_entry(Some(index as usize)));
        on!(weak, library, on_delete_entry, (index) gui => gui.ask_delete_entry(index as usize));
        let entry = &self.windows.entry;
        on!(weak, entry, on_images_edited, (_images) gui => gui.show_entry());
        on!(weak, entry, on_replace_toggled, (_replace) gui => gui.show_entry());
        on!(weak, entry, on_save, () gui => gui.save_entry());
        on!(weak, entry, on_cancel, () gui => gui.close_entry());
    }

    /// Opens the entry window to add a ready-made split from the selected splits, or to edit the
    /// `index`th of the game's.
    fn open_entry(&self, index: Option<usize>) {
        if !cfg!(debug_assertions) || !self.editing() {
            return;
        }
        let maker = self.maker.borrow();
        let Some(preset) = maker.edit.as_ref().map(editor::Editing::preset) else { return };
        let game = preset.game.clone();
        let (editing, description) = match index {
            Some(index) => {
                let Some((folder, place)) = maker.library.place_of(&game, index) else { return };
                let steps = maker.library.steps_of(&game, index).unwrap_or_default();
                let description = steps.iter().map(|step| step.description.clone()).find(|text| !text.is_empty());
                (Some((folder, place, index)), description.unwrap_or_default())
            }
            None => (None, String::new()),
        };
        drop(maker);
        let window = &self.windows.entry;
        window.set_window_title(if editing.is_some() { "Edit Library Entry" } else { "Add to Library" }.into());
        window.set_editing(editing.is_some());
        window.set_replace(false);
        window.set_images(1.0);
        window.set_description_text(description.into());
        window.set_game(game.trim().into());
        self.maker.borrow_mut().entry = Some(Entry { editing });
        self.show_entry();
        self.show_window(window.window());
    }

    /// Keeps an open entry window on the selected splits.
    pub(super) fn refresh_entry(&self) {
        if self.windows.entry.window().is_visible() {
            self.show_entry();
        }
    }

    /// Closes the entry window without writing anything.
    pub(super) fn close_entry(&self) {
        self.maker.borrow_mut().entry = None;
        let _ = self.windows.entry.hide();
    }

    /// The images the ready-made split is made of: the selected split and as many after it as
    /// the window says, each with its mask and a base image from its grab or screenshot, or the
    /// live capture. Or what's missing.
    fn planned_steps(&self) -> Result<Vec<Planned>, String> {
        let images = self.windows.entry.get_images().max(1.0) as usize;
        let maker = self.maker.borrow();
        let Some(preset) = maker.edit.as_ref().map(editor::Editing::preset) else {
            return Err("Edit a preset first.".to_owned());
        };
        let selected = maker.selected;
        if selected >= preset.splits.len() {
            return Err("Select the split to make it from in Make Split Images.".to_owned());
        }
        let mut planned = Vec::new();
        for (index, split) in preset.splits.iter().enumerate().skip(selected).take(images) {
            if split.mask.is_empty() {
                return Err(format!("{} needs a mask first.", library_heading(split)));
            }
            let source = maker.splits.get(index).and_then(|work| work.source.clone());
            let (screenshot, from) = match (source, &maker.last_live) {
                (Some(source), _) => {
                    let from = match &source {
                        Source::File(path) => {
                            format!("Base image from its screenshot {}", file_name_text(path).unwrap_or_default())
                        }
                        Source::Grabbed(_) => "Base image from its grab".to_owned(),
                    };
                    (source.image().map_err(|error| error.to_string())?.as_ref().clone(), from)
                }
                (None, Some(frame)) => (frame_image(frame), "Base image from the live capture".to_owned()),
                (None, None) => {
                    return Err(format!(
                        "{} has no grab or screenshot, and there's no live capture, to make its base image from.",
                        library_heading(split)
                    ));
                }
            };
            let savestate = (!split.savestate.is_empty()).then(|| maker.file(&split.savestate));
            let step = NewStep { split: split.clone(), mask: maker.file(&split.mask), savestate, screenshot };
            planned.push(Planned { step, source: from });
        }
        Ok(planned)
    }

    /// Puts the ready-made split as it would be written in the entry window, and what's in the way.
    fn show_entry(&self) {
        let defaults = self.state.borrow().document.profile.split_defaults();
        let window = &self.windows.entry;
        let maker = self.maker.borrow();
        let Some(entry) = &maker.entry else { return };
        let Some(preset) = maker.edit.as_ref().map(editor::Editing::preset) else { return };
        let game = preset.game.clone();
        let most = preset.splits.len().saturating_sub(maker.selected).clamp(1, 8);
        let kept = entry.editing.as_ref().filter(|_| !window.get_replace()).map(|(_, _, index)| *index);
        let step_row = |split: &PresetSplit, thumb: slint::Image, source: String| {
            let file = library_file_name(split, Some(&defaults));
            EntryStep {
                heading: library_heading(split).into(),
                thumb,
                file_parts: library_file_parts(&file),
                file: file.into(),
                source: source.into(),
            }
        };
        // Kept as they are, only the description changes
        let kept_rows: Option<Vec<EntryStep>> = kept.map(|index| {
            let folder = entry.editing.as_ref().map(|(folder, _, _)| folder.clone()).unwrap_or_default();
            (maker.library.steps_of(&game, index).unwrap_or_default().iter())
                .map(|step| {
                    let thumb = maker.library.thumb(&folder, &step.base_image).unwrap_or_default();
                    step_row(&step.split, thumb, "Kept as it is".to_owned())
                })
                .collect()
        });
        drop(maker);
        let (rows, mut problem) = match kept_rows {
            Some(rows) => (rows, String::new()),
            None => match self.planned_steps() {
                Ok(planned) => {
                    let rows = (planned.into_iter())
                        .map(|planned| {
                            step_row(&planned.step.split, base_picture(&planned.step.screenshot), planned.source)
                        })
                        .collect();
                    (rows, String::new())
                }
                Err(problem) => (Vec::new(), problem),
            },
        };
        if game.trim().is_empty() {
            problem = "Pick the preset's game first: the split library is by game.".to_owned();
        }
        window.set_most_images(most as f32);
        window.set_steps(ModelRc::new(VecModel::from(rows)));
        window.set_problem(problem.into());
    }

    /// Writes the ready-made split into the library, then shows the library as it is now.
    fn save_entry(&self) {
        let maker = self.maker.borrow();
        let Some(entry) = &maker.entry else { return };
        let editing = entry.editing.clone();
        let game = maker.edit.as_ref().map(|edit| edit.preset().game.trim().to_owned()).unwrap_or_default();
        drop(maker);
        let window = &self.windows.entry;
        let description = window.get_description_text().trim().to_owned();
        let steps = match (&editing, window.get_replace()) {
            (Some(_), false) => None,
            _ => match self.planned_steps() {
                Ok(planned) => Some(planned.into_iter().map(|planned| planned.step).collect()),
                Err(problem) => return self.show_message(Message::error(problem)),
            },
        };
        let written = match (editing, steps) {
            (Some((folder, place, _)), steps) => change_ready_made(&folder, place, &description, steps),
            (None, Some(steps)) => add_ready_made(&splits_folder(), &game, &description, steps).map(|_| ()),
            (None, None) => return,
        };
        if let Err(error) = written {
            return self.show_message(Message::error(format!("Couldn't write the split library. {error}")));
        }
        self.close_entry();
        self.read_library();
        self.refresh_library();
    }

    /// Asks to delete the `index`th of the game's ready-made splits, then deletes it with the
    /// files only it used.
    fn ask_delete_entry(self: &Rc<Self>, index: usize) {
        if !cfg!(debug_assertions) {
            return;
        }
        let maker = self.maker.borrow();
        let Some(preset) = maker.edit.as_ref().map(editor::Editing::preset) else { return };
        let Some((folder, place)) = maker.library.place_of(&preset.game, index) else { return };
        let steps = maker.library.steps_of(&preset.game, index).unwrap_or_default();
        let name = steps.last().map(|step| library_heading(&step.split)).unwrap_or_default();
        drop(maker);
        let text = format!(
            "Delete {name} from the split library? Its masks and base images are deleted too, unless another ready-made split uses them."
        );
        let weak = Rc::downgrade(self);
        self.show_message(Message::choice(text, ["Delete", "", "Cancel"], 2, move |answer| {
            let (Some(gui), Answer::Yes) = (weak.upgrade(), answer) else { return };
            if let Err(error) = delete_ready_made(&folder, place) {
                return gui.show_message(Message::error(format!("Couldn't write the split library. {error}")));
            }
            gui.read_library();
            gui.refresh_library();
        }));
    }
}
