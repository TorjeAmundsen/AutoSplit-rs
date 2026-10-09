//! The split state machine: watching for the start image, comparing split images in order,
//! split delays and pauses, the reset image, and skip and undo, without blocking.
//!
//! The host calls [`Runner::tick`] with the latest capture whenever [`Runner::next_wakeup`] is
//! due, and acts on the [`Effect`]s it returns. Time is passed in, so runs are reproducible.

use std::time::Duration;

use crate::commands::Command;
use crate::compare::CompareScratch;
use crate::image::BgraView;
use crate::profile::UserProfile;
use crate::split_folder::{MAX_GROUP_SIZE, SplitSet};
use crate::split_image::{SplitDefaults, SplitImage};

/// Profile values the runner reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RunnerSettings {
    pub defaults: SplitDefaults,
    pub loop_splits: bool,
    pub enable_auto_reset: bool,
}

impl RunnerSettings {
    pub fn from_profile(profile: &UserProfile) -> Self {
        Self {
            defaults: profile.split_defaults(),
            loop_splits: profile.loop_splits,
            enable_auto_reset: profile.enable_auto_reset,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Send this command to the timer (see [`crate::commands::outputs_for`]).
    Send(Command),
    /// The start image matched. Start the run the same way the split hotkey does: reload and
    /// validate the split image folder, then call [`Runner::start`].
    RequestStart,
    /// The run ended, by reset or after the last split. Reload the folder and call
    /// [`Runner::arm`] so the start image is looked for again.
    Stopped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageRef {
    Start,
    /// Index into [`SplitSet::splits`].
    Split(usize),
}

/// What the "current split image" area shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Showing {
    Nothing,
    Image(ImageRef),
    /// "Delayed Before Starting:" countdown.
    StartDelay {
        until: Duration,
    },
    /// "None (Paused)." countdown, after the start image or after a split.
    Pause {
        until: Duration,
    },
    /// "Delayed Split:" countdown.
    SplitDelay {
        until: Duration,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartStatus {
    NotFound,
    Ready,
    DelayingStart,
    Started,
    NotReady,
}

/// The reset image's live similarity cell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ResetReading {
    /// No reset image.
    NotApplicable,
    /// Auto reset is turned off.
    Disabled,
    /// Still within the start or reset image's pause time since the run started.
    Paused,
    Similarity(f64),
}

/// One image of the split being compared.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageReading {
    pub similarity: Option<f64>,
    pub highest: Option<f64>,
    pub threshold: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Buttons {
    pub start: bool,
    pub reset: bool,
    pub skip_split: bool,
    pub undo_split: bool,
    pub next_image: bool,
    pub previous_image: bool,
}

/// Everything the UI shows about the run.
#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    pub running: bool,
    pub start_status: StartStatus,
    pub showing: Showing,
    pub current_filename: Option<String>,
    pub similarity: Option<f64>,
    pub highest: Option<f64>,
    pub threshold: Option<f64>,
    /// (current loop, total loops) of the current split image.
    pub loop_position: Option<(i64, i64)>,
    /// (position, count) of the current image among all split images, loops counted.
    pub split_position: Option<(usize, usize)>,
    /// The current image has the below flag, `{b}`: it splits once the similarity drops back
    /// below the threshold.
    pub below: bool,
    /// Each image of the split being compared, in the order they're compared. Empty for the
    /// start image.
    pub group: Vec<ImageReading>,
    pub reset_similarity: Option<ResetReading>,
    pub reset_highest: Option<f64>,
    pub reset_threshold: Option<f64>,
    pub buttons: Buttons,
}

impl Status {
    fn idle(timer_synced: bool) -> Self {
        Self {
            running: false,
            start_status: StartStatus::NotFound,
            showing: Showing::Nothing,
            current_filename: None,
            similarity: None,
            highest: None,
            threshold: None,
            loop_position: None,
            split_position: None,
            below: false,
            group: Vec::new(),
            reset_similarity: None,
            reset_highest: None,
            reset_threshold: None,
            buttons: Buttons {
                start: !timer_synced,
                reset: false,
                skip_split: false,
                undo_split: false,
                next_image: false,
                previous_image: false,
            },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    /// Not running, and no start image to look for.
    Idle,
    WaitingForStart {
        epoch: Duration,
        next_tick: Duration,
    },
    StartDelay {
        until: Duration,
    },
    /// After the start image, or after a split. Ends early when the split index changes.
    Paused {
        until: Duration,
        index_at_start: usize,
        next_tick: Duration,
    },
    Comparing {
        epoch: Duration,
        next_tick: Duration,
    },
    SplitDelay {
        until: Duration,
        next_tick: Duration,
    },
}

pub struct Runner {
    settings: RunnerSettings,
    /// The timer reports its splits, skips, undos and resets (LiveSplit with `--auto-controlled`,
    /// or LiveSplit One). Fixed for the runner's lifetime, like AutoSplit's command line flag.
    timer_synced: bool,
    images: SplitSet,
    /// `split_images_and_loop_number`: each group of [`SplitSet::groups`] repeated by its loop
    /// count.
    plan: Vec<(usize, i64)>,
    /// Consecutive plan entries up to and including the next non-dummy split.
    groups: Vec<Vec<usize>>,
    /// The group each plan entry is in.
    group_of: Vec<usize>,
    /// Position in `plan`. Past the end once the last split is done.
    index: usize,
    state: State,
    highest: f64,
    reset_highest: f64,
    /// Per image of the group being compared, or `[0]` for the start image.
    below_armed: [bool; MAX_GROUP_SIZE],
    /// The split image that matched, which decides what the split does.
    matched: usize,
    run_start: Duration,
    scratch: CompareScratch,
    cache: ComparisonCache,
    status: Status,
    effects: Vec<Effect>,
}

/// Which image a comparison was of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Compared {
    Start,
    Reset,
    /// Index into [`SplitSet::splits`].
    Split(usize),
}

/// A captured frame, with a number that's different for every frame.
#[derive(Clone, Copy, Debug)]
pub struct Captured<'a> {
    pub view: BgraView<'a>,
    pub sequence: u64,
}

/// Similarities computed for the current frame.
#[derive(Debug, Default)]
struct ComparisonCache {
    frame: Option<u64>,
    results: Vec<(Compared, f64)>,
}

impl ComparisonCache {
    /// Starts a tick of `frame`. Results stay when it's the frame of the previous tick.
    fn begin(&mut self, frame: Option<u64>) {
        if self.frame != frame || frame.is_none() {
            self.frame = frame;
            self.results.clear();
        }
    }

    /// The similarity of `compared` against the current frame, computed only the first time.
    fn get_or_compare(&mut self, compared: Compared, compare: impl FnOnce() -> f64) -> f64 {
        if self.frame.is_none() {
            return compare();
        }
        if let Some(&(_, similarity)) = self.results.iter().find(|(cached, _)| *cached == compared) {
            return similarity;
        }
        let similarity = compare();
        self.results.push((compared, similarity));
        similarity
    }

    /// Images or settings changed, so earlier results don't apply.
    fn clear(&mut self) {
        self.frame = None;
        self.results.clear();
    }
}

/// At least 1ns, so absurd FPS limits can't make the runner wake up without time passing.
fn interval(fps_limit: i64) -> Duration {
    Duration::from_secs_f64(1.0 / fps_limit.max(1) as f64).max(Duration::from_nanos(1))
}

fn secs(seconds: f64) -> Duration {
    Duration::try_from_secs_f64(seconds).unwrap_or(Duration::MAX)
}

/// The next point on a grid of `interval` steps from `epoch` that's after `now`, which keeps
/// comparisons at a steady rate however long each one takes.
///
/// Integer nanoseconds, because float rounding can land back on `now` and never advance.
fn next_grid_tick(epoch: Duration, now: Duration, interval: Duration) -> Duration {
    let step = interval.as_nanos().max(1);
    let steps = now.saturating_sub(epoch).as_nanos() / step + 1;
    epoch.saturating_add(Duration::from_nanos((steps * step).min(u64::MAX as u128) as u64))
}

impl Runner {
    pub fn new(settings: RunnerSettings, timer_synced: bool) -> Self {
        Self {
            settings,
            timer_synced,
            images: SplitSet::default(),
            plan: Vec::new(),
            groups: Vec::new(),
            group_of: Vec::new(),
            index: 0,
            state: State::Idle,
            highest: 0.0,
            reset_highest: 0.0,
            below_armed: [false; MAX_GROUP_SIZE],
            matched: 0,
            run_start: Duration::ZERO,
            scratch: CompareScratch::default(),
            cache: ComparisonCache::default(),
            status: Status::idle(timer_synced),
            effects: Vec::new(),
        }
    }

    /// Whether the timer reports its own splits, after the timer connection changed. Only
    /// between runs: a run keeps how it started.
    pub fn set_timer_synced(&mut self, timer_synced: bool) {
        if self.is_running() || timer_synced == self.timer_synced {
            return;
        }
        self.timer_synced = timer_synced;
        self.status.buttons.start = !timer_synced;
    }

    pub fn settings(&self) -> &RunnerSettings {
        &self.settings
    }

    /// Takes effect from the next comparison, like editing settings mid-run in AutoSplit.
    pub fn set_settings(&mut self, settings: RunnerSettings) {
        self.settings = settings;
        self.cache.clear();
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    pub fn images(&self) -> &SplitSet {
        &self.images
    }

    pub fn is_running(&self) -> bool {
        matches!(self.state, State::Paused { .. } | State::Comparing { .. } | State::SplitDelay { .. })
    }

    /// Whether ticks at `now` compare frames: while looking for the start image or a split
    /// image, and during pauses and delays when the reset image is checked, which it isn't
    /// during its pause time.
    pub fn needs_frames(&self, now: Duration) -> bool {
        match self.state {
            State::Idle | State::StartDelay { .. } => false,
            State::WaitingForStart { .. } | State::Comparing { .. } => true,
            State::Paused { .. } | State::SplitDelay { .. } => {
                self.reset_check_interval() != Duration::MAX && !self.in_reset_pause_window(now)
            }
        }
    }

    /// Within the start or reset image's pause time since the run started, when the reset
    /// image isn't checked.
    fn in_reset_pause_window(&self, now: Duration) -> bool {
        self.reset_pause_secs().is_some_and(|pause| now.saturating_sub(self.run_start).as_secs_f64() <= pause)
    }

    /// The reset image's pause time, or the start image's when that's longer.
    fn reset_pause_secs(&self) -> Option<f64> {
        let defaults = self.settings.defaults;
        let reset = self.images.reset.as_ref()?;
        let start_pause = self.images.start.as_ref().map_or(0.0, |start| start.pause_time_secs(&defaults));
        Some(reset.pause_time_secs(&defaults).max(start_pause))
    }

    /// When to check the reset image next during a pause or delay: at its FPS limit, but not
    /// before its pause time is over, since nothing is compared until then.
    fn next_reset_check(&self, now: Duration) -> Duration {
        let next = now.saturating_add(self.reset_check_interval());
        match self.reset_pause_secs() {
            Some(pause) if self.in_reset_pause_window(now) => {
                next.max(self.run_start.saturating_add(Duration::try_from_secs_f64(pause).unwrap_or(Duration::MAX)))
            }
            _ => next,
        }
    }

    pub fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    /// When [`Runner::tick`] should be called next. `None` when there's nothing to compare.
    pub fn next_wakeup(&self) -> Option<Duration> {
        match self.state {
            State::Idle => None,
            State::StartDelay { until } => Some(until),
            State::WaitingForStart { next_tick, .. } | State::Comparing { next_tick, .. } => Some(next_tick),
            State::Paused { until, next_tick, .. } | State::SplitDelay { until, next_tick } => {
                Some(until.min(next_tick))
            }
        }
    }

    /// Looks for the folder's start image, if it has one. Called after loading the folder.
    /// Returns false, and does nothing, while running.
    ///
    /// Clears the reset image's highest similarity, which AutoSplit only does when there's a
    /// start image (it otherwise carries it over between runs).
    pub fn arm(&mut self, images: SplitSet, now: Duration) -> bool {
        if self.is_running() {
            return false;
        }
        self.cache.clear();
        self.images = images;
        self.status = Status::idle(self.timer_synced);
        self.index = 0;
        self.highest = 0.0;
        self.reset_highest = 0.0;
        self.below_armed = [false; MAX_GROUP_SIZE];
        self.state = if self.images.start.is_some() {
            self.status.start_status = StartStatus::Ready;
            self.show_image(ImageRef::Start);
            State::WaitingForStart { epoch: now, next_tick: now + interval(self.settings.defaults.fps_limit) }
        } else {
            State::Idle
        };
        true
    }

    /// Stops looking for the start image, e.g. after the folder failed validation.
    /// Returns false, and does nothing, while running.
    pub fn disarm(&mut self) -> bool {
        if self.is_running() {
            return false;
        }
        self.state = State::Idle;
        self.status = Status::idle(self.timer_synced);
        true
    }

    /// Starts a run with a freshly validated folder. `run_start` is when the start was
    /// requested, before the folder was parsed, which the reset image's pause window counts from.
    ///
    /// Starting during the start image's delay drops that pending start. AutoSplit would still
    /// send it once the delay ends, which the timer takes as a split.
    ///
    /// Returns false, and does nothing, while already running.
    pub fn start(&mut self, images: SplitSet, now: Duration, run_start: Duration) -> bool {
        if self.is_running() {
            return false;
        }
        self.cache.clear();
        self.images = images;
        self.run_start = run_start;
        self.build_plan();
        if self.status.start_status == StartStatus::Ready {
            self.status.start_status = StartStatus::NotReady;
        }
        self.status.running = true;
        self.index = 0;
        self.below_armed = [false; MAX_GROUP_SIZE];
        let start_pause =
            self.images.start.as_ref().map_or(0.0, |start| start.pause_time_secs(&self.settings.defaults));
        if start_pause > 0.0 {
            self.enter_pause(now, start_pause);
        } else {
            self.begin_split(now);
        }
        self.refresh_buttons();
        true
    }

    fn build_plan(&mut self) {
        // Images in a group have the same loop count
        self.plan = (self.images.groups.iter().enumerate())
            .flat_map(|(group, range)| {
                (1..=self.images.splits[range.start].loops).map(move |loop_number| (group, loop_number))
            })
            .collect();
        self.groups = vec![Vec::new()];
        self.group_of.clear();
        for i in 0..self.plan.len() {
            self.groups.last_mut().unwrap().push(i);
            self.group_of.push(self.groups.len() - 1);
            // A group only counts as a dummy for skip and undo when every image in it is one
            if !self.plan_images(i).iter().all(SplitImage::is_dummy) && i < self.plan.len() - 1 {
                self.groups.push(Vec::new());
            }
        }
    }

    /// The images compared for a plan entry.
    fn plan_images(&self, plan_index: usize) -> &[SplitImage] {
        &self.images.splits[self.images.groups[self.plan[plan_index].0].clone()]
    }

    fn out_of_range(&self) -> bool {
        self.index >= self.plan.len()
    }

    fn matched(&self) -> &SplitImage {
        &self.images.splits[self.matched]
    }

    fn image(&self, image: ImageRef) -> &SplitImage {
        match image {
            ImageRef::Start => self.images.start.as_ref().expect("start image"),
            ImageRef::Split(index) => &self.images.splits[index],
        }
    }

    fn show_image(&mut self, image_ref: ImageRef) {
        let defaults = self.settings.defaults;
        let image = self.image(image_ref);
        let (filename, threshold) = (image.display_name(), image.similarity_threshold(&defaults));
        self.status.below = image.is_below();
        self.status.showing = Showing::Image(image_ref);
        self.status.current_filename = Some(filename);
        self.status.threshold = Some(threshold);
        self.status.group = match image_ref {
            ImageRef::Start => Vec::new(),
            ImageRef::Split(_) => {
                let group = self.images.groups[self.plan[self.index].0].clone();
                let reading = |image: &SplitImage| ImageReading {
                    similarity: None,
                    highest: None,
                    threshold: image.similarity_threshold(&defaults),
                };
                self.images.splits[group].iter().map(reading).collect()
            }
        };
        (self.status.loop_position, self.status.split_position) = match image_ref {
            ImageRef::Start => (None, None),
            ImageRef::Split(split) => {
                let loop_number = self.plan[self.index].1;
                (Some((loop_number, self.images.splits[split].loops)), Some((self.index + 1, self.plan.len())))
            }
        };
    }

    /// Shows the current split image, the first of its group, and resets its highest
    /// similarity. Returns false past the last split.
    fn update_split_image(&mut self) -> bool {
        self.highest = 0.0;
        self.below_armed = [false; MAX_GROUP_SIZE];
        if self.out_of_range() {
            return false;
        }
        self.show_image(ImageRef::Split(self.images.groups[self.plan[self.index].0].start));
        true
    }

    /// Reset button or hotkey, or the timer's reset. Nothing is sent: the timer was already
    /// reset by whatever triggered this.
    pub fn reset(&mut self) {
        if self.is_running() {
            self.stop();
        }
    }

    /// "Skip Split" (`navigate_image_only = false`): moves past the current split and the dummies
    /// before it, and presses the skip hotkey. "Next Img." or the skip hotkey (`true`): moves to
    /// the next image only. Neither moves past the last split. The timer's own split does, and
    /// ends the run there like it ends the timer.
    pub fn skip(&mut self, now: Duration, navigate_image_only: bool) {
        if !self.can_navigate() {
            return;
        }
        let target = if navigate_image_only {
            self.index + 1
        } else {
            self.groups[self.group_of[self.index]].last().unwrap() + 1
        };
        let timer_split = self.timer_synced && !navigate_image_only;
        if target >= self.plan.len() && !timer_split {
            return;
        }
        self.index = target;
        self.after_navigation(now, navigate_image_only, Command::Skip);
    }

    /// "Undo Split" (`navigate_image_only = false`): goes back to the previous split and presses
    /// the undo hotkey. "Prev. Img." or the undo hotkey (`true`): goes back one image. Both do
    /// nothing when there's nothing to go back to.
    pub fn undo(&mut self, now: Duration, navigate_image_only: bool) {
        if !self.can_navigate() {
            return;
        }
        let target = if navigate_image_only {
            self.index.checked_sub(1)
        } else {
            let group = self.group_of[self.index];
            group.checked_sub(1).map(|previous| *self.groups[previous].last().unwrap())
        };
        let Some(target) = target else {
            return;
        };
        self.index = target;
        self.after_navigation(now, navigate_image_only, Command::Undo);
    }

    fn can_navigate(&self) -> bool {
        self.is_running() && !matches!(self.state, State::SplitDelay { .. }) && !self.out_of_range()
    }

    fn after_navigation(&mut self, now: Duration, navigate_image_only: bool, command: Command) {
        let in_range = self.update_split_image();
        // A synced timer skipped or undid already, so it isn't sent this back, but "screenshot on"
        // still applies (see `outputs_for`)
        if !navigate_image_only {
            self.effects.push(Effect::Send(command));
        }
        if !in_range {
            self.stop();
            return;
        }
        // A pause ends as soon as the index changes, so check on the next tick right away
        if let State::Paused { next_tick, .. } = &mut self.state {
            *next_tick = now;
        }
        self.refresh_buttons();
    }

    /// Which buttons can do something right now, by the same rules as [`Runner::skip`] and
    /// [`Runner::undo`]. Skip and undo split are the timer's job when it's synced.
    fn refresh_buttons(&mut self) {
        if !self.is_running() {
            return;
        }
        let navigable = self.can_navigate();
        let group = if navigable { self.group_of[self.index] } else { 0 };
        let manual = !self.timer_synced;
        self.status.buttons = Buttons {
            start: false,
            reset: manual,
            next_image: navigable && self.index + 1 < self.plan.len(),
            previous_image: navigable && self.index > 0,
            skip_split: navigable && manual && group + 1 < self.groups.len(),
            undo_split: navigable && manual && group > 0,
        };
    }

    /// Processes everything due at `now`. `capture` is `None` (or empty) when the capture was
    /// lost.
    ///
    /// Comparisons against a frame already compared, by its sequence number, reuse their
    /// results: ticks often come faster than new frames, and AutoSplit compares the same frame
    /// again every time, so this changes nothing but the CPU time.
    pub fn tick(&mut self, now: Duration, capture: Option<Captured>) {
        let capture = capture.filter(|capture| !capture.view.is_empty());
        let sequence = capture.map(|capture| capture.sequence);
        self.cache.begin(sequence);
        if let Some(sequence) = sequence {
            self.scratch.begin_frame(sequence);
        }
        self.process(now, capture.map(|capture| capture.view));
        self.refresh_buttons();
    }

    fn process(&mut self, now: Duration, capture: Option<BgraView>) {
        match self.state {
            State::Idle => {}
            State::WaitingForStart { epoch, next_tick } if now >= next_tick => {
                self.compare_start_image(epoch, now, capture)
            }
            State::StartDelay { until } if now >= until => self.started(),
            State::Paused { until, index_at_start, .. } => {
                if self.check_reset(now, capture) {
                    return;
                }
                if now >= until || self.index != index_at_start {
                    // After the start pause AutoSplit goes back to the first split even when
                    // a skip was sent during the pause. Keeping the index stays in sync with
                    // the timer instead.
                    self.begin_split(now);
                } else {
                    let check_at = self.next_reset_check(now);
                    if let State::Paused { next_tick, .. } = &mut self.state {
                        *next_tick = check_at;
                    }
                }
            }
            State::Comparing { epoch, next_tick } if now >= next_tick => self.compare_split_image(epoch, now, capture),
            State::SplitDelay { until, .. } => {
                if self.check_reset(now, capture) {
                    return;
                }
                if now >= until {
                    self.send_split();
                    self.advance(now);
                } else {
                    self.state = State::SplitDelay { until, next_tick: self.next_reset_check(now) };
                }
            }
            State::WaitingForStart { .. } | State::StartDelay { .. } | State::Comparing { .. } => {}
        }
    }

    /// One comparison of the start image.
    fn compare_start_image(&mut self, epoch: Duration, now: Duration, capture: Option<BgraView>) {
        let defaults = self.settings.defaults;
        let start = self.images.start.as_ref().expect("start image");
        let similarity = self.cache.get_or_compare(Compared::Start, || {
            start.compare(capture, start.comparison_method(&defaults), &mut self.scratch)
        });
        let threshold = start.similarity_threshold(&defaults);
        let (below, dummy, delay) = (start.is_below(), start.is_dummy(), start.delay_time_ms(&defaults));
        let next_tick = next_grid_tick(epoch, now, interval(self.settings.defaults.fps_limit));

        self.status.start_status = StartStatus::Ready;
        if self.status.showing != Showing::Image(ImageRef::Start) {
            self.show_image(ImageRef::Start);
        }
        self.highest = self.highest.max(similarity);
        self.status.similarity = Some(similarity);
        self.status.highest = Some(self.highest);

        let difference = similarity - threshold;
        if below && !self.below_armed[0] && difference >= 0.0 {
            self.below_armed[0] = true;
            self.state = State::WaitingForStart { epoch, next_tick };
            return;
        }
        let triggered =
            (below && self.below_armed[0] && difference < 0.0 && capture.is_some()) || (!below && difference >= 0.0);
        if !triggered {
            self.state = State::WaitingForStart { epoch, next_tick };
            return;
        }
        self.below_armed = [false; MAX_GROUP_SIZE];
        if dummy {
            self.status.start_status = StartStatus::Started;
            self.state = State::Idle;
            self.effects.push(Effect::RequestStart);
        } else if delay > 0 {
            let until = now + Duration::from_millis(delay as u64);
            self.status.start_status = StartStatus::DelayingStart;
            self.status.showing = Showing::StartDelay { until };
            self.state = State::StartDelay { until };
        } else {
            self.started();
        }
    }

    /// The start image triggered and its delay passed.
    fn started(&mut self) {
        self.effects.push(Effect::Send(Command::Start));
        self.status.start_status = StartStatus::Started;
        self.state = State::Idle;
        self.effects.push(Effect::RequestStart);
    }

    /// Shows the current split image and starts comparing it, or ends the run after the last one.
    fn begin_split(&mut self, now: Duration) {
        if !self.update_split_image() {
            self.stop();
            return;
        }
        // Compare right away: AutoSplit doesn't wait between a split and the next image
        self.state = State::Comparing { epoch: now, next_tick: now };
    }

    /// One comparison of the current split image, or of each image in its group. The first one
    /// that matches splits. The status shows the best similarity in the group.
    fn compare_split_image(&mut self, epoch: Duration, now: Duration, capture: Option<BgraView>) {
        if self.check_reset(now, capture) {
            return;
        }
        let defaults = self.settings.defaults;
        let group = self.images.groups[self.plan[self.index].0].clone();
        let mut best = 0.0_f64;
        let mut matched = None;
        for (slot, split) in group.enumerate() {
            let image = &self.images.splits[split];
            let similarity = self.cache.get_or_compare(Compared::Split(split), || {
                image.compare(capture, image.comparison_method(&defaults), &mut self.scratch)
            });
            best = best.max(similarity);
            if let Some(reading) = self.status.group.get_mut(slot) {
                reading.similarity = Some(similarity);
                reading.highest = Some(reading.highest.map_or(similarity, |highest| highest.max(similarity)));
            }
            if matched.is_some() {
                continue;
            }
            let armed = &mut self.below_armed[slot];
            if similarity >= image.similarity_threshold(&defaults) {
                if image.is_below() {
                    *armed = true;
                } else {
                    matched = Some(split);
                }
            } else if image.is_below() && *armed && capture.is_some() {
                matched = Some(split);
            }
        }
        let next_tick = next_grid_tick(epoch, now, interval(self.settings.defaults.fps_limit));

        self.highest = self.highest.max(best);
        self.status.similarity = Some(best);
        self.status.highest = Some(self.highest);

        if let Some(split) = matched {
            self.matched = split;
            return self.split_found(now);
        }
        self.state = State::Comparing { epoch, next_tick };
    }

    /// Acts on the image that matched: its own flags, delay and pause time apply.
    fn split_found(&mut self, now: Duration) {
        let defaults = self.settings.defaults;
        let image = self.matched();
        if image.is_dummy() {
            return self.advance(now);
        }
        let delay = image.delay_time_ms(&defaults);
        if delay <= 0 {
            self.send_split();
            return self.advance(now);
        }
        let until = now + Duration::from_millis(delay as u64);
        self.status.current_filename = None;
        self.status.showing = Showing::SplitDelay { until };
        self.state = State::SplitDelay { until, next_tick: now };
    }

    fn send_split(&mut self) {
        let command = if self.matched().is_pause() { Command::Pause } else { Command::Split };
        self.effects.push(Effect::Send(command));
    }

    /// Move past the image that just split, then pause for its pause time.
    fn advance(&mut self, now: Duration) {
        let pause = self.matched().pause_time_secs(&self.settings.defaults);
        if self.settings.loop_splits && self.index == self.plan.len() - 1 {
            self.index = 0;
        } else {
            self.index += 1;
        }
        if pause > 0.0 {
            self.enter_pause(now, pause);
        } else {
            self.begin_split(now);
        }
    }

    fn enter_pause(&mut self, now: Duration, pause: f64) {
        let until = now.saturating_add(secs(pause));
        self.status.current_filename = None;
        self.status.showing = Showing::Pause { until };
        self.state = State::Paused { until, index_at_start: self.index, next_tick: now };
    }

    /// How often the reset image is checked during pauses and delays. AutoSplit checks it every
    /// millisecond; checking at the reset image's FPS limit catches the same frames.
    fn reset_check_interval(&self) -> Duration {
        match &self.images.reset {
            Some(reset) if self.settings.enable_auto_reset => interval(self.settings.defaults.fps_limit),
            _ => Duration::MAX,
        }
    }

    /// Compares the reset image, outside its pause window, and resets on a match. Returns true
    /// when the run is over.
    fn check_reset(&mut self, now: Duration, capture: Option<BgraView>) -> bool {
        let defaults = self.settings.defaults;
        match &self.images.reset {
            None => {
                self.status.reset_similarity = Some(ResetReading::NotApplicable);
                self.status.reset_threshold = None;
                self.status.reset_highest = None;
            }
            Some(_) if !self.settings.enable_auto_reset => self.status.reset_similarity = Some(ResetReading::Disabled),
            Some(reset) => {
                let threshold = reset.similarity_threshold(&defaults);
                let paused = self.in_reset_pause_window(now);
                self.status.reset_threshold = Some(threshold);
                if paused {
                    self.status.reset_similarity = Some(ResetReading::Paused);
                } else {
                    let similarity = self.cache.get_or_compare(Compared::Reset, || {
                        reset.compare(capture, reset.comparison_method(&defaults), &mut self.scratch)
                    });
                    self.reset_highest = self.reset_highest.max(similarity);
                    self.status.reset_similarity = Some(ResetReading::Similarity(similarity));
                    self.status.reset_highest = Some(self.reset_highest);
                    if similarity >= threshold {
                        self.effects.push(Effect::Send(Command::Reset));
                        self.stop();
                    }
                }
            }
        }
        !self.is_running()
    }

    fn stop(&mut self) {
        self.state = State::Idle;
        self.status = Status::idle(self.timer_synced);
        self.effects.push(Effect::Stopped);
    }
}
