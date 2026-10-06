//! AutoSplit: without arguments (or started by LiveSplit) the window, otherwise commands that
//! split from a capture device without a window, check split image folders, compare images,
//! replay recorded frames through the split state machine, and benchmark comparisons.

// On Windows, a GUI program, so opening the window shows no console. Terminals run autosplit.com
// instead, which runs this one inside the terminal (src/bin/autosplit-console.rs).
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use autosplit::gui::{self, Options};
use autosplit::{device, hotkeys, platform, profile, run};
use autosplit_core::commands::{Output, TimerLink, outputs_for};
use autosplit_core::compare::{CompareScratch, ComparisonMethod};
use autosplit_core::image::BgraImage;
use autosplit_core::profile::UserProfile;
use autosplit_core::runner::{Captured, Effect, Runner, RunnerSettings, Showing};
use autosplit_core::split_folder::{FolderLoad, SplitSet, load_profile_folder, load_profile_folder_to_start};
use autosplit_core::split_image::{ImageType, SplitContent, SplitImage};
use autosplit_core::text::{Compared, decimal, decimal_or_dash};
use clap::{Parser, Subcommand};

/// Ticks at one timestamp after which `replay` gives up: the run is splitting in a loop.
const MAX_TICKS_WITHOUT_TIME_PASSING: u32 = 10_000;

/// AutoSplit: splits from a capture card, webcam or OBS Virtual Camera. Without a command, opens
/// the window.
#[derive(Parser)]
#[command(
    version,
    about,
    args_conflicts_with_subcommands = true,
    after_help = "Start with `autosplit devices`, then `autosplit check <profile.toml>`, then `autosplit run \
                  <profile.toml>`. TUTORIAL.md in the zip explains each step."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<CliCommand>,
    /// Only for LiveSplit's AutoSplit Integration component, which starts autosplit with it. Not
    /// for starting by hand.
    #[arg(long)]
    auto_controlled: bool,
    /// Sent by LiveSplit along with --auto-controlled: starts with the window minimized.
    #[arg(long, requires = "auto_controlled", hide = true)]
    minimized: bool,
}

#[derive(Subcommand)]
enum CliCommand {
    /// Split from the profile's capture device. Press Ctrl+C to stop.
    Run {
        /// The settings profile (.toml) saved with AutoSplit.
        profile: PathBuf,
        /// Use this split image folder instead of the profile's.
        #[arg(long)]
        split_dir: Option<PathBuf>,
    },
    /// List capture devices with their numbers. Use the name with `grab --device` or as
    /// capture_device_name in a profile.
    Devices,
    /// Save a frame from a capture device as PNG, to check what it sends.
    Grab {
        /// The device's name or number, as `autosplit devices` lists them.
        #[arg(long)]
        device: String,
        /// PNG file to write.
        output: PathBuf,
        /// Save only this profile's capture region, which is what gets compared.
        #[arg(long)]
        profile: Option<PathBuf>,
        /// Frames to skip first, while the device settles.
        #[arg(long, default_value_t = 30)]
        skip: u64,
    },
    /// Validate a profile's split image folder and show how each image is read.
    Check {
        /// The settings profile (.toml) saved with AutoSplit.
        profile: PathBuf,
        /// Use this split image folder instead of the profile's.
        #[arg(long)]
        split_dir: Option<PathBuf>,
    },
    /// Compare a split image with a captured frame (PNG) using every comparison method.
    Compare {
        /// Split image (PNG).
        split_image: PathBuf,
        /// Captured frame (PNG), for example from `grab --profile`.
        capture: PathBuf,
        /// Take the default threshold and method from this profile.
        #[arg(long)]
        profile: Option<PathBuf>,
    },
    /// Run the split state machine over recorded frames and print what would be sent.
    Replay {
        /// The settings profile (.toml) saved with AutoSplit.
        profile: PathBuf,
        /// Folder of PNG frames, played in file name order.
        frames: PathBuf,
        /// Frame rate the frames were recorded at.
        #[arg(long, default_value_t = 60.0, value_parser = parse_fps)]
        fps: f64,
        /// Use this split image folder instead of the profile's.
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
        /// Comparisons per method and image.
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

/// An error that was already printed, so it's not printed again.
#[derive(Debug)]
struct Reported;

impl std::fmt::Display for Reported {
    fn fmt(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("already reported")
    }
}

impl std::error::Error for Reported {}

fn main() -> ExitCode {
    // Commands write to the terminal they were started from
    let auto_controlled = std::env::args_os().any(|argument| argument == "--auto-controlled");
    let terminal = std::env::args_os().len() > 1 && !auto_controlled && platform::attach_console();
    match run_command() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) if error.downcast_ref::<Reported>().is_some() => ExitCode::FAILURE,
        Err(error) => {
            // Windows GUI programs have no stderr without a terminal. Elsewhere it may still be
            // read, and the dialog needs a display (and zenity on Linux) to show.
            if terminal || !cfg!(windows) {
                eprintln!("error: {error:#}");
            }
            if !terminal {
                // Started from a file manager, a shortcut or LiveSplit, with nowhere to print
                platform::show_error(&format!("{error:#}"));
            }
            ExitCode::FAILURE
        }
    }
}

fn run_command() -> Result<()> {
    let cli = Cli::parse();
    if cli.auto_controlled {
        return match gui::run(Options { auto_controlled: true, minimized: cli.minimized }) {
            // Without a window, LiveSplit can still be served
            Err(gui::Error::NoWindow(error)) => run::auto_controlled(Some(error)),
            result => result.map_err(gui::Error::into_inner),
        };
    }
    let Some(command) = cli.command else {
        return gui::run(Options { auto_controlled: false, minimized: false }).map_err(gui::Error::into_inner);
    };
    match command {
        CliCommand::Run { profile, split_dir } => run::run(&profile, split_dir),
        CliCommand::Devices => device::print_devices(),
        CliCommand::Grab { device, output, profile, skip } => {
            let profile = profile.map(|path| load_profile(&path, None)).transpose()?;
            device::grab(&device, &output, skip, profile.as_ref())
        }
        CliCommand::Check { profile, split_dir } => check(&profile, split_dir),
        CliCommand::Compare { split_image, capture, profile } => compare(&split_image, &capture, profile.as_deref()),
        CliCommand::Replay { profile, frames, fps, split_dir, auto_controlled } => {
            replay(&profile, &frames, fps, split_dir, auto_controlled)
        }
        CliCommand::Bench { split_images, capture, iterations } => bench(&split_images, &capture, iterations),
    }
}

fn load_profile(path: &Path, split_dir: Option<PathBuf>) -> Result<UserProfile> {
    let (profile, warnings) = profile::load(path, split_dir.as_deref())?;
    for warning in warnings {
        eprintln!("warning: {warning}");
    }
    Ok(profile)
}

/// Prints the folder's errors and warnings, each once, like AutoSplit's message boxes, then
/// fails if the folder can't be used. Returns how many warnings there were.
fn folder_result(load: FolderLoad) -> Result<(SplitSet, usize)> {
    let errors: Vec<String> = load.load_errors.iter().map(ToString::to_string).collect();
    for error in &errors {
        eprintln!("error: {error}");
    }
    for warning in &load.warnings {
        eprintln!("warning: {warning}");
    }
    match load.result {
        Ok(set) => Ok((set, load.warnings.len())),
        Err(error) => {
            let error = error.to_string();
            if !errors.contains(&error) {
                eprintln!("error: {error}");
            }
            Err(Reported.into())
        }
    }
}

fn load_split_image(path: &Path) -> Result<SplitImage> {
    if !path.exists() {
        bail!("{} doesn't exist", path.display());
    }
    let image = SplitImage::load(path);
    if let Some(error) = image.load_error() {
        bail!("{error}");
    }
    Ok(image)
}

fn count(number: usize, what: &str) -> String {
    format!("{number} {what}{}", if number == 1 { "" } else { "s" })
}

fn check(profile_path: &Path, split_dir: Option<PathBuf>) -> Result<()> {
    let profile = load_profile(profile_path, split_dir)?;
    let link = TimerLink::from(profile.timer_connection);
    let hotkey_problems = hotkeys::hotkey_problems(&profile, link);
    for problem in &hotkey_problems {
        eprintln!("warning: {problem}");
    }
    let (set, warnings) = folder_result(load_profile_folder(&profile, link.timer_synced()))?;
    let defaults = profile.split_defaults();
    println!(
        "{:<44} {:<6} {:>9} {:>8} {:>8} {:>5} {:<10} {:<6} content",
        "file", "type", "threshold", "pause", "delay", "loops", "method", "flags"
    );
    let images = set.start.iter().chain(&set.reset).chain(&set.splits);
    for image in images {
        let image_type = match image.image_type {
            ImageType::Start => "start",
            ImageType::Reset => "reset",
            ImageType::Split => "split",
        };
        let letters: String = [(image.is_dummy(), 'd'), (image.is_below(), 'b'), (image.is_pause(), 'p')]
            .iter()
            .filter_map(|(set, flag)| set.then_some(*flag))
            .collect();
        let flags = if letters.is_empty() { String::new() } else { format!("{{{letters}}}") };
        let method = image.comparison_method(&defaults).map_or("never", ComparisonMethod::name);
        let content = match &image.content {
            SplitContent::Image { pixels, mask: None, .. } => format!("{}x{}", pixels.width(), pixels.height()),
            SplitContent::Image { pixels, mask: Some(_), .. } => {
                format!("{}x{} masked", pixels.width(), pixels.height())
            }
            SplitContent::Invalid(_) => "invalid".into(),
        };
        println!(
            "{:<44} {:<6} {:>9} {:>7}s {:>6}ms {:>5} {:<10} {:<6} {content}",
            image.display_name(),
            image_type,
            decimal(image.similarity_threshold(&defaults)),
            image.pause_time_secs(&defaults),
            image.delay_time_ms(&defaults),
            image.loops,
            method,
            flags,
        );
    }
    let warnings = warnings + hotkey_problems.len();
    let summary = if warnings == 0 {
        "Folder OK".to_owned()
    } else {
        format!("Folder OK, with {} (above)", count(warnings, "warning"))
    };
    println!(
        "\n{summary}: {}, {}, {}",
        count(set.start.iter().count(), "start image"),
        count(set.reset.iter().count(), "reset image"),
        count(set.splits.len(), "split image")
    );
    Ok(())
}

fn compare(split_image: &Path, capture_path: &Path, profile: Option<&Path>) -> Result<()> {
    let image = load_split_image(split_image)?;
    if !capture_path.exists() {
        bail!("{} doesn't exist", capture_path.display());
    }
    let capture = BgraImage::open_png(capture_path)
        .map_err(|error| anyhow::anyhow!("{} isn't a PNG image ({error})", capture_path.display()))?;
    let defaults = match profile {
        Some(path) => load_profile(path, None)?.split_defaults(),
        None => UserProfile::default().split_defaults(),
    };
    let method = image.comparison_method(&defaults);
    let threshold = image.similarity_threshold(&defaults);
    let default_note = if profile.is_none() && !image.filename.contains('(') {
        ", the default; use --profile for your profile's"
    } else {
        ""
    };
    let mut scratch = CompareScratch::default();
    for candidate in ComparisonMethod::ALL {
        let similarity = image.compare(Some(capture.view()), Some(candidate), &mut scratch);
        let note = if method == Some(candidate) {
            let reached = if similarity >= threshold { "reached" } else { "not reached" };
            let below = if image.is_below() {
                ". With {b} it splits once the similarity drops below the threshold again"
            } else {
                ""
            };
            format!("  (this image's method, threshold {}{default_note}: {reached}{below})", decimal(threshold))
        } else {
            String::new()
        };
        println!("{:<11} {}{note}", candidate.name(), decimal(similarity));
    }
    if method.is_none() {
        println!("This image's comparison method is unknown, so it never matches.");
    }
    if let SplitContent::Image { pixels, .. } = &image.content {
        let (image_ratio, capture_ratio) = (
            pixels.width() as f64 / pixels.height().max(1) as f64,
            capture.width() as f64 / capture.height().max(1) as f64,
        );
        if (image_ratio / capture_ratio - 1.0).abs() > 0.1 {
            println!(
                "The capture is {}x{} but the split image {}x{}. If this is a whole frame, grab it with --profile so \
                 only the capture region is compared.",
                capture.width(),
                capture.height(),
                pixels.width(),
                pixels.height()
            );
        }
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

/// [`folder_result`] for reloading a folder that loaded before: its messages were printed then.
fn reloaded(load: FolderLoad) -> Result<SplitSet> {
    load.result.map_err(|error| {
        eprintln!("error: {error}");
        Reported.into()
    })
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
    let link = if auto_controlled { TimerLink::AutoControlled } else { TimerLink::from(profile.timer_connection) };
    let synced = link.timer_synced();
    let paths = frame_paths(frames)?;
    let end = Duration::from_secs_f64(paths.len() as f64 / fps);
    let reload = || reloaded(load_profile_folder(&profile, synced));
    let reload_to_start = || reloaded(load_profile_folder_to_start(&profile, synced));
    let print =
        |now: Duration, frame: usize, text: &str| println!("{:>10.3}s frame {frame:>6}  {text}", now.as_secs_f64());

    let mut runner = Runner::new(RunnerSettings::from_profile(&profile), synced);
    let (initial, _) = folder_result(load_profile_folder(&profile, synced))?;
    let has_start_image = initial.start.is_some();
    runner.arm(initial, Duration::ZERO);
    let mut runs = 0;
    if !has_start_image {
        runner.start(reload_to_start()?, Duration::ZERO, Duration::ZERO);
        print(Duration::ZERO, 0, "run started");
        runs += 1;
    }

    let mut loaded: Option<(usize, BgraImage)> = None;
    let mut now = Duration::ZERO;
    let mut ticks_at_now = 0;
    let mut compared: Option<Compared> = None;
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
        Compared::follow(&mut compared, runner.status());
        let capture = loaded.as_ref().map(|(index, frame)| Captured { view: frame.view(), sequence: *index as u64 });
        runner.tick(now, capture);
        for effect in runner.take_effects() {
            match effect {
                Effect::Send(command) => {
                    let outputs: Vec<String> = outputs_for(command, &profile, link)
                        .into_iter()
                        .map(|output| match output {
                            Output::Screenshot => "screenshot".into(),
                            Output::PressHotkey(hotkey) => format!("press {hotkey:?}"),
                            Output::PrintLine(line) => format!("print {line:?}"),
                            Output::LiveSplitOne(message) => format!("send {message}"),
                        })
                        .collect();
                    let details = Compared::details(compared.as_ref(), command, runner.status().similarity);
                    print(now, frame_index, &format!("{:<6} {}{details}", command.name(), outputs.join(", ")));
                }
                Effect::RequestStart => {
                    runner.start(reload_to_start()?, now, now);
                    print(now, frame_index, "run started");
                    runs += 1;
                }
                Effect::Stopped => {
                    print(now, frame_index, "run ended");
                    runner.arm(reload()?, now);
                }
            }
        }
    }
    println!("Played {} ({:.3}s).", count(paths.len(), "frame"), end.as_secs_f64());
    let status = runner.status();
    if runs == 0 && status.showing != Showing::Nothing {
        let (best, threshold) = (decimal_or_dash(status.highest), decimal_or_dash(status.threshold));
        let reached = status.highest.zip(status.threshold).is_some_and(|(best, threshold)| best >= threshold);
        if status.below && reached {
            println!(
                "The start image reached its threshold (best {best} / {threshold}) but, with {{b}}, never dropped \
                 below it again."
            );
        } else {
            println!("The start image never matched (best {best} / {threshold}).");
        }
    }
    Ok(())
}

fn parse_size(text: &str) -> Result<(usize, usize)> {
    let parsed = text.split_once('x').and_then(|(width, height)| Some((width.parse().ok()?, height.parse().ok()?)));
    match parsed {
        Some((width, height)) if width > 0 && height > 0 => Ok((width, height)),
        _ => bail!("the size must look like 1920x1080"),
    }
}

fn bench(split_images: &[PathBuf], capture_size: &str, iterations: u32) -> Result<()> {
    let (width, height) = parse_size(capture_size)?;
    let capture = BgraImage::from_raw(width, height, (0..width * height * 4).map(|i| (i * 7 % 251) as u8).collect());

    let mut images: Vec<(String, SplitImage)> = Vec::new();
    for path in split_images {
        let image = load_split_image(path)?;
        images.push((image.display_name(), image));
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
            println!("{name:<32} {:<12} {:>9.1} us", method.name(), per_compare.as_secs_f64() * 1e6);
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
