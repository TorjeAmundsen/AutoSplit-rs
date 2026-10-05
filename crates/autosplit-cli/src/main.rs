//! Headless AutoSplit: checks split image folders, compares images, replays recorded frames
//! through the split state machine, and benchmarks comparisons.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use autosplit_core::commands::{Output, outputs_for};
use autosplit_core::compare::{CompareScratch, ComparisonMethod};
use autosplit_core::image::BgraImage;
use autosplit_core::profile::UserProfile;
use autosplit_core::runner::{Effect, Runner, RunnerSettings};
use autosplit_core::split_folder::{FolderLoad, SplitSet, load_profile_folder, load_profile_folder_to_start};
use autosplit_core::split_image::{ImageType, SplitContent, SplitImage};
use autosplit_core::text::decimal;
use clap::{Parser, Subcommand};

/// Ticks at one timestamp after which `replay` gives up: the run is splitting in a loop.
const MAX_TICKS_WITHOUT_TIME_PASSING: u32 = 10_000;

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: CliCommand,
}

#[derive(Subcommand)]
enum CliCommand {
    /// Validate a profile's split image folder and show how each image is read.
    Check {
        profile: PathBuf,
        /// Use this folder instead of the profile's.
        #[arg(long)]
        split_dir: Option<PathBuf>,
    },
    /// Compare a split image with a captured frame (PNG) using every comparison method.
    Compare { split_image: PathBuf, capture: PathBuf },
    /// Run the split state machine over recorded frames and print what would be sent.
    Replay {
        profile: PathBuf,
        /// Folder of PNG frames, played in file name order.
        frames: PathBuf,
        /// Frame rate the frames were recorded at.
        #[arg(long, default_value_t = 60.0, value_parser = parse_fps)]
        fps: f64,
        #[arg(long)]
        split_dir: Option<PathBuf>,
        /// Print what LiveSplit would receive instead of hotkey presses.
        #[arg(long)]
        auto_controlled: bool,
    },
    /// Time comparisons, including resizing the capture, for each method.
    Bench {
        /// Split images to benchmark. Defaults to a 320x240 opaque image and a masked one.
        split_images: Vec<PathBuf>,
        /// Capture size to resize from.
        #[arg(long, default_value = "1920x1080")]
        capture: String,
        #[arg(long, default_value_t = 2000, value_parser = clap::value_parser!(u32).range(1..))]
        iterations: u32,
    },
}

fn parse_fps(text: &str) -> Result<f64, String> {
    match text.parse::<f64>() {
        Ok(fps) if fps.is_finite() && fps > 0.0 => Ok(fps),
        _ => Err("must be a number above 0".into()),
    }
}

fn main() -> Result<()> {
    match Cli::parse().command {
        CliCommand::Check { profile, split_dir } => check(&profile, split_dir),
        CliCommand::Compare { split_image, capture } => compare(&split_image, &capture),
        CliCommand::Replay { profile, frames, fps, split_dir, auto_controlled } => {
            replay(&profile, &frames, fps, split_dir, auto_controlled)
        }
        CliCommand::Bench { split_images, capture, iterations } => bench(&split_images, &capture, iterations),
    }
}

fn load_profile(path: &Path, split_dir: Option<PathBuf>) -> Result<UserProfile> {
    let mut profile = UserProfile::load(path).with_context(|| format!("loading {}", path.display()))?;
    if let Some(split_dir) = split_dir {
        profile.split_image_directory = split_dir.to_string_lossy().into_owned();
    }
    Ok(profile)
}

/// Prints load errors like AutoSplit's message boxes, then fails if the folder is invalid.
fn folder_result(load: FolderLoad) -> Result<SplitSet> {
    for error in &load.load_errors {
        eprintln!("error: {error}");
    }
    for warning in &load.warnings {
        eprintln!("warning: {warning}");
    }
    Ok(load.result?)
}

fn load_split_image(path: &Path) -> Result<SplitImage> {
    let image = SplitImage::load(path);
    if let Some(error) = image.load_error() {
        bail!("{error}");
    }
    Ok(image)
}

fn check(profile_path: &Path, split_dir: Option<PathBuf>) -> Result<()> {
    let profile = load_profile(profile_path, split_dir)?;
    let set = folder_result(load_profile_folder(&profile, false))?;
    let defaults = profile.split_defaults();
    println!(
        "{:<44} {:<6} {:>9} {:>8} {:>8} {:>5} {:>6} {:<5} content",
        "file", "type", "threshold", "pause", "delay", "loops", "method", "flags"
    );
    let images = set.start.iter().chain(&set.reset).chain(&set.splits);
    for image in images {
        let image_type = match image.image_type {
            ImageType::Start => "start",
            ImageType::Reset => "reset",
            ImageType::Split => "split",
        };
        let flags: String = [(image.is_dummy(), 'd'), (image.is_below(), 'b'), (image.is_pause(), 'p')]
            .iter()
            .filter_map(|(set, flag)| set.then_some(*flag))
            .collect();
        let method = image.comparison_method(&defaults).map_or("none".into(), |method| method.index().to_string());
        let content = match &image.content {
            SplitContent::Image { pixels, mask: None, .. } => format!("{}x{}", pixels.width(), pixels.height()),
            SplitContent::Image { pixels, mask: Some(_), .. } => {
                format!("{}x{} masked", pixels.width(), pixels.height())
            }
            SplitContent::Invalid(_) => "invalid".into(),
        };
        println!(
            "{:<44} {:<6} {:>9} {:>7}s {:>6}ms {:>5} {:>6} {:<5} {content}",
            image.filename,
            image_type,
            decimal(image.similarity_threshold(&defaults)),
            image.pause_time_secs(&defaults),
            image.delay_time_ms(&defaults),
            image.loops,
            method,
            flags,
        );
    }
    Ok(())
}

fn compare(split_image: &Path, capture_path: &Path) -> Result<()> {
    let image = load_split_image(split_image)?;
    let capture = BgraImage::open_png(capture_path).with_context(|| format!("loading {}", capture_path.display()))?;
    let mut scratch = CompareScratch::default();
    for method in ComparisonMethod::ALL {
        let similarity = image.compare(Some(capture.view()), Some(method), &mut scratch);
        println!("{method:?}: {similarity}");
    }
    Ok(())
}

fn frame_paths(frames: &Path) -> Result<Vec<PathBuf>> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(frames)
        .with_context(|| format!("reading {}", frames.display()))?
        .filter_map(|entry| Some(entry.ok()?.path()))
        .filter(|path| path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("png")))
        .collect();
    paths.sort();
    if paths.is_empty() {
        bail!("no PNG frames in {}", frames.display());
    }
    Ok(paths)
}

/// Plays the host: reloads the folder when asked, starts right away when there's no start
/// image (like pressing the split hotkey), and prints each command with its timestamp.
fn replay(
    profile_path: &Path,
    frames: &Path,
    fps: f64,
    split_dir: Option<PathBuf>,
    auto_controlled: bool,
) -> Result<()> {
    let profile = load_profile(profile_path, split_dir)?;
    let paths = frame_paths(frames)?;
    let end = Duration::from_secs_f64(paths.len() as f64 / fps);
    let reload = || folder_result(load_profile_folder(&profile, auto_controlled));
    let reload_to_start = || folder_result(load_profile_folder_to_start(&profile, auto_controlled));

    let mut runner = Runner::new(RunnerSettings::from_profile(&profile), auto_controlled);
    let initial = reload()?;
    let has_start_image = initial.start.is_some();
    runner.arm(initial, Duration::ZERO);
    if !has_start_image {
        runner.start(reload_to_start()?, Duration::ZERO, Duration::ZERO);
    }

    let mut loaded: Option<(usize, BgraImage)> = None;
    let mut now = Duration::ZERO;
    let mut ticks_at_now = 0;
    while let Some(next) = runner.next_wakeup().filter(|next| *next < end) {
        ticks_at_now = if next > now { 0 } else { ticks_at_now + 1 };
        if ticks_at_now > MAX_TICKS_WITHOUT_TIME_PASSING {
            bail!(
                "at {:.3}s the run keeps splitting without time passing. Is loop_splits on with no pause time?",
                now.as_secs_f64()
            );
        }
        now = now.max(next);
        let frame_index = ((now.as_secs_f64() * fps) as usize).min(paths.len() - 1);
        if loaded.as_ref().is_none_or(|(index, _)| *index != frame_index) {
            let frame = BgraImage::open_png(&paths[frame_index])
                .with_context(|| format!("loading {}", paths[frame_index].display()))?;
            loaded = Some((frame_index, frame));
        }
        runner.tick(now, loaded.as_ref().map(|(_, frame)| frame.view()));
        for effect in runner.take_effects() {
            match effect {
                Effect::Send(command) => {
                    let outputs: Vec<String> = outputs_for(command, &profile, auto_controlled)
                        .into_iter()
                        .map(|output| match output {
                            Output::Screenshot => "screenshot".into(),
                            Output::PressHotkey(hotkey) => format!("press {hotkey:?}"),
                            Output::PrintLine(line) => format!("print {line:?}"),
                        })
                        .collect();
                    println!(
                        "{:>10.3}s frame {frame_index:>6}  {:<6} {}",
                        now.as_secs_f64(),
                        command.name(),
                        outputs.join(", ")
                    );
                }
                Effect::RequestStart => {
                    runner.start(reload_to_start()?, now, now);
                }
                Effect::Stopped => {
                    println!("{:>10.3}s frame {frame_index:>6}  run ended", now.as_secs_f64());
                    runner.arm(reload()?, now);
                }
            }
        }
    }
    Ok(())
}

fn parse_size(text: &str) -> Result<(usize, usize)> {
    let (width, height) = text.split_once('x').context("size must look like 1920x1080")?;
    Ok((width.parse()?, height.parse()?))
}

fn bench(split_images: &[PathBuf], capture_size: &str, iterations: u32) -> Result<()> {
    let (width, height) = parse_size(capture_size)?;
    let capture = BgraImage::from_raw(width, height, (0..width * height * 4).map(|i| (i * 7 % 251) as u8).collect());

    let mut images: Vec<(String, SplitImage)> = Vec::new();
    for path in split_images {
        let image = load_split_image(path)?;
        images.push((image.filename.clone(), image));
    }
    if images.is_empty() {
        images.push(("opaque 320x240".into(), synthetic_split(false)?));
        images.push(("masked".into(), synthetic_split(true)?));
    }

    println!("capture {width}x{height}, {iterations} iterations each");
    let mut scratch = CompareScratch::default();
    for (name, image) in &images {
        for method in ComparisonMethod::ALL {
            image.compare(Some(capture.view()), Some(method), &mut scratch);
            let started = Instant::now();
            for _ in 0..iterations {
                let capture = std::hint::black_box(capture.view());
                std::hint::black_box(image.compare(Some(capture), Some(method), &mut scratch));
            }
            let per_compare = started.elapsed() / iterations;
            println!("{name:<32} {:<12} {:>9.1} µs", format!("{method:?}"), per_compare.as_secs_f64() * 1e6);
        }
    }
    Ok(())
}

/// A split image built in memory, written to a temporary PNG so it loads like a real one.
fn synthetic_split(masked: bool) -> Result<SplitImage> {
    let (width, height) = (480, 360);
    let mut data: Vec<u8> = (0..width * height * 4).map(|i| (i * 13 % 241) as u8).collect();
    for (i, pixel) in data.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let visible = !masked || (i % width < 200 && i / width < 150);
        pixel[3] = if visible { 255 } else { 0 };
    }
    let image = BgraImage::from_raw(width, height, data);
    let path = std::env::temp_dir().join(format!("autosplit-bench-{}-{masked}.png", std::process::id()));
    std::fs::write(&path, image.encode_png()?)?;
    let split = load_split_image(&path);
    std::fs::remove_file(&path).ok();
    split
}
