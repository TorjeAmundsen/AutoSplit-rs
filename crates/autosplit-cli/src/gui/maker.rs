//! The Make Split Images window: a preset's masks applied to the capture or to screenshots, saved
//! into the split image folder with the names AutoSplit reads.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use autosplit_core::image::{BgraImage, BgraView};
use autosplit_core::imgproc::{ResizeBuffers, stretch_to_rgba};
use autosplit_core::mask::{Mask, apply_mask};
use autosplit_core::preset::{LoadedPreset, Preset, PresetError, game_key, load_presets};
use autosplit_core::split_folder::user_files;
use slint::{ComponentHandle, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};

use super::maker_plan::{
    canonical, file_name_text, looks_made, move_to_base, plan_writes, saves_in_place, screenshots_to_move,
    stale_to_delete,
};
use super::messages::{Answer, Message};
use super::placement::beside;
use super::preset_form::{Draft, Problem, instruction_cards, problem_flags, problem_summary, split_heading};
use super::ui::{InstructionCard, MakerRow};
use super::view::{Pixels, rgba};
use super::{Gui, Tool, exe_folder, file_name_parts, with_gui};
use crate::session::UiCommand;

mod editor;
mod preview;
mod savestates;
mod write;

/// Where a split image comes from.
#[derive(Clone)]
enum Source {
    /// A screenshot file.
    File(PathBuf),
    /// The capture region, grabbed for the split.
    Grabbed(Arc<BgraImage>),
}

impl Source {
    /// What the window says the split image is made from.
    fn label(&self) -> String {
        match self {
            Self::File(path) => file_name_text(path).unwrap_or_default(),
            Self::Grabbed(_) => "grabbed".into(),
        }
    }

    /// Whether it's the same screenshot file or the same grab.
    fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::File(a), Self::File(b)) => a == b,
            (Self::Grabbed(a), Self::Grabbed(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }

    fn image(&self) -> Result<Arc<BgraImage>, String> {
        match self {
            Self::File(path) => BgraImage::open_png(path)
                .map(Arc::new)
                .map_err(|error| format!("Couldn't read {}: {error}", path.display())),
            Self::Grabbed(image) => Ok(image.clone()),
        }
    }
}

/// The window's preset, and what's been done with it.
#[derive(Default)]
pub(super) struct Maker {
    presets: Vec<LoadedPreset>,
    preset: usize,
    /// What's been done for each split, in the splits' order.
    splits: Vec<SplitWork>,
    selected: usize,
    /// Masks by path, read once.
    masks: HashMap<PathBuf, Arc<Mask>>,
    /// The selected split's mask at the size of the capture's preview, with that size, for the
    /// live preview of its split image.
    live_mask: Option<((usize, usize), Vec<u8>)>,
    /// Changes when another preset is chosen, so split images saved for the one before don't
    /// count for this one.
    generation: u64,
    /// The preset being edited, which the window shows instead of the chosen one.
    edit: Option<editor::Editing>,
    /// How many times editing started, which tells each time apart, for a file picked for the
    /// time before.
    edits: u64,
    /// Split images being written and pictures being made. Editing waits for them, since going
    /// back from it restores the splits as they were before, which would lose what they bring.
    working: usize,
    /// Split images are being written, which other saves wait for.
    saving: bool,
    /// Splits to save once the ones being saved are, for the preset of that generation.
    queued: Option<(u64, Vec<usize>)>,
}

/// What's been done for a split.
#[derive(Clone, Default)]
struct SplitWork {
    /// What its image is made from.
    source: Option<Source>,
    /// Its split image, once it's written.
    saved: Option<PathBuf>,
    /// A small picture of its split image, once it's made from its source.
    thumb: Option<slint::Image>,
}

impl SplitWork {
    /// Makes its image from `source` from now on: what was saved or pictured of the one before
    /// is another image.
    fn set_source(&mut self, source: Source) {
        *self = Self { source: Some(source), ..Self::default() };
    }
}

/// What the window has for each split, kept while a preset is edited to go back to.
struct Kept {
    splits: Vec<SplitWork>,
    selected: usize,
}

impl Maker {
    fn loaded(&self) -> Option<&LoadedPreset> {
        self.presets.get(self.preset)
    }

    /// The preset the window shows: the one being edited, or the chosen one.
    fn shown(&self) -> Option<&Preset> {
        match &self.edit {
            Some(edit) => Some(&edit.draft.preset),
            None => self.loaded().map(|loaded| &loaded.preset),
        }
    }

    /// A mask or savestate of the shown preset: relative to its folder, or picked while editing.
    fn file(&self, path: &str) -> PathBuf {
        let folder = match &self.edit {
            Some(edit) => edit.source.as_deref(),
            None => self.loaded().map(|loaded| loaded.folder.as_path()),
        };
        folder.map_or_else(|| PathBuf::from(path), |folder| folder.join(path))
    }

    fn keep(&self) -> Kept {
        Kept { splits: self.splits.clone(), selected: self.selected }
    }

    fn restore(&mut self, kept: Kept) {
        (self.splits, self.selected) = (kept.splits, kept.selected);
    }

    /// Nothing done yet for `count` splits, the first selected.
    fn start_over(&mut self, count: usize) {
        (self.splits, self.selected) = (vec![SplitWork::default(); count], 0);
    }

    /// What split `index`'s image is made from, if anything.
    fn source(&self, index: usize) -> Option<&Source> {
        self.splits.get(index)?.source.as_ref()
    }

    /// The games of the presets, in the order they're listed, each as its first preset spells it.
    fn games(&self) -> Vec<&str> {
        let mut games: Vec<&str> = self.presets.iter().map(|loaded| loaded.preset.game.as_str()).collect();
        // Sorted by game_key, so the presets of a game are next to each other
        games.dedup_by(|game, before| game_key(game) == game_key(before));
        games
    }

    /// Where `game` is among the games, however it's spelled.
    fn game_index(&self, game: &str) -> Option<usize> {
        let key = game_key(game);
        self.games().iter().position(|&each| game_key(each) == key)
    }

    /// The presets of `game`, however it's spelled, by index.
    fn presets_of(&self, game: &str) -> Vec<usize> {
        let key = game_key(game);
        (0..self.presets.len()).filter(|&index| game_key(&self.presets[index].preset.game) == key).collect()
    }
}

/// `count` things, like "1 split image" or "2 split images".
fn plural(count: usize, thing: &str) -> String {
    if count == 1 { format!("1 {thing}") } else { format!("{count} {thing}s") }
}

/// The size of the split list's pictures of the split images, twice what's shown for sharp
/// pictures on high resolution screens.
pub const THUMB_SIZE: (usize, usize) = (96, 72);

/// A small picture of a split's screenshot or grab, unmasked: the mask usually keeps too little
/// of it to tell at a glance.
pub fn thumbnail(shot: BgraView) -> Pixels {
    let (width, height) = THUMB_SIZE;
    let mut pixels = vec![[0; 4]; width * height];
    stretch_to_rgba(shot, width, height, &mut pixels);
    pixels.iter_mut().for_each(|pixel| pixel[3] = 255);
    Pixels::clone_from_slice(pixels.as_flattened(), width as u32, height as u32)
}

/// What the window shows of the preset and its splits, besides the edit form and the preview,
/// worked out before any of it is shown.
struct MakerView {
    rows: Vec<MakerRow>,
    /// What has to be fixed before anything's made from the chosen preset, or empty.
    preset_problem: String,
    can_grab: bool,
    can_save_selected: bool,
    can_save_all: bool,
    can_test: bool,
    needs_grouping: bool,
    working: bool,
    has_savestates: bool,
    has_instructions: bool,
}

/// What the window shows for `maker`, whose splits have the file names `names`. `in_place` says
/// which are saved, `problems` which have something to fix, `preset_problems` are the chosen
/// preset's, and `grouping` is whether the profile compares images with the same number for one
/// split.
fn maker_view(
    maker: &Maker,
    names: &[String],
    in_place: &[bool],
    problems: &[bool],
    preset_problems: &[Problem],
    grouping: bool,
) -> MakerView {
    let joined = |index| maker.shown().is_some_and(|preset| preset.joins_previous(index));
    // Every other split is shaded, splits compared together as one
    let mut block = 0;
    let rows = (names.iter().enumerate())
        .map(|(index, name)| MakerRow {
            heading: maker.shown().map(|preset| split_heading(preset, index)).unwrap_or_default().into(),
            parts: file_name_parts(name),
            file: name.into(),
            source: maker.source(index).map(Source::label).unwrap_or_default().into(),
            saved: in_place.get(index).copied().unwrap_or_default(),
            thumb: maker.splits.get(index).and_then(|split| split.thumb.clone()).unwrap_or_default(),
            has_thumb: maker.splits.get(index).is_some_and(|split| split.thumb.is_some()),
            problem: problems.get(index).copied().unwrap_or_default(),
            joined: joined(index),
            shaded: {
                if index > 0 && !joined(index) {
                    block += 1;
                }
                block % 2 == 1
            },
        })
        .collect();
    // A preset with problems is only for fixing, so nothing's made from it
    let summary = maker.loaded().and_then(|loaded| problem_summary(&loaded.preset, preset_problems));
    let usable = summary.is_none();
    let selected = maker.selected;
    let splits = maker.loaded().map_or(&[][..], |loaded| &loaded.preset.splits[..]);
    MakerView {
        rows,
        preset_problem: summary.unwrap_or_default(),
        can_grab: !maker.splits.is_empty() && usable,
        can_save_selected: maker.source(selected).is_some() && usable,
        can_save_all: maker.splits.iter().any(|split| split.source.is_some()) && usable,
        can_test: in_place.get(selected).copied().unwrap_or_default() && usable,
        needs_grouping: !grouping && maker.loaded().is_some_and(|loaded| loaded.preset.needs_grouping()),
        working: maker.working > 0,
        has_savestates: usable && splits.iter().any(|split| !split.savestate.is_empty()),
        has_instructions: splits.iter().any(|split| !split.savestate.is_empty() && !split.instructions.is_empty()),
    }
}

/// A game's name in the list of games.
fn game_label(game: &str) -> SharedString {
    if game.is_empty() { "No game".into() } else { game.into() }
}

/// The transparency grid behind split images: AutoMask's grey squares.
pub fn transparency_grid(width: u32, height: u32) -> slint::Image {
    const SQUARE: u32 = 8;
    let mut grid = SharedPixelBuffer::<Rgba8Pixel>::new(width, height);
    for (index, pixel) in grid.make_mut_slice().iter_mut().enumerate() {
        let (x, y) = (index as u32 % width, index as u32 / width);
        let light = (x / SQUARE + y / SQUARE).is_multiple_of(2);
        let value = if light { 0xBB } else { 0x88 };
        *pixel = Rgba8Pixel { r: value, g: value, b: value, a: 255 };
    }
    slint::Image::from_rgba8(grid)
}

/// The folder with the bundled presets, next to the program. Builds that aren't installed use
/// the source tree's.
fn presets_folder() -> PathBuf {
    let next_to_program = exe_folder().join("presets");
    if next_to_program.is_dir() || !cfg!(debug_assertions) {
        return next_to_program;
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../presets")
}

impl Gui {
    pub(super) fn connect_maker(&self) {
        let weak = self.me.clone();
        let maker = &self.windows.maker;
        on!(weak, maker, on_game_chosen, (index) gui => gui.choose_game(index as usize));
        on!(weak, maker, on_preset_chosen, (index) gui => {
            let maker = gui.maker.borrow();
            let game = maker.loaded().map(|loaded| loaded.preset.game.clone()).unwrap_or_default();
            let chosen = maker.presets_of(&game).get(index as usize).copied();
            drop(maker);
            if let Some(chosen) = chosen {
                gui.choose_preset(chosen);
            }
        });
        on!(weak, maker, on_split_chosen, (index) gui => gui.choose_split(index as usize));
        on!(weak, maker, on_grab, () gui => gui.send(UiCommand::Grab));
        on!(weak, maker, on_use_screenshots, () gui => gui.pick_screenshots());
        on!(weak, maker, on_save_selected, () gui => {
            let selected = gui.maker.borrow().selected;
            gui.save_split_images(vec![selected]);
        });
        on!(weak, maker, on_save_all, () gui => {
            let all = (0..gui.maker.borrow().splits.len()).collect();
            gui.save_split_images(all);
        });
        on!(weak, maker, on_test, () gui => gui.test_split_image());
        on!(weak, maker, on_copy_savestates, () gui => gui.copy_savestates());
        on!(weak, maker, on_show_instructions, () gui => gui.show_instructions());
        on!(weak, maker, on_browse_folder, () gui => gui.browse_folder(gui.windows.maker.window(), |gui| gui.maker_folder_changed()));
        on!(weak, maker, on_generate_profile, () gui => gui.generate_profile());
        on!(weak, maker, on_turn_on_grouping, () gui => gui.turn_on_grouping());
        on!(always weak, maker, on_blocked_clicked, () gui => gui.focus_message());
        let close = weak.clone();
        maker.window().on_close_requested(move || {
            let Some(gui) = close.upgrade() else { return slint::CloseRequestResponse::HideWindow };
            gui.close_maker();
            // Closed by close_maker, unless there's a question first
            slint::CloseRequestResponse::KeepWindowShown
        });
        self.connect_editor();
    }

    /// Opens Make Split Images beside the main window, with the presets read again.
    pub(super) fn open_maker(self: &Rc<Self>) {
        let window = self.windows.maker.window();
        if window.is_visible() {
            return self.show_window(window);
        }
        let folder = presets_folder();
        let before = self.maker.borrow().loaded().cloned();
        let errors = self.read_presets();
        let mut maker = self.maker.borrow_mut();
        // The preset from before, which presets added or removed meanwhile may have moved
        let found = before.as_ref().and_then(|before| {
            let folder = canonical(&before.folder);
            maker.presets.iter().position(|loaded| canonical(&loaded.folder) == folder)
        });
        let chosen = found.unwrap_or(maker.preset.min(maker.presets.len().saturating_sub(1)));
        // What was grabbed or picked for the same preset is kept, unless its splits changed
        let same = found.is_some()
            && before
                .zip(maker.presets.get(chosen))
                .is_some_and(|(before, now)| before.preset.splits == now.preset.splits);
        // Masks are read again, in case they were edited
        maker.masks.clear();
        let empty = maker.presets.is_empty();
        drop(maker);
        let maker_window = &self.windows.maker;
        maker_window.set_rows(ModelRc::from(self.maker_rows.clone()));
        maker_window.set_grid_image(transparency_grid(320, 240));
        maker_window.set_status(SharedString::new());
        if same {
            self.show_preset(chosen);
        } else {
            self.choose_preset(chosen);
        }
        if empty {
            maker_window.set_status(format!("No presets found in {}.", folder.display()).into());
        }
        self.place_by_main(window, beside);
        self.show_window(window);
        self.tool_opened(Tool::Maker);
        self.send(UiCommand::SetMakerOpen(true));
        self.update_visibility();
        self.show_preset_errors(errors);
    }

    /// Reads the presets in the presets folder, and lists their games. Returns the ones that
    /// couldn't be read.
    fn read_presets(&self) -> Vec<PresetError> {
        let (presets, errors) = load_presets(&presets_folder());
        let mut maker = self.maker.borrow_mut();
        maker.presets = presets;
        let games: Vec<SharedString> = maker.games().into_iter().map(game_label).collect();
        drop(maker);
        self.windows.maker.set_games(ModelRc::new(VecModel::from(games)));
        errors
    }

    fn show_preset_errors(&self, errors: Vec<PresetError>) {
        if !errors.is_empty() {
            let errors: Vec<String> = errors.iter().map(ToString::to_string).collect();
            self.show_message(Message::warning(format!("Some presets couldn't be read:\n\n{}", errors.join("\n\n"))));
        }
    }

    /// Closes Make Split Images, from its close button or the main window's, after asking about
    /// a preset with unsaved changes. Editing ends with it.
    pub(super) fn close_maker(self: &Rc<Self>) {
        if self.blocked() {
            return self.focus_message();
        }
        self.unless_discarded(|gui| {
            gui.cancel_editing();
            gui.maker_closed();
            let _ = gui.windows.maker.hide();
        });
    }

    /// Make Split Images closes: the capture is only needed for it while it's open.
    fn maker_closed(&self) {
        // They're the window's preset's
        let _ = self.windows.instructions.window().hide();
        self.tool_closed(Tool::Maker);
        self.send(UiCommand::SetMakerOpen(false));
        self.maker_visible.store(false, std::sync::atomic::Ordering::Relaxed);
    }

    /// The first preset of the `index`th game.
    fn choose_game(&self, index: usize) {
        let maker = self.maker.borrow();
        let first = maker.games().get(index).and_then(|game| maker.presets_of(game).first().copied());
        drop(maker);
        if let Some(first) = first {
            self.choose_preset(first);
        }
    }

    /// Chooses the `index`th preset, and its game with the game's presets in the lists.
    fn choose_preset(&self, index: usize) {
        let mut maker = self.maker.borrow_mut();
        maker.preset = index;
        maker.generation += 1;
        let count = maker.loaded().map_or(0, |loaded| loaded.preset.splits.len());
        maker.start_over(count);
        drop(maker);
        self.windows.maker.set_status(SharedString::new());
        self.show_preset(index);
    }

    /// Shows the `index`th preset, its game and the game's presets in the lists, and what's
    /// been done with its splits.
    fn show_preset(&self, index: usize) {
        let mut maker = self.maker.borrow_mut();
        maker.preset = index;
        let game = maker.loaded().map(|loaded| loaded.preset.game.clone()).unwrap_or_default();
        let game_index = maker.game_index(&game).unwrap_or(0);
        let of_game = maker.presets_of(&game);
        let names: Vec<SharedString> =
            of_game.iter().map(|&each| maker.presets[each].preset.name.clone().into()).collect();
        let window = &self.windows.maker;
        window.set_game_index(game_index as i32);
        window.set_presets(ModelRc::new(VecModel::from(names)));
        window.set_preset_index(of_game.iter().position(|&each| each == index).unwrap_or(0) as i32);
        let selected = maker.selected;
        drop(maker);
        self.choose_split(selected);
    }

    fn choose_split(&self, index: usize) {
        self.maker.borrow_mut().selected = index;
        self.windows.maker.set_selected(index as i32);
        self.maker.borrow_mut().live_mask = None;
        self.show_form();
        self.show_maker();
        self.refresh_instructions();
    }

    /// The split image folder, as the profile says.
    fn split_image_folder(&self) -> Option<PathBuf> {
        let state = self.state.borrow();
        let directory = &state.document.profile.split_image_directory;
        (!directory.is_empty()).then(|| state.document.resolve(&exe_folder(), directory))
    }

    /// The preset's file names, with the profile's defaults for what they leave out.
    fn maker_file_names(&self) -> Vec<String> {
        let defaults = self.state.borrow().document.profile.split_defaults();
        self.maker.borrow().shown().map(|preset| preset.file_names(Some(&defaults))).unwrap_or_default()
    }

    /// Which splits' saved split images are in the split image folder under the split's file
    /// name in `names`. One that isn't, because the folder changed, the file was deleted or the
    /// split's settings changed its name, is kept and counts again once it is, like after a
    /// setting is changed and changed back while editing.
    fn saves_in_place(&self, names: &[String]) -> Vec<bool> {
        let folder = self.split_image_folder();
        let saved: Vec<Option<PathBuf>> = self.maker.borrow().splits.iter().map(|split| split.saved.clone()).collect();
        saves_in_place(&saved, names, folder.as_deref())
    }

    /// Puts the preset's splits, the selected split's split image and the buttons in the window.
    fn show_maker(&self) {
        let names = self.maker_file_names();
        let in_place = self.saves_in_place(&names);
        let mut problems = self.show_editing();
        let preset_problems = self.preset_problems();
        if !self.editing() {
            problems = problem_flags(&preset_problems, names.len());
        }
        let grouping = self.state.borrow().document.profile.group_split_images;
        let view = maker_view(&self.maker.borrow(), &names, &in_place, &problems, &preset_problems, grouping);
        self.maker_rows.set_vec(view.rows);
        let window = &self.windows.maker;
        // Empty without one, which the window says
        let folder = self.split_image_folder();
        window.set_output_folder(folder.map(|folder| folder.display().to_string()).unwrap_or_default().into());
        window.set_preset_problem(view.preset_problem.into());
        window.set_can_grab(view.can_grab);
        window.set_can_save_selected(view.can_save_selected);
        window.set_can_save_all(view.can_save_all);
        window.set_can_test(view.can_test);
        window.set_needs_grouping(view.needs_grouping);
        window.set_working(view.working);
        window.set_has_savestates(view.has_savestates);
        window.set_has_instructions(view.has_instructions);
        self.show_maker_preview();
    }

    /// What the editor would ask to fix in the chosen preset, like splits compared together
    /// with the same name in a preset written by hand.
    fn preset_problems(&self) -> Vec<Problem> {
        let maker = self.maker.borrow();
        maker.loaded().map(|loaded| Draft::edit(loaded.preset.clone()).problems()).unwrap_or_default()
    }

    /// Says what to fix when the chosen preset has problems, and whether it has. Making split
    /// images or copying savestates from it waits until it's fixed.
    fn refused_for_problems(&self) -> bool {
        let problems = self.preset_problems();
        let maker = self.maker.borrow();
        let Some(summary) = maker.loaded().and_then(|loaded| problem_summary(&loaded.preset, &problems)) else {
            return false;
        };
        drop(maker);
        self.show_message(Message::warning(format!("Fix the preset with Edit first. {summary}")));
        true
    }

    /// The session's capture region, for the selected split: its split image is written, and the
    /// next split is selected.
    pub(super) fn grabbed(self: &Rc<Self>, image: Result<BgraImage, String>) {
        if !self.windows.maker.window().is_visible() {
            return;
        }
        // The split's file name and mask aren't settled until the preset is saved
        if self.editing() {
            return self.show_message(Message::warning("Save or cancel the changes to the preset first, then grab."));
        }
        if self.refused_for_problems() {
            return;
        }
        let image = match image {
            Ok(image) => Arc::new(image),
            Err(error) => return self.show_message(Message::error(error)),
        };
        let mut maker = self.maker.borrow_mut();
        let selected = maker.selected;
        let count = maker.splits.len();
        let Some(split) = maker.splits.get_mut(selected) else { return };
        split.set_source(Source::Grabbed(image));
        drop(maker);
        self.make_thumbnails(vec![selected]);
        // Stays on the split when its image can't be saved, to grab it again once that's fixed
        if self.save_split_images(vec![selected]) && selected + 1 < count {
            self.choose_split(selected + 1);
        } else {
            self.show_maker();
        }
    }

    /// Asks for screenshots: one for the selected split, or several for the splits in order from
    /// the first, like AutoSplit numbers its screenshots.
    fn pick_screenshots(self: &Rc<Self>) {
        if self.refused_for_problems() {
            return;
        }
        let state = self.state.borrow();
        let profile = &state.document.profile;
        let folder = match profile.screenshot_directory.as_str() {
            "" => profile.split_image_directory.clone(),
            screenshots => screenshots.to_owned(),
        };
        let start = state.document.resolve(&exe_folder(), &folder);
        drop(state);
        let dialog = self
            .file_dialog_over("Select Screenshots", self.windows.maker.window())
            .set_directory(start)
            .add_filter("PNG image", &["png"])
            .pick_files();
        let weak = Rc::downgrade(self);
        let waiting = slint::spawn_local(async move {
            let Some(files) = dialog.await else { return };
            let Some(gui) = weak.upgrade() else { return };
            let mut paths: Vec<PathBuf> = files.iter().map(|file| file.path().to_owned()).collect();
            paths.sort_by_key(|path| path.file_name().map(|name| name.to_string_lossy().to_lowercase()));
            gui.use_screenshots(paths);
        });
        if let Err(error) = waiting {
            self.add_log_line(format!("error: Couldn't open a file dialog: {error}"));
        }
    }

    fn use_screenshots(&self, paths: Vec<PathBuf>) {
        let mut maker = self.maker.borrow_mut();
        let first = if paths.len() == 1 { maker.selected } else { 0 };
        let count = maker.splits.len();
        let mut set = Vec::new();
        for (offset, path) in paths.iter().enumerate().filter(|(offset, _)| first + offset < count) {
            maker.splits[first + offset].set_source(Source::File(path.clone()));
            set.push(first + offset);
        }
        drop(maker);
        self.make_thumbnails(set);
        // Unlike a grab, they're saved when asked
        let status = match paths.len() {
            1 => "Screenshot set for the selected split. Save Selected writes its split image.".to_owned(),
            n if n == count => format!("{n} screenshots set, one for each split. Save All writes their split images."),
            n => format!(
                "{n} screenshots for {count} splits, set in order from the first. Save All writes their split images."
            ),
        };
        self.windows.maker.set_status(status.into());
        // Split images picked as screenshots would be masked twice
        let made: Vec<String> =
            paths.iter().filter_map(|path| file_name_text(path)).filter(|name| looks_made(name)).collect();
        if !made.is_empty() {
            self.show_message(Message::warning(format!(
                "These look like split images already, not screenshots:\n\n{}",
                made.join("\n")
            )));
        }
        self.show_maker();
    }

    /// Another split image folder was picked: the split images saved before are in the old one.
    fn maker_folder_changed(&self) {
        self.windows.maker.set_status(SharedString::new());
        self.show_maker();
    }

    /// Saves the profile as a new file named for the preset, which is loaded from then on like
    /// Save Profile As does.
    fn generate_profile(self: &Rc<Self>) {
        if self.refused_for_problems() {
            return;
        }
        let maker = self.maker.borrow();
        let Some(loaded) = maker.loaded() else { return };
        let (name, needs_grouping) = (loaded.preset.profile_file_name(), loaded.preset.needs_grouping());
        drop(maker);
        let folder = self.state.borrow().document.folder(&exe_folder());
        let dialog = self
            .file_dialog_over("Generate Profile", self.windows.maker.window())
            .set_directory(folder)
            .set_file_name(name)
            .add_filter("TOML", &["toml"])
            .save_file();
        self.when_picked(dialog, move |gui, path| {
            if needs_grouping {
                gui.turn_on_grouping();
            }
            let name = file_name_text(&path).unwrap_or_default();
            let saved =
                move |gui: &Rc<Gui>| gui.windows.maker.set_status(format!("Saved the profile as {name}.").into());
            gui.save_to(path, Some(Box::new(saved)));
        });
    }

    /// Has the profile compare split images with the same number for one split, which the preset
    /// needs.
    fn turn_on_grouping(&self) {
        self.state.borrow_mut().document.profile.group_split_images = true;
        self.profile_edited();
        self.windows.maker.set_status("Grouping is on. Save the profile to keep it.".into());
    }

    /// Shows what the profile decides in the window when it changes while the window is open,
    /// like from Settings or LiveSplit: the file names, the output folder, whether grouping is on
    /// and the defaults that unticked fields show.
    pub(super) fn maker_follows_profile(&self) {
        if !self.windows.maker.window().is_visible() {
            return;
        }
        self.show_maker();
        self.show_form();
    }

    /// Opens the Live Tester on the selected split's split image.
    fn test_split_image(self: &Rc<Self>) {
        if self.refused_for_problems() {
            return;
        }
        let maker = self.maker.borrow();
        let Some(path) = maker.splits.get(maker.selected).and_then(|split| split.saved.clone()) else { return };
        drop(maker);
        self.state.borrow_mut().tester_path = Some(path);
        if self.windows.tester.window().is_visible() {
            self.load_tester_image();
            self.show_window(self.windows.tester.window());
        } else {
            self.open_tester();
        }
    }
}
