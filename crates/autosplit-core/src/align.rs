//! AutoSplit's Align Region (`region_selection.align_region`): finds where a reference image is
//! in the capture region, trying it at 56 sizes from 20% to 300%, and returns the best match.
//!
//! Like AutoSplit, each size is compared with OpenCV's `TM_SQDIFF` over all 4 channels. The sums
//! of products are computed with FFTs, which is what makes this take a moment rather than minutes.

use rustfft::FftPlanner;
use rustfft::num_complex::Complex;

use crate::image::{BgraImage, BgraView};
use crate::imgproc::{ResizeBuffers, resize_nearest_to};

/// The similarity a match needs, like AutoSplit's `ALIGN_REGION_THRESHOLD`.
pub const ALIGN_THRESHOLD: f64 = 0.9;
const SCALES: usize = 56;
const MIN_SCALE: f64 = 0.2;
const MAX_SCALE: f64 = 3.0;

/// Where the reference image matched, in pixels of the capture region.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aligned {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
    /// 1 for a perfect match.
    pub similarity: f64,
}

/// A 2D FFT of `height` rows of `width`, done as rows then columns.
struct Fft2 {
    width: usize,
    rows: std::sync::Arc<dyn rustfft::Fft<f64>>,
    columns: std::sync::Arc<dyn rustfft::Fft<f64>>,
    column: Vec<Complex<f64>>,
}

impl Fft2 {
    fn new(planner: &mut FftPlanner<f64>, width: usize, height: usize, inverse: bool) -> Self {
        let (rows, columns) = if inverse {
            (planner.plan_fft_inverse(width), planner.plan_fft_inverse(height))
        } else {
            (planner.plan_fft_forward(width), planner.plan_fft_forward(height))
        };
        Self { width, rows, columns, column: vec![Complex::default(); height] }
    }

    fn process(&mut self, data: &mut [Complex<f64>]) {
        self.rows.process(data);
        for x in 0..self.width {
            for (y, value) in self.column.iter_mut().enumerate() {
                *value = data[y * self.width + x];
            }
            self.columns.process(&mut self.column);
            for (y, value) in self.column.iter().enumerate() {
                data[y * self.width + x] = *value;
            }
        }
    }
}

/// `np.linspace(0.2, 3, num=56)`.
fn scales() -> impl Iterator<Item = f64> {
    let step = (MAX_SCALE - MIN_SCALE) / (SCALES - 1) as f64;
    (0..SCALES).map(move |i| if i == SCALES - 1 { MAX_SCALE } else { MIN_SCALE + i as f64 * step })
}

/// Finds `template` in `capture`. `None` when no size matches with at least
/// [`ALIGN_THRESHOLD`], or the template never fits.
pub fn align_region(capture: BgraView, template: BgraView) -> Option<Aligned> {
    let (width, height) = (capture.width(), capture.height());
    if width == 0 || height == 0 || template.is_empty() {
        return None;
    }
    let mut planner = FftPlanner::new();
    let mut forward = Fft2::new(&mut planner, width, height, false);
    let mut inverse = Fft2::new(&mut planner, width, height, true);

    // The capture's channels, transformed once
    let spectra: Vec<Vec<Complex<f64>>> = (0..4)
        .map(|channel| {
            let mut plane: Vec<Complex<f64>> = (0..height)
                .flat_map(|y| capture.row(y).iter().skip(channel).step_by(4).map(|&v| Complex::new(f64::from(v), 0.0)))
                .collect();
            forward.process(&mut plane);
            plane
        })
        .collect();
    // Sums of squares over any window, from a summed-area table
    let mut squares = vec![0.0_f64; (width + 1) * (height + 1)];
    for y in 0..height {
        let mut row_sum = 0.0;
        for (x, pixel) in capture.row(y).as_chunks::<4>().0.iter().enumerate() {
            row_sum += pixel.iter().map(|&v| f64::from(v) * f64::from(v)).sum::<f64>();
            squares[(y + 1) * (width + 1) + x + 1] = squares[y * (width + 1) + x + 1] + row_sum;
        }
    }
    let window_squares = |x: usize, y: usize, w: usize, h: usize| {
        let at = |x: usize, y: usize| squares[y * (width + 1) + x];
        at(x + w, y + h) - at(x, y + h) - at(x + w, y) + at(x, y)
    };

    let mut best: Option<Aligned> = None;
    let mut buffers = ResizeBuffers::default();
    let mut resized = BgraImage::default();
    let mut product = vec![Complex::default(); width * height];
    let mut plane = vec![Complex::default(); width * height];
    for scale in scales() {
        let w = (template.width() as f64 * scale) as usize;
        let h = (template.height() as f64 * scale) as usize;
        if w == 0 || h == 0 || w > width || h > height {
            continue;
        }
        resize_nearest_to(template, w, h, &mut buffers, &mut resized);
        let template_squares: f64 = resized.data().iter().map(|&v| f64::from(v) * f64::from(v)).sum();

        // Sum over channels of the capture correlated with the template, in one inverse FFT
        product.iter_mut().for_each(|value| *value = Complex::default());
        for (channel, spectrum) in spectra.iter().enumerate() {
            plane.iter_mut().for_each(|value| *value = Complex::default());
            for (y, row) in resized.data().chunks_exact(w * 4).enumerate() {
                for (x, pixel) in row.as_chunks::<4>().0.iter().enumerate() {
                    plane[y * width + x] = Complex::new(f64::from(pixel[channel]), 0.0);
                }
            }
            forward.process(&mut plane);
            for ((sum, capture), template) in product.iter_mut().zip(spectrum).zip(&plane) {
                *sum += capture * template.conj();
            }
        }
        inverse.process(&mut product);
        let normalize = (width * height) as f64;

        // `cv2.minMaxLoc`: the first smallest difference, scanning rows
        let mut minimum = (f64::INFINITY, 0, 0);
        for y in 0..=height - h {
            for x in 0..=width - w {
                let correlation = product[y * width + x].re / normalize;
                let difference = (window_squares(x, y, w, h) - 2.0 * correlation + template_squares).max(0.0);
                if difference < minimum.0 {
                    minimum = (difference, x, y);
                }
            }
        }
        let max_error = (w * h * 4) as f64 * 255.0 * 255.0;
        let similarity = 1.0 - minimum.0 / max_error;
        if best.is_none_or(|best| similarity > best.similarity) {
            best = Some(Aligned { x: minimum.1, y: minimum.2, width: w, height: h, similarity });
        }
    }
    best.filter(|best| best.similarity >= ALIGN_THRESHOLD)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scales_are_numpys_linspace() {
        // Act
        let scales: Vec<f64> = scales().collect();

        // Assert
        assert_eq!((scales.len(), scales[0], scales[55]), (56, 0.2, 3.0));
    }
}
