//! Align Region finds a reference image in the capture region.

use autosplit_core::align::align_region;
use autosplit_core::image::BgraImage;

/// A capture with some texture, so every place looks different.
fn capture() -> BgraImage {
    let (width, height) = (96, 64);
    let data = (0..width * height)
        .flat_map(|i| {
            let (x, y) = (i % width, i / width);
            [(x * 7 + y * 3) as u8, (x * x + y * 5) as u8, (y * y + x) as u8, 255]
        })
        .collect();
    BgraImage::from_raw(width, height, data)
}

#[test]
fn finds_a_reference_cut_from_the_capture() {
    // Arrange
    let capture = capture();
    let reference = capture.view().crop(20, 60, 10, 40).to_image();

    // Act
    let aligned = align_region(capture.view(), reference.view()).unwrap();

    // Assert
    assert_eq!((aligned.x, aligned.y, aligned.width, aligned.height), (20, 10, 40, 30));
}

#[test]
fn finds_a_reference_twice_the_size() {
    // Arrange
    let capture = capture();
    let cut = capture.view().crop(32, 64, 16, 48).to_image();
    let mut reference = BgraImage::default();
    let mut buffers = autosplit_core::imgproc::ResizeBuffers::default();
    autosplit_core::imgproc::resize_nearest_to(cut.view(), 64, 64, &mut buffers, &mut reference);

    // Act
    let aligned = align_region(capture.view(), reference.view()).unwrap();

    // Assert
    assert_eq!((aligned.x, aligned.y, aligned.width, aligned.height), (32, 16, 32, 32));
}

#[test]
fn unrelated_reference_doesnt_align() {
    // Arrange
    let capture = capture();
    let reference = BgraImage::from_raw(16, 16, [255, 255, 255, 0].repeat(256));

    // Act
    let aligned = align_region(capture.view(), reference.view());

    // Assert
    assert_eq!(aligned, None);
}

#[test]
fn empty_capture_doesnt_align() {
    // Arrange
    let reference = capture().view().crop(0, 16, 0, 16).to_image();

    // Act
    let aligned = align_region(BgraImage::default().view(), reference.view());

    // Assert
    assert_eq!(aligned, None);
}

#[test]
fn empty_reference_doesnt_align() {
    // Arrange
    let capture = capture();

    // Act
    let aligned = align_region(capture.view(), BgraImage::default().view());

    // Assert
    assert_eq!(aligned, None);
}

#[test]
fn reference_too_large_at_every_size_doesnt_align() {
    // Arrange
    let capture = capture().view().crop(0, 8, 0, 8).to_image();
    let reference = BgraImage::from_raw(64, 64, [10, 20, 30, 255].repeat(64 * 64));

    // Act
    let aligned = align_region(capture.view(), reference.view());

    // Assert
    assert_eq!(aligned, None);
}
