//! A single split image, matching AutoSplit's `AutoSplitImage.py`.

use std::path::{Path, PathBuf};

use crate::compare::{self, CompareScratch, ComparisonMethod, Mask, SourceFeatures};
use crate::filename::{self, BELOW_FLAG, DUMMY_FLAG, FilenameWarning, PAUSE_FLAG, RESET_KEYWORD, START_KEYWORD};
use crate::image::{BgraImage, BgraView};
use crate::imgproc::{self, ResizeBuffers};

const COMPARISON_RESIZE_WIDTH: usize = 320;
const COMPARISON_RESIZE_HEIGHT: usize = 240;
const COMPARISON_RESIZE_AREA: f64 = (COMPARISON_RESIZE_WIDTH * COMPARISON_RESIZE_HEIGHT) as f64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageType {
    Split,
    Reset,
    Start,
}

#[derive(Clone, Debug)]
pub enum SplitContent {
    Image {
        pixels: BgraImage,
        /// Where the original alpha was at least 1, for images with transparency.
        mask: Option<Mask>,
        features: SourceFeatures,
    },
    /// The file can't be used. Validation reports it before a run can start.
    Invalid(LoadError),
}

/// Why a file in the split image folder can't be used.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum LoadError {
    #[error("'{}' couldn't be read as an image.", file_name(.0))]
    Unreadable(PathBuf),
    #[error("'{}' isn't a PNG file. Only PNG split images are supported.", file_name(.0))]
    NotPng(PathBuf),
    #[error("'{}' is an OCR split file. OCR split files aren't supported yet.", file_name(.0))]
    OcrUnsupported(PathBuf),
    #[error("'{}' is fully transparent, so there's nothing to compare.", file_name(.0))]
    FullyTransparent(PathBuf),
}

/// A filename value that's ignored, with the file it's in.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
#[error("'{}': {warning}", file_name(.path))]
pub struct LoadWarning {
    pub path: PathBuf,
    pub warning: FilenameWarning,
}

/// Values a split image falls back to when its filename doesn't set them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplitDefaults {
    pub comparison_method: i64,
    pub similarity_threshold: f64,
    /// Milliseconds
    pub delay_time: i64,
    /// Seconds
    pub pause_time: f64,
    pub fps_limit: i64,
}

#[derive(Clone, Debug)]
pub struct SplitImage {
    pub path: PathBuf,
    /// Lowercased file name, which is what every filename convention is read from.
    pub filename: String,
    pub image_type: ImageType,
    pub loops: i64,
    pub content: SplitContent,
    /// Filename values that are ignored. The image still loads, like in AutoSplit.
    pub warnings: Vec<FilenameWarning>,
    flags: u32,
    delay_time: Option<i64>,
    comparison_method: Option<i64>,
    pause_time: Option<f64>,
    similarity_threshold: Option<f64>,
}

impl SplitImage {
    pub fn load(path: &Path) -> Self {
        let filename = path.file_name().map(|name| name.to_string_lossy().to_lowercase()).unwrap_or_default();
        let image_type = if filename::has_keyword(&filename, START_KEYWORD) {
            ImageType::Start
        } else if filename::has_keyword(&filename, RESET_KEYWORD) {
            ImageType::Reset
        } else {
            ImageType::Split
        };
        let content = if path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("txt")) {
            SplitContent::Invalid(LoadError::OcrUnsupported(path.to_owned()))
        } else {
            load_image(path)
        };
        Self {
            path: path.to_owned(),
            image_type,
            flags: filename::flags_from_filename(&filename),
            loops: filename::loop_from_filename(&filename),
            delay_time: filename::delay_time_from_filename(&filename),
            comparison_method: filename::comparison_method_from_filename(&filename),
            pause_time: filename::pause_from_filename(&filename),
            similarity_threshold: filename::threshold_from_filename(&filename),
            warnings: filename::filename_warnings(&filename),
            filename,
            content,
        }
    }

    /// The file name as it is on disk. [`SplitImage::filename`] is lowercased.
    pub fn display_name(&self) -> String {
        file_name(&self.path)
    }

    pub fn load_error(&self) -> Option<&LoadError> {
        match &self.content {
            SplitContent::Invalid(error) => Some(error),
            SplitContent::Image { .. } => None,
        }
    }

    pub fn is_dummy(&self) -> bool {
        self.flags & DUMMY_FLAG != 0
    }

    pub fn is_below(&self) -> bool {
        self.flags & BELOW_FLAG != 0
    }

    pub fn is_pause(&self) -> bool {
        self.flags & PAUSE_FLAG != 0
    }

    /// Whether the file name sets the threshold, instead of the profile.
    pub fn names_threshold(&self) -> bool {
        self.similarity_threshold.is_some()
    }

    /// Whether the file name sets the comparison method, instead of the profile.
    pub fn names_comparison_method(&self) -> bool {
        self.comparison_method.is_some()
    }

    /// Whether the file name sets the delay, instead of the profile.
    pub fn names_delay(&self) -> bool {
        self.delay_time.is_some()
    }

    pub fn delay_time_ms(&self, defaults: &SplitDefaults) -> i64 {
        self.delay_time.unwrap_or(defaults.delay_time)
    }

    pub fn pause_time_secs(&self, defaults: &SplitDefaults) -> f64 {
        self.pause_time.unwrap_or(defaults.pause_time)
    }

    pub fn similarity_threshold(&self, defaults: &SplitDefaults) -> f64 {
        self.similarity_threshold.unwrap_or(defaults.similarity_threshold)
    }

    /// The `^n^` method, or the profile's. `None` for indexes that always compare as 0.
    pub fn comparison_method(&self, defaults: &SplitDefaults) -> Option<ComparisonMethod> {
        ComparisonMethod::from_index(self.comparison_method.unwrap_or(defaults.comparison_method))
    }

    /// `compare_with_capture`. A missing or empty capture (lost window or device) compares as 0.
    pub fn compare(
        &self,
        capture: Option<BgraView>,
        method: Option<ComparisonMethod>,
        scratch: &mut CompareScratch,
    ) -> f64 {
        let (Some(capture), Some(method)) = (capture.filter(|capture| !capture.is_empty()), method) else {
            return 0.0;
        };
        let SplitContent::Image { pixels, mask, features } = &self.content else {
            return 0.0;
        };
        let size = (pixels.width(), pixels.height());
        let CompareScratch { resize, resized, resized_visible, masked, resized_for_frame, frame_active, .. } = scratch;
        let resized_capture = match mask {
            _ if *resized_for_frame == Some(size) => &*resized,
            // The comparisons only read the visible pixels
            Some(mask) if size != (capture.width(), capture.height()) => {
                imgproc::resize_nearest_to_spans(capture, size.0, size.1, mask.spans(), resize, resized_visible);
                &*resized_visible
            }
            _ => {
                imgproc::resize_nearest_to(capture, size.0, size.1, resize, resized);
                if *frame_active {
                    *resized_for_frame = Some(size);
                }
                &*resized
            }
        };
        compare::compare_resized(method, pixels, features, resized_capture, mask.as_ref(), masked, resize)
    }
}

fn load_image(path: &Path) -> SplitContent {
    let invalid = |error: fn(PathBuf) -> LoadError| SplitContent::Invalid(error(path.to_owned()));
    let Ok(bytes) = std::fs::read(path) else {
        return invalid(LoadError::Unreadable);
    };
    if !BgraImage::is_png(&bytes) {
        return invalid(LoadError::NotPng);
    }
    let image = match BgraImage::decode_png(&bytes) {
        Ok(image) if !image.is_empty() => image,
        _ => return invalid(LoadError::Unreadable),
    };
    if image.data().as_chunks::<4>().0.iter().all(|pixel| pixel[3] == 0) {
        return invalid(LoadError::FullyTransparent);
    }
    let (pixels, mask) = prepare_for_comparison(&image);
    let features = SourceFeatures::new(&pixels, mask.as_ref());
    SplitContent::Image { pixels, mask, features }
}

/// Resizes a split image the way AutoSplit does before comparing, and builds its mask.
///
/// Opaque images become 320x240. Images with transparency are scaled so that their visible
/// area is about 320x240 worth of pixels, but never upscaled.
fn prepare_for_comparison(image: &BgraImage) -> (BgraImage, Option<Mask>) {
    let mut buffers = ResizeBuffers::default();
    let mut resized = BgraImage::default();
    let alpha = || image.data().as_chunks::<4>().0.iter().map(|pixel| pixel[3]);

    let alpha_sum: u64 = alpha().map(u64::from).sum();
    let pixel_count = (image.width() * image.height()) as u64;
    let has_transparency = alpha_sum != 255 * pixel_count;
    if !has_transparency {
        imgproc::resize_nearest_to(
            image.view(),
            COMPARISON_RESIZE_WIDTH,
            COMPARISON_RESIZE_HEIGHT,
            &mut buffers,
            &mut resized,
        );
        return (resized, None);
    }

    let visible = alpha().filter(|&a| a != 0).count() as f64;
    let scale = (COMPARISON_RESIZE_AREA / visible).sqrt().min(1.0);
    // cv2.resize(dsize=None, fx=scale, fy=scale) rounds the size with cvRound. A size of 0 only
    // happens for absurd aspect ratios, where AutoSplit fails; 1 keeps the image usable.
    let width = ((image.width() as f64 * scale).round_ties_even() as usize).max(1);
    let height = ((image.height() as f64 * scale).round_ties_even() as usize).max(1);
    imgproc::resize_nearest(image.view(), width, height, scale, scale, &mut buffers, &mut resized);
    let mask = resized.data().as_chunks::<4>().0.iter().map(|pixel| if pixel[3] >= 1 { 255 } else { 0 }).collect();
    (resized, Some(Mask::new(mask)))
}

/// The file name, which is enough to find a file in the split image folder.
pub(crate) fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned())
}
