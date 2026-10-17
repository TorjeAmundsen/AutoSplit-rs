//! Split Library: the ready-made splits of the edited preset's game, from the splits library
//! next to the program, added to the preset one at a time.

use autosplit_core::preset::{LibrarySplit, LoadedLibrary, PresetSplit, game_key, load_libraries};
use slint::Model;
use slint::winit_030::WinitWindowAccessor;

use autosplit_core::split_image::SplitDefaults;

use super::super::library_file_parts;
use super::super::preset_form::{import_blocker, in_preset, library_file_name, library_heading, matches_search};
use super::super::ui::{LibraryCard, LibraryStep, NamePart};
use super::*;

/// The splits libraries, read when Split Library opens, with their base images.
#[derive(Default)]
pub(super) struct Library {
    libraries: Vec<LoadedLibrary>,
    /// Each base image by its path. They're small already, so they're shown as they are.
    thumbs: HashMap<PathBuf, slint::Image>,
    /// The pointer is over Insert Above (true) or Insert Below (false), whose place the split
    /// list shows.
    hovering: Option<bool>,
    /// Where the card being dragged would go in the split list, which it shows instead. Some
    /// while a card is dragged, with none while it's not over the list.
    dragging: Option<Option<usize>>,
    /// The ready-made split shown in full, by its place among the game's.
    chosen: Option<usize>,
}

impl Library {
    /// The ready-made splits for `game`, each with its steps and its library's folder, in the
    /// order the cards show them.
    fn splits_for(&self, game: &str) -> Vec<(&Path, &[LibrarySplit])> {
        let key = game_key(game);
        (self.libraries.iter())
            .filter(|loaded| !key.is_empty() && game_key(&loaded.library.game) == key)
            .flat_map(|loaded| loaded.library.ready_made().map(|steps| (loaded.folder.as_path(), steps)))
            .collect()
    }

    /// Where the `index`th of `game`'s ready-made splits is: its library's folder, and its place
    /// among that library's.
    pub(super) fn place_of(&self, game: &str, index: usize) -> Option<(PathBuf, usize)> {
        let key = game_key(game);
        let mut left = index;
        for loaded in self.libraries.iter().filter(|loaded| !key.is_empty() && game_key(&loaded.library.game) == key) {
            let count = loaded.library.ready_made().count();
            if left < count {
                return Some((loaded.folder.clone(), left));
            }
            left -= count;
        }
        None
    }

    /// The `index`th of `game`'s ready-made splits, its images in order.
    pub(super) fn steps_of(&self, game: &str, index: usize) -> Option<Vec<LibrarySplit>> {
        self.splits_for(game).get(index).map(|(_, steps)| steps.to_vec())
    }

    /// The picture of a base image, once it's read.
    pub(super) fn thumb(&self, folder: &Path, base_image: &str) -> Option<slint::Image> {
        self.thumbs.get(&folder.join(base_image)).cloned()
    }
}

/// Where `window`'s inside is on the screen, below its title bar and inside its frame. Those
/// differ between windows on screens of different scales, so where the windows themselves are
/// isn't enough to find a point of one in the other. Where the window is, when there's no inside
/// to ask about.
fn inside_position(window: &slint::Window) -> slint::PhysicalPosition {
    (window.with_winit_window(|winit| winit.inner_position().ok()).flatten())
        .map(|at| slint::PhysicalPosition::new(at.x, at.y))
        .unwrap_or_else(|| window.position())
}

/// A PNG as a picture, read each time: Slint keeps pictures loaded by path, which would show a
/// base image written again under the same name as it was.
fn png_image(path: &Path) -> Option<slint::Image> {
    let image = BgraImage::open_png(path).ok()?;
    Some(slint::Image::from_rgba8(rgba(image.view())))
}

/// The folder with the splits library, next to the program. Builds that aren't installed use
/// the source tree's.
pub(super) fn splits_folder() -> PathBuf {
    let next_to_program = exe_folder().join("splits");
    if next_to_program.is_dir() || !cfg!(debug_assertions) {
        return next_to_program;
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../splits")
}

/// A ready-made split's image as its card shows it: the settings it has, or empty for the
/// profile's `defaults`, and its file name without its number.
fn library_step(step: &LibrarySplit, thumb: Option<&slint::Image>, defaults: &SplitDefaults) -> LibraryStep {
    let split = &step.split;
    let file = library_file_name(split, Some(defaults));
    LibraryStep {
        file_parts: library_file_parts(&file),
        file: file.into(),
        heading: library_heading(split).into(),
        thumb: thumb.cloned().unwrap_or_default(),
        threshold: split.threshold.map(|threshold| threshold.to_string()).unwrap_or_default().into(),
        delay: split.delay.map(|delay| format!("{delay} ms")).unwrap_or_default().into(),
        pause: split.pause.map(|pause| format!("{pause} s")).unwrap_or_default().into(),
        dummy: split.dummy,
        below: split.below,
    }
}

/// The start of a file name, its number or keyword, as the split list colors it, or an empty
/// part without one.
fn in_preset_part(in_preset: &Option<(String, String)>) -> NamePart {
    let parts = in_preset.as_ref().map(|(file, _)| file_name_parts(file));
    parts.and_then(|parts| parts.row_data(0)).unwrap_or_default()
}

impl Gui {
    pub(super) fn connect_library(&self) {
        let weak = self.me.clone();
        let maker = &self.windows.maker;
        on!(weak, maker, on_open_library, () gui => gui.open_library());
        let library = &self.windows.library;
        on!(weak, library, on_insert, (index, above) gui => gui.insert_split(index as usize, above));
        on!(weak, library, on_insert_hovered, (above, on) gui => {
            let mut maker = gui.maker.borrow_mut();
            let hovering = &mut maker.library.hovering;
            // Leaving one button can come after going over the other
            if on {
                *hovering = Some(above);
            } else if *hovering == Some(above) {
                *hovering = None;
            }
            drop(maker);
            gui.show_insert_line();
        });
        on!(weak, library, on_card_dragged, (_index, x, y) gui => gui.card_dragged(x, y));
        on!(weak, library, on_card_dropped, (index) gui => gui.card_dropped(index as usize));
        on!(weak, library, on_chosen, (index) gui => gui.maker.borrow_mut().library.chosen = Some(index as usize));
        on!(weak, library, on_search_edited, (_text) gui => gui.refresh_library());
        on!(weak, library, on_close_clicked, () gui => gui.close_library());
        let close = weak.clone();
        library.window().on_close_requested(move || {
            if let Some(gui) = close.upgrade() {
                gui.close_library();
            }
            slint::CloseRequestResponse::KeepWindowShown
        });
    }

    /// Opens Split Library with the libraries read again, in case they changed, and the search
    /// cleared.
    fn open_library(&self) {
        if !self.editing() {
            return;
        }
        self.read_library();
        let window = &self.windows.library;
        window.set_search_text(SharedString::new());
        window.set_can_drag(!self.under_wayland());
        window.set_developer(cfg!(debug_assertions));
        // Shown first, since only an open window is kept up to date
        self.show_window(window.window());
        self.refresh_library();
    }

    /// Reads the libraries and their base images, keeping which one is shown in full, and says
    /// which couldn't be read.
    pub(super) fn read_library(&self) {
        let (libraries, errors) = load_libraries(&splits_folder());
        let thumbs: HashMap<PathBuf, slint::Image> = (libraries.iter())
            .flat_map(|loaded| {
                let splits = loaded.library.splits.iter().filter(|split| !split.base_image.is_empty());
                splits.map(|split| loaded.folder.join(&split.base_image))
            })
            // One that can't be read keeps its empty box
            .filter_map(|path| Some((path.clone(), png_image(&path)?)))
            .collect();
        let mut maker = self.maker.borrow_mut();
        let chosen = maker.library.chosen;
        maker.library = Library { libraries, thumbs, chosen, ..Library::default() };
        drop(maker);
        let window = &self.windows.library;
        let errors: Vec<String> = errors.iter().map(ToString::to_string).collect();
        let short = match errors.len() {
            0 => String::new(),
            1 => "A splits library couldn't be read.".to_owned(),
            count => format!("{count} splits libraries couldn't be read."),
        };
        window.set_errors(short.into());
        window.set_error_details(errors.join("\n\n").into());
    }

    /// Closes Split Library, from Close, its close button, Escape, or when editing ends.
    pub(in crate::gui) fn close_library(&self) {
        let _ = self.windows.library.hide();
        self.library_hidden();
    }

    /// Split Library is hidden, so a split it was dragging or about to insert goes nowhere: the
    /// line it showed in the split list goes, and the row being dragged stops being dragged. The
    /// pointer's release or leaving Insert, which would do that, never reaches a hidden window.
    pub(in crate::gui) fn library_hidden(&self) {
        let mut maker = self.maker.borrow_mut();
        maker.library.hovering = None;
        maker.library.dragging = None;
        drop(maker);
        let window = &self.windows.library;
        window.set_drag_reset(window.get_drag_reset().wrapping_add(1));
        self.windows.maker.set_drop_line(-1);
    }

    /// Keeps an open Split Library on the preset being edited, its game and which splits it
    /// has, and on what's searched.
    pub(super) fn refresh_library(&self) {
        let window = &self.windows.library;
        if !window.window().is_visible() {
            return;
        }
        let defaults = self.state.borrow().document.profile.split_defaults();
        let search = window.get_search_text();
        let maker = self.maker.borrow();
        let Some(preset) = maker.edit.as_ref().map(editor::Editing::preset) else { return };
        let ready_made = maker.library.splits_for(&preset.game);
        let cards: Vec<LibraryCard> = (ready_made.iter().enumerate())
            .filter(|(_, (_, steps))| {
                let headings: Vec<String> = steps.iter().map(|step| library_heading(&step.split)).collect();
                let names = steps.iter().map(|step| step.split.name.as_str());
                let descriptions = steps.iter().map(|step| step.description.as_str());
                let texts: Vec<&str> = names.chain(headings.iter().map(String::as_str)).chain(descriptions).collect();
                matches_search(&texts, &search)
            })
            .map(|(index, &(folder, steps))| {
                let splits: Vec<PresetSplit> = steps.iter().map(|step| step.split.clone()).collect();
                let shown: Vec<LibraryStep> = (steps.iter())
                    .map(|step| library_step(step, maker.library.thumbs.get(&folder.join(&step.base_image)), &defaults))
                    .collect();
                // The split it's for is its last image, which the ones before lead up to
                let last = splits.last();
                let in_preset = last.and_then(|last| in_preset(preset, &last.name, Some(&defaults)));
                let description = steps.iter().map(|step| step.description.as_str()).find(|text| !text.is_empty());
                LibraryCard {
                    ready_made: index as i32,
                    heading: last.map(library_heading).unwrap_or_default().into(),
                    description: description.unwrap_or_default().into(),
                    steps: ModelRc::new(VecModel::from(shown)),
                    in_preset: in_preset_part(&in_preset),
                    in_preset_file: in_preset.as_ref().map(|(file, _)| file.as_str()).unwrap_or_default().into(),
                    in_preset_note: in_preset.map(|(_, note)| note).unwrap_or_default().into(),
                    blocker: import_blocker(preset, &splits).unwrap_or_default().into(),
                }
            })
            .collect();
        let empty = match preset.game.trim() {
            "" => "Pick the preset's game first. Its ready-made splits show here.".to_owned(),
            _ if !ready_made.is_empty() => format!("No ready-made splits match {}.", search.trim()),
            game => format!("There are no ready-made splits for {game} yet."),
        };
        let title = match preset.game.trim() {
            "" => "Split Library".to_owned(),
            game => format!("Split Library - {game}"),
        };
        // The one shown in full stays, while it's listed, and the first is shown otherwise
        let chosen = maker.library.chosen;
        let selected = (cards.iter().position(|card| Some(card.ready_made as usize) == chosen))
            .or_else(|| (!cards.is_empty()).then_some(0));
        drop(maker);
        self.maker.borrow_mut().library.chosen = selected.map(|selected| cards[selected].ready_made as usize);
        window.set_empty_message(empty.into());
        window.set_window_title(title.into());
        let model = window.get_cards();
        if model.row_count() == cards.len() {
            // In place, so the list keeps its place
            for (index, card) in cards.into_iter().enumerate() {
                model.set_row_data(index, card);
            }
        } else {
            window.set_cards(ModelRc::new(VecModel::from(cards)));
        }
        window.set_selected(selected.map_or(-1, |selected| selected as i32));
        self.show_insert_line();
    }

    /// Shows in the split list where a ready-made split would go: where the one being dragged
    /// is, or around the selected split for the Insert button the pointer is over.
    pub(super) fn show_insert_line(&self) {
        let maker = self.maker.borrow();
        let library = &maker.library;
        let line = match (&maker.edit, library.dragging, library.hovering) {
            (Some(_), Some(place), _) => place,
            (Some(edit), None, Some(above)) => {
                let (draft, selected) = (edit.draft(), Some(maker.selected));
                Some(if above { draft.place_before(selected) } else { draft.place_after(selected) })
            }
            _ => None,
        };
        drop(maker);
        self.windows.maker.set_drop_line(line.map_or(-1, |line| line as i32));
    }

    /// A card is dragged with the pointer at `x` and `y` in Split Library: where it would go in
    /// the split list, when it's over it. Make Split Images is found from where both windows'
    /// insides are on the screen.
    fn card_dragged(&self, x: f32, y: f32) {
        let from = self.windows.library.window();
        // A move that comes after it's hidden, which ended the drag
        if !from.is_visible() {
            return;
        }
        let to = self.windows.maker.window();
        let (scale, size) = (from.scale_factor(), from.size());
        let (inside_x, inside_y) = (x * scale, y * scale);
        // Over Split Library itself, which is in front
        let over_itself =
            inside_x >= 0.0 && inside_y >= 0.0 && inside_x < size.width as f32 && inside_y < size.height as f32;
        let gap = if over_itself || !to.is_visible() {
            -1
        } else {
            let (from_at, to_at, to_scale) = (inside_position(from), inside_position(to), to.scale_factor());
            let maker_x = (from_at.x as f32 + inside_x - to_at.x as f32) / to_scale;
            let maker_y = (from_at.y as f32 + inside_y - to_at.y as f32) / to_scale;
            self.windows.maker.invoke_drop_gap(maker_x, maker_y)
        };
        let mut maker = self.maker.borrow_mut();
        let place =
            (maker.edit.as_ref()).zip(usize::try_from(gap).ok()).map(|(edit, gap)| edit.draft().drop_place(gap));
        maker.library.dragging = Some(place);
        drop(maker);
        self.show_insert_line();
    }

    /// The card being dragged was let go: its split goes where the line shows, if anywhere.
    fn card_dropped(&self, index: usize) {
        let place = self.maker.borrow_mut().library.dragging.take().flatten();
        self.show_insert_line();
        if let Some(place) = place {
            self.import_at(index, place, false);
        }
    }

    /// Insert Above or Insert Below on a card, by its place among the game's ready-made splits.
    fn insert_split(&self, index: usize, above: bool) {
        let maker = self.maker.borrow();
        let Some(edit) = &maker.edit else { return };
        let selected = Some(maker.selected);
        let place = if above { edit.draft().place_before(selected) } else { edit.draft().place_after(selected) };
        drop(maker);
        self.import_at(index, place, above);
    }

    /// Puts the `index`th of the game's ready-made splits in the preset at `place`, unless a
    /// message box is open or there'd be too many reset or start images.
    fn import_at(&self, index: usize, place: usize, keep_selected: bool) {
        if self.blocked() {
            return self.focus_message();
        }
        let maker = self.maker.borrow();
        let Some(preset) = maker.edit.as_ref().map(editor::Editing::preset) else { return };
        let Some((folder, steps)) = maker.library.splits_for(&preset.game).get(index).copied() else { return };
        let steps: Vec<PresetSplit> = steps.iter().map(|step| step.split.clone()).collect();
        if import_blocker(preset, &steps).is_some() {
            return;
        }
        let folder = folder.to_owned();
        drop(maker);
        self.import_library_split(&steps, &folder, place, keep_selected);
    }
}
