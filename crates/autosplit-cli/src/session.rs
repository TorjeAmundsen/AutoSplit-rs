//! A splitting session: captures from the profile's video capture device and splits, driven by
//! hotkeys, LiveSplit (stdin and stdout) or LiveSplit One (WebSocket). The command line and the
//! GUI each show it through a [`View`], and the GUI drives it with [`UiCommand`]s.

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
use autosplit_core::hotkeys::HotkeyAction;
use autosplit_core::image::BgraImage;
use autosplit_core::profile::UserProfile;
use autosplit_core::runner::{Captured, Effect, Runner, RunnerSettings, Status};
use autosplit_core::split_folder::{FolderLoad, SplitSet, load_profile_folder, load_profile_folder_to_start};
use autosplit_core::text::{decimal_or_dash, status_line};

use crate::console::Console;
use crate::device::{self, Device, Opened};
use crate::hotkeys::Hotkeys;
use crate::livesplit_one::{self, Link, ServerEvent};
use crate::platform::PreciseTimer;
use crate::{profile, screenshot};

/// How many new frames each image is compared with to measure the comparison rate, like
/// AutoSplit's Max FPS button.
const CHECK_FPS_ITERATIONS: u32 = 10;
/// The longest the loop sleeps, so a failed capture device is noticed.
const MAX_SLEEP: Duration = Duration::from_millis(250);
/// How often a split image folder with errors is checked again.
const FOLDER_RETRY_INTERVAL: Duration = Duration::from_secs(2);
/// How long before a wakeup Windows' timer is made precise, when it isn't already for comparing.
const PRECISE_LEAD: Duration = Duration::from_millis(32);

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
    /// The profile as edited in the GUI. Relative folders are read from `folder`.
    SetProfile {
        profile: Box<UserProfile>,
        folder: PathBuf,
    },
    /// Move the capture region onto where this image is in the capture.
    AlignRegion(PathBuf),
}

/// Shows a session: the command line's status line, or the GUI's window.
pub trait View: Send {
    /// How often [`View::update`] wants calling, `None` for never.
    fn interval(&self, profile: &UserProfile) -> Option<Duration>;

    /// Whether frames have to be converted for showing, besides comparing.
    fn wants_frames(&self, _profile: &UserProfile) -> bool {
        false
    }

    fn update(&mut self, session: &Session, console: &mut Console);

    /// An error that needs the runner to do something, which AutoSplit shows in a message box.
    /// It's already logged.
    fn dialog(&mut self, _message: &str) {}

    /// "Max FPS" finished.
    fn fps(&mut self, _fps: Option<u32>) {}

    /// The session changed the profile itself, like Align Region does.
    fn profile_changed(&mut self, _profile: &UserProfile) {}

    /// LiveSplit sent a profile, which the session loaded.
    fn profile_loaded(&mut self, _path: &Path) {}
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
    view: Option<Box<dyn View>>,
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
    /// A session for `profile`. Call [`Session::start`] once, then [`Session::run`].
    pub fn new(
        profile: UserProfile,
        link: TimerLink,
        console: Console,
        view: Box<dyn View>,
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
            view: Some(view),
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

    /// The runner's clock.
    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    pub fn console(&mut self) -> &mut Console {
        &mut self.console
    }

    pub fn profile(&self) -> &UserProfile {
        &self.profile
    }

    pub fn profile_loaded(&self) -> bool {
        self.profile_loaded
    }

    pub fn runner(&self) -> &Runner {
        &self.runner
    }

    pub fn status(&self) -> &Status {
        self.runner.status()
    }

    pub fn link(&self) -> TimerLink {
        self.link
    }

    /// The capture device's name, when it's open.
    pub fn device_name(&self) -> Option<&str> {
        self.device.capture().map(|capture| capture.device().name.as_str())
    }

    pub fn receiving(&self) -> bool {
        self.device.receiving()
    }

    /// The newest frame, when the device sends frames.
    pub fn latest_frame(&self) -> Option<Arc<Frame>> {
        self.device.latest()
    }

    /// Whether `autosplit` waits for LiveSplit One to connect.
    pub fn livesplit_one_waiting(&self) -> Option<String> {
        self.livesplit_one.as_ref().filter(|link| !link.connected()).map(Link::address)
    }

    /// Logs an error that AutoSplit would show in a message box, and lets the view show it.
    fn dialog(&mut self, message: impl std::fmt::Display) {
        let message = message.to_string();
        self.console.error(&message);
        if let Some(view) = &mut self.view {
            view.dialog(&message);
        }
    }

    /// Listens for LiveSplit One on `port`. Fails when the port is taken.
    pub fn listen_for_livesplit_one(&mut self, port: u16) -> anyhow::Result<()> {
        let events = self.events.clone();
        livesplit_one::listen(port, move |event| {
            let _ = events.send(Event::LiveSplitOne(event));
        })
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::AddrInUse => anyhow::anyhow!(
                "Port {port} is already in use, probably by another autosplit that's still running. Close it, or set \
                 livesplit_one_port in the profile to another port and use that port in LiveSplit One."
            ),
            _ => anyhow::anyhow!("Couldn't listen for LiveSplit One on port {port}: {error}"),
        })?;
        self.livesplit_one = Some(Link::new(port));
        Ok(())
    }

    /// Prepares pressing hotkeys for the timer, when hotkeys are how the timer is told.
    pub fn start_sending_hotkeys(&mut self) {
        self.hotkeys.start_sending(&mut self.console);
    }

    /// With `--auto-controlled`, nothing is set up until LiveSplit sends a profile.
    pub fn wait_for_profile(&mut self) {
        self.profile_loaded = false;
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
            self.arm();
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
            self.update_view(now);

            let now = Instant::now();
            let mut wake = now + MAX_SLEEP;
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
                    self.arm();
                }
            }
            UiCommand::SetProfile { profile, folder } => {
                let mut profile = *profile;
                profile.resolve_directories(&folder);
                self.apply_profile(profile);
            }
            UiCommand::AlignRegion(path) => self.align_region(&path),
        }
        self.handle_effects();
    }

    /// AutoSplit's Max FPS: compares each image with new frames, and reports how many
    /// comparisons a second that makes.
    fn check_fps(&mut self) {
        let load = load_profile_folder(&self.profile, self.link.timer_synced());
        let Some(images) = self.folder(load) else {
            if let Some(view) = &mut self.view {
                view.fps(None);
            }
            return;
        };
        let images: Vec<_> = images.splits.into_iter().chain(images.start).chain(images.reset).collect();
        let defaults = self.profile.split_defaults();
        let mut scratch = autosplit_core::compare::CompareScratch::default();
        let started = Instant::now();
        let deadline = started + Duration::from_secs(30);
        let mut last = None;
        for image in &images {
            let mut count = 0;
            while count < CHECK_FPS_ITERATIONS && Instant::now() < deadline {
                let frame = self.device.latest();
                let capture = frame.as_ref().and_then(|frame| frame.region_view(&self.profile.capture_region));
                image.compare(capture, image.comparison_method(&defaults), &mut scratch);
                let sequence = frame.as_ref().map(|frame| frame.sequence());
                if sequence != last {
                    count += 1;
                } else {
                    std::thread::sleep(Duration::from_millis(1));
                }
                last = sequence;
            }
        }
        let comparisons = CHECK_FPS_ITERATIONS * images.len() as u32;
        let fps = (Instant::now() < deadline).then(|| (comparisons as f64 / started.elapsed().as_secs_f64()) as u32);
        if fps.is_none() {
            self.dialog("Max FPS needs frames from the capture device, and none came.");
        }
        if let Some(view) = &mut self.view {
            view.fps(fps);
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
        let Some(capture) = frame.as_ref().and_then(|frame| frame.region_view(&self.profile.capture_region)) else {
            return self.dialog("There's no capture to align the region in.");
        };
        let Some(aligned) = align_region(capture, &template.view()) else {
            return self.dialog(
                "The reference image doesn't match the capture well enough. Check that it's in the capture region.",
            );
        };
        drop(frame);
        let region = &mut self.profile.capture_region;
        region.x += aligned.x as i64;
        region.y += aligned.y as i64;
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
        if let Some(view) = &mut self.view {
            view.profile_changed(&self.profile);
        }
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
                if let Some(view) = &mut self.view {
                    view.profile_changed(&self.profile);
                }
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
                        self.arm();
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

    /// Logs a folder's errors and warnings, once each, and returns its images if it's valid.
    fn folder(&mut self, load: FolderLoad) -> Option<SplitSet> {
        let messages = load
            .load_errors
            .iter()
            .map(|error| format!("error: {error}"))
            .chain(load.warnings.iter().map(|warning| format!("warning: {warning}")))
            .chain(load.result.as_ref().err().map(|error| format!("error: {error}")));
        for message in messages.collect::<Vec<_>>() {
            if !self.logged_folder_messages.insert(message.clone()) {
                continue;
            }
            match message.strip_prefix("error: ") {
                Some(error) => self.dialog(error),
                None => self.console.log(message),
            }
        }
        self.folder_error = load.result.as_ref().err().map(ToString::to_string);
        load.result.ok()
    }

    /// Loads the split image folder and looks for its start image.
    fn arm(&mut self) {
        let load = load_profile_folder(&self.profile, self.link.timer_synced());
        match self.folder(load) {
            Some(images) => {
                let has_start = images.start.is_some();
                let now = self.elapsed();
                if self.runner.arm(images, now) && has_start {
                    self.console.log("Looking for the start image");
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
        self.arm();
        if self.folder_error.is_none() {
            self.console.log("The split image folder is OK now");
            self.logged_folder_messages.clear();
        }
    }

    fn start_run(&mut self) {
        let run_start = self.elapsed();
        let load = load_profile_folder_to_start(&self.profile, self.link.timer_synced());
        let Some(images) = self.folder(load) else {
            let reason = self.folder_error.clone().unwrap_or_default();
            let retry = match self.link {
                TimerLink::Hotkeys => "Fix it, then press the split hotkey to try again.",
                _ => "Fix it, then reset the timer and start it again.",
            };
            self.dialog(format_args!("The run couldn't start: {reason} {retry}"));
            return;
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
        let (profile, warnings) = match profile::load(Path::new(path), None) {
            Ok(loaded) => loaded,
            Err(error) => return self.dialog(format_args!("{error:#}")),
        };
        self.console.log(format_args!("Loaded {path}"));
        for warning in warnings {
            self.console.warn(warning);
        }
        if let Some(view) = &mut self.view {
            view.profile_loaded(Path::new(path));
        }
        self.apply_profile(profile);
    }

    /// Switches to `profile`: reopens the device when it changed, and sets up hotkeys and the
    /// start image again when they did.
    fn apply_profile(&mut self, profile: UserProfile) {
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
        if folder_changed && !self.runner.is_running() {
            self.logged_folder_messages.clear();
            self.arm();
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
        self.view.as_ref().and_then(|view| view.interval(&self.profile))
    }

    fn update_view(&mut self, now: Instant) {
        let Some(interval) = self.view_interval() else { return };
        if now < self.next_view {
            return;
        }
        self.next_view = now + interval;
        if let Some(mut view) = self.view.take() {
            let mut console = std::mem::replace(&mut self.console, Console::silent());
            view.update(self, &mut console);
            self.console = console;
            self.view = Some(view);
        }
    }

    /// What the session is waiting for, when no image is shown.
    pub fn idle_text(&self) -> String {
        let link = self.livesplit_one.as_ref();
        match self.link {
            TimerLink::AutoControlled if !self.profile_loaded => "waiting for LiveSplit to send a profile".into(),
            _ if self.folder_error.is_some() => "can't start: fix the split images (see above)".into(),
            TimerLink::Hotkeys if self.profile.split_hotkey.is_empty() => {
                "can't start: the profile has no split hotkey".into()
            }
            TimerLink::Hotkeys if self.hotkeys.listening_failed() => {
                "can't start: hotkeys don't work (see above)".into()
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
    pub fn status_line(&self) -> String {
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
