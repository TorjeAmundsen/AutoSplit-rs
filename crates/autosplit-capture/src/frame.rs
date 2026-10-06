use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use autosplit_core::image::{BgraImage, BgraView, Rect};
use autosplit_core::profile::Region;

use crate::convert::{Converted, Layout, PixelFormat, yuv_to_bgra};

/// How long without a new frame before the capture counts as lost. OpenCV's DirectShow read
/// waits one second for a frame, after which AutoSplit has no frame to compare.
pub const FRAME_TIMEOUT: Duration = Duration::from_secs(1);

/// Spare frame buffers, so capturing doesn't allocate once it's running.
type Pool = Mutex<Vec<BgraImage>>;

const POOL_SIZE: usize = 2;

/// Frame numbers, unique across devices, so a reopened device's frames are never mistaken for
/// frames compared before.
static NEXT_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// A captured frame, in BGRA with alpha 255. Usually only the capture region is converted
/// (see [`FrameSlot::set_region`]). Its buffer goes back to the capture for reuse when the last
/// reference is dropped.
#[derive(Debug)]
pub struct Frame {
    /// The converted part of the frame.
    image: BgraImage,
    /// Where `image` is in the frame.
    area: Rect,
    frame_size: (usize, usize),
    sequence: u64,
    pool: Weak<Pool>,
}

impl Frame {
    /// Different for every frame.
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    /// The whole frame's width and height.
    pub fn frame_size(&self) -> (usize, usize) {
        self.frame_size
    }

    /// The part of the frame `region` covers, as AutoSplit crops it. `None` when that part
    /// wasn't converted, which happens briefly after the region changes.
    pub fn region_view(&self, region: &Region) -> Option<BgraView<'_>> {
        let (width, height) = self.frame_size;
        let rect = region.rect(width, height);
        if !self.area.contains(&rect) {
            return None;
        }
        let area = self.area;
        let relative = Rect {
            left: rect.left - area.left,
            top: rect.top - area.top,
            right: rect.right - area.left,
            bottom: rect.bottom - area.top,
        };
        Some(self.image.view().crop_rect(relative))
    }

    /// The whole frame, when it was converted whole.
    pub fn full_view(&self) -> Option<BgraView<'_>> {
        (self.area == Rect::full(self.frame_size.0, self.frame_size.1)).then(|| self.image.view())
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        if let Some(pool) = self.pool.upgrade() {
            recycle(&pool, std::mem::take(&mut self.image));
        }
    }
}

fn recycle(pool: &Pool, image: BgraImage) {
    let mut pool = pool.lock().unwrap();
    if pool.len() < POOL_SIZE {
        pool.push(image);
    }
}

/// The area of a `width` by `height` frame to convert for `region`: all of it without one.
fn area_for(region: Option<&Region>, width: usize, height: usize) -> Rect {
    region.map_or(Rect::full(width, height), |region| region.rect(width, height))
}

/// The newest frame before conversion, kept while nobody needs converted frames.
#[derive(Default)]
struct Raw {
    data: Vec<u8>,
    layout: Option<Layout>,
    arrival: Option<Instant>,
    pending: bool,
    /// For converting the 4 corners of each frame.
    corners: BgraImage,
}

/// Hands the newest frame from a backend's capture thread to the caller.
///
/// Converting a 1080p frame takes milliseconds, so frames are only converted while the caller
/// says it needs them, and only its region. While it doesn't (between comparisons, like during
/// a split's pause), a raw frame is copied instead when that's cheaper, and the newest one is
/// converted when asked for.
#[doc(hidden)]
pub struct FrameSlot {
    latest: Mutex<Option<Arc<Frame>>>,
    pool: Arc<Pool>,
    raw: Mutex<Raw>,
    region: Mutex<Option<Region>>,
    base: Instant,
    /// Nanoseconds after `base` the last frame arrived, plus one. 0 before the first frame.
    last_received: AtomicU64,
    wanted: AtomicBool,
    failed: AtomicBool,
    placeholder_seen: AtomicBool,
}

impl Default for FrameSlot {
    fn default() -> Self {
        Self {
            latest: Mutex::default(),
            pool: Arc::default(),
            raw: Mutex::default(),
            region: Mutex::default(),
            base: Instant::now(),
            last_received: AtomicU64::new(0),
            wanted: AtomicBool::new(true),
            failed: AtomicBool::new(false),
            placeholder_seen: AtomicBool::new(false),
        }
    }
}

impl FrameSlot {
    /// The part of frames to convert, `None` for whole frames. Frames that arrive after this
    /// are converted for the new region.
    pub fn set_region(&self, region: Option<Region>) {
        *self.region.lock().unwrap() = region;
    }

    fn region(&self) -> Option<Region> {
        *self.region.lock().unwrap()
    }

    /// Whether frames should be converted as they arrive.
    pub fn set_wanted(&self, wanted: bool) {
        self.wanted.store(wanted, Ordering::Relaxed);
    }

    fn mark_received(&self, arrival: Instant) {
        let nanos = arrival.saturating_duration_since(self.base).as_nanos() as u64;
        self.last_received.fetch_max(nanos.saturating_add(1), Ordering::Relaxed);
    }

    /// Takes a frame from the device: converted right away when frames are wanted or the
    /// region is small, otherwise copied for later.
    ///
    /// OBS Virtual Camera sends frames filled with byte 127 while it isn't started. AutoSplit
    /// means to skip those by the color of the frame's 4 corners. They're skipped here too, so
    /// the previous frame stays current, and they don't count as frames.
    pub fn receive(&self, data: &[u8], layout: Layout) {
        let arrival = Instant::now();
        // Held while converting, so `latest` waits for this newer frame instead of returning
        // the one before it
        let mut raw = self.raw.lock().unwrap();
        if layout.corners(data, &mut raw.corners).is_some_and(|corners| corners.iter().all(|&c| c == placeholder())) {
            self.placeholder_seen.store(true, Ordering::Relaxed);
            return;
        }
        let region = self.region();
        let area = area_for(region.as_ref(), layout.width, layout.height);
        if self.wanted.load(Ordering::Relaxed) || layout.converting_is_cheaper(area) {
            raw.pending = false;
            self.convert_and_publish(data, layout, region.as_ref(), arrival);
            return;
        }
        raw.data.clear();
        raw.data.extend_from_slice(data);
        raw.layout = Some(layout);
        raw.arrival = Some(arrival);
        raw.pending = true;
        self.mark_received(arrival);
    }

    fn convert_and_publish(&self, data: &[u8], layout: Layout, region: Option<&Region>, arrival: Instant) {
        let mut image = self.pool.lock().unwrap().pop().unwrap_or_default();
        let area = area_for(region, layout.width, layout.height);
        // A conversion bug on a strange frame shouldn't take the program down
        let converted = match catch_unwind(AssertUnwindSafe(|| layout.convert(data, area, &mut image))) {
            Ok(Some(converted)) => converted,
            Ok(None) => return recycle(&self.pool, image),
            Err(_) => return self.fail(),
        };
        // MJPEG frames are only known after decoding them, whole
        if layout.format == PixelFormat::Mjpeg && is_obs_placeholder(&image) {
            self.placeholder_seen.store(true, Ordering::Relaxed);
            return recycle(&self.pool, image);
        }
        self.mark_received(arrival);
        let Converted { area, frame_size } = converted;
        let sequence = NEXT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let frame = Arc::new(Frame { image, area, frame_size, sequence, pool: Arc::downgrade(&self.pool) });
        // The previous frame's buffer is recycled once whoever still holds it lets go
        let previous = self.latest.lock().unwrap().replace(frame);
        drop(previous);
    }

    /// Marks the device as gone, e.g. unplugged.
    pub fn fail(&self) {
        self.failed.store(true, Ordering::Relaxed);
    }

    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::Relaxed)
    }

    /// Whether OBS Virtual Camera's placeholder arrived, meaning the camera isn't started in OBS.
    pub fn placeholder_seen(&self) -> bool {
        self.placeholder_seen.load(Ordering::Relaxed)
    }

    /// Whether a frame arrived within [`FRAME_TIMEOUT`] of `now`.
    pub fn receiving(&self, now: Instant) -> bool {
        match self.last_received.load(Ordering::Relaxed) {
            0 => false,
            received => {
                let received = self.base + Duration::from_nanos(received - 1);
                now.saturating_duration_since(received) <= FRAME_TIMEOUT
            }
        }
    }

    /// The newest frame, or `None` when none arrived within [`FRAME_TIMEOUT`]. A frame that
    /// was kept unconverted is converted now.
    pub fn latest(&self, now: Instant) -> Option<Arc<Frame>> {
        {
            let mut raw = self.raw.lock().unwrap();
            if raw.pending
                && let (Some(layout), Some(arrival)) = (raw.layout, raw.arrival)
            {
                raw.pending = false;
                self.convert_and_publish(&raw.data, layout, self.region().as_ref(), arrival);
            }
        }
        if !self.receiving(now) {
            return None;
        }
        self.latest.lock().unwrap().clone()
    }
}

fn placeholder() -> [u8; 4] {
    yuv_to_bgra(127, 127, 127)
}

/// Whether a converted frame's 4 corners have OBS Virtual Camera's placeholder color.
pub fn is_obs_placeholder(image: &BgraImage) -> bool {
    if image.is_empty() {
        return false;
    }
    let (pixels, _) = image.data().as_chunks::<4>();
    let (width, height) = (image.width(), image.height());
    [0, width - 1, (height - 1) * width, height * width - 1].iter().all(|&i| pixels[i] == placeholder())
}
