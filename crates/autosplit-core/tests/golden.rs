//! Checks the Rust port against values produced by AutoSplit's Python code and OpenCV.
//! See tools/golden/generate.py. Each golden case is its own test.

use std::path::{Path, PathBuf};

use autosplit_core::compare::{self, CompareScratch, ComparisonMethod};
use autosplit_core::filename::{self, BELOW_FLAG, DUMMY_FLAG, PAUSE_FLAG};
use autosplit_core::image::BgraImage;
use autosplit_core::imgproc::{self, ResizeBuffers};
use autosplit_core::split_image::{ImageType, SplitContent, SplitImage};
use libtest_mimic::{Arguments, Failed, Trial};
use serde::Deserialize;
use sha2::{Digest, Sha256};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn golden<T: for<'de> Deserialize<'de>>(name: &str) -> Vec<T> {
    let text = std::fs::read_to_string(repo_root().join("tests/golden").join(name)).expect("golden file");
    serde_json::from_str(&text).expect("golden json")
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn check<T: PartialEq + std::fmt::Debug>(what: &str, actual: T, expected: T) -> Result<(), Failed> {
    if actual == expected { Ok(()) } else { Err(format!("{what}: expected {expected:?}, got {actual:?}").into()) }
}

fn load_split(path: &str) -> SplitImage {
    SplitImage::load(&repo_root().join(path))
}

#[derive(Deserialize)]
struct ImageCase {
    path: String,
    image_type: String,
    flags: u32,
    loops: i64,
    width: Option<usize>,
    height: Option<usize>,
    has_mask: Option<bool>,
    pixels_sha256: Option<String>,
    mask_sha256: Option<String>,
}

fn image_trials() -> Vec<Trial> {
    golden::<ImageCase>("images.json")
        .into_iter()
        .map(|case| {
            Trial::test(format!("image::{}", case.path), move || {
                // Act
                let image = load_split(&case.path);

                // Assert
                let image_type = match image.image_type {
                    ImageType::Split => "SPLIT",
                    ImageType::Reset => "RESET",
                    ImageType::Start => "START",
                };
                check("image type", image_type, case.image_type.as_str())?;
                let flags =
                    [(image.is_dummy(), DUMMY_FLAG), (image.is_below(), BELOW_FLAG), (image.is_pause(), PAUSE_FLAG)]
                        .iter()
                        .filter_map(|(set, flag)| set.then_some(*flag))
                        .sum::<u32>();
                check("flags", flags, case.flags)?;
                check("loops", image.loops, case.loops)?;
                let SplitContent::Image { pixels, mask, .. } = &image.content else {
                    return check("valid image", false, case.width.is_some());
                };
                check("size", (pixels.width(), pixels.height()), (case.width.unwrap(), case.height.unwrap()))?;
                check("has mask", mask.is_some(), case.has_mask.unwrap())?;
                check("pixels", sha256(pixels.data()), case.pixels_sha256.unwrap())?;
                check("mask", mask.as_ref().map(|mask| sha256(mask.bytes())), case.mask_sha256)
            })
        })
        .collect()
}

#[derive(Deserialize)]
struct ComparisonCase {
    split: String,
    capture: String,
    resized_capture_sha256: String,
    method_0: f64,
    method_1: f64,
    method_2: f64,
}

fn comparison_trials() -> Vec<Trial> {
    let mut trials = Vec::new();
    for case in golden::<ComparisonCase>("comparisons.json") {
        let name = |what: &str| format!("compare::{}::{}::{what}", short(&case.split), short(&case.capture));
        let (split, capture) = (case.split.clone(), case.capture.clone());
        let expected_resized = case.resized_capture_sha256.clone();
        trials.push(Trial::test(name("resized_capture"), move || {
            // Arrange
            let image = load_split(&split);
            let SplitContent::Image { pixels, .. } = &image.content else { panic!("not an image") };
            let capture = BgraImage::open_png(&repo_root().join(&capture)).unwrap();
            let mut resized = BgraImage::default();

            // Act
            imgproc::resize_nearest_to(
                capture.view(),
                pixels.width(),
                pixels.height(),
                &mut ResizeBuffers::default(),
                &mut resized,
            );

            // Assert
            check("resized capture", sha256(resized.data()), expected_resized)
        }));
        for (method, expected) in [(0, case.method_0), (1, case.method_1), (2, case.method_2)] {
            let (split, capture) = (case.split.clone(), case.capture.clone());
            trials.push(Trial::test(name(&format!("method_{method}")), move || {
                // Arrange
                let image = load_split(&split);
                let capture = BgraImage::open_png(&repo_root().join(&capture)).unwrap();
                let mut scratch = CompareScratch::default();
                // A first comparison leaves the scratch as a run would, before the one checked
                image.compare(Some(capture.view()), Some(ComparisonMethod::L2Norm), &mut scratch);

                // Act
                let actual = image.compare(Some(capture.view()), ComparisonMethod::from_index(method), &mut scratch);

                // Assert
                check("similarity", actual, expected)
            }));
        }
    }
    trials
}

fn short(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

#[derive(Deserialize)]
struct PhashCase {
    input: Vec<u8>,
    upscaled_sha256: String,
    gray_sha256: String,
    hash: Vec<u8>,
}

fn phash_trials() -> Vec<Trial> {
    let mut trials = Vec::new();
    for (index, case) in golden::<PhashCase>("phash.json").into_iter().enumerate() {
        let input: [u8; 256] = case.input.clone().try_into().unwrap();
        let expected_upscaled = case.upscaled_sha256.clone();
        trials.push(Trial::test(format!("phash::{index:03}::upscale"), move || {
            // Arrange
            let mut upscaled = [0_u8; 32 * 32 * 4];

            // Act
            imgproc::resize_linear_exact(&input, 8, 8, 4, 32, 32, &mut ResizeBuffers::default(), &mut upscaled);

            // Assert
            check("upscaled", sha256(&upscaled), expected_upscaled)
        }));
        let expected_gray = case.gray_sha256.clone();
        trials.push(Trial::test(format!("phash::{index:03}::gray"), move || {
            // Arrange
            let mut upscaled = [0_u8; 32 * 32 * 4];
            imgproc::resize_linear_exact(&input, 8, 8, 4, 32, 32, &mut ResizeBuffers::default(), &mut upscaled);
            let mut gray = [0_u8; 32 * 32];

            // Act
            imgproc::bgr_to_gray(&upscaled, 4, &mut gray);

            // Assert
            check("gray", sha256(&gray), expected_gray)
        }));
        let expected_hash = u64::from_le_bytes(case.hash.clone().try_into().unwrap());
        trials.push(Trial::test(format!("phash::{index:03}::hash"), move || {
            // Act
            let hash = compare::phash(&input, &mut ResizeBuffers::default());

            // Assert
            check("hash", format!("{hash:064b}"), format!("{expected_hash:064b}"))
        }));
    }
    trials
}

#[derive(Deserialize)]
struct AreaCase {
    split: String,
    area_8x8: Vec<u8>,
}

fn area_trials() -> Vec<Trial> {
    golden::<AreaCase>("area.json")
        .into_iter()
        .map(|case| {
            Trial::test(format!("area::{}", short(&case.split)), move || {
                // Arrange
                let image = load_split(&case.split);
                let SplitContent::Image { pixels, .. } = &image.content else { panic!("not an image") };
                let mut small = [0_u8; 8 * 8 * 4];

                // Act
                imgproc::resize_area(
                    pixels.data(),
                    pixels.width(),
                    pixels.height(),
                    4,
                    8,
                    8,
                    &mut ResizeBuffers::default(),
                    &mut small,
                );

                // Assert
                check("area 8x8", small.to_vec(), case.area_8x8)
            })
        })
        .collect()
}

/// A float, or Python's repr of a non-finite one ("inf", "nan").
#[derive(Deserialize)]
#[serde(untagged)]
enum JsonFloat {
    Number(f64),
    Repr(String),
}

impl JsonFloat {
    fn value(&self) -> f64 {
        match self {
            Self::Number(value) => *value,
            Self::Repr(text) => text.parse().expect("float repr"),
        }
    }
}

#[derive(Deserialize)]
struct FilenameCase {
    filename: String,
    threshold: Option<JsonFloat>,
    pause: Option<JsonFloat>,
    delay: Option<i64>,
    loops: i64,
    comparison_method: Option<i64>,
    flags: u32,
}

fn filename_trials() -> Vec<Trial> {
    golden::<FilenameCase>("filenames.json")
        .into_iter()
        .map(|case| {
            Trial::test(format!("filename::{}", case.filename), move || {
                // Act
                let name = case.filename.as_str();
                let actual = (
                    filename::threshold_from_filename(name),
                    filename::pause_from_filename(name),
                    filename::delay_time_from_filename(name),
                    filename::loop_from_filename(name),
                    filename::comparison_method_from_filename(name),
                    filename::flags_from_filename(name),
                );

                // Assert
                check(
                    "threshold, pause, delay, loops, method, flags",
                    actual,
                    (
                        case.threshold.map(|t| t.value()),
                        case.pause.map(|p| p.value()),
                        case.delay,
                        case.loops,
                        case.comparison_method,
                        case.flags,
                    ),
                )
            })
        })
        .collect()
}

fn main() {
    let args = Arguments::from_args();
    let trials = [image_trials(), comparison_trials(), phash_trials(), area_trials(), filename_trials()]
        .into_iter()
        .flatten()
        .collect();
    libtest_mimic::run(&args, trials).exit();
}
