//! Preset masks keep the screenshot only where they're fully opaque, stretched to its size.

use std::path::Path;

use autosplit_core::image::BgraImage;
use autosplit_core::imgproc::ResizeBuffers;
use autosplit_core::mask::{Mask, apply_mask};
use test_case::test_case;

const SHOT: [u8; 4] = [10, 20, 30, 255];

fn shot(width: usize, height: usize) -> BgraImage {
    BgraImage::from_raw(width, height, SHOT.repeat(width * height))
}

/// A mask of `width` by `height` with alpha `left` on its left half and `right` on its right.
fn mask(width: usize, height: usize, left: u8, right: u8) -> Mask {
    let data = (0..width * height).flat_map(|i| [0, 0, 0, if i % width < width / 2 { left } else { right }]);
    Mask::from_image(&BgraImage::from_raw(width, height, data.collect()))
}

fn pixels(image: &BgraImage) -> Vec<[u8; 4]> {
    image.data().as_chunks::<4>().0.to_vec()
}

#[test]
fn opaque_mask_keeps_everything() {
    // Arrange
    let shot = shot(32, 24);

    // Act
    let masked = apply_mask(shot.view(), &mask(1440, 1080, 255, 255), &mut ResizeBuffers::default());

    // Assert
    assert!(pixels(&masked).iter().all(|pixel| *pixel == SHOT));
}

#[test]
fn partly_transparent_pixels_are_removed() {
    // Arrange
    let shot = shot(32, 24);

    // Act
    let masked = apply_mask(shot.view(), &mask(32, 24, 254, 0), &mut ResizeBuffers::default());

    // Assert
    assert!(pixels(&masked).iter().all(|pixel| *pixel == [0, 0, 0, 0]));
}

#[test]
fn half_mask_keeps_its_half_when_stretched() {
    // Arrange
    let shot = shot(320, 240);

    // Act
    let masked = apply_mask(shot.view(), &mask(1440, 1080, 255, 0), &mut ResizeBuffers::default());

    // Assert, away from the edge between the halves, which bilinear scaling blends
    let row = &pixels(&masked)[..320];
    assert_eq!((row[0], row[150], row[170], row[319]), (SHOT, SHOT, [0, 0, 0, 0], [0, 0, 0, 0]));
}

/// Split images AutoMask made from the library's masks. Their transparency is where AutoMask
/// kept the screenshot.
#[test_case("05_Bottle_(0.93)_[35]_#1100#_{b}.png", "bottle.png"; "bottle")]
#[test_case("reset_(0.96)_[0.1].png", "reset.png"; "reset")]
fn masks_keep_what_automask_kept(made_by_automask: &str, mask: &str) {
    // Arrange
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let made = BgraImage::open_png(&root.join("tests/fixtures/real").join(made_by_automask)).unwrap();
    let mask = Mask::open(&root.join("splits/OcarinaOfTime").join(mask)).unwrap();

    // Act
    let kept = mask.scaled(made.width(), made.height(), &mut ResizeBuffers::default());

    // Assert
    let automask: Vec<bool> = made.data().as_chunks::<4>().0.iter().map(|pixel| pixel[3] == 255).collect();
    assert_eq!(kept.iter().map(|alpha| *alpha == 255).collect::<Vec<_>>(), automask);
}
