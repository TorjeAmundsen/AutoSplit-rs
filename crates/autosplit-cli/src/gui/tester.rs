//! The Live Tester window: one split image compared with the capture region like a run would,
//! outside of a run.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::paths::{canonical, file_name_text};
use super::placement::beside;
use super::ui::{InfoCell, TesterReading};
use super::view::{KeptFrames, TesterUpdate};
use super::{Gui, Tool, exe_folder, file_name_parts, png_images, shown_image};
use crate::session::UiCommand;

/// What the Live Tester says before it has an image.
const TESTER_INTRO: &str = "Open a split image to compare it with the capture region, like a run would.";
impl Gui {
    pub(super) fn connect_tester(&self) {
        let weak = self.me.clone();
        let tester = &self.windows.tester;
        tester.set_reading(cleared_reading());
        on!(weak, tester, on_open_image, () gui => gui.pick_tester_image());
        on!(weak, tester, on_previous_image, () gui => gui.step_tester_image(-1));
        on!(weak, tester, on_next_image, () gui => gui.step_tester_image(1));
        on!(weak, tester, on_reload_image, () gui => gui.load_tester_image());
        on!(weak, tester, on_rearm, () gui => gui.send(UiCommand::RearmTester));
        on!(weak, tester, on_previous_kept, () gui => gui.show_kept(-1));
        on!(weak, tester, on_next_kept, () gui => gui.show_kept(1));
        on!(always weak, tester, on_blocked_clicked, () gui => gui.focus_message());
        on!(always weak, tester.window(), on_close_requested, () gui => {
            gui.tool_closed(Tool::Tester);
        } else slint::CloseRequestResponse::HideWindow);
    }

    /// Opens the Live Tester beside the main window, with the image it had when it was closed,
    /// from any folder. The split image folder's first image when the folder changed meanwhile,
    /// the image is gone, or it had none, and else asks for one.
    pub(super) fn open_tester(self: &Rc<Self>) {
        let window = self.windows.tester.window();
        if window.is_visible() {
            return self.show_window(window);
        }
        self.place_by_main(window, beside);
        self.show_window(window);
        self.tool_opened(Tool::Tester);
        self.update_visibility();
        let gone = self.tester_state.borrow().path.as_ref().is_none_or(|path| !path.is_file());
        if gone {
            let first = self.split_image_folder().and_then(|folder| png_images(&folder).into_iter().next());
            self.tester_state.borrow_mut().path = first;
        }
        if self.tester_state.borrow().path.is_some() {
            self.load_tester_image();
        } else {
            self.pick_tester_image();
        }
    }

    /// The first image of the profile's split image folder, unless the Live Tester's image is
    /// from that folder already. `None` without a split image folder or images in it.
    fn split_folder_image(&self) -> Option<PathBuf> {
        let folder = self.split_image_folder()?;
        let current = self.tester_state.borrow().path.as_ref().and_then(|path| path.parent()).map(Path::to_owned);
        // Compared as the system sees them, since the profile's folder can be written another way
        if current.is_some_and(|current| canonical(&current) == canonical(&folder)) {
            return None;
        }
        png_images(&folder).into_iter().next()
    }

    /// The profile's split image folder changed: the Live Tester switches to its first image, at
    /// once when it's open, else when it opens again.
    pub(super) fn tester_follows_split_folder(&self) {
        if let Some(image) = self.split_folder_image() {
            self.tester_state.borrow_mut().path = Some(image);
            if self.windows.tester.window().is_visible() {
                self.load_tester_image();
            }
        }
    }

    /// The main window's Close button.
    pub(super) fn close_tester(&self) {
        self.tool_closed(Tool::Tester);
        let _ = self.windows.tester.hide();
    }

    /// The Live Tester closes: it stops comparing. See [`Gui::tool_closed`].
    /// The Live Tester closed, which `tool_closed` says.
    pub(super) fn tester_closed(&self) {
        self.send(UiCommand::CloseTester);
        self.tester_seen.hidden();
        // The session's update saying it has no image is for the image that was closed, which
        // the window keeps until it opens again
        self.tester_state.borrow_mut().request += 1;
    }

    /// Asks for the Live Tester's image, in its folder or the split image folder.
    fn pick_tester_image(self: &Rc<Self>) {
        let path = self.tester_state.borrow().path.clone();
        let folder = match path {
            Some(path) => path.parent().map_or_else(exe_folder, Path::to_owned),
            None => self.split_image_folder().unwrap_or_else(exe_folder),
        };
        let dialog = self
            .file_dialog_over("Select Split Image", self.windows.tester.window())
            .set_directory(folder)
            .add_filter("PNG image", &["png"])
            .pick_file();
        self.when_picked(dialog, |gui, path| {
            // Closed while the dialog was open
            if !gui.windows.tester.window().is_visible() {
                return;
            }
            gui.tester_state.borrow_mut().path = Some(path);
            gui.load_tester_image();
        });
    }

    /// The PNG images in the Live Tester's image's folder, in the split image folder's order.
    fn tester_folder_images(&self) -> Vec<PathBuf> {
        let folder = self.tester_state.borrow().path.as_ref().and_then(|path| path.parent()).map(Path::to_owned);
        folder.map(|folder| png_images(&folder)).unwrap_or_default()
    }

    /// Opens the image `step` images from the Live Tester's in its folder.
    fn step_tester_image(&self, step: isize) {
        let images = self.tester_folder_images();
        let current = self.tester_state.borrow().path.clone();
        let Some(index) = images.iter().position(|image| Some(image) == current.as_ref()) else { return };
        if let Some(image) = index.checked_add_signed(step).and_then(|index| images.get(index)) {
            self.tester_state.borrow_mut().path = Some(image.clone());
            self.load_tester_image();
        }
    }

    /// Has the session load the Live Tester's image. The window keeps showing the last image and
    /// the capture until the new one is loaded, so going through a folder doesn't flash.
    pub(super) fn load_tester_image(&self) {
        let mut state = self.tester_state.borrow_mut();
        let Some(path) = state.path.clone() else { return };
        state.request += 1;
        let request = state.request;
        drop(state);
        let images = self.tester_folder_images();
        let index = images.iter().position(|image| *image == path);
        let tester = &self.windows.tester;
        self.show_tester_file(&file_name_text(&path).unwrap_or_default());
        tester.set_path(path.display().to_string().into());
        tester.set_can_reload(true);
        tester.set_can_previous_image(index.is_some_and(|index| index > 0));
        tester.set_can_next_image(index.is_some_and(|index| index + 1 < images.len()));
        self.send(UiCommand::OpenTester { path, request });
    }

    /// Shows the Live Tester's file name, each part colored by what it does.
    fn show_tester_file(&self, name: &str) {
        let tester = &self.windows.tester;
        tester.set_file(name.into());
        tester.set_file_parts(file_name_parts(name));
    }

    /// Shows no image in the Live Tester, with `message` saying why.
    fn clear_tester(&self, message: &str) {
        let tester = &self.windows.tester;
        tester.set_has_image(false);
        tester.set_message(message.into());
        self.tester_info.clear();
        tester.set_reading(cleared_reading());
        tester.set_live_image(slint::Image::default());
        tester.set_split_image(slint::Image::default());
        self.take_kept(KeptFrames { cleared: true, added: Vec::new() });
    }

    /// Frames the Live Tester kept. Shows the newest.
    fn take_kept(&self, kept: KeptFrames) {
        if !kept.cleared && kept.added.is_empty() {
            return;
        }
        let mut state = self.tester_state.borrow_mut();
        if kept.cleared {
            state.kept.clear();
        }
        let added = kept.added.into_iter().map(|(pixels, caption)| {
            (pixels.map(slint::Image::from_rgba8).unwrap_or_default(), SharedString::from(caption))
        });
        state.kept.extend(added);
        state.kept_index = state.kept.len().saturating_sub(1);
        drop(state);
        self.show_kept(0);
    }

    /// Shows the kept frame `step` frames from the one shown.
    fn show_kept(&self, step: isize) {
        let mut state = self.tester_state.borrow_mut();
        let count = state.kept.len();
        state.kept_index = state.kept_index.saturating_add_signed(step).min(count.saturating_sub(1));
        let index = state.kept_index;
        let (image, caption) = state.kept.get(index).cloned().unwrap_or_default();
        drop(state);
        let tester = &self.windows.tester;
        tester.set_kept_image(image);
        tester.set_kept_caption(caption);
        tester.set_kept_position(if count > 1 { format!("{}/{count}", index + 1).into() } else { SharedString::new() });
        tester.set_can_previous_kept(index > 0);
        tester.set_can_next_kept(index + 1 < count);
    }

    /// The Live Tester's part of the session's update.
    pub(super) fn show_tester_update(&self, update: TesterUpdate) {
        // The Live Tester's part is for an image the window has asked to replace
        if update.request != self.tester_state.borrow().request {
            return;
        }
        let tester = &self.windows.tester;
        if update.gone {
            let tried = self.tester_state.borrow().path.is_some();
            self.clear_tester(if tried { "Couldn't load the image. Fix it, then press Reload." } else { TESTER_INTRO });
        }
        if let Some(texts) = update.texts {
            tester.set_has_image(true);
            self.show_tester_file(&texts.file);
            tester.set_path(texts.path.into());
            let cell = |cell: autosplit_core::text::InfoCell| InfoCell {
                wide: cell.name == autosplit_core::text::ACTS_CELL,
                name: cell.name.into(),
                value: cell.value.into(),
                note: cell.note.into(),
            };
            self.tester_info.set_vec(texts.info.into_iter().map(cell).collect::<Vec<_>>());
            let (live_value, highest_value, threshold_value) = texts.meter;
            tester.set_reading(TesterReading {
                ignored: texts.ignored.into(),
                status: texts.status.into(),
                live_message: texts.live_message.into(),
                live: texts.live.into(),
                highest: texts.highest.into(),
                threshold: texts.threshold.into(),
                noise: ModelRc::new(VecModel::from_iter(texts.noise.into_iter().map(SharedString::from))),
                reached: texts.reached,
                highest_reached: texts.highest_reached,
                live_value,
                highest_value,
                threshold_value,
            });
        }
        if let Some(live) = shown_image(update.live) {
            tester.set_live_image(live);
        }
        if let Some(split) = shown_image(update.image) {
            tester.set_split_image(split);
        }
        self.take_kept(update.kept);
    }
}

/// What the Live Tester reads without an image: dashes, and no similarity for the meter.
fn cleared_reading() -> TesterReading {
    let dash = || SharedString::from("-");
    TesterReading {
        live_message: "Open a split image to start comparing.".into(),
        live: dash(),
        highest: dash(),
        threshold: dash(),
        live_value: -1.0,
        highest_value: -1.0,
        threshold_value: -1.0,
        ..TesterReading::default()
    }
}

/// What the Live Tester window keeps of its own, besides what it shows.
#[derive(Default)]
pub(super) struct TesterState {
    /// Its image.
    pub(super) path: Option<PathBuf>,
    /// Numbers its loads, so updates for an older one are ignored.
    pub(super) request: u64,
    /// The frames it kept, with their captions, and which one it shows.
    kept: Vec<(slint::Image, SharedString)>,
    kept_index: usize,
}
