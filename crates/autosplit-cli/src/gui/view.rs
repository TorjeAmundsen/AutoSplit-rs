//! Shows the session in the main window. Runs on the session's thread and hands what changed to
//! the window's thread.

use std::path::{Path, PathBuf};
use std::time::Duration;

use autosplit_core::image::BgraView;
use autosplit_core::profile::UserProfile;
use autosplit_core::runner::{ImageRef, ResetReading, Showing, StartStatus};
use autosplit_core::split_image::SplitContent;
use autosplit_core::text::{decimal, decimal_or_dash, seconds_remaining_text};
use slint::{Rgba8Pixel, SharedPixelBuffer, SharedString};

use super::ui::MainWindow;
use crate::console::Console;
use crate::session::{Session, View};

/// The live preview's size, like AutoSplit's.
const PREVIEW_WIDTH: u32 = 320;
const PREVIEW_HEIGHT: u32 = 240;
/// How often the window is updated when it doesn't show a live preview.
const STATUS_INTERVAL: Duration = Duration::from_millis(100);

/// What the window shows, made on the session's thread.
struct Shown {
    running: bool,
    device_name: SharedString,
    live_message: SharedString,
    /// A new live image, or `None` to keep the one shown.
    live_image: Option<SharedPixelBuffer<Rgba8Pixel>>,
    clear_live_image: bool,
    current_file: SharedString,
    current_message: SharedString,
    /// A new current image, `Some(None)` to clear it.
    current_image: Option<Option<SharedPixelBuffer<Rgba8Pixel>>>,
    table: [SharedString; 6],
    image_loop: SharedString,
    start_status: SharedString,
    buttons: [bool; 6],
}

pub struct GuiView {
    window: slint::Weak<MainWindow>,
    /// The frame shown in the live preview.
    shown_frame: Option<u64>,
    /// The image shown as the current image: which one and its file name.
    shown_image: Option<(ImageRef, String)>,
    on_dialog: Box<dyn Fn(String) + Send>,
    on_profile: Box<dyn Fn(UserProfile) + Send>,
    on_loaded: Box<dyn Fn(PathBuf) + Send>,
}

impl GuiView {
    pub fn new(
        window: slint::Weak<MainWindow>,
        on_dialog: impl Fn(String) + Send + 'static,
        on_profile: impl Fn(UserProfile) + Send + 'static,
        on_loaded: impl Fn(PathBuf) + Send + 'static,
    ) -> Self {
        Self {
            window,
            shown_frame: None,
            shown_image: None,
            on_dialog: Box::new(on_dialog),
            on_profile: Box::new(on_profile),
            on_loaded: Box::new(on_loaded),
        }
    }
}

/// BGRA stretched to the preview's size with bilinear filtering, like Qt's smooth scaling.
fn preview(view: BgraView) -> SharedPixelBuffer<Rgba8Pixel> {
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(PREVIEW_WIDTH, PREVIEW_HEIGHT);
    let (width, height) = (view.width(), view.height());
    let scale_x = width as f32 / PREVIEW_WIDTH as f32;
    let scale_y = height as f32 / PREVIEW_HEIGHT as f32;
    let pixel = |x: usize, y: usize| -> [f32; 4] {
        let at = &view.row(y)[x * 4..x * 4 + 4];
        [f32::from(at[0]), f32::from(at[1]), f32::from(at[2]), f32::from(at[3])]
    };
    for (index, out) in buffer.make_mut_slice().iter_mut().enumerate() {
        let (x, y) = (index % PREVIEW_WIDTH as usize, index / PREVIEW_WIDTH as usize);
        let source_x = ((x as f32 + 0.5) * scale_x - 0.5).clamp(0.0, (width - 1) as f32);
        let source_y = ((y as f32 + 0.5) * scale_y - 0.5).clamp(0.0, (height - 1) as f32);
        let (x0, y0) = (source_x as usize, source_y as usize);
        let (x1, y1) = ((x0 + 1).min(width - 1), (y0 + 1).min(height - 1));
        let (fx, fy) = (source_x - x0 as f32, source_y - y0 as f32);
        let mix = |channel: usize| {
            let top = pixel(x0, y0)[channel] * (1.0 - fx) + pixel(x1, y0)[channel] * fx;
            let bottom = pixel(x0, y1)[channel] * (1.0 - fx) + pixel(x1, y1)[channel] * fx;
            (top * (1.0 - fy) + bottom * fy).round() as u8
        };
        *out = Rgba8Pixel { r: mix(2), g: mix(1), b: mix(0), a: mix(3) };
    }
    buffer
}

/// A whole image in BGRA as RGBA, for the current image, which the window stretches itself.
fn rgba(view: BgraView) -> SharedPixelBuffer<Rgba8Pixel> {
    let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::new(view.width() as u32, view.height() as u32);
    let rows = buffer.make_mut_slice().chunks_exact_mut(view.width().max(1));
    for (y, out) in rows.enumerate() {
        for (pixel, bgra) in out.iter_mut().zip(view.row(y).as_chunks::<4>().0) {
            *pixel = Rgba8Pixel { r: bgra[2], g: bgra[1], b: bgra[0], a: bgra[3] };
        }
    }
    buffer
}

impl View for GuiView {
    fn interval(&self, profile: &UserProfile) -> Option<Duration> {
        // AutoSplit redraws the live preview at the comparison rate
        let preview =
            profile.live_capture_region.then(|| Duration::from_secs_f64(1.0 / profile.fps_limit.max(1) as f64));
        Some(preview.map_or(STATUS_INTERVAL, |preview| preview.min(STATUS_INTERVAL)))
    }

    fn wants_frames(&self, profile: &UserProfile) -> bool {
        profile.live_capture_region
    }

    fn update(&mut self, session: &Session, _console: &mut Console) {
        let status = session.status();
        let profile = session.profile();
        let now = session.elapsed();

        let frame = session.latest_frame();
        let mut live_image = None;
        let mut clear_live_image = false;
        let live_message = if !profile.live_capture_region {
            clear_live_image = self.shown_frame.take().is_some();
            String::new()
        } else if let Some(frame) = frame.as_ref().filter(|_| session.receiving()) {
            if self.shown_frame != Some(frame.sequence()) {
                self.shown_frame = Some(frame.sequence());
                live_image = frame.region_view(&profile.capture_region).filter(|view| !view.is_empty()).map(preview);
            }
            String::new()
        } else {
            self.shown_frame = None;
            "Waiting for capture device...".into()
        };

        let image_ref = match status.showing {
            Showing::Image(image) => Some(image),
            _ => None,
        };
        let shown = image_ref.zip(status.current_filename.clone());
        let current_image = (shown != self.shown_image).then(|| {
            self.shown_image = shown.clone();
            let images = session.runner().images();
            let image = image_ref.and_then(|image| match image {
                ImageRef::Start => images.start.as_ref(),
                ImageRef::Split(index) => images.splits.get(index),
            });
            image.and_then(|image| match &image.content {
                SplitContent::Image { pixels, .. } => Some(rgba(pixels.view())),
                SplitContent::Invalid(_) => None,
            })
        });
        let left = |until: Duration| seconds_remaining_text(until.saturating_sub(now).as_secs_f64());
        let current_message = match status.showing {
            Showing::StartDelay { until } => format!("Delayed Before Starting:\n {}", left(until)),
            Showing::Pause { until } => format!("None (Paused). {}", left(until)),
            Showing::SplitDelay { until } => format!("Delayed Split: {}", left(until)),
            Showing::Image(_) => String::new(),
            Showing::Nothing => session.idle_text(),
        };
        let (reset_live, reset_highest, reset_threshold) = match status.reset_similarity {
            None | Some(ResetReading::NotApplicable) => ("N/A".into(), "N/A".into(), "N/A".into()),
            Some(ResetReading::Disabled) => {
                ("disabled".into(), decimal_or_dash(status.reset_highest), decimal_or_dash(status.reset_threshold))
            }
            Some(ResetReading::Paused) => {
                ("paused".into(), decimal_or_dash(status.reset_highest), decimal_or_dash(status.reset_threshold))
            }
            Some(ResetReading::Similarity(similarity)) => {
                (decimal(similarity), decimal_or_dash(status.reset_highest), decimal_or_dash(status.reset_threshold))
            }
        };
        let buttons = status.buttons;
        let shown = Shown {
            running: status.running,
            device_name: session.device_name().unwrap_or(&profile.capture_device_name).into(),
            live_message: live_message.into(),
            live_image,
            clear_live_image,
            current_file: status.current_filename.clone().unwrap_or_else(|| "-".into()).into(),
            current_message: current_message.into(),
            current_image,
            table: [
                decimal_or_dash(status.similarity).into(),
                decimal_or_dash(status.highest).into(),
                decimal_or_dash(status.threshold).into(),
                reset_live.into(),
                reset_highest.into(),
                reset_threshold.into(),
            ],
            image_loop: status.loop_position.map_or("N/A".into(), |(at, of)| format!("{at}/{of}")).into(),
            start_status: match status.start_status {
                StartStatus::NotFound => "not found",
                StartStatus::Ready => "ready",
                StartStatus::DelayingStart => "delaying start...",
                StartStatus::Started => "started",
                StartStatus::NotReady => "not ready",
            }
            .into(),
            buttons: [
                buttons.start,
                buttons.reset,
                buttons.skip_split,
                buttons.undo_split,
                buttons.next_image,
                buttons.previous_image,
            ],
        };
        drop(frame);
        let _ = self.window.upgrade_in_event_loop(move |window| show(&window, shown));
    }

    fn dialog(&mut self, message: &str) {
        (self.on_dialog)(message.to_owned());
    }

    fn fps(&mut self, fps: Option<u32>) {
        let text: SharedString = fps.map_or(String::new(), |fps| fps.to_string()).into();
        let _ = self.window.upgrade_in_event_loop(move |window| window.set_fps_value(text));
    }

    fn profile_changed(&mut self, profile: &UserProfile) {
        (self.on_profile)(profile.clone());
    }

    fn profile_loaded(&mut self, path: &Path) {
        (self.on_loaded)(path.to_owned());
    }
}

fn show(window: &MainWindow, shown: Shown) {
    window.set_running(shown.running);
    window.set_device_name(if shown.device_name.is_empty() { "-".into() } else { shown.device_name });
    window.set_live_message(shown.live_message);
    if let Some(image) = shown.live_image {
        window.set_live_image(slint::Image::from_rgba8(image));
    } else if shown.clear_live_image {
        window.set_live_image(slint::Image::default());
    }
    window.set_current_image_file(shown.current_file);
    window.set_current_image_message(shown.current_message);
    if let Some(image) = shown.current_image {
        window.set_current_image(image.map(slint::Image::from_rgba8).unwrap_or_default());
    }
    let [current_live, current_highest, current_threshold, reset_live, reset_highest, reset_threshold] = shown.table;
    window.set_current_live(current_live);
    window.set_current_highest(current_highest);
    window.set_current_threshold(current_threshold);
    window.set_reset_live(reset_live);
    window.set_reset_highest(reset_highest);
    window.set_reset_threshold(reset_threshold);
    window.set_image_loop(shown.image_loop);
    window.set_start_image_status(shown.start_status);
    let [start, reset, skip, undo, next, previous] = shown.buttons;
    window.set_can_start(start);
    window.set_can_reset(reset);
    window.set_can_skip(skip);
    window.set_can_undo(undo);
    window.set_can_next(next);
    window.set_can_previous(previous);
}
