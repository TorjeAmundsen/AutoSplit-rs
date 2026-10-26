//! The Live Tester: which frames it keeps, decided like the runner decides when to split.

use std::path::PathBuf;
use std::time::Duration;

use autosplit_core::image::BgraImage;
use autosplit_core::runner::Captured;
use autosplit_core::split_image::{LoadError, SplitDefaults};
use autosplit_core::tester::{Tester, TriggerKind};
use autosplit_core::text::{tester_info, tester_status, trigger_caption};
use tempfile::TempDir;

type Bgr = [u8; 3];
const RED: Bgr = [0, 0, 255];
const BLUE: Bgr = [255, 0, 0];

const DEFAULTS: SplitDefaults = SplitDefaults {
    comparison_method: 0,
    similarity_threshold: 0.95,
    delay_time: 0,
    pause_time: 0.0,
    fps_limit: 60,
    fade_tolerance: 10,
};

fn solid(color: Bgr) -> BgraImage {
    let pixel = [color[0], color[1], color[2], 255];
    BgraImage::from_raw(320, 240, pixel.repeat(320 * 240))
}

/// Feeds frames to a tester of one red split image, ticking whenever it asks to.
struct Harness {
    _directory: TempDir,
    tester: Tester,
    now: Duration,
    next_sequence: u64,
}

impl Harness {
    fn new(name: &str) -> Self {
        Self::with_defaults(name, DEFAULTS)
    }

    fn with_defaults(name: &str, defaults: SplitDefaults) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(name);
        std::fs::write(&path, solid(RED).encode_png().unwrap()).unwrap();
        let tester = Tester::open(&path, defaults, Duration::ZERO).unwrap();
        Self { _directory: directory, tester, now: Duration::ZERO, next_sequence: 0 }
    }

    /// Feeds `frame` (or no capture) for `duration`, a new frame every tick.
    fn feed(&mut self, frame: Option<Bgr>, duration: Duration) {
        let frame = frame.map(solid);
        let end = self.now + duration;
        while self.tester.next_wakeup() <= end {
            self.now = self.now.max(self.tester.next_wakeup());
            self.next_sequence += 1;
            let sequence = self.next_sequence;
            self.tester.tick(self.now, frame.as_ref().map(|frame| Captured { view: frame.view(), sequence }));
        }
        self.now = end;
    }

    /// The kept frames' kinds, and whether each is red. A frame without a capture isn't.
    fn kept(&self) -> Vec<(TriggerKind, bool)> {
        let red = |frame: &BgraImage| frame.data()[..3] == RED;
        self.tester.triggers().iter().map(|trigger| (trigger.kind, trigger.frame.as_ref().is_some_and(red))).collect()
    }

    /// The information cells as `NAME: value (note)`.
    fn info(&self) -> Vec<String> {
        let text = |cell: &autosplit_core::text::InfoCell| match cell.note.as_str() {
            "" => format!("{}: {}", cell.name, cell.value),
            note => format!("{}: {} ({note})", cell.name, cell.value),
        };
        tester_info(&self.tester).iter().map(text).collect()
    }

    fn captions(&self) -> Vec<String> {
        self.tester.triggers().iter().map(|trigger| trigger_caption(&self.tester, trigger)).collect()
    }
}

const SECOND: Duration = Duration::from_secs(1);

fn ms(milliseconds: u64) -> Duration {
    Duration::from_millis(milliseconds)
}

#[test]
fn keeps_the_frame_that_splits() {
    // Arrange
    let mut harness = Harness::new("1_split.png");

    // Act
    harness.feed(Some(BLUE), SECOND);
    let highest_before = harness.tester.highest().is_some();
    harness.feed(Some(RED), SECOND);

    // Assert: what didn't match still shows as the highest
    assert_eq!((highest_before, harness.kept()), (true, vec![(TriggerKind::Acted { delay: 0 }, true)]));
}

#[test]
fn delay_keeps_the_frame_after_the_delay_too() {
    // Arrange
    let mut harness = Harness::new("1_split_#500#.png");

    // Act
    harness.feed(Some(RED), ms(100));
    harness.feed(Some(BLUE), SECOND);

    // Assert
    assert_eq!(harness.kept(), [(TriggerKind::Reached, true), (TriggerKind::Acted { delay: 500 }, false)]);
}

#[test]
fn profile_delay_applies_without_a_filename_delay() {
    // Arrange
    let mut harness = Harness::with_defaults("1_split.png", SplitDefaults { delay_time: 200, ..DEFAULTS });

    // Act
    harness.feed(Some(RED), SECOND);

    // Assert
    assert_eq!(harness.kept(), [(TriggerKind::Reached, true), (TriggerKind::Acted { delay: 200 }, true)]);
}

#[test]
fn below_flag_splits_once_the_similarity_drops_back() {
    // Arrange
    let mut harness = Harness::new("1_split_{b}.png");

    // Act
    harness.feed(Some(RED), SECOND);
    let while_above = harness.kept();
    harness.feed(Some(BLUE), SECOND);

    // Assert
    assert_eq!(
        (while_above, harness.kept()),
        (
            vec![(TriggerKind::Reached, true)],
            vec![(TriggerKind::Reached, true), (TriggerKind::Acted { delay: 0 }, false)]
        )
    );
}

#[test]
fn below_flag_with_a_delay_keeps_the_drop_and_the_split() {
    // Arrange
    let mut harness = Harness::new("1_split_#300#_{b}.png");

    // Act
    harness.feed(Some(RED), SECOND);
    harness.feed(Some(BLUE), ms(100));
    harness.feed(Some(RED), SECOND);

    // Assert
    assert_eq!(
        harness.kept(),
        [(TriggerKind::Reached, true), (TriggerKind::DroppedBelow, false), (TriggerKind::Acted { delay: 300 }, true)]
    );
}

#[test]
fn below_flag_does_not_split_on_a_lost_capture() {
    // Arrange
    let mut harness = Harness::new("1_split_{b}.png");

    // Act
    harness.feed(Some(RED), SECOND);
    harness.feed(None, SECOND);

    // Assert
    assert_eq!((harness.kept(), harness.tester.similarity()), (vec![(TriggerKind::Reached, true)], None));
}

#[test]
fn reset_image_ignores_below_and_delay() {
    // Arrange
    let mut harness = Harness::new("reset_#500#_{b}.png");

    // Act
    harness.feed(Some(RED), SECOND);

    // Assert
    assert_eq!(harness.kept(), [(TriggerKind::Acted { delay: 0 }, true)]);
}

#[test]
fn keeps_nothing_more_until_rearmed() {
    // Arrange
    let mut harness = Harness::new("1_split.png");
    harness.feed(Some(RED), SECOND);
    harness.feed(Some(BLUE), SECOND);
    let before = (harness.kept().len(), harness.tester.highest().is_some(), harness.tester.generation());

    // Act
    harness.tester.rearm();
    let cleared = (harness.kept().len(), harness.tester.highest());
    harness.feed(Some(RED), SECOND);

    // Assert
    assert_eq!((before, cleared, harness.kept().len(), harness.tester.generation()), ((1, true, 0), (0, None), 1, 1));
}

#[test]
fn threshold_comes_from_the_filename_or_the_profile() {
    // Arrange
    let from_filename = Harness::new("1_split_(0.8).png");
    let mut from_profile = Harness::new("1_split.png");

    // Act
    from_profile.tester.set_defaults(SplitDefaults { similarity_threshold: 0.7, ..DEFAULTS });

    // Assert
    assert_eq!((from_filename.tester.threshold(), from_profile.tester.threshold()), (0.8, 0.7));
}

#[test]
fn a_file_that_is_not_a_png_does_not_open() {
    // Arrange
    let directory = tempfile::tempdir().unwrap();
    let path: PathBuf = directory.path().join("1_split.png");
    std::fs::write(&path, "not an image").unwrap();

    // Act
    let opened = Tester::open(&path, DEFAULTS, Duration::ZERO);

    // Assert
    assert_eq!(opened.err(), Some(LoadError::NotPng(path)));
}

#[test]
fn captions_say_what_each_kept_frame_is() {
    // Arrange
    let mut harness = Harness::new("1_split_#300#_{b}.png");

    // Act
    harness.feed(Some(RED), SECOND);
    harness.feed(Some(BLUE), SECOND);

    // Assert
    assert_eq!(
        (harness.captions(), tester_status(&harness.tester)),
        (
            vec!["Reached: 1.000".to_owned(), "Dropped below: 0.292".to_owned(), "Split +300 ms: 0.292".to_owned()],
            "Done: this is where it splits. Press Start Over to watch for the next match.".to_owned()
        )
    );
}

#[test]
fn info_says_which_values_come_from_the_profile() {
    // Arrange
    let harness = Harness::new("1_split_(0.8)_#300#_{b}.png");

    // Act
    let info = harness.info();

    // Assert
    assert_eq!(
        info,
        [
            "ACTS: splits once it drops back below",
            "METHOD: L2 Norm (from profile)",
            "THRESHOLD: 0.800",
            "DELAY: 300 ms",
        ]
    );
}

#[test]
fn dummy_acts_right_away_without_its_delay() {
    // Arrange
    let mut harness = Harness::new("1_split_#2000#_{d}.png");

    // Act
    harness.feed(Some(RED), ms(100));

    // Assert
    assert_eq!(
        (harness.kept(), harness.info()[0].clone(), harness.info()[3].clone()),
        (
            vec![(TriggerKind::Acted { delay: 0 }, true)],
            "ACTS: moves to the next image without splitting (dummy)".to_owned(),
            "DELAY: none (dummies don't wait)".to_owned()
        )
    );
}

#[test]
fn delay_ends_without_a_capture() {
    // Arrange
    let mut harness = Harness::new("1_split_#500#.png");

    // Act
    harness.feed(Some(RED), ms(100));
    harness.feed(None, SECOND);

    // Assert
    assert_eq!(harness.kept(), [(TriggerKind::Reached, true), (TriggerKind::Acted { delay: 500 }, false)]);
}

#[test]
fn zero_threshold_matches_without_a_capture() {
    // Arrange
    let mut harness = Harness::new("1_split_(0).png");

    // Act
    harness.feed(None, SECOND);

    // Assert
    assert_eq!(harness.kept(), [(TriggerKind::Acted { delay: 0 }, false)]);
}

#[test]
fn delay_keeps_its_length_when_the_profile_changes() {
    // Arrange
    let mut harness = Harness::with_defaults("1_split.png", SplitDefaults { delay_time: 500, ..DEFAULTS });
    harness.feed(Some(RED), ms(100));

    // Act
    harness.tester.set_defaults(SplitDefaults { delay_time: 2000, ..DEFAULTS });
    harness.feed(Some(RED), SECOND);

    // Assert
    assert_eq!(harness.captions(), ["Reached: 1.000", "Split +500 ms: 1.000"]);
}

#[test]
fn fade_image_only_acts_on_a_fade() {
    // Arrange: a red fade image
    let mut harness = Harness::new("1_{f}.png");
    harness.feed(Some(BLUE), SECOND);
    harness.feed(Some(RED), SECOND);
    let after_cut = harness.kept();
    harness.feed(Some(BLUE), SECOND);

    // Act: from blue to red, a new color every 50 ms
    for step in 1..=20_u8 {
        let red = (255 * u32::from(step) / 20) as u8;
        harness.feed(Some([255 - red, 0, red]), Duration::from_millis(50));
    }

    // Assert: on a frame close to red, which reaches the threshold before the fade ends
    let kinds: Vec<TriggerKind> = harness.kept().into_iter().map(|(kind, _)| kind).collect();
    assert_eq!((after_cut, kinds), (vec![], vec![TriggerKind::Acted { delay: 0 }]));
}

#[test]
fn info_shows_a_fade_image_s_color_and_fade_time() {
    // Arrange: a red fade image
    let harness = Harness::new("1_split_(0.9)_~600~_{f}.png");

    // Act
    let info = harness.info();

    // Assert
    assert_eq!(info[1], "METHOD: fade to #ff0000 (about 600 ms, tolerance 10 levels)");
}

#[test]
fn status_says_why_a_cut_to_a_fade_image_s_color_does_nothing() {
    // Arrange: a red fade image
    let mut harness = Harness::new("1_{bf}.png");
    harness.feed(Some(BLUE), SECOND);

    // Act
    harness.feed(Some(RED), SECOND);
    let after_cut = tester_status(&harness.tester);
    harness.feed(Some(BLUE), SECOND);
    for step in 1..=20_u8 {
        let red = (255 * u32::from(step) / 20) as u8;
        harness.feed(Some([255 - red, 0, red]), Duration::from_millis(50));
    }
    let after_fade = tester_status(&harness.tester);

    // Assert
    assert_eq!(
        (after_cut.as_str(), after_fade.as_str()),
        (
            "At the threshold, but it didn't fade there, like after a cut. Waiting for a fade to the color.",
            "Faded to the color. Waiting for it to drop back below."
        )
    );
}

#[test]
fn info_shows_the_fade_tolerance_comparisons_use() {
    // Arrange: a hand-edited profile's tolerance, past the 255 levels a channel has
    let harness = Harness::with_defaults("1_split_~600~_{f}.png", SplitDefaults { fade_tolerance: 300, ..DEFAULTS });

    // Act
    let info = harness.info();

    // Assert
    assert_eq!(info[1], "METHOD: fade to #ff0000 (about 600 ms, tolerance 255 levels)");
}
