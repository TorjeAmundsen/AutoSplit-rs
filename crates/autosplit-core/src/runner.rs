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
use crate::split_folder::SplitSet;
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
    pub reset_similarity: Option<ResetReading>,
    pub reset_highest: Option<f64>,
    pub reset_threshold: Option<f64>,
    pub buttons: Buttons,
}

impl Status {
    fn idle(auto_controlled: bool) -> Self {
        Self {
            running: false,
            start_status: StartStatus::NotFound,
            showing: Showing::Nothing,
            current_filename: None,
            similarity: None,
            highest: None,
            threshold: None,
            loop_position: None,
            reset_similarity: None,
            reset_highest: None,
            reset_threshold: None,
            buttons: Buttons {
                start: !auto_controlled,
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
    /// Controlled by LiveSplit. Fixed for the runner's lifetime, like AutoSplit's command line flag.
    auto_controlled: bool,
    images: SplitSet,
    /// `split_images_and_loop_number`: each split image repeated by its loop count.
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
    below_armed: bool,
    run_start: Duration,
    scratch: CompareScratch,
    status: Status,
    effects: Vec<Effect>,
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
    pub fn new(settings: RunnerSettings, auto_controlled: bool) -> Self {
        Self {
            settings,
            auto_controlled,
            images: SplitSet::default(),
            plan: Vec::new(),
            groups: Vec::new(),
            group_of: Vec::new(),
            index: 0,
            state: State::Idle,
            highest: 0.0,
            reset_highest: 0.0,
            below_armed: false,
            run_start: Duration::ZERO,
            scratch: CompareScratch::default(),
            status: Status::idle(auto_controlled),
            effects: Vec::new(),
        }
    }

    pub fn settings(&self) -> &RunnerSettings {
        &self.settings
    }

    /// Takes effect from the next comparison, like editing settings mid-run in AutoSplit.
    pub fn set_settings(&mut self, settings: RunnerSettings) {
        self.settings = settings;
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
        self.images = images;
        self.status = Status::idle(self.auto_controlled);
        self.index = 0;
        self.highest = 0.0;
        self.reset_highest = 0.0;
        self.below_armed = false;
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
        self.status = Status::idle(self.auto_controlled);
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
        self.images = images;
        self.run_start = run_start;
        self.build_plan();
        if self.status.start_status == StartStatus::Ready {
            self.status.start_status = StartStatus::NotReady;
        }
        self.status.running = true;
        self.index = 0;
        self.below_armed = false;
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
        self.plan = self
            .images
            .splits
            .iter()
            .enumerate()
            .flat_map(|(index, image)| (1..=image.loops).map(move |loop_number| (index, loop_number)))
            .collect();
        self.groups = vec![Vec::new()];
        self.group_of.clear();
        for i in 0..self.plan.len() {
            self.groups.last_mut().unwrap().push(i);
            self.group_of.push(self.groups.len() - 1);
            if !self.plan_image(i).is_dummy() && i < self.plan.len() - 1 {
                self.groups.push(Vec::new());
            }
        }
    }

    fn plan_image(&self, plan_index: usize) -> &SplitImage {
        &self.images.splits[self.plan[plan_index].0]
    }

    fn out_of_range(&self) -> bool {
        self.index >= self.plan.len()
    }

    fn current(&self) -> &SplitImage {
        self.plan_image(self.index)
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
        let (filename, threshold) = (image.filename.clone(), image.similarity_threshold(&defaults));
        self.status.showing = Showing::Image(image_ref);
        self.status.current_filename = Some(filename);
        self.status.threshold = Some(threshold);
        self.status.loop_position = match image_ref {
            ImageRef::Start => None,
            ImageRef::Split(_) => {
                let (split, loop_number) = self.plan[self.index];
                Some((loop_number, self.images.splits[split].loops))
            }
        };
    }

    /// Shows the current split image and resets its highest similarity. Returns false past the
    /// last split.
    fn update_split_image(&mut self) -> bool {
        self.highest = 0.0;
        self.below_armed = false;
        if self.out_of_range() {
            return false;
        }
        self.show_image(ImageRef::Split(self.plan[self.index].0));
        true
    }

    /// Reset button or hotkey, or LiveSplit's "reset". Nothing is sent: the timer was already
    /// reset by whatever triggered this.
    pub fn reset(&mut self) {
        if self.is_running() {
            self.stop();
        }
    }

    /// "Skip Split" (`navigate_image_only = false`): moves past the current split and the dummies
    /// before it, and presses the skip hotkey. "Next Img." or the skip hotkey (`true`): moves to
    /// the next image only. Neither moves past the last split. LiveSplit's own split does, and
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
        let livesplit_split = self.auto_controlled && !navigate_image_only;
        if target >= self.plan.len() && !livesplit_split {
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
        // When LiveSplit controls the timer, it skipped or undid already
        if !navigate_image_only && !self.auto_controlled {
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
    /// [`Runner::undo`]. Skip and undo split are LiveSplit's job when it controls the timer.
    fn refresh_buttons(&mut self) {
        if !self.is_running() {
            return;
        }
        let navigable = self.can_navigate();
        let group = if navigable { self.group_of[self.index] } else { 0 };
        let manual = !self.auto_controlled;
        self.status.buttons = Buttons {
            start: false,
            reset: manual,
            next_image: navigable && self.index + 1 < self.plan.len(),
            previous_image: navigable && self.index > 0,
            skip_split: navigable && manual && group + 1 < self.groups.len(),
            undo_split: navigable && manual && group > 0,
        };
    }

    /// Processes everything due at `now`. `capture` is `None` (or empty) when the capture was lost.
    pub fn tick(&mut self, now: Duration, capture: Option<BgraView>) {
        self.process(now, capture.filter(|capture| !capture.is_empty()));
        self.refresh_buttons();
    }

    fn process(&mut self, now: Duration, capture: Option<BgraView>) {
        self.scratch.begin_frame();
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
                    let check_at = now.saturating_add(self.reset_check_interval());
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
                    self.state =
                        State::SplitDelay { until, next_tick: now.saturating_add(self.reset_check_interval()) };
                }
            }
            State::WaitingForStart { .. } | State::StartDelay { .. } | State::Comparing { .. } => {}
        }
    }

    /// One comparison of the start image.
    fn compare_start_image(&mut self, epoch: Duration, now: Duration, capture: Option<BgraView>) {
        let defaults = self.settings.defaults;
        let start = self.images.start.as_ref().expect("start image");
        let similarity = start.compare(capture, start.comparison_method(&defaults), &mut self.scratch);
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
        if below && !self.below_armed && difference >= 0.0 {
            self.below_armed = true;
            self.state = State::WaitingForStart { epoch, next_tick };
            return;
        }
        let triggered =
            (below && self.below_armed && difference < 0.0 && capture.is_some()) || (!below && difference >= 0.0);
        if !triggered {
            self.state = State::WaitingForStart { epoch, next_tick };
            return;
        }
        self.below_armed = false;
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

    /// One comparison of the current split image.
    fn compare_split_image(&mut self, epoch: Duration, now: Duration, capture: Option<BgraView>) {
        if self.check_reset(now, capture) {
            return;
        }
        let defaults = self.settings.defaults;
        let image = &self.images.splits[self.plan[self.index].0];
        let similarity = image.compare(capture, image.comparison_method(&defaults), &mut self.scratch);
        let threshold = image.similarity_threshold(&defaults);
        let below = image.is_below();
        let next_tick = next_grid_tick(epoch, now, interval(self.settings.defaults.fps_limit));

        self.highest = self.highest.max(similarity);
        self.status.similarity = Some(similarity);
        self.status.highest = Some(self.highest);

        if similarity >= threshold {
            if !below {
                return self.split_found(now);
            }
            self.below_armed = true;
        } else if below && self.below_armed && capture.is_some() {
            self.below_armed = false;
            return self.split_found(now);
        }
        self.state = State::Comparing { epoch, next_tick };
    }

    fn split_found(&mut self, now: Duration) {
        let defaults = self.settings.defaults;
        let image = self.current();
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
        let command = if self.current().is_pause() { Command::Pause } else { Command::Split };
        self.effects.push(Effect::Send(command));
    }

    /// Move past the image that just split, then pause for its pause time.
    fn advance(&mut self, now: Duration) {
        let pause = self.current().pause_time_secs(&self.settings.defaults);
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
                let start_pause = self.images.start.as_ref().map_or(0.0, |start| start.pause_time_secs(&defaults));
                let pause_window = reset.pause_time_secs(&defaults).max(start_pause);
                let paused = now.saturating_sub(self.run_start).as_secs_f64() <= pause_window;
                self.status.reset_threshold = Some(threshold);
                if paused {
                    self.status.reset_similarity = Some(ResetReading::Paused);
                } else {
                    let similarity = reset.compare(capture, reset.comparison_method(&defaults), &mut self.scratch);
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
        self.status = Status::idle(self.auto_controlled);
        self.effects.push(Effect::Stopped);
    }
}
