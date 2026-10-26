//! The Live Tester: compares one split image with the capture outside of a run, and keeps the
//! frames that would have made it act, decided the way the runner decides.

use std::path::Path;
use std::time::Duration;

use crate::compare::CompareScratch;
use crate::fade::{FadeTracker, reached};
use crate::image::BgraImage;
use crate::runner::{Captured, interval, next_grid_tick};
use crate::split_image::{ImageType, LoadError, SplitDefaults, SplitImage};

/// Where the tester is in the image's trigger sequence.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    /// Waiting for the similarity to reach the threshold.
    Waiting,
    /// A `{b}` image reached the threshold, and acts once it drops back below.
    Armed,
    /// The image acts once the delay of `delay` milliseconds is over, at `until`.
    Delaying { until: Duration, delay: i64 },
    /// The image did what it does. Nothing more is kept until [`Tester::rearm`].
    Done,
}

/// What a kept frame is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriggerKind {
    /// The similarity reached the threshold, which acts later: after `{b}` drops back below it,
    /// or after the delay.
    Reached,
    /// A `{b}` image dropped back below the threshold, which acts after the delay.
    DroppedBelow,
    /// The image did its action on this frame: split, start, reset, pause, or move on for a
    /// dummy. `delay` is the delay it waited for, in milliseconds.
    Acted { delay: i64 },
}

/// A frame that triggered the image.
#[derive(Clone, Debug)]
pub struct Trigger {
    pub kind: TriggerKind,
    /// The similarity of the last frame compared, which is 0 without a capture.
    pub similarity: f64,
    /// The capture region as captured, or `None` when the capture was lost.
    pub frame: Option<BgraImage>,
}

/// One split image compared with the capture at the profile's FPS limit, with the frames that
/// would have made it act in a run.
pub struct Tester {
    image: SplitImage,
    defaults: SplitDefaults,
    scratch: CompareScratch,
    epoch: Duration,
    next_tick: Duration,
    /// The frame the similarity is of, so a frame isn't compared twice.
    frame: Option<u64>,
    /// `None` without a capture, which compares as 0 like in the runner.
    similarity: Option<f64>,
    /// The noise a fade image left out of the last frame (red, green, blue), with a capture.
    noise: Option<[f64; 3]>,
    highest: Option<f64>,
    phase: Phase,
    /// For a fade image, which only reaches the threshold by a fade.
    fade: FadeTracker,
    triggers: Vec<Trigger>,
    /// Counts [`Tester::rearm`]s, which clear the kept frames.
    generation: u64,
}

impl Tester {
    /// Loads the image at `path`, the way the split image folder loads it.
    pub fn open(path: &Path, defaults: SplitDefaults, now: Duration) -> Result<Self, LoadError> {
        let image = SplitImage::load(path);
        if let Some(error) = image.load_error() {
            return Err(error.clone());
        }
        Ok(Self {
            image,
            defaults,
            scratch: CompareScratch::default(),
            epoch: now,
            next_tick: now,
            frame: None,
            similarity: None,
            noise: None,
            highest: None,
            phase: Phase::Waiting,
            fade: FadeTracker::default(),
            triggers: Vec::new(),
            generation: 0,
        })
    }

    pub fn image(&self) -> &SplitImage {
        &self.image
    }

    pub fn defaults(&self) -> &SplitDefaults {
        &self.defaults
    }

    /// The profile's defaults changed. Takes effect from the next comparison. A delay already
    /// running keeps its end, like in the runner.
    pub fn set_defaults(&mut self, defaults: SplitDefaults) {
        self.defaults = defaults;
        self.frame = None;
    }

    /// The similarity of the last frame, or `None` without a capture.
    pub fn similarity(&self) -> Option<f64> {
        self.similarity
    }

    /// The noise a fade image left out of the last frame (red, green, blue), or `None` without a
    /// capture or for other images.
    pub fn noise(&self) -> Option<[f64; 3]> {
        self.noise
    }

    /// The highest similarity since opening or the last [`Tester::rearm`].
    pub fn highest(&self) -> Option<f64> {
        self.highest
    }

    pub fn threshold(&self) -> f64 {
        self.image.similarity_threshold(&self.defaults)
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn triggers(&self) -> &[Trigger] {
        &self.triggers
    }

    /// Changes on every [`Tester::rearm`], so a window knows its kept frames were cleared.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Clears the highest similarity and the kept frames, and waits for the threshold again.
    pub fn rearm(&mut self) {
        self.highest = None;
        self.phase = Phase::Waiting;
        self.fade.clear();
        self.triggers.clear();
        self.generation += 1;
    }

    /// Whether `{b}` applies. The reset image resets as soon as it matches.
    pub fn below(&self) -> bool {
        self.image.image_type != ImageType::Reset && self.image.is_below()
    }

    /// The delay the image waits before acting, in milliseconds. The reset image doesn't wait,
    /// and a dummy acts right away in the runner too.
    pub fn delay(&self) -> i64 {
        if self.image.image_type == ImageType::Reset || self.image.is_dummy() {
            0
        } else {
            self.image.delay_time_ms(&self.defaults)
        }
    }

    /// When [`Tester::tick`] should be called next: at the profile's FPS limit, like the runner.
    pub fn next_wakeup(&self) -> Duration {
        match self.phase {
            Phase::Delaying { until, .. } => until.min(self.next_tick),
            _ => self.next_tick,
        }
    }

    /// Compares the capture, when it's due at `now`. `capture` is `None` without a frame, which
    /// compares as 0, like in the runner.
    pub fn tick(&mut self, now: Duration, capture: Option<Captured>) {
        if now < self.next_wakeup() {
            return;
        }
        self.next_tick = next_grid_tick(self.epoch, now, interval(self.defaults.fps_limit));
        let capture = capture.filter(|capture| !capture.view.is_empty());
        let (similarity, noise) = match (capture, self.similarity) {
            (Some(capture), Some(similarity)) if self.frame == Some(capture.sequence) => (similarity, self.noise),
            (Some(capture), _) => {
                self.scratch.begin_frame(capture.sequence);
                let view = Some(capture.view);
                match self.image.fade_comparison(view, &self.defaults, &mut self.scratch) {
                    Some(fade) => (fade.similarity, Some(fade.noise)),
                    None => (self.image.similarity(view, &self.defaults, &mut self.scratch), None),
                }
            }
            (None, _) => (0.0, None),
        };
        self.frame = capture.map(|capture| capture.sequence);
        self.similarity = capture.map(|_| similarity);
        self.noise = noise;
        self.highest = Some(self.highest.map_or(similarity, |highest| highest.max(similarity)));

        let threshold = self.threshold();
        let reached = reached(&self.image, &mut self.fade, now, similarity, threshold);
        let (below, delay) = (self.below(), self.delay());
        let keep = |kind| Trigger { kind, similarity, frame: capture.map(|capture| capture.view.to_image()) };
        // A delay of 0 or less acts on the triggering frame, like the runner
        let trigger = |triggers: &mut Vec<Trigger>, kind| {
            if delay > 0 {
                triggers.push(keep(kind));
                Phase::Delaying { until: now + Duration::from_millis(delay as u64), delay }
            } else {
                triggers.push(keep(TriggerKind::Acted { delay: 0 }));
                Phase::Done
            }
        };
        self.phase = match self.phase {
            Phase::Waiting if reached && below => {
                self.triggers.push(keep(TriggerKind::Reached));
                Phase::Armed
            }
            Phase::Waiting if reached => trigger(&mut self.triggers, TriggerKind::Reached),
            // Like the runner, a lost capture doesn't count as dropping below
            Phase::Armed if similarity < threshold && capture.is_some() => {
                trigger(&mut self.triggers, TriggerKind::DroppedBelow)
            }
            // The runner acts when the delay is over, with or without a capture
            Phase::Delaying { until, delay } if now >= until => {
                self.triggers.push(keep(TriggerKind::Acted { delay }));
                Phase::Done
            }
            phase => phase,
        };
    }
}
