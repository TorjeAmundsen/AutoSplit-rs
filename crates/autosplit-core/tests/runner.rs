//! Split state machine scenarios: real split image folders, solid color frames, simulated time.

use std::time::Duration;

use autosplit_core::commands::Command;
use autosplit_core::compare::{CompareScratch, ComparisonMethod};
use autosplit_core::image::BgraImage;
use autosplit_core::runner::{Buttons, Captured, Effect, ResetReading, Runner, RunnerSettings, StartStatus};
use autosplit_core::split_folder::{SplitSet, ValidationContext, load_split_folder};
use autosplit_core::split_image::SplitDefaults;
use autosplit_core::split_image::SplitImage;
use tempfile::TempDir;
use test_case::test_case;

type Bgr = [u8; 3];
const RED: Bgr = [0, 0, 255];
const BLUE: Bgr = [255, 0, 0];
const GREEN: Bgr = [0, 255, 0];
const BLACK: Bgr = [0, 0, 0];

fn solid(color: Bgr) -> BgraImage {
    let pixel = [color[0], color[1], color[2], 255];
    BgraImage::from_raw(320, 240, pixel.repeat(320 * 240))
}

fn defaults() -> RunnerSettings {
    RunnerSettings {
        defaults: SplitDefaults {
            comparison_method: 0,
            similarity_threshold: 0.95,
            delay_time: 0,
            pause_time: 0.0,
            fps_limit: 60,
        },
        loop_splits: false,
        enable_auto_reset: true,
    }
}

const ALL_HOTKEYS_SET: ValidationContext = ValidationContext {
    split_hotkey_set: true,
    reset_hotkey_set: true,
    pause_hotkey_set: true,
    hotkeys_needed: true,
    group_split_images: false,
};

/// Plays the host's part: reloads the folder on start and after a run, like AutoSplit does.
struct Harness {
    directory: TempDir,
    runner: Runner,
    now: Duration,
    effects: Vec<(Duration, Effect)>,
    /// Numbers frames for the runner's comparison cache.
    next_sequence: u64,
    /// Gives every tick a new frame number, which turns the cache off.
    new_frame_every_tick: bool,
    context: ValidationContext,
}

impl Harness {
    fn new(files: &[(&str, Bgr)], settings: RunnerSettings) -> Self {
        Self::with_link(files, settings, false)
    }

    /// Split images with the same number are compared for the same split.
    fn grouped(files: &[(&str, Bgr)], settings: RunnerSettings) -> Self {
        let mut harness = Self::new(files, settings);
        harness.context.group_split_images = true;
        let images = harness.load();
        harness.runner.arm(images, Duration::ZERO);
        harness
    }

    /// Driven by LiveSplit or LiveSplit One, which report the timer's splits.
    fn synced(files: &[(&str, Bgr)], settings: RunnerSettings) -> Self {
        Self::with_link(files, settings, true)
    }

    fn with_link(files: &[(&str, Bgr)], settings: RunnerSettings, timer_synced: bool) -> Self {
        let directory = tempfile::tempdir().unwrap();
        for (name, color) in files {
            std::fs::write(directory.path().join(name), solid(*color).encode_png().unwrap()).unwrap();
        }
        let runner = Runner::new(settings, timer_synced);
        let mut harness = Self {
            directory,
            runner,
            now: Duration::ZERO,
            effects: Vec::new(),
            next_sequence: 0,
            new_frame_every_tick: false,
            context: ALL_HOTKEYS_SET,
        };
        let images = harness.load();
        harness.runner.arm(images, Duration::ZERO);
        harness
    }

    fn load(&self) -> SplitSet {
        load_split_folder(self.directory.path(), &self.context).result.expect("valid folder")
    }

    /// The split hotkey or start button.
    fn start(&mut self) {
        let images = self.load();
        self.runner.start(images, self.now, self.now);
    }

    /// Feeds `frame` (or a lost capture) for `duration`, ticking whenever the runner asks to.
    fn feed(&mut self, frame: Option<Bgr>, duration: Duration) {
        let frame = frame.map(solid);
        self.next_sequence += 1;
        let end = self.now + duration;
        for _ in 0..100_000 {
            match self.runner.next_wakeup() {
                Some(next) if next <= end => {
                    self.now = self.now.max(next);
                    if self.new_frame_every_tick {
                        self.next_sequence += 1;
                    }
                    let sequence = self.next_sequence;
                    self.runner.tick(self.now, frame.as_ref().map(|frame| Captured { view: frame.view(), sequence }));
                    self.handle_effects();
                }
                _ => break,
            }
        }
        self.now = end;
    }

    /// Feeds a zero-size frame, like a minimized window can produce.
    fn feed_empty(&mut self, duration: Duration) {
        let empty = BgraImage::default();
        let end = self.now + duration;
        while let Some(next) = self.runner.next_wakeup().filter(|next| *next <= end) {
            self.now = self.now.max(next);
            self.next_sequence += 1;
            let sequence = self.next_sequence;
            self.runner.tick(self.now, Some(Captured { view: empty.view(), sequence }));
            self.handle_effects();
        }
        self.now = end;
    }

    fn handle_effects(&mut self) {
        for effect in self.runner.take_effects() {
            match effect {
                Effect::RequestStart => self.start(),
                Effect::Stopped => {
                    let images = self.load();
                    self.runner.arm(images, self.now);
                }
                Effect::Send(_) => {}
            }
            self.effects.push((self.now, effect));
        }
    }

    fn sent(&self) -> Vec<Command> {
        self.effects
            .iter()
            .filter_map(|(_, effect)| if let Effect::Send(command) = effect { Some(*command) } else { None })
            .collect()
    }

    fn sent_at(&self) -> Vec<(Duration, Command)> {
        self.effects
            .iter()
            .filter_map(|(at, effect)| if let Effect::Send(command) = effect { Some((*at, *command)) } else { None })
            .collect()
    }

    fn stopped(&self) -> bool {
        self.effects.iter().any(|(_, effect)| *effect == Effect::Stopped)
    }

    fn skip(&mut self, navigate_image_only: bool) {
        self.runner.skip(self.now, navigate_image_only);
        self.handle_effects();
    }

    fn undo(&mut self, navigate_image_only: bool) {
        self.runner.undo(self.now, navigate_image_only);
        self.handle_effects();
    }

    fn current_filename(&self) -> Option<&str> {
        self.runner.status().current_filename.as_deref()
    }
}

const SECOND: Duration = Duration::from_secs(1);

fn ms(milliseconds: u64) -> Duration {
    Duration::from_millis(milliseconds)
}

#[test]
fn split_is_sent_when_the_image_matches() {
    // Arrange
    let mut harness = Harness::new(&[("1.png", RED), ("2.png", BLUE)], defaults());
    harness.start();

    // Act
    harness.feed(Some(RED), SECOND);

    // Assert
    assert_eq!(harness.sent(), [Command::Split]);
}

#[test]
fn nothing_is_sent_while_the_image_does_not_match() {
    // Arrange
    let mut harness = Harness::new(&[("1.png", RED)], defaults());
    harness.start();

    // Act
    harness.feed(Some(BLUE), 5 * SECOND);

    // Assert
    assert_eq!(harness.sent(), []);
}

#[test]
fn run_stops_after_the_last_split() {
    // Arrange
    let mut harness = Harness::new(&[("1.png", RED), ("2.png", BLUE)], defaults());
    harness.start();

    // Act
    harness.feed(Some(RED), SECOND);
    harness.feed(Some(BLUE), SECOND);

    // Assert
    assert_eq!(
        (harness.sent(), harness.stopped(), harness.runner.is_running()),
        (vec![Command::Split, Command::Split], true, false)
    );
}

#[test]
fn pause_time_stops_comparisons_until_it_ends() {
    // Arrange
    let mut harness = Harness::new(&[("1_[2].png", RED), ("2.png", RED)], defaults());
    harness.start();

    // Act
    harness.feed(Some(RED), ms(1990));
    let during_pause = harness.sent();
    harness.feed(Some(RED), ms(100));

    // Assert
    assert_eq!((during_pause, harness.sent()), (vec![Command::Split], vec![Command::Split, Command::Split]));
}

#[test]
fn delay_sends_the_split_after_the_delay() {
    // Arrange
    let mut harness = Harness::new(&[("1_#500#.png", RED), ("2.png", BLUE)], defaults());
    harness.start();

    // Act
    harness.feed(Some(RED), SECOND);

    // Assert
    assert_eq!(harness.sent_at(), [(ms(500), Command::Split)]);
}

#[test]
fn below_flag_splits_once_similarity_drops_below_the_threshold() {
    // Arrange
    let mut harness = Harness::new(&[("1_{b}.png", RED), ("2.png", GREEN)], defaults());
    harness.start();

    // Act
    harness.feed(Some(RED), SECOND);
    let while_above = harness.sent();
    harness.feed(Some(BLUE), ms(20));

    // Assert
    assert_eq!((while_above, harness.sent()), (vec![], vec![Command::Split]));
}

#[test]
fn below_flag_does_not_split_on_a_lost_capture() {
    // Arrange
    let mut harness = Harness::new(&[("1_{b}.png", RED), ("2.png", GREEN)], defaults());
    harness.start();

    // Act
    harness.feed(Some(RED), SECOND);
    harness.feed(None, SECOND);

    // Assert
    assert_eq!(harness.sent(), []);
}

#[test]
fn dummy_flag_moves_on_without_sending() {
    // Arrange
    let mut harness = Harness::new(&[("1_{d}.png", RED), ("2.png", BLUE)], defaults());
    harness.start();

    // Act
    harness.feed(Some(RED), SECOND);

    // Assert
    assert_eq!((harness.sent(), harness.current_filename()), (vec![], Some("2.png")));
}

#[test]
fn pause_flag_sends_pause_instead_of_split() {
    // Arrange
    let mut harness = Harness::new(&[("1_{p}.png", RED), ("2.png", BLUE)], defaults());
    harness.start();

    // Act
    harness.feed(Some(RED), SECOND);

    // Assert
    assert_eq!(harness.sent(), [Command::Pause]);
}

#[test]
fn loops_use_the_same_image_again() {
    // Arrange
    let mut harness = Harness::new(&[("1_@2@.png", RED), ("2.png", BLUE)], defaults());
    harness.start();

    // Act
    harness.feed(Some(RED), ms(10));

    // Assert
    assert_eq!(
        (harness.sent(), harness.runner.status().loop_position),
        (vec![Command::Split, Command::Split], Some((1, 1)))
    );
}

#[test]
fn loop_splits_goes_back_to_the_first_image() {
    // Arrange
    let mut harness =
        Harness::new(&[("1.png", RED), ("2.png", BLUE)], RunnerSettings { loop_splits: true, ..defaults() });
    harness.start();

    // Act
    harness.feed(Some(RED), ms(10));
    harness.feed(Some(BLUE), ms(10));

    // Assert
    assert_eq!((harness.current_filename(), harness.runner.is_running()), (Some("1.png"), true));
}

#[test]
fn reset_image_resets_the_run() {
    // Arrange
    let mut harness = Harness::new(&[("1.png", RED), ("2.png", BLUE), ("reset.png", BLACK)], defaults());
    harness.start();

    // Act
    harness.feed(Some(BLACK), ms(50));

    // Assert
    assert_eq!((harness.sent(), harness.stopped()), (vec![Command::Reset], true));
}

#[test]
fn reset_image_is_paused_during_its_pause_time_after_the_start() {
    // Arrange
    let mut harness = Harness::new(&[("1.png", RED), ("reset_[3].png", BLACK)], defaults());
    harness.start();

    // Act
    harness.feed(Some(BLACK), ms(2900));

    // Assert
    assert_eq!((harness.sent(), harness.runner.status().reset_similarity), (vec![], Some(ResetReading::Paused)));
}

#[test]
fn pause_wakes_up_when_the_reset_image_pause_is_over() {
    // Arrange
    let mut harness = Harness::new(&[("1_[30].png", RED), ("2.png", BLUE), ("reset_[3].png", BLACK)], defaults());
    harness.start();
    harness.feed(Some(RED), ms(100));

    // Act
    let wakeup = harness.runner.next_wakeup().unwrap();

    // Assert
    assert!(wakeup >= ms(3000), "woke at {wakeup:?}");
}

#[test]
fn reset_image_is_checked_once_its_pause_is_over() {
    // Arrange
    let mut harness = Harness::new(&[("1_[30].png", RED), ("2.png", BLUE), ("reset_[3].png", BLACK)], defaults());
    harness.start();
    harness.feed(Some(RED), ms(100));

    // Act
    harness.feed(Some(BLACK), ms(3100));

    // Assert
    assert_eq!(harness.sent(), [Command::Split, Command::Reset]);
}

#[test]
fn disabled_auto_reset_ignores_the_reset_image() {
    // Arrange
    let settings = RunnerSettings { enable_auto_reset: false, ..defaults() };
    let mut harness = Harness::new(&[("1.png", RED), ("reset.png", BLACK)], settings);
    harness.start();

    // Act
    harness.feed(Some(BLACK), SECOND);

    // Assert
    assert_eq!((harness.sent(), harness.runner.status().reset_similarity), (vec![], Some(ResetReading::Disabled)));
}

#[test]
fn reset_image_is_checked_during_pauses() {
    // Arrange
    let mut harness = Harness::new(&[("1_[30].png", RED), ("2.png", BLUE), ("reset.png", BLACK)], defaults());
    harness.start();

    // Act
    harness.feed(Some(RED), ms(10));
    harness.feed(Some(BLACK), ms(50));

    // Assert
    assert_eq!(harness.sent(), [Command::Split, Command::Reset]);
}

#[test]
fn start_image_starts_the_run() {
    // Arrange
    let mut harness = Harness::new(&[("start_auto_splitter.png", GREEN), ("1.png", RED)], defaults());

    // Act
    harness.feed(Some(GREEN), ms(100));

    // Assert
    assert_eq!((harness.sent(), harness.runner.is_running()), (vec![Command::Start], true));
}

#[test]
fn start_image_waits_for_its_delay() {
    // Arrange
    let mut harness = Harness::new(&[("start_auto_splitter_#300#.png", GREEN), ("1.png", RED)], defaults());

    // Act
    harness.feed(Some(GREEN), SECOND);

    // Assert
    let first_tick = Duration::from_secs_f64(1.0 / 60.0);
    assert_eq!(harness.sent_at(), [(first_tick + ms(300), Command::Start)]);
}

#[test]
fn start_image_pause_delays_the_first_split_image() {
    // Arrange
    let mut harness = Harness::new(&[("start_auto_splitter_[2].png", GREEN), ("1.png", RED)], defaults());
    harness.feed(Some(GREEN), ms(100));

    // Act
    harness.feed(Some(RED), ms(1500));

    // Assert
    assert_eq!(harness.sent(), [Command::Start]);
}

#[test]
fn start_image_pause_shows_no_file_name() {
    // Arrange
    let mut harness = Harness::new(&[("start_auto_splitter_[2].png", GREEN), ("1.png", RED)], defaults());

    // Act
    harness.feed(Some(GREEN), ms(100));

    // Assert
    assert_eq!((harness.runner.is_running(), harness.current_filename()), (true, None));
}

#[test]
fn pause_after_a_split_shows_no_file_name() {
    // Arrange
    let mut harness = Harness::new(&[("1_[2].png", RED), ("2.png", BLUE)], defaults());
    harness.start();

    // Act
    harness.feed(Some(RED), ms(100));

    // Assert
    assert_eq!((harness.sent(), harness.current_filename()), (vec![Command::Split], None));
}

#[test]
fn dummy_start_image_starts_without_sending() {
    // Arrange
    let mut harness = Harness::new(&[("start_auto_splitter_{d}.png", GREEN), ("1.png", RED)], defaults());

    // Act
    harness.feed(Some(GREEN), ms(100));

    // Assert
    assert_eq!((harness.sent(), harness.runner.is_running()), (vec![], true));
}

#[test]
fn skip_split_skips_dummies_up_to_the_next_real_split() {
    // Arrange
    let mut harness = Harness::new(&[("1_{d}.png", RED), ("2.png", RED), ("3.png", BLUE)], defaults());
    harness.start();
    harness.feed(Some(BLACK), ms(10));

    // Act
    harness.skip(false);

    // Assert
    assert_eq!((harness.sent(), harness.current_filename()), (vec![Command::Skip], Some("3.png")));
}

#[test]
fn undo_split_goes_back_to_the_previous_real_split() {
    // Arrange
    let mut harness =
        Harness::new(&[("1.png", RED), ("2_{d}.png", BLUE), ("3.png", GREEN), ("4.png", BLACK)], defaults());
    harness.start();
    harness.feed(Some(RED), ms(100));
    harness.feed(Some(BLUE), ms(100));
    harness.feed(Some(GREEN), ms(100));

    // Act
    harness.undo(false);

    // Assert
    assert_eq!(
        (harness.sent(), harness.current_filename()),
        (vec![Command::Split, Command::Split, Command::Undo], Some("3.png"))
    );
}

#[test]
fn next_image_on_the_last_split_is_ignored() {
    // Arrange
    let mut harness = Harness::new(&[("1.png", RED)], defaults());
    harness.start();
    harness.feed(Some(BLACK), ms(10));

    // Act
    harness.skip(true);

    // Assert
    assert_eq!((harness.runner.is_running(), harness.current_filename()), (true, Some("1.png")));
}

#[test]
fn skip_is_ignored_during_a_split_delay() {
    // Arrange
    let mut harness = Harness::new(&[("1_#1000#.png", RED), ("2.png", BLUE), ("3.png", GREEN)], defaults());
    harness.start();
    harness.feed(Some(RED), ms(100));

    // Act
    harness.skip(false);

    // Assert
    assert_eq!((harness.sent(), harness.runner.status().current_filename.as_deref()), (vec![], None));
}

#[test]
fn skip_during_a_pause_ends_the_pause() {
    // Arrange
    let mut harness = Harness::new(&[("1_[60].png", RED), ("2.png", BLUE), ("3.png", GREEN)], defaults());
    harness.start();
    harness.feed(Some(RED), ms(10));
    harness.skip(false);

    // Act
    harness.feed(Some(GREEN), ms(50));

    // Assert
    assert_eq!(harness.sent(), [Command::Split, Command::Skip, Command::Split]);
}

#[test_case(60; "60 fps")]
#[test_case(144; "144 fps")]
#[test_case(240; "240 fps")]
#[test_case(7; "7 fps")]
fn wakeups_always_move_forward(fps_limit: i64) {
    // Arrange
    let settings = RunnerSettings { defaults: SplitDefaults { fps_limit, ..defaults().defaults }, ..defaults() };
    let mut harness = Harness::new(&[("1.png", RED)], settings);
    harness.start();
    let mut previous = None;

    // Act
    let mut stalled = false;
    for _ in 0..5000 {
        let next = harness.runner.next_wakeup().unwrap();
        stalled |= previous.is_some_and(|previous| next <= previous);
        previous = Some(next);
        harness.runner.tick(next, Some(Captured { view: solid(BLUE).view(), sequence: 1 }));
    }

    // Assert
    assert!(!stalled);
}

#[test]
fn comparisons_follow_the_fps_limit() {
    // Arrange
    let mut harness = Harness::new(&[("1.png", RED)], defaults());
    harness.start();
    let mut ticks = 0;

    // Act
    let end = harness.now + SECOND;
    while let Some(next) = harness.runner.next_wakeup().filter(|next| *next < end) {
        harness.now = next;
        harness.runner.tick(next, Some(Captured { view: solid(BLUE).view(), sequence: 1 }));
        ticks += 1;
    }

    // Assert
    assert_eq!(ticks, 60);
}

#[test]
fn undo_hotkey_on_the_first_split_is_ignored() {
    // Arrange
    let mut harness = Harness::new(&[("1.png", RED), ("2.png", BLUE)], defaults());
    harness.start();
    harness.feed(Some(BLACK), ms(100));

    // Act
    harness.undo(true);

    // Assert
    assert_eq!((harness.runner.is_running(), harness.current_filename()), (true, Some("1.png")));
}

#[test]
fn undo_split_button_on_the_first_split_is_ignored() {
    // Arrange
    let mut harness = Harness::new(&[("1.png", RED), ("2.png", BLUE)], defaults());
    harness.start();
    harness.feed(Some(BLACK), ms(100));

    // Act
    harness.undo(false);

    // Assert
    assert_eq!((harness.sent(), harness.runner.is_running()), (vec![], true));
}

#[test]
fn livesplit_undo_on_the_first_split_does_nothing() {
    // Arrange
    let mut harness = Harness::synced(&[("1.png", RED), ("2.png", BLUE)], defaults());
    harness.start();
    harness.feed(Some(BLACK), ms(100));

    // Act
    harness.undo(false);

    // Assert
    assert_eq!((harness.sent(), harness.current_filename()), (vec![], Some("1.png")));
}

#[test]
fn livesplit_skip_moves_on_and_reports_the_skip_for_screenshots() {
    // Arrange
    let mut harness = Harness::synced(&[("1.png", RED), ("2.png", BLUE)], defaults());
    harness.start();
    harness.feed(Some(BLACK), ms(100));

    // Act
    harness.skip(false);

    // Assert
    assert_eq!((harness.sent(), harness.current_filename()), (vec![Command::Skip], Some("2.png")));
}

#[test]
fn livesplit_split_on_the_last_split_ends_the_run() {
    // Arrange
    let mut harness = Harness::synced(&[("1.png", RED)], defaults());
    harness.start();
    harness.feed(Some(BLACK), ms(100));

    // Act
    harness.skip(false);

    // Assert
    assert_eq!((harness.sent(), harness.runner.is_running()), (vec![Command::Skip], false));
}

#[test]
fn undo_split_during_the_pause_after_the_first_split_goes_back() {
    // Arrange
    let mut harness = Harness::new(&[("1_[10].png", RED), ("2.png", BLUE)], defaults());
    harness.start();
    harness.feed(Some(RED), ms(100));

    // Act
    harness.undo(false);

    // Assert
    assert_eq!((harness.sent(), harness.current_filename()), (vec![Command::Split, Command::Undo], Some("1_[10].png")));
}

#[test]
fn buttons_on_the_first_split_only_allow_moving_forward() {
    // Arrange
    let mut harness = Harness::new(&[("1.png", RED), ("2.png", BLUE)], defaults());
    harness.start();

    // Act
    harness.feed(Some(BLACK), ms(100));

    // Assert
    let buttons = harness.runner.status().buttons;
    let expected = Buttons {
        start: false,
        reset: true,
        skip_split: true,
        undo_split: false,
        next_image: true,
        previous_image: false,
    };
    assert_eq!(buttons, expected);
}

#[test]
fn buttons_are_disabled_during_a_split_delay() {
    // Arrange
    let mut harness = Harness::new(&[("1_#1000#.png", RED), ("2.png", BLUE), ("3.png", GREEN)], defaults());
    harness.start();

    // Act
    harness.feed(Some(RED), ms(100));

    // Assert
    let buttons = harness.runner.status().buttons;
    let expected = Buttons {
        start: false,
        reset: true,
        skip_split: false,
        undo_split: false,
        next_image: false,
        previous_image: false,
    };
    assert_eq!(buttons, expected);
}

#[test]
fn skip_split_button_on_the_last_real_split_is_ignored() {
    // Arrange
    let mut harness = Harness::new(&[("1.png", RED), ("2.png", BLUE)], defaults());
    harness.start();
    harness.feed(Some(RED), ms(100));

    // Act
    harness.skip(false);

    // Assert
    assert_eq!((harness.sent(), harness.runner.is_running()), (vec![Command::Split], true));
}

#[test]
fn empty_capture_does_not_trigger_a_below_flag_split() {
    // Arrange
    let mut harness = Harness::new(&[("1_{b}.png", RED), ("2.png", GREEN)], defaults());
    harness.start();
    harness.feed(Some(RED), ms(100));

    // Act
    harness.feed_empty(ms(100));

    // Assert
    assert_eq!(harness.sent(), []);
}

#[test]
fn wakeups_move_forward_during_pauses_with_an_absurd_fps_limit() {
    // Arrange
    let settings =
        RunnerSettings { defaults: SplitDefaults { fps_limit: 10_000_000_000, ..defaults().defaults }, ..defaults() };
    let mut harness = Harness::new(&[("1_[5].png", RED), ("2.png", BLUE), ("reset.png", BLACK)], settings);
    harness.start();
    harness.feed(Some(RED), Duration::ZERO);
    let mut previous = harness.now;

    // Act
    let mut stalled = false;
    for _ in 0..100 {
        let next = harness.runner.next_wakeup().unwrap();
        stalled |= next <= previous;
        previous = next;
        harness.runner.tick(next, Some(Captured { view: solid(BLUE).view(), sequence: 1 }));
    }

    // Assert
    assert!(!stalled);
}

#[test]
fn arm_is_ignored_while_running() {
    // Arrange
    let mut harness = Harness::new(&[("1.png", RED)], defaults());
    harness.start();
    let images = harness.load();

    // Act
    let armed = harness.runner.arm(images, harness.now);

    // Assert
    assert_eq!((armed, harness.runner.is_running()), (false, true));
}

#[test]
fn start_is_ignored_while_running() {
    // Arrange
    let mut harness = Harness::new(&[("1.png", RED)], defaults());
    harness.start();
    let images = harness.load();

    // Act
    let started = harness.runner.start(images, harness.now, harness.now);

    // Assert
    assert!(!started);
}

#[test]
fn disarm_stops_looking_for_the_start_image() {
    // Arrange
    let mut harness = Harness::new(&[("start_auto_splitter.png", GREEN), ("1.png", RED)], defaults());

    // Act
    harness.runner.disarm();
    harness.feed(Some(GREEN), SECOND);

    // Assert
    assert_eq!((harness.sent(), harness.runner.next_wakeup()), (vec![], None));
}

#[test]
fn manual_start_without_a_start_image_keeps_start_status_not_found() {
    // Arrange
    let mut harness = Harness::new(&[("1.png", RED)], defaults());

    // Act
    harness.start();

    // Assert
    assert_eq!(harness.runner.status().start_status, StartStatus::NotFound);
}

#[test]
fn reset_image_sharing_a_resize_with_the_split_image_compares_the_same() {
    // Arrange
    let mut harness = Harness::new(&[("1.png", RED)], defaults());
    let mut half_transparent = solid(BLACK);
    for pixel in half_transparent.data_mut().as_chunks_mut::<4>().0.iter_mut().step_by(2) {
        pixel[3] = 0;
    }
    let reset_path = harness.directory.path().join("reset.png");
    std::fs::write(&reset_path, half_transparent.encode_png().unwrap()).unwrap();
    let frame = solid(BLUE);
    let expected = SplitImage::load(&reset_path).compare(
        Some(frame.view()),
        Some(ComparisonMethod::L2Norm),
        &mut CompareScratch::default(),
    );
    harness.start();

    // Act
    harness.feed(Some(BLUE), ms(100));

    // Assert
    assert_eq!(harness.runner.status().reset_similarity, Some(ResetReading::Similarity(expected)));
}

#[test]
fn needs_no_frames_while_idle() {
    // Arrange
    let harness = Harness::new(&[("1.png", RED)], defaults());

    // Act
    let needs = harness.runner.needs_frames(harness.now);

    // Assert
    assert!(!needs);
}

#[test]
fn needs_frames_while_comparing() {
    // Arrange
    let mut harness = Harness::new(&[("1.png", RED)], defaults());

    // Act
    harness.start();

    // Assert
    assert!(harness.runner.needs_frames(harness.now));
}

#[test]
fn needs_no_frames_during_a_pause_without_a_reset_image() {
    // Arrange
    let mut harness = Harness::new(&[("1_[5].png", RED), ("2.png", BLUE)], defaults());
    harness.start();

    // Act
    harness.feed(Some(RED), ms(100));

    // Assert
    assert!(!harness.runner.needs_frames(harness.now));
}

#[test]
fn needs_frames_during_a_pause_with_a_reset_image() {
    // Arrange
    let mut harness = Harness::new(&[("1_[5].png", RED), ("2.png", BLUE), ("reset.png", GREEN)], defaults());
    harness.start();

    // Act
    harness.feed(Some(RED), ms(100));

    // Assert
    assert!(harness.runner.needs_frames(harness.now));
}

type Feed = (Option<Bgr>, u64);

/// Feeds the same frames with the cache on and off, returning what each run sent when.
fn sent_with_and_without_cache(files: &[(&str, Bgr)], feeds: &[Feed]) -> [Vec<(Duration, Command)>; 2] {
    [false, true].map(|new_frame_every_tick| {
        let mut harness = Harness::new(files, defaults());
        harness.new_frame_every_tick = new_frame_every_tick;
        harness.start();
        for &(frame, milliseconds) in feeds {
            harness.feed(frame, ms(milliseconds));
        }
        harness.sent_at()
    })
}

#[test_case(&[("1.png", RED), ("2.png", BLUE)], &[(Some(BLACK), 100), (Some(RED), 50), (Some(BLUE), 50)]; "plain splits")]
#[test_case(&[("1_{d}.png", RED), ("2_{d}.png", RED), ("3.png", RED)], &[(Some(RED), 100)]; "dummy chain on one frame")]
#[test_case(&[("1_{b}.png", RED), ("2.png", BLUE)], &[(Some(RED), 100), (Some(BLACK), 100), (Some(BLUE), 100)]; "below flag")]
#[test_case(&[("1_[0.2].png", RED), ("2.png", BLUE), ("reset_[0.1].png", GREEN)], &[(Some(RED), 100), (Some(GREEN), 300)]; "reset during a pause")]
#[test_case(&[("1_#200#.png", RED), ("2.png", BLUE)], &[(Some(RED), 50), (Some(BLUE), 400)]; "split delay")]
fn cached_comparisons_split_at_the_same_times(files: &[(&str, Bgr)], feeds: &[Feed]) {
    // Act
    let [cached, uncached] = sent_with_and_without_cache(files, feeds);

    // Assert
    assert_eq!((cached.is_empty(), cached), (false, uncached));
}

#[test]
fn any_image_of_a_group_splits() {
    // Arrange
    let mut harness = Harness::grouped(&[("1_a.png", RED), ("1_b.png", BLUE), ("2.png", GREEN)], defaults());
    harness.start();

    // Act
    harness.feed(Some(BLUE), SECOND);

    // Assert
    assert_eq!((harness.sent(), harness.current_filename()), (vec![Command::Split], Some("2.png")));
}

#[test]
fn a_group_is_one_split_position() {
    // Arrange
    let mut harness = Harness::grouped(&[("1_a.png", RED), ("1_b.png", BLUE), ("2.png", GREEN)], defaults());

    // Act
    harness.start();

    // Assert
    let status = harness.runner.status();
    assert_eq!((status.split_position, status.current_filename.as_deref()), (Some((1, 2)), Some("1_a.png")));
}

#[test_case(RED => vec![(ms(500), Command::Split)]; "image with a delay")]
#[test_case(BLUE => vec![(Duration::ZERO, Command::Split)]; "image without a delay")]
fn the_matched_image_decides_the_delay(frame: Bgr) -> Vec<(Duration, Command)> {
    // Arrange
    let mut harness = Harness::grouped(&[("1_a_#500#.png", RED), ("1_b.png", BLUE), ("2.png", GREEN)], defaults());
    harness.start();

    // Act
    harness.feed(Some(frame), SECOND);

    // Assert
    harness.sent_at()
}

#[test_case(RED => (vec![], Some("2.png".into())); "dummy image")]
#[test_case(BLUE => (vec![Command::Split], Some("2.png".into())); "split image")]
fn the_matched_image_decides_the_dummy_flag(frame: Bgr) -> (Vec<Command>, Option<String>) {
    // Arrange
    let mut harness = Harness::grouped(&[("1_a_{d}.png", RED), ("1_b.png", BLUE), ("2.png", GREEN)], defaults());
    harness.start();

    // Act
    harness.feed(Some(frame), ms(10));

    // Assert
    (harness.sent(), harness.current_filename().map(str::to_owned))
}

#[test]
fn below_flag_is_tracked_per_image_of_a_group() {
    // Arrange
    let mut harness = Harness::grouped(&[("1_a_{b}.png", RED), ("1_b.png", BLUE), ("2.png", GREEN)], defaults());
    harness.start();

    // Act
    harness.feed(Some(RED), ms(100));
    let while_matching = harness.sent();
    harness.feed(Some(BLACK), ms(100));

    // Assert
    assert_eq!((while_matching, harness.sent()), (vec![], vec![Command::Split]));
}

#[test]
fn skip_split_moves_past_the_whole_group() {
    // Arrange
    let mut harness = Harness::grouped(&[("1_a.png", RED), ("1_b.png", BLUE), ("2.png", GREEN)], defaults());
    harness.start();

    // Act
    harness.skip(false);

    // Assert
    assert_eq!((harness.sent(), harness.current_filename()), (vec![Command::Skip], Some("2.png")));
}

#[test]
fn each_image_of_a_group_has_its_own_reading() {
    // Arrange
    let mut harness = Harness::grouped(&[("1_a_{b}.png", RED), ("1_b_(0.9).png", BLUE), ("2.png", GREEN)], defaults());
    harness.start();

    // Act
    harness.feed(Some(RED), ms(100));

    // Assert
    let readings: Vec<_> = harness
        .runner
        .status()
        .group
        .iter()
        .map(|reading| (reading.similarity == Some(1.0), reading.highest == Some(1.0), reading.threshold))
        .collect();
    assert_eq!(readings, [(true, true, 0.95), (false, false, 0.9)]);
}
