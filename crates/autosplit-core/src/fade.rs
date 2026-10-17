//! Fade images (`{f}`): how close the capture is to the image's color, and whether it got there
//! by a gradual fade rather than a cut. Not in AutoSplit.

use std::collections::VecDeque;
use std::time::Duration;

use crate::compare::Mask;
use crate::image::BgraImage;

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
/// How far a fade may go back from one sample to the next, like a game's fades sometimes do.
const MAX_DIP: f64 = 0.03;
const MIN_SAMPLE_INTERVAL: Duration = Duration::from_millis(50);
const MAX_SAMPLE_INTERVAL: Duration = Duration::from_millis(100);

/// The similarity of `capture` to a solid `color` (BGR), over the pixels `mask` shows:
/// `1 - sqrt(offset^2 + spread^2) / max`, where the offset is how far the capture's mean color is
/// from `color` and the spread how far its pixels are from that mean. Each channel's mean may be
/// up to `tolerance` levels off for free, for capture setups whose colors are a little off.
///
/// Unlike AutoSplit's L2 norm, alpha isn't counted, so 1.0 and 0.0 mean the same with or without
/// a mask.
pub(crate) fn similarity(color: [u8; 3], capture: &BgraImage, mask: Option<&Mask>, tolerance: u8) -> f64 {
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
        Some(mask) => mask.spans().iter().for_each(|span| add(&capture.data()[span.start * 4..span.end * 4])),
        None => add(capture.data()),
    }
    if count == 0 {
        return 0.0;
    }
    let count = count as f64;
    let mut squared = 0.0;
    for channel in 0..3 {
        let mean = sums[channel] as f64 / count;
        let variance = (squares[channel] as f64 / count - mean * mean).max(0.0);
        let offset = ((mean - f64::from(color[channel])).abs() - f64::from(tolerance)).max(0.0);
        squared += offset * offset + variance;
    }
    1.0 - squared.sqrt() / MAX_DISTANCE
}

/// Watches a fade image's similarity over time, to tell a fade to its color from a cut.
///
/// The similarity is sampled every quarter of the fade time, between 50 and 100 ms. A frame at
/// the threshold is a fade when the samples before it came closer to the color step by step, none
/// of the last three steps to the threshold making more than 40% of the way. A cut makes all of
/// the way in one step.
#[derive(Clone, Debug, Default)]
pub struct FadeTracker {
    /// When each sample was taken, and its similarity.
    samples: VecDeque<(Duration, f64)>,
    next_sample: Option<Duration>,
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
        let way: Vec<f64> = self.samples.iter().map(|(_, sample)| *sample).chain(current).collect();
        let start = (1..way.len()).rev().find(|&i| way[i] < way[i - 1] - MAX_DIP).unwrap_or(0);
        let way = &way[start..];
        let depth = similarity - way.iter().copied().fold(f64::INFINITY, f64::min);
        // The steps into the first sample at the threshold. Later ones stay at the color, after a
        // cut too.
        let Some(arrival) = way.iter().position(|sample| *sample >= threshold).filter(|&arrival| arrival > 0) else {
            return false;
        };
        let last_steps = &way[arrival.saturating_sub(ARRIVAL_STEPS)..=arrival];
        let largest_step = last_steps.windows(2).map(|pair| pair[1] - pair[0]).fold(0.0, f64::max);
        depth >= MIN_FADE_DEPTH.max(1.0 - threshold) && largest_step <= MAX_STEP_SHARE * depth
    }
}
