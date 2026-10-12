//! The Make Split Images window: a preset's masks applied to the capture or to screenshots, saved
//! into the split image folder with the names AutoSplit reads.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use autosplit_core::filename::name_parts;
use autosplit_core::image::BgraImage;
use autosplit_core::imgproc::ResizeBuffers;
use autosplit_core::mask::{Mask, apply_mask};
use autosplit_core::preset::{LoadedPreset, load_presets};
use autosplit_core::split_folder::user_files;
use slint::{ComponentHandle, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, VecModel};

use super::maker_plan::plan_writes;
use super::messages::{Answer, Message};
use super::placement::beside;
use super::ui::{InstructionCard, MakerRow};
use super::view::{Pixels, rgba};
use super::{Gui, Tool, exe_folder, file_name_parts, with_gui};
use crate::session::UiCommand;

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
            Self::File(path) => path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default(),
            Self::Grabbed(_) => "grabbed from the capture".into(),
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
    /// What each split's image is made from.
    sources: Vec<Option<Source>>,
    /// Each split's split image, once it's written.
    saved: Vec<Option<PathBuf>>,
    selected: usize,
    /// Masks by path, read once.
    masks: HashMap<PathBuf, Arc<Mask>>,
    /// The selected split's mask at the size of the capture's preview, with that size, for the
    /// live preview of its split image.
    live_mask: Option<((usize, usize), Vec<u8>)>,
}

impl Maker {
    fn loaded(&self) -> Option<&LoadedPreset> {
        self.presets.get(self.preset)
    }
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

/// A split image to write: what it's made from, its mask and its file.
struct Job {
    index: usize,
    source: Source,
    mask: Arc<Mask>,
    target: PathBuf,
}

impl Job {
    fn run(&self) -> Result<(), String> {
        let shot = self.source.image()?;
        let image = apply_mask(shot.view(), &self.mask, &mut ResizeBuffers::default());
        let png = image.encode_png().map_err(|error| format!("Couldn't encode {}: {error}", self.target.display()))?;
        std::fs::write(&self.target, png).map_err(|error| format!("Couldn't write {}: {error}", self.target.display()))
    }
}

/// Moves screenshots that are in the split image folder into its `base` folder, where the
/// split image loader doesn't look. Returns how many it moved.
fn move_to_base(folder: &Path, sources: &[PathBuf]) -> Result<usize, String> {
    let folder = std::fs::canonicalize(folder).map_err(|error| error.to_string())?;
    let mut moved = 0;
    for source in sources {
        let in_folder = source.parent().and_then(|parent| std::fs::canonicalize(parent).ok()) == Some(folder.clone());
        let Some(name) = source.file_name().filter(|_| in_folder && source.is_file()) else { continue };
        let base = folder.join("base");
        std::fs::create_dir_all(&base).map_err(|error| error.to_string())?;
        std::fs::rename(source, base.join(name)).map_err(|error| error.to_string())?;
        moved += 1;
    }
    Ok(moved)
}

impl Gui {
    pub(super) fn connect_maker(&self) {
        let weak = self.me.clone();
        let maker = &self.windows.maker;
        on!(weak, maker, on_preset_chosen, (index) gui => gui.choose_preset(index as usize));
        on!(weak, maker, on_split_chosen, (index) gui => gui.choose_split(index as usize));
        on!(weak, maker, on_grab, () gui => gui.send(UiCommand::Grab));
        on!(weak, maker, on_use_screenshots, () gui => gui.pick_screenshots());
        on!(weak, maker, on_save_selected, () gui => {
            let selected = gui.maker.borrow().selected;
            gui.save_split_images(vec![selected]);
        });
        on!(weak, maker, on_save_all, () gui => {
            let all = (0..gui.maker.borrow().sources.len()).collect();
            gui.save_split_images(all);
        });
        on!(weak, maker, on_test, () gui => gui.test_split_image());
        on!(weak, maker, on_copy_savestates, () gui => gui.copy_savestates());
        on!(weak, maker, on_show_instructions, () gui => gui.show_instructions());
        on!(always weak, maker, on_blocked_clicked, () gui => gui.focus_message());
        on!(always weak, maker.window(), on_close_requested, () gui => {
            gui.maker_closed();
        } else slint::CloseRequestResponse::HideWindow);
    }

    /// Opens Make Split Images beside the main window, with the presets read again.
    pub(super) fn open_maker(self: &Rc<Self>) {
        let window = self.windows.maker.window();
        if window.is_visible() {
            return self.show_window(window);
        }
        let (presets, errors) = load_presets(&presets_folder());
        let names: Vec<SharedString> = presets
            .iter()
            .map(|loaded| match loaded.preset.game.as_str() {
                "" => loaded.preset.name.clone().into(),
                game => format!("{game}: {}", loaded.preset.name).into(),
            })
            .collect();
        let chosen = self.maker.borrow().preset.min(presets.len().saturating_sub(1));
        self.maker.borrow_mut().presets = presets;
        let maker = &self.windows.maker;
        maker.set_presets(ModelRc::new(VecModel::from(names)));
        maker.set_preset_index(chosen as i32);
        maker.set_rows(ModelRc::from(self.maker_rows.clone()));
        maker.set_grid_image(transparency_grid(320, 240));
        self.choose_preset(chosen);
        self.place_by_main(window, beside);
        self.show_window(window);
        self.tool_opened(Tool::Maker);
        self.send(UiCommand::SetMakerOpen(true));
        self.update_visibility();
        if !errors.is_empty() {
            let errors: Vec<String> = errors.iter().map(ToString::to_string).collect();
            self.show_message(Message::warning(format!("Some presets couldn't be read:\n\n{}", errors.join("\n\n"))));
        }
    }

    /// The main window's Close button.
    pub(super) fn close_maker(&self) {
        self.maker_closed();
        let _ = self.windows.maker.hide();
    }

    /// Make Split Images closes: the capture is only needed for it while it's open.
    fn maker_closed(&self) {
        self.tool_closed(Tool::Maker);
        self.send(UiCommand::SetMakerOpen(false));
        self.maker_visible.store(false, std::sync::atomic::Ordering::Relaxed);
    }

    fn choose_preset(&self, index: usize) {
        let mut maker = self.maker.borrow_mut();
        maker.preset = index;
        let count = maker.loaded().map_or(0, |loaded| loaded.preset.splits.len());
        (maker.sources, maker.saved, maker.selected) = (vec![None; count], vec![None; count], 0);
        drop(maker);
        self.windows.maker.set_status(SharedString::new());
        self.choose_split(0);
    }

    fn choose_split(&self, index: usize) {
        self.maker.borrow_mut().selected = index;
        self.windows.maker.set_selected(index as i32);
        self.maker.borrow_mut().live_mask = None;
        self.show_maker();
    }

    /// The split image folder, as the profile says.
    fn split_image_folder(&self) -> Option<PathBuf> {
        let state = self.state.borrow();
        let directory = &state.document.profile.split_image_directory;
        (!directory.is_empty()).then(|| state.document.folder(&exe_folder()).join(directory))
    }

    /// The preset's file names, with the profile's defaults for what they leave out.
    fn maker_file_names(&self) -> Vec<String> {
        let defaults = self.state.borrow().document.profile.split_defaults();
        self.maker.borrow().loaded().map(|loaded| loaded.preset.file_names(Some(&defaults))).unwrap_or_default()
    }

    /// Puts the preset's splits, the selected split's split image and the buttons in the window.
    fn show_maker(&self) {
        let names = self.maker_file_names();
        let maker = self.maker.borrow();
        let rows: Vec<MakerRow> = names
            .iter()
            .enumerate()
            .map(|(index, name)| MakerRow {
                parts: file_name_parts(name),
                file: name.into(),
                source: maker.sources[index].as_ref().map(Source::label).unwrap_or_default().into(),
                saved: maker.saved[index].is_some(),
            })
            .collect();
        let selected = maker.selected;
        let has_source = maker.sources.get(selected).is_some_and(Option::is_some);
        let any_source = maker.sources.iter().any(Option::is_some);
        let saved = maker.saved.get(selected).is_some_and(Option::is_some);
        let has_splits = !maker.sources.is_empty();
        drop(maker);
        self.maker_rows.set_vec(rows);
        let window = &self.windows.maker;
        let folder = self.split_image_folder();
        window.set_output_folder(match &folder {
            Some(folder) => folder.display().to_string().into(),
            None => "no split image folder, set one in the main window".into(),
        });
        window.set_can_grab(has_splits);
        window.set_can_save_selected(has_source);
        window.set_can_save_all(any_source);
        window.set_can_test(saved);
        let maker = self.maker.borrow();
        let splits = maker.loaded().map_or(&[][..], |loaded| &loaded.preset.splits[..]);
        window.set_has_savestates(splits.iter().any(|split| !split.savestate.is_empty()));
        window.set_has_instructions(
            splits.iter().any(|split| !split.savestate.is_empty() && !split.instructions.is_empty()),
        );
        drop(maker);
        self.show_maker_preview();
    }

    /// The mask of split `index`, read once.
    fn maker_mask(&self, index: usize) -> Result<Arc<Mask>, String> {
        let mut maker = self.maker.borrow_mut();
        let Some(path) = maker.loaded().and_then(|loaded| Some(loaded.file(&loaded.preset.splits.get(index)?.mask)))
        else {
            return Err("There's no preset.".into());
        };
        if let Some(mask) = maker.masks.get(&path) {
            return Ok(mask.clone());
        }
        let mask = Arc::new(Mask::open(&path).map_err(|error| format!("Couldn't read {}: {error}", path.display()))?);
        maker.masks.insert(path, mask.clone());
        Ok(mask)
    }

    /// The selected split's split image, or its mask when nothing's set to make it from.
    fn show_maker_preview(&self) {
        let maker = self.maker.borrow();
        let selected = maker.selected;
        let source = maker.sources.get(selected).cloned().flatten();
        let split =
            maker.loaded().and_then(|loaded| loaded.preset.splits.get(selected)).map(|split| split.name.clone());
        drop(maker);
        let window = &self.windows.maker;
        let Some(split) = split else {
            window.set_preview_caption(SharedString::new());
            window.set_preview_message("The preset has no splits.".into());
            return;
        };
        let Some(source) = source else {
            // The capture's preview fills it in, masked, as frames come
            window.set_preview_caption(format!("{split}: preview of the capture, nothing grabbed yet").into());
            let message = window.get_live_message();
            window.set_preview_message(message);
            return;
        };
        let shown = self.maker_mask(selected).and_then(|mask| {
            let shot = source.image()?;
            Ok(apply_mask(shot.view(), &mask, &mut ResizeBuffers::default()))
        });
        match shown {
            Ok(image) => {
                window.set_preview_image(slint::Image::from_rgba8(rgba(image.view())));
                window.set_preview_message(SharedString::new());
                window.set_preview_caption(format!("{split}: split image").into());
            }
            Err(error) => {
                window.set_preview_message(error.into());
                window.set_preview_caption(split.into());
            }
        }
    }

    /// Whether the selected split's split image is previewed from the capture: nothing's set to
    /// make it from yet.
    fn previews_capture(&self) -> bool {
        let maker = self.maker.borrow();
        maker.sources.get(maker.selected).is_some_and(Option::is_none)
    }

    /// A frame of the capture's preview: until something's set to make the selected split's
    /// split image from, it's previewed from the capture, masked like a grab would be.
    pub(super) fn maker_live_frame(&self, pixels: &Pixels) {
        if !self.previews_capture() {
            return;
        }
        let size = (pixels.width() as usize, pixels.height() as usize);
        if self.maker.borrow().live_mask.as_ref().is_none_or(|(at, _)| *at != size) {
            let selected = self.maker.borrow().selected;
            let Ok(mask) = self.maker_mask(selected) else { return };
            let alpha = mask.scaled(size.0, size.1, &mut ResizeBuffers::default());
            self.maker.borrow_mut().live_mask = Some((size, alpha));
        }
        let maker = self.maker.borrow();
        let Some((_, alpha)) = &maker.live_mask else { return };
        let mut masked = pixels.clone();
        for (pixel, &alpha) in masked.make_mut_slice().iter_mut().zip(alpha) {
            if alpha != 255 {
                *pixel = Default::default();
            }
        }
        drop(maker);
        let window = &self.windows.maker;
        window.set_preview_image(slint::Image::from_rgba8(masked));
        window.set_preview_message(SharedString::new());
    }

    /// The capture's preview shows a message instead, which the split image's preview shows too
    /// while it's previewed from the capture.
    pub(super) fn maker_live_message(&self, message: &str) {
        if self.previews_capture() {
            self.windows.maker.set_preview_message(message.into());
        }
    }

    /// The session's capture region, for the selected split: its split image is written, and the
    /// next split is selected.
    pub(super) fn grabbed(self: &Rc<Self>, image: Result<BgraImage, String>) {
        if !self.windows.maker.window().is_visible() {
            return;
        }
        let image = match image {
            Ok(image) => Arc::new(image),
            Err(error) => return self.show_message(Message::error(error)),
        };
        let mut maker = self.maker.borrow_mut();
        let selected = maker.selected;
        let count = maker.sources.len();
        let Some(source) = maker.sources.get_mut(selected) else { return };
        *source = Some(Source::Grabbed(image));
        maker.saved[selected] = None;
        drop(maker);
        self.save_split_images(vec![selected]);
        if selected + 1 < count {
            self.choose_split(selected + 1);
        } else {
            self.show_maker();
        }
    }

    /// Asks for screenshots: one for the selected split, or several for the splits in order from
    /// the first, like AutoSplit numbers its screenshots.
    fn pick_screenshots(self: &Rc<Self>) {
        let state = self.state.borrow();
        let profile = &state.document.profile;
        let folder = match profile.screenshot_directory.as_str() {
            "" => profile.split_image_directory.clone(),
            screenshots => screenshots.to_owned(),
        };
        let start = state.document.folder(&exe_folder()).join(folder);
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
        let count = maker.sources.len();
        for (offset, path) in paths.iter().enumerate().filter(|(offset, _)| first + offset < count) {
            maker.sources[first + offset] = Some(Source::File(path.clone()));
            maker.saved[first + offset] = None;
        }
        drop(maker);
        let status = match paths.len() {
            1 => "Screenshot set for the selected split.".to_owned(),
            n if n == count => format!("{n} screenshots set, one for each split."),
            n => format!("{n} screenshots for {count} splits, set in order from the first."),
        };
        self.windows.maker.set_status(status.into());
        // Split images picked as screenshots would be masked twice
        let made: Vec<String> = paths
            .iter()
            .filter_map(|path| path.file_name().map(|name| name.to_string_lossy().into_owned()))
            .filter(|name| name_parts(name).len() > 2)
            .collect();
        if !made.is_empty() {
            self.show_message(Message::warning(format!(
                "These look like split images already, not screenshots:\n\n{}",
                made.join("\n")
            )));
        }
        self.show_maker();
    }

    /// Writes the split images of `indexes` that have something to make them from, after asking
    /// about files they replace.
    fn save_split_images(self: &Rc<Self>, indexes: Vec<usize>) {
        let Some(folder) = self.split_image_folder().filter(|folder| folder.is_dir()) else {
            return self.show_message(Message::error(
                "Set a split image folder that exists in the main window first: the split images are saved there.",
            ));
        };
        let names = self.maker_file_names();
        let sources: Vec<(usize, Source)> = {
            let maker = self.maker.borrow();
            indexes.into_iter().filter_map(|index| Some((index, maker.sources.get(index)?.clone()?))).collect()
        };
        let mut jobs = Vec::new();
        for (index, source) in sources {
            match self.maker_mask(index) {
                Ok(mask) => jobs.push(Job { index, source, mask, target: folder.join(&names[index]) }),
                Err(error) => return self.show_message(Message::error(error)),
            }
        }
        let written: Vec<String> = jobs.iter().map(|job| names[job.index].clone()).collect();
        let plan = plan_writes(&written, &user_files(&folder).unwrap_or_default());
        if plan.overwrites.is_empty() && plan.stale.is_empty() {
            return self.write_split_images(folder, jobs, Vec::new());
        }
        let mut text = String::new();
        if !plan.overwrites.is_empty() {
            text += &format!("These split images are replaced:\n{}\n\n", plan.overwrites.join("\n"));
        }
        let file_name = |path: &PathBuf| path.file_name().map(|name| name.to_string_lossy().into_owned());
        if plan.stale.is_empty() {
            text += "Replace them?";
        } else {
            let stale: Vec<String> = plan.stale.iter().filter_map(file_name).collect();
            text += &format!(
                "These older split images are for the same splits under other names, and would be used too:\n{}\n\n\
                 Yes deletes them, No keeps them.",
                stale.join("\n")
            );
        }
        let weak = Rc::downgrade(self);
        let stale = plan.stale;
        let has_stale = !stale.is_empty();
        self.show_message(Message::question(text, move |answer| {
            let Some(gui) = weak.upgrade() else { return };
            match answer {
                Answer::Yes => gui.write_split_images(folder, jobs, stale),
                Answer::No if has_stale => gui.write_split_images(folder, jobs, Vec::new()),
                _ => {}
            }
        }));
    }

    /// Writes split images on a thread of their own, deletes `stale` ones, and moves screenshots
    /// out of the split image folder.
    fn write_split_images(&self, folder: PathBuf, jobs: Vec<Job>, stale: Vec<PathBuf>) {
        self.windows.maker.set_status(format!("Saving {} split images...", jobs.len()).into());
        let spawned = std::thread::Builder::new().name("split images".into()).spawn(move || {
            let results: Vec<(usize, Result<PathBuf, String>)> =
                jobs.iter().map(|job| (job.index, job.run().map(|()| job.target.clone()))).collect();
            let mut errors: Vec<String> = Vec::new();
            for path in &stale {
                if let Err(error) = std::fs::remove_file(path) {
                    errors.push(format!("Couldn't delete {}: {error}", path.display()));
                }
            }
            let files: Vec<PathBuf> = jobs
                .iter()
                .filter_map(|job| match &job.source {
                    Source::File(path) => Some(path.clone()),
                    Source::Grabbed(_) => None,
                })
                .collect();
            let moved = move_to_base(&folder, &files).unwrap_or_else(|error| {
                errors.push(format!("Couldn't move the screenshots to the base folder: {error}"));
                0
            });
            with_gui(move |gui| gui.split_images_written(results, moved, errors));
        });
        if let Err(error) = spawned {
            self.show_message(Message::error(format!("Couldn't save the split images: {error}")));
        }
    }

    fn split_images_written(
        &self,
        results: Vec<(usize, Result<PathBuf, String>)>,
        moved: usize,
        mut errors: Vec<String>,
    ) {
        let mut written = 0;
        let mut maker = self.maker.borrow_mut();
        for (index, result) in results {
            match result {
                Ok(path) => {
                    written += 1;
                    if let Some(saved) = maker.saved.get_mut(index) {
                        *saved = Some(path);
                    }
                    // A screenshot moved to the base folder is read from there
                    if let Some(Some(Source::File(source))) = maker.sources.get_mut(index)
                        && let (Some(parent), Some(name)) = (source.parent(), source.file_name())
                        && parent.join("base").join(name).is_file()
                    {
                        *source = parent.join("base").join(name);
                    }
                }
                Err(error) => errors.push(error),
            }
        }
        drop(maker);
        let moved = if moved > 0 { format!(", moved {moved} screenshots to the base folder") } else { String::new() };
        self.windows.maker.set_status(format!("Saved {written} split images{moved}.").into());
        self.send(UiCommand::ReloadSplitImages);
        self.show_maker();
        if !errors.is_empty() {
            self.show_message(Message::error(errors.join("\n\n")));
        }
    }

    /// Copies the preset's savestates to a folder of their own, named for the emulator, and puts
    /// them on the clipboard, or opens the folder when the clipboard can't take files.
    fn copy_savestates(&self) {
        let maker = self.maker.borrow();
        let Some(loaded) = maker.loaded() else { return };
        let copies: Vec<(PathBuf, String)> =
            loaded.preset.savestate_copies().into_iter().map(|(path, name)| (loaded.file(path), name)).collect();
        drop(maker);
        let count = self.savestate_folders.borrow().len();
        let folder = std::env::temp_dir().join(format!("AutoSplit_savestates_{}_{count}", std::process::id()));
        let mut copied = Vec::new();
        let mut missing = Vec::new();
        if let Err(error) = std::fs::create_dir_all(&folder) {
            return self.show_message(Message::error(format!("Couldn't make {}: {error}", folder.display())));
        }
        self.savestate_folders.borrow_mut().push(folder.clone());
        for (source, name) in copies {
            let target = folder.join(name);
            match std::fs::copy(&source, &target) {
                Ok(_) => copied.push(target),
                Err(_) => missing.push(source.display().to_string()),
            }
        }
        if copied.is_empty() {
            return self.show_message(Message::error("None of the preset's savestate files were found."));
        }
        let mut clipboard = self.clipboard.borrow_mut();
        if clipboard.is_none() {
            *clipboard = arboard::Clipboard::new().ok();
        }
        let set = clipboard.as_mut().map(|clipboard| clipboard.set().file_list(&copied));
        drop(clipboard);
        let status = match set {
            Some(Ok(())) => format!("Copied {} savestates to the clipboard.", copied.len()),
            _ => {
                self.open(&folder.display().to_string());
                format!(
                    "The clipboard can't take files here, so the {} savestates are in the folder that opened.",
                    copied.len()
                )
            }
        };
        self.windows.maker.set_status(status.into());
        if !missing.is_empty() {
            self.show_message(Message::warning(format!("These savestates weren't found:\n\n{}", missing.join("\n"))));
        }
    }

    /// The instructions of the preset's savestates, in a window of their own.
    fn show_instructions(&self) {
        let maker = self.maker.borrow();
        let Some(loaded) = maker.loaded() else { return };
        let width = loaded.preset.splits.len().to_string().len();
        let cards: Vec<InstructionCard> = loaded
            .preset
            .splits
            .iter()
            .enumerate()
            .filter(|(_, split)| !split.savestate.is_empty() && !split.instructions.is_empty())
            .map(|(index, split)| InstructionCard {
                heading: format!("{index:0width$}. {}", split.name).into(),
                savestate: split.savestate.as_str().into(),
                text: split.instructions.as_str().into(),
            })
            .collect();
        let title = format!("Savestate Instructions - {}", loaded.preset.name);
        drop(maker);
        let window = &self.windows.instructions;
        window.set_cards(ModelRc::new(VecModel::from(cards)));
        window.set_window_title(title.into());
        self.show_window(window.window());
    }

    /// Opens the Live Tester on the selected split's split image.
    fn test_split_image(self: &Rc<Self>) {
        let maker = self.maker.borrow();
        let Some(path) = maker.saved.get(maker.selected).cloned().flatten() else { return };
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
