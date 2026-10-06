//! The live preview's stretch to RGBA.

use autosplit_core::image::BgraImage;
use autosplit_core::imgproc::stretch_to_rgba;

fn stretched(image: &BgraImage, width: usize, height: usize) -> Vec<[u8; 4]> {
    let mut out = vec![[0; 4]; width * height];
    stretch_to_rgba(image.view(), width, height, &mut out);
    out
}

#[test]
fn same_size_swaps_blue_and_red() {
    // Arrange
    let image = BgraImage::from_raw(2, 1, vec![1, 2, 3, 4, 5, 6, 7, 8]);

    // Act
    let out = stretched(&image, 2, 1);

    // Assert
    assert_eq!(out, [[3, 2, 1, 4], [7, 6, 5, 8]]);
}

#[test]
fn half_size_averages_each_block() {
    // Arrange
    let image = BgraImage::from_raw(2, 2, vec![0, 0, 0, 0, 100, 100, 100, 100, 20, 20, 20, 20, 40, 40, 40, 40]);

    // Act
    let out = stretched(&image, 1, 1);

    // Assert
    assert_eq!(out, [[40, 40, 40, 40]]);
}

#[test]
fn one_pixel_fills_the_output() {
    // Arrange
    let image = BgraImage::from_raw(1, 1, vec![10, 20, 30, 255]);

    // Act
    let out = stretched(&image, 3, 2);

    // Assert
    assert_eq!(out, [[30, 20, 10, 255]; 6]);
}

#[test]
fn empty_image_is_transparent() {
    // Act
    let out = stretched(&BgraImage::default(), 2, 2);

    // Assert
    assert_eq!(out, [[0; 4]; 4]);
}
