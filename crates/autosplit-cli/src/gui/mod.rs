//! The window: AutoSplit's main window and settings, driving a [`Session`] on a thread of its
//! own. The window owns the profile; the session gets a copy after every change, and tells the
//! window about the changes it makes itself.

pub mod document;
pub mod log;
pub mod maker_plan;
pub mod messages;
pub mod paths;
pub mod placement;
pub mod preset_form;
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
use std::time::{Duration, Instant};

use autosplit_capture::{DeviceInfo, list_devices};
use autosplit_core::commands::{Command, TimerLink};
use autosplit_core::hotkeys::HotkeyAction;
use autosplit_core::profile::UserProfile;
use autosplit_core::split_folder::{MAX_GROUP_SIZE, user_files};
use autosplit_input::{Modifiers, hotkey_name, parse_hotkey, same_hotkey, system_layout};
use slint::winit_030::winit::event::{ElementState, KeyEvent, WindowEvent};
use slint::winit_030::winit::keyboard::{Key, NamedKey};
use slint::winit_030::winit::window::WindowLevel;
use slint::winit_030::{EventResult, WinitWindowAccessor};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use self::document::{Document, folder_text, profile_on_open};
use self::log::LogLines;
use self::messages::{Answer, Message, MessageQueue};
use self::placement::{Area, centered_over};
use self::recorder::{PRIMARY, key_character, modifiers_held, pressed_key};
use self::settings_form::SettingEdit;
use self::ui::{
    AboutWindow, HotkeyRow, InfoCell, InlineMessage, InstructionsWindow, LogWindow, MainWindow, MakerRow, MakerWindow,
    MessageWindow, NameKind, NamePart, Setting, SettingsWindow, SimilarityRow, TesterWindow, WindowFocus,
};
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

/// Help > View Help and the settings' Tutorial button, at this version's release tag so it
/// describes the program that's running.
const TUTORIAL_URL: &str =
    concat!("https://github.com/TorjeAmundsen/AutoSplit-rs/blob/v", env!("CARGO_PKG_VERSION"), "/TUTORIAL.md");
const IMAGE_SETTINGS_URL: &str = concat!(
    "https://github.com/TorjeAmundsen/AutoSplit-rs/blob/v",
    env!("CARGO_PKG_VERSION"),
    "/MANUAL.md#split-image-file-names"
);
/// What to do once the profile is saved.
type AfterSave = Box<dyn FnOnce(&Rc<Gui>)>;

/// How often the main window is checked for being minimized.
const VISIBILITY_INTERVAL: Duration = Duration::from_millis(500);
/// How long a window under Wayland can go without a frame it asked for before it counts as
/// minimized or covered. Compositors send one at the screen's refresh rate while it's seen.
const FRAME_STALL: Duration = Duration::from_secs(1);
/// AutoSplit's largest capture region position.
const MAX_POSITION: i64 = 999_999_999;
/// How long messages from opening wait for the main window's focus, which a minimized one never
/// gets.
const FIRST_MESSAGES_FALLBACK: Duration = Duration::from_secs(2);
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
    can_start: bool,
    log: LogLines,
    messages: MessageQueue,
    /// The tool windows that are open, the newest last, which the main window can't be used
    /// beside.
    tools: Vec<Tool>,
}

/// The windows the main window can't be used beside.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tool {
    Tester,
    Maker,
}

impl Tool {
    fn title(self) -> &'static str {
        match self {
            Self::Tester => "Live Tester",
            Self::Maker => "Make Split Images",
        }
    }
}

struct Gui {
    windows: Windows,
    /// For timers and callbacks to reach the GUI without keeping it alive.
    me: Weak<Gui>,
    hotkeys: Rc<VecModel<HotkeyRow>>,
    /// The similarity viewer's rows for grouped split images.
    group_rows: Rc<VecModel<SimilarityRow>>,
    /// The current image grid's tiles.
    current_tiles: Rc<VecModel<slint::Image>>,
    /// The Live Tester's information about its image.
    tester_info: Rc<VecModel<InfoCell>>,
    /// The Live Tester's image and kept frames.
    tester_state: RefCell<tester::TesterState>,
    maker: RefCell<maker::Maker>,
    /// Make Split Images' splits.
    maker_rows: Rc<VecModel<MakerRow>>,
    /// Kept while savestates are on it: X11 serves the clipboard from the program that set it.
    clipboard: RefCell<Option<arboard::Clipboard>>,
    state: RefCell<State>,
    events: mpsc::Sender<Event>,
    /// Whether the main window can be seen, for the live preview.
    seen: Seen,
    /// The same for the Live Tester's window and Make Split Images, for theirs.
    tester_seen: Seen,
    maker_seen: Seen,
    visibility_check: slint::Timer,
    log_flush: slint::Timer,
    log_dirty: Cell<bool>,
    /// Messages from opening, shown once the main window is open in front, so they can open over
    /// it and in front of it.
    first_messages: RefCell<Vec<Message>>,
    first_messages_fallback: slint::Timer,
}

/// Whether a window can be seen, for the previews only made while one can: shown, and not
/// minimized, covered or sized to nothing. The flag is read by the session's thread.
struct Seen {
    flag: Arc<AtomicBool>,
    occluded: Cell<bool>,
    zero_size: Cell<bool>,
    /// Under Wayland, when a redraw was asked for that hasn't come yet.
    frame_asked: Cell<Option<Instant>>,
    /// Under Wayland, the compositor stopped sending the window frames.
    frames_stalled: Cell<bool>,
}

impl Seen {
    fn new(flag: Arc<AtomicBool>) -> Self {
        Self {
            flag,
            occluded: Cell::new(false),
            zero_size: Cell::new(false),
            frame_asked: Cell::new(None),
            frames_stalled: Cell::new(false),
        }
    }

    /// Notes `event` when it covers or uncovers the window, or sizes it. Returns whether it did,
    /// so the flag is worked out again.
    fn note(&self, event: &WindowEvent) -> bool {
        match event {
            WindowEvent::Occluded(occluded) => self.occluded.set(*occluded),
            // Windows minimizes to a size of 0
            WindowEvent::Resized(size) => self.zero_size.set(size.width == 0 || size.height == 0),
            // A frame came, so the window is seen again if frames had stopped
            WindowEvent::RedrawRequested => {
                self.frame_asked.set(None);
                return self.frames_stalled.replace(false);
            }
            _ => return false,
        }
        true
    }

    /// Works the flag out for `window`, which is `shown` or not. `wayland` when the window is a
    /// Wayland one.
    fn refresh(&self, window: &slint::Window, shown: bool, wayland: bool) {
        if wayland {
            self.watch_frames(window, shown);
        }
        let minimized = window.with_winit_window(|window| window.is_minimized()).flatten().unwrap_or(false);
        let seen = shown && !minimized && !self.occluded.get() && !self.zero_size.get() && !self.frames_stalled.get();
        self.flag.store(seen, Ordering::Relaxed);
    }

    /// Wayland doesn't say when a window is minimized or covered, but compositors stop sending it
    /// the frames winit waits for before each redraw. So a redraw is asked for now and then, and
    /// when it doesn't come, the window isn't seen. The next frame makes it seen again.
    fn watch_frames(&self, window: &slint::Window, shown: bool) {
        if !shown {
            self.frame_asked.set(None);
            self.frames_stalled.set(false);
            return;
        }
        match self.frame_asked.get() {
            None => {
                self.frame_asked.set(Some(Instant::now()));
                // winit's, which leaves what Slint draws alone, so a seen window draws nothing new
                window.with_winit_window(|window| window.request_redraw());
            }
            Some(asked) => self.frames_stalled.set(asked.elapsed() >= FRAME_STALL),
        }
    }

    /// The window closed.
    fn hidden(&self) {
        self.flag.store(false, Ordering::Relaxed);
    }
}

thread_local! {
    static GUI: RefCell<Option<Rc<Gui>>> = const { RefCell::new(None) };
}

/// Has Cmd+Q, the app menu's Quit, the Dock's Quit and logging out ask first, like closing the
/// main window, on macOS. AppKit otherwise ends the process at once, without asking to save the
/// profile or a preset being edited, and without stopping the session.
#[cfg(target_os = "macos")]
fn ask_before_quitting() {
    use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
    use objc2::{ffi, sel};

    /// `applicationShouldTerminate:`, which winit's app delegate doesn't have. It answers no, and
    /// the window asks instead, then quits by itself when it may.
    unsafe extern "C-unwind" fn should_terminate(
        _delegate: *mut AnyObject,
        _cmd: Sel,
        _sender: *mut AnyObject,
    ) -> usize {
        with_gui(|gui| gui.close_requested());
        // NSTerminateCancel
        0
    }

    // Made with the event loop, which the windows made
    let Some(class) = AnyClass::get(c"WinitApplicationDelegate") else { return };
    type ShouldTerminate = unsafe extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> usize;
    let method: ShouldTerminate = should_terminate;
    // SAFETY: the runtime calls it with the arguments the type string says: an NSUInteger answer
    // for self, _cmd and the sender, which match the function's own
    let imp = unsafe { std::mem::transmute::<ShouldTerminate, Imp>(method) };
    unsafe {
        ffi::class_addMethod(
            std::ptr::from_ref(class).cast_mut(),
            sel!(applicationShouldTerminate:),
            imp,
            c"Q@:@".as_ptr(),
        )
    };
}

/// Puts AutoSplit's icon in the Dock and the app menu's About panel on macOS. A bare binary has no
/// app bundle for macOS to take it from, so they'd show the generic program icon.
#[cfg(target_os = "macos")]
fn show_dock_icon() {
    use objc2::{AllocAnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let Some(main_thread) = MainThreadMarker::new() else { return };
    let data = NSData::with_bytes(include_bytes!("../../assets/icon.png"));
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else { return };
    unsafe { NSApplication::sharedApplication(main_thread).setApplicationIconImage(Some(&image)) };
}

/// Runs `action` with the GUI, on the window's thread.
fn with_gui(action: impl FnOnce(&Rc<Gui>) + Send + 'static) {
    let _ = slint::invoke_from_event_loop(move || {
        if let Some(gui) = GUI.with(|gui| gui.borrow().clone()) {
            action(&gui);
        }
    });
}

/// A split image's file name in parts, for the windows to color by what each part does.
pub fn file_name_parts(name: &str) -> ModelRc<NamePart> {
    use autosplit_core::filename::PartKind;
    let kind = |kind: PartKind| match kind {
        PartKind::Text => NameKind::Text,
        PartKind::Number => NameKind::Number,
        PartKind::Keyword => NameKind::Keyword,
        PartKind::Threshold => NameKind::Threshold,
        PartKind::Pause => NameKind::Pause,
        PartKind::Delay => NameKind::Delay,
        PartKind::Loops => NameKind::Loops,
        PartKind::Method => NameKind::Method,
        PartKind::Flags => NameKind::Flags,
        PartKind::Extension => NameKind::Extension,
    };
    let part = |part: autosplit_core::filename::NamePart| NamePart {
        text: part.text.into(),
        kind: kind(part.kind),
        meaning: part.meaning.into(),
        ignored: part.ignored,
    };
    let parts: Vec<NamePart> = autosplit_core::filename::name_parts(name).into_iter().map(part).collect();
    ModelRc::new(VecModel::from(parts))
}

/// The PNG images in `folder`, in the split image folder's order.
fn png_images(folder: &Path) -> Vec<PathBuf> {
    user_files(folder).unwrap_or_default().into_iter().filter(|path| paths::is_png(path)).collect()
}

/// A picture of an update as an image for the window, or `None` to keep what it shows.
fn shown_image(picture: Picture) -> Option<slint::Image> {
    match picture {
        Picture::Keep => None,
        Picture::Clear => Some(slint::Image::default()),
        Picture::Show(pixels) => Some(slint::Image::from_rgba8(pixels)),
    }
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
    tester: TesterWindow,
    maker: MakerWindow,
    instructions: InstructionsWindow,
}

/// The app id windows get on Linux, which desktops match to the .desktop file named after it, for
/// its icon and name. assets/install-desktop-entry.sh, in the Linux zip, writes that file. The
/// macOS bundle id in build.rs is the same.
const APP_ID: &str = "io.github.torjeamundsen.autosplit";

/// Draws on the CPU, which only redraws what changed: the preview and a few labels.
fn open_windows() -> anyhow::Result<Windows> {
    slint::BackendSelector::new().backend_name("winit".into()).renderer_name("software".into()).select()?;
    // Before any window is made. Does nothing outside Linux.
    slint::set_xdg_app_id(APP_ID)?;
    platform::allow_dark_menus();
    Ok(Windows {
        main: MainWindow::new()?,
        settings: SettingsWindow::new()?,
        about: AboutWindow::new()?,
        log: LogWindow::new()?,
        message: MessageWindow::new()?,
        tester: TesterWindow::new()?,
        maker: MakerWindow::new()?,
        instructions: InstructionsWindow::new()?,
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
            // Nothing failed on a first start without a profile, so it isn't shown as an error
            (Document::untitled(), vec![Message::warning(message)])
        }
    }
}

impl SessionThread {
    /// Runs a session for `profile` on a thread of its own, shown in the window.
    fn spawn(
        profile: UserProfile,
        link: TimerLink,
        console: Console,
        [visible, tester_visible, maker_visible]: [Arc<AtomicBool>; 3],
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
                let notices = |notice| with_gui(move |gui| gui.notice(notice));
                let view = GuiView::new(visible, tester_visible, maker_visible, deliver, notices);
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
    picker::catch_errors();

    let (events, receiver) = mpsc::channel();
    let (mut console, log_path) = Console::with_listener(Box::new(|line| {
        let line = line.to_owned();
        with_gui(move |gui| gui.add_log_line(line));
    }));
    let (document, first_messages) = open_document(&options, &mut console, log_path.as_deref());
    let link =
        if options.auto_controlled { TimerLink::AutoControlled } else { document.profile.timer_connection.into() };

    let visible = Arc::new(AtomicBool::new(!options.minimized));
    let tester_visible = Arc::new(AtomicBool::new(false));
    let maker_visible = Arc::new(AtomicBool::new(false));
    let gui = Rc::new_cyclic(|me| Gui {
        windows,
        me: me.clone(),
        hotkeys: Rc::new(VecModel::default()),
        group_rows: Rc::new(VecModel::from(vec![SimilarityRow::default(); MAX_GROUP_SIZE])),
        current_tiles: Rc::new(VecModel::from(vec![slint::Image::default(); MAX_GROUP_SIZE])),
        tester_info: Rc::new(VecModel::default()),
        tester_state: RefCell::default(),
        maker: RefCell::default(),
        maker_rows: Rc::new(VecModel::default()),
        clipboard: RefCell::new(None),
        state: RefCell::new(State {
            document,
            devices: Devices::default(),
            recording: None,
            link,
            running: false,
            can_start: true,
            log: LogLines::default(),
            messages: MessageQueue::default(),
            tools: Vec::new(),
        }),
        events: events.clone(),
        seen: Seen::new(visible.clone()),
        tester_seen: Seen::new(tester_visible.clone()),
        maker_seen: Seen::new(maker_visible.clone()),
        visibility_check: slint::Timer::default(),
        log_flush: slint::Timer::default(),
        log_dirty: Cell::new(false),
        first_messages: RefCell::new(Vec::new()),
        first_messages_fallback: slint::Timer::default(),
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
    windows.main.set_group_rows(ModelRc::from(gui.group_rows.clone()));
    windows.main.set_current_tiles(ModelRc::from(gui.current_tiles.clone()));
    windows.tester.set_info(ModelRc::from(gui.tester_info.clone()));
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
    let visibility = [visible, tester_visible, maker_visible];
    let session = SessionThread::spawn(profile, link, console, visibility, events, receiver)
        .map_err(|error| Error::Failed(error.into()))?;

    *gui.first_messages.borrow_mut() = first_messages;
    // Shown when the main window gets focus, or after a while when it opens minimized
    let weak = Rc::downgrade(&gui);
    gui.first_messages_fallback.start(slint::TimerMode::SingleShot, FIRST_MESSAGES_FALLBACK, move || {
        if let Some(gui) = weak.upgrade() {
            gui.show_first_messages();
        }
    });
    #[cfg(target_os = "macos")]
    {
        ask_before_quitting();
        show_dock_icon();
    }
    slint::run_event_loop_until_quit().map_err(|error| Error::Failed(error.into()))?;
    // Hidden first, since stopping the session can take a moment
    gui.hide_all();
    let result = session.finish();
    GUI.with(|cell| cell.borrow_mut().take());
    result.map_err(|error| Error::Failed(anyhow::anyhow!("autosplit stopped because of an error: {error}")))
}

/// The windows' callbacks that act on the GUI. Message boxes are modal like AutoSplit's, so
/// these do nothing while one is open; `always` ones still run. `main` ones also do nothing while
/// the Live Tester is open. An `else` value is what the callback returns.
macro_rules! on {
    (main $weak:expr, $window:expr, $callback:ident, ($($argument:ident),*) $gui:ident => $body:expr) => {{
        let weak = $weak.clone();
        $window.$callback(move |$($argument),*| {
            if let Some($gui) = weak.upgrade() {
                if $gui.main_blocked() {
                    return $gui.focus_blocking();
                }
                $body;
            }
        });
    }};
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

mod maker;
pub mod picker;
mod tester;

pub use maker::{THUMB_SIZE, thumbnail, transparency_grid};

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
        self.maker_follows_profile();
    }

    /// The capture region was set by something other than its fields, which show it even while
    /// one is being typed in.
    fn region_set_for_user(&self) {
        let main = &self.windows.main;
        main.set_region_generation(main.get_region_generation().wrapping_add(1));
        self.profile_edited();
    }

    /// The split image folder, as the profile says, or `None` without one.
    fn split_image_folder(&self) -> Option<PathBuf> {
        let state = self.state.borrow();
        let directory = &state.document.profile.split_image_directory;
        (!directory.is_empty()).then(|| state.document.resolve(&exe_folder(), directory))
    }

    /// The profile changed in a way the windows show elsewhere too.
    fn profile_edited(&self) {
        self.send_profile(false);
        self.show_profile();
    }

    fn blocked(&self) -> bool {
        self.state.borrow().messages.is_showing()
    }

    /// The main window is blocked by a message box, and while a tool window is open.
    fn main_blocked(&self) -> bool {
        self.blocked() || !self.state.borrow().tools.is_empty()
    }

    /// Brings what blocks the main window to the front: a message box, or the newest tool window.
    fn focus_blocking(&self) {
        if self.blocked() {
            return self.focus_message();
        }
        if let Some(&tool) = self.state.borrow().tools.last() {
            self.tool_window(tool).with_winit_window(|window| window.focus_window());
        }
    }

    fn tool_window(&self, tool: Tool) -> &slint::Window {
        match tool {
            Tool::Tester => self.windows.tester.window(),
            Tool::Maker => self.windows.maker.window(),
        }
    }

    /// A tool window opened, which the main window can't be used beside.
    fn tool_opened(&self, tool: Tool) {
        let mut state = self.state.borrow_mut();
        state.tools.retain(|open| *open != tool);
        state.tools.push(tool);
        drop(state);
        self.show_tool();
    }

    /// A tool window closes. Called before it's hidden.
    fn tool_closed(&self, tool: Tool) {
        self.state.borrow_mut().tools.retain(|open| *open != tool);
        match tool {
            Tool::Tester => self.tester_closed(),
            Tool::Maker => self.maker_closed(),
        }
        self.show_tool();
    }

    /// The main window says which tool window is open, if one is.
    fn show_tool(&self) {
        let title = self.state.borrow().tools.last().map_or("", |tool| tool.title());
        self.windows.main.set_tool(title.into());
    }

    /// The main window's Close button for the newest tool window.
    fn close_tool(self: &Rc<Self>) {
        let Some(tool) = self.state.borrow().tools.last().copied() else { return };
        match tool {
            Tool::Tester => self.close_tester(),
            Tool::Maker => self.close_maker(),
        }
    }

    fn connect(self: &Rc<Self>) {
        self.connect_main();
        self.connect_settings();
        self.connect_small_windows();
        self.connect_tester();
        self.connect_maker();
        self.connect_window_events();
    }

    fn connect_main(&self) {
        let weak = self.me.clone();
        let main = &self.windows.main;
        on!(main weak, main, on_browse_folder, () gui => gui.browse_folder(gui.windows.main.window(), |_| {}));
        on!(main weak, main, on_region_edited, (x, y, width, height) gui => {
            let mut state = gui.state.borrow_mut();
            let region = &mut state.document.profile.capture_region;
            // Spin boxes hold f32, which rounds large whole numbers; AutoSplit's limit stays the limit
            let whole = |value: f32| (value.round() as i64).min(MAX_POSITION);
            (region.x, region.y, region.width, region.height) = (whole(x), whole(y), whole(width), whole(height));
            drop(state);
            gui.send_profile(false);
        });
        on!(main weak, main, on_take_screenshot, () gui => gui.send(UiCommand::Screenshot));
        on!(main weak, main, on_crop_to_4x3, () gui => gui.send(UiCommand::CropTo4x3));
        on!(main weak, main, on_start_auto_splitter, () gui => gui.send(UiCommand::StartAutoSplitter));
        // A run goes on while a tool window is open, so these stay usable from its overlay
        on!(weak, main, on_reset, () gui => gui.send(UiCommand::Reset));
        on!(weak, main, on_skip_split, () gui => gui.send(UiCommand::SkipSplit));
        on!(weak, main, on_undo_split, () gui => gui.send(UiCommand::UndoSplit));
        on!(main weak, main, on_next_image, () gui => gui.send(UiCommand::NextImage));
        on!(main weak, main, on_previous_image, () gui => gui.send(UiCommand::PreviousImage));
        on!(main weak, main, on_reload_split_images, () gui => gui.send(UiCommand::ReloadSplitImages));
        // Also beside a tool window, which may need the capture device or grouping changed
        on!(weak, main, on_open_settings, () gui => gui.open_settings());
        on!(main weak, main, on_open_tester, () gui => gui.open_tester());
        on!(main weak, main, on_open_maker, () gui => gui.open_maker());
        on!(main weak, main, on_save_profile, () gui => gui.save_profile(false, None));
        on!(main weak, main, on_save_profile_as, () gui => gui.save_profile(true, None));
        on!(main weak, main, on_load_profile, () gui => gui.load_profile());
        // Help, the log and About don't change anything, so they open beside a tool window too
        on!(weak, main, on_view_help, () gui => gui.open(TUTORIAL_URL));
        on!(weak, main, on_view_log, () gui => gui.show_log());
        on!(weak, main, on_about, () gui => gui.show_window(gui.windows.about.window()));
        on!(always weak, main, on_blocked_clicked, () gui => gui.focus_blocking());
        on!(weak, main, on_close_tool, () gui => gui.close_tool());
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
        on!(weak, about, on_open_licenses, () gui => match crate::licenses::write() {
            Ok(path) => gui.open(&path.to_string_lossy()),
            Err(error) => gui.show_message(Message::error(format!("Couldn't write the licenses: {error}"))),
        });
        on!(weak, log, on_open_log_file, () gui => {
            let path = gui.windows.log.get_log_path();
            if !path.is_empty() {
                gui.open(path.as_str());
            }
        });
        on!(always weak, message, on_answered, (button) gui => gui.message_answered(Some(button as usize)));
        // The same message box, drawn in the windows it blocks under Wayland
        let windows = &self.windows;
        on!(always weak, windows.main, on_inline_answered, (button) gui => gui.message_answered(usize::try_from(button).ok()));
        on!(always weak, windows.settings, on_inline_answered, (button) gui => gui.message_answered(usize::try_from(button).ok()));
        on!(always weak, windows.tester, on_inline_answered, (button) gui => gui.message_answered(usize::try_from(button).ok()));
        on!(always weak, windows.maker, on_inline_answered, (button) gui => gui.message_answered(usize::try_from(button).ok()));
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
            if gui.swallowed_while_blocked(event) {
                return EventResult::PreventDefault;
            }
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
                    if gui.mac_window_key(gui.windows.settings.window(), event, held) || gui.shortcut(event, held) {
                        return EventResult::PreventDefault;
                    }
                    // Escape closes it, like Qt's dialogs
                    if event.logical_key == Key::Named(NamedKey::Escape) && !gui.blocked() {
                        let _ = gui.windows.settings.hide();
                        return EventResult::PreventDefault;
                    }
                }
                _ => {}
            }
            EventResult::Propagate
        });

        // Escape closes the small windows, and Enter About too, whose only button is OK
        let windows = [
            (self.windows.about.window(), true),
            (self.windows.log.window(), false),
            (self.windows.instructions.window(), false),
        ];
        for (window, enter_too) in windows {
            let weak = self.me.clone();
            let mut held = Modifiers::empty();
            window.on_winit_window_event(move |window, event| {
                if let WindowEvent::ModifiersChanged(modifiers) = event {
                    held = modifiers_held(modifiers.state());
                }
                let WindowEvent::KeyboardInput { event, .. } = event else { return EventResult::Propagate };
                if weak.upgrade().is_some_and(|gui| !gui.blocked() && gui.mac_window_key(window, event, held)) {
                    return EventResult::PreventDefault;
                }
                let closes = match &event.logical_key {
                    Key::Named(NamedKey::Escape) => true,
                    Key::Named(NamedKey::Enter) => enter_too,
                    _ => false,
                };
                if !closes || event.state != ElementState::Pressed || weak.upgrade().is_none_or(|gui| gui.blocked()) {
                    return EventResult::Propagate;
                }
                let _ = window.hide();
                EventResult::PreventDefault
            });
        }

        let weak = self.me.clone();
        let mut held = Modifiers::empty();
        self.windows.main.window().on_winit_window_event(move |window, event| {
            let Some(gui) = weak.upgrade() else { return EventResult::Propagate };
            if gui.inline_message_key(event) {
                return EventResult::PreventDefault;
            }
            match event {
                WindowEvent::ModifiersChanged(modifiers) => held = modifiers_held(modifiers.state()),
                // Closing asks first, like the close button
                WindowEvent::KeyboardInput { event, .. } if gui.mac_window_key(window, event, held) => {
                    return EventResult::PreventDefault;
                }
                WindowEvent::Focused(focused) => {
                    gui.windows.main.global::<WindowFocus>().set_active(*focused);
                    if *focused {
                        gui.show_first_messages();
                    }
                }
                event if gui.seen.note(event) => gui.update_visibility(),
                _ => {}
            }
            EventResult::Propagate
        });

        let weak = self.me.clone();
        let mut held = Modifiers::empty();
        self.windows.tester.window().on_winit_window_event(move |_, event| {
            let Some(gui) = weak.upgrade() else { return EventResult::Propagate };
            if gui.swallowed_while_blocked(event) {
                return EventResult::PreventDefault;
            }
            match event {
                WindowEvent::ModifiersChanged(modifiers) => held = modifiers_held(modifiers.state()),
                WindowEvent::KeyboardInput { event, .. }
                    if event.state == ElementState::Pressed
                        && (gui.mac_window_key(gui.windows.tester.window(), event, held)
                            || gui.tool_shortcut(Tool::Tester, event, held)) =>
                {
                    return EventResult::PreventDefault;
                }
                event if gui.tester_seen.note(event) => gui.update_visibility(),
                _ => {}
            }
            EventResult::Propagate
        });

        // The preset editor's keys
        let weak = self.me.clone();
        let mut held = Modifiers::empty();
        self.windows.maker.window().on_winit_window_event(move |_, event| {
            let Some(gui) = weak.upgrade() else { return EventResult::Propagate };
            if gui.swallowed_while_blocked(event) {
                return EventResult::PreventDefault;
            }
            match event {
                WindowEvent::ModifiersChanged(modifiers) => held = modifiers_held(modifiers.state()),
                WindowEvent::KeyboardInput { event, .. }
                    if event.state == ElementState::Pressed
                        && (gui.mac_window_key(gui.windows.maker.window(), event, held)
                            || gui.editor_shortcut(event, held)
                            || gui.tool_shortcut(Tool::Maker, event, held)) =>
                {
                    return EventResult::PreventDefault;
                }
                event if gui.maker_seen.note(event) => gui.update_visibility(),
                _ => {}
            }
            EventResult::Propagate
        });
    }

    /// Checks now and then whether the main window and the Live Tester can be seen, since not
    /// every system tells when they're minimized (X11) or covered (Windows).
    fn watch_visibility(&self) {
        let weak = self.me.clone();
        self.visibility_check.start(slint::TimerMode::Repeated, VISIBILITY_INTERVAL, move || {
            if let Some(gui) = weak.upgrade() {
                gui.update_visibility();
            }
        });
    }

    fn update_visibility(&self) {
        let wayland = self.under_wayland();
        // The main window is the program, so it counts as shown
        self.seen.refresh(self.windows.main.window(), true, wayland);
        let tester = self.windows.tester.window();
        self.tester_seen.refresh(tester, tester.is_visible(), wayland);
        let maker = self.windows.maker.window();
        self.maker_seen.refresh(maker, maker.is_visible(), wayland);
    }

    fn hide_all(&self) {
        let windows = &self.windows;
        for window in [
            windows.main.window(),
            windows.settings.window(),
            windows.about.window(),
            windows.log.window(),
            windows.message.window(),
            windows.tester.window(),
            windows.maker.window(),
            windows.instructions.window(),
        ] {
            let _ = window.hide();
        }
    }

    /// AutoSplit's keyboard shortcuts. Returns whether the key was one.
    fn shortcut(self: &Rc<Self>, event: &KeyEvent, held: Modifiers) -> bool {
        if self.blocked() {
            return false;
        }
        let control = held.contains(PRIMARY);
        let shift = held.contains(Modifiers::SHIFT);
        match (control, shift, key_character(event).map(|c| c.to_ascii_lowercase()), &event.logical_key) {
            (true, _, Some(','), _) => self.open_settings(),
            (true, _, Some('t'), _) => self.open_tester(),
            // Cmd+M minimizes on macOS, like in its other apps
            (true, cfg!(target_os = "macos"), Some('m'), _) => self.open_maker(),
            (true, true, Some('s'), _) => self.save_profile(true, None),
            (true, false, Some('s'), _) => self.save_profile(false, None),
            (true, _, Some('o'), _) => self.load_profile(),
            (false, _, _, Key::Named(NamedKey::F1)) => self.open(TUTORIAL_URL),
            _ => return false,
        }
        true
    }

    /// macOS's keys for any window, which its apps share: Cmd+W closes it like its close button,
    /// and Cmd+M minimizes it. Returns whether the key was one. None of them elsewhere, where
    /// Ctrl+M opens Make Split Images.
    fn mac_window_key(&self, window: &slint::Window, event: &KeyEvent, held: Modifiers) -> bool {
        if !cfg!(target_os = "macos") || event.state != ElementState::Pressed || event.repeat || held != PRIMARY {
            return false;
        }
        match key_character(event).map(|c| c.to_ascii_lowercase()) {
            Some('w') => window.dispatch_event(slint::platform::WindowEvent::CloseRequested),
            Some('m') => window.set_minimized(true),
            _ => return false,
        }
        true
    }

    /// The keys of tool window `tool`: Ctrl+, for Settings and F1 for Help, which can be open
    /// beside it, and Escape to close it, like the other windows. Escape closes an open dropdown
    /// list first. Returns whether the key was one.
    fn tool_shortcut(self: &Rc<Self>, tool: Tool, event: &KeyEvent, held: Modifiers) -> bool {
        if self.blocked() || event.repeat {
            return false;
        }
        let comma = key_character(event) == Some(',');
        match (&event.logical_key, held) {
            (_, PRIMARY) if comma => self.open_settings(),
            (Key::Named(NamedKey::F1), held) if held.is_empty() => self.open(TUTORIAL_URL),
            (Key::Named(NamedKey::Escape), held) if held.is_empty() => match tool {
                Tool::Tester => self.close_tester(),
                Tool::Maker if self.windows.maker.get_list_showing() => return false,
                Tool::Maker => self.close_maker(),
            },
            _ => return false,
        }
        true
    }

    /// Starts or stops waiting for a key for a hotkey.
    fn record_hotkey(&self, action: Option<HotkeyAction>) {
        self.state.borrow_mut().recording = action;
        // So keys held with Cmd reach Set Hotkey on macOS, like Cmd+Q
        platform::hold_menu_shortcuts(action.is_some());
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
        // Named so the name reads back as this key, whatever the layout
        let character = autosplit_input::recorded_character(key, key_character(event));
        // A modifier on its own: wait for the key it goes with
        let Some(name) = hotkey_name(key, character, held) else { return };
        self.assign_hotkey(action, &name);
    }

    /// A hotkey name typed in its field. Returns whether it's a hotkey, like AutoSplit checks.
    fn hotkey_typed(&self, action: HotkeyAction, text: &str) -> bool {
        let name = text.trim();
        if action.hotkey(&self.state.borrow().document.profile) == name {
            return true;
        }
        if !name.is_empty()
            && let Err(error) = parse_hotkey(name, system_layout())
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
        if !window.is_visible() {
            self.place_by_main(window, centered_over);
        }
        if let Err(error) = window.show() {
            self.add_log_line(format!("error: Couldn't show a window: {error}"));
        }
        // Wayland only maps a window once it draws, and the windows got their first redraw
        // while hidden at startup, so the first show would wait for some other redraw
        window.request_redraw();
    }

    /// Puts `window` where `place` says, by the main window, so it doesn't show up on another
    /// screen.
    ///
    /// Windows and Linux lay screens out in pixels. macOS lays them out in points, which winit
    /// turns into pixels by each screen's or window's own scale, so with a Retina screen and a
    /// 1x one, their pixel positions don't line up. There it places windows in points.
    fn place_by_main(&self, window: &slint::Window, place: fn(Area, u32, u32, Option<Area>) -> (i32, i32)) {
        let points = cfg!(target_os = "macos");
        // What a pixel position at `scale` is divided by
        let unit = |scale: f64| if points { scale } else { 1.0 };
        let area = |x: i32, y: i32, width: u32, height: u32, scale: f64| {
            let unit = unit(scale);
            let (x, y) = ((f64::from(x) / unit).round() as i32, (f64::from(y) / unit).round() as i32);
            Area {
                x,
                y,
                width: (f64::from(width) / unit).round() as u32,
                height: (f64::from(height) / unit).round() as u32,
            }
        };
        let main = self.windows.main.window();
        let Some((minimized, screen)) = main.with_winit_window(|main| {
            let screen = main.current_monitor().map(|monitor| {
                let (position, size) = (monitor.position(), monitor.size());
                area(position.x, position.y, size.width, size.height, monitor.scale_factor())
            });
            (main.is_minimized().unwrap_or(false), screen)
        }) else {
            return;
        };
        // A minimized window sits far off screen on Windows
        if minimized {
            return;
        }
        let (position, size) = (main.position(), main.size());
        let main_area = area(position.x, position.y, size.width, size.height, main.scale_factor().into());
        // A hidden window's size can be at another screen's scale, so it goes by the main window's
        let logical = window.size().to_logical(window.scale_factor());
        let size = if points { logical.to_physical(1.0) } else { logical.to_physical(main.scale_factor()) };
        let (x, y) = place(main_area, size.width, size.height, screen);
        if points {
            window.set_position(slint::LogicalPosition::new(x as f32, y as f32));
        } else {
            window.set_position(slint::PhysicalPosition::new(x, y));
        }
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
                    let picked = settings_form::pick_default_device(&mut state.document.profile, &list);
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
        main.set_group_split_images(profile.group_split_images);

        let settings = &self.windows.settings;
        settings.set_fps_limit(profile.fps_limit as f32);
        settings.set_live_capture_region(profile.live_capture_region);
        settings.set_display_pause_timer(profile.display_pause_timer);
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
        settings.set_group_split_images(profile.group_split_images);
        settings.set_split_image_grid(profile.split_image_grid);
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
            Setting::DisplayPauseTimer => SettingEdit::DisplayPauseTimer(settings.get_display_pause_timer()),
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
            Setting::GroupSplitImages => SettingEdit::GroupSplitImages(settings.get_group_split_images()),
            Setting::SplitImageGrid => SettingEdit::SplitImageGrid(settings.get_split_image_grid()),
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
            Notice::AutoResetToggled(on) => {
                self.state.borrow_mut().document.profile.enable_auto_reset = on;
                self.profile_edited();
            }
            Notice::TimerLinkChanged(link) => {
                self.state.borrow_mut().link = link;
                self.show_profile();
            }
            Notice::ScreenshotSaved(path) => {
                let name = paths::file_name_text(&path).unwrap_or_default();
                self.windows.main.invoke_screenshot_saved(name.into());
            }
            Notice::Grabbed(image) => {
                if let Some(gui) = self.me.upgrade() {
                    gui.grabbed(image);
                }
            }
            Notice::CroppedRegion(region) => {
                self.state.borrow_mut().document.profile.capture_region = region;
                self.region_set_for_user();
            }
            Notice::FullFrameRegion(region) => {
                // Unless the region was set meanwhile, which the session gets back either way
                let mut state = self.state.borrow_mut();
                let profile = &mut state.document.profile;
                if profile.region_untouched() {
                    profile.capture_region = region;
                }
                drop(state);
                self.region_set_for_user();
            }
            Notice::ProfileLoaded { path, profile } => {
                self.state.borrow_mut().document = Document::new(*profile, Some(path));
                self.profile_edited();
                self.tester_follows_split_folder();
            }
        }
    }

    /// What the session shows changed.
    fn show_update(&self, mut update: Update) {
        let main = &self.windows.main;
        if let Some(texts) = update.texts.take() {
            main.set_running(texts.running);
            self.windows.settings.set_running(texts.running);
            main.set_device_name(if texts.device_name.is_empty() { "-".into() } else { texts.device_name.into() });
            main.set_live_message(texts.live_message.into());
            let window = texts.window;
            main.set_current_image_file(window.current_file.into());
            main.set_current_image_message(window.current_message.into());
            main.set_current_grid(texts.grid);
            let [current_live, current_highest, current_threshold, reset_live, reset_highest, reset_threshold] =
                window.table;
            main.set_current_live(current_live.into());
            main.set_current_highest(current_highest.into());
            main.set_current_threshold(current_threshold.into());
            main.set_reset_live(reset_live.into());
            main.set_reset_highest(reset_highest.into());
            main.set_reset_threshold(reset_threshold.into());
            for (index, [live, highest, threshold]) in window.group_rows.into_iter().enumerate() {
                let row = SimilarityRow { live: live.into(), highest: highest.into(), threshold: threshold.into() };
                if self.group_rows.row_data(index).as_ref() != Some(&row) {
                    self.group_rows.set_row_data(index, row);
                }
            }
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
            state.can_start = start;
            drop(state);
            self.show_buttons();
        }
        if let Some(live) = shown_image(std::mem::take(&mut update.live)) {
            main.set_live_image(live);
        }
        if let Some(current) = shown_image(std::mem::take(&mut update.current)) {
            main.set_current_image(current);
        }
        for (index, tile) in std::mem::take(&mut update.tiles).into_iter().enumerate() {
            if let Some(tile) = shown_image(tile) {
                self.current_tiles.set_row_data(index, tile);
            }
        }
        let maker = &self.windows.maker;
        let maker_live = std::mem::take(&mut update.maker.live);
        if let Picture::Show(pixels) = &maker_live {
            self.maker_live_frame(pixels);
        }
        if let Some(live) = shown_image(maker_live) {
            maker.set_live_image(live);
        }
        if let Some(message) = update.maker.message.take() {
            self.maker_live_message(&message);
            maker.set_live_message(message.into());
        }
        self.show_tester_update(update.tester);
    }

    /// Start Auto Splitter, which AutoSplit disables while Set Hotkey waits for a key.
    fn show_buttons(&self) {
        let state = self.state.borrow();
        self.windows.main.set_can_start(state.can_start && state.recording.is_none());
    }

    /// Asks for the split image folder over `window`, then does `then`.
    fn browse_folder(self: &Rc<Self>, window: &slint::Window, then: impl FnOnce(&Rc<Gui>) + 'static) {
        let state = self.state.borrow();
        let current = &state.document.profile.split_image_directory;
        // AutoSplit starts in the folder above the current one
        let start = if current.is_empty() {
            exe_folder()
        } else {
            let folder = state.document.resolve(&exe_folder(), current);
            folder.parent().map_or(folder.clone(), Path::to_owned)
        };
        drop(state);
        let dialog = self.file_dialog_over("Select Split Image Directory", window).set_directory(start).pick_folder();
        self.when_picked(dialog, |gui, folder| {
            gui.state.borrow_mut().document.profile.split_image_directory = folder_text(&folder);
            gui.profile_edited();
            gui.tester_follows_split_folder();
            then(gui);
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
        let start = state.document.resolve(&exe_folder(), current);
        drop(state);
        let dialog = self.file_dialog("Select Screenshots Directory").set_directory(start).pick_folder();
        self.when_picked(dialog, |gui, folder| {
            gui.state.borrow_mut().document.profile.screenshot_directory = folder_text(&folder);
            gui.profile_edited();
        });
    }

    /// A file dialog in front of the main window, or of Settings while it has the focus, like for
    /// its Browse... button or Save Profile's shortcut there. On macOS a dialog is a sheet on the
    /// window it's in front of, so on the main window it could open behind Settings.
    fn file_dialog(&self, title: &str) -> rfd::AsyncFileDialog {
        let settings = &self.windows.settings;
        let in_settings = settings.window().is_visible() && settings.global::<WindowFocus>().get_active();
        let window = if in_settings { settings.window() } else { self.windows.main.window() };
        self.file_dialog_over(title, window)
    }

    /// A file dialog in front of `window`.
    fn file_dialog_over(&self, title: &str, window: &slint::Window) -> rfd::AsyncFileDialog {
        picker::clear_error();
        rfd::AsyncFileDialog::new().set_title(title).set_parent(&window.window_handle())
    }

    /// Waits for a file dialog without blocking the window, then does `then` with the path.
    fn when_picked(
        self: &Rc<Self>,
        dialog: impl Future<Output = Option<rfd::FileHandle>> + 'static,
        then: impl FnOnce(&Rc<Gui>, PathBuf) + 'static,
    ) {
        let weak = Rc::downgrade(self);
        let waiting = slint::spawn_local(async move {
            let picked = dialog.await;
            let Some(gui) = weak.upgrade() else { return };
            match picked {
                Some(file) => then(&gui, file.path().to_owned()),
                // Answered nothing because it couldn't open, rather than cancelled
                None => {
                    if let Some(error) = picker::take_error() {
                        gui.show_message(Message::error(error));
                    }
                }
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

    /// Asks to load another profile, after asking to save changes to this one.
    fn load_profile(self: &Rc<Self>) {
        self.unless_cancelled(|gui| gui.pick_profile());
    }

    fn pick_profile(self: &Rc<Self>) {
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
            gui.tester_follows_split_folder();
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
        // A preset being edited is asked about first, then the profile
        self.unless_discarded(|gui| gui.unless_cancelled(|_| quit()));
    }

    /// Does `then` after asking whether to save the profile's changes, if it has any, like
    /// AutoSplit does before closing. Nothing happens when that's cancelled.
    fn unless_cancelled(self: &Rc<Self>, then: impl FnOnce(&Rc<Self>) + 'static) {
        let state = self.state.borrow();
        if !state.document.has_changes() {
            drop(state);
            return then(self);
        }
        let name = state.document.name();
        drop(state);
        let gui = Rc::downgrade(self);
        self.show_message(Message::question(
            format!("Do you want to save changes made to settings file {name}?"),
            move |answer| {
                let Some(gui) = gui.upgrade() else { return };
                match answer {
                    Answer::Yes => gui.save_profile(false, Some(Box::new(then))),
                    Answer::No => then(&gui),
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

    /// The messages from opening. Before the main window is open, a message can't be placed
    /// over it, kept on top or focused, and the main window opens in front of it.
    fn show_first_messages(&self) {
        self.first_messages_fallback.stop();
        let messages = std::mem::take(&mut *self.first_messages.borrow_mut());
        for message in messages {
            self.show_message(message);
        }
    }

    fn show_message(&self, message: Message) {
        let shown = self.state.borrow_mut().messages.push(message);
        if let Some(shown) = shown {
            self.present_message(shown);
        }
    }

    fn present_message(&self, shown: messages::Shown) {
        let [primary, secondary, tertiary] = shown.labels;
        if self.messages_inline() {
            self.show_inline_message(InlineMessage {
                shown: true,
                title: shown.buttons.title().into(),
                message: shown.text.into(),
                primary: primary.into(),
                secondary: secondary.into(),
                tertiary: tertiary.into(),
                default_button: shown.default as i32,
            });
            return self.set_all_blocked(true);
        }
        self.windows.message.set_window_title(shown.buttons.title().into());
        self.windows.message.set_message(shown.text.into());
        self.windows.message.set_primary(primary.into());
        self.windows.message.set_secondary(secondary.into());
        self.windows.message.set_tertiary(tertiary.into());
        self.windows.message.set_default_button(shown.default as i32);
        self.set_all_blocked(true);
        self.show_window(self.windows.message.window());
        self.windows.message.window().with_winit_window(|window| window.set_window_level(WindowLevel::AlwaysOnTop));
        self.windows.message.invoke_focus_keys();
        self.focus_message();
    }

    fn focus_message(&self) {
        // Drawn in every window it blocks, so it's in front already
        if !self.messages_inline() {
            self.windows.message.window().with_winit_window(|window| window.focus_window());
        }
    }

    /// Whether message boxes are drawn inside the windows they block instead of in a window of
    /// their own: under Wayland, where a window can't be kept in front or brought there, so the
    /// message window could end up behind the windows it blocks, which would look frozen.
    fn messages_inline(&self) -> bool {
        self.under_wayland()
    }

    /// Whether the windows are Wayland ones.
    fn under_wayland(&self) -> bool {
        #[cfg(target_os = "linux")]
        {
            use slint::winit_030::winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
            let display = self.windows.main.window().with_winit_window(|window| {
                window.display_handle().is_ok_and(|handle| matches!(handle.as_raw(), RawDisplayHandle::Wayland(_)))
            });
            return display.unwrap_or(false);
        }
        #[allow(unreachable_code)]
        false
    }

    /// Shows `message` in every window a message box blocks, or hides it there.
    fn show_inline_message(&self, message: InlineMessage) {
        let windows = &self.windows;
        windows.main.set_inline_message(message.clone());
        windows.settings.set_inline_message(message.clone());
        windows.tester.set_inline_message(message.clone());
        windows.maker.set_inline_message(message);
    }

    /// Enter and Escape answer a message box drawn in the window, like the message window's, and
    /// other keys don't reach the window under it. Returns whether `event` was a key for it.
    fn inline_message_key(&self, event: &WindowEvent) -> bool {
        let message = self.windows.main.get_inline_message();
        let WindowEvent::KeyboardInput { event, .. } = event else { return false };
        if !message.shown {
            return false;
        }
        if event.state == ElementState::Pressed && !event.repeat {
            match event.logical_key {
                Key::Named(NamedKey::Enter) => self.message_answered(usize::try_from(message.default_button).ok()),
                Key::Named(NamedKey::Escape) => self.message_answered(None),
                _ => {}
            }
        }
        true
    }

    /// While a message box is open, keys, typed text and scrolling don't reach the windows under
    /// it. Slint would change a field or the selected split before the callback that's turned
    /// away, so the window would show what Rust doesn't have. A key press brings the message box
    /// to the front instead.
    fn swallowed_while_blocked(&self, event: &WindowEvent) -> bool {
        if !self.blocked() {
            return false;
        }
        if self.inline_message_key(event) {
            return true;
        }
        match event {
            // A long message drawn in the window scrolls, and its backdrop keeps the wheel from
            // the window under it
            WindowEvent::MouseWheel { .. } if self.windows.main.get_inline_message().shown => false,
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed {
                    self.focus_message();
                }
                true
            }
            WindowEvent::Ime(_) | WindowEvent::MouseWheel { .. } => true,
            _ => false,
        }
    }

    /// The windows a message box blocks, which AutoSplit's are modal for, take no input while
    /// one shows.
    fn set_all_blocked(&self, blocked: bool) {
        let windows = &self.windows;
        windows.main.set_blocked(blocked);
        windows.settings.set_blocked(blocked);
        windows.tester.set_blocked(blocked);
        windows.maker.set_blocked(blocked);
    }

    /// A message box was answered with a button, or closed.
    fn message_answered(&self, button: Option<usize>) {
        let _ = self.windows.message.hide();
        let (answered, next) = self.state.borrow_mut().messages.answer(button);
        match next {
            Some(next) => self.present_message(next),
            None => {
                self.show_inline_message(InlineMessage::default());
                self.set_all_blocked(false);
            }
        }
        if let Some((answered, answer)) = answered {
            answered(answer);
        }
    }
}
