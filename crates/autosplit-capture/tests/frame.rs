//! Handing frames from a capture thread to the caller.

use std::time::{Duration, Instant};

use autosplit_capture::convert::{Layout, PixelFormat, Yuv420};
use autosplit_capture::{FRAME_TIMEOUT, FrameSlot};
use autosplit_core::profile::Region;

const GRAY: Layout = Layout { format: PixelFormat::Gray, width: 2, height: 2, stride: 2 };
const NV12: Layout = Layout { format: PixelFormat::Yuv420(Yuv420::Nv12), width: 2, height: 2, stride: 2 };

/// OBS Virtual Camera's frames while it isn't started.
const PLACEHOLDER: [u8; 6] = [127; 6];

fn first_pixel(slot: &FrameSlot) -> u8 {
    slot.latest(Instant::now()).unwrap().full_view().unwrap().row(0)[0]
}

#[test]
fn received_frame_is_converted() {
    // Arrange
    let slot = FrameSlot::default();

    // Act
    slot.receive(&[9, 9, 9, 9], GRAY);

    // Assert
    let frame = slot.latest(Instant::now()).unwrap();
    assert_eq!(frame.full_view().unwrap().to_image().data(), [9, 9, 9, 255].repeat(4));
}

#[test]
fn frame_received_while_not_wanted_is_converted_when_asked_for() {
    // Arrange
    let slot = FrameSlot::default();
    slot.set_wanted(false);

    // Act
    slot.receive(&[7, 7, 7, 7], GRAY);

    // Assert
    assert_eq!(first_pixel(&slot), 7);
}

#[test]
fn newer_converted_frame_wins_over_a_kept_raw_frame() {
    // Arrange
    let slot = FrameSlot::default();
    slot.set_wanted(false);
    slot.receive(&[1, 1, 1, 1], GRAY);
    slot.set_wanted(true);

    // Act
    slot.receive(&[2, 2, 2, 2], GRAY);

    // Assert
    assert_eq!(first_pixel(&slot), 2);
}

#[test]
fn frames_are_numbered_upwards() {
    // Arrange
    let slot = FrameSlot::default();
    slot.receive(&[1, 1, 1, 1], GRAY);
    let first = slot.latest(Instant::now()).unwrap().sequence();

    // Act
    slot.receive(&[2, 2, 2, 2], GRAY);

    // Assert
    assert!(slot.latest(Instant::now()).unwrap().sequence() > first);
}

#[test]
fn frames_of_another_device_get_other_numbers() {
    // Arrange
    let (first, second) = (FrameSlot::default(), FrameSlot::default());
    first.receive(&[1, 1, 1, 1], GRAY);
    second.receive(&[1, 1, 1, 1], GRAY);

    // Act
    let (a, b) = (first.latest(Instant::now()).unwrap(), second.latest(Instant::now()).unwrap());

    // Assert
    assert_ne!(a.sequence(), b.sequence());
}

#[test]
fn placeholder_keeps_the_previous_frame() {
    // Arrange
    let slot = FrameSlot::default();
    slot.receive(&[0; 6], NV12);
    let before = slot.latest(Instant::now()).unwrap().sequence();

    // Act
    slot.receive(&PLACEHOLDER, NV12);

    // Assert
    assert_eq!(slot.latest(Instant::now()).unwrap().sequence(), before);
}

#[test]
fn placeholder_alone_is_not_a_frame() {
    // Arrange
    let slot = FrameSlot::default();

    // Act
    slot.receive(&PLACEHOLDER, NV12);

    // Assert
    assert!(!slot.receiving(Instant::now()));
}

#[test]
fn placeholder_while_not_wanted_is_not_a_frame() {
    // Arrange
    let slot = FrameSlot::default();
    slot.set_wanted(false);

    // Act
    slot.receive(&PLACEHOLDER, NV12);

    // Assert
    assert!(!slot.receiving(Instant::now()));
}

#[test]
fn placeholder_is_noticed() {
    // Arrange
    let slot = FrameSlot::default();

    // Act
    slot.receive(&PLACEHOLDER, NV12);

    // Assert
    assert!(slot.placeholder_seen());
}

#[test]
fn region_with_the_placeholder_color_is_still_a_frame() {
    // Arrange
    let slot = FrameSlot::default();
    slot.set_region(Some(Region { x: 0, y: 0, width: 2, height: 1 }));
    // The top row is placeholder gray, the bottom row isn't
    let frame = [127, 127, 0, 0, 127, 127];

    // Act
    slot.receive(&frame, NV12);

    // Assert
    assert!(slot.receiving(Instant::now()));
}

#[test]
fn frame_older_than_the_timeout_is_not_returned() {
    // Arrange
    let slot = FrameSlot::default();
    slot.receive(&[1, 1, 1, 1], GRAY);

    // Act
    let latest = slot.latest(Instant::now() + FRAME_TIMEOUT + Duration::from_millis(1));

    // Assert
    assert!(latest.is_none());
}

#[test]
fn kept_frame_times_out_from_its_arrival() {
    // Arrange
    let slot = FrameSlot::default();
    slot.set_wanted(false);
    slot.receive(&[1, 1, 1, 1], GRAY);

    // Act
    let latest = slot.latest(Instant::now() + FRAME_TIMEOUT + Duration::from_millis(1));

    // Assert
    assert!(latest.is_none());
}

#[test]
fn frame_that_fails_to_convert_is_skipped() {
    // Arrange
    let slot = FrameSlot::default();
    slot.receive(&[3, 3, 3, 3], GRAY);

    // Act
    slot.receive(&[4], GRAY);

    // Assert
    assert_eq!(first_pixel(&slot), 3);
}

#[test]
fn only_the_region_is_converted() {
    // Arrange
    let slot = FrameSlot::default();
    let region = Region { x: 1, y: 0, width: 1, height: 2 };
    slot.set_region(Some(region));

    // Act
    slot.receive(&[1, 2, 3, 4], GRAY);

    // Assert
    let frame = slot.latest(Instant::now()).unwrap();
    assert_eq!(
        (frame.full_view().is_none(), frame.region_view(&region).unwrap().to_image().data().to_vec()),
        (true, [2, 2, 2, 255, 4, 4, 4, 255].to_vec())
    );
}

#[test]
fn region_view_of_another_region_is_none() {
    // Arrange
    let slot = FrameSlot::default();
    slot.set_region(Some(Region { x: 1, y: 1, width: 1, height: 1 }));
    slot.receive(&[1, 2, 3, 4], GRAY);
    let frame = slot.latest(Instant::now()).unwrap();

    // Act
    let view = frame.region_view(&Region { x: 0, y: 0, width: 1, height: 1 });

    // Assert
    assert!(view.is_none());
}

#[test]
fn region_view_crops_a_whole_frame() {
    // Arrange
    let slot = FrameSlot::default();
    slot.receive(&[1, 2, 3, 4], GRAY);
    let frame = slot.latest(Instant::now()).unwrap();

    // Act
    let view = frame.region_view(&Region { x: 0, y: 1, width: 5, height: 5 }).unwrap();

    // Assert
    assert_eq!((view.width(), view.height(), view.row(0)[0]), (2, 1, 3));
}
