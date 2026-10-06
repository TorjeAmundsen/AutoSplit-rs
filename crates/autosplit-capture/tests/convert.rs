//! Converting part of a frame gives the same pixels as cutting them from the whole frame.

use autosplit_capture::convert::{Layout, PixelFormat, Yuv420};
use autosplit_core::image::{BgraImage, Rect};
use test_case::test_case;

const WIDTH: usize = 7;
const HEIGHT: usize = 5;
const PADDING: usize = 3;

fn frame(format: PixelFormat) -> (Vec<u8>, Layout) {
    let stride = WIDTH * format.bytes_per_pixel() + PADDING;
    let data = (0..stride * HEIGHT).map(|i| (i * 37 % 251) as u8).collect();
    (data, Layout { format, width: WIDTH, height: HEIGHT, stride })
}

#[test_case(PixelFormat::Bgr24 { bottom_up: true }; "bottom up bgr24")]
#[test_case(PixelFormat::Bgr24 { bottom_up: false }; "bgr24")]
#[test_case(PixelFormat::Rgb24; "rgb24")]
#[test_case(PixelFormat::Bgrx; "bgrx")]
#[test_case(PixelFormat::Gray; "gray")]
fn area_matches_the_full_conversion(format: PixelFormat) {
    // Arrange
    let (data, layout) = frame(format);
    let mut full = BgraImage::default();
    layout.convert(&data, Rect::full(WIDTH, HEIGHT), &mut full).unwrap();
    let area = Rect { left: 1, top: 2, right: 6, bottom: 5 };
    let mut image = BgraImage::default();

    // Act
    layout.convert(&data, area, &mut image).unwrap();

    // Assert
    assert_eq!(image.data(), full.view().crop_rect(area).to_image().data());
}

#[test]
fn area_outside_the_frame_converts_everything() {
    // Arrange
    let (data, layout) = frame(PixelFormat::Gray);
    let mut image = BgraImage::default();

    // Act
    let converted = layout.convert(&data, Rect { left: 3, top: 0, right: 99, bottom: 2 }, &mut image);

    // Assert
    assert_eq!(converted.map(|converted| converted.area), Some(Rect::full(WIDTH, HEIGHT)));
}

#[test]
fn too_short_frame_isnt_converted() {
    // Arrange
    let (data, layout) = frame(PixelFormat::Rgb24);
    let mut image = BgraImage::default();

    // Act
    let converted = layout.convert(&data[..data.len() - 1 - PADDING], Rect::full(WIDTH, HEIGHT), &mut image);

    // Assert
    assert_eq!(converted, None);
}

#[test]
fn odd_sized_yuv_420_frame_isnt_converted() {
    // Arrange
    let layout = Layout::packed(PixelFormat::Yuv420(Yuv420::Nv12), 3, 2);
    let mut image = BgraImage::default();

    // Act
    let converted = layout.convert(&[0; 12], Rect::full(3, 2), &mut image);

    // Assert
    assert_eq!(converted, None);
}

#[test]
fn corners_are_the_frame_corners() {
    // Arrange
    let (data, layout) = frame(PixelFormat::Gray);
    let mut full = BgraImage::default();
    layout.convert(&data, Rect::full(WIDTH, HEIGHT), &mut full).unwrap();
    let pixel = |x: usize, y: usize| -> [u8; 4] { full.data()[(y * WIDTH + x) * 4..][..4].try_into().unwrap() };

    // Act
    let corners = layout.corners(&data, &mut BgraImage::default());

    // Assert
    assert_eq!(corners, Some([pixel(0, 0), pixel(WIDTH - 1, 0), pixel(0, HEIGHT - 1), pixel(WIDTH - 1, HEIGHT - 1)]));
}

#[test_case(Layout::packed(PixelFormat::Yuv420(Yuv420::Nv12), 4, 2) => 12; "nv12")]
#[test_case(Layout::packed(PixelFormat::Yuv420(Yuv420::I420), 4, 2) => 12; "i420")]
#[test_case(Layout::packed(PixelFormat::Bgr24 { bottom_up: true }, 4, 2) => 24; "bgr24")]
fn frame_lengths(layout: Layout) -> usize {
    // Act, with the expected value checked by test_case
    layout.frame_len()
}
