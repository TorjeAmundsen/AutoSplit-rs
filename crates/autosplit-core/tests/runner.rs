//! Split state machine scenarios: real split image folders, solid color frames, simulated time.

use std::time::Duration;

use autosplit_core::commands::Command;
use autosplit_core::compare::{CompareScratch, ComparisonMethod};
use autosplit_core::image::BgraImage;
use autosplit_core::runner::{Buttons, Effect, ResetReading, Runner, RunnerSettings, StartStatus};
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
    auto_controlled: false,
};

/// Plays the host's part: reloads the folder on start and after a run, like AutoSplit does.
struct Harness {
    directory: TempDir,
    runner: Runner,
    now: Duration,
    effects: Vec<(Duration, Effect)>,
}

impl Harness {
    fn new(files: &[(&str, Bgr)], settings: RunnerSettings) -> Self {
        Self::with_control(files, settings, false)
    }

    fn with_control(files: &[(&str, Bgr)], settings: RunnerSettings, auto_controlled: bool) -> Self {
        let directory = tempfile::tempdir().unwrap();
        for (name, color) in files {
            std::fs::write(directory.path().join(name), solid(*color).encode_png().unwrap()).unwrap();
        }
        let runner = Runner::new(settings, auto_controlled);
        let mut harness = Self { directory, runner, now: Duration::ZERO, effects: Vec::new() };
        let images = harness.load();
        harness.runner.arm(images, Duration::ZERO);
        harness
    }

    fn load(&self) -> SplitSet {
        load_split_folder(self.directory.path(), &ALL_HOTKEYS_SET).result.expect("valid folder")
    }

    /// The split hotkey or start button.
    fn start(&mut self) {
        let images = self.load();
        self.runner.start(images, self.now, self.now);
    }

    /// Feeds `frame` (or a lost capture) for `duration`, ticking whenever the runner asks to.
    fn feed(&mut self, frame: Option<Bgr>, duration: Duration) {
        let frame = frame.map(solid);
        let end = self.now + duration;
        for _ in 0..100_000 {
            match self.runner.next_wakeup() {
                Some(next) if next <= end => {
                    self.now = self.now.max(next);
                    self.runner.tick(self.now, frame.as_ref().map(BgraImage::view));
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
            self.runner.tick(self.now, Some(empty.view()));
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
        harness.runner.tick(next, Some(solid(BLUE).view()));
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
        harness.runner.tick(next, Some(solid(BLUE).view()));
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
    let mut harness = Harness::with_control(&[("1.png", RED), ("2.png", BLUE)], defaults(), true);
    harness.start();
    harness.feed(Some(BLACK), ms(100));

    // Act
    harness.undo(false);

    // Assert
    assert_eq!((harness.sent(), harness.current_filename()), (vec![], Some("1.png")));
}

#[test]
fn livesplit_skip_moves_on_without_printing_back() {
    // Arrange
    let mut harness = Harness::with_control(&[("1.png", RED), ("2.png", BLUE)], defaults(), true);
    harness.start();
    harness.feed(Some(BLACK), ms(100));

    // Act
    harness.skip(false);

    // Assert
    assert_eq!((harness.sent(), harness.current_filename()), (vec![], Some("2.png")));
}

#[test]
fn livesplit_split_on_the_last_split_ends_the_run() {
    // Arrange
    let mut harness = Harness::with_control(&[("1.png", RED)], defaults(), true);
    harness.start();
    harness.feed(Some(BLACK), ms(100));

    // Act
    harness.skip(false);

    // Assert
    assert_eq!((harness.sent(), harness.runner.is_running()), (vec![], false));
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
        harness.runner.tick(next, Some(solid(BLUE).view()));
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
