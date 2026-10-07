//! The window: AutoSplit's main window and settings, driving a [`Session`] on a thread of its
//! own. The window owns the profile; the session gets a copy after every change, and tells the
//! window about the changes it makes itself.

pub mod document;
pub mod log;
pub mod messages;
pub mod recorder;
pub mod settings_form;
pub mod view;

pub mod ui {
    slint::include_modules!();
}

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use autosplit_capture::{DeviceInfo, list_devices};
use autosplit_core::commands::{Command, TimerLink};
use autosplit_core::hotkeys::HotkeyAction;
use autosplit_core::profile::UserProfile;
use autosplit_input::{Modifiers, UsLayout, hotkey_name, parse_hotkey, same_hotkey};
use slint::winit_030::winit::event::{ElementState, KeyEvent, WindowEvent};
use slint::winit_030::winit::keyboard::{Key, NamedKey};
use slint::winit_030::winit::window::WindowLevel;
use slint::winit_030::{EventResult, WinitWindowAccessor};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use self::document::{Document, folder_text, profile_on_open};
use self::log::LogLines;
use self::messages::{Answer, Message, MessageQueue};
use self::recorder::{key_character, modifiers_held, pressed_key};
use self::settings_form::SettingEdit;
use self::ui::{AboutWindow, HotkeyRow, LogWindow, MainWindow, MessageWindow, Setting, SettingsWindow, WindowFocus};
use self::view::{GuiView, Picture, Update};
use crate::console::Console;
use crate::session::{Event, Notice, Session, UiCommand};
use crate::{livesplit, platform, profile};

/// How the GUI was started.
pub struct Options {
    /// By LiveSplit's AutoSplit Integration component.
    pub auto_controlled: bool,
    pub minimized: bool,
}

#[derive(Debug)]
pub enum Error {
    /// No window could be opened, before anything else started.
    NoWindow(anyhow::Error),
    Failed(anyhow::Error),
}

impl Error {
    pub fn into_inner(self) -> anyhow::Error {
        match self {
            Self::NoWindow(error) | Self::Failed(error) => error,
        }
    }
}

/// The hotkeys in the settings window, in AutoSplit's order and with its labels.
pub const HOTKEY_ROWS: [(HotkeyAction, &str); 7] = [
    (HotkeyAction::Split, "Start / Split:"),
    (HotkeyAction::Reset, "Reset:"),
    (HotkeyAction::UndoSplit, "Undo Split:"),
    (HotkeyAction::SkipSplit, "Skip Split:"),
    (HotkeyAction::Pause, "Pause:"),
    (HotkeyAction::Screenshot, "Screenshot:"),
    (HotkeyAction::ToggleAutoResetImage, "Toggle auto\nreset image"),
];

/// Help > View Help and the settings' Tutorial button.
const TUTORIAL_URL: &str = "https://github.com/TorjeAmundsen/AutoSplit-rs/blob/main/TUTORIAL.md";
const IMAGE_SETTINGS_URL: &str =
    "https://github.com/TorjeAmundsen/AutoSplit-rs/blob/main/TUTORIAL.md#split-image-file-names";
/// What to do once the profile is saved.
type AfterSave = Box<dyn FnOnce(&Rc<Gui>)>;

/// How often the main window is checked for being minimized.
const VISIBILITY_INTERVAL: Duration = Duration::from_millis(500);
/// AutoSplit's largest capture region position.
const MAX_POSITION: i64 = 999_999_999;
/// How long log lines are gathered before the log window shows them.
const LOG_DELAY: Duration = Duration::from_millis(100);

/// The capture devices in the settings window.
#[derive(Default)]
struct Devices {
    list: Vec<DeviceInfo>,
    scanning: bool,
    /// Counts listings, so an older one finishing late is ignored.
    generation: u64,
}

struct State {
    document: Document,
    devices: Devices,
    /// The hotkey Set Hotkey is waiting for a key for.
    recording: Option<HotkeyAction>,
    /// How the session tells the timer, which stays the same until a restart.
    link: TimerLink,
    running: bool,
    capturing: bool,
    can_start: bool,
    log: LogLines,
    messages: MessageQueue,
}

struct Gui {
    windows: Windows,
    /// For timers and callbacks to reach the GUI without keeping it alive.
    me: Weak<Gui>,
    hotkeys: Rc<VecModel<HotkeyRow>>,
    state: RefCell<State>,
    events: mpsc::Sender<Event>,
    /// Whether the main window can be seen, for the live preview.
    visible: Arc<AtomicBool>,
    occluded: Cell<bool>,
    zero_size: Cell<bool>,
    visibility_check: slint::Timer,
    log_flush: slint::Timer,
    log_dirty: Cell<bool>,
}

thread_local! {
    static GUI: RefCell<Option<Rc<Gui>>> = const { RefCell::new(None) };
}

/// Runs `action` with the GUI, on the window's thread.
fn with_gui(action: impl FnOnce(&Rc<Gui>) + Send + 'static) {
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(gui) = GUI.with(|gui| gui.borrow().clone()) {
            action(&gui);
        }
    });
}

/// The folder next to the executable, where AutoSplit looks for a profile on open.
fn exe_folder() -> PathBuf {
    std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_owned)).unwrap_or_else(|| ".".into())
}

/// The session's thread. Dropping it stops the session and waits for it.
struct SessionThread {
    events: mpsc::Sender<Event>,
    thread: Option<std::thread::JoinHandle<Result<(), String>>>,
}

impl SessionThread {
    fn finish(mut self) -> Result<(), String> {
        let _ = self.events.send(Event::Stop);
        self.thread.take().map_or(Ok(()), |thread| thread.join().unwrap_or_else(|_| Err("it panicked".into())))
    }
}

impl Drop for SessionThread {
    fn drop(&mut self) {
        let _ = self.events.send(Event::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown error".into())
}

/// The windows: the main window, settings and the small ones.
struct Windows {
    main: MainWindow,
    settings: SettingsWindow,
    about: AboutWindow,
    log: LogWindow,
    message: MessageWindow,
}

/// Draws on the CPU, which only redraws what changed: the preview and a few labels.
fn open_windows() -> anyhow::Result<Windows> {
    slint::BackendSelector::new().backend_name("winit".into()).renderer_name("software".into()).select()?;
    Ok(Windows {
        main: MainWindow::new()?,
        settings: SettingsWindow::new()?,
        about: AboutWindow::new()?,
        log: LogWindow::new()?,
        message: MessageWindow::new()?,
    })
}

/// The profile to start with: the one next to the executable, or none until LiveSplit sends one.
/// Returns messages to show once the window is open.
fn open_document(options: &Options, console: &mut Console, log_path: Option<&Path>) -> (Document, Vec<Message>) {
    if options.auto_controlled {
        livesplit::log_started(console, log_path);
        return (Document::untitled(), Vec::new());
    }
    console.log(format_args!("AutoSplit {}", env!("CARGO_PKG_VERSION")));
    match profile_on_open(&exe_folder()) {
        Ok((path, profile, warnings)) => {
            console.log(format_args!("Loaded {}", path.display()));
            for warning in &warnings {
                console.warn(warning);
            }
            let messages = if warnings.is_empty() { Vec::new() } else { vec![Message::warning(warnings.join("\n\n"))] };
            (Document::new(profile, Some(path)), messages)
        }
        Err(message) => {
            console.error(&message);
            (Document::untitled(), vec![Message::error(message)])
        }
    }
}

impl SessionThread {
    /// Runs a session for `profile` on a thread of its own, shown in the window.
    fn spawn(
        profile: UserProfile,
        link: TimerLink,
        console: Console,
        visible: Arc<AtomicBool>,
        events: mpsc::Sender<Event>,
        receiver: mpsc::Receiver<Event>,
    ) -> std::io::Result<Self> {
        let session_events = events.clone();
        let thread = std::thread::Builder::new().name("session".into()).spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let deliver: view::Deliver = Box::new(|pending| {
                    with_gui(move |gui| {
                        let update = pending.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take();
                        if let Some(update) = update {
                            gui.show_update(update);
                        }
                    });
                });
                let view = GuiView::new(visible, deliver, |notice| with_gui(move |gui| gui.notice(notice)));
                let mut session = Session::new(profile, link, console, Some(Box::new(view)), session_events);
                if let Err(error) = session.connect_timer() {
                    let message = match link {
                        TimerLink::AutoControlled => format!("Couldn't talk to LiveSplit: {error:#}"),
                        _ => format!("{error:#}"),
                    };
                    session.dialog(message);
                }
                session.start();
                session.run(&receiver);
            }));
            // LiveSplit closed AutoSplit, the window was closed, or the session failed
            let _ = slint::invoke_from_event_loop(quit);
            result.map_err(|payload| panic_text(&*payload))
        })?;
        Ok(Self { events, thread: Some(thread) })
    }
}

/// Ends the event loop, which closes the program.
fn quit() {
    let _ = slint::quit_event_loop();
}

/// Opens the window, and runs until it's closed (or LiveSplit closes AutoSplit).
pub fn run(options: Options) -> Result<(), Error> {
    let windows =
        open_windows().map_err(|error| Error::NoWindow(anyhow::anyhow!("Couldn't open a window: {error}")))?;
    platform::keep_running_when_hidden();

    let (events, receiver) = mpsc::channel();
    let (mut console, log_path) = Console::with_listener(Box::new(|line| {
        let line = line.to_owned();
        with_gui(move |gui| gui.add_log_line(line));
    }));
    let (document, first_messages) = open_document(&options, &mut console, log_path.as_deref());
    let link =
        if options.auto_controlled { TimerLink::AutoControlled } else { document.profile.timer_connection.into() };

    let visible = Arc::new(AtomicBool::new(!options.minimized));
    let gui = Rc::new_cyclic(|me| Gui {
        windows,
        me: me.clone(),
        hotkeys: Rc::new(VecModel::default()),
        state: RefCell::new(State {
            document,
            devices: Devices::default(),
            recording: None,
            link,
            running: false,
            capturing: false,
            can_start: true,
            log: LogLines::default(),
            messages: MessageQueue::default(),
        }),
        events: events.clone(),
        visible: visible.clone(),
        occluded: Cell::new(false),
        zero_size: Cell::new(false),
        visibility_check: slint::Timer::default(),
        log_flush: slint::Timer::default(),
        log_dirty: Cell::new(false),
    });
    GUI.with(|cell| *cell.borrow_mut() = Some(gui.clone()));
    gui.connect();
    let windows = &gui.windows;
    windows.main.set_window_title(
        format!(
            "AutoSplit v{}{}",
            env!("CARGO_PKG_VERSION"),
            if options.auto_controlled { " (externally controlled)" } else { "" }
        )
        .into(),
    );
    windows.settings.set_hotkeys(ModelRc::from(gui.hotkeys.clone()));
    windows.settings.set_auto_controlled(options.auto_controlled);
    windows.settings.set_molok_listed(cfg!(target_os = "linux"));
    windows.about.set_version(env!("CARGO_PKG_VERSION").into());
    windows.log.set_log_path(log_path.as_ref().map(|path| path.display().to_string()).unwrap_or_default().into());
    gui.show_profile();
    windows.main.show().map_err(|error| Error::NoWindow(anyhow::anyhow!("Couldn't show the window: {error}")))?;
    if options.minimized {
        windows.main.window().set_minimized(true);
    }
    gui.watch_visibility();

    let profile = {
        let state = gui.state.borrow();
        let mut profile = state.document.profile.clone();
        profile.resolve_directories(&state.document.folder(&exe_folder()));
        profile
    };
    let session = SessionThread::spawn(profile, link, console, visible, events, receiver)
        .map_err(|error| Error::Failed(error.into()))?;

    for message in first_messages {
        gui.show_message(message);
    }
    slint::run_event_loop_until_quit().map_err(|error| Error::Failed(error.into()))?;
    // Hidden first, since stopping the session can take a moment
    gui.hide_all();
    let result = session.finish();
    GUI.with(|cell| cell.borrow_mut().take());
    result.map_err(|error| Error::Failed(anyhow::anyhow!("autosplit stopped because of an error: {error}")))
}

/// The windows' callbacks that act on the GUI. Message boxes are modal like AutoSplit's, so
/// these do nothing while one is open; `always` ones still run. An `else` value is what the
/// callback returns.
macro_rules! on {
    ($weak:expr, $window:expr, $callback:ident, ($($argument:ident),*) $gui:ident => $body:expr) => {{
        let weak = $weak.clone();
        $window.$callback(move |$($argument),*| {
            if let Some($gui) = weak.upgrade() {
                if $gui.blocked() {
                    return $gui.focus_message();
                }
                $body;
            }
        });
    }};
    (always $weak:expr, $window:expr, $callback:ident, ($($argument:ident),*) $gui:ident => $body:block else $result:expr) => {{
        let weak = $weak.clone();
        $window.$callback(move |$($argument),*| {
            if let Some($gui) = weak.upgrade() {
                $body;
            }
            $result
        });
    }};
    (always $weak:expr, $window:expr, $callback:ident, ($($argument:ident),*) $gui:ident => $body:expr) => {{
        let weak = $weak.clone();
        $window.$callback(move |$($argument),*| {
            if let Some($gui) = weak.upgrade() {
                $body;
            }
        });
    }};
}

impl Gui {
    fn send(&self, command: UiCommand) {
        let _ = self.events.send(Event::Ui(command));
    }

    /// Sends the profile to the session, which reads its relative folders from the profile's
    /// folder. `loaded` when it was just loaded from a file.
    fn send_profile(&self, loaded: bool) {
        let state = self.state.borrow();
        let profile = Box::new(state.document.profile.clone());
        let folder = state.document.folder(&exe_folder());
        drop(state);
        self.send(UiCommand::SetProfile { profile, folder, loaded });
    }

    /// The profile changed in a way the windows show elsewhere too.
    fn profile_edited(&self) {
        self.send_profile(false);
        self.show_profile();
    }

    fn blocked(&self) -> bool {
        self.state.borrow().messages.is_showing()
    }

    fn connect(self: &Rc<Self>) {
        self.connect_main();
        self.connect_settings();
        self.connect_small_windows();
        self.connect_window_events();
    }

    fn connect_main(&self) {
        let weak = self.me.clone();
        let main = &self.windows.main;
        on!(weak, main, on_browse_folder, () gui => gui.browse_folder());
        on!(weak, main, on_region_edited, (x, y, width, height) gui => {
            let mut state = gui.state.borrow_mut();
            let region = &mut state.document.profile.capture_region;
            // Spin boxes hold f32, which rounds large whole numbers; AutoSplit's limit stays the limit
            let whole = |value: f32| (value.round() as i64).min(MAX_POSITION);
            (region.x, region.y, region.width, region.height) = (whole(x), whole(y), whole(width), whole(height));
            drop(state);
            gui.send_profile(false);
        });
        on!(weak, main, on_align_region, () gui => gui.align_region());
        on!(weak, main, on_take_screenshot, () gui => gui.send(UiCommand::Screenshot));
        on!(weak, main, on_start_auto_splitter, () gui => gui.send(UiCommand::StartAutoSplitter));
        on!(weak, main, on_reset, () gui => gui.send(UiCommand::Reset));
        on!(weak, main, on_skip_split, () gui => gui.send(UiCommand::SkipSplit));
        on!(weak, main, on_undo_split, () gui => gui.send(UiCommand::UndoSplit));
        on!(weak, main, on_next_image, () gui => gui.send(UiCommand::NextImage));
        on!(weak, main, on_previous_image, () gui => gui.send(UiCommand::PreviousImage));
        on!(weak, main, on_reload_start_image, () gui => gui.send(UiCommand::ReloadStartImage));
        on!(weak, main, on_open_settings, () gui => gui.open_settings());
        on!(weak, main, on_save_profile, () gui => gui.save_profile(false, None));
        on!(weak, main, on_save_profile_as, () gui => gui.save_profile(true, None));
        on!(weak, main, on_load_profile, () gui => gui.load_profile());
        on!(weak, main, on_view_help, () gui => gui.open(TUTORIAL_URL));
        on!(weak, main, on_view_log, () gui => gui.show_log());
        on!(weak, main, on_about, () gui => gui.show_window(gui.windows.about.window()));
        on!(always weak, main, on_blocked_clicked, () gui => gui.focus_message());
        on!(always weak, main.window(), on_close_requested, () gui => {
            gui.close_requested();
        } else slint::CloseRequestResponse::KeepWindowShown);
    }

    fn connect_settings(&self) {
        let weak = self.me.clone();
        let settings = &self.windows.settings;
        on!(weak, settings, on_changed, (setting) gui => gui.setting_changed(setting));
        on!(weak, settings, on_capture_devices_requested, () gui => gui.refresh_devices());
        on!(weak, settings, on_browse_screenshot_directory, () gui => gui.browse_screenshot_directory());
        on!(weak, settings, on_set_hotkey, (index) gui => {
            let Some(&(action, _)) = HOTKEY_ROWS.get(index as usize) else { return };
            let recording = gui.state.borrow().recording;
            gui.record_hotkey((recording != Some(action)).then_some(action));
        });
        let typed = weak.clone();
        settings.on_hotkey_typed(move |index, text| {
            let Some(gui) = typed.upgrade() else { return false };
            let Some(&(action, _)) = HOTKEY_ROWS.get(index as usize) else { return false };
            gui.hotkey_typed(action, &text)
        });
        on!(weak, settings, on_open_readme, () gui => gui.open(IMAGE_SETTINGS_URL));
        on!(always weak, settings, on_blocked_clicked, () gui => gui.focus_message());
        on!(always weak, settings.window(), on_close_requested, () gui => {
            gui.record_hotkey(None);
        } else slint::CloseRequestResponse::HideWindow);
    }

    fn connect_small_windows(&self) {
        let weak = self.me.clone();
        let (about, log, message) = (&self.windows.about, &self.windows.log, &self.windows.message);
        on!(weak, about, on_ok, () gui => {
            let _ = gui.windows.about.hide();
        });
        on!(weak, about, on_open_link, (link) gui => gui.open(link.as_str()));
        on!(weak, log, on_open_log_file, () gui => {
            let path = gui.windows.log.get_log_path();
            if !path.is_empty() {
                gui.open(path.as_str());
            }
        });
        on!(always weak, message, on_answered, (button) gui => gui.message_answered(Some(button as usize)));
        on!(always weak, message, on_dismissed, () gui => gui.message_answered(None));
        on!(always weak, message.window(), on_close_requested, () gui => {
            gui.message_answered(None);
        } else slint::CloseRequestResponse::KeepWindowShown);
    }

    /// Key presses for Set Hotkey and AutoSplit's shortcuts, whether the windows are active, and
    /// whether the live preview can be seen.
    fn connect_window_events(&self) {
        // Set Hotkey reads the next key pressed while the settings window has focus. Otherwise
        // AutoSplit's shortcuts work there too; the main window's menu has them.
        let weak = self.me.clone();
        let mut held = Modifiers::empty();
        self.windows.settings.window().on_winit_window_event(move |_, event| {
            let Some(gui) = weak.upgrade() else { return EventResult::Propagate };
            match event {
                WindowEvent::ModifiersChanged(modifiers) => held = modifiers_held(modifiers.state()),
                WindowEvent::Focused(focused) => {
                    gui.windows.settings.global::<WindowFocus>().set_active(*focused);
                    if !focused {
                        gui.record_hotkey(None);
                    }
                }
                WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                    let recording = gui.state.borrow().recording;
                    if let Some(action) = recording {
                        gui.key_recorded(action, event, held);
                        return EventResult::PreventDefault;
                    }
                    if gui.shortcut(event, held) {
                        return EventResult::PreventDefault;
                    }
                }
                _ => {}
            }
            EventResult::Propagate
        });

        let weak = self.me.clone();
        self.windows.main.window().on_winit_window_event(move |_, event| {
            let Some(gui) = weak.upgrade() else { return EventResult::Propagate };
            match event {
                WindowEvent::Focused(focused) => gui.windows.main.global::<WindowFocus>().set_active(*focused),
                WindowEvent::Occluded(occluded) => {
                    gui.occluded.set(*occluded);
                    gui.update_visibility();
                }
                // Windows minimizes to a size of 0
                WindowEvent::Resized(size) => {
                    gui.zero_size.set(size.width == 0 || size.height == 0);
                    gui.update_visibility();
                }
                _ => {}
            }
            EventResult::Propagate
        });
    }

    /// Checks now and then whether the main window can be seen, since not every system tells
    /// when it's minimized (X11) or covered (Windows).
    fn watch_visibility(&self) {
        let weak = self.me.clone();
        self.visibility_check.start(slint::TimerMode::Repeated, VISIBILITY_INTERVAL, move || {
            if let Some(gui) = weak.upgrade() {
                gui.update_visibility();
            }
        });
    }

    fn update_visibility(&self) {
        let minimized =
            self.windows.main.window().with_winit_window(|window| window.is_minimized()).flatten().unwrap_or(false);
        let visible = !minimized && !self.zero_size.get() && !self.occluded.get();
        self.visible.store(visible, Ordering::Relaxed);
    }

    fn hide_all(&self) {
        let windows = &self.windows;
        for window in [
            windows.main.window(),
            windows.settings.window(),
            windows.about.window(),
            windows.log.window(),
            windows.message.window(),
        ] {
            let _ = window.hide();
        }
    }

    /// AutoSplit's keyboard shortcuts. Returns whether the key was one.
    fn shortcut(self: &Rc<Self>, event: &KeyEvent, held: Modifiers) -> bool {
        if self.blocked() {
            return false;
        }
        let control = held.contains(Modifiers::CONTROL);
        let shift = held.contains(Modifiers::SHIFT);
        match (control, shift, key_character(event).map(|c| c.to_ascii_lowercase()), &event.logical_key) {
            (true, _, Some(','), _) => self.open_settings(),
            (true, true, Some('s'), _) => self.save_profile(true, None),
            (true, false, Some('s'), _) => self.save_profile(false, None),
            (true, _, Some('o'), _) => self.load_profile(),
            (false, _, _, Key::Named(NamedKey::F1)) => self.open(TUTORIAL_URL),
            _ => return false,
        }
        true
    }

    /// Starts or stops waiting for a key for a hotkey.
    fn record_hotkey(&self, action: Option<HotkeyAction>) {
        self.state.borrow_mut().recording = action;
        let index = action.and_then(|action| HOTKEY_ROWS.iter().position(|&(row, _)| row == action));
        self.windows.settings.set_setting_hotkey(index.map_or(-1, |index| index as i32));
        self.show_buttons();
    }

    /// A key pressed while Set Hotkey waits for one.
    fn key_recorded(&self, action: HotkeyAction, event: &KeyEvent, held: Modifiers) {
        // Escape clears the hotkey, like in LiveSplit
        if event.logical_key == Key::Named(NamedKey::Escape) {
            return self.assign_hotkey(action, "");
        }
        let Some(key) = pressed_key(event) else { return };
        // A modifier on its own: wait for the key it goes with
        let Some(name) = hotkey_name(key, key_character(event), held) else { return };
        self.assign_hotkey(action, &name);
    }

    /// A hotkey name typed in its field. Returns whether it's a hotkey, like AutoSplit checks.
    fn hotkey_typed(&self, action: HotkeyAction, text: &str) -> bool {
        let name = text.trim();
        if action.hotkey(&self.state.borrow().document.profile) == name {
            return true;
        }
        if !name.is_empty()
            && let Err(error) = parse_hotkey(name, &UsLayout)
        {
            self.show_message(Message::error(format!("Invalid hotkey '{name}': {error}")));
            return false;
        }
        self.assign_hotkey(action, name);
        true
    }

    fn assign_hotkey(&self, action: HotkeyAction, name: &str) {
        action.assign(&mut self.state.borrow_mut().document.profile, name, same_hotkey);
        self.record_hotkey(None);
        self.profile_edited();
    }

    fn open_settings(&self) {
        self.show_profile();
        self.refresh_devices();
        self.show_window(self.windows.settings.window());
    }

    fn show_window(&self, window: &slint::Window) {
        if let Err(error) = window.show() {
            self.add_log_line(format!("error: Couldn't show a window: {error}"));
        }
        // Wayland only maps a window once it draws, and the windows got their first redraw
        // while hidden at startup, so the first show would wait for some other redraw
        window.request_redraw();
    }

    /// Lists capture devices in the background, since some take a while to answer.
    fn refresh_devices(&self) {
        let mut state = self.state.borrow_mut();
        state.devices.generation += 1;
        state.devices.scanning = true;
        let generation = state.devices.generation;
        drop(state);
        self.show_profile();
        std::thread::spawn(move || {
            let list = list_devices().unwrap_or_default();
            with_gui(move |gui| {
                let mut state = gui.state.borrow_mut();
                if state.devices.generation == generation {
                    let picked = settings_form::pick_first_device(&mut state.document.profile, &list);
                    state.devices = Devices { list, scanning: false, generation };
                    drop(state);
                    if picked {
                        gui.send_profile(false);
                    }
                    gui.show_profile();
                }
            });
        });
    }

    /// Puts the profile's values in the windows.
    fn show_profile(&self) {
        let state = self.state.borrow();
        let profile = &state.document.profile;
        let main = &self.windows.main;
        main.set_split_image_folder(if profile.split_image_directory.is_empty() {
            "No Folder Selected".into()
        } else {
            // AutoSplit shows the folder with a slash at the end
            format!("{}/", profile.split_image_directory.trim_end_matches(['/', '\\'])).into()
        });
        let region = profile.capture_region;
        main.set_region_x(region.x as f32);
        main.set_region_y(region.y as f32);
        main.set_region_width(region.width as f32);
        main.set_region_height(region.height as f32);

        let settings = &self.windows.settings;
        settings.set_fps_limit(profile.fps_limit as f32);
        settings.set_live_capture_region(profile.live_capture_region);
        let shown = settings_form::Shown::new(profile, &state.devices.list, state.devices.scanning, state.link);
        let strings = |items: Vec<String>| ModelRc::new(VecModel::from_iter(items.into_iter().map(SharedString::from)));
        settings.set_capture_methods(strings(shown.capture_methods));
        settings.set_capture_method_index(shown.capture_method_index);
        settings.set_capture_device_placeholder(shown.device_placeholder.into());
        settings.set_capture_devices(strings(shown.device_names));
        settings.set_capture_device_index(shown.device_index);
        settings.set_screenshot_directory(profile.screenshot_directory.as_str().into());
        settings.set_open_screenshot(profile.open_screenshot);
        let on = |command| profile.screenshot_on.contains(&command);
        settings.set_screenshot_on_start(on(Command::Start));
        settings.set_screenshot_on_split(on(Command::Split));
        settings.set_screenshot_on_reset(on(Command::Reset));
        settings.set_screenshot_on_skip(on(Command::Skip));
        settings.set_screenshot_on_undo(on(Command::Undo));
        settings.set_screenshot_on_pause(on(Command::Pause));
        settings.set_default_comparison_method(profile.default_comparison_method.clamp(0, 2) as i32);
        settings.set_default_similarity_threshold(profile.default_similarity_threshold as f32);
        settings.set_default_delay_time(profile.default_delay_time as f32);
        settings.set_default_pause_time(profile.default_pause_time as f32);
        settings.set_loop_splits(profile.loop_splits);
        settings.set_start_also_resets(profile.start_also_resets);
        settings.set_enable_auto_reset(profile.enable_auto_reset);
        for (index, &(action, name)) in HOTKEY_ROWS.iter().enumerate() {
            let row = HotkeyRow {
                name: name.into(),
                hotkey: action.hotkey(profile).into(),
                enabled: action.listened(state.link),
            };
            // Replacing a row that didn't change would take the focus from its field
            if self.hotkeys.row_data(index).as_ref() != Some(&row) {
                if index < self.hotkeys.row_count() {
                    self.hotkeys.set_row_data(index, row);
                } else {
                    self.hotkeys.push(row);
                }
            }
        }
        settings.set_hotkeys_note(shown.hotkeys_note.into());
        settings.set_timer_connection(shown.timer_connection);
        settings.set_livesplit_one_port(f32::from(profile.livesplit_one_port));
        settings.set_timer_connection_waiting(shown.timer_connection_waiting);
    }

    /// A setting the settings window changed.
    fn setting_changed(&self, setting: Setting) {
        let settings = &self.windows.settings;
        let edit = match setting {
            Setting::FpsLimit => SettingEdit::FpsLimit(settings.get_fps_limit().round() as i32),
            Setting::LiveCaptureRegion => SettingEdit::LiveCaptureRegion(settings.get_live_capture_region()),
            Setting::CaptureMethod => SettingEdit::CaptureMethod(settings.get_capture_method_index()),
            Setting::CaptureDevice => SettingEdit::CaptureDevice(settings.get_capture_device_index()),
            Setting::OpenScreenshot => SettingEdit::OpenScreenshot(settings.get_open_screenshot()),
            Setting::ScreenshotOn => SettingEdit::ScreenshotOn {
                start: settings.get_screenshot_on_start(),
                split: settings.get_screenshot_on_split(),
                reset: settings.get_screenshot_on_reset(),
                skip: settings.get_screenshot_on_skip(),
                undo: settings.get_screenshot_on_undo(),
                pause: settings.get_screenshot_on_pause(),
            },
            Setting::DefaultComparisonMethod => {
                SettingEdit::DefaultComparisonMethod(settings.get_default_comparison_method())
            }
            Setting::DefaultSimilarityThreshold => {
                SettingEdit::DefaultSimilarityThreshold(settings.get_default_similarity_threshold().into())
            }
            Setting::DefaultDelayTime => SettingEdit::DefaultDelayTime(settings.get_default_delay_time().into()),
            Setting::DefaultPauseTime => SettingEdit::DefaultPauseTime(settings.get_default_pause_time().into()),
            Setting::LoopSplits => SettingEdit::LoopSplits(settings.get_loop_splits()),
            Setting::StartAlsoResets => SettingEdit::StartAlsoResets(settings.get_start_also_resets()),
            Setting::EnableAutoReset => SettingEdit::EnableAutoReset(settings.get_enable_auto_reset()),
            Setting::TimerConnection => SettingEdit::TimerConnection(settings.get_timer_connection()),
            Setting::LivesplitOnePort => SettingEdit::LiveSplitOnePort(settings.get_livesplit_one_port().into()),
        };
        let refresh = edit.changes_other_fields();
        let mut state = self.state.borrow_mut();
        let State { document, devices, .. } = &mut *state;
        let applied = settings_form::apply(&mut document.profile, edit, &devices.list);
        drop(state);
        if applied {
            self.send_profile(false);
        }
        if refresh {
            self.show_profile();
        }
    }

    /// Something the session did by itself, or wants shown.
    fn notice(&self, notice: Notice) {
        match notice {
            Notice::Dialog(message) => self.show_message(Message::error(message)),
            Notice::RegionAligned(region) => {
                self.state.borrow_mut().document.profile.capture_region = region;
                // Sent back, so it isn't lost to an edit the session got in the meantime
                self.profile_edited();
            }
            Notice::AutoResetToggled(on) => {
                self.state.borrow_mut().document.profile.enable_auto_reset = on;
                self.profile_edited();
            }
            Notice::TimerLinkChanged(link) => {
                self.state.borrow_mut().link = link;
                self.show_profile();
            }
            Notice::ProfileLoaded { path, profile } => {
                self.state.borrow_mut().document = Document::new(*profile, Some(path));
                self.profile_edited();
            }
        }
    }

    /// What the session shows changed.
    fn show_update(&self, update: Update) {
        let main = &self.windows.main;
        if let Some(texts) = update.texts {
            main.set_running(texts.running);
            self.windows.settings.set_running(texts.running);
            main.set_device_name(if texts.device_name.is_empty() { "-".into() } else { texts.device_name.into() });
            main.set_live_message(texts.live_message.into());
            let window = texts.window;
            main.set_current_image_file(window.current_file.into());
            main.set_current_image_message(window.current_message.into());
            let [current_live, current_highest, current_threshold, reset_live, reset_highest, reset_threshold] =
                window.table;
            main.set_current_live(current_live.into());
            main.set_current_highest(current_highest.into());
            main.set_current_threshold(current_threshold.into());
            main.set_reset_live(reset_live.into());
            main.set_reset_highest(reset_highest.into());
            main.set_reset_threshold(reset_threshold.into());
            main.set_image_loop(window.image_loop.into());
            main.set_start_image_status(window.start_status.into());
            let [start, reset, skip, undo, next, previous] = texts.buttons;
            main.set_can_reset(reset);
            main.set_can_skip(skip);
            main.set_can_undo(undo);
            main.set_can_next(next);
            main.set_can_previous(previous);
            let mut state = self.state.borrow_mut();
            state.running = texts.running;
            state.capturing = texts.capturing;
            state.can_start = start;
            drop(state);
            self.show_buttons();
        }
        let image = |picture| match picture {
            Picture::Keep => None,
            Picture::Clear => Some(slint::Image::default()),
            Picture::Show(pixels) => Some(slint::Image::from_rgba8(pixels)),
        };
        if let Some(live) = image(update.live) {
            main.set_live_image(live);
        }
        if let Some(current) = image(update.current) {
            main.set_current_image(current);
        }
    }

    /// Start Auto Splitter, which AutoSplit disables while Set Hotkey waits for a key.
    fn show_buttons(&self) {
        let state = self.state.borrow();
        self.windows.main.set_can_start(state.can_start && state.recording.is_none());
    }

    fn browse_folder(self: &Rc<Self>) {
        let state = self.state.borrow();
        let current = &state.document.profile.split_image_directory;
        // AutoSplit starts in the folder above the current one
        let start = if current.is_empty() {
            exe_folder()
        } else {
            let folder = state.document.folder(&exe_folder()).join(current);
            folder.parent().map_or(folder.clone(), Path::to_owned)
        };
        drop(state);
        let dialog = self.file_dialog("Select Split Image Directory").set_directory(start).pick_folder();
        self.when_picked(dialog, |gui, folder| {
            gui.state.borrow_mut().document.profile.split_image_directory = folder_text(&folder);
            gui.profile_edited();
        });
    }

    fn browse_screenshot_directory(self: &Rc<Self>) {
        // AutoSplit starts in the screenshot folder, or else the split image folder
        let state = self.state.borrow();
        let profile = &state.document.profile;
        let current = if profile.screenshot_directory.is_empty() {
            &profile.split_image_directory
        } else {
            &profile.screenshot_directory
        };
        let start = state.document.folder(&exe_folder()).join(current);
        drop(state);
        let dialog = self.file_dialog("Select Screenshots Directory").set_directory(start).pick_folder();
        self.when_picked(dialog, |gui, folder| {
            gui.state.borrow_mut().document.profile.screenshot_directory = folder_text(&folder);
            gui.profile_edited();
        });
    }

    fn align_region(self: &Rc<Self>) {
        let state = self.state.borrow();
        // Like AutoSplit, before asking for the image
        if !state.capturing {
            drop(state);
            return self.show_message(Message::error("There's no capture to align the region in."));
        }
        let folder = state.document.folder(&exe_folder()).join(&state.document.profile.split_image_directory);
        drop(state);
        let dialog = self
            .file_dialog("Select Reference Image")
            .set_directory(folder)
            .add_filter("PNG image", &["png"])
            .pick_file();
        self.when_picked(dialog, |gui, path| gui.send(UiCommand::AlignRegion(path)));
    }

    /// A file dialog in front of the main window.
    fn file_dialog(&self, title: &str) -> rfd::AsyncFileDialog {
        rfd::AsyncFileDialog::new().set_title(title).set_parent(&self.windows.main.window().window_handle())
    }

    /// Waits for a file dialog without blocking the window, then does `then` with the path.
    fn when_picked(
        self: &Rc<Self>,
        dialog: impl Future<Output = Option<rfd::FileHandle>> + 'static,
        then: impl FnOnce(&Rc<Gui>, PathBuf) + 'static,
    ) {
        let weak = Rc::downgrade(self);
        let waiting = slint::spawn_local(async move {
            if let Some(file) = dialog.await
                && let Some(gui) = weak.upgrade()
            {
                then(&gui, file.path().to_owned());
            }
        });
        if let Err(error) = waiting {
            self.add_log_line(format!("error: Couldn't open a file dialog: {error}"));
        }
    }

    /// AutoSplit's Save Profile: to the loaded file, or asks for one. Then does `saved`.
    fn save_profile(self: &Rc<Self>, ask: bool, saved: Option<AfterSave>) {
        let state = self.state.borrow();
        let path = state.document.path.clone();
        drop(state);
        if let Some(path) = path.as_ref().filter(|_| !ask) {
            return self.save_to(path.clone(), saved);
        }
        let (folder, name) = match &path {
            Some(path) => (
                path.parent().map_or_else(exe_folder, Path::to_owned),
                path.file_name().map_or("settings.toml".into(), |name| name.to_string_lossy().into_owned()),
            ),
            None => (exe_folder(), "settings.toml".to_owned()),
        };
        let dialog = self
            .file_dialog("Save Settings As")
            .set_directory(folder)
            .set_file_name(name)
            .add_filter("TOML", &["toml"])
            .save_file();
        self.when_picked(dialog, |gui, path| gui.save_to(path, saved));
    }

    fn save_to(self: &Rc<Self>, path: PathBuf, saved: Option<AfterSave>) {
        let profile = self.state.borrow().document.to_save();
        if let Err(error) = profile.save(&path) {
            return self.show_message(Message::error(format!("Couldn't save {}: {error}", path.display())));
        }
        self.state.borrow_mut().document.saved(path, profile);
        // Relative folders are read from the new file's folder
        self.profile_edited();
        if let Some(saved) = saved {
            saved(self);
        }
    }

    fn load_profile(self: &Rc<Self>) {
        let folder = self.state.borrow().document.folder(&exe_folder());
        let dialog = self.file_dialog("Load Profile").set_directory(folder).add_filter("TOML", &["toml"]).pick_file();
        self.when_picked(dialog, |gui, path| {
            let (profile, warnings) = match profile::load_raw(&path) {
                Ok(loaded) => loaded,
                Err(error) => return gui.show_message(Message::error(format!("{error:#}"))),
            };
            gui.add_log_line(format!("Loaded {}", path.display()));
            if !warnings.is_empty() {
                gui.show_message(Message::warning(warnings.join("\n\n")));
            }
            gui.state.borrow_mut().document = Document::new(profile, Some(path));
            gui.send_profile(true);
            gui.show_profile();
        });
    }

    /// Opens a link or file with its app.
    fn open(&self, target: &str) {
        if let Err(error) = platform::open(target) {
            self.show_message(Message::error(format!("Couldn't open {target}: {error}")));
        }
    }

    /// AutoSplit asks whether to save changes before closing.
    fn close_requested(self: &Rc<Self>) {
        if self.blocked() {
            return self.focus_message();
        }
        let state = self.state.borrow();
        if !state.document.has_changes() {
            drop(state);
            return quit();
        }
        let name = state.document.name();
        drop(state);
        let gui = Rc::downgrade(self);
        self.show_message(Message::question(
            format!("Do you want to save changes made to settings file {name}?"),
            move |answer| {
                let Some(gui) = gui.upgrade() else { return };
                match answer {
                    Answer::Yes => gui.save_profile(false, Some(Box::new(|_| quit()))),
                    Answer::No => quit(),
                    Answer::Cancel => {}
                }
            },
        ));
    }

    fn add_log_line(&self, line: String) {
        self.state.borrow_mut().log.push(line);
        self.log_dirty.set(true);
        // Gathered for a moment, and only shown while the log window is open
        if self.windows.log.window().is_visible() && !self.log_flush.running() {
            let weak = self.me.clone();
            self.log_flush.start(slint::TimerMode::SingleShot, LOG_DELAY, move || {
                if let Some(gui) = weak.upgrade() {
                    gui.flush_log();
                }
            });
        }
    }

    fn flush_log(&self) {
        if self.log_dirty.replace(false) {
            self.windows.log.set_log(self.state.borrow().log.text().into());
        }
    }

    fn show_log(&self) {
        self.flush_log();
        self.show_window(self.windows.log.window());
    }

    fn show_message(&self, message: Message) {
        let shown = self.state.borrow_mut().messages.push(message);
        if let Some(shown) = shown {
            self.present_message(shown);
        }
    }

    fn present_message(&self, (text, buttons): messages::Shown) {
        let [primary, secondary, tertiary] = buttons.labels();
        self.windows.message.set_window_title(buttons.title().into());
        self.windows.message.set_message(text.into());
        self.windows.message.set_primary(primary.into());
        self.windows.message.set_secondary(secondary.into());
        self.windows.message.set_tertiary(tertiary.into());
        self.windows.main.set_blocked(true);
        self.windows.settings.set_blocked(true);
        self.show_window(self.windows.message.window());
        self.windows.message.window().with_winit_window(|window| window.set_window_level(WindowLevel::AlwaysOnTop));
        self.windows.message.invoke_focus_keys();
        self.focus_message();
    }

    fn focus_message(&self) {
        self.windows.message.window().with_winit_window(|window| window.focus_window());
    }

    /// A message box was answered with a button, or closed.
    fn message_answered(&self, button: Option<usize>) {
        let _ = self.windows.message.hide();
        let (answered, next) = self.state.borrow_mut().messages.answer(button);
        match next {
            Some(next) => self.present_message(next),
            None => {
                self.windows.main.set_blocked(false);
                self.windows.settings.set_blocked(false);
            }
        }
        if let Some((answered, answer)) = answered {
            answered(answer);
        }
    }
}
