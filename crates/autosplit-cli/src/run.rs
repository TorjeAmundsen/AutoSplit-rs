//! `autosplit run` and `--auto-controlled`: captures from the profile's video capture device and
//! splits, driven by hotkeys, LiveSplit (stdin and stdout) or LiveSplit One (WebSocket).

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use anyhow::Result;
use autosplit_core::auto_controlled::{handshake_lines, parse_control_line};
use autosplit_core::commands::{Command, ControlRequest, Output, TimerLink, TimerReport, outputs_for};
use autosplit_core::hotkeys::HotkeyAction;
use autosplit_core::profile::UserProfile;
use autosplit_core::runner::{Captured, Effect, Runner, RunnerSettings};
use autosplit_core::split_folder::{FolderLoad, SplitSet, load_profile_folder, load_profile_folder_to_start};
use autosplit_core::text::{decimal_or_dash, status_line};

use crate::console::{Console, utc_now};
use crate::device::{self, Device, Opened};
use crate::hotkeys::Hotkeys;
use crate::livesplit_one::{self, Link, ServerEvent};
use crate::platform::{self, PreciseTimer};
use crate::{profile, screenshot, stdin};

/// How often the status line is redrawn.
const STATUS_INTERVAL: Duration = Duration::from_millis(100);
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
    /// Ctrl+C.
    Stop,
}

/// The image compared before a tick, to say in the log what split.
struct Compared {
    filename: String,
    threshold: Option<f64>,
    below: bool,
}

struct Host {
    profile: UserProfile,
    /// With `--auto-controlled`, nothing is set up until LiveSplit sends a profile.
    profile_loaded: bool,
    link: TimerLink,
    runner: Runner,
    console: Console,
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
    next_status: Instant,
    quit: bool,
}

/// `autosplit run <profile>`.
pub fn run(profile_path: &Path, split_dir: Option<PathBuf>) -> Result<()> {
    let (profile, warnings) = profile::load(profile_path, split_dir.as_deref())?;
    let link = TimerLink::from(profile.timer_connection);
    let (events, receiver) = mpsc::channel();
    let mut host = Host::new(profile, link, Console::stdout(), events);
    host.console.log(format_args!("Loaded {}", profile_path.display()));
    for warning in warnings {
        host.console.warn(warning);
    }
    if link == TimerLink::LiveSplitOne {
        let port = host.profile.livesplit_one_port;
        let events = host.events.clone();
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
        host.livesplit_one = Some(Link::new(port));
    } else {
        host.hotkeys.start_sending(&mut host.console);
    }
    host.log_timer();
    host.setup();
    host.run(&receiver);
    Ok(())
}

/// `autosplit --auto-controlled`: started by LiveSplit's AutoSplit Integration component, which
/// sends the profile to load through stdin.
pub fn auto_controlled() -> Result<()> {
    platform::leave_own_console();
    let mut stdout = std::io::stdout().lock();
    for line in handshake_lines(env!("CARGO_PKG_VERSION")) {
        writeln!(stdout, "{line}")?;
    }
    stdout.flush()?;
    drop(stdout);

    let (events, receiver) = mpsc::channel();
    let (line_events, closed_events) = (events.clone(), events.clone());
    stdin::read_lines(
        move |line| line_events.send(Event::Stdin(line)).is_ok(),
        move || {
            let _ = closed_events.send(Event::StdinClosed);
        },
    );
    let (console, log_path) = Console::auto_controlled();
    let mut host = Host::new(UserProfile::default(), TimerLink::AutoControlled, console, events);
    host.profile_loaded = false;
    host.console.log(format_args!(
        "AutoSplit {} started by LiveSplit on {}. Waiting for it to send the profile set in the AutoSplit \
         Integration component's settings.",
        env!("CARGO_PKG_VERSION"),
        utc_now()
    ));
    if let Some(log_path) = log_path {
        host.console.log(format_args!("Logging to {}", log_path.display()));
    }
    host.log_timer();
    host.run(&receiver);
    Ok(())
}

/// AutoSplit's names for its window capture methods.
fn capture_method_name(method: &str) -> &str {
    match method {
        "WINDOWS_GRAPHICS_CAPTURE" => "Windows Graphics Capture",
        "BITBLT" => "BitBlt",
        "PRINTWINDOW_RENDERFULLCONTENT" => "Force Full Content Rendering",
        "DESKTOP_DUPLICATION" => "Direct3D Desktop Duplication",
        other => other,
    }
}

impl Host {
    fn new(profile: UserProfile, link: TimerLink, console: Console, events: mpsc::Sender<Event>) -> Self {
        let now = Instant::now();
        let hotkey_events = events.clone();
        let stop_events = events.clone();
        // Without a handler, Ctrl+C would leave the status line behind
        let _ = ctrlc::set_handler(move || {
            let _ = stop_events.send(Event::Stop);
        });
        Self {
            runner: Runner::new(RunnerSettings::from_profile(&profile), link.timer_synced()),
            profile,
            profile_loaded: true,
            link,
            console,
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
            next_status: now,
            quit: false,
        }
    }

    /// The runner's clock.
    fn elapsed(&self) -> Duration {
        self.started.elapsed()
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

    fn run(&mut self, receiver: &mpsc::Receiver<Event>) {
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
            self.update_status(now);

            let now = Instant::now();
            let mut wake = now + MAX_SLEEP;
            if self.console.shows_status() {
                wake = wake.min(self.next_status);
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
        let wanted = self.runner.needs_frames(self.elapsed());
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
            if self.logged_folder_messages.insert(message.clone()) {
                self.console.log(message);
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
            self.console.error(format_args!("The run couldn't start: {reason} {retry}"));
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
            Err(error) => {
                self.console.error(format_args!("{error:#}"));
                return;
            }
        };
        let device_changed = (&profile.capture_method, &profile.capture_device_name, profile.capture_device_id)
            != (&self.profile.capture_method, &self.profile.capture_device_name, self.profile.capture_device_id);
        self.profile = profile;
        self.console.log(format_args!("Loaded {path}"));
        for warning in warnings {
            self.console.warn(warning);
        }
        self.runner.set_settings(RunnerSettings::from_profile(&self.profile));
        self.logged_folder_messages.clear();
        if device_changed || !self.profile_loaded {
            self.device.reset();
        }
        self.profile_loaded = true;
        self.setup();
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
            self.console.error(error);
        }
    }

    fn update_status(&mut self, now: Instant) {
        if !self.console.shows_status() || now < self.next_status {
            return;
        }
        self.next_status = now + STATUS_INTERVAL;
        let line = self.status_line();
        self.console.status(&line);
    }

    fn status_line(&self) -> String {
        let link = self.livesplit_one.as_ref();
        let idle = match self.link {
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
        };
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
