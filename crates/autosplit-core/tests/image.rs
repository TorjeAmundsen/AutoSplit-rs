//! Borrowed frames with padded rows, and odd split image shapes.

use autosplit_core::compare::{CompareScratch, ComparisonMethod};
use autosplit_core::image::{BgraImage, BgraView};
use autosplit_core::split_image::{LoadError, SplitImage};
use test_case::test_case;

/// A 3x2 image whose rows are padded to 16 bytes, like capture APIs return them.
fn padded() -> Vec<u8> {
    let mut data = Vec::new();
    for y in 0..2_u8 {
        for x in 0..3_u8 {
            data.extend_from_slice(&[x, y, 10 * x + y, 255]);
        }
        data.extend_from_slice(&[0xEE; 4]);
    }
    data
}

#[test]
fn padded_view_copies_without_the_padding() {
    // Arrange
    let data = padded();
    let view = BgraView::new(&data, 3, 2, 16).unwrap();

    // Act
    let image = view.to_image();

    // Assert
    let expected = BgraImage::from_raw(3, 2, [&data[0..12], &data[16..28]].concat());
    assert_eq!(image, expected);
}

#[test_case(0, 2, 1, 2 => vec![0, 1, 1, 255, 1, 1, 11, 255]; "bottom left corner")]
#[test_case(1, 99, 0, 99 => vec![1, 0, 10, 255, 2, 0, 20, 255, 1, 1, 11, 255, 2, 1, 21, 255]; "out of range bounds clamp like numpy")]
#[test_case(2, 1, 0, 2 => Vec::<u8>::new(); "inverted bounds are empty")]
fn crop_of_a_padded_view(left: usize, right: usize, top: usize, bottom: usize) -> Vec<u8> {
    // Arrange
    let data = padded();
    let view = BgraView::new(&data, 3, 2, 16).unwrap();

    // Act
    let cropped = view.crop(left, right, top, bottom);

    // Assert, with the expected value checked by test_case
    cropped.to_image().data().to_vec()
}

#[test_case(&[0; 23], 3, 2, 12 => false; "data too short for the last row")]
#[test_case(&[0; 24], 3, 2, 8 => false; "stride shorter than a row")]
#[test_case(&[0; 28], 3, 2, 16 => true; "last row needs no padding")]
fn view_size_checks(data: &[u8], width: usize, height: usize, stride: usize) -> bool {
    // Act, with the expected value checked by test_case
    BgraView::new(data, width, height, stride).is_some()
}

#[test]
fn comparing_a_padded_frame_matches_comparing_it_packed() {
    // Arrange
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("split.png");
    let split: Vec<u8> = (0..64 * 48 * 4).map(|i| (i * 7 % 251) as u8).collect();
    std::fs::write(&path, BgraImage::from_raw(64, 48, split).encode_png().unwrap()).unwrap();
    let image = SplitImage::load(&path);
    let packed: Vec<u8> = (0..100 * 80 * 4).map(|i| (i * 13 % 241) as u8).collect();
    let mut padded = Vec::new();
    for row in packed.chunks(100 * 4) {
        padded.extend_from_slice(row);
        padded.extend_from_slice(&[0xAB; 24]);
    }
    let packed_frame = BgraImage::from_raw(100, 80, packed);
    let padded_frame = BgraView::new(&padded, 100, 80, 100 * 4 + 24).unwrap();
    let expected: Vec<f64> = ComparisonMethod::ALL
        .map(|method| image.compare(Some(packed_frame.view()), Some(method), &mut CompareScratch::default()))
        .to_vec();

    // Act
    let actual: Vec<f64> = ComparisonMethod::ALL
        .map(|method| image.compare(Some(padded_frame), Some(method), &mut CompareScratch::default()))
        .to_vec();

    // Assert
    assert_eq!(actual, expected);
}

#[test]
fn extremely_wide_transparent_image_loads() {
    // Arrange
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("wide.png");
    let mut data = vec![255_u8; 400_000 * 4];
    data[3] = 0;
    std::fs::write(&path, BgraImage::from_raw(400_000, 1, data).encode_png().unwrap()).unwrap();

    // Act
    let image = SplitImage::load(&path);

    // Assert
    assert_eq!(image.load_error(), None);
}

#[test]
fn fully_transparent_image_is_a_load_error() {
    // Arrange
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("invisible.png");
    std::fs::write(&path, BgraImage::from_raw(4, 4, vec![0; 64]).encode_png().unwrap()).unwrap();

    // Act
    let image = SplitImage::load(&path);

    // Assert
    assert_eq!(image.load_error(), Some(&LoadError::FullyTransparent(path)));
}
