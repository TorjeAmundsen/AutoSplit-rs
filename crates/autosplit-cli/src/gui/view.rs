//! Shows the session in the main window. Runs on the session's thread and hands what changed to
//! the window's thread.

use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use autosplit_core::image::BgraView;
use autosplit_core::imgproc::stretch_to_rgba;
use autosplit_core::profile::UserProfile;
use autosplit_core::runner::{ImageRef, Showing};
use autosplit_core::split_folder::MAX_GROUP_SIZE;
use autosplit_core::split_image::{SplitContent, SplitImage};
use autosplit_core::text::{WindowTexts, window_texts};
use slint::{Rgba8Pixel, SharedPixelBuffer};

use crate::session::{Notice, Snapshot, View};

/// The live preview's size, like AutoSplit's.
const PREVIEW_WIDTH: u32 = 320;
const PREVIEW_HEIGHT: u32 = 240;
/// How often the window's texts are updated when it doesn't show a live preview.
const TEXTS_INTERVAL: Duration = Duration::from_millis(100);
/// The live preview's rate on macOS at most, where every preview frame redraws the whole window.
const MACOS_PREVIEW_INTERVAL: Duration = Duration::from_millis(33);
/// How often while it's minimized or covered, when nobody sees it.
const HIDDEN_INTERVAL: Duration = Duration::from_millis(500);

pub type Pixels = SharedPixelBuffer<Rgba8Pixel>;

/// A preview's image, when it changed.
pub enum Picture {
    Keep,
    Clear,
    Show(Pixels),
}

impl Picture {
    /// This change after `older`, which the window hasn't shown yet.
    pub fn after(self, older: Picture) -> Picture {
        match self {
            Self::Keep => older,
            newer => newer,
        }
    }
}

/// The main window's texts and buttons.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Texts {
    pub running: bool,
    pub device_name: String,
    pub live_message: String,
    pub window: WindowTexts,
    /// Start, reset, skip, undo, next image and previous image.
    pub buttons: [bool; 6],
    /// Whether there's a capture, for Align Region.
    pub capturing: bool,
    /// The current image area shows the split's images in a 2x2 grid.
    pub grid: bool,
}

/// What changed since the window last showed the session.
pub struct Update {
    pub texts: Option<Texts>,
    pub live: Picture,
    pub current: Picture,
    /// The grid's tiles, cleared when there's no image for them.
    pub tiles: [Picture; MAX_GROUP_SIZE],
}

impl Update {
    /// This update after `older`, which the window hasn't shown yet: the newest texts and
    /// pictures.
    pub fn after(self, older: Update) -> Update {
        Update {
            texts: self.texts.or(older.texts),
            live: self.live.after(older.live),
            current: self.current.after(older.current),
            tiles: {
                let mut older = older.tiles.into_iter();
                self.tiles.map(|tile| tile.after(older.next().unwrap_or(Picture::Keep)))
            },
        }
    }
}

/// Hands an update to the window's thread, which takes the newest one from the slot.
pub type Deliver = Box<dyn Fn(Arc<Mutex<Option<Update>>>) + Send>;

pub struct GuiView {
    /// Whether the window can be seen: not minimized or covered.
    visible: Arc<AtomicBool>,
    /// An update the window's thread hasn't taken yet. Newer ones are merged into it, so a busy
    /// or blocked window thread never gets a growing queue.
    pending: Arc<Mutex<Option<Update>>>,
    deliver: Deliver,
    notices: Box<dyn Fn(Notice) + Send>,
    texts: Option<Texts>,
    /// The frame shown in the live preview.
    shown_frame: Option<u64>,
    /// The image shown as the current image, which loading of the images it's from, and
    /// whether it's shown in the grid.
    shown_image: Option<(ImageRef, u64, bool)>,
    /// Whether the device sent frames at the last update.
    receiving: bool,
}

impl GuiView {
    pub fn new(visible: Arc<AtomicBool>, deliver: Deliver, notices: impl Fn(Notice) + Send + 'static) -> Self {
        Self {
            visible,
            pending: Arc::default(),
            deliver,
            notices: Box::new(notices),
            texts: None,
            shown_frame: None,
            shown_image: None,
            receiving: false,
        }
    }

    fn visible(&self) -> bool {
        self.visible.load(Ordering::Relaxed)
    }

    fn live_preview(&mut self, snapshot: &Snapshot) -> (String, Picture) {
        let profile = snapshot.profile;
        if !profile.live_capture_region || !profile.has_capture_device() {
            return (String::new(), if self.shown_frame.take().is_some() { Picture::Clear } else { Picture::Keep });
        }
        let Some(frame) = snapshot.frame.as_ref().filter(|_| snapshot.receiving) else {
            self.shown_frame = None;
            return ("Waiting for capture device...".into(), Picture::Keep);
        };
        let (width, height) = frame.frame_size();
        if profile.capture_region.rect(width, height).area() == 0 {
            self.shown_frame = None;
            return ("The capture region is empty or outside the capture.".into(), Picture::Keep);
        }
        if self.shown_frame == Some(frame.sequence()) {
            return (String::new(), Picture::Keep);
        }
        // Right after the region changes, its part of the frame isn't converted yet
        let Some(view) = frame.region_view(&profile.capture_region) else { return (String::new(), Picture::Keep) };
        self.shown_frame = Some(frame.sequence());
        (String::new(), Picture::Show(preview(view)))
    }

    /// The current image, or the grid's tiles when `grid` has the images of the split.
    fn current_image(
        &mut self,
        snapshot: &Snapshot,
        grid: Option<Range<usize>>,
    ) -> (Picture, [Picture; MAX_GROUP_SIZE]) {
        let shown = match snapshot.status.showing {
            Showing::Image(image) => Some((image, snapshot.images_loaded, grid.is_some())),
            _ => None,
        };
        if shown == self.shown_image {
            return (Picture::Keep, [const { Picture::Keep }; MAX_GROUP_SIZE]);
        }
        self.shown_image = shown;
        let picture = |image: Option<&SplitImage>| match image.map(|image| &image.content) {
            Some(SplitContent::Image { pixels, .. }) => Picture::Show(rgba(pixels.view())),
            _ => Picture::Clear,
        };
        if let Some(group) = grid {
            let images = &snapshot.images.splits[group];
            return (Picture::Clear, std::array::from_fn(|slot| picture(images.get(slot))));
        }
        let image = shown.and_then(|(image, ..)| match image {
            ImageRef::Start => snapshot.images.start.as_ref(),
            ImageRef::Split(index) => snapshot.images.splits.get(index),
        });
        (picture(image), [const { Picture::Clear }; MAX_GROUP_SIZE])
    }
}

/// The capture region stretched to the preview's size.
fn preview(view: BgraView) -> Pixels {
    let mut pixels = Pixels::new(PREVIEW_WIDTH, PREVIEW_HEIGHT);
    let (rgba, _) = pixels.make_mut_bytes().as_chunks_mut::<4>();
    stretch_to_rgba(view, PREVIEW_WIDTH as usize, PREVIEW_HEIGHT as usize, rgba);
    pixels
}

/// A whole image in BGRA as RGBA, for the current image, which the window stretches itself.
fn rgba(view: BgraView) -> Pixels {
    let mut buffer = Pixels::new(view.width() as u32, view.height() as u32);
    let rows = buffer.make_mut_slice().chunks_exact_mut(view.width().max(1));
    for (y, out) in rows.enumerate() {
        for (pixel, bgra) in out.iter_mut().zip(view.row(y).as_chunks::<4>().0) {
            *pixel = Rgba8Pixel { r: bgra[2], g: bgra[1], b: bgra[0], a: bgra[3] };
        }
    }
    buffer
}

impl View for GuiView {
    fn interval(&self, profile: &UserProfile) -> Duration {
        if !self.visible() {
            return HIDDEN_INTERVAL;
        }
        // AutoSplit redraws the live preview at the comparison rate, while frames come
        let preview = (profile.live_capture_region && self.receiving)
            .then(|| Duration::from_secs_f64(1.0 / profile.fps_limit.max(1) as f64))
            .map(|preview| if cfg!(target_os = "macos") { preview.max(MACOS_PREVIEW_INTERVAL) } else { preview });
        preview.map_or(TEXTS_INTERVAL, |preview| preview.min(TEXTS_INTERVAL))
    }

    fn wants_frames(&self, profile: &UserProfile) -> bool {
        profile.live_capture_region && self.visible()
    }

    fn update(&mut self, snapshot: Snapshot<'_>) {
        self.receiving = snapshot.receiving;
        let (live_message, live) =
            if self.visible() { self.live_preview(&snapshot) } else { (String::new(), Picture::Keep) };
        let status = snapshot.status;
        let profile = snapshot.profile;
        // The images of the split being compared, when there are several to show in the grid
        let grid = match status.showing {
            Showing::Image(ImageRef::Split(first)) if profile.split_image_grid => {
                snapshot.images.groups.iter().find(|group| group.start == first && group.len() > 1).cloned()
            }
            _ => None,
        };
        let shows_grid = grid.is_some();
        let (current, tiles) = self.current_image(&snapshot, grid);
        let buttons = status.buttons;
        let texts = Texts {
            running: status.running,
            device_name: snapshot.device_name.unwrap_or(&profile.capture_device_name).to_owned(),
            live_message,
            window: window_texts(status, snapshot.now, &snapshot.idle, snapshot.receiving),
            buttons: [
                buttons.start,
                buttons.reset,
                buttons.skip_split,
                buttons.undo_split,
                buttons.next_image,
                buttons.previous_image,
            ],
            capturing: snapshot.receiving,
            grid: shows_grid,
        };
        let changed = self.texts.as_ref() != Some(&texts);
        let pictures_kept = [&live, &current].into_iter().chain(&tiles).all(|picture| matches!(picture, Picture::Keep));
        if !changed && pictures_kept {
            return;
        }
        if changed {
            self.texts = Some(texts.clone());
        }
        let update = Update { texts: changed.then_some(texts), live, current, tiles };
        let mut pending = self.pending.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        match pending.take() {
            Some(older) => *pending = Some(update.after(older)),
            None => {
                *pending = Some(update);
                drop(pending);
                (self.deliver)(self.pending.clone());
            }
        }
    }

    fn notice(&mut self, notice: Notice) {
        (self.notices)(notice);
    }
}
