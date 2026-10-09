//! A splitting session: captures from the profile's video capture device and splits, driven by
//! hotkeys, LiveSplit (stdin and stdout), LiveSplit One (WebSocket) or Molok (signals). The command
//! line shows it with a status line, the window through a [`View`], which drives it with
//! [`UiCommand`]s.

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
use autosplit_core::profile::{Region, UserProfile, capture_method_name};
use autosplit_core::runner::{Captured, Effect, Runner, RunnerSettings, Status};
use autosplit_core::split_folder::{
    FolderLoad, SplitSet, ValidationError, load_profile_folder, load_profile_folder_to_start,
};
use autosplit_core::text::{Compared, status_line};

use crate::console::Console;
use crate::device::{self, Device, Opened};
use crate::hotkeys::Hotkeys;
use crate::livesplit_one::{self, Link, ServerEvent};
#[cfg(target_os = "linux")]
use crate::molok;
use crate::platform::PreciseTimer;
use crate::{livesplit, profile, screenshot};

/// The longest the loop sleeps, so a failed capture device is noticed.
const MAX_SLEEP: Duration = Duration::from_millis(250);
/// How long before a wakeup Windows' timer is made precise, when it isn't already for comparing.
const PRECISE_LEAD: Duration = Duration::from_millis(32);
/// How often the terminal's status line is redrawn.
const STATUS_INTERVAL: Duration = Duration::from_millis(100);
/// The wait between two signals to Molok, for "start also resets". Pending real-time signals
/// are delivered lowest first, so a start sent right after a reset could arrive before it.
#[cfg(target_os = "linux")]
const MOLOK_GAP: Duration = Duration::from_millis(5);

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
    ReloadSplitImages,
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
    /// Align Region moved the capture region.
    RegionAligned(Region),
    /// The toggle auto reset image hotkey was pressed.
    AutoResetToggled(bool),
    /// LiveSplit sent a profile, which the session loaded. It's as in its file.
    ProfileLoaded { path: PathBuf, profile: Box<UserProfile> },
    /// The timer is told another way now, after the profile's timer connection changed.
    TimerLinkChanged(TimerLink),
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

pub struct Session {
    profile: UserProfile,
    /// With `--auto-controlled`, nothing is set up until LiveSplit sends a profile.
    profile_loaded: bool,
    link: TimerLink,
    /// The link and LiveSplit One port set up, to tell when the profile asks for others.
    connected_to: (TimerLink, Option<u16>),
    /// The profile asked for another timer connection during a run.
    link_after_run: bool,
    runner: Runner,
    console: Console,
    /// The window, or `None` for the terminal's status line. Taken out while it updates.
    view: Option<Box<dyn View>>,
    /// Whether there's a window, which the texts point to instead of the terminal.
    windowed: bool,
    images_loaded: u64,
    started: Instant,
    events: mpsc::Sender<Event>,
    hotkeys: Hotkeys,
    device: Device,
    livesplit_one: Option<Link>,
    /// Folder errors and warnings already logged, which reloading the folder repeats.
    logged_folder_messages: HashSet<String>,
    /// Why the folder can't be used, if it can't.
    folder_error: Option<String>,
    compared: Option<Compared>,
    timer: PreciseTimer,
    next_view: Instant,
    quit: bool,
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
            connected_to: (link, (link == TimerLink::LiveSplitOne).then_some(profile.livesplit_one_port)),
            link_after_run: false,
            profile,
            profile_loaded: true,
            link,
            console,
            windowed: view.is_some(),
            view,
            images_loaded: 0,
            started: now,
            events,
            hotkeys: Hotkeys::new(link, move |action| {
                let _ = hotkey_events.send(Event::Hotkey(action));
            }),
            device: Device::default(),
            livesplit_one: None,
            logged_folder_messages: HashSet::new(),
            folder_error: None,
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

    /// Connects to how the timer is told: LiveSplit's stdin and stdout, LiveSplit One, Molok's
    /// signals, or pressing hotkeys.
    pub fn connect_timer(&mut self) -> anyhow::Result<()> {
        match self.link {
            TimerLink::AutoControlled => {
                // Nothing is set up until LiveSplit sends a profile
                self.profile_loaded = false;
                livesplit::connect(&self.events)
            }
            TimerLink::LiveSplitOne => self.listen_for_livesplit_one(self.profile.livesplit_one_port),
            // Signals need no setup
            #[cfg(target_os = "linux")]
            TimerLink::Molok => Ok(()),
            TimerLink::Hotkeys => {
                self.hotkeys.start_sending(&mut self.console);
                Ok(())
            }
        }
    }

    /// Listens for LiveSplit One on `port`. Fails when the port is taken.
    fn listen_for_livesplit_one(&mut self, port: u16) -> anyhow::Result<()> {
        let events = self.events.clone();
        let server = livesplit_one::listen(port, move |event| {
            let _ = events.send(Event::LiveSplitOne(event));
        })
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::AddrInUse => anyhow::anyhow!(
                "Port {port} is already in use, probably by another autosplit that's still running. Close it, or \
                 change the LiveSplit One port (in Settings > Other, or livesplit_one_port in the profile), and use \
                 that port in LiveSplit One."
            ),
            _ => anyhow::anyhow!("Couldn't listen for LiveSplit One on port {port}: {error}"),
        })?;
        self.livesplit_one = Some(Link::new(port, server));
        Ok(())
    }

    /// Switches how the timer is told, to the profile's timer connection and LiveSplit One port.
    /// A run keeps the one it started with, and switches when it ends. LiveSplit's link never
    /// changes: LiveSplit started autosplit.
    /// Returns whether it switched, which loads the split image folder again.
    fn follow_timer_connection(&mut self) -> bool {
        if self.link == TimerLink::AutoControlled {
            return false;
        }
        let link = TimerLink::from(self.profile.timer_connection);
        let port = self.profile.livesplit_one_port;
        let wanted = (link, (link == TimerLink::LiveSplitOne).then_some(port));
        if wanted == self.connected_to {
            return false;
        }
        if self.runner.is_running() {
            self.link_after_run = true;
            return false;
        }
        self.link_after_run = false;
        self.connected_to = wanted;
        // Closes the port and LiveSplit One's connection
        self.livesplit_one = None;
        if link == TimerLink::LiveSplitOne
            && let Err(error) = self.listen_for_livesplit_one(port)
        {
            self.dialog(format_args!("{error:#}"));
        }
        self.link = link;
        self.hotkeys.set_link(link, &self.profile, &mut self.console);
        self.runner.set_timer_synced(link.timer_synced());
        self.log_timer();
        self.notify(Notice::TimerLinkChanged(link));
        self.logged_folder_messages.clear();
        self.arm(false);
        true
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
            #[cfg(target_os = "linux")]
            (None, TimerLink::Molok) => "Timer: Molok (POSIX signals)".into(),
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
        Compared::follow(&mut self.compared, self.runner.status());
        let now = self.elapsed();
        // During pauses, when nothing is compared, the frame isn't converted for nothing
        let frame = if self.runner.needs_frames(now) { self.device.latest() } else { None };
        let capture = frame.as_ref().and_then(|frame| {
            let view = frame.region_view(&self.profile.capture_region)?;
            Some(Captured { view, sequence: frame.sequence() })
        });
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
        let (name, index) = (&self.profile.capture_device_name, self.profile.capture_device_id);
        let events = &self.events;
        self.device.update(now, &self.profile.capture_region, wanted, &mut self.console, |generation| {
            let (name, events) = (name.clone(), events.clone());
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
            UiCommand::ReloadSplitImages => {
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
                        if !self.follow_timer_connection() {
                            self.arm(false);
                        }
                    }
                }
            }
        }
    }

    fn send(&mut self, command: Command) {
        let outputs = outputs_for(command, &self.profile, self.link);
        let details = Compared::details(self.compared.as_ref(), command, self.runner.status().similarity);
        let name = command.name();
        #[cfg(target_os = "linux")]
        let mut signaled = false;
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
                #[cfg(target_os = "linux")]
                Output::Molok(offset) => {
                    if signaled {
                        std::thread::sleep(MOLOK_GAP);
                    }
                    signaled = true;
                    match molok::signal(offset) {
                        Ok(()) => self.console.log(format_args!("{name}{details}: signaled Molok SIGRTMIN+{offset}")),
                        Err(error) => self.console.error(format_args!("{name}{details}: {error}")),
                    }
                }
            }
        }
    }

    /// Logs a folder's errors and warnings, and returns its images if it's valid. Errors are
    /// shown in message boxes for `dialogs`, when a button asked for the folder like in
    /// AutoSplit. Otherwise each message is only logged once, since the folder is loaded again
    /// and again.
    fn folder(&mut self, load: FolderLoad, dialogs: bool) -> Result<SplitSet, String> {
        let errors = load
            .ignored_files_message()
            .into_iter()
            .chain(load.load_errors.iter().map(ToString::to_string))
            .chain(load.result.as_ref().err().map(ToString::to_string));
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
        load.result.map_err(|error| error.to_string())
    }

    /// Loads the split image folder and looks for its start image. `asked` when a button asked,
    /// which shows errors in message boxes.
    fn arm(&mut self, asked: bool) {
        let load = load_profile_folder(&self.profile, self.link.needs_hotkeys());
        self.images_loaded += 1;
        let images = self.folder(load, asked);
        self.folder_error = images.as_ref().err().cloned();
        match images {
            Ok(images) => {
                let has_start = images.start.is_some();
                let now = self.elapsed();
                if self.runner.arm(images, now) && has_start {
                    self.console.log("Looking for the start image");
                }
            }
            Err(_) => {
                self.runner.disarm();
            }
        }
    }

    fn start_run(&mut self) {
        let run_start = self.elapsed();
        let load = load_profile_folder_to_start(&self.profile, self.link.needs_hotkeys());
        // Not a problem with the folder, whose state stays as it was
        if let Err(error @ ValidationError::NoSplitHotkey) = &load.result {
            return self.dialog(error);
        }
        self.images_loaded += 1;
        let images = self.folder(load, false);
        self.folder_error = images.as_ref().err().cloned();
        let reason = match images {
            Ok(images) => {
                let now = self.elapsed();
                if self.runner.start(images, now, run_start) {
                    self.console.log("Run started");
                }
                return;
            }
            Err(reason) => reason,
        };
        let retry = match (self.link, self.windowed) {
            (TimerLink::Hotkeys, true) => "Fix it, then press Start Auto Splitter or the split hotkey.",
            (TimerLink::Hotkeys, false) => "Fix it, then press the split hotkey to try again.",
            #[cfg(target_os = "linux")]
            (TimerLink::Molok, true) => "Fix it, then press Start Auto Splitter or the split hotkey.",
            #[cfg(target_os = "linux")]
            (TimerLink::Molok, false) => "Fix it, then press the split hotkey to try again.",
            _ => "Fix it, then reset the timer and start it again.",
        };
        self.dialog(format_args!("The run couldn't start: {reason} {retry}"));
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
        let folder_changed = profile.split_image_directory != self.profile.split_image_directory
            || profile.group_split_images != self.profile.group_split_images;
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
        let switched = self.follow_timer_connection();
        if (folder_changed || loaded) && !switched && !self.runner.is_running() {
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
        let see_log = if self.windowed { "see Help > View Log" } else { "see above" };
        match self.link {
            TimerLink::AutoControlled if !self.profile_loaded => "waiting for LiveSplit to send a profile".into(),
            _ if self.folder_error.is_some() && self.windowed => {
                format!("can't start: fix the split images, then press Reload Split Images ({see_log})")
            }
            _ if self.folder_error.is_some() => format!("can't start: fix the split images ({see_log})"),
            TimerLink::Hotkeys if self.profile.split_hotkey.is_empty() => {
                "can't start: the profile has no split hotkey".into()
            }
            TimerLink::Hotkeys if self.hotkeys.listening_failed() => {
                format!("can't start: hotkeys don't work ({see_log})")
            }
            TimerLink::Hotkeys => format!("ready: press {} to start", self.profile.split_hotkey),
            #[cfg(target_os = "linux")]
            TimerLink::Molok if !self.profile.split_hotkey.is_empty() => {
                format!("ready: press {} to start", self.profile.split_hotkey)
            }
            #[cfg(target_os = "linux")]
            TimerLink::Molok if self.windowed => "ready: press Start Auto Splitter to start".into(),
            #[cfg(target_os = "linux")]
            TimerLink::Molok => "can't start: the profile has no split hotkey or start image".into(),
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
