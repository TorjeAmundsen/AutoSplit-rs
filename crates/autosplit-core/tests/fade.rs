//! Fade images: the similarity to their color, and telling a fade to it from a cut.

use std::path::Path;
use std::time::Duration;

use autosplit_core::fade::FadeTracker;
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
    // Arrange: a steady fade, cut off partway
    let trace = fade_trace(0.8, 1.0, 1.0, 1.0);
    let last = trace.len() - 1;
    let mut tracker = FadeTracker::default();
    let at = |frame: usize| Duration::from_secs_f64(frame as f64 / 60.0);
    for (frame, similarity) in trace[..last - 75].iter().enumerate() {
        tracker.update(at(frame), *similarity, 0.97, 1000);
    }
    let mut cleared = tracker.clone();

    // Act
    cleared.clear();
    let rest =
        |tracker: &mut FadeTracker| (last - 75..=last).any(|frame| tracker.update(at(frame), trace[frame], 0.97, 1000));
    let found = (rest(&mut tracker), rest(&mut cleared));

    // Assert
    assert_eq!(found, (true, false));
}

const DEFAULTS: SplitDefaults = SplitDefaults {
    comparison_method: 2,
    similarity_threshold: 0.9,
    delay_time: 0,
    pause_time: 0.0,
    fps_limit: 60,
    fade_tolerance: 10,
    fade_ignore_noise: true,
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

/// A 320x240 capture, the size opaque fade images are compared at, with `pixel` (BGR) at each x
/// and y.
fn capture(pixel: impl Fn(usize, usize) -> [u8; 3]) -> BgraImage {
    let data = (0..320 * 240)
        .flat_map(|index| {
            let [blue, green, red] = pixel(index % 320, index / 320);
            [blue, green, red, 255]
        })
        .collect();
    BgraImage::from_raw(320, 240, data)
}

/// `even` in every other 2x2 block and `odd` in the rest, like a checkerboard, so they split
/// evenly. Pixels 2 apart, which noise is estimated from at 240 lines, always differ.
fn blocks(x: usize, y: usize, even: u8, odd: u8) -> [u8; 3] {
    [if (x / 2 + y / 2).is_multiple_of(2) { even } else { odd }; 3]
}

/// `1 - sqrt(squared) / max`, worked out another way than the code does.
fn from_squared(squared: f64) -> f64 {
    1.0 - squared.sqrt() / (255.0 * 3.0_f64.sqrt())
}

/// The similarity to `color` of a gray capture with these `values`, with nothing taken out as
/// noise.
fn without_noise(values: &[f64], color: f64) -> f64 {
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let variance = values.iter().map(|value| (value - mean).powi(2)).sum::<f64>() / values.len() as f64;
    from_squared(3.0 * ((mean - color).powi(2) + variance))
}

#[test]
fn noise_spread_over_neighboring_pixels_counts_as_the_color() {
    // Arrange: every channel 1 level either side of the color, in 3x3 blocks, like an analog
    // capture scaled up spreads each speck of noise. Pixels next to each other mostly share it.
    let (_directory, image) = fade_image([160, 160, 160]);
    let capture = capture(|x, y| [if (x / 3 + y / 3).is_multiple_of(2) { 159 } else { 161 }; 3]);

    // Act: 1 level of tolerance for the mean, which the blocks don't split evenly
    let similarity = similarity(&image, &capture, 1);

    // Assert: without taking the noise out it would be about 0.996, and comparing neighboring
    // pixels would only find part of it
    assert_eq!(similarity, 1.0);
}

#[test]
fn noise_counts_against_the_color_when_it_isnt_left_out() {
    // Arrange: every channel 1 level either side of the color
    let (_directory, image) = fade_image([160, 160, 160]);
    let capture = capture(|x, y| blocks(x, y, 159, 161));

    // Act: left out, then not
    let similarities = [true, false].map(|fade_ignore_noise| {
        let defaults = SplitDefaults { fade_tolerance: 0, fade_ignore_noise, ..DEFAULTS };
        image.similarity(Some(capture.view()), &defaults, &mut Default::default())
    });

    // Assert
    let noisy = from_squared(3.0);
    assert_eq!(similarities[0], 1.0);
    assert!((similarities[1] - noisy).abs() < 1e-12, "{} isn't {noisy}", similarities[1]);
}

#[test]
fn spread_that_changes_slowly_across_the_picture_isnt_noise() {
    // Arrange: what's left of a scene at the end of a fade, 3 levels either side of the color
    // from left to right, about as much spread as the noise in the test above
    let (_directory, image) = fade_image([160, 160, 160]);
    let capture = capture(|x, _| [(157 + x * 7 / 320) as u8; 3]);

    // Act
    let similarity = similarity(&image, &capture, 0);

    // Assert: about the same as with nothing taken out, about 0.992, where it would reach 1 if
    // its spread counted as noise
    let values: Vec<f64> = (0..320).map(|x| f64::from((157 + x * 7 / 320) as u8)).collect();
    let expected = without_noise(&values, 160.0);
    assert!((similarity - expected).abs() < 0.0005, "{similarity} isn't about {expected}");
}

#[test]
fn fine_detail_counts_as_at_most_2_levels_of_noise() {
    // Arrange: 10 levels either side of the color, like fine detail left of a scene
    let (_directory, image) = fade_image([160, 160, 160]);
    let capture = capture(|x, y| blocks(x, y, 150, 170));

    // Act
    let similarity = similarity(&image, &capture, 0);

    // Assert: only 2 levels of its spread of 10 per channel come off
    let expected = from_squared(3.0 * (100.0 - 4.0));
    assert!((similarity - expected).abs() < 1e-12, "{similarity} isn't {expected}");
}

#[test]
fn noise_near_black_is_at_most_half_the_mean() {
    // Arrange: what's left of a scene fading to black, 2 levels either side of a mean of 2
    let (_directory, image) = fade_image([0, 0, 0]);
    let capture = capture(|x, y| blocks(x, y, 0, 4));

    // Act
    let similarity = similarity(&image, &capture, 15);

    // Assert: a capture clips noise at black, so 1 level of noise comes off, not 2
    let expected = from_squared(3.0 * (4.0 - 1.0));
    assert!((similarity - expected).abs() < 1e-12, "{similarity} isn't {expected}");
}

#[test]
fn noise_is_only_estimated_from_visible_pixels() {
    // Arrange: a fade image showing only its top left, with the capture white around it. In it,
    // the capture changes slowly from top to bottom, like what's left of a scene. It ends on a
    // row whose pixels a few rows down are hidden.
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("01_fade_{f}.png");
    let shown = |x: usize, y: usize| x < 160 && y < 118;
    image(&path, 320, 240, |x, y| if shown(x, y) { [160, 160, 160, 255] } else { [0, 0, 0, 0] });
    let image = SplitImage::load(&path);
    let capture = capture(|x, y| if shown(x, y) { [(157 + y * 7 / 118) as u8; 3] } else { [255; 3] });

    // Act
    let similarity = similarity(&image, &capture, 0);

    // Assert: about the same as with nothing taken out, where the white next to and below what
    // shows would have counted as noise
    let values: Vec<f64> = (0..118).map(|y| f64::from((157 + y * 7 / 118) as u8)).collect();
    let expected = without_noise(&values, 160.0);
    assert!((similarity - expected).abs() < 0.0005, "{similarity} isn't about {expected}");
}

/// A 320x240 fade image of gray that only shows the rows `shown` picks, across the whole width.
fn gray_rows(shown: impl Fn(usize) -> bool) -> (tempfile::TempDir, SplitImage) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("01_fade_{f}.png");
    image(&path, 320, 240, |_, y| if shown(y) { [160, 160, 160, 255] } else { [0, 0, 0, 0] });
    let image = SplitImage::load(&path);
    (directory, image)
}

fn noise_left_out(image: &SplitImage, capture: &BgraImage) -> [f64; 3] {
    let defaults = SplitDefaults { fade_tolerance: 15, ..DEFAULTS };
    image.fade_comparison(Some(capture.view()), &defaults, &mut Default::default()).unwrap().noise
}

#[test]
fn noise_isnt_estimated_across_the_end_of_a_row() {
    // Arrange: rows showing across the whole width, which the mask keeps as one run of pixels
    // going on from one row into the next. The capture goes slowly from 130 on the left to 190
    // on the right, so pixels at the end of a row and the start of the next are 60 apart.
    let (_directory, image) = gray_rows(|y| y < 118);
    let capture = capture(|x, _| [(130 + x * 60 / 320) as u8; 3]);

    // Act
    let noise = noise_left_out(&image, &capture);

    // Assert: pixels 2 apart within a row are at most a level apart. Pairs going on into the
    // next row would have made it the most there is, 2.
    assert!(noise.iter().all(|&noise| noise < 0.5), "{noise:?}");
}

#[test]
fn noise_is_estimated_from_every_row_when_the_sampled_ones_dont_show() {
    // Arrange: a strip only rows 1 to 3 show, none of them a row noise is usually estimated from,
    // with every channel 1 level either side of the color
    let (_directory, image) = gray_rows(|y| (1..=3).contains(&y));
    let capture = capture(|x, y| blocks(x, y, 159, 161));

    // Act
    let noise = noise_left_out(&image, &capture);

    // Assert: pixels 2 apart always differ by 2 levels, which is the square root of 2 of noise
    let expected = 2.0_f64.sqrt();
    assert!(noise.iter().all(|noise| (noise - expected).abs() < 1e-12), "{noise:?}");
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
