//! Recording screenshots: which frames are saved, where, and when it stops by itself.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use autosplit::recording::{MAX_LENGTH, Recording, Summary};
use autosplit_core::image::BgraImage;
use test_case::test_case;

fn frame() -> Option<BgraImage> {
    Some(BgraImage::solid([10, 20, 30], 4, 3))
}

/// The files in `folder`, sorted.
fn files(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = (std::fs::read_dir(folder).unwrap().flatten())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn each_recording_gets_a_folder_of_its_own() {
    // Arrange: a recording from before
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join("recording_001")).unwrap();

    // Act
    let recording = Recording::start(directory.path(), 60, Instant::now(), |_| {}).unwrap();

    // Assert
    assert_eq!(recording.folder(), directory.path().join("recording_002"));
    assert!(recording.folder().is_dir());
}

#[test]
fn a_frame_due_before_the_capture_sends_one_is_looked_for_again_shortly() {
    // Arrange: 10 fps, the first frame saved, and the next one due at 100 ms
    let directory = tempfile::tempdir().unwrap();
    let start = Instant::now();
    let mut recording = Recording::start(directory.path(), 10, start, |_| {}).unwrap();
    recording.offer(start, 1, frame);
    let due = start + Duration::from_millis(100);

    // Act: due, but still the same frame
    recording.offer(due, 1, frame);

    // Assert: later than now, so the session doesn't keep waking up to look
    assert!(recording.next_wakeup() > due);
}

#[test]
fn a_frame_rate_a_profile_file_sets_too_high_is_the_highest_settings_allows() {
    // Arrange: far past 240, where a frame would last no time at all
    let directory = tempfile::tempdir().unwrap();
    let start = Instant::now();
    let mut recording = Recording::start(directory.path(), i64::MAX, start, |_| {}).unwrap();

    // Act
    recording.offer(start, 1, frame);

    // Assert: the next frame is due a 240th of a second later
    assert_eq!(recording.next_wakeup(), start + Duration::from_secs(1) / 240);
}

#[test]
fn a_new_frame_is_saved_each_time_one_is_due() {
    // Arrange: 10 fps, so a frame every 100 ms
    let directory = tempfile::tempdir().unwrap();
    let start = Instant::now();
    let (done, summary) = mpsc::channel();
    let mut recording =
        Recording::start(directory.path(), 10, start, move |result| done.send(result).unwrap()).unwrap();
    let folder = recording.folder().to_owned();
    let at = |ms: u64| start + Duration::from_millis(ms);

    // Act
    recording.offer(at(0), 1, frame);
    // Not due yet
    recording.offer(at(10), 2, frame);
    recording.offer(at(100), 2, frame);
    // Due, but the same frame as the last one saved
    recording.offer(at(200), 2, frame);
    recording.offer(at(300), 3, frame);
    recording.stop();

    // Assert: once every frame is written
    let summary = summary.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(summary, Summary { folder: folder.clone(), written: 3, dropped: 0, error: None });
    assert_eq!(files(&folder), ["0001.png", "0002.png", "0003.png"]);
    assert_eq!(BgraImage::open_png(&folder.join("0002.png")).unwrap().width(), 4);
}

#[test_case(1000, 11 => 9; "late with the frames sent meanwhile")]
#[test_case(1000, 2 => 0; "late with a capture slower than the recording")]
#[test_case(100, 3 => 0; "on time with a capture faster than the recording")]
fn frames_due_while_the_loop_was_late_are_counted_as_skipped(next_at_ms: u64, next_sequence: u64) -> usize {
    // Arrange: 10 fps, and the first frame saved
    let directory = tempfile::tempdir().unwrap();
    let start = Instant::now();
    let (done, summary) = mpsc::channel();
    let mut recording =
        Recording::start(directory.path(), 10, start, move |result| done.send(result).unwrap()).unwrap();
    recording.offer(start, 1, frame);

    // Act
    recording.offer(start + Duration::from_millis(next_at_ms), next_sequence, frame);
    recording.stop();

    // Assert: both frames written, and the skipped ones checked by test_case
    let summary = summary.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(summary.written, 2);
    summary.dropped
}

#[test]
fn frames_are_saved_without_an_alpha_channel() {
    // Arrange
    let directory = tempfile::tempdir().unwrap();
    let start = Instant::now();
    let (done, summary) = mpsc::channel();
    let mut recording =
        Recording::start(directory.path(), 10, start, move |result| done.send(result).unwrap()).unwrap();
    let path = recording.folder().join("0001.png");

    // Act
    recording.offer(start, 1, frame);
    recording.stop();

    // Assert: the colors kept, with no alpha channel in the file
    summary.recv_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(image::open(&path).unwrap().color(), image::ColorType::Rgb8);
    assert_eq!(BgraImage::open_png(&path).unwrap(), BgraImage::solid([10, 20, 30], 4, 3));
}

#[test]
fn a_recording_stops_by_itself_after_the_longest_it_runs() {
    // Arrange
    let directory = tempfile::tempdir().unwrap();
    let start = Instant::now();
    let recording = Recording::start(directory.path(), 60, start, |_| {}).unwrap();

    // Act
    let (before, at) =
        (recording.expired(start + MAX_LENGTH - Duration::from_millis(1)), recording.expired(start + MAX_LENGTH));

    // Assert
    assert_eq!((before, at), (false, true));
}

#[test_case(600, 0, None => "Recorded 600 screenshots into recording_001."; "every frame")]
#[test_case(1, 0, None => "Recorded 1 screenshot into recording_001."; "one frame")]
#[test_case(590, 10, None => "Recorded 590 screenshots into recording_001. 10 frames were skipped, since AutoSplit fell behind."; "fell behind")]
#[test_case(599, 1, None => "Recorded 599 screenshots into recording_001. 1 frame was skipped, since AutoSplit fell behind."; "one frame skipped")]
#[test_case(598, 0, Some("disk full") => "Recorded 598 screenshots into recording_001. Some couldn't be written: disk full"; "frames that couldn't be written")]
#[test_case(0, 0, Some("disk full") => "Couldn't write the screenshots into recording_001: disk full"; "none could be written")]
fn the_summary_says_what_was_recorded(written: usize, dropped: usize, error: Option<&str>) -> String {
    // Arrange
    let summary = Summary {
        folder: PathBuf::from("shots").join("recording_001"),
        written,
        dropped,
        error: error.map(Into::into),
    };

    // Act, with the expected value checked by test_case
    summary.message()
}
