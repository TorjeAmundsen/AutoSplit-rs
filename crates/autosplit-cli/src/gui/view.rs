//! Shows the session in the main window. Runs on the session's thread and hands what changed to
//! the window's thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use autosplit_core::image::BgraView;
use autosplit_core::imgproc::stretch_to_rgba;
use autosplit_core::profile::UserProfile;
use autosplit_core::runner::{ImageRef, Showing};
use autosplit_core::split_folder::MAX_GROUP_SIZE;
use autosplit_core::split_image::{SplitContent, SplitImage};
use autosplit_core::text::{
    InfoCell, WindowTexts, decimal, decimal_or_dash, tester_ignored, tester_info, tester_status, trigger_caption,
    window_texts,
};
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
#[derive(Default)]
pub enum Picture {
    #[default]
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
    /// The current image area shows the split's images in a 2x2 grid.
    pub grid: bool,
}

/// The Live Tester window's texts.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TesterTexts {
    pub file: String,
    pub path: String,
    pub info: Vec<InfoCell>,
    /// Values in the image's name that are ignored, or nothing.
    pub ignored: String,
    /// What the tester waits for, or that it's done.
    pub status: String,
    pub live: String,
    pub highest: String,
    pub threshold: String,
    /// The live similarity reaches the threshold.
    pub reached: bool,
    /// The highest similarity reached the threshold, which stays until the tester starts over.
    pub highest_reached: bool,
    /// The live similarity, the highest and the threshold for the meter. The similarities are
    /// negative without one.
    pub meter: (f32, f32, f32),
    /// Shown instead of the live preview.
    pub live_message: String,
}

/// Frames the Live Tester kept that the window doesn't have yet, with their captions. A frame
/// is `None` when there was no capture.
#[derive(Default)]
pub struct KeptFrames {
    /// The window's frames are from before a rearm or another image, so it drops them first.
    pub cleared: bool,
    pub added: Vec<(Option<Pixels>, String)>,
}

impl KeptFrames {
    /// Nothing for the window to change.
    pub fn is_empty(&self) -> bool {
        !self.cleared && self.added.is_empty()
    }

    /// These frames after `older`, which the window hasn't taken yet.
    pub fn after(self, mut older: KeptFrames) -> KeptFrames {
        if self.cleared {
            return self;
        }
        older.added.extend(self.added);
        older
    }
}

/// What changed since the window last showed the session.
#[derive(Default)]
pub struct Update {
    pub texts: Option<Texts>,
    pub live: Picture,
    pub current: Picture,
    /// The grid's tiles, cleared when there's no image for them.
    pub tiles: [Picture; MAX_GROUP_SIZE],
    pub tester: TesterUpdate,
    pub maker: MakerUpdate,
}

impl Update {
    /// This update after `older`, which the window hasn't shown yet: the newest texts and
    /// pictures.
    pub fn after(self, older: Update) -> Update {
        let mut older_tiles = older.tiles.into_iter();
        Update {
            texts: self.texts.or(older.texts),
            live: self.live.after(older.live),
            current: self.current.after(older.current),
            tiles: self.tiles.map(|tile| tile.after(older_tiles.next().unwrap_or(Picture::Keep))),
            tester: self.tester.after(older.tester),
            maker: self.maker.after(older.maker),
        }
    }

    /// Nothing for the window to change.
    pub fn is_empty(&self) -> bool {
        let pictures = [&self.live, &self.current].into_iter().chain(&self.tiles);
        self.texts.is_none()
            && pictures.into_iter().all(|picture| matches!(picture, Picture::Keep))
            && self.tester.is_empty()
            && self.maker.is_empty()
    }
}

/// The Live Tester's part of an update.
#[derive(Default)]
pub struct TesterUpdate {
    pub texts: Option<TesterTexts>,
    pub live: Picture,
    pub image: Picture,
    pub kept: KeptFrames,
    /// It has no image: it couldn't load one, or it was closed.
    pub gone: bool,
    /// The window's request this is for. The window ignores it when it asked for another image
    /// since.
    pub request: u64,
}

impl TesterUpdate {
    /// This part after `older`, which is dropped when it's for another request.
    pub fn after(self, older: TesterUpdate) -> TesterUpdate {
        if self.request != older.request {
            return self;
        }
        TesterUpdate {
            texts: self.texts.or(older.texts),
            live: self.live.after(older.live),
            image: self.image.after(older.image),
            kept: self.kept.after(older.kept),
            gone: self.gone || older.gone,
            request: self.request,
        }
    }

    pub fn is_empty(&self) -> bool {
        let pictures = [&self.live, &self.image];
        self.texts.is_none()
            && pictures.into_iter().all(|picture| matches!(picture, Picture::Keep))
            && self.kept.is_empty()
            && !self.gone
    }
}

/// Make Split Images' part of an update.
#[derive(Default)]
pub struct MakerUpdate {
    /// Its preview of the capture region.
    pub live: Picture,
    /// Shown instead of its preview, when it changed.
    pub message: Option<String>,
}

impl MakerUpdate {
    pub fn after(self, older: MakerUpdate) -> MakerUpdate {
        MakerUpdate { live: self.live.after(older.live), message: self.message.or(older.message) }
    }

    pub fn is_empty(&self) -> bool {
        matches!(self.live, Picture::Keep) && self.message.is_none()
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
    /// Whether the Live Tester's window can be seen: not minimized or covered.
    tester_visible: Arc<AtomicBool>,
    /// Whether Make Split Images can be seen.
    maker_visible: Arc<AtomicBool>,
    tester: TesterView,
    maker: MakerPreview,
}

/// What the window shows of the Live Tester, as last sent.
#[derive(Default)]
struct TesterView {
    /// Whether it was open at the last update.
    open: bool,
    /// The request the window was last told has no image for.
    gone_sent: Option<u64>,
    texts: Option<TesterTexts>,
    /// The frame shown in its preview.
    frame: Option<u64>,
    /// Its image shown, by which loading it's from.
    shown: Option<u64>,
    /// The kept frames the window has: of which loading and rearm, and how many.
    kept_sent: (u64, u64, usize),
}

/// What Make Split Images shows of the capture, as last sent.
#[derive(Default)]
struct MakerPreview {
    /// The frame shown in its preview.
    frame: Option<u64>,
    /// The message it shows instead of its preview.
    message: Option<String>,
}

impl GuiView {
    pub fn new(
        visible: Arc<AtomicBool>,
        tester_visible: Arc<AtomicBool>,
        maker_visible: Arc<AtomicBool>,
        deliver: Deliver,
        notices: impl Fn(Notice) + Send + 'static,
    ) -> Self {
        Self {
            visible,
            tester_visible,
            maker_visible,
            tester: TesterView::default(),
            maker: MakerPreview::default(),
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
        let message = match (profile.live_capture_region, profile.has_capture_device()) {
            (true, true) => return region_preview(snapshot, &mut self.shown_frame),
            (true, false) => NO_DEVICE.to_owned(),
            (false, _) => String::new(),
        };
        (message, if self.shown_frame.take().is_some() { Picture::Clear } else { Picture::Keep })
    }

    fn maker_seen(&self) -> bool {
        self.maker_visible.load(Ordering::Relaxed)
    }

    /// Make Split Images' preview of the capture region, while it can be seen.
    fn maker(&mut self, snapshot: &Snapshot) -> MakerUpdate {
        let shown = &mut self.maker;
        if !self.maker_visible.load(Ordering::Relaxed) {
            shown.frame = None;
            return MakerUpdate::default();
        }
        let (message, live) = if snapshot.profile.has_capture_device() {
            region_preview(snapshot, &mut shown.frame)
        } else {
            (NO_DEVICE_IN_TOOL.into(), Picture::Keep)
        };
        let message = (shown.message.as_ref() != Some(&message)).then(|| {
            shown.message = Some(message.clone());
            message
        });
        MakerUpdate { live, message }
    }

    /// The Live Tester's texts and pictures, when it's open.
    fn tester(&mut self, snapshot: &Snapshot) -> TesterUpdate {
        let mut update = TesterUpdate { request: snapshot.tester_request, ..TesterUpdate::default() };
        let tester_visible = self.tester_visible.load(Ordering::Relaxed);
        let shown = &mut self.tester;
        let Some(tester) = snapshot.tester else {
            shown.open = false;
            (shown.texts, shown.frame, shown.shown) = (None, None, None);
            // Said once, so the window can clear what it showed of the last image
            if shown.gone_sent != Some(snapshot.tester_request) {
                shown.gone_sent = Some(snapshot.tester_request);
                update.gone = true;
            }
            return update;
        };
        shown.open = true;
        shown.gone_sent = None;
        let (live_message, live) = if !snapshot.profile.has_capture_device() {
            shown.frame = None;
            (NO_DEVICE_IN_TOOL.into(), Picture::Keep)
        } else if tester_visible {
            region_preview(snapshot, &mut shown.frame)
        } else {
            // Nobody sees it, so frames aren't converted for it
            (shown.texts.as_ref().map(|texts| texts.live_message.clone()).unwrap_or_default(), Picture::Keep)
        };
        update.live = live;
        let image = tester.image();
        if shown.shown != Some(snapshot.tester_loaded) {
            shown.shown = Some(snapshot.tester_loaded);
            // The window cleared its texts for the new image
            shown.texts = None;
            update.image = match &image.content {
                SplitContent::Image { pixels, .. } => Picture::Show(rgba(pixels.view())),
                SplitContent::Invalid(_) => Picture::Clear,
            };
        }
        let threshold = tester.threshold();
        let similarity = tester.similarity().filter(|_| snapshot.receiving);
        let texts = TesterTexts {
            file: image.display_name(),
            path: image.path.display().to_string(),
            info: tester_info(tester),
            ignored: tester_ignored(tester),
            status: tester_status(tester),
            live: decimal_or_dash(similarity),
            highest: decimal_or_dash(tester.highest()),
            threshold: decimal(threshold),
            reached: similarity.is_some_and(|similarity| similarity >= threshold),
            highest_reached: tester.highest().is_some_and(|highest| highest >= threshold),
            meter: (
                similarity.map_or(-1.0, |similarity| similarity as f32),
                tester.highest().map_or(-1.0, |highest| highest as f32),
                threshold as f32,
            ),
            live_message,
        };
        if shown.texts.as_ref() != Some(&texts) {
            shown.texts = Some(texts.clone());
            update.texts = Some(texts);
        }
        let triggers = tester.triggers();
        let (loaded, generation, sent) = shown.kept_sent;
        let cleared = (loaded, generation) != (snapshot.tester_loaded, tester.generation());
        let from = if cleared { 0 } else { sent.min(triggers.len()) };
        let added = triggers[from..]
            .iter()
            .map(|trigger| {
                (trigger.frame.as_ref().map(|frame| preview(frame.view())), trigger_caption(tester, trigger))
            })
            .collect();
        update.kept = KeptFrames { cleared, added };
        shown.kept_sent = (snapshot.tester_loaded, tester.generation(), triggers.len());
        update
    }

    /// The current image, or the grid's tiles when `grid` has the images of the split, or the
    /// start images.
    fn current_image(
        &mut self,
        snapshot: &Snapshot,
        grid: Option<&[SplitImage]>,
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
        if let Some(images) = grid {
            return (Picture::Clear, std::array::from_fn(|slot| picture(images.get(slot))));
        }
        let image = shown.and_then(|(image, ..)| match image {
            ImageRef::Start => snapshot.images.start.first(),
            ImageRef::Split(index) => snapshot.images.splits.get(index),
        });
        (picture(image), [const { Picture::Clear }; MAX_GROUP_SIZE])
    }
}

/// Shown instead of the capture when the profile has no capture device.
pub const NO_DEVICE: &str = "No capture device. Pick one in File > Settings.";

/// `NO_DEVICE` in a tool window, which the main window's menu can't be used beside, but Settings
/// can be opened from.
pub const NO_DEVICE_IN_TOOL: &str =
    "No capture device. Pick one in Settings (Ctrl+, or the Settings button in the main window).";

/// The capture region of the newest frame, unless `shown` is that frame already, or why there's
/// none.
fn region_preview(snapshot: &Snapshot, shown: &mut Option<u64>) -> (String, Picture) {
    let profile = snapshot.profile;
    let Some(frame) = snapshot.frame.as_ref().filter(|_| snapshot.receiving) else {
        *shown = None;
        let message = snapshot.device_problem.unwrap_or("Waiting for capture device...");
        return (message.into(), Picture::Keep);
    };
    let (width, height) = frame.frame_size();
    if profile.capture_region.rect(width, height).area() == 0 {
        *shown = None;
        return ("The capture region is empty or outside the capture.".into(), Picture::Keep);
    }
    if *shown == Some(frame.sequence()) {
        return (String::new(), Picture::Keep);
    }
    // Right after the region changes, its part of the frame isn't converted yet
    let Some(view) = frame.region_view(&profile.capture_region) else { return (String::new(), Picture::Keep) };
    *shown = Some(frame.sequence());
    (String::new(), Picture::Show(preview(view)))
}

/// The capture region stretched to the preview's size.
fn preview(view: BgraView) -> Pixels {
    let mut pixels = Pixels::new(PREVIEW_WIDTH, PREVIEW_HEIGHT);
    let (rgba, _) = pixels.make_mut_bytes().as_chunks_mut::<4>();
    stretch_to_rgba(view, PREVIEW_WIDTH as usize, PREVIEW_HEIGHT as usize, rgba);
    pixels
}

/// A whole image in BGRA as RGBA, for the current image, which the window stretches itself.
pub(crate) fn rgba(view: BgraView) -> Pixels {
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
        // The Live Tester's window can be seen while the main window can't
        let tool_seen = (self.tester.open && self.tester_visible.load(Ordering::Relaxed)) || self.maker_seen();
        if !self.visible() && !tool_seen {
            return HIDDEN_INTERVAL;
        }
        // AutoSplit redraws the live preview at the comparison rate, while frames come
        let preview = ((profile.live_capture_region || tool_seen) && self.receiving)
            .then(|| Duration::from_secs_f64(1.0 / profile.fps_limit.max(1) as f64))
            .map(|preview| if cfg!(target_os = "macos") { preview.max(MACOS_PREVIEW_INTERVAL) } else { preview });
        preview.map_or(TEXTS_INTERVAL, |preview| preview.min(TEXTS_INTERVAL))
    }

    fn wants_frames(&self, profile: &UserProfile) -> bool {
        (profile.live_capture_region && self.visible()) || self.maker_seen()
    }

    fn update(&mut self, snapshot: Snapshot<'_>) {
        self.receiving = snapshot.receiving;
        let (live_message, live) =
            if self.visible() { self.live_preview(&snapshot) } else { (String::new(), Picture::Keep) };
        let status = snapshot.status;
        let profile = snapshot.profile;
        // The images of the split being compared, or the start images, when there are several to
        // show in the grid
        let images = &snapshot.images;
        let grid = match status.showing {
            Showing::Image(ImageRef::Split(first)) if profile.split_image_grid => (images.groups.iter())
                .find(|group| group.start == first && group.len() > 1)
                .map(|group| &images.splits[group.clone()]),
            Showing::Image(ImageRef::Start) if profile.split_image_grid && images.start.len() > 1 => {
                Some(&images.start[..])
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
            window: window_texts(status, snapshot.now, &snapshot.idle, snapshot.receiving, profile.display_pause_timer),
            buttons: [
                buttons.start,
                buttons.reset,
                buttons.skip_split,
                buttons.undo_split,
                buttons.next_image,
                buttons.previous_image,
            ],
            grid: shows_grid,
        };
        let changed = self.texts.as_ref() != Some(&texts);
        if changed {
            self.texts = Some(texts.clone());
        }
        let tester = self.tester(&snapshot);
        let maker = self.maker(&snapshot);
        let update = Update { texts: changed.then_some(texts), live, current, tiles, tester, maker };
        if update.is_empty() {
            return;
        }
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
