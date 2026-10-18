//! Add to Library and Edit Library Entry, in developer builds: ready-made splits made from the
//! splits selected in the preset being edited, written into the splits library next to the
//! program, and changed or deleted from Split Library.

use autosplit_core::filename::validate_split_name;
use autosplit_core::library::{NewStep, add_ready_made, base_image, change_ready_made, delete_ready_made};
use autosplit_core::preset::{LibrarySplit, PresetSplit};
use slint::Model;

use super::super::name_parts_model;
use super::super::preset_form::{SplitKind, library_file_name, library_file_parts, library_heading, saved_split};
use super::super::ui::EntryStep;
use super::library::splits_folder;
use super::*;

/// What the entry window is for, while it's open.
pub(super) struct Entry {
    /// The ready-made split being edited, or none when one is added.
    editing: Option<Edited>,
    /// The live capture as the window opened, for base images of splits without a grab or
    /// screenshot, so the one saved is the one shown.
    live: Option<BgraImage>,
    /// Pictures chosen for the base images, by the image's place in the ready-made split, with
    /// their files. They come before anything else.
    pictures: HashMap<usize, (PathBuf, BgraImage)>,
    /// Names typed for the images, by their place in the ready-made split, as typed.
    names: HashMap<usize, String>,
}

impl Entry {
    /// `split`, the image at `place`, with the name typed for it.
    fn named(&self, place: usize, split: &PresetSplit) -> PresetSplit {
        match self.names.get(&place) {
            Some(name) => PresetSplit { name: name.trim().to_owned(), ..split.clone() },
            None => split.clone(),
        }
    }
}

/// What's wrong with `named`, the image `own` with the name typed for it, if anything.
fn name_problem(own: &PresetSplit, named: &PresetSplit) -> Option<String> {
    if SplitKind::of(named) != SplitKind::of(own) {
        return Some(format!("{} can't be named {}.", library_heading(own), named.name));
    }
    let error = validate_split_name(&named.name).err()?;
    Some(format!("{}: {error}", library_heading(own)))
}

/// A ready-made split being edited, as it was when the window opened: the library can change
/// meanwhile, which saving finds out instead of changing another one.
struct Edited {
    /// Its library's folder, and its place among that library's.
    folder: PathBuf,
    place: usize,
    steps: Vec<LibrarySplit>,
}

/// An image of the ready-made split as it would be written, with where its base image is from.
struct Planned {
    /// With the name typed for it.
    split: PresetSplit,
    /// As the preset has it.
    own: PresetSplit,
    /// None for a fade split of the whole screen.
    mask: Option<PathBuf>,
    savestate: Option<PathBuf>,
    /// None while there's nothing to make its base image from, which a picture can be chosen for.
    screenshot: Option<BgraImage>,
    source: String,
}

impl Planned {
    /// What's written, once it has something to make its base image from.
    fn step(self) -> Option<NewStep> {
        let Self { split, mask, savestate, screenshot, .. } = self;
        Some(NewStep { split, mask, savestate, screenshot: screenshot? })
    }
}

/// What's wrong with a name typed for one of `planned`, or the first without a base image, if
/// anything.
fn planned_problem(planned: &[Planned]) -> Option<String> {
    (planned.iter().find_map(|planned| name_problem(&planned.own, &planned.split))).or_else(|| missing_picture(planned))
}

/// What to do about the first of `planned` without a base image, if any.
fn missing_picture(planned: &[Planned]) -> Option<String> {
    let split = &planned.iter().find(|planned| planned.screenshot.is_none())?.split;
    Some(format!(
        "{} needs a picture for its base image. Choose one with Picture..., or give the split a grab or screenshot.",
        library_heading(split)
    ))
}

/// The live capture's latest frame as an image, for a base image without a grab or screenshot.
fn frame_image(pixels: &Pixels) -> BgraImage {
    let bgra = pixels.as_bytes().as_chunks::<4>().0.iter().flat_map(|&[r, g, b, _]| [b, g, r, 255]).collect();
    BgraImage::from_raw(pixels.width() as usize, pixels.height() as usize, bgra)
}

/// Where a base image chosen as a picture is from.
fn picked_from(path: &Path) -> String {
    format!("Base image from {}", file_name_text(path).unwrap_or_default())
}

/// A base image as a picture.
fn base_picture(image: &BgraImage) -> slint::Image {
    slint::Image::from_rgba8(rgba(base_image(image.view()).view()))
}

impl Gui {
    pub(super) fn connect_library_entry(&self) {
        let weak = self.me.clone();
        let maker = &self.windows.maker;
        maker.set_developer(DEVELOPER);
        on!(weak, maker, on_add_to_library, () gui => gui.open_entry(None));
        let library = &self.windows.library;
        on!(weak, library, on_edit_entry, (index) gui => gui.open_entry(Some(index as usize)));
        on!(weak, library, on_delete_entry, (index) gui => gui.ask_delete_entry(index as usize));
        let entry = &self.windows.entry;
        on!(weak, entry, on_images_edited, (_images) gui => gui.show_entry());
        on!(weak, entry, on_name_edited, (index, name) gui => {
            if let Some(entry) = gui.maker.borrow_mut().entry.as_mut() {
                entry.names.insert(index as usize, name.into());
            }
            gui.show_entry();
        });
        on!(weak, entry, on_pick_picture, (index) gui => gui.pick_picture(index as usize));
        on!(weak, entry, on_clear_picture, (index) gui => {
            if let Some(entry) = gui.maker.borrow_mut().entry.as_mut() {
                entry.pictures.remove(&(index as usize));
            }
            gui.show_entry();
        });
        on!(weak, entry, on_replace_toggled, (_replace) gui => gui.show_entry());
        on!(weak, entry, on_save, () gui => gui.save_entry());
        on!(weak, entry, on_cancel, () gui => gui.close_entry());
    }

    /// Opens the entry window to add a ready-made split from the selected splits, or to edit the
    /// `index`th of the game's.
    fn open_entry(&self, index: Option<usize>) {
        if !DEVELOPER || !self.editing() {
            return;
        }
        let maker = self.maker.borrow();
        let Some(preset) = maker.edit.as_ref().map(editor::Editing::preset) else { return };
        let game = preset.game.clone();
        let live = maker.last_live.as_ref().map(frame_image);
        let (editing, description) = match index {
            Some(index) => {
                let Some((folder, place)) = maker.library.place_of(&game, index) else { return };
                let Some(steps) = maker.library.steps_of(&game, index) else { return };
                let description = steps.iter().map(|step| step.description.clone()).find(|text| !text.is_empty());
                (Some(Edited { folder, place, steps }), description.unwrap_or_default())
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
        self.maker.borrow_mut().entry = Some(Entry { editing, live, pictures: HashMap::new(), names: HashMap::new() });
        // Made anew, so the name fields show the names of these images
        window.set_steps(ModelRc::default());
        self.show_entry();
        super::super::fit_window(window.window(), window.get_fitted_width(), window.get_fitted_height());
        self.show_window(window.window());
    }

    /// Asks for a picture for the base image of the image at `index`. It has to read as a PNG.
    fn pick_picture(self: &Rc<Self>, index: usize) {
        let folder = self.maker.borrow().edit.as_ref().and_then(|edit| edit.source().map(Path::to_owned));
        let mut picker = self.file_dialog_over("Select Base Image", self.windows.entry.window());
        picker = picker.add_filter("PNG image", &["png"]);
        if let Some(folder) = folder {
            picker = picker.set_directory(folder);
        }
        self.when_picked(picker.pick_file(), move |gui, path| {
            let image = match BgraImage::open_png(&path) {
                Ok(image) => image,
                Err(error) => {
                    return gui.show_message(Message::error(format!("Couldn't read {}: {error}", path.display())));
                }
            };
            if let Some(entry) = gui.maker.borrow_mut().entry.as_mut() {
                entry.pictures.insert(index, (path, image));
            }
            gui.show_entry();
        });
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
    /// the window says, each with its mask and a base image from a chosen picture, its grab or
    /// screenshot, or the live capture as the window opened, or none yet. Or what's missing that
    /// a picture can't fix, like a mask.
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
        let pictures = maker.entry.as_ref().map(|entry| &entry.pictures);
        let mut planned = Vec::new();
        for (place, (index, split)) in preset.splits.iter().enumerate().skip(selected).take(images).enumerate() {
            // As it's saved: a fade split's savestate, hidden in the form, isn't its own
            let split = &saved_split(split);
            if split.mask.is_empty() && !split.is_fade() {
                return Err(format!("{} needs a mask first.", library_heading(split)));
            }
            // Missing files are said here, before anything is written
            let file = |path: &str| (!path.is_empty()).then(|| maker.file(path));
            let (mask, savestate) = (file(&split.mask), file(&split.savestate));
            for path in mask.iter().chain(&savestate) {
                if !path.is_file() {
                    return Err(format!("{} uses {}, which doesn't exist.", library_heading(split), path.display()));
                }
            }
            let picture = pictures.and_then(|pictures| pictures.get(&place));
            let source = maker.source(index);
            let live = maker.entry.as_ref().and_then(|entry| entry.live.as_ref());
            let (screenshot, from) = match (picture, source, live) {
                (Some((path, image)), _, _) => (Some(image.clone()), picked_from(path)),
                (None, Some(source), _) => {
                    let from = match &source {
                        Source::File(path) => {
                            format!("Base image from its screenshot {}", file_name_text(path).unwrap_or_default())
                        }
                        Source::Grabbed(_) => "Base image from its grab".to_owned(),
                        Source::Fade(_) => "Base image from its color".to_owned(),
                    };
                    (Some(source.image().map_err(|error| error.to_string())?.as_ref().clone()), from)
                }
                (None, None, Some(frame)) => (Some(frame.clone()), "Base image from the live capture".to_owned()),
                // Shown without one, so a picture can be chosen for it
                (None, None, None) => (None, "No picture for its base image yet".to_owned()),
            };
            let named = maker.entry.as_ref().map_or_else(|| split.clone(), |entry| entry.named(place, split));
            planned.push(Planned { split: named, own: split.clone(), mask, savestate, screenshot, source: from });
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
        let kept = entry.editing.as_ref().filter(|_| !window.get_replace());
        let picked: Vec<usize> = entry.pictures.keys().copied().collect();
        let names = entry.names.clone();
        // `own` is the image as it is, and `split` with the name typed for it
        let step_row = |place: usize, own: &PresetSplit, split: &PresetSplit, thumb: slint::Image, source: String| {
            let file = library_file_name(split, Some(&defaults));
            EntryStep {
                heading: library_heading(own).into(),
                name: names.get(&place).cloned().unwrap_or_else(|| split.name.clone()).into(),
                renamable: SplitKind::of(own) == SplitKind::Split,
                thumb,
                file_parts: name_parts_model(library_file_parts(split, Some(&defaults))),
                file: file.into(),
                source: source.into(),
                picked: picked.contains(&place),
            }
        };
        // Kept as they are, only the description and names change, and a base image a picture is
        // chosen for
        let kept_rows: Option<(Vec<EntryStep>, Option<String>)> = kept.map(|edited| {
            let mut problem = None;
            let rows = (edited.steps.iter().enumerate())
                .map(|(place, step)| {
                    let split = entry.named(place, &step.split);
                    problem = problem.take().or_else(|| name_problem(&step.split, &split));
                    match entry.pictures.get(&place) {
                        Some((path, image)) => {
                            step_row(place, &step.split, &split, base_picture(image), picked_from(path))
                        }
                        None => {
                            let thumb = maker.library.thumb(&edited.folder, &step.base_image).unwrap_or_default();
                            step_row(place, &step.split, &split, thumb, "Kept as it is".to_owned())
                        }
                    }
                })
                .collect();
            (rows, problem)
        });
        drop(maker);
        let (rows, mut problem) = match kept_rows {
            Some((rows, problem)) => (rows, problem.unwrap_or_default()),
            None => match self.planned_steps() {
                Ok(planned) => {
                    let problem = planned_problem(&planned).unwrap_or_default();
                    let rows = (planned.into_iter().enumerate())
                        .map(|(place, planned)| {
                            let thumb = planned.screenshot.as_ref().map(base_picture).unwrap_or_default();
                            step_row(place, &planned.own, &planned.split, thumb, planned.source)
                        })
                        .collect();
                    (rows, problem)
                }
                Err(problem) => (Vec::new(), problem),
            },
        };
        if game.trim().is_empty() {
            problem = "Pick the preset's game first: the split library is by game.".to_owned();
        }
        window.set_most_images(most as f32);
        // Changed in place while it has as many images, so a name field keeps its focus
        let steps = window.get_steps();
        match steps.as_any().downcast_ref::<VecModel<EntryStep>>().filter(|steps| steps.row_count() == rows.len()) {
            Some(steps) => {
                for (row, step) in rows.into_iter().enumerate() {
                    steps.set_row_data(row, step);
                }
            }
            None => window.set_steps(ModelRc::new(VecModel::from(rows))),
        }
        window.set_problem(problem.into());
    }

    /// Writes the ready-made split into the library, then shows the library as it is now.
    fn save_entry(&self) {
        let maker = self.maker.borrow();
        let Some(entry) = &maker.entry else { return };
        let editing = entry.editing.as_ref().map(|edited| (edited.folder.clone(), edited.place, edited.steps.clone()));
        let game = maker.edit.as_ref().map(|edit| edit.preset().game.trim().to_owned()).unwrap_or_default();
        drop(maker);
        let in_use = self.files_in_use();
        let window = &self.windows.entry;
        let description = window.get_description_text().trim().to_owned();
        let steps = match (&editing, window.get_replace()) {
            (Some((folder, _, kept)), false) => match self.kept_steps(folder, kept) {
                Ok(steps) => steps,
                Err(problem) => return self.show_message(Message::error(problem)),
            },
            _ => match self.planned_steps() {
                Ok(planned) => match planned_problem(&planned) {
                    Some(problem) => return self.show_message(Message::error(problem)),
                    None => Some(planned.into_iter().filter_map(Planned::step).collect()),
                },
                Err(problem) => return self.show_message(Message::error(problem)),
            },
        };
        let written = match (editing, steps) {
            (Some((folder, place, expected)), steps) => {
                change_ready_made(&folder, place, &expected, &description, steps, &in_use)
            }
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

    /// The images of a ready-made split being edited, kept as they are but for base images a
    /// picture is chosen for, or none when nothing but the description changes.
    fn kept_steps(&self, folder: &Path, kept: &[LibrarySplit]) -> Result<Option<Vec<NewStep>>, String> {
        let maker = self.maker.borrow();
        let Some(entry) = &maker.entry else { return Ok(None) };
        let named: Vec<PresetSplit> =
            kept.iter().enumerate().map(|(place, step)| entry.named(place, &step.split)).collect();
        if let Some(problem) = kept.iter().zip(&named).find_map(|(step, named)| name_problem(&step.split, named)) {
            return Err(problem);
        }
        let renamed = kept.iter().zip(&named).any(|(step, named)| step.split.name != named.name);
        if entry.pictures.is_empty() && !renamed {
            return Ok(None);
        }
        let mut steps = Vec::new();
        for ((place, step), named) in kept.iter().enumerate().zip(named) {
            let screenshot = match entry.pictures.get(&place) {
                Some((_, image)) => image.clone(),
                // The base image it has, which is written again as it is
                None => BgraImage::open_png(&folder.join(&step.base_image)).map_err(|error| {
                    let name = library_heading(&step.split);
                    format!("{name}'s base image can't be read, so choose a picture for it: {error}")
                })?,
            };
            let savestate = (!step.split.savestate.is_empty()).then(|| folder.join(&step.split.savestate));
            steps.push(NewStep {
                split: named,
                mask: (!step.split.mask.is_empty()).then(|| folder.join(&step.split.mask)),
                savestate,
                screenshot,
            });
        }
        Ok(Some(steps))
    }

    /// The masks and savestates of the preset being edited, which it takes from where they are
    /// until it's saved, like the library's for a split inserted from it.
    fn files_in_use(&self) -> Vec<PathBuf> {
        let maker = self.maker.borrow();
        let Some(preset) = maker.edit.as_ref().map(editor::Editing::preset) else { return Vec::new() };
        (preset.splits.iter())
            .flat_map(|split| [&split.mask, &split.savestate])
            .filter(|path| !path.is_empty())
            .map(|path| maker.file(path))
            .collect()
    }

    /// Asks to delete the `index`th of the game's ready-made splits, then deletes it with the
    /// files only it used.
    fn ask_delete_entry(self: &Rc<Self>, index: usize) {
        if !DEVELOPER {
            return;
        }
        let maker = self.maker.borrow();
        let Some(preset) = maker.edit.as_ref().map(editor::Editing::preset) else { return };
        let Some((folder, place)) = maker.library.place_of(&preset.game, index) else { return };
        let steps = maker.library.steps_of(&preset.game, index).unwrap_or_default();
        let name = steps.last().map(|step| library_heading(&step.split)).unwrap_or_default();
        drop(maker);
        let in_use = self.files_in_use();
        let text = format!(
            "Delete {name} from the split library? Its masks and base images are deleted too, unless another ready-made split uses them."
        );
        let weak = Rc::downgrade(self);
        self.show_message(Message::choice(text, ["Delete", "", "Cancel"], 2, move |answer| {
            let (Some(gui), Answer::Yes) = (weak.upgrade(), answer) else { return };
            if let Err(error) = delete_ready_made(&folder, place, &steps, &in_use) {
                return gui.show_message(Message::error(format!("Couldn't write the split library. {error}")));
            }
            gui.read_library();
            gui.refresh_library();
        }));
    }
}
