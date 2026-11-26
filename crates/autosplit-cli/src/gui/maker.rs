//! The Make Split Images window: a preset's masks applied to the capture or to screenshots, saved
//! into the split image folder with the names AutoSplit reads.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use autosplit_core::image::{BgraImage, BgraView};
use autosplit_core::imgproc::{ResizeBuffers, stretch_to_rgba};
use autosplit_core::mask::{Mask, apply_mask, fade_image};
use autosplit_core::preset::{
    LoadedPreset, Preset, PresetError, PresetSplit, SplitChoice, load_presets, sweep_shared_files,
};
use autosplit_core::profile::UserProfile;
use autosplit_core::split_folder::user_files;
use autosplit_core::text::hex_color;
use slint::{ComponentHandle, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};

use super::document::Document;
use super::maker_plan::{
    PresetList, choice_summary, images_of_splits, looks_made, move_to_backup, move_to_base, next_to_grab, old_images,
    plan_writes, row_places, saves_in_place, screenshots_set_status, screenshots_status, screenshots_to_move,
    stale_to_delete, writes_over,
};
use super::messages::{Answer, Message};
use super::paths::{canonical, file_name_text};
use super::placement::beside;
use super::preset_form::{
    Problem, instruction_cards, problem_flags, problem_summary, problems_of, split_heading, why_not_toggleable,
};
use super::ui::{InstructionCard, MakerRow};
use super::view::{Pixels, rgba};
use super::{Gui, Tool, exe_folder, file_name_parts, with_gui};
use crate::session::UiCommand;

mod editor;
mod library;
mod library_entry;
mod preview;
mod savestates;
mod sharing;
mod write;

/// Where a split image comes from.
#[derive(Clone)]
enum Source {
    /// A screenshot file picked for the split.
    File(PathBuf),
    /// The screenshot the split's preset has for it, which a grab or picked file comes before.
    Preset(PathBuf),
    /// The capture region, grabbed for the split.
    Grabbed(Arc<BgraImage>),
    /// A fade split's color (RGB), which it's made of whatever was grabbed or picked for it.
    Fade([u8; 3]),
}

impl Source {
    /// What the window says the split image is made from.
    fn label(&self) -> String {
        match self {
            Self::File(path) => file_name_text(path).unwrap_or_default(),
            Self::Preset(_) => "preset's screenshot".into(),
            Self::Grabbed(_) => "grabbed".into(),
            Self::Fade(color) => format!("fade to {}", hex_color(*color)),
        }
    }

    /// Whether it's the same screenshot file or the same grab.
    fn same(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::File(a), Self::File(b)) | (Self::Preset(a), Self::Preset(b)) => a == b,
            (Self::Grabbed(a), Self::Grabbed(b)) => Arc::ptr_eq(a, b),
            (Self::Fade(a), Self::Fade(b)) => a == b,
            _ => false,
        }
    }

    fn image(&self) -> Result<Arc<BgraImage>, String> {
        match self {
            Self::File(path) => BgraImage::open_png(path)
                .map(Arc::new)
                .map_err(|error| format!("Couldn't read {}: {error}", path.display())),
            // Not the runner's file to fix, so it says what to do instead
            Self::Preset(path) => BgraImage::open_png(path).map(Arc::new).map_err(|error| {
                let name = file_name_text(path).unwrap_or_default();
                format!("The preset's screenshot {name} can't be read: {error}. Grab its split instead.")
            }),
            Self::Grabbed(image) => Ok(image.clone()),
            // At the size grabs are compared at, which a mask is stretched to
            Self::Fade(color) => Ok(Arc::new(BgraImage::solid(*color, 320, 240))),
        }
    }
}

/// The window's preset, and what's been done with it.
#[derive(Default)]
pub(super) struct Maker {
    presets: PresetList,
    /// What each preset has to have fixed before anything's made from it.
    problems: Vec<Vec<Problem>>,
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
    /// What Split Library adds from.
    library: library::Library,
    /// What the window that adds to or edits the split library is for, while it's open.
    entry: Option<library_entry::Entry>,
    /// The live capture's latest frame, for a base image for the split library when a split has
    /// no grab or screenshot.
    last_live: Option<Pixels>,
    /// Splits to save once the ones being saved are, for the preset of that generation.
    queued: Option<(u64, Vec<usize>)>,
    /// Every split is shown, with a checkbox for the ones the runner can turn on or off, which
    /// Choose Splits under the list starts and Done ends. Off, splits that are off aren't shown.
    choosing: bool,
    /// Splits turned off while split images were being saved, for the preset of that generation,
    /// whose images deleting waits for: one being written would come back after.
    turned_off: Option<(u64, Vec<usize>)>,
}

/// Where a split's image comes from, as far as its preset's screenshot goes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ScreenshotUse {
    /// The preset has no screenshot it can be made from.
    None,
    /// It's made from the preset's screenshot.
    FromPreset,
    /// It has a grab or picked file of its own over the preset's screenshot.
    Overridden,
}

/// What's been done for a split.
#[derive(Clone, Default)]
struct SplitWork {
    /// The grab or screenshot it was given. What its image is made from is [`Maker::source`], which
    /// for a fade split is its color, whatever this is: a split switched to a fade keeps it, for
    /// switching back.
    shot: Option<Source>,
    /// Its split image, once it's written.
    saved: Option<PathBuf>,
    /// What `saved` was made from. A fade split's color is in its preset, so changing it doesn't
    /// go through [`SplitWork::set_shot`].
    saved_from: Option<Source>,
    /// A small picture of its grab or screenshot, once it's made.
    thumb: Option<slint::Image>,
}

impl SplitWork {
    /// Makes its image from the grab or screenshot `shot` from now on: what was saved or pictured
    /// of the one before is another image.
    fn set_shot(&mut self, shot: Source) {
        *self = Self { shot: Some(shot), ..Self::default() };
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
            Some(edit) => Some(edit.preset()),
            None => self.loaded().map(|loaded| &loaded.preset),
        }
    }

    /// A file of the shown preset: relative to its folder, or picked while editing.
    fn file(&self, path: &str) -> PathBuf {
        let folder = match &self.edit {
            Some(edit) => edit.source(),
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

    /// What split `index`'s image is made from, if anything: a fade split's color, once it's
    /// written right, or else its grab or screenshot, or else the screenshot its preset has for it.
    fn source(&self, index: usize) -> Option<Source> {
        if let Some(split) = self.fade_split(index) {
            return split.fade_color().map(Source::Fade);
        }
        let shot = self.splits.get(index).and_then(|split| split.shot.clone());
        shot.or_else(|| self.usable_preset_screenshot(index).map(Source::Preset))
    }

    /// The screenshot the shown preset names for split `index`, if any, whether or not its file
    /// is there.
    fn preset_screenshot(&self, index: usize) -> Option<PathBuf> {
        let screenshot = &self.shown()?.splits.get(index)?.screenshot;
        (!screenshot.is_empty()).then(|| self.file(screenshot))
    }

    /// The preset's screenshot that split `index` can be made from: not a fade split's, which is
    /// made from its color, and not one whose file is missing, which the split is grabbed for
    /// instead, like a split without one.
    fn usable_preset_screenshot(&self, index: usize) -> Option<PathBuf> {
        if self.fade_split(index).is_some() {
            return None;
        }
        self.preset_screenshot(index).filter(|file| file.is_file())
    }

    /// Where split `index`'s image comes from, as far as its preset's screenshot goes.
    fn screenshot_use(&self, index: usize) -> ScreenshotUse {
        let own = self.splits.get(index).is_some_and(|split| split.shot.is_some());
        match (self.usable_preset_screenshot(index).is_some(), own) {
            (false, _) => ScreenshotUse::None,
            (true, false) => ScreenshotUse::FromPreset,
            (true, true) => ScreenshotUse::Overridden,
        }
    }

    /// Whether split `index` is off, where `enabled` is [`Gui::maker_enabled`]'s, so it gets no
    /// split image. Never while editing, which shows the preset's splits as they are.
    fn is_off(&self, enabled: &[bool], index: usize) -> bool {
        self.edit.is_none() && enabled.get(index) == Some(&false)
    }

    /// The splits that aren't off, in order, with `enabled` like [`Maker::is_off`].
    fn on_splits<'a>(&'a self, enabled: &'a [bool]) -> impl Iterator<Item = usize> + Clone + 'a {
        (0..self.splits.len()).filter(move |&index| !self.is_off(enabled, index))
    }

    /// How many of the shown preset's splits have a screenshot from it, and how many have to be
    /// grabbed. Fade splits are in neither, since they're made from their color, and splits
    /// that are off, with `enabled` like [`Maker::is_off`].
    fn preset_screenshot_counts(&self, enabled: &[bool]) -> (usize, usize) {
        let grabbed = self.on_splits(enabled).filter(|&index| self.fade_split(index).is_none());
        let with = grabbed.clone().filter(|&index| self.usable_preset_screenshot(index).is_some()).count();
        (with, grabbed.count() - with)
    }

    /// The splits made from their preset's screenshot that have no picture of it yet.
    fn unpictured_screenshots(&self) -> Vec<usize> {
        (0..self.splits.len())
            .filter(|&index| self.splits[index].thumb.is_none())
            .filter(|&index| self.screenshot_use(index) == ScreenshotUse::FromPreset)
            .collect()
    }

    /// The split a grab or Grab* goes on to after split `after`: the next one, past those whose
    /// image comes from their preset's screenshot, which grabbing would replace, and those that
    /// are off, with `enabled` like [`Maker::is_off`]. None when there's none.
    fn next_to_grab(&self, after: usize, enabled: &[bool]) -> Option<usize> {
        let skipped: Vec<bool> = (0..self.splits.len())
            .map(|index| self.screenshot_use(index) == ScreenshotUse::FromPreset || self.is_off(enabled, index))
            .collect();
        next_to_grab(&skipped, after)
    }

    /// Split `index` of the shown preset, when it's a fade split.
    fn fade_split(&self, index: usize) -> Option<&PresetSplit> {
        self.shown()?.splits.get(index).filter(|split| split.is_fade())
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
    /// Grab is the next thing to do for the selected split. Not when its preset has a screenshot
    /// for it, where Save All is.
    grab_suggested: bool,
    /// The selected split has a grab or picked file over its preset's screenshot, which Use
    /// Preset's drops.
    over_preset_screenshot: bool,
    can_save_selected: bool,
    can_save_all: bool,
    can_test: bool,
    needs_grouping: bool,
    working: bool,
    has_savestates: bool,
    has_instructions: bool,
    /// The chosen preset has splits the runner can turn on or off.
    has_choices: bool,
    /// How many of them are off, for the strip under the list.
    splits_off: usize,
    /// How many splits that are off still have their image in the split image folder.
    splits_kept: usize,
    /// How many rows are shown: splits that are off are only shown while choosing.
    shown_rows: usize,
    /// Every split is shown, with checkboxes, which Choose Splits starts. Never while editing.
    choosing: bool,
}

/// What the window shows for `maker`, whose splits have the file names `names`. `in_place` says
/// which are saved, `problems` which have something to fix, `preset_problems` are the chosen
/// preset's, and `grouping` is whether the profile compares images with the same number for one
/// split. `enabled` says which of the chosen preset's splits are on, and `kept` which of those
/// that are off still have their image in the split image folder.
#[allow(clippy::too_many_arguments)]
fn maker_view(
    maker: &Maker,
    names: &[String],
    enabled: &[bool],
    kept: &[bool],
    in_place: &[bool],
    problems: &[bool],
    preset_problems: &[Problem],
    grouping: bool,
) -> MakerView {
    let joined = |index| maker.shown().is_some_and(|preset| preset.joins_previous(index));
    // The chosen preset, while not editing, which the checkboxes are for
    let chosen = maker.loaded().filter(|_| maker.edit.is_none()).map(|loaded| &loaded.preset);
    // The split each one is on or off with, once, since each looks at the splits after it
    let leaders: Vec<Option<usize>> =
        (0..names.len()).map(|index| chosen.and_then(|preset| preset.leader(index))).collect();
    let toggleable = |index: usize| leaders.get(index) == Some(&Some(index));
    // Splits that are off are only shown while choosing
    let choosing = maker.choosing && chosen.is_some();
    let shown: Vec<bool> = (0..names.len()).map(|index| choosing || !maker.is_off(enabled, index)).collect();
    let places = row_places(&shown);
    let slot = |index: Option<usize>| index.map_or(-1, |index| index as i32);
    // Every other shown split is shaded, splits compared together as one
    let mut block = 0;
    let rows = (names.iter().enumerate())
        .map(|(index, name)| (index, name, thumb(maker, index)))
        .map(|(index, name, thumb)| (index, name, thumb, places[index], leaders[index]))
        .map(|(index, name, thumb, place, leader)| {
            // A checkbox's splits compared together: the first one's place, and how many they are
            let group = chosen.map_or(index..index + 1, |preset| preset.group(index));
            (index, name, thumb, place, leader, group)
        })
        .map(|(index, name, thumb, place, leader, group)| MakerRow {
            heading: maker.shown().map(|preset| split_heading(preset, index)).unwrap_or_default().into(),
            parts: file_name_parts(name),
            file: name.into(),
            source: maker.source(index).as_ref().map(Source::label).unwrap_or_default().into(),
            saved: in_place.get(index).copied().unwrap_or_default(),
            has_thumb: thumb.is_some(),
            thumb: thumb.unwrap_or_default(),
            problem: problems.get(index).copied().unwrap_or_default(),
            joined: joined(index),
            shaded: {
                if !place.hidden && place.place > 0 && !joined(index) {
                    block += 1;
                }
                block % 2 == 1
            },
            toggleable: toggleable(index),
            off: maker.is_off(enabled, index),
            // Not for one compared together with a checkbox's split, where the checkbox is
            fixed: chosen
                .filter(|_| !leader.is_some_and(|leader| group.contains(&leader)))
                .and_then(|preset| why_not_toggleable(preset, index))
                .unwrap_or_default()
                .into(),
            kept: kept.get(index).copied().unwrap_or_default(),
            hidden: place.hidden,
            place: place.place as i32,
            above: slot(place.above),
            below: slot(place.below),
            always_on: chosen.is_some() && leader.is_none(),
            leader: slot(leader),
            box_top: places[group.start].place as i32,
            box_rows: group.len() as i32,
        })
        .collect();
    // A preset with problems is only for fixing, so nothing's made from it
    let summary = maker.loaded().and_then(|loaded| problem_summary(&loaded.preset, preset_problems));
    let usable = summary.is_none();
    let selected = maker.selected;
    let selected_on = !maker.is_off(enabled, selected);
    let splits = maker.loaded().map_or(&[][..], |loaded| &loaded.preset.splits[..]);
    let on = || splits.iter().zip(enabled).filter(|(_, on)| **on).map(|(split, _)| split);
    MakerView {
        rows,
        preset_problem: summary.unwrap_or_default(),
        can_grab: !maker.splits.is_empty() && usable && selected_on,
        grab_suggested: maker.screenshot_use(selected) != ScreenshotUse::FromPreset,
        over_preset_screenshot: maker.screenshot_use(selected) == ScreenshotUse::Overridden,
        can_save_selected: maker.source(selected).is_some() && usable && selected_on,
        can_save_all: maker.on_splits(enabled).any(|index| maker.source(index).is_some()) && usable,
        can_test: in_place.get(selected).copied().unwrap_or_default() && usable,
        needs_grouping: !grouping && maker.loaded().is_some_and(|loaded| loaded.preset.needs_grouping(enabled)),
        working: maker.working > 0,
        has_savestates: usable && on().any(|split| !split.savestate.is_empty()),
        has_instructions: on().any(|split| !split.savestate.is_empty() && !split.instructions.is_empty()),
        has_choices: (0..names.len()).any(toggleable),
        splits_off: (0..names.len()).filter(|&index| toggleable(index) && maker.is_off(enabled, index)).count(),
        splits_kept: kept.iter().filter(|&&kept| kept).count(),
        shown_rows: shown.iter().filter(|&&shown| shown).count(),
        choosing,
    }
}

/// The list's picture of split `index`: its color for a fade split, or of its grab or screenshot.
fn thumb(maker: &Maker, index: usize) -> Option<slint::Image> {
    if let Some(split) = maker.fade_split(index) {
        // None while its color isn't written right
        let [red, green, blue] = split.fade_color()?;
        let (width, height) = THUMB_SIZE;
        let pixels = [red, green, blue, 255].repeat(width * height);
        return Some(slint::Image::from_rgba8(Pixels::clone_from_slice(&pixels, width as u32, height as u32)));
    }
    maker.splits.get(index)?.thumb.clone()
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

/// Whether this is a developer build, like from `cargo run`, which can change the split library
/// next to the program. Release builds don't show how.
const DEVELOPER: bool = cfg!(debug_assertions);

/// A folder the program comes with, like `presets`, next to it. Builds that aren't installed use
/// the source tree's.
fn bundled_folder(name: &str) -> PathBuf {
    let next_to_program = exe_folder().join(name);
    if next_to_program.is_dir() || !cfg!(debug_assertions) {
        return next_to_program;
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(name)
}

/// The folder with the bundled presets.
fn presets_folder() -> PathBuf {
    bundled_folder("presets")
}

/// Deletes the screenshots and savestates in the shared folders of `game_folder` that no preset
/// or library uses anymore, and none of `in_use`, which the preset being edited takes from the
/// library.
fn sweep_shared(game_folder: &Path, in_use: &[PathBuf]) {
    sweep_shared_files(game_folder, &presets_folder(), &library::splits_folder(), in_use);
}

impl Gui {
    pub(super) fn connect_maker(&self) {
        let weak = self.me.clone();
        let maker = &self.windows.maker;
        on!(weak, maker, on_game_chosen, (index) gui => gui.choose_game(index as usize));
        on!(weak, maker, on_preset_chosen, (index) gui => {
            let maker = gui.maker.borrow();
            let game = maker.loaded().map(|loaded| loaded.preset.game.clone()).unwrap_or_default();
            let chosen = maker.presets.presets_of(&game).get(index as usize).copied();
            drop(maker);
            if let Some(chosen) = chosen {
                gui.choose_preset(chosen);
            }
        });
        on!(weak, maker, on_split_chosen, (index) gui => gui.choose_split(index as usize));
        on!(weak, maker, on_grab, () gui => {
            // Made from its color, so it doesn't wait for the capture
            if gui.selected_fade() {
                gui.grab_fade();
            } else {
                gui.send(UiCommand::Grab);
            }
        });
        on!(weak, maker, on_use_screenshots, () gui => gui.pick_screenshots());
        on!(weak, maker, on_use_preset_screenshot, () gui => gui.use_preset_screenshot());
        on!(weak, maker, on_save_selected, () gui => {
            let selected = gui.maker.borrow().selected;
            gui.save_split_images(vec![selected]);
        });
        on!(weak, maker, on_save_all, () gui => {
            let enabled = gui.maker_enabled();
            let on = gui.maker.borrow().on_splits(&enabled).collect();
            gui.save_split_images(on);
        });
        on!(weak, maker, on_split_toggled, (index) gui => gui.toggle_split(index as usize));
        on!(weak, maker, on_choose_splits, () gui => gui.choose_splits());
        on!(weak, maker, on_test, () gui => gui.test_split_image());
        on!(weak, maker, on_copy_savestates, () gui => gui.copy_savestates());
        on!(weak, maker, on_show_instructions, () gui => gui.show_instructions());
        let instructions = &self.windows.instructions;
        on!(weak, instructions, on_card_clicked, (split) gui => gui.instruction_card_clicked(split as usize));
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
        self.connect_library();
        self.connect_library_entry();
        self.connect_sharing();
    }

    /// Opens Make Split Images beside the main window, with the presets read again.
    pub(super) fn open_maker(self: &Rc<Self>) {
        let window = self.windows.maker.window();
        if window.is_visible() {
            return self.show_window(window);
        }
        if self.tool_blocked_by_run() {
            return;
        }
        let folder = presets_folder();
        let before = self.maker.borrow().loaded().cloned();
        let errors = self.read_presets();
        let mut maker = self.maker.borrow_mut();
        // The preset from before, which presets added or removed meanwhile may have moved
        let found = before.as_ref().and_then(|before| maker.presets.find(&before.folder));
        let chosen = found.unwrap_or(maker.preset.min(maker.presets.len().saturating_sub(1)));
        // What was grabbed or picked for the same preset is kept, unless its splits changed
        let same = found.is_some()
            && before
                .zip(maker.presets.get(chosen))
                .is_some_and(|(before, now)| before.preset.splits == now.preset.splits);
        // Masks are read again, in case they were edited
        maker.masks.clear();
        // And the preset's screenshots pictured again
        for split in maker.splits.iter_mut().filter(|split| split.shot.is_none()) {
            split.thumb = None;
        }
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
        // Only reading them again changes them
        maker.problems = presets.iter().map(|loaded| problems_of(&loaded.preset)).collect();
        maker.presets = PresetList::new(presets);
        let games: Vec<SharedString> = maker.presets.games().into_iter().map(game_label).collect();
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
            gui.tool_closed(Tool::Maker);
            let _ = gui.windows.maker.hide();
        });
    }

    /// Make Split Images closes: the capture is only needed for it while it's open.
    /// Make Split Images closed, which `tool_closed` says.
    pub(super) fn maker_closed(&self) {
        // They're the window's preset's
        let _ = self.windows.instructions.window().hide();
        // It shows no capture anymore, so its last frame gets old
        self.maker.borrow_mut().last_live = None;
        self.send(UiCommand::SetMakerOpen(false));
        self.maker_seen.hidden();
    }

    /// The first preset of the `index`th game.
    fn choose_game(&self, index: usize) {
        let maker = self.maker.borrow();
        let first = maker.presets.games().get(index).and_then(|game| maker.presets.presets_of(game).first().copied());
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
        self.show_screenshots_status();
        self.show_preset(index);
    }

    /// Shows the `index`th preset, its game and the game's presets in the lists, and what's
    /// been done with its splits, with the selected split, or the next one shown when it's hidden,
    /// like after the preset was saved with the selected split turned off.
    fn show_preset(&self, index: usize) {
        let mut maker = self.maker.borrow_mut();
        maker.preset = index;
        let game = maker.loaded().map(|loaded| loaded.preset.game.clone()).unwrap_or_default();
        let game_index = maker.presets.game_index(&game).unwrap_or(0);
        let of_game = maker.presets.presets_of(&game);
        let names: Vec<SharedString> = (of_game.iter())
            .filter_map(|&each| maker.presets.get(each))
            .map(|loaded| loaded.preset.name.as_str().into())
            .collect();
        let window = &self.windows.maker;
        window.set_game_index(game_index as i32);
        window.set_presets(ModelRc::new(VecModel::from(names)));
        window.set_preset_index(of_game.iter().position(|&each| each == index).unwrap_or(0) as i32);
        drop(maker);
        self.picture_preset_screenshots();
        self.choose_split(self.shown_selection());
    }

    /// The selected split, or when it's hidden, since it's off and not being chosen, the next split
    /// that's on, or the last one before it. The first split that's on for a preset just chosen,
    /// which the screenshot hotkey's first press grabs.
    fn shown_selection(&self) -> usize {
        let enabled = self.maker_enabled();
        let maker = self.maker.borrow();
        let selected = maker.selected;
        if maker.choosing || !maker.is_off(&enabled, selected) {
            return selected;
        }
        let on: Vec<usize> = maker.on_splits(&enabled).collect();
        on.iter().find(|&&index| index > selected).or(on.last()).copied().unwrap_or(selected)
    }

    /// Makes the list's pictures of the splits made from their preset's screenshot, which no
    /// grab or pick asked for.
    fn picture_preset_screenshots(&self) {
        let unpictured = self.maker.borrow().unpictured_screenshots();
        if !unpictured.is_empty() {
            self.make_thumbnails(unpictured);
        }
    }

    fn choose_split(&self, index: usize) {
        self.maker.borrow_mut().selected = index;
        self.windows.maker.set_selected(index as i32);
        self.maker.borrow_mut().live_mask = None;
        self.show_form();
        self.show_maker();
        // Once the list has its rows, so a split just added or moved isn't left out of view
        self.windows.maker.invoke_scroll_to(index as i32);
        self.refresh_instructions();
    }

    /// Whether the selected split is a fade split, which Grab saves from its color.
    fn selected_fade(&self) -> bool {
        let maker = self.maker.borrow();
        maker.fade_split(maker.selected).is_some()
    }

    /// Why Grab can't work yet, which its tooltip says, or empty. The hotkey still tries, and says
    /// so in a message. A fade split doesn't need the capture.
    fn grab_blocker(&self) -> String {
        if let Some(off) = self.selected_off() {
            off
        } else if !self.selected_fade() && !self.state.borrow().document.profile.has_capture_device() {
            concat!(
                "Pick a capture device in Settings (",
                primary_key!(),
                "+,) first, which Grab takes the capture region from."
            )
            .to_owned()
        } else if !self.split_image_folder().is_some_and(|folder| folder.is_dir()) {
            format!("Pick a split image folder that exists with {} first, which Grab saves into.", self.folder_button())
        } else {
            String::new()
        }
    }

    /// Why the selected split gets no split image when it's off, naming the split to turn on for
    /// one that goes with another, like a dummy, or `None` when it's on.
    fn selected_off(&self) -> Option<String> {
        let enabled = self.maker_enabled();
        let maker = self.maker.borrow();
        let selected = maker.selected;
        if !maker.is_off(&enabled, selected) {
            return None;
        }
        let preset = &maker.loaded()?.preset;
        Some(match preset.leader(selected).filter(|&leader| leader != selected) {
            Some(leader) => {
                let name = split_heading(preset, leader);
                format!(
                    "This split is off with {name}, so it gets no split image. Turn {name} on with Choose Splits to make one."
                )
            }
            None => {
                "This split is off, so it gets no split image. Turn it on with Choose Splits to make one.".to_owned()
            }
        })
    }

    /// Choose Splits or Done: shows every split with checkboxes, or only the splits that are on.
    /// Done moves the selection off a split that's hidden then, to the next one shown.
    fn choose_splits(&self) {
        let choosing = !self.maker.borrow().choosing;
        self.maker.borrow_mut().choosing = choosing;
        // Scrolled into view, since the rows above it show or hide
        self.choose_split(self.shown_selection());
    }

    /// Turns split `index`, with the splits that follow it, on or off in the profile's choice for
    /// the chosen preset. A choice that's the same as the preset again is dropped.
    fn toggle_split(self: &Rc<Self>, index: usize) {
        let enabled = self.maker_enabled();
        let Some(&on) = enabled.get(index) else { return };
        let maker = self.maker.borrow();
        let Some(loaded) = maker.loaded().filter(|_| maker.edit.is_none()) else { return };
        let mut state = self.state.borrow_mut();
        let choices = &mut state.document.profile.split_choices;
        let key = loaded.preset.choice_key();
        let choice = choices.entry(key.clone()).or_default();
        loaded.preset.choose(choice, index, !on);
        if *choice == SplitChoice::default() {
            choices.remove(&key);
        }
        // The split that was turned on or off, also from Space on a split that goes with it
        let name = split_heading(&loaded.preset, loaded.preset.leader(index).unwrap_or(index));
        drop((state, maker));
        self.profile_edited();
        // The timer's splits have to match, or the run splits on the wrong segments
        let status = if on {
            format!("{name} is off. Take its segment out of your LiveSplit splits too.")
        } else {
            format!("{name} is on. Add a segment for it to your LiveSplit splits too.")
        };
        self.windows.maker.set_status(status.into());
        self.show_maker();
        // Splits that are off have no instructions to show
        self.refresh_instructions();
        if on {
            let after = self.maker_enabled();
            let turned_off = (0..after.len()).filter(|&index| enabled[index] && !after[index]).collect();
            self.offer_to_delete_off_images(turned_off);
        }
    }

    /// The images in the split image folder of those of the chosen preset's splits `indexes` that
    /// are off, by split, which AutoSplit would still compare. Splits without any aren't listed.
    fn off_images_in_folder(&self, indexes: &[usize]) -> Vec<(usize, Vec<PathBuf>)> {
        let Some(folder) = self.split_image_folder().filter(|folder| folder.is_dir()) else { return Vec::new() };
        let enabled = self.maker_enabled();
        let maker = self.maker.borrow();
        let Some(loaded) = maker.loaded() else { return Vec::new() };
        let (numbers, _) = loaded.preset.numbers();
        let off: Vec<usize> = indexes.iter().copied().filter(|&index| maker.is_off(&enabled, index)).collect();
        if off.is_empty() {
            return Vec::new();
        }
        let existing = user_files(&folder).unwrap_or_default();
        (off.into_iter())
            .filter_map(|index| {
                let split = (numbers.get(index)?, loaded.preset.splits.get(index)?.name.as_str());
                let images = images_of_splits(&existing, &[(*split.0, split.1)]);
                (!images.is_empty()).then_some((index, images))
            })
            .collect()
    }

    /// Asks whether to delete the split images in the split image folder of the splits
    /// `turned_off`, which AutoSplit would still compare. Keeping them is the default, like for
    /// other deletes. While split images are being saved, it waits until they are.
    fn offer_to_delete_off_images(self: &Rc<Self>, turned_off: Vec<usize>) {
        let mut maker = self.maker.borrow_mut();
        if maker.saving {
            let generation = maker.generation;
            let waiting = maker.turned_off.get_or_insert_with(|| (generation, Vec::new()));
            // Ones turned off in a preset chosen before don't count, like for queued saves
            if waiting.0 != generation {
                *waiting = (generation, Vec::new());
            }
            // Each once, so turning one off, on and off again doesn't list its images twice
            for index in turned_off {
                if !waiting.1.contains(&index) {
                    waiting.1.push(index);
                }
            }
            return;
        }
        drop(maker);
        // Not ones turned back on while a save was waited for
        let images: Vec<PathBuf> =
            self.off_images_in_folder(&turned_off).into_iter().flat_map(|(_, images)| images).collect();
        if images.is_empty() {
            return;
        }
        let names: Vec<String> = images.iter().filter_map(|path| file_name_text(path)).collect();
        let text = format!(
            "The split image folder has images of the splits turned off, which AutoSplit would still compare:\n{}\n\n\
             Delete them?",
            names.join("\n")
        );
        let weak = Rc::downgrade(self);
        // Enter keeps them: deleting is never the default
        let labels = ["Delete Images", "Keep Images", ""];
        self.show_message(Message::choice(text, labels, 1, move |answer| {
            let Some(gui) = weak.upgrade().filter(|_| answer == Answer::Yes) else { return };
            let errors: Vec<String> = (images.iter())
                .filter_map(|path| {
                    let error = std::fs::remove_file(path).err()?;
                    Some(format!("Couldn't delete {}: {error}", path.display()))
                })
                .collect();
            let deleted = images.len() - errors.len();
            gui.windows.maker.set_status(format!("Deleted {}.", plural(deleted, "split image")).into());
            gui.show_maker();
            if !errors.is_empty() {
                gui.show_message(Message::error(errors.join("\n")));
            }
        }));
    }

    /// What the split image folder's button is called in the messages: Browse..., or the folder
    /// icon that's there instead without advanced.
    pub(super) fn folder_button(&self) -> &'static str {
        if self.state.borrow().document.profile.advanced_maker { "Browse..." } else { "the folder button" }
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
        let maker = self.maker.borrow();
        // A fade split saved in another color isn't its split image anymore
        let saved: Vec<Option<PathBuf>> = (maker.splits.iter().enumerate())
            .map(|(index, split)| {
                let current = split.saved_from.as_ref().zip(maker.source(index));
                split.saved.clone().filter(|_| current.is_some_and(|(from, now)| from.same(&now)))
            })
            .collect();
        drop(maker);
        saves_in_place(&saved, names, folder.as_deref())
    }

    /// Whether each split of the chosen preset is on, with the profile's choice for it.
    pub(super) fn maker_enabled(&self) -> Vec<bool> {
        let maker = self.maker.borrow();
        let Some(loaded) = maker.loaded() else { return Vec::new() };
        let state = self.state.borrow();
        let choice = state.document.profile.split_choices.get(&loaded.preset.choice_key());
        loaded.preset.enabled(choice.unwrap_or(&SplitChoice::default()))
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
        let enabled = self.maker_enabled();
        // Only splits that are off are looked for, so the folder isn't read without any
        let mut kept = vec![false; names.len()];
        for (index, _) in self.off_images_in_folder(&(0..names.len()).collect::<Vec<_>>()) {
            kept[index] = true;
        }
        let view =
            maker_view(&self.maker.borrow(), &names, &enabled, &kept, &in_place, &problems, &preset_problems, grouping);
        let grab_blocker = self.grab_blocker();
        self.maker_rows.set_vec(view.rows);
        let window = &self.windows.maker;
        // Empty without one, which the window says
        let folder = self.split_image_folder();
        window.set_output_folder(folder.map(|folder| folder.display().to_string()).unwrap_or_default().into());
        window.set_advanced(self.state.borrow().document.profile.advanced_maker);
        window.set_preset_problem(view.preset_problem.into());
        window.set_can_grab(view.can_grab && grab_blocker.is_empty());
        window.set_grab_blocker(grab_blocker.into());
        window.set_grab_fade(self.selected_fade());
        window.set_can_save_selected(view.can_save_selected);
        window.set_grab_suggested(view.grab_suggested);
        window.set_over_preset_screenshot(view.over_preset_screenshot);
        window.set_can_save_all(view.can_save_all);
        window.set_can_test(view.can_test);
        window.set_needs_grouping(view.needs_grouping);
        window.set_working(view.working);
        window.set_has_savestates(view.has_savestates);
        window.set_has_instructions(view.has_instructions);
        window.set_has_choices(view.has_choices);
        // Splits that are off aren't shown, so the strip says when one is still compared
        window.set_choice_summary(choice_summary(view.splits_off, view.splits_kept).into());
        window.set_choice_kept(view.splits_kept > 0);
        window.set_shown_rows(view.shown_rows as i32);
        window.set_choosing_splits(view.choosing);
        self.show_maker_preview();
        self.refresh_library();
        self.refresh_entry();
    }

    /// What the editor would ask to fix in the chosen preset, like splits compared together
    /// with the same name in a preset written by hand.
    fn preset_problems(&self) -> Vec<Problem> {
        let maker = self.maker.borrow();
        maker.problems.get(maker.preset).cloned().unwrap_or_default()
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
        self.show_message(Message::warning(format!("Fix the preset with Fix It first. {summary}")));
        true
    }

    /// The session's capture region, for the selected split: its split image is written, and the
    /// next split is selected.
    pub(super) fn grabbed(self: &Rc<Self>, image: Result<BgraImage, String>) {
        if !self.windows.maker.window().is_visible() {
            return;
        }
        // Like from the screenshot hotkey: it's made from its color
        if self.selected_fade() {
            return self.grab_fade();
        }
        // The split's file name and mask aren't settled until the preset is saved
        if self.editing() {
            return self.show_message(Message::warning("Save or cancel the changes to the preset first, then grab."));
        }
        if self.refused_for_problems() || self.refused_for_off() {
            return;
        }
        let image = match image {
            Ok(image) => Arc::new(image),
            Err(error) => return self.show_message(Message::error(error)),
        };
        let mut maker = self.maker.borrow_mut();
        let selected = maker.selected;
        let Some(split) = maker.splits.get_mut(selected) else { return };
        split.set_shot(Source::Grabbed(image));
        drop(maker);
        self.make_thumbnails(vec![selected]);
        // Stays on the split when its image can't be saved, to grab it again once that's fixed
        self.after_grab(selected, self.save_split_images(vec![selected]));
    }

    /// Grab on a fade split: its split image is written from its color, and the next split is
    /// selected, like after a grab.
    fn grab_fade(self: &Rc<Self>) {
        if self.editing() {
            return self.show_message(Message::warning("Save or cancel the changes to the preset first, then grab."));
        }
        if self.refused_for_problems() || self.refused_for_off() {
            return;
        }
        let selected = self.maker.borrow().selected;
        self.after_grab(selected, self.save_split_images(vec![selected]));
    }

    /// Says the selected split is off, for a grab, like from the screenshot hotkey, which Grab's
    /// tooltip says too. Returns whether it is.
    fn refused_for_off(&self) -> bool {
        let Some(off) = self.selected_off() else { return false };
        self.show_message(Message::warning(off));
        true
    }

    /// After a grab or Grab* of split `grabbed`, selects the next split to grab, when its image
    /// is being `saved`. It skips splits made from their preset's screenshot, which the next press
    /// of the screenshot hotkey would replace while playing through. Without one, it stays.
    fn after_grab(&self, grabbed: usize, saved: bool) {
        let enabled = self.maker_enabled();
        let next = self.maker.borrow().next_to_grab(grabbed, &enabled).filter(|_| saved);
        match next {
            Some(next) => self.choose_split(next),
            None => self.show_maker(),
        }
    }

    /// Asks for screenshots: one for the selected split, or several for the splits in order from
    /// the first, like AutoSplit numbers its screenshots. Fade splits don't use any.
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
        let enabled = self.maker_enabled();
        let mut maker = self.maker.borrow_mut();
        if paths.len() == 1 && maker.fade_split(maker.selected).is_some() {
            drop(maker);
            return self.windows.maker.set_status("Fade splits are made from their color, not a screenshot.".into());
        }
        if paths.len() == 1 && maker.is_off(&enabled, maker.selected) {
            drop(maker);
            return self.windows.maker.set_status("The selected split is off, so it gets no split image.".into());
        }
        // Fade splits are made from their color, so screenshots go to the others in order, past
        // the ones that are off, which were never split on
        let (takes_screenshot, fades): (Vec<usize>, Vec<usize>) =
            maker.on_splits(&enabled).partition(|&index| maker.fade_split(index).is_none());
        let (off, fades) = (maker.splits.len() - takes_screenshot.len() - fades.len(), fades.len());
        // Without splits, like with no presets or all of them removed, there's no selected one
        let selected = (maker.selected < maker.splits.len()).then_some(maker.selected);
        let count = takes_screenshot.len();
        let targets = if paths.len() == 1 { selected.into_iter().collect() } else { takes_screenshot };
        let mut set = Vec::new();
        for (&index, path) in targets.iter().zip(&paths) {
            maker.splits[index].set_shot(Source::File(path.clone()));
            set.push(index);
        }
        drop(maker);
        self.make_thumbnails(set);
        // Unlike a grab, they're saved when asked
        let status = match paths.len() {
            // Save Selected is only there with advanced
            1 => format!(
                "Screenshot set for the selected split. {} writes its split image.",
                if self.state.borrow().document.profile.advanced_maker { "Save Selected" } else { "Save All" }
            ),
            picked => screenshots_set_status(picked, count, fades, off),
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

    /// Drops the selected split's grab or picked file, so its image is made from its preset's
    /// screenshot again. Like a picked file, it's saved when asked.
    fn use_preset_screenshot(&self) {
        let mut maker = self.maker.borrow_mut();
        if maker.screenshot_use(maker.selected) != ScreenshotUse::Overridden {
            return;
        }
        let selected = maker.selected;
        maker.splits[selected] = SplitWork::default();
        // The status line is hidden while editing, where the previews show the change instead
        let editing = maker.edit.is_some();
        drop(maker);
        self.picture_preset_screenshots();
        if !editing {
            let save = if self.state.borrow().document.profile.advanced_maker { "Save Selected" } else { "Save All" };
            let status =
                format!("Back to the preset's screenshot for the selected split. {save} writes its split image.");
            self.windows.maker.set_status(status.into());
        }
        self.show_maker();
    }

    /// Another split image folder was picked: the split images saved before are in the old one.
    fn maker_folder_changed(&self) {
        // Picking the folder comes right after the preset, so what it said about screenshots stays
        self.show_screenshots_status();
        self.show_maker();
    }

    /// Says how many of the preset's splits have a screenshot and how many are left to grab, or
    /// nothing for a preset without screenshots.
    fn show_screenshots_status(&self) {
        let enabled = self.maker_enabled();
        let (with, without) = self.maker.borrow().preset_screenshot_counts(&enabled);
        self.windows.maker.set_status(screenshots_status(with, without).unwrap_or_default().into());
    }

    /// Saves the profile as a new file named for the preset, which is loaded from then on like
    /// Save Profile As does.
    fn generate_profile(self: &Rc<Self>) {
        if self.refused_for_problems() {
            return;
        }
        if self.split_image_folder().is_none() {
            let message = format!("Pick a split image folder with {} first.", self.folder_button());
            return self.show_message(Message::warning(message));
        }
        let maker = self.maker.borrow();
        let Some(loaded) = maker.loaded() else { return };
        let name = loaded.preset.profile_file_name();
        let needs_grouping = loaded.preset.needs_grouping(&self.maker_enabled());
        drop(maker);
        let folder = self.state.borrow().document.folder(&exe_folder());
        let dialog = self
            .file_dialog_over("Generate Profile", self.windows.maker.window())
            .set_directory(folder)
            .set_file_name(name)
            .add_filter("TOML", &["toml"])
            .save_file();
        self.when_picked(dialog, move |gui, path| {
            let mut profile = gui.state.borrow().document.to_save();
            // The folder needs it, which the current profile gets only if this one is loaded
            profile.group_split_images |= needs_grouping;
            if let Err(error) = profile.save(&path) {
                return gui.show_message(Message::error(format!("Couldn't save {}: {error}", path.display())));
            }
            let name = file_name_text(&path).unwrap_or_default();
            // Written over the current profile's file, which is what's loaded now
            let current = gui.state.borrow().document.path.as_deref().map(canonical);
            if current == Some(canonical(&path)) {
                return gui.use_generated_profile(path, profile);
            }
            gui.windows.maker.set_status(format!("Saved the profile as {name}.").into());
            let text = format!(
                "Load the generated profile {name}? The main window uses it from now on, like after Load Profile. \
                 Otherwise the current profile stays loaded."
            );
            let weak = Rc::downgrade(gui);
            gui.show_message(Message::choice(text, ["Load It", "", "Keep Current"], 0, move |answer| {
                if let (Some(gui), Answer::Yes) = (weak.upgrade(), answer) {
                    gui.use_generated_profile(path, profile);
                }
            }));
        });
    }

    /// Loads the profile Generate Profile wrote to `path`, like Load Profile does.
    fn use_generated_profile(&self, path: PathBuf, profile: UserProfile) {
        let name = file_name_text(&path).unwrap_or_default();
        self.state.borrow_mut().document = Document::new(profile, Some(path));
        self.send_profile(true);
        self.show_profile();
        self.tester_follows_split_folder();
        self.windows.maker.set_status(format!("Saved and loaded the profile {name}.").into());
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
        self.tester_state.borrow_mut().path = Some(path);
        if self.windows.tester.window().is_visible() {
            self.load_tester_image();
            self.show_window(self.windows.tester.window());
        } else {
            self.open_tester();
        }
    }
}
