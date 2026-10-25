//! Fade images: the similarity to their color, and telling a fade to it from a cut.

use std::path::Path;
use std::time::Duration;

use autosplit_core::fade::{FadeNoise, FadeTracker, NoiseColor, NoiseError, measure_noise};
use autosplit_core::image::BgraImage;
use autosplit_core::split_image::{LoadError, SplitDefaults, SplitImage};
use test_case::test_case;

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

/// Similarities against #a0a0a0 of a strip at the bottom of an analog capture (GV-USB2), one
/// per frame the game draws at 20 FPS, with a fade color tolerance of 15. Something dark crosses
/// the strip as the fade starts (at 10), so the fade's first steps are big.
const STRIP_FADE: [f64; 22] = [
    0.826, 0.827, 0.833, 0.848, 0.857, 0.869, 0.874, 0.885, 0.893, 0.897, 0.769, 0.832, 0.880, 0.909, 0.925, 0.937,
    0.945, 0.954, 0.965, 0.975, 0.988, 0.996,
];

/// Similarities against black of the whole screen, one per frame the game draws at 20 FPS, from
/// an HDMI capture of Ocarina of Time entering the Deku Tree. The camera turns into the tree's
/// black mouth (at 18), so the fade jumps and then barely changes while only Link is left in it,
/// and is fully black at 22.
#[allow(clippy::approx_constant, reason = "measured similarities, one of them near pi / 4")]
const DEKU_FADE: [f64; 23] = [
    0.7850, 0.7854, 0.7797, 0.7801, 0.7914, 0.8048, 0.8170, 0.8337, 0.8620, 0.8877, 0.9199, 0.9356, 0.9283, 0.9400,
    0.9500, 0.9621, 0.9743, 0.9839, 0.9937, 0.9943, 0.9947, 0.9978, 1.0000,
];

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
fn fade_with_big_first_steps_is_found_on_its_last_frame() {
    // Arrange: each game frame shown for 3 capture frames, starting at each of them
    let traces = [0, 1, 2].map(|phase| {
        let mut trace = vec![STRIP_FADE[0]; 30 + phase];
        trace.extend(STRIP_FADE.iter().flat_map(|similarity| [*similarity; 3]));
        (30 + phase, trace)
    });

    // Act
    let found = traces.map(|(start, trace)| first_fade(&trace, 0.995, 1000).map(|frame| (frame - start) / 3));

    // Assert
    assert_eq!(found, [Some(21); 3]);
}

#[test]
fn fade_that_jumps_then_barely_changes_before_the_color_is_found_on_its_last_frame() {
    // Arrange: each game frame shown for 3 capture frames, starting at each of 6, so the samples
    // land on every other game frame both ways
    let traces = [0, 1, 2, 3, 4, 5].map(|phase| {
        let mut trace = vec![DEKU_FADE[0]; 30 + phase];
        trace.extend(DEKU_FADE.iter().flat_map(|similarity| [*similarity; 3]));
        trace.extend([1.0; 30]);
        (30 + phase, trace)
    });

    // Act
    let found = traces.map(|(start, trace)| first_fade(&trace, 0.999, 800).map(|frame| (frame - start) / 3));

    // Assert
    assert_eq!(found, [Some(22); 6]);
}

#[test]
fn cut_after_the_scene_kept_changing_is_not_a_fade() {
    // Arrange: a camera turning toward something dark, slower for the last 3 samples, then a cut
    // to the color that makes 0.926 of their rise
    let mut trace = vec![0.5; 60];
    for step in 1..=12 {
        trace.extend([0.5 + 0.03 * f64::from(step); 3]);
    }
    for similarity in [0.865, 0.870, 0.875] {
        trace.extend([similarity; 6]);
    }
    trace.extend([1.0; 60]);

    // Act
    let found = [0.97, 0.995].map(|threshold| first_fade(&trace, threshold, 800));

    // Assert
    assert_eq!(found, [None; 2]);
}

#[test]
fn cut_blended_over_two_frames_is_not_a_fade() {
    // Arrange
    let mut trace = vec![0.8; 60];
    trace.extend([0.87, 0.93]);
    trace.extend([1.0; 60]);

    // Act
    let found = first_fade(&trace, 0.97, 1000);

    // Assert
    assert_eq!(found, None);
}

#[test_case(&[(0.484, 60), (0.913, 60), (1.0, 90)]; "a cut to a dark room, then a cut to the color")]
#[test_case(&[(0.9, 60), (0.0, 3), (0.9, 30), (1.0, 60)]; "a lost capture, then a cut to the color")]
fn cut_soon_after_another_cut_is_not_a_fade(parts: &[(f64, usize)]) {
    // Arrange: the earlier cut is still in the history when the cut to the color comes
    let trace: Vec<f64> = parts.iter().flat_map(|&(similarity, frames)| vec![similarity; frames]).collect();

    // Act
    let found = [0.97, 0.995].map(|threshold| first_fade(&trace, threshold, 1000));

    // Assert
    assert_eq!(found, [None; 2]);
}

#[test_case(&[(0.3, 60), (0.990, 6), (0.991, 6), (0.993, 6), (0.996, 60)]; "straight to just under it")]
#[test_case(&[(0.3, 60), (0.85, 6), (0.990, 6), (0.991, 6), (0.993, 6), (0.996, 60)]; "through a dark frame")]
#[test_case(&[(0.5, 60), (0.98, 6), (0.984, 6), (0.988, 6), (0.992, 6), (0.996, 60)]; "a longer creep")]
fn cut_that_creeps_up_to_the_threshold_is_not_a_fade(parts: &[(f64, usize)]) {
    // Arrange: a cut to just under the color, then a few samples creeping up to it, like an analog
    // capture's black level settling
    let trace: Vec<f64> = parts.iter().flat_map(|&(similarity, frames)| vec![similarity; frames]).collect();

    // Act
    let found = first_fade(&trace, 0.995, 1000);

    // Assert
    assert_eq!(found, None);
}

#[test]
fn fade_soon_after_a_cut_to_a_dark_room_is_found() {
    // Arrange: a cut to a dark room, and half a second later a fade to the color
    let mut trace = vec![0.3; 60];
    trace.extend(fade_trace(0.91, 1.0, 0.5, 1.0));

    // Act
    let found = first_fade(&trace, 0.99, 1000);

    // Assert
    assert!(found.is_some());
}

#[test]
fn cut_after_a_dark_room_that_flickers_is_not_a_fade() {
    // Arrange: a cut to a dark room whose similarity moves a little, then a cut to the color
    let mut trace = vec![0.3; 30];
    trace.extend((0..60).map(|frame| 0.85 + 0.015 * (frame as f64).sin()));
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
    fade_noise: FadeNoise::NONE,
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

    // Assert: worked out another way than the code does, so equal up to rounding
    let off = 1.0 - (3.0_f64 * 81.0).sqrt() / (255.0 * 3.0_f64.sqrt());
    assert_eq!(similarities[0], 1.0);
    assert!((similarities[1] - off).abs() < 1e-12, "{} isn't {off}", similarities[1]);
}

/// A 4x2 fade image of `color` (BGR), and a capture that alternates between `even` and `odd`
/// (BGR), the same size so it isn't shrunk to one pixel.
fn alternating(color: [u8; 3], even: [u8; 3], odd: [u8; 3]) -> (tempfile::TempDir, SplitImage, BgraImage) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("01_fade_{f}.png");
    image(&path, 4, 2, |_, _| [color[0], color[1], color[2], 255]);
    let pixels = (0..8).flat_map(|i| {
        let [blue, green, red] = if i % 2 == 0 { even } else { odd };
        [blue, green, red, 255]
    });
    (directory, SplitImage::load(&path), BgraImage::from_raw(4, 2, pixels.collect()))
}

fn noisy_similarity(image: &SplitImage, capture: &BgraImage, fade_tolerance: i64, fade_noise: FadeNoise) -> f64 {
    let defaults = SplitDefaults { fade_tolerance, fade_noise, ..DEFAULTS };
    image.similarity(Some(capture.view()), &defaults, &mut Default::default())
}

/// `1 - sqrt(squared) / max`, worked out another way than the code does.
fn from_squared(squared: f64) -> f64 {
    1.0 - squared.sqrt() / (255.0 * 3.0_f64.sqrt())
}

#[test]
fn noise_up_to_the_fade_noise_counts_as_the_color() {
    // Arrange: a noisy capture of the color, every channel 2 levels above or below it
    let (_directory, image, capture) = alternating([160; 3], [158; 3], [162; 3]);

    // Act
    let similarities =
        [[2.0; 3], [0.0; 3]].map(|gray| noisy_similarity(&image, &capture, 0, FadeNoise { gray, ..FadeNoise::NONE }));

    // Assert
    let noisy = from_squared(3.0 * 4.0);
    assert_eq!(similarities[0], 1.0);
    assert!((similarities[1] - noisy).abs() < 1e-12, "{} isn't {noisy}", similarities[1]);
}

#[test]
fn fade_noise_is_taken_out_of_each_channel_by_itself() {
    // Arrange: only red is noisy, 2 levels either side of the color
    let (_directory, image, capture) = alternating([160; 3], [160, 160, 158], [160, 160, 162]);

    // Act: measured on red, then on blue
    let similarities = [[2.0, 0.0, 0.0], [0.0, 0.0, 2.0]]
        .map(|gray| noisy_similarity(&image, &capture, 0, FadeNoise { gray, ..FadeNoise::NONE }));

    // Assert
    let red_left = from_squared(4.0);
    assert_eq!(similarities[0], 1.0);
    assert!((similarities[1] - red_left).abs() < 1e-12, "{} isn't {red_left}", similarities[1]);
}

#[test]
fn fade_noise_only_takes_the_noise_out_of_a_scene() {
    // Arrange: a scene still 30 levels either side of the color, near the end of a fade
    let (_directory, image, capture) = alternating([160; 3], [130; 3], [190; 3]);

    // Act
    let similarity = noisy_similarity(&image, &capture, 0, FadeNoise { gray: [2.0; 3], ..FadeNoise::NONE });

    // Assert: only the 2 levels of noise come off its spread of 30 per channel
    let expected = from_squared(3.0 * (900.0 - 4.0));
    assert!((similarity - expected).abs() < 1e-12, "{similarity} isn't {expected}");
}

#[test]
fn fade_noise_near_black_is_at_most_half_the_mean() {
    // Arrange: what's left of a scene fading to black, 2 levels either side of a mean of 2
    let (_directory, image, capture) = alternating([0; 3], [0; 3], [4; 3]);

    // Act
    let similarity = noisy_similarity(&image, &capture, 15, FadeNoise { black: [2.0; 3], ..FadeNoise::NONE });

    // Assert: 1 level of noise comes off, not 2
    let expected = from_squared(3.0 * (4.0 - 1.0));
    assert!((similarity - expected).abs() < 1e-12, "{similarity} isn't {expected}");
}

#[test]
fn fade_noise_goes_from_blacks_to_grays_by_each_channels_level() {
    // Arrange
    let noise = FadeNoise { black: [1.0, 1.0, 1.0], gray: [3.0, 3.0, 3.0] };

    // Act
    let at = noise.at([80, 0, 255]);

    // Assert: halfway to gray, black, and past gray
    assert_eq!(at, [2.0, 1.0, 3.0]);
}

#[test]
fn fade_noise_of_a_hand_edited_profile_stays_within_a_channel() {
    // Arrange
    let noise = FadeNoise { black: [-1.0, f64::NAN, 300.0], gray: [0.0; 3] };

    // Act
    let at = noise.at([0, 0, 0]);

    // Assert
    assert_eq!(at, [0.0, 0.0, 255.0]);
}

/// A capture of `width` by `height` with the middle that [`measure_noise`] reads alternating
/// between `even` and `odd` (BGRA), and white around it.
fn noisy_middle(width: usize, height: usize, even: [u8; 4], odd: [u8; 4]) -> BgraImage {
    let middle = |x: usize, y: usize| {
        (width * 5 / 16..width * 11 / 16).contains(&x) && (height / 4..height * 3 / 4).contains(&y)
    };
    let data = (0..height)
        .flat_map(|y| (0..width).map(move |x| (x, y)))
        .flat_map(|(x, y)| match middle(x, y) {
            true if (x + y) % 2 == 0 => even,
            true => odd,
            false => [255, 255, 255, 255],
        })
        .collect();
    BgraImage::from_raw(width, height, data)
}

#[test]
fn noise_is_measured_per_channel_in_the_middle() {
    // Arrange: gray with red 2 levels either side of it and blue 1, and white edges
    let capture = noisy_middle(32, 16, [159, 160, 158, 255], [161, 160, 162, 255]);

    // Act
    let noise = measure_noise(capture.view(), NoiseColor::Gray, 15);

    // Assert
    assert_eq!(noise, Ok([2.0, 0.0, 1.0]));
}

#[test]
fn noise_is_measured_in_hundredths() {
    // Arrange: a raised black, like a Wii2HDMI's, one in three pixels a level up
    let capture = BgraImage::from_raw(
        48,
        16,
        (0..48 * 16).flat_map(|i| if i % 3 == 0 { [8, 6, 6, 255] } else { [7, 5, 5, 255] }).collect(),
    );

    // Act
    let noise = measure_noise(capture.view(), NoiseColor::Black, 15);

    // Assert: sqrt(2) / 3
    assert_eq!(noise, Ok([0.47, 0.47, 0.47]));
}

#[test]
fn noise_isnt_measured_on_another_color() {
    // Arrange: black, measured as gray
    let capture = BgraImage::solid([0, 0, 0], 32, 16);

    // Act
    let noise = measure_noise(capture.view(), NoiseColor::Gray, 15);

    // Assert
    assert_eq!(noise, Err(NoiseError::NotTheColor { color: NoiseColor::Gray, found: [0, 0, 0] }));
}

#[test]
fn noise_isnt_measured_on_a_scene() {
    // Arrange: gray with 3 levels either side of it, like the frame before a fade ends, where a
    // twentieth of the scene is left
    let capture = noisy_middle(32, 16, [157, 157, 157, 255], [163, 163, 163, 255]);

    // Act
    let noise = measure_noise(capture.view(), NoiseColor::Gray, 15);

    // Assert
    assert_eq!(noise, Err(NoiseError::NotSolid { color: NoiseColor::Gray, spread: 3.0 }));
}

#[test]
fn noise_isnt_measured_in_a_region_too_small_for_a_middle() {
    // Arrange
    let capture = BgraImage::solid([0, 0, 0], 3, 1);

    // Act
    let noise = measure_noise(capture.view(), NoiseColor::Black, 15);

    // Assert
    assert_eq!(noise, Err(NoiseError::RegionTooSmall));
}

#[test]
fn noise_off_the_color_points_to_the_tolerance() {
    // Arrange: a capture whose black is a little gray
    let error = NoiseError::NotTheColor { color: NoiseColor::Black, found: [20, 20, 20] };

    // Act
    let message = error.to_string();

    // Assert
    assert_eq!(
        message,
        "The middle of the capture region is #141414, further from black than Fade Color Tolerance allows. Show a \
         frame that's all black, like the end of a fade, and measure again. If your capture shows black a little \
         off, raise Fade Color Tolerance."
    );
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
