//! Fade images (`{f}`): how close the capture is to the image's color, and whether it got there
//! by a gradual fade rather than a cut. Not in AutoSplit.

use std::collections::VecDeque;
use std::time::Duration;

use crate::compare::Mask;
use crate::image::{BgraImage, BgraView};
use crate::split_image::SplitImage;
use crate::text::hex_color;

/// The largest RGB distance: from black to white.
const MAX_DISTANCE: f64 = 441.672_955_930_063_7; // 255 * sqrt(3)

/// How much further from the color than the current frame a fade has to have started, at least.
/// Lower thresholds ask for more: their `1 - threshold`.
const MIN_FADE_DEPTH: f64 = 0.08;
/// The most one sample may make of a fade, in the last steps to the threshold. A cut makes all of
/// it in one, and a cut that a capture card blends over a frame half of it.
const MAX_STEP_SHARE: f64 = 0.4;
/// The steps to the threshold that [`MAX_STEP_SHARE`] applies to. Earlier steps can be big, like
/// when something dark crosses a small region as the fade starts.
const ARRIVAL_STEPS: usize = 3;
/// The most one of the [`ARRIVAL_STEPS`] may make of their rise. A fade still rises in the steps
/// before the last one, while a cut after something that held still, like a dark room entered
/// with a cut, makes all of it in one. Without this, the earlier cut would count toward the
/// fade's depth and make the later one look small. A fade can still make up to about 0.83 of it
/// in one, when the camera turns toward something dark just before the end, and cuts make 0.95
/// and up.
const MAX_ARRIVAL_SHARE: f64 = 0.9;
/// How much bigger than the steps after it a step of a fade may be. A fade's steps grow or shrink
/// a little at a time, while a cut is a jump between small ones. So a cut that lands just under the
/// threshold and then creeps up to it doesn't count as the gradual part of a fade, which has to be
/// [`MIN_FADE_DEPTH`] deep on its own.
const MAX_STEP_GROWTH: f64 = 4.0;
/// How far a fade may go back from one sample to the next, like a game's fades sometimes do.
const MAX_DIP: f64 = 0.03;
const MIN_SAMPLE_INTERVAL: Duration = Duration::from_millis(50);
const MAX_SAMPLE_INTERVAL: Duration = Duration::from_millis(100);

/// The gray that noise is measured on besides black: OoT's fades to white, #a0a0a0.
pub const NOISE_GRAY: [u8; 3] = [160, 160, 160];
/// The most noise a channel may measure. Analog captures of OoT's #a0a0a0 measure up to 1.52,
/// and the frame before it, with a twentieth of the scene left, 2.46 and up. So more is a scene
/// rather than noise, and measuring it would make fades reach the threshold a frame early.
const MAX_MEASURED_NOISE: f64 = 2.0;

/// How much noise a capture adds to each channel (red, green, blue), measured on black and on
/// [`NOISE_GRAY`]. Analog captures have noise that black hides when they crush it, so the two
/// are measured apart.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FadeNoise {
    pub black: [f64; 3],
    pub gray: [f64; 3],
}

impl FadeNoise {
    /// No noise measured, as for a clean capture.
    pub const NONE: Self = Self { black: [0.0; 3], gray: [0.0; 3] };

    /// The noise of each channel at `color` (red, green, blue): from black's to gray's by the
    /// channel's level, and gray's above it. A hand-edited profile's values are kept within 0
    /// to 255.
    pub fn at(&self, color: [u8; 3]) -> [f64; 3] {
        std::array::from_fn(|channel| {
            let share = (f64::from(color[channel]) / f64::from(NOISE_GRAY[channel])).min(1.0);
            let [black, gray] = [self.black[channel], self.gray[channel]]
                .map(|noise| if noise.is_nan() { 0.0 } else { noise.clamp(0.0, 255.0) });
            black + (gray - black) * share
        })
    }
}

/// Why [`measure_noise`] measured nothing.
#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum NoiseError {
    #[error("There's no capture to measure.")]
    Empty,
    #[error(
        "The middle of the capture is {found}, too far from {expected}. Show a frame that's all {expected}, \
         like the end of a fade, and measure again."
    )]
    NotTheColor { expected: String, found: String },
    #[error(
        "The middle of the capture isn't one solid color: one of red, green and blue is {spread} levels \
         from its mean on average. Show a frame that's all {expected}, like the end of a fade, and measure again."
    )]
    NotSolid { expected: String, spread: String },
}

/// The noise of each channel (red, green, blue) of a capture that shows `color`: how far its
/// pixels are from their mean, rounded to hundredths. Only the middle is read, where presets'
/// fade masks are: x 5/16 to 11/16 and y 1/4 to 3/4, away from the edges that analog captures
/// light unevenly. It has to be within `tolerance` levels of `color`, and look like noise rather
/// than a scene.
pub fn measure_noise(capture: BgraView, color: [u8; 3], tolerance: u8) -> Result<[f64; 3], NoiseError> {
    let (width, height) = (capture.width(), capture.height());
    let middle = capture.crop(width * 5 / 16, width * 11 / 16, height / 4, height * 3 / 4);
    if middle.is_empty() {
        return Err(NoiseError::Empty);
    }
    let (mut sums, mut squares) = ([0_u64; 3], [0_u64; 3]);
    for y in 0..middle.height() {
        for pixel in middle.row(y).as_chunks::<4>().0 {
            // BGRA to red, green, blue
            for (channel, &value) in [pixel[2], pixel[1], pixel[0]].iter().enumerate() {
                sums[channel] += u64::from(value);
                squares[channel] += u64::from(value) * u64::from(value);
            }
        }
    }
    let count = (middle.width() * middle.height()) as f64;
    let mean: [f64; 3] = std::array::from_fn(|channel| sums[channel] as f64 / count);
    let noise: [f64; 3] = std::array::from_fn(|channel| {
        (squares[channel] as f64 / count - mean[channel] * mean[channel]).max(0.0).sqrt()
    });
    let expected = hex_color(color);
    if mean.iter().zip(color).any(|(mean, color)| (mean - f64::from(color)).abs() > f64::from(tolerance)) {
        return Err(NoiseError::NotTheColor { expected, found: hex_color(mean.map(|mean| mean.round() as u8)) });
    }
    let spread = noise.iter().copied().fold(0.0, f64::max);
    if spread > MAX_MEASURED_NOISE {
        return Err(NoiseError::NotSolid { expected, spread: format!("{spread:.1}") });
    }
    Ok(noise.map(|noise| (noise * 100.0).round() / 100.0))
}

/// The similarity of `capture` to a solid `color` (BGR), over the pixels `mask` shows:
/// `1 - sqrt(offset^2 + spread^2) / max`, where the offset is how far the capture's mean color is
/// from `color` and the spread how far its pixels are from that mean. Each channel's mean may be
/// up to `tolerance` levels off for free, for capture setups whose colors are a little off, and
/// each channel's `noise` (BGR) is taken out of its spread, for capture setups that add noise,
/// like analog ones. Noise adds to a scene's spread as variance, so this takes out about what the
/// noise added, and frames before the fade ends stay about as far from the color as without it.
/// A capture clips its noise at black and white, so a channel with a mean near either has at most
/// half that distance taken out. Otherwise the last frames of a fade to black, whose spread is
/// all that's left of the scene, would reach the threshold early.
///
/// Unlike AutoSplit's L2 norm, alpha isn't counted, so 1.0 and 0.0 mean the same with or without
/// a mask.
pub(crate) fn similarity(
    color: [u8; 3],
    capture: &BgraImage,
    mask: Option<&Mask>,
    tolerance: u8,
    noise: [f64; 3],
) -> f64 {
    let mut count = 0_u64;
    let mut sums = [0_u64; 3];
    let mut squares = [0_u64; 3];
    let mut add = |pixels: &[u8]| {
        for pixel in pixels.as_chunks::<4>().0 {
            for channel in 0..3 {
                let value = u64::from(pixel[channel]);
                sums[channel] += value;
                squares[channel] += value * value;
            }
        }
        count += (pixels.len() / 4) as u64;
    };
    match mask {
        Some(mask) => mask.byte_spans().for_each(|span| add(&capture.data()[span])),
        None => add(capture.data()),
    }
    if count == 0 {
        return 0.0;
    }
    let count = count as f64;
    let mut squared = 0.0;
    for channel in 0..3 {
        let mean = sums[channel] as f64 / count;
        let noise = noise[channel].min(mean.min(255.0 - mean) / 2.0);
        let variance = (squares[channel] as f64 / count - mean * mean - noise * noise).max(0.0);
        let offset = ((mean - f64::from(color[channel])).abs() - f64::from(tolerance)).max(0.0);
        squared += offset * offset + variance;
    }
    1.0 - squared.sqrt() / MAX_DISTANCE
}

/// Whether `image` reached `threshold` at `now`, for the runner and the Live Tester: a fade image
/// only by a fade, which `fade` watches for.
pub(crate) fn reached(
    image: &SplitImage,
    fade: &mut FadeTracker,
    now: Duration,
    similarity: f64,
    threshold: f64,
) -> bool {
    if image.is_fade() {
        fade.update(now, similarity, threshold, image.fade_time_ms())
    } else {
        similarity >= threshold
    }
}

/// Watches a fade image's similarity over time, to tell a fade to its color from a cut.
///
/// The similarity is sampled every quarter of the fade time, from `MIN_SAMPLE_INTERVAL` to
/// `MAX_SAMPLE_INTERVAL`. A frame at the threshold is a fade when the samples before it came closer
/// to the color step by step: none of the last `ARRIVAL_STEPS` to the threshold makes more than
/// `MAX_STEP_SHARE` of the way or `MAX_ARRIVAL_SHARE` of their rise, and `MIN_FADE_DEPTH` of the
/// way came without a jump of more than `MAX_STEP_GROWTH` times the steps after it. A cut makes all
/// of the way in one step.
#[derive(Clone, Debug, Default)]
pub struct FadeTracker {
    /// When each sample was taken, and its similarity.
    samples: VecDeque<(Duration, f64)>,
    next_sample: Option<Duration>,
    /// The samples and the current frame, for [`FadeTracker::update`], kept so it doesn't allocate
    /// on every frame at the threshold.
    way: Vec<f64>,
}

impl FadeTracker {
    /// Forgets the samples, when the image starts being compared.
    pub fn clear(&mut self) {
        self.samples.clear();
        self.next_sample = None;
    }

    /// Takes `similarity`, the image's similarity at `now`, and returns whether it reached
    /// `threshold` by a fade. `fade_time` is about how long fades take, in milliseconds. Fades up
    /// to twice as long still count.
    pub fn update(&mut self, now: Duration, similarity: f64, threshold: f64, fade_time: i64) -> bool {
        let fade_time = Duration::from_millis(fade_time.max(0) as u64);
        let interval = (fade_time / 4).clamp(MIN_SAMPLE_INTERVAL, MAX_SAMPLE_INTERVAL);
        if self.next_sample.is_none_or(|next| now >= next) {
            self.samples.push_back((now, similarity));
            // Steady steps, unless ticks were further apart than a step
            let next = self.next_sample.map_or(now, |next| next + interval);
            self.next_sample = Some(if next > now { next } else { now + interval });
        }
        // A fade slowed down by lag can take twice as long
        let oldest = now.saturating_sub(fade_time * 2);
        while self.samples.front().is_some_and(|(at, _)| *at < oldest) {
            self.samples.pop_front();
        }
        if similarity < threshold {
            return false;
        }
        // The way here: back from this frame, for as long as each sample came closer
        let current = (self.samples.back().is_none_or(|(at, _)| *at != now)).then_some(similarity);
        self.way.clear();
        self.way.extend(self.samples.iter().map(|(_, sample)| *sample).chain(current));
        let way = &self.way;
        let start = (1..way.len()).rev().find(|&i| way[i] < way[i - 1] - MAX_DIP).unwrap_or(0);
        let way = &way[start..];
        let depth = similarity - way.iter().copied().fold(f64::INFINITY, f64::min);
        // The steps into the first sample at the threshold. Later ones stay at the color, after a
        // cut too.
        let Some(arrival) = way.iter().position(|sample| *sample >= threshold).filter(|&arrival| arrival > 0) else {
            return false;
        };
        let first = arrival.saturating_sub(ARRIVAL_STEPS);
        let last_steps = &way[first..=arrival];
        let largest_step = last_steps.windows(2).map(|pair| pair[1] - pair[0]).fold(0.0, f64::max);
        let rise = last_steps[last_steps.len() - 1] - last_steps[0];
        // The gradual part: back from those steps until a step jumps far past the ones after it
        let (mut gradual, mut seen) = (first, largest_step.max(f64::EPSILON));
        while gradual > 0 && way[gradual] - way[gradual - 1] <= MAX_STEP_GROWTH * seen {
            seen = seen.max(way[gradual] - way[gradual - 1]);
            gradual -= 1;
        }
        let gradual_depth = similarity - way[gradual..].iter().copied().fold(f64::INFINITY, f64::min);
        let min_depth = MIN_FADE_DEPTH.max(1.0 - threshold);
        depth >= min_depth
            && gradual_depth >= min_depth
            && largest_step <= MAX_STEP_SHARE * depth
            && largest_step <= MAX_ARRIVAL_SHARE * rise
    }
}
