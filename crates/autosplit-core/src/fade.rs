//! Fade images (`{f}`): how close the capture is to the image's color, and whether it got there
//! by a gradual fade rather than a cut. Not in AutoSplit.

use std::collections::VecDeque;
use std::ops::Range;
use std::time::Duration;

use crate::compare::Mask;
use crate::image::BgraImage;
use crate::split_image::SplitImage;

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

/// The most noise taken out of a channel. Analog captures have up to about 1.5 levels, and fine
/// detail left of a scene just before a fade ends mustn't count as more than that.
pub(crate) const MAX_NOISE: f64 = 2.0;
/// The share of an image's height between the pixels noise is estimated from: about 8 pixels at
/// 1080 lines. Closer ones share their noise in analog captures scaled up from 480 lines, which
/// spread each speck over about 2 pixels.
const NOISE_DISTANCE: f64 = 1.0 / 135.0;
/// Noise is estimated from every this many rows. Noise is the same all over, so a quarter of
/// them, tens of thousands of pixels, estimate it about as well as all of them, at a quarter of
/// the time.
const NOISE_ROW_STEP: usize = 4;

/// How close a capture is to a fade image's color.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FadeComparison {
    pub similarity: f64,
    /// The noise left out of each channel (red, green, blue), in levels.
    pub noise: [f64; 3],
}

/// The similarity of `capture` to a solid `color` (BGR), over the pixels `mask` shows:
/// `1 - sqrt(offset^2 + spread^2) / max`, where the offset is how far the capture's mean color is
/// from `color` and the spread how far its pixels are from that mean. Each channel's mean may be
/// up to `tolerance` levels off for free, for capture setups whose colors are a little off. With
/// `ignore_noise`, the capture's [`noise`] is taken out of its spread, for capture setups that add
/// noise, like analog ones, and games with film grain or dithering. A capture clips its noise at
/// black and white, so a channel with a mean near either has at most half that distance taken
/// out. Otherwise the last frames of a fade to black, whose spread is all that's left of the
/// scene, would reach the threshold early.
///
/// Unlike AutoSplit's L2 norm, alpha isn't counted, so 1.0 and 0.0 mean the same with or without
/// a mask.
pub(crate) fn similarity(
    color: [u8; 3],
    capture: &BgraImage,
    mask: Option<&Mask>,
    tolerance: u8,
    ignore_noise: bool,
) -> FadeComparison {
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
        return FadeComparison::default();
    }
    let estimated = if ignore_noise { noise(capture, mask) } else { [0.0; 3] };
    let count = count as f64;
    let mut squared = 0.0;
    let mut left_out = [0.0; 3];
    for channel in 0..3 {
        let mean = sums[channel] as f64 / count;
        let noise = estimated[channel].min(mean.min(255.0 - mean) / 2.0);
        let variance = (squares[channel] as f64 / count - mean * mean - noise * noise).max(0.0);
        let offset = ((mean - f64::from(color[channel])).abs() - f64::from(tolerance)).max(0.0);
        squared += offset * offset + variance;
        left_out[channel] = noise;
    }
    left_out.reverse();
    FadeComparison { similarity: 1.0 - squared.sqrt() / MAX_DISTANCE, noise: left_out }
}

/// The noise of each channel (BGR) of `capture` where `mask` shows it, up to [`MAX_NOISE`]: the
/// root mean square difference between visible pixels [`NOISE_DISTANCE`] apart, across and down,
/// over the square root of 2. Noise differs from pixel to pixel, while what's left of a scene
/// near the end of a fade changes slowly across the picture, so this is about the noise alone,
/// and the capture doesn't have to be measured first.
fn noise(capture: &BgraImage, mask: Option<&Mask>) -> [f64; 3] {
    let (width, height) = (capture.width(), capture.height());
    let distance = ((height as f64 * NOISE_DISTANCE).round() as usize).max(1);
    let data = capture.data();
    let mut pairs = 0_u64;
    let mut squares = [0_u64; 3];
    // Pixels at the same place in two runs of the same length
    let mut add = |from: Range<usize>, to: usize| {
        let pixels = |start: usize| data[start * 4..(start + from.len()) * 4].as_chunks::<4>().0;
        for (a, b) in pixels(from.start).iter().zip(pixels(to)) {
            for channel in 0..3 {
                let difference = u64::from(a[channel].abs_diff(b[channel]));
                squares[channel] += difference * difference;
            }
        }
        pairs += from.len() as u64;
    };
    // Each sampled row's visible runs, as pixel indices: masks' spans can go on into the next row
    let rows = (0..height).step_by(NOISE_ROW_STEP).map(|y| y * width..(y + 1) * width);
    let runs: Box<dyn Iterator<Item = Range<usize>>> = match mask {
        None => Box::new(rows),
        Some(mask) => Box::new(mask.spans().iter().flat_map(move |span| {
            (span.start / width..=(span.end - 1) / width)
                .filter(|y| y.is_multiple_of(NOISE_ROW_STEP))
                .map(move |y| span.start.max(y * width)..span.end.min((y + 1) * width))
        })),
    };
    for run in runs {
        // Across, within the run
        if run.len() > distance {
            add(run.start..run.end - distance, run.start + distance);
        }
        // Down, where the pixels below show too
        if run.start / width + distance < height {
            let below = run.start + distance * width;
            match mask {
                None => add(run.clone(), below),
                Some(mask) => {
                    let shown = &mask.bytes()[below..below + run.len()];
                    let mut offset = 0;
                    for part in shown.chunk_by(|a, b| (*a != 0) == (*b != 0)) {
                        if part[0] != 0 {
                            add(run.start + offset..run.start + offset + part.len(), below + offset);
                        }
                        offset += part.len();
                    }
                }
            }
        }
    }
    if pairs == 0 {
        return [0.0; 3];
    }
    squares.map(|squares| (squares as f64 / pairs as f64 / 2.0).sqrt().min(MAX_NOISE))
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
