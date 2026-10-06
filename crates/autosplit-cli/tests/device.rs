//! What the log says about an open capture device.

use std::time::{Duration, Instant};

use autosplit::device::{Notice, Observed, Watch, region_warning};
use autosplit_core::profile::Region;
use test_case::test_case;

const REGION: Region = Region { x: 0, y: 0, width: 640, height: 480 };

fn observed(receiving: bool) -> Observed {
    Observed { failed: false, receiving, placeholder_seen: false, frame_size: receiving.then_some((1920, 1080)) }
}

#[test]
fn first_frames_log_their_size() {
    // Arrange
    let opened = Instant::now();
    let mut watch = Watch::new("Elgato", opened);

    // Act
    let notices = watch.observe(observed(true), opened, &REGION);

    // Assert
    assert_eq!(notices, [Notice::Log("Receiving 1920x1080 frames from 'Elgato'".into())]);
}

#[test]
fn frames_stopping_warn() {
    // Arrange
    let opened = Instant::now();
    let mut watch = Watch::new("Elgato", opened);
    watch.observe(observed(true), opened, &REGION);

    // Act
    let notices = watch.observe(observed(false), opened, &REGION);

    // Assert
    assert_eq!(
        notices,
        [Notice::Warn(
            "No frames from 'Elgato' for 1 second. Is it in use by another program, or switched off?".into()
        )]
    );
}

#[test]
fn frames_coming_back_at_the_same_size_say_again() {
    // Arrange
    let opened = Instant::now();
    let mut watch = Watch::new("Elgato", opened);
    watch.observe(observed(true), opened, &REGION);
    watch.observe(observed(false), opened, &REGION);

    // Act
    let notices = watch.observe(observed(true), opened, &REGION);

    // Assert
    assert_eq!(notices, [Notice::Log("Receiving frames from 'Elgato' again".into())]);
}

#[test_case(Duration::from_millis(500) => 0; "before the wait")]
#[test_case(Duration::from_secs(3) => 1; "after the wait")]
fn missing_first_frame_warns_after_a_while(after: Duration) -> usize {
    // Arrange
    let opened = Instant::now();
    let mut watch = Watch::new("Elgato", opened);

    // Act
    let notices = watch.observe(observed(false), opened + after, &REGION);

    // Assert
    notices.len()
}

#[test]
fn obs_placeholder_says_to_start_the_virtual_camera() {
    // Arrange
    let opened = Instant::now();
    let mut watch = Watch::new("OBS Virtual Camera", opened);
    let placeholder = Observed { placeholder_seen: true, ..observed(false) };

    // Act
    let notices = watch.observe(placeholder, opened + Duration::from_secs(3), &REGION);

    // Assert
    assert_eq!(
        notices,
        [Notice::Warn("'OBS Virtual Camera' is connected but not started. In OBS, click Start Virtual Camera.".into())]
    );
}

#[test]
fn failed_device_is_lost() {
    // Arrange
    let opened = Instant::now();
    let mut watch = Watch::new("Elgato", opened);

    // Act
    let notices = watch.observe(Observed { failed: true, ..observed(true) }, opened, &REGION);

    // Assert
    assert!(matches!(notices.as_slice(), [Notice::Lost(_)]));
}

#[test]
fn first_frames_warn_about_a_region_that_doesnt_fit() {
    // Arrange
    let opened = Instant::now();
    let mut watch = Watch::new("Elgato", opened);
    let region = Region { x: 1800, y: 0, width: 640, height: 480 };

    // Act
    let notices = watch.observe(observed(true), opened, &region);

    // Assert
    assert_eq!(notices.len(), 2);
}

#[test_case(Region { x: 0, y: 0, width: 640, height: 480 } => None; "inside")]
#[test_case(Region { x: 0, y: 0, width: 0, height: 0 } => Some("The profile has no capture region (its width or height is 0), so nothing can match. Set [capture_region] in the profile.".to_owned()); "empty")]
#[test_case(Region { x: 1800, y: 0, width: 640, height: 480 } => Some("The capture region (x 1800, y 0, 640x480) doesn't fit in the 1920x1080 frames, so only part of it is compared. Fix [capture_region] in the profile.".to_owned()); "partly outside")]
fn region_warnings(region: Region) -> Option<String> {
    // Act, with the expected value checked by test_case
    region_warning(&region, (1920, 1080))
}
