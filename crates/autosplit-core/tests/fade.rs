//! Fade images: the similarity to their color, and telling a fade to it from a cut.

use std::path::Path;
use std::time::Duration;

use autosplit_core::fade::FadeTracker;
use autosplit_core::image::BgraImage;
use autosplit_core::split_image::{LoadError, SplitDefaults, SplitImage};

/// Similarities against black at 60 FPS, from a capture of Ocarina of Time, 2 seconds each.
///
/// Gameplay, a camera cut that's almost black for 4 frames (at 56), then the fade to black into
/// Gohma's room, which the game draws at 20 FPS. It dips back on the way, at 86 to 91.
const FADE_WITH_SPIKE: [f64; 120] = [
    0.883, 0.883, 0.883, 0.883, 0.883, 0.883, 0.881, 0.881, 0.881, 0.881, 0.881, 0.881, 0.880, 0.880, 0.879, 0.879,
    0.879, 0.879, 0.878, 0.878, 0.877, 0.877, 0.877, 0.877, 0.877, 0.877, 0.879, 0.879, 0.879, 0.879, 0.878, 0.878,
    0.839, 0.839, 0.839, 0.839, 0.849, 0.849, 0.839, 0.839, 0.839, 0.839, 0.830, 0.830, 0.868, 0.868, 0.869, 0.869,
    0.883, 0.883, 0.882, 0.882, 0.882, 0.882, 0.848, 0.848, 0.974, 0.974, 0.974, 0.974, 0.883, 0.883, 0.870, 0.870,
    0.870, 0.870, 0.869, 0.869, 0.879, 0.879, 0.879, 0.879, 0.889, 0.889, 0.899, 0.899, 0.899, 0.899, 0.909, 0.909,
    0.918, 0.918, 0.918, 0.918, 0.925, 0.925, 0.916, 0.916, 0.916, 0.916, 0.901, 0.901, 0.926, 0.926, 0.926, 0.926,
    0.943, 0.943, 0.958, 0.958, 0.958, 0.958, 0.968, 0.968, 0.974, 0.974, 0.974, 0.974, 0.980, 0.980, 0.987, 0.987,
    0.987, 0.987, 0.994, 0.994, 0.998, 0.998, 0.999, 0.999,
];

/// A paused game, then the cut to black of a savewarp (at 103).
const CUT: [f64; 120] = {
    let mut trace = [0.852; 120];
    trace[102] = 0.858;
    let after = [
        1.000, 1.000, 0.997, 0.994, 0.991, 0.988, 0.985, 0.982, 0.979, 0.975, 0.972, 0.970, 0.966, 0.963, 0.960, 0.956,
        0.953,
    ];
    let mut index = 0;
    while index < after.len() {
        trace[103 + index] = after[index];
        index += 1;
    }
    trace
};

/// The first frame of `trace`, played at 60 FPS, that a fade reached `threshold` on.
fn first_fade(trace: &[f64], threshold: f64, fade_time: i64) -> Option<usize> {
    let mut tracker = FadeTracker::default();
    (0..trace.len()).find(|&frame| {
        let now = Duration::from_secs_f64(frame as f64 / 60.0);
        tracker.update(now, trace[frame], threshold, fade_time)
    })
}

/// `seconds` of `from`, then a steady change to `to` over `fade` seconds in steps of 3 frames,
/// like a game drawing at 20 FPS, then `to` for a second.
fn fade_trace(from: f64, to: f64, seconds: f64, fade: f64) -> Vec<f64> {
    let hold = (seconds * 60.0) as usize;
    let steps = ((fade * 20.0) as usize).max(1);
    let mut trace = vec![from; hold];
    for step in 1..=steps {
        trace.extend([from + (to - from) * step as f64 / steps as f64; 3]);
    }
    trace.extend([to; 60]);
    trace
}

#[test]
fn fade_is_found_where_it_reaches_the_threshold_not_at_a_cut_before_it() {
    // Act
    let found = first_fade(&FADE_WITH_SPIKE, 0.97, 1000);

    // Assert
    assert_eq!(found, Some(104));
}

#[test]
fn cut_to_the_color_is_not_a_fade() {
    // Act
    let found = [0.95, 0.97, 0.99].map(|threshold| first_fade(&CUT, threshold, 1000));

    // Assert
    assert_eq!(found, [None; 3]);
}

#[test]
fn steady_fade_is_found() {
    // Arrange
    let trace = fade_trace(0.8, 1.0, 1.0, 1.0);

    // Act
    let found = first_fade(&trace, 0.97, 1000);

    // Assert
    assert!(found.is_some());
}

#[test]
fn deep_fast_fade_is_found() {
    // Arrange: a bright scene fading to black, much further each 100 ms than a dark one
    let trace = fade_trace(0.1, 1.0, 1.0, 0.75);

    // Act
    let found = first_fade(&trace, 0.97, 1000);

    // Assert
    assert!(found.is_some());
}

#[test]
fn cut_blended_over_one_frame_is_not_a_fade() {
    // Arrange
    let mut trace = vec![0.8; 60];
    trace.push(0.9);
    trace.extend([1.0; 60]);

    // Act
    let found = first_fade(&trace, 0.97, 1000);

    // Assert
    assert_eq!(found, None);
}

#[test]
fn cut_after_the_scene_darkened_is_not_a_fade() {
    // Arrange: darkening slowly for a second, then a cut
    let mut trace = fade_trace(0.7, 0.85, 1.0, 1.0);
    trace.extend([1.0; 60]);

    // Act
    let found = first_fade(&trace, 0.97, 1000);

    // Assert
    assert_eq!(found, None);
}

#[test]
fn dark_scene_around_the_threshold_is_not_a_fade() {
    // Arrange
    let trace: Vec<f64> = (0..600).map(|frame| 0.965 + 0.01 * ((frame as f64) * 0.7).sin()).collect();

    // Act
    let found = first_fade(&trace, 0.97, 1000);

    // Assert
    assert_eq!(found, None);
}

#[test]
fn fade_slowed_down_to_twice_the_fade_time_is_found() {
    // Arrange
    let trace = fade_trace(0.88, 1.0, 1.0, 2.0);

    // Act
    let found = first_fade(&trace, 0.99, 1000);

    // Assert
    assert!(found.is_some());
}

#[test]
fn short_fade_time_samples_often_enough_for_a_short_fade() {
    // Arrange
    let trace = fade_trace(0.8, 1.0, 1.0, 0.2);

    // Act
    let found = [200, 1000].map(|fade_time| first_fade(&trace, 0.97, fade_time).is_some());

    // Assert
    assert_eq!(found, [true, false]);
}

#[test]
fn clear_forgets_the_fade_so_far() {
    // Arrange
    let trace = fade_trace(0.8, 1.0, 1.0, 1.0);
    let last = trace.len() - 1;
    let mut tracker = FadeTracker::default();
    let at = |frame: usize| Duration::from_secs_f64(frame as f64 / 60.0);
    for (frame, similarity) in trace[..last - 75].iter().enumerate() {
        tracker.update(at(frame), *similarity, 0.97, 1000);
    }

    // Act
    tracker.clear();
    let found = (last - 75..=last).any(|frame| tracker.update(at(frame), trace[frame], 0.97, 1000));

    // Assert
    assert!(!found);
}

const DEFAULTS: SplitDefaults = SplitDefaults {
    comparison_method: 2,
    similarity_threshold: 0.9,
    delay_time: 0,
    pause_time: 0.0,
    fps_limit: 60,
    fade_tolerance: 10,
};

fn image(path: &Path, width: usize, height: usize, pixel: impl Fn(usize, usize) -> [u8; 4]) {
    let data = (0..width * height).flat_map(|index| pixel(index % width, index / width)).collect();
    std::fs::write(path, BgraImage::from_raw(width, height, data).encode_png().unwrap()).unwrap();
}

fn solid(width: usize, height: usize, [blue, green, red]: [u8; 3]) -> BgraImage {
    BgraImage::from_raw(width, height, [blue, green, red, 255].repeat(width * height))
}

/// A 1x1 fade image of `color` (BGR).
fn fade_image(color: [u8; 3]) -> (tempfile::TempDir, SplitImage) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("01_fade_{f}.png");
    image(&path, 1, 1, |_, _| [color[0], color[1], color[2], 255]);
    let image = SplitImage::load(&path);
    (directory, image)
}

fn similarity(image: &SplitImage, capture: &BgraImage, fade_tolerance: i64) -> f64 {
    let defaults = SplitDefaults { fade_tolerance, ..DEFAULTS };
    image.similarity(Some(capture.view()), &defaults, &mut Default::default())
}

#[test]
fn fade_image_compares_by_color_whatever_the_method() {
    // Arrange
    let (_directory, image) = fade_image([160, 160, 160]);

    // Act
    let similarity = similarity(&image, &solid(640, 480, [160, 160, 160]), 0);

    // Assert
    assert_eq!((image.fade_color(), similarity), (Some([160, 160, 160]), 1.0));
}

#[test]
fn colors_off_by_the_tolerance_count_as_the_color() {
    // Arrange: an analog capture's gray
    let (_directory, image) = fade_image([160, 160, 160]);
    let capture = solid(640, 480, [151, 151, 151]);

    // Act
    let similarities = [10, 0].map(|tolerance| similarity(&image, &capture, tolerance));

    // Assert
    let off = 1.0 - (3.0_f64 * 81.0).sqrt() / (255.0 * 3.0_f64.sqrt());
    assert_eq!(similarities, [1.0, off]);
}

#[test]
fn fade_image_only_compares_its_visible_pixels() {
    // Arrange: black on the left, transparent on the right, and a capture white on the right
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("01_fade_{f}.png");
    image(&path, 4, 2, |x, _| if x < 2 { [0, 0, 0, 255] } else { [0, 0, 0, 0] });
    let image = SplitImage::load(&path);
    let capture = BgraImage::from_raw(
        4,
        2,
        (0..8).flat_map(|i| if i % 4 < 2 { [0, 0, 0, 255] } else { [255, 255, 255, 255] }).collect(),
    );

    // Act
    let similarity = similarity(&image, &capture, 0);

    // Assert
    assert_eq!(similarity, 1.0);
}

#[test]
fn fade_image_that_is_not_one_color_cannot_be_used() {
    // Arrange
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("01_fade_{f}.png");
    image(&path, 2, 1, |x, _| [x as u8, 0, 0, 255]);

    // Act
    let image = SplitImage::load(&path);

    // Assert
    assert_eq!(image.load_error(), Some(&LoadError::FadeNotSolid(path)));
}

#[test]
fn fade_time_defaults_to_a_second() {
    // Arrange
    let (_directory, image) = fade_image([0, 0, 0]);

    // Act
    let fade_time = image.fade_time_ms();

    // Assert
    assert_eq!(fade_time, 1000);
}
