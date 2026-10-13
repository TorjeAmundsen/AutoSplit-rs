//! The profile's capture device: opened in the background, reopened when lost, with messages
//! that say what to do when it doesn't work. Also `autosplit devices` and `autosplit grab`.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use autosplit_capture::{Capture, DeviceInfo, FRAME_TIMEOUT, Frame, PickedBy, list_devices, pick_device};
use autosplit_core::profile::{Region, RegionFit, UserProfile};

use crate::console::Console;

/// How often a missing or lost device is looked for again.
pub const REOPEN_INTERVAL: Duration = Duration::from_secs(2);
/// How long a device may take to send its first frame before that's worth a warning.
const FIRST_FRAME_WAIT: Duration = Duration::from_secs(2);

/// Why the device couldn't be opened.
pub struct OpenError {
    /// The device isn't there, as opposed to failing to open.
    pub missing: bool,
    pub message: String,
}

/// An opened device, with a warning about how it was found.
pub type Opened = Result<(Capture, Option<String>), OpenError>;

fn connected_list(devices: &[DeviceInfo]) -> String {
    if devices.is_empty() {
        return "none".into();
    }
    let names: Vec<String> = devices.iter().map(|device| format!("{} '{}'", device.index, device.name)).collect();
    names.join(", ")
}

const NO_DEVICES: &str = "No capture devices found. Is the capture card plugged in? OBS Virtual Camera must be \
     started in OBS (on Linux it needs v4l2loopback).";

/// Lists the devices and opens the profile's. Slow for some devices, so it runs on its own thread.
pub fn open(name: &str, index: i64) -> Opened {
    let error = |missing, message| Err(OpenError { missing, message });
    if name.is_empty() && index < 0 {
        return error(
            true,
            "The profile has no capture device. Set capture_device_name in the profile to a name from `autosplit devices`."
                .into(),
        );
    }
    let devices = match list_devices() {
        Ok(devices) => devices,
        Err(list_error) => return error(false, list_error.to_string()),
    };
    let Some((device, picked_by)) = pick_device(&devices, name, index) else {
        let which = if name.is_empty() { format!("number {index}") } else { format!("'{name}'") };
        let obs =
            if name.contains("OBS") { " If it's OBS Virtual Camera, click Start Virtual Camera in OBS." } else { "" };
        return error(
            true,
            format!(
                "The capture device {which} isn't connected. Looking for it every {} seconds. Connected: {}.{obs}",
                REOPEN_INTERVAL.as_secs(),
                connected_list(&devices)
            ),
        );
    };
    let warning = (picked_by == PickedBy::Index && !name.is_empty()).then(|| {
        format!(
            "No capture device is named '{name}', so number {index} '{}' is used. If that's the wrong one, set \
             capture_device_name in the profile to a name from `autosplit devices`.",
            device.name
        )
    });
    Capture::open(device)
        .map(|capture| (capture, warning))
        .or_else(|open_error| error(false, with_hint(&device.name, &open_error.to_string())))
}

/// An open error with advice on what to do.
fn with_hint(name: &str, reason: &str) -> String {
    let reason = reason.trim_end_matches('.');
    let hint = if reason.contains("Camera access is denied") {
        None
    } else if reason.contains("Permission denied") {
        Some("Add yourself to the video group with `sudo usermod -a -G video $USER`, then log out and back in.")
    } else if name.contains("OBS Virtual Camera") {
        Some("Start the virtual camera in OBS first.")
    } else {
        Some(
            "A capture card can only be used by one program at a time: if OBS shows it, close it there, or use OBS \
             Virtual Camera here.",
        )
    };
    match hint {
        Some(hint) => format!("{reason}. {hint}"),
        None => format!("{reason}."),
    }
}

/// What the open device is doing, as seen on one loop pass.
#[derive(Clone, Copy, Debug)]
pub struct Observed {
    pub failed: bool,
    pub receiving: bool,
    pub placeholder_seen: bool,
    /// The size of the newest frame, when there is one.
    pub frame_size: Option<(usize, usize)>,
}

/// A message about the device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    Log(String),
    Warn(String),
    /// It stopped working, and has to be opened again.
    Lost(String),
}

/// Follows an open device, turning what it does into messages.
#[derive(Debug)]
pub struct Watch {
    name: String,
    opened_at: Instant,
    receiving: bool,
    warned_no_frames: bool,
    /// The frame size last logged.
    size: Option<(usize, usize)>,
    /// Why no frames arrive, once that was warned about.
    problem: Option<String>,
}

impl Watch {
    pub fn new(name: &str, opened_at: Instant) -> Self {
        Self { name: name.to_owned(), opened_at, receiving: false, warned_no_frames: false, size: None, problem: None }
    }

    /// Why no frames arrive, once that was warned about.
    pub fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }

    pub fn receiving(&self) -> bool {
        self.receiving
    }

    pub fn observe(&mut self, observed: Observed, now: Instant, region: &Region) -> Vec<Notice> {
        let name = &self.name;
        if observed.failed {
            return vec![Notice::Lost(format!(
                "Lost '{name}'. Looking for it again every {} seconds.",
                REOPEN_INTERVAL.as_secs()
            ))];
        }
        let mut notices = Vec::new();
        let not_started = observed.placeholder_seen && name.contains("OBS");
        if observed.receiving && !self.receiving {
            if let Some(size) = observed.frame_size {
                if self.size == Some(size) {
                    notices.push(Notice::Log(format!("Receiving frames from '{name}' again")));
                } else {
                    notices.push(Notice::Log(format!("Receiving {}x{} frames from '{name}'", size.0, size.1)));
                    notices.extend(region_warning(region, size).map(Notice::Warn));
                    self.size = Some(size);
                }
            }
            self.warned_no_frames = false;
            self.problem = None;
        } else if !observed.receiving && self.receiving {
            let when = format!("for {} second", FRAME_TIMEOUT.as_secs());
            let problem = no_frames(name, not_started, &when);
            notices.push(Notice::Warn(problem.clone()));
            self.problem = Some(problem);
            self.warned_no_frames = true;
        } else if !observed.receiving
            && !self.warned_no_frames
            && now.duration_since(self.opened_at) >= FIRST_FRAME_WAIT
        {
            let problem = no_frames(name, not_started, "yet");
            notices.push(Notice::Warn(problem.clone()));
            self.problem = Some(problem);
            self.warned_no_frames = true;
        }
        self.receiving = observed.receiving;
        notices
    }
}

fn no_frames(name: &str, not_started: bool, when: &str) -> String {
    if not_started {
        return format!("'{name}' is connected but not started. In OBS, click Start Virtual Camera.");
    }
    format!("No frames from '{name}' {when}. Is it in use by another program, or switched off?")
}

/// What's wrong with comparing `region` of frames this size, if anything.
pub fn region_warning(region: &Region, (width, height): (usize, usize)) -> Option<String> {
    match region.fit(width, height) {
        RegionFit::Inside => None,
        RegionFit::Empty => Some(
            "The profile has no capture region (its width or height is 0), so nothing can match. Set \
             [capture_region] in the profile."
                .into(),
        ),
        RegionFit::Partly => Some(format!(
            "The capture region (x {}, y {}, {}x{}) doesn't fit in the {width}x{height} frames, so only part of it is \
             compared. Fix [capture_region] in the profile.",
            region.x, region.y, region.width, region.height
        )),
    }
}

/// What was last said about a closed device, so retries don't repeat it.
#[derive(Clone, Default)]
struct Problems {
    /// The last problem logged.
    last: Option<String>,
    /// Lost after working, which was already logged.
    lost: bool,
}

enum State {
    Closed {
        next_open: Option<Instant>,
        problems: Problems,
    },
    Opening {
        problems: Problems,
        /// The device to open changed meanwhile, so this open's result is dropped and the
        /// profile's device opened next.
        stale: bool,
    },
    Open {
        capture: Capture,
        watch: Watch,
    },
}

pub struct Device {
    state: State,
    /// Counts opens, so the result of an earlier open can be told apart.
    generation: u64,
}

impl Default for Device {
    fn default() -> Self {
        Self { state: State::Closed { next_open: None, problems: Problems::default() }, generation: 0 }
    }
}

impl Device {
    pub fn capture(&self) -> Option<&Capture> {
        match &self.state {
            State::Open { capture, .. } => Some(capture),
            _ => None,
        }
    }

    /// The newest frame, if frames are arriving.
    pub fn latest(&self) -> Option<Arc<Frame>> {
        self.capture().and_then(Capture::latest)
    }

    pub fn receiving(&self) -> bool {
        matches!(&self.state, State::Open { watch, .. } if watch.receiving())
    }

    /// Why there are no frames, when that's known: the device isn't there, couldn't be opened,
    /// or sends nothing.
    pub fn problem(&self) -> Option<&str> {
        match &self.state {
            State::Closed { problems, .. } | State::Opening { problems, .. } => problems.last.as_deref(),
            State::Open { watch, .. } => watch.problem(),
        }
    }

    /// Closes the device, to open the profile's again right away. An open still running is
    /// left to finish first, so two opens never hold the same device.
    pub fn reset(&mut self) {
        self.state = match self.state {
            State::Opening { .. } => State::Opening { problems: Problems::default(), stale: true },
            _ => State::Closed { next_open: None, problems: Problems::default() },
        };
    }

    /// When to check again for opening the device.
    pub fn next_open(&self) -> Option<Instant> {
        match self.state {
            State::Closed { next_open, .. } => next_open,
            _ => None,
        }
    }

    /// Opens the device when it's time, through `open`, which gets a generation to hand back
    /// to [`Device::opened`]. Otherwise follows the open device: what to convert of its frames,
    /// and whether it still works.
    pub fn update(
        &mut self,
        now: Instant,
        region: &Region,
        wanted: bool,
        console: &mut Console,
        open: impl FnOnce(u64),
    ) {
        match &mut self.state {
            State::Closed { next_open, problems } if next_open.is_none_or(|next| now >= next) => {
                let problems = std::mem::take(problems);
                self.generation += 1;
                self.state = State::Opening { problems, stale: false };
                open(self.generation);
            }
            State::Open { capture, watch } => {
                capture.set_wanted(wanted);
                capture.set_region(Some(*region));
                let observed = Observed {
                    failed: capture.failed(),
                    receiving: capture.receiving(),
                    placeholder_seen: capture.placeholder_seen(),
                    frame_size: if watch.receiving() { None } else { capture.latest().map(|frame| frame.frame_size()) },
                };
                for notice in watch.observe(observed, now, region) {
                    match notice {
                        Notice::Log(message) => console.log(message),
                        Notice::Warn(message) => console.warn(message),
                        Notice::Lost(message) => {
                            console.warn(&message);
                            let problems = Problems { last: Some(message), lost: true };
                            self.state = State::Closed { next_open: Some(now + REOPEN_INTERVAL), problems };
                            return;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    pub fn opened(&mut self, generation: u64, opened: Opened, now: Instant, console: &mut Console) {
        let State::Opening { problems, stale } = &mut self.state else { return };
        if *stale || generation != self.generation {
            self.state = State::Closed { next_open: None, problems: Problems::default() };
            return;
        }
        let mut problems = std::mem::take(problems);
        match opened {
            Ok((capture, warning)) => {
                if let Some(warning) = warning {
                    console.warn(warning);
                }
                let name = capture.device().name.clone();
                console.log(format_args!("Opened '{name}'"));
                self.state = State::Open { capture, watch: Watch::new(&name, now) };
            }
            Err(error) => {
                let already_said = (error.missing && problems.lost) || problems.last.as_ref() == Some(&error.message);
                if !already_said {
                    console.warn(&error.message);
                }
                problems.last = Some(error.message);
                self.state = State::Closed { next_open: Some(now + REOPEN_INTERVAL), problems };
            }
        }
    }
}

/// `autosplit devices`.
pub fn print_devices() -> Result<()> {
    let devices = list_devices()?;
    if devices.is_empty() {
        println!("{NO_DEVICES}");
        return Ok(());
    }
    println!("  #  name");
    for DeviceInfo { index, name } in devices {
        println!("{index:>3}  {name}");
    }
    Ok(())
}

/// `autosplit grab`: saves one frame from a device as PNG, to check what it sends. With a
/// profile, only its capture region, which is what gets compared. The first `skip` frames are
/// skipped while the device settles.
pub fn grab(device: &str, output: &Path, skip: u64, profile: Option<&UserProfile>) -> Result<()> {
    let devices = list_devices()?;
    anyhow::ensure!(!devices.is_empty(), "{NO_DEVICES}");
    let (name, index) = match device.parse::<i64>() {
        Ok(index) => ("", index),
        Err(_) => (device, -1),
    };
    let (device, _) = pick_device(&devices, name, index).with_context(|| {
        format!("No capture device is named or numbered '{device}'. Connected: {}.", connected_list(&devices))
    })?;
    let capture =
        Capture::open(device).map_err(|error| anyhow::anyhow!(with_hint(&device.name, &error.to_string())))?;
    if let Some(profile) = profile {
        capture.set_region(Some(profile.capture_region));
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut first = None;
    loop {
        let frame = capture.latest().filter(|frame| {
            let first = *first.get_or_insert(frame.sequence());
            frame.sequence() >= first + skip
        });
        if let Some(frame) = frame {
            let view = match profile {
                Some(profile) => {
                    if let Some(warning) = region_warning(&profile.capture_region, frame.frame_size()) {
                        eprintln!("warning: {warning}");
                    }
                    frame.region_view(&profile.capture_region)
                }
                None => frame.full_view(),
            };
            let image = view.map(|view| view.to_image()).unwrap_or_default();
            anyhow::ensure!(!image.is_empty(), "The profile's capture region is empty");
            std::fs::write(output, image.encode_png()?)?;
            println!(
                "Saved a {}x{} image from '{}' to {}",
                image.width(),
                image.height(),
                device.name,
                output.display()
            );
            return Ok(());
        }
        anyhow::ensure!(!capture.failed(), "'{}' stopped working", device.name);
        if Instant::now() >= deadline {
            let hint = if capture.placeholder_seen() { " Start the virtual camera in OBS." } else { "" };
            anyhow::bail!("No frames from '{}' within 10 seconds.{hint}", device.name);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
