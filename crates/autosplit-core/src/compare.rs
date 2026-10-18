//! The three image comparison methods, matching AutoSplit's `compare.py`.
//!
//! They compare a split image with a capture already resized to the split image's size,
//! using the split image's mask when it has transparency.

use std::ops::Range;
use std::sync::OnceLock;

use crate::image::BgraImage;
use crate::imgproc::{self, ResizeBuffers};

const MAXBYTE: f64 = 255.0;
const PHASH_INPUT_SIZE: usize = 8;
const PHASH_DCT_SIZE: usize = 32;
const HISTOGRAM_BINS: usize = 8 * 8 * 8;

/// The comparison methods, by the index used in profiles and in `^n^` filename flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComparisonMethod {
    L2Norm,
    Histograms,
    PHash,
}

impl ComparisonMethod {
    pub const ALL: [ComparisonMethod; 3] = [Self::L2Norm, Self::Histograms, Self::PHash];

    /// `None` for indexes AutoSplit maps to its dummy comparison, which always returns 0.
    pub fn from_index(index: i64) -> Option<Self> {
        match index {
            0 => Some(Self::L2Norm),
            1 => Some(Self::Histograms),
            2 => Some(Self::PHash),
            _ => None,
        }
    }

    /// The name AutoSplit's settings window uses.
    pub fn name(self) -> &'static str {
        match self {
            Self::L2Norm => "L2 Norm",
            Self::Histograms => "Histograms",
            Self::PHash => "pHash",
        }
    }

    pub fn index(self) -> i64 {
        match self {
            Self::L2Norm => 0,
            Self::Histograms => 1,
            Self::PHash => 2,
        }
    }
}

/// A split image's transparency mask: 255 where its alpha is at least 1, 0 elsewhere.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mask {
    bytes: Vec<u8>,
    /// Runs of visible pixels. Masks are usually a few large shapes, so working run by run
    /// avoids testing every pixel.
    spans: Vec<Range<usize>>,
    count: usize,
}

impl Mask {
    /// One byte per pixel, non-zero where visible.
    pub fn new(bytes: Vec<u8>) -> Self {
        let mut spans: Vec<Range<usize>> = Vec::new();
        for (index, _) in bytes.iter().enumerate().filter(|(_, value)| **value != 0) {
            match spans.last_mut() {
                Some(span) if span.end == index => span.end += 1,
                _ => spans.push(index..index + 1),
            }
        }
        let count = spans.iter().map(ExactSizeIterator::len).sum();
        Self { bytes, spans, count }
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn spans(&self) -> &[Range<usize>] {
        &self.spans
    }

    pub(crate) fn byte_spans(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.spans.iter().map(|span| span.start * 4..span.end * 4)
    }
}

/// What the histogram and pHash methods compute from the split image. It never changes, so
/// it's computed once at load instead of on every frame.
#[derive(Clone, Debug)]
pub struct SourceFeatures {
    histogram: Box<[f32; HISTOGRAM_BINS]>,
    phash: u64,
}

impl SourceFeatures {
    pub(crate) fn new(source: &BgraImage, mask: Option<&Mask>) -> Self {
        let mut scratch = CompareScratch::default();
        Self {
            histogram: Box::new(normalized_histogram(source, mask)),
            phash: phash_of(source, mask, &mut scratch.masked, &mut scratch.resize),
        }
    }
}

/// Working memory for comparisons, kept between frames so they don't allocate.
#[derive(Default)]
pub struct CompareScratch {
    pub(crate) resize: ResizeBuffers,
    /// The capture resized to a split image's full size.
    pub(crate) resized: BgraImage,
    /// The capture resized for a masked image, with only its visible pixels filled in.
    pub(crate) resized_visible: BgraImage,
    pub(crate) masked: Vec<u8>,
    /// Size `resized` holds for the current frame, so a split image and the reset image of the
    /// same size share one resize.
    pub(crate) resized_for_frame: Option<(usize, usize)>,
    /// Only the runner starts frames, so callers comparing one-off images never reuse a resize.
    pub(crate) frame_active: bool,
    /// The frame `resized_for_frame` belongs to.
    frame: Option<u64>,
}

impl CompareScratch {
    /// Forgets resizes of the previous frame, unless `frame` is that frame again. The runner
    /// calls this once per tick.
    pub(crate) fn begin_frame(&mut self, frame: u64) {
        self.frame_active = true;
        if self.frame != Some(frame) {
            self.frame = Some(frame);
            self.resized_for_frame = None;
        }
    }
}

/// Compares `source` with `resized_capture`, which has `source`'s size.
pub(crate) fn compare_resized(
    method: ComparisonMethod,
    source: &BgraImage,
    features: &SourceFeatures,
    resized_capture: &BgraImage,
    mask: Option<&Mask>,
    masked: &mut Vec<u8>,
    resize: &mut ResizeBuffers,
) -> f64 {
    match method {
        ComparisonMethod::L2Norm => compare_l2_norm(source, resized_capture, mask),
        ComparisonMethod::Histograms => {
            1.0 - bhattacharyya(&features.histogram, &normalized_histogram(resized_capture, mask))
        }
        ComparisonMethod::PHash => {
            let capture_hash = phash_of(resized_capture, mask, masked, resize);
            1.0 - (features.phash ^ capture_hash).count_ones() as f64 / 64.0
        }
    }
}

/// `compare_l2_norm`: `1 - cv2.norm(source, capture, NORM_L2, mask) / max_error`.
///
/// The error runs over all 4 channels, alpha included. Without a mask the maximum error also
/// counts 4 channels, but with a mask it only counts 3, so semi-transparent split pixels push
/// the similarity down (and it can go below 0). AutoSplit behaves the same way.
fn compare_l2_norm(source: &BgraImage, capture: &BgraImage, mask: Option<&Mask>) -> f64 {
    debug_assert_eq!(source.data().len(), capture.data().len());
    let (source, capture) = (source.data(), capture.data());
    let (squared_error, max_error) = match mask {
        None => (squared_error(source, capture), (source.len() as f64).sqrt() * MAXBYTE),
        Some(mask) => (
            mask.byte_spans().map(|span| squared_error(&source[span.clone()], &capture[span])).sum(),
            (mask.count as f64 * 3.0 * MAXBYTE * MAXBYTE).sqrt(),
        ),
    };
    if max_error == 0.0 {
        return 0.0;
    }
    1.0 - (squared_error as f64).sqrt() / max_error
}

/// Sum of squared byte differences. Exact integer math, so any evaluation order gives the
/// same result as OpenCV.
fn squared_error(a: &[u8], b: &[u8]) -> u64 {
    // 4096 * 255^2 fits in the i32 lanes
    a.chunks(4096).zip(b.chunks(4096)).map(|(a, b)| squared_error_block(a, b)).sum()
}

/// Written as 16 independent lanes so LLVM vectorizes it (about twice as fast as a plain loop).
fn squared_error_block(a: &[u8], b: &[u8]) -> u64 {
    let (a_blocks, a_rest) = a.as_chunks::<16>();
    let (b_blocks, b_rest) = b.as_chunks::<16>();
    let mut lanes = [0_i32; 16];
    for (x, y) in a_blocks.iter().zip(b_blocks) {
        for k in 0..16 {
            let difference = (x[k] as i16 - y[k] as i16) as i32;
            lanes[k] += difference * difference;
        }
    }
    let rest: i32 = a_rest.iter().zip(b_rest).map(|(&x, &y)| (x as i32 - y as i32).pow(2)).sum();
    lanes.iter().map(|&lane| lane as u64).sum::<u64>() + rest as u64
}

/// `cv2.calcHist([image], (2, 1, 0), mask, (8, 8, 8), (0, 256) * 3)` then `cv2.normalize`.
fn normalized_histogram(image: &BgraImage, mask: Option<&Mask>) -> [f32; HISTOGRAM_BINS] {
    // Four tables, so runs of same-colored pixels don't wait on each other's increments
    let mut tables = [[0_u32; HISTOGRAM_BINS]; 4];
    let bin = |pixel: &[u8; 4]| (pixel[2] as usize >> 5) * 64 + (pixel[1] as usize >> 5) * 8 + (pixel[0] as usize >> 5);
    let mut count = |pixels: &[u8]| {
        let (quads, rest) = pixels.as_chunks::<16>();
        for quad in quads {
            for (table, pixel) in tables.iter_mut().zip(quad.as_chunks::<4>().0) {
                table[bin(pixel)] += 1;
            }
        }
        for pixel in rest.as_chunks::<4>().0 {
            tables[0][bin(pixel)] += 1;
        }
    };
    match mask {
        None => count(image.data()),
        Some(mask) => mask.byte_spans().for_each(|span| count(&image.data()[span])),
    }
    let counts: [u32; HISTOGRAM_BINS] = std::array::from_fn(|i| tables.iter().map(|table| table[i]).sum());

    // The sum of squares is an integer below 2^53, so it's exact in any summation order
    let norm = counts.iter().map(|&count| count as f64 * count as f64).sum::<f64>().sqrt();
    let scale = if norm > f64::EPSILON { (1.0 / norm) as f32 } else { 0.0 };
    counts.map(|count| count as f32 * scale)
}

/// `cv2.compareHist(..., HISTCMP_BHATTACHARYYA)`.
///
/// Sums in two interleaved f64 lanes like OpenCV's SSE2/NEON path, so the rounding matches.
fn bhattacharyya(h1: &[f32; HISTOGRAM_BINS], h2: &[f32; HISTOGRAM_BINS]) -> f64 {
    let mut s1 = [0.0_f64; 2];
    let mut s2 = [0.0_f64; 2];
    let mut result = [0.0_f64; 2];
    for (index, (&a, &b)) in h1.iter().zip(h2).enumerate() {
        let (a, b) = (a as f64, b as f64);
        let lane = index % 2;
        s1[lane] += a;
        s2[lane] += b;
        result[lane] += (a * b).sqrt();
    }
    let s1 = (s1[0] + s1[1]) * (s2[0] + s2[1]);
    let s1 = if s1.abs() > f32::EPSILON as f64 { 1.0 / s1.sqrt() } else { 1.0 };
    (1.0 - (result[0] + result[1]) * s1).max(0.0).sqrt()
}

/// `compare_phash`'s per-image part: zero masked-out pixels, shrink to 8x8 with INTER_AREA,
/// then hash.
fn phash_of(image: &BgraImage, mask: Option<&Mask>, masked: &mut Vec<u8>, resize: &mut ResizeBuffers) -> u64 {
    let pixels = match mask {
        None => image.data(),
        Some(mask) => {
            masked.clear();
            masked.resize(image.data().len(), 0);
            for span in mask.byte_spans() {
                masked[span.clone()].copy_from_slice(&image.data()[span]);
            }
            masked
        }
    };
    let mut small = [0_u8; PHASH_INPUT_SIZE * PHASH_INPUT_SIZE * 4];
    imgproc::resize_area(
        pixels,
        image.width(),
        image.height(),
        4,
        PHASH_INPUT_SIZE,
        PHASH_INPUT_SIZE,
        resize,
        &mut small,
    );
    phash(&small, resize)
}

/// `cv::img_hash::PHash::compute` on an 8x8 BGRA image, as a 64-bit hash.
///
/// Bit `i` (row-major over the top-left 8x8 DCT coefficients) is set when the coefficient is
/// greater than their mean, with the DC term zeroed. This is the same bit layout as OpenCV's
/// 8 hash bytes read little-endian.
#[doc(hidden)]
pub fn phash(bgra_8x8: &[u8; PHASH_INPUT_SIZE * PHASH_INPUT_SIZE * 4], resize: &mut ResizeBuffers) -> u64 {
    let mut upscaled = [0_u8; PHASH_DCT_SIZE * PHASH_DCT_SIZE * 4];
    imgproc::resize_linear_exact(
        bgra_8x8,
        PHASH_INPUT_SIZE,
        PHASH_INPUT_SIZE,
        4,
        PHASH_DCT_SIZE,
        PHASH_DCT_SIZE,
        resize,
        &mut upscaled,
    );
    let mut gray = [0_u8; PHASH_DCT_SIZE * PHASH_DCT_SIZE];
    imgproc::bgr_to_gray(&upscaled, 4, &mut gray);

    let mut coefficients = dct_top_left(&gray);
    coefficients[0] = 0.0;
    let mean = (coefficients.iter().map(|&c| c as f64).sum::<f64>() / 64.0) as f32;
    coefficients.iter().enumerate().fold(0_u64, |hash, (i, &c)| hash | (u64::from(c > mean) << i))
}

/// Top-left 8x8 of the orthonormal 2D DCT-II of a 32x32 image (what `cv::dct` computes),
/// without the DC term, which pHash discards.
///
/// Computed in f64 and rounded to f32, where OpenCV computes in f32 throughout. A hash bit can
/// differ when a coefficient is within float noise of the mean.
fn dct_top_left(gray: &[u8; PHASH_DCT_SIZE * PHASH_DCT_SIZE]) -> [f32; 64] {
    let basis = dct_basis();
    // Subtracting a constant only changes the DC term. It makes flat images give exact zeros
    // like OpenCV does, instead of rounding noise that would set random hash bits.
    let offset = gray[0] as f64;
    let mut rows = [[0.0_f64; 8]; PHASH_DCT_SIZE];
    for (y, row) in rows.iter_mut().enumerate() {
        let pixels = &gray[y * PHASH_DCT_SIZE..(y + 1) * PHASH_DCT_SIZE];
        for (v, out) in row.iter_mut().enumerate() {
            *out = pixels.iter().zip(&basis[v]).map(|(&p, &b)| (p as f64 - offset) * b).sum();
        }
    }
    let mut out = [0.0_f32; 64];
    for u in 0..8 {
        for v in 0..8 {
            out[u * 8 + v] = rows.iter().zip(&basis[u]).map(|(row, &b)| row[v] * b).sum::<f64>() as f32;
        }
    }
    out
}

fn dct_basis() -> &'static [[f64; PHASH_DCT_SIZE]; 8] {
    static BASIS: OnceLock<[[f64; PHASH_DCT_SIZE]; 8]> = OnceLock::new();
    BASIS.get_or_init(|| {
        let n = PHASH_DCT_SIZE as f64;
        std::array::from_fn(|k| {
            let scale = if k == 0 { (1.0 / n).sqrt() } else { (2.0 / n).sqrt() };
            std::array::from_fn(|i| scale * (std::f64::consts::PI * (2 * i + 1) as f64 * k as f64 / (2.0 * n)).cos())
        })
    })
}
