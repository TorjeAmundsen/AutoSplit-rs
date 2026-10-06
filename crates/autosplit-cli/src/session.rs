//! A splitting session: captures from the profile's video capture device and splits, driven by
//! hotkeys, LiveSplit (stdin and stdout) or LiveSplit One (WebSocket). The command line shows it
//! with a status line, the window through a [`View`], which drives it with [`UiCommand`]s.

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use autosplit_capture::Frame;
use autosplit_core::align::align_region;
use autosplit_core::auto_controlled::parse_control_line;
use autosplit_core::commands::{Command, ControlRequest, Output, TimerLink, TimerReport, outputs_for};
use autosplit_core::compare::CompareScratch;
use autosplit_core::hotkeys::HotkeyAction;
use autosplit_core::image::BgraImage;
use autosplit_core::profile::{Region, UserProfile};
use autosplit_core::runner::{Captured, Effect, Runner, RunnerSettings, Status};
use autosplit_core::split_folder::{
    FolderLoad, SplitSet, ValidationError, load_profile_folder, load_profile_folder_to_start,
};
use autosplit_core::split_image::{SplitDefaults, SplitImage};
use autosplit_core::text::{decimal_or_dash, status_line};

use crate::console::Console;
use crate::device::{self, Device, Opened};
use crate::hotkeys::Hotkeys;
use crate::livesplit_one::{self, Link, ServerEvent};
use crate::platform::PreciseTimer;
use crate::{livesplit, profile, screenshot};

/// How many new frames each image is compared with to measure the comparison rate, like
/// AutoSplit's Max FPS button.
const CHECK_FPS_ITERATIONS: u32 = 10;
/// The longest the loop sleeps, so a failed capture device is noticed.
const MAX_SLEEP: Duration = Duration::from_millis(250);
/// How often a split image folder with errors is checked again.
const FOLDER_RETRY_INTERVAL: Duration = Duration::from_secs(2);
/// How long before a wakeup Windows' timer is made precise, when it isn't already for comparing.
const PRECISE_LEAD: Duration = Duration::from_millis(32);
/// How often the terminal's status line is redrawn.
const STATUS_INTERVAL: Duration = Duration::from_millis(100);
/// How long Max FPS waits for a new frame before giving up.
const CHECK_FPS_PATIENCE: Duration = Duration::from_secs(2);

pub enum Event {
    Hotkey(HotkeyAction),
    Stdin(String),
    StdinClosed,
    LiveSplitOne(ServerEvent),
    DeviceOpened(u64, Opened),
    /// A message from a background task: a log line, or an error.
    Log(Result<String, String>),
    /// Ctrl+C, or the window closing.
    Stop,
    Ui(UiCommand),
}

/// What the GUI asks the session to do: its buttons, menus and settings.
pub enum UiCommand {
    /// "Start Auto Splitter", like the split hotkey.
    StartAutoSplitter,
    Reset,
    SkipSplit,
    UndoSplit,
    NextImage,
    PreviousImage,
    Screenshot,
    /// "Max FPS": how many comparisons a second the profile's images can do.
    CheckFps,
    ReloadStartImage,
    /// The profile as edited in the GUI. Relative folders are read from `folder`. `loaded`
    /// when it was just loaded from a file, which looks for the start image again, like
    /// AutoSplit.
    SetProfile {
        profile: Box<UserProfile>,
        folder: PathBuf,
        loaded: bool,
    },
    /// Move the capture region onto where this image is in the capture.
    AlignRegion(PathBuf),
}

/// What the session tells the window, besides what [`View::update`] shows.
pub enum Notice {
    /// An error that needs the runner to do something, which AutoSplit shows in a message box.
    /// It's already logged.
    Dialog(String),
    /// "Max FPS" finished, `None` when it failed.
    Fps(Option<u32>),
    /// Align Region moved the capture region.
    RegionAligned(Region),
    /// The toggle auto reset image hotkey was pressed.
    AutoResetToggled(bool),
    /// LiveSplit sent a profile, which the session loaded. It's as in its file.
    ProfileLoaded { path: PathBuf, profile: Box<UserProfile> },
}

/// What a [`View`] shows of the session.
pub struct Snapshot<'a> {
    pub status: &'a Status,
    pub profile: &'a UserProfile,
    /// The runner's clock.
    pub now: Duration,
    /// What the session is waiting for, when no image is shown.
    pub idle: String,
    /// The capture device's name, when it's open.
    pub device_name: Option<&'a str>,
    /// Whether the device sends frames.
    pub receiving: bool,
    /// The newest frame, when the view wants frames.
    pub frame: Option<Arc<Frame>>,
    pub images: &'a SplitSet,
    /// Changes whenever the images are loaded again.
    pub images_loaded: u64,
}

/// Shows a session in a window.
pub trait View: Send {
    /// How often [`View::update`] wants calling.
    fn interval(&self, profile: &UserProfile) -> Duration;

    /// Whether frames have to be converted for showing, besides comparing.
    fn wants_frames(&self, profile: &UserProfile) -> bool;

    fn update(&mut self, snapshot: Snapshot<'_>);

    fn notice(&mut self, notice: Notice);
}

/// AutoSplit's Max FPS in progress: each image is compared with new frames, one comparison
/// per frame, without holding up hotkeys and LiveSplit.
struct FpsCheck {
    images: Vec<SplitImage>,
    defaults: SplitDefaults,
    scratch: CompareScratch,
    /// The image being compared, and how many frames it was compared with.
    image: usize,
    compared: u32,
    last_frame: Option<u64>,
    started: Instant,
    last_frame_at: Instant,
}

/// The image compared before a tick, to say in the log what split.
struct Compared {
    filename: String,
    threshold: Option<f64>,
    below: bool,
}

pub struct Session {
    profile: UserProfile,
    /// With `--auto-controlled`, nothing is set up until LiveSplit sends a profile.
    profile_loaded: bool,
    link: TimerLink,
    runner: Runner,
    console: Console,
    /// The window, or `None` for the terminal's status line.
    view: Option<Box<dyn View>>,
    images_loaded: u64,
    fps_check: Option<FpsCheck>,
    started: Instant,
    events: mpsc::Sender<Event>,
    hotkeys: Hotkeys,
    device: Device,
    livesplit_one: Option<Link>,
    /// Folder errors and warnings already logged, which reloading the folder repeats.
    logged_folder_messages: HashSet<String>,
    /// Why the folder can't be used, if it can't.
    folder_error: Option<String>,
    next_folder_retry: Instant,
    compared: Option<Compared>,
    timer: PreciseTimer,
    next_view: Instant,
    quit: bool,
}

/// AutoSplit's names for its window capture methods.
pub fn capture_method_name(method: &str) -> &str {
    match method {
        "WINDOWS_GRAPHICS_CAPTURE" => "Windows Graphics Capture",
        "BITBLT" => "BitBlt",
        "PRINTWINDOW_RENDERFULLCONTENT" => "Force Full Content Rendering",
        "DESKTOP_DUPLICATION" => "Direct3D Desktop Duplication",
        other => other,
    }
}

impl Session {
    /// A session for `profile`. Call [`Session::connect_timer`] and [`Session::start`] once,
    /// then [`Session::run`].
    pub fn new(
        profile: UserProfile,
        link: TimerLink,
        console: Console,
        view: Option<Box<dyn View>>,
        events: mpsc::Sender<Event>,
    ) -> Self {
        let now = Instant::now();
        let hotkey_events = events.clone();
        Self {
            runner: Runner::new(RunnerSettings::from_profile(&profile), link.timer_synced()),
            profile,
            profile_loaded: true,
            link,
            console,
            view,
            images_loaded: 0,
            fps_check: None,
            started: now,
            events,
            hotkeys: Hotkeys::new(link, move |action| {
                let _ = hotkey_events.send(Event::Hotkey(action));
            }),
            device: Device::default(),
            livesplit_one: None,
            logged_folder_messages: HashSet::new(),
            folder_error: None,
            next_folder_retry: now,
            compared: None,
            timer: PreciseTimer::default(),
            next_view: now,
            quit: false,
        }
    }

    fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    pub fn console(&mut self) -> &mut Console {
        &mut self.console
    }

    fn notify(&mut self, notice: Notice) {
        if let Some(view) = &mut self.view {
            view.notice(notice);
        }
    }

    /// Logs an error that AutoSplit would show in a message box, and lets the view show it.
    pub fn dialog(&mut self, message: impl std::fmt::Display) {
        let message = message.to_string();
        self.console.error(&message);
        self.notify(Notice::Dialog(message));
    }

    /// Connects to how the timer is told: LiveSplit's stdin and stdout, LiveSplit One, or
    /// pressing hotkeys.
    pub fn connect_timer(&mut self) -> anyhow::Result<()> {
        match self.link {
            TimerLink::AutoControlled => {
                // Nothing is set up until LiveSplit sends a profile
                self.profile_loaded = false;
                livesplit::connect(&self.events)
            }
            TimerLink::LiveSplitOne => self.listen_for_livesplit_one(self.profile.livesplit_one_port),
            TimerLink::Hotkeys => {
                self.hotkeys.start_sending(&mut self.console);
                Ok(())
            }
        }
    }

    /// Listens for LiveSplit One on `port`. Fails when the port is taken.
    fn listen_for_livesplit_one(&mut self, port: u16) -> anyhow::Result<()> {
        let events = self.events.clone();
        livesplit_one::listen(port, move |event| {
            let _ = events.send(Event::LiveSplitOne(event));
        })
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::AddrInUse => anyhow::anyhow!(
                "Port {port} is already in use, probably by another autosplit that's still running. Close it, or \
                 change the LiveSplit One port (in Settings > Other, or livesplit_one_port in the profile), restart, \
                 and use that port in LiveSplit One."
            ),
            _ => anyhow::anyhow!("Couldn't listen for LiveSplit One on port {port}: {error}"),
        })?;
        self.livesplit_one = Some(Link::new(port));
        Ok(())
    }

    /// Logs how the timer is told, and sets up the profile.
    pub fn start(&mut self) {
        self.log_timer();
        if self.profile_loaded {
            self.setup();
        }
    }

    fn log_timer(&mut self) {
        let message = match (&self.livesplit_one, self.link) {
            (Some(link), _) => format!(
                "Timer: LiveSplit One. In LiveSplit One's menu, choose Connect to Server and enter {}",
                link.address()
            ),
            (None, TimerLink::Hotkeys) => {
                let hotkeys: Vec<String> = HotkeyAction::SENT
                    .iter()
                    .map(|action| (action.name(), action.hotkey(&self.profile)))
                    .filter(|(_, hotkey)| !hotkey.is_empty())
                    .map(|(name, hotkey)| format!("{name} {hotkey}"))
                    .collect();
                let hotkeys = if hotkeys.is_empty() { "none set".into() } else { hotkeys.join(", ") };
                format!("Timer: hotkeys ({hotkeys})")
            }
            (None, _) => "Timer: LiveSplit".into(),
        };
        self.console.log(message);
    }

    /// Hotkeys, the capture method check and the start image, for a newly loaded profile.
    fn setup(&mut self) {
        self.hotkeys.set_profile(&self.profile, &mut self.console);
        if let Some(method) = self.profile.window_capture_method() {
            let tail = if self.profile.has_capture_device() {
                "Its capture device is used instead, but the capture region was made for the window and probably \
                 needs changing."
            } else {
                "Only video capture devices work: pick Video Capture Device in AutoSplit and save, or set \
                 capture_device_name in the profile (see `autosplit devices`). The capture region was made for the \
                 window and probably needs changing too."
            };
            self.console.warn(format_args!(
                "The profile captures a window ({}), which isn't supported yet. {tail}",
                capture_method_name(method)
            ));
        }
        if !self.runner.is_running() {
            self.arm(false);
        }
    }

    /// Runs until stopped, or until `receiver`'s senders are gone.
    pub fn run(&mut self, receiver: &mpsc::Receiver<Event>) {
        while !self.quit {
            // Events first, so slow comparisons can't keep hotkeys and LiveSplit waiting
            while let Ok(event) = receiver.try_recv() {
                self.handle(event);
            }
            if self.quit {
                return;
            }
            let now = Instant::now();
            self.update_device(now);
            self.hotkeys.check_health(now, &mut self.console);
            self.retry_folder(now);
            if self.runner.next_wakeup().is_some_and(|due| self.elapsed() >= due) {
                self.tick();
            }
            self.check_fps_step(now);
            self.update_view(now);

            let now = Instant::now();
            let mut wake = now + MAX_SLEEP;
            if self.fps_check.is_some() {
                wake = now + Duration::from_millis(1);
            }
            if self.view_interval().is_some() {
                wake = wake.min(self.next_view);
            }
            for next in [self.device.next_open(), self.hotkeys.next_health_check()].into_iter().flatten() {
                wake = wake.min(next);
            }
            if self.folder_error.is_some() {
                wake = wake.min(self.next_folder_retry);
            }
            // Waits are only precise to Windows' 15.6 ms tick unless asked for, which costs
            // power, so precision is asked for while comparing and just before other wakeups
            let mut precise = self.runner.needs_frames(self.elapsed());
            if let Some(due) = self.runner.next_wakeup().map(|due| self.started + due) {
                if precise || due <= now + PRECISE_LEAD {
                    precise = true;
                    wake = wake.min(due);
                } else {
                    wake = wake.min(due - PRECISE_LEAD);
                }
            }
            self.timer.set(precise);
            match receiver.recv_timeout(wake.saturating_duration_since(Instant::now())) {
                Ok(event) => self.handle(event),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    fn tick(&mut self) {
        let status = self.runner.status();
        // Kept through a split delay, which shows no image, so the delayed split still says
        // which image split
        if let Some(filename) = &status.current_filename
            && self.compared.as_ref().is_none_or(|compared| compared.filename != *filename)
        {
            self.compared =
                Some(Compared { filename: filename.clone(), threshold: status.threshold, below: status.below });
        }
        let frame = self.device.latest();
        let capture = frame.as_ref().and_then(|frame| {
            let view = frame.region_view(&self.profile.capture_region)?;
            Some(Captured { view, sequence: frame.sequence() })
        });
        let now = self.elapsed();
        self.runner.tick(now, capture);
        drop(frame);
        self.handle_effects();
    }

    fn update_device(&mut self, now: Instant) {
        if !self.profile_loaded {
            return;
        }
        // The window capture warning already says there's no device
        if self.profile.window_capture_method().is_some() && !self.profile.has_capture_device() {
            return;
        }
        let wanted = self.runner.needs_frames(self.elapsed())
            || self.fps_check.is_some()
            || self.view.as_ref().is_some_and(|view| view.wants_frames(&self.profile));
        let (name, index) = (self.profile.capture_device_name.clone(), self.profile.capture_device_id);
        let events = self.events.clone();
        self.device.update(now, &self.profile.capture_region, wanted, &mut self.console, |generation| {
            std::thread::spawn(move || {
                let _ = events.send(Event::DeviceOpened(generation, device::open(&name, index)));
            });
        });
    }

    fn handle(&mut self, event: Event) {
        match event {
            Event::Hotkey(action) => self.hotkey(action),
            Event::Stdin(line) => {
                let report = parse_control_line(&line);
                if report.request.is_some() {
                    self.console.log(format_args!("LiveSplit: {line}"));
                }
                self.timer_report(report);
            }
            Event::StdinClosed => {
                self.console.log("LiveSplit closed the connection");
                self.quit = true;
            }
            Event::LiveSplitOne(event) => {
                let report = self.livesplit_one.as_mut().and_then(|link| link.event(event, &mut self.console));
                if let Some(report) = report {
                    self.timer_report(report);
                }
            }
            Event::DeviceOpened(generation, opened) => {
                self.device.opened(generation, opened, Instant::now(), &mut self.console)
            }
            Event::Log(Ok(message)) => self.console.log(message),
            Event::Log(Err(message)) => self.console.error(message),
            Event::Stop => {
                self.console.log("Stopped");
                self.quit = true;
            }
            Event::Ui(command) => self.ui(command),
        }
    }

    fn ui(&mut self, command: UiCommand) {
        let now = self.elapsed();
        match command {
            UiCommand::StartAutoSplitter => {
                if !self.runner.is_running() && self.runner.status().buttons.start {
                    self.start_run();
                }
            }
            UiCommand::Reset => self.runner.reset(),
            UiCommand::SkipSplit => self.runner.skip(now, false),
            UiCommand::UndoSplit => self.runner.undo(now, false),
            UiCommand::NextImage => self.runner.skip(now, true),
            UiCommand::PreviousImage => self.runner.undo(now, true),
            UiCommand::Screenshot => self.screenshot(),
            UiCommand::CheckFps => self.check_fps(),
            UiCommand::ReloadStartImage => {
                if !self.runner.is_running() {
                    self.logged_folder_messages.clear();
                    self.arm(true);
                }
            }
            UiCommand::SetProfile { profile, folder, loaded } => {
                let mut profile = *profile;
                profile.resolve_directories(&folder);
                if self.profile_loaded {
                    self.apply_profile(profile, loaded);
                } else {
                    // Still waiting for LiveSplit's profile, which sets everything up
                    self.profile = profile;
                    self.runner.set_settings(RunnerSettings::from_profile(&self.profile));
                }
            }
            UiCommand::AlignRegion(path) => self.align_region(&path),
        }
        self.handle_effects();
    }

    /// AutoSplit's Max FPS: compares each image with new frames, and reports how many
    /// comparisons a second that makes.
    fn check_fps(&mut self) {
        if self.fps_check.is_some() {
            return;
        }
        let load = load_profile_folder(&self.profile, self.link.timer_synced());
        let Some(images) = self.folder(load, true) else {
            return self.notify(Notice::Fps(None));
        };
        let now = Instant::now();
        self.fps_check = Some(FpsCheck {
            images: images.splits.into_iter().chain(images.start).chain(images.reset).collect(),
            defaults: self.profile.split_defaults(),
            scratch: CompareScratch::default(),
            image: 0,
            compared: 0,
            last_frame: None,
            started: now,
            last_frame_at: now,
        });
    }

    /// Compares the next image with a new frame, when one came.
    fn check_fps_step(&mut self, now: Instant) {
        let Some(check) = &mut self.fps_check else { return };
        let frame = self.device.latest();
        let sequence = frame.as_ref().map(|frame| frame.sequence());
        if sequence.is_some() && sequence != check.last_frame {
            let capture = frame.as_ref().and_then(|frame| frame.region_view(&self.profile.capture_region));
            if let Some(image) = check.images.get(check.image) {
                image.compare(capture, image.comparison_method(&check.defaults), &mut check.scratch);
            }
            check.last_frame = sequence;
            check.last_frame_at = now;
            check.compared += 1;
            if check.compared == CHECK_FPS_ITERATIONS {
                check.image += 1;
                check.compared = 0;
            }
        }
        drop(frame);
        if check.image >= check.images.len() {
            let comparisons = CHECK_FPS_ITERATIONS as usize * check.images.len();
            let fps = (comparisons as f64 / check.started.elapsed().as_secs_f64()) as u32;
            self.fps_check = None;
            self.notify(Notice::Fps(Some(fps)));
        } else if now.duration_since(check.last_frame_at) > CHECK_FPS_PATIENCE {
            self.fps_check = None;
            self.dialog("Max FPS needs frames from the capture device, and none came.");
            self.notify(Notice::Fps(None));
        }
    }

    /// AutoSplit's Align Region: finds the image in the current capture region, and makes the
    /// region the part it matched.
    fn align_region(&mut self, path: &Path) {
        let template = match BgraImage::open_png(path) {
            Ok(template) => template,
            Err(error) => return self.dialog(format_args!("Couldn't read {} as a PNG image: {error}", path.display())),
        };
        let frame = self.device.latest();
        let found = frame.as_ref().and_then(|frame| {
            let capture = frame.region_view(&self.profile.capture_region)?;
            let (width, height) = frame.frame_size();
            Some((self.profile.capture_region.rect(width, height), align_region(capture, template.view())))
        });
        let Some((rect, aligned)) = found else {
            return self.dialog("There's no capture to align the region in.");
        };
        drop(frame);
        let Some(aligned) = aligned else {
            return self.dialog("No area in capture region matched reference image. Alignment failed.");
        };
        // From where the region really starts in the frame, which a region partly outside it doesn't
        let region = &mut self.profile.capture_region;
        region.x = (rect.left + aligned.x) as i64;
        region.y = (rect.top + aligned.y) as i64;
        region.width = aligned.width as i64;
        region.height = aligned.height as i64;
        self.console.log(format_args!(
            "Aligned the capture region to {} (x {}, y {}, {}x{})",
            path.display(),
            region.x,
            region.y,
            region.width,
            region.height
        ));
        self.notify(Notice::RegionAligned(self.profile.capture_region));
    }

    /// Something the timer reported, from LiveSplit's stdin or a LiveSplit One event.
    fn timer_report(&mut self, report: TimerReport) {
        if report.wants_screenshot(&self.profile) {
            self.screenshot();
        }
        let now = self.elapsed();
        match report.request {
            None => {}
            Some(ControlRequest::Start) => {
                if !self.runner.is_running() {
                    self.start_run();
                }
            }
            Some(ControlRequest::Skip) => self.runner.skip(now, false),
            Some(ControlRequest::Undo) => self.runner.undo(now, false),
            Some(ControlRequest::Reset) => self.runner.reset(),
            Some(ControlRequest::LoadSettings(path)) => self.load_settings(&path),
            Some(ControlRequest::Kill) => self.quit = true,
        }
        self.handle_effects();
    }

    fn hotkey(&mut self, action: HotkeyAction) {
        let now = self.elapsed();
        if self.hotkeys.is_own(action.hotkey(&self.profile), now) {
            return;
        }
        match action {
            HotkeyAction::Split => {
                if !self.runner.is_running() {
                    self.console.log("Split hotkey pressed: starting the run");
                    self.start_run();
                }
            }
            HotkeyAction::Reset => {
                if self.runner.is_running() {
                    self.console.log("Reset hotkey pressed");
                    self.runner.reset();
                }
            }
            HotkeyAction::SkipSplit => self.runner.skip(now, true),
            HotkeyAction::UndoSplit => self.runner.undo(now, true),
            HotkeyAction::Pause => {}
            HotkeyAction::Screenshot => self.screenshot(),
            HotkeyAction::ToggleAutoResetImage => {
                self.profile.enable_auto_reset = !self.profile.enable_auto_reset;
                self.runner.set_settings(RunnerSettings::from_profile(&self.profile));
                let state = if self.profile.enable_auto_reset { "on" } else { "off" };
                self.console.log(format_args!("Auto reset image turned {state}"));
                self.notify(Notice::AutoResetToggled(self.profile.enable_auto_reset));
            }
        }
        self.handle_effects();
    }

    fn handle_effects(&mut self) {
        loop {
            let effects = self.runner.take_effects();
            if effects.is_empty() {
                return;
            }
            for effect in effects {
                match effect {
                    Effect::Send(command) => self.send(command),
                    Effect::RequestStart => self.start_run(),
                    Effect::Stopped => {
                        self.console.log("Run ended");
                        self.arm(false);
                    }
                }
            }
        }
    }

    /// What split, for the log: the image and the similarity that matched.
    fn split_details(&self) -> String {
        let Some(Compared { filename, threshold, below }) = &self.compared else { return String::new() };
        let (similarity, threshold) = (decimal_or_dash(self.runner.status().similarity), decimal_or_dash(*threshold));
        if *below {
            format!(" ({filename} dropped to {similarity}, below {threshold})")
        } else {
            format!(" ({filename} at {similarity} / {threshold})")
        }
    }

    fn send(&mut self, command: Command) {
        let outputs = outputs_for(command, &self.profile, self.link);
        let details =
            if matches!(command, Command::Split | Command::Pause) { self.split_details() } else { String::new() };
        let name = command.name();
        for output in outputs {
            match output {
                Output::Screenshot => self.screenshot(),
                Output::PressHotkey(hotkey) => match self.hotkeys.press(&hotkey, self.elapsed()) {
                    Ok(pressed) => self.console.log(format_args!("{name}{details}: {pressed}")),
                    Err(error) => self.console.error(format_args!("{name}{details}: {error}")),
                },
                Output::PrintLine(line) => {
                    self.console.log(format_args!("{name}{details}: told LiveSplit {line}"));
                    let mut stdout = std::io::stdout().lock();
                    if writeln!(stdout, "{line}").and_then(|()| stdout.flush()).is_err() {
                        self.quit = true;
                    }
                }
                Output::LiveSplitOne(message) => {
                    if let Some(link) = &mut self.livesplit_one {
                        link.send(command, message, &details, &mut self.console);
                    }
                }
            }
        }
    }

    /// Logs a folder's errors and warnings, and returns its images if it's valid. Errors are
    /// shown in message boxes for `dialogs`, when a button asked for the folder like in
    /// AutoSplit. Otherwise each message is only logged once, since the folder is loaded again
    /// and again.
    fn folder(&mut self, load: FolderLoad, dialogs: bool) -> Option<SplitSet> {
        let errors =
            load.load_errors.iter().map(ToString::to_string).chain(load.result.as_ref().err().map(ToString::to_string));
        for error in errors.collect::<Vec<_>>() {
            if dialogs {
                self.logged_folder_messages.insert(format!("error: {error}"));
                self.dialog(error);
            } else if self.logged_folder_messages.insert(format!("error: {error}")) {
                self.console.error(error);
            }
        }
        for warning in &load.warnings {
            let message = format!("warning: {warning}");
            if self.logged_folder_messages.insert(message.clone()) {
                self.console.log(message);
            }
        }
        self.folder_error = load.result.as_ref().err().map(ToString::to_string);
        load.result.ok()
    }

    /// Loads the split image folder and looks for its start image. `asked` when a button asked,
    /// which shows errors in message boxes.
    fn arm(&mut self, asked: bool) {
        let load = load_profile_folder(&self.profile, self.link.timer_synced());
        self.images_loaded += 1;
        match self.folder(load, asked) {
            Some(images) => {
                let has_start = images.start.is_some();
                let now = self.elapsed();
                if self.runner.arm(images, now) && has_start {
                    self.console.log("Looking for the start image");
                } else if asked && !has_start {
                    self.dialog(
                        "Your Split Image Folder does not contain an image with the keyword 'start_auto_splitter'.",
                    );
                }
            }
            None => {
                self.runner.disarm();
            }
        }
    }

    /// Checks a folder with errors again, so fixing it doesn't need a restart.
    fn retry_folder(&mut self, now: Instant) {
        if self.folder_error.is_none() || now < self.next_folder_retry || self.runner.is_running() {
            return;
        }
        self.next_folder_retry = now + FOLDER_RETRY_INTERVAL;
        self.arm(false);
        if self.folder_error.is_none() {
            self.console.log("The split image folder is OK now");
            self.logged_folder_messages.clear();
        }
    }

    fn start_run(&mut self) {
        let run_start = self.elapsed();
        let load = load_profile_folder_to_start(&self.profile, self.link.timer_synced());
        let no_split_hotkey = matches!(load.result, Err(ValidationError::NoSplitHotkey));
        self.images_loaded += 1;
        let Some(images) = self.folder(load, false) else {
            let reason = self.folder_error.clone().unwrap_or_default();
            if no_split_hotkey {
                return self.dialog(reason);
            }
            let retry = match (self.link, self.view.is_some()) {
                (TimerLink::Hotkeys, true) => "Fix it, then press Start Auto Splitter or the split hotkey.",
                (TimerLink::Hotkeys, false) => "Fix it, then press the split hotkey to try again.",
                _ => "Fix it, then reset the timer and start it again.",
            };
            return self.dialog(format_args!("The run couldn't start: {reason} {retry}"));
        };
        let now = self.elapsed();
        if self.runner.start(images, now, run_start) {
            self.console.log("Run started");
        }
    }

    fn load_settings(&mut self, path: &str) {
        if path.is_empty() {
            self.console.warn("LiveSplit asked to load a profile without giving its path");
            return;
        }
        let path = Path::new(path);
        let (raw, warnings) = match profile::load_raw(path) {
            Ok(loaded) => loaded,
            Err(error) => return self.dialog(format_args!("{error:#}")),
        };
        self.console.log(format_args!("Loaded {}", path.display()));
        for warning in warnings {
            self.console.warn(warning);
        }
        let mut profile = raw.clone();
        profile.resolve_directories(path.parent().unwrap_or(Path::new(".")));
        self.notify(Notice::ProfileLoaded { path: path.to_owned(), profile: Box::new(raw) });
        self.apply_profile(profile, true);
    }

    /// Switches to `profile`: reopens the device when it changed, and sets up hotkeys and the
    /// start image again when they did, or when it was just `loaded` from a file.
    fn apply_profile(&mut self, profile: UserProfile, loaded: bool) {
        let device_changed = (&profile.capture_method, &profile.capture_device_name, profile.capture_device_id)
            != (&self.profile.capture_method, &self.profile.capture_device_name, self.profile.capture_device_id);
        let hotkeys_changed =
            HotkeyAction::ALL.iter().any(|action| action.hotkey(&profile) != action.hotkey(&self.profile));
        let folder_changed = profile.split_image_directory != self.profile.split_image_directory;
        let first = !self.profile_loaded;
        self.profile = profile;
        self.runner.set_settings(RunnerSettings::from_profile(&self.profile));
        if device_changed || first {
            self.device.reset();
        }
        self.profile_loaded = true;
        if first {
            return self.setup();
        }
        if hotkeys_changed {
            self.hotkeys.set_profile(&self.profile, &mut self.console);
        }
        if (folder_changed || loaded) && !self.runner.is_running() {
            self.logged_folder_messages.clear();
            self.arm(false);
        }
    }

    fn screenshot(&mut self) {
        let image = self
            .device
            .latest()
            .and_then(|frame| frame.region_view(&self.profile.capture_region).map(|view| view.to_image()));
        let events = self.events.clone();
        let saving = screenshot::save(&self.profile, image, move |message| {
            let _ = events.send(Event::Log(message));
        });
        if let Err(error) = saving {
            self.dialog(error);
        }
    }

    fn view_interval(&self) -> Option<Duration> {
        match &self.view {
            Some(view) => Some(view.interval(&self.profile)),
            None => self.console.shows_status().then_some(STATUS_INTERVAL),
        }
    }

    fn update_view(&mut self, now: Instant) {
        let Some(interval) = self.view_interval() else { return };
        if now < self.next_view {
            return;
        }
        self.next_view = now + interval;
        let Some(mut view) = self.view.take() else {
            let line = self.status_line();
            return self.console.status(&line);
        };
        let frame = view.wants_frames(&self.profile).then(|| self.device.latest()).flatten();
        view.update(Snapshot {
            status: self.runner.status(),
            profile: &self.profile,
            now: self.elapsed(),
            idle: self.idle_text(),
            device_name: self.device.capture().map(|capture| capture.device().name.as_str()),
            receiving: self.device.receiving(),
            frame,
            images: self.runner.images(),
            images_loaded: self.images_loaded,
        });
        self.view = Some(view);
    }

    /// What the session is waiting for, when no image is shown.
    fn idle_text(&self) -> String {
        let link = self.livesplit_one.as_ref();
        let see_log = if self.view.is_some() { "see Help > View Log" } else { "see above" };
        match self.link {
            TimerLink::AutoControlled if !self.profile_loaded => "waiting for LiveSplit to send a profile".into(),
            _ if self.folder_error.is_some() => format!("can't start: fix the split images ({see_log})"),
            TimerLink::Hotkeys if self.profile.split_hotkey.is_empty() => {
                "can't start: the profile has no split hotkey".into()
            }
            TimerLink::Hotkeys if self.hotkeys.listening_failed() => {
                format!("can't start: hotkeys don't work ({see_log})")
            }
            TimerLink::Hotkeys => format!("ready: press {} to start", self.profile.split_hotkey),
            TimerLink::AutoControlled => "ready: start the timer in LiveSplit".into(),
            TimerLink::LiveSplitOne => match link {
                Some(link) if !link.connected() => format!("connect LiveSplit One: {}", link.address()),
                _ => "ready: start the timer in LiveSplit One".into(),
            },
        }
    }

    /// The command line's one-line status.
    fn status_line(&self) -> String {
        let link = self.livesplit_one.as_ref();
        let idle = self.idle_text();
        let mut problems = Vec::new();
        if self.profile_loaded {
            match self.device.capture() {
                None => problems.push("NO DEVICE"),
                Some(_) if !self.device.receiving() => problems.push("NO FRAMES"),
                Some(_) => {}
            }
        }
        // Idle, the idle text already says to connect it
        if link.is_some_and(|link| !link.connected()) && self.runner.is_running() {
            problems.push("LIVESPLIT ONE NOT CONNECTED");
        }
        let status = status_line(self.runner.status(), self.elapsed(), &idle, self.device.receiving());
        problems.into_iter().map(|problem| format!("{problem} | ")).collect::<String>() + &status
    }
}
