//! The Live Tester: compares a split image, or the images compared together for one split, with
//! the capture outside of a run, and keeps the frames that would have made them act, decided the
//! way the runner decides.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::compare::CompareScratch;
use crate::fade::{FadeTracker, reached};
use crate::filename::{RESET_KEYWORD, START_KEYWORD, has_keyword, number_prefix};
use crate::image::BgraImage;
use crate::runner::{Captured, interval, next_grid_tick};
use crate::split_folder::{MAX_GROUP_SIZE, user_files};
use crate::split_image::{ImageType, LoadError, SplitDefaults, SplitImage};

/// Where the tester is in the images' trigger sequence.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    /// Waiting for a similarity to reach its threshold.
    Waiting,
    /// A `{b}` image reached its threshold, and acts once it drops back below, unless another
    /// image acts first.
    Armed,
    /// The image at `slot` acts once the delay of `delay` milliseconds is over, at `until`.
    Delaying { until: Duration, delay: i64, slot: usize },
    /// The image at `slot` did what it does. Nothing more is kept until [`Tester::rearm`].
    Done { slot: usize },
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

/// A frame that triggered an image.
#[derive(Clone, Debug)]
pub struct Trigger {
    pub kind: TriggerKind,
    /// Which of the tester's images it triggered.
    pub slot: usize,
    /// The image's similarity on the last frame compared, which is 0 without a capture.
    pub similarity: f64,
    /// The capture region as captured, or `None` when the capture was lost.
    pub frame: Option<BgraImage>,
}

/// One of the tester's images, with what it read from the capture.
pub struct Slot {
    image: SplitImage,
    /// `None` without a capture, which compares as 0 like in the runner.
    similarity: Option<f64>,
    /// The noise a fade image left out of the last frame (red, green, blue), with a capture.
    noise: Option<[f64; 3]>,
    highest: Option<f64>,
    /// A `{b}` image reached its threshold.
    armed: bool,
    /// For a fade image, which only reaches the threshold by a fade.
    fade: FadeTracker,
}

impl Slot {
    pub fn image(&self) -> &SplitImage {
        &self.image
    }

    /// The similarity of the last frame, or `None` without a capture.
    pub fn similarity(&self) -> Option<f64> {
        self.similarity
    }

    /// The highest similarity since opening or the last [`Tester::rearm`].
    pub fn highest(&self) -> Option<f64> {
        self.highest
    }
}

/// Split images compared with the capture at the profile's FPS limit, with the frames that would
/// have made them act in a run. Several images are compared like the images of one split: the
/// first in order that matches acts.
pub struct Tester {
    slots: Vec<Slot>,
    /// The image that was opened, which [`Tester::image`] and the others like it are about.
    opened: usize,
    defaults: SplitDefaults,
    scratch: CompareScratch,
    epoch: Duration,
    next_tick: Duration,
    /// The frame the similarities are of, so a frame isn't compared twice.
    frame: Option<u64>,
    phase: Phase,
    triggers: Vec<Trigger>,
    /// Counts [`Tester::rearm`]s, which clear the kept frames.
    generation: u64,
}

impl Tester {
    /// Loads the image at `path`, the way the split image folder loads it.
    pub fn open(path: &Path, defaults: SplitDefaults, now: Duration) -> Result<Self, LoadError> {
        Self::open_group(&[path.to_owned()], 0, defaults, now)
    }

    /// Loads the images at `paths`, compared like the images of one split, in that order.
    /// `opened` is the one the window opened.
    pub fn open_group(
        paths: &[PathBuf],
        opened: usize,
        defaults: SplitDefaults,
        now: Duration,
    ) -> Result<Self, LoadError> {
        let mut slots = Vec::with_capacity(paths.len());
        for path in paths {
            let image = SplitImage::load(path);
            if let Some(error) = image.load_error() {
                return Err(error.clone());
            }
            let fade = FadeTracker::default();
            slots.push(Slot { image, similarity: None, noise: None, highest: None, armed: false, fade });
        }
        Ok(Self {
            slots,
            opened,
            defaults,
            scratch: CompareScratch::default(),
            epoch: now,
            next_tick: now,
            frame: None,
            phase: Phase::Waiting,
            triggers: Vec::new(),
            generation: 0,
        })
    }

    /// Its images, in the order they're compared.
    pub fn slots(&self) -> &[Slot] {
        &self.slots
    }

    /// It compares more than one image.
    pub fn grouped(&self) -> bool {
        self.slots.len() > 1
    }

    /// Which of [`Tester::slots`] was opened.
    pub fn opened(&self) -> usize {
        self.opened
    }

    /// The opened image.
    pub fn image(&self) -> &SplitImage {
        &self.slots[self.opened].image
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

    /// The opened image's similarity of the last frame, or `None` without a capture.
    pub fn similarity(&self) -> Option<f64> {
        self.slots[self.opened].similarity
    }

    /// The noise the opened fade image left out of the last frame (red, green, blue), or `None`
    /// without a capture or for other images.
    pub fn noise(&self) -> Option<[f64; 3]> {
        self.slots[self.opened].noise
    }

    /// The opened image's highest similarity since opening or the last [`Tester::rearm`].
    pub fn highest(&self) -> Option<f64> {
        self.slots[self.opened].highest
    }

    /// The opened image's threshold.
    pub fn threshold(&self) -> f64 {
        self.threshold_of(self.opened)
    }

    pub fn threshold_of(&self, slot: usize) -> f64 {
        self.slots[slot].image.similarity_threshold(&self.defaults)
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// The first image that's waiting to drop back below its threshold.
    pub fn armed(&self) -> Option<usize> {
        self.slots.iter().position(|slot| slot.armed)
    }

    pub fn triggers(&self) -> &[Trigger] {
        &self.triggers
    }

    /// Changes on every [`Tester::rearm`], so a window knows its kept frames were cleared.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Clears the highest similarities and the kept frames, and waits for a threshold again.
    pub fn rearm(&mut self) {
        for slot in &mut self.slots {
            slot.highest = None;
            slot.armed = false;
            slot.fade.clear();
        }
        self.phase = Phase::Waiting;
        self.triggers.clear();
        self.generation += 1;
    }

    /// Whether `{b}` applies to the opened image.
    pub fn below(&self) -> bool {
        self.below_of(self.opened)
    }

    /// Whether `{b}` applies. The reset image resets as soon as it matches.
    pub fn below_of(&self, slot: usize) -> bool {
        let image = &self.slots[slot].image;
        image.image_type != ImageType::Reset && image.is_below()
    }

    /// The delay the opened image waits before acting, in milliseconds.
    pub fn delay(&self) -> i64 {
        self.delay_of(self.opened)
    }

    /// The delay an image waits before acting, in milliseconds. The reset image doesn't wait,
    /// and a dummy acts right away in the runner too.
    pub fn delay_of(&self, slot: usize) -> i64 {
        let image = &self.slots[slot].image;
        if image.image_type == ImageType::Reset || image.is_dummy() { 0 } else { image.delay_time_ms(&self.defaults) }
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
        let fresh = capture.is_some_and(|capture| self.frame != Some(capture.sequence));
        if let Some(capture) = capture.filter(|_| fresh) {
            self.scratch.begin_frame(capture.sequence);
        }
        let comparing = matches!(self.phase, Phase::Waiting | Phase::Armed);
        // Like the runner: the first image that matches acts, the ones after it are only read
        let mut matched = None;
        for index in 0..self.slots.len() {
            let threshold = self.threshold_of(index);
            let below = self.below_of(index);
            let slot = &mut self.slots[index];
            let similarity = match capture {
                Some(_) if !fresh && slot.similarity.is_some() => slot.similarity.unwrap_or_default(),
                Some(capture) => {
                    let view = Some(capture.view);
                    match slot.image.fade_comparison(view, &self.defaults, &mut self.scratch) {
                        Some(fade) => {
                            slot.noise = Some(fade.noise);
                            fade.similarity
                        }
                        None => {
                            slot.noise = None;
                            let method = slot.image.comparison_method(&self.defaults);
                            slot.image.compare(view, method, &mut self.scratch)
                        }
                    }
                }
                None => {
                    slot.noise = None;
                    0.0
                }
            };
            slot.similarity = capture.map(|_| similarity);
            slot.highest = Some(slot.highest.map_or(similarity, |highest| highest.max(similarity)));
            if !comparing || matched.is_some() {
                continue;
            }
            if reached(&slot.image, &mut slot.fade, now, similarity, threshold) {
                if !below {
                    matched = Some((index, TriggerKind::Reached));
                } else if !slot.armed {
                    slot.armed = true;
                    let frame = capture.map(|capture| capture.view.to_image());
                    self.triggers.push(Trigger { kind: TriggerKind::Reached, slot: index, similarity, frame });
                }
            // Like the runner, a lost capture doesn't count as dropping below
            } else if below && slot.armed && similarity < threshold && capture.is_some() {
                matched = Some((index, TriggerKind::DroppedBelow));
            }
        }
        self.frame = capture.map(|capture| capture.sequence);

        let keep = |kind, slot: usize, slots: &[Slot]| Trigger {
            kind,
            slot,
            similarity: slots[slot].similarity.unwrap_or_default(),
            frame: capture.map(|capture| capture.view.to_image()),
        };
        self.phase = match (self.phase, matched) {
            // A delay of 0 or less acts on the triggering frame, like the runner
            (_, Some((slot, kind))) => {
                for slot in &mut self.slots {
                    slot.armed = false;
                }
                let delay = self.delay_of(slot);
                if delay > 0 {
                    self.triggers.push(keep(kind, slot, &self.slots));
                    Phase::Delaying { until: now + Duration::from_millis(delay as u64), delay, slot }
                } else {
                    self.triggers.push(keep(TriggerKind::Acted { delay: 0 }, slot, &self.slots));
                    Phase::Done { slot }
                }
            }
            (Phase::Waiting | Phase::Armed, None) if self.armed().is_some() => Phase::Armed,
            // The runner acts when the delay is over, with or without a capture
            (Phase::Delaying { until, delay, slot }, None) if now >= until => {
                self.triggers.push(keep(TriggerKind::Acted { delay }, slot, &self.slots));
                Phase::Done { slot }
            }
            (phase, None) => phase,
        };
    }
}

/// The images compared together with the image at `path` in a run that compares images with the
/// same number for one split, in the order they're compared, with `path` among them: the PNG
/// images in its folder with its number, or the other start or reset images. Only `path` when
/// there are none or more than a run can compare, or for a split image without a number.
pub fn group_paths(path: &Path) -> Vec<PathBuf> {
    let alone = vec![path.to_owned()];
    let Some(folder) = path.parent() else { return alone };
    let Some(key) = group_key(path) else { return alone };
    let group: Vec<PathBuf> = (user_files(folder).unwrap_or_default().into_iter())
        .filter(|other| other.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("png")))
        .filter(|other| group_key(other).as_ref() == Some(&key))
        .collect();
    if group.len() > MAX_GROUP_SIZE || !group.iter().any(|other| other == path) {
        return alone;
    }
    group
}

/// What images in one group share, read from the file name like [`SplitImage::load`] and
/// [`crate::split_folder`] do: start, reset, or a split image's number without its leading zeros.
fn group_key(path: &Path) -> Option<String> {
    let filename = path.file_name()?.to_string_lossy().to_lowercase();
    if has_keyword(&filename, START_KEYWORD) {
        Some(START_KEYWORD.into())
    } else if has_keyword(&filename, RESET_KEYWORD) {
        Some(RESET_KEYWORD.into())
    } else {
        number_prefix(&filename).map(|digits| format!("#{}", digits.trim_start_matches('0')))
    }
}
