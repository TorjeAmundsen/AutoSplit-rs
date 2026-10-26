//! Checks the pixel conversions against OpenCV 4.11, the version AutoSplit uses.
//! See tools/golden/generate.py. Each golden case is its own test.

use std::path::{Path, PathBuf};

use std::sync::Arc;

use autosplit_capture::convert::{Layout, PixelFormat, Yuv420, Yuv422};
use autosplit_capture::is_obs_placeholder;
use autosplit_core::image::{BgraImage, Rect};
use libtest_mimic::{Arguments, Failed, Trial};
use serde::Deserialize;
use sha2::{Digest, Sha256};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[derive(Deserialize)]
struct Golden {
    yuv: Vec<YuvCase>,
    obs_placeholder_bgr: [u8; 3],
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    jpeg: Vec<JpegCase>,
}

#[derive(Deserialize)]
struct YuvCase {
    format: String,
    width: usize,
    height: usize,
    input: Vec<u8>,
    bgr_sha256: String,
}

#[derive(Deserialize)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
struct JpegCase {
    path: String,
    bgr_sha256: String,
}

/// OpenCV's output is BGR, ours BGRA, so the alpha bytes are dropped before hashing.
fn bgr_sha256(image: &BgraImage) -> String {
    let bgr: Vec<u8> =
        image.data().as_chunks::<4>().0.iter().flat_map(|pixel| [pixel[0], pixel[1], pixel[2]]).collect();
    Sha256::digest(&bgr).iter().map(|b| format!("{b:02x}")).collect()
}

fn check<T: PartialEq + std::fmt::Debug>(what: &str, actual: T, expected: T) -> Result<(), Failed> {
    if actual == expected { Ok(()) } else { Err(format!("{what}: expected {expected:?}, got {actual:?}").into()) }
}

/// Bytes added to each row in the padded variant of a case.
const PADDING: usize = 8;

fn format(name: &str) -> PixelFormat {
    match name {
        "nv12" => PixelFormat::Yuv420(Yuv420::Nv12),
        "nv21" => PixelFormat::Yuv420(Yuv420::Nv21),
        "i420" => PixelFormat::Yuv420(Yuv420::I420),
        "yv12" => PixelFormat::Yuv420(Yuv420::Yv12),
        "yuyv" => PixelFormat::Yuv422(Yuv422::Yuyv),
        "uyvy" => PixelFormat::Yuv422(Yuv422::Uyvy),
        other => panic!("unknown format {other}"),
    }
}

fn layout(case: &YuvCase) -> Layout {
    Layout::packed(format(&case.format), case.width, case.height)
}

/// The case's frame with `PADDING` bytes after every row of every plane, and its layout.
fn padded(case: &YuvCase) -> (Vec<u8>, Layout) {
    let (width, height, input) = (case.width, case.height, case.input.as_slice());
    let pad_rows = |plane: &[u8], row_bytes: usize, padding: usize| -> Vec<u8> {
        plane.chunks(row_bytes).flat_map(|row| row.iter().copied().chain(std::iter::repeat_n(0xAB, padding))).collect()
    };
    let (data, stride) = match case.format.as_str() {
        "yuyv" | "uyvy" => (pad_rows(input, width * 2, PADDING), width * 2 + PADDING),
        "nv12" | "nv21" => {
            let (y, chroma) = input.split_at(width * height);
            let mut data = pad_rows(y, width, PADDING);
            data.extend(pad_rows(chroma, width, PADDING));
            (data, width + PADDING)
        }
        _ => {
            let (y, chroma) = input.split_at(width * height);
            let mut data = pad_rows(y, width, PADDING);
            data.extend(pad_rows(chroma, width / 2, PADDING / 2));
            (data, width + PADDING)
        }
    };
    (data, Layout { stride, ..layout(case) })
}

/// The odd area converted in the area trials: one pixel in from the top left, to the right
/// edge minus one, and at least one pixel wide.
fn odd_area(case: &YuvCase) -> Rect {
    let left = 1.min(case.width - 1);
    Rect { left, top: 1.min(case.height - 1), right: (case.width - 1).max(left + 1), bottom: case.height }
}

fn yuv_trials(cases: Vec<YuvCase>) -> Vec<Trial> {
    cases
        .into_iter()
        .map(Arc::new)
        .flat_map(|case| {
            let name = format!("yuv::{}::{}x{}", case.format, case.width, case.height);
            let trial = |suffix: &str, test: fn(&YuvCase) -> Result<(), Failed>| {
                let case = case.clone();
                Trial::test(format!("{name}{suffix}"), move || test(&case))
            };
            [
                trial("", |case| {
                    // Arrange
                    let mut image = BgraImage::default();

                    // Act
                    layout(case).convert(&case.input, Rect::full(case.width, case.height), &mut image);

                    // Assert
                    check("converted pixels", bgr_sha256(&image), case.bgr_sha256.clone())
                }),
                trial("::padded_rows", |case| {
                    // Arrange
                    let (data, layout) = padded(case);
                    let mut image = BgraImage::default();

                    // Act
                    layout.convert(&data, Rect::full(case.width, case.height), &mut image);

                    // Assert
                    check("converted pixels", bgr_sha256(&image), case.bgr_sha256.clone())
                }),
                trial("::odd_area_covers_the_wanted_pixels", |case| {
                    // Arrange
                    let mut image = BgraImage::default();

                    // Act
                    let converted = layout(case).convert(&case.input, odd_area(case), &mut image).ok_or("failed")?;

                    // Assert
                    check("covers", converted.area.contains(&odd_area(case)), true)
                }),
                trial("::odd_area_matches_the_full_conversion", |case| {
                    // Arrange
                    let layout = layout(case);
                    let mut full = BgraImage::default();
                    layout.convert(&case.input, Rect::full(case.width, case.height), &mut full);
                    let mut image = BgraImage::default();

                    // Act
                    let converted = layout.convert(&case.input, odd_area(case), &mut image).ok_or("failed")?;

                    // Assert
                    let expected = full.view().crop_rect(converted.area).to_image();
                    check("area pixels", image.data(), expected.data())
                }),
            ]
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn jpeg_trials(cases: Vec<JpegCase>) -> Vec<Trial> {
    cases
        .into_iter()
        .map(|case| {
            Trial::test(format!("jpeg::{}", case.path.rsplit('/').next().unwrap()), move || {
                // Arrange
                let jpeg = std::fs::read(repo_root().join(&case.path)).unwrap();
                let mut image = BgraImage::default();

                // Act
                let layout = Layout::packed(PixelFormat::Mjpeg, 0, 0);
                layout.convert(&jpeg, Rect::full(0, 0), &mut image).ok_or("decoding failed")?;

                // Assert
                check("decoded pixels", bgr_sha256(&image), case.bgr_sha256)
            })
        })
        .collect()
}

#[cfg(not(target_os = "linux"))]
fn jpeg_trials(_: Vec<JpegCase>) -> Vec<Trial> {
    Vec::new()
}

fn placeholder_frame() -> BgraImage {
    let mut image = BgraImage::default();
    let layout = Layout::packed(PixelFormat::Yuv420(Yuv420::Nv12), 6, 4);
    layout.convert(&[127; 6 * 4 * 3 / 2], Rect::full(6, 4), &mut image);
    image
}

fn placeholder_trials(expected: [u8; 3]) -> Vec<Trial> {
    vec![
        Trial::test("obs_placeholder_converts_like_opencv", move || {
            // Act
            let image = placeholder_frame();

            // Assert
            check("placeholder color", &image.data()[..3], expected.as_slice())
        }),
        Trial::test("obs_placeholder_is_detected", || {
            // Arrange
            let image = placeholder_frame();

            // Act
            let detected = is_obs_placeholder(&image);

            // Assert
            check("detected", detected, true)
        }),
        Trial::test("frame_with_one_other_corner_is_not_the_placeholder", || {
            // Arrange
            let mut image = placeholder_frame();
            image.data_mut()[0] = 0;

            // Act
            let detected = is_obs_placeholder(&image);

            // Assert
            check("detected", detected, false)
        }),
    ]
}

fn main() {
    let args = Arguments::from_args();
    let text = std::fs::read_to_string(repo_root().join("tests/golden/capture.json")).expect("golden file");
    let golden: Golden = serde_json::from_str(&text).expect("golden json");
    let mut trials = yuv_trials(golden.yuv);
    trials.extend(jpeg_trials(golden.jpeg));
    trials.extend(placeholder_trials(golden.obs_placeholder_bgr));
    libtest_mimic::run(&args, trials).exit();
}
