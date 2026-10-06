//! The window: AutoSplit's main window and settings, driving a [`Session`] on a thread of its
//! own.

pub mod app_settings;
mod view;

pub mod ui {
    slint::include_modules!();
}

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;

use anyhow::Result;
use autosplit_capture::{DeviceInfo, list_devices};
use autosplit_core::commands::{Command, TimerLink};
use autosplit_core::hotkeys::HotkeyAction;
use autosplit_core::profile::{TimerConnection, UserProfile, VIDEO_CAPTURE_DEVICE};
use autosplit_input::{KeyCode, Modifiers, hotkey_name};
use slint::winit_030::winit::event::{ElementState, WindowEvent};
use slint::winit_030::winit::keyboard::{Key, NamedKey, PhysicalKey};
use slint::winit_030::{EventResult, WinitWindowAccessor};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use self::app_settings::AppSettings;
use self::ui::{AboutWindow, HotkeyRow, LogWindow, MainWindow, MessageWindow, SettingsWindow};
use self::view::GuiView;
use crate::console::Console;
use crate::session::{Event, Session, UiCommand, capture_method_name};
use crate::{profile, run, screenshot};

/// How the GUI was started.
pub struct Options {
    /// By LiveSplit's AutoSplit Integration component.
    pub auto_controlled: bool,
    pub minimized: bool,
}

/// Lines kept in the log window.
const LOG_LINES: usize = 2000;

/// The hotkeys in the settings window, in AutoSplit's order and with its labels.
const HOTKEY_ROWS: [(HotkeyAction, &str); 7] = [
    (HotkeyAction::Split, "Start / Split:"),
    (HotkeyAction::Reset, "Reset:"),
    (HotkeyAction::UndoSplit, "Undo Split:"),
    (HotkeyAction::SkipSplit, "Skip Split:"),
    (HotkeyAction::Pause, "Pause:"),
    (HotkeyAction::Screenshot, "Screenshot:"),
    (HotkeyAction::ToggleAutoResetImage, "Toggle auto reset image"),
];

fn hotkey_mut(profile: &mut UserProfile, action: HotkeyAction) -> &mut String {
    match action {
        HotkeyAction::Split => &mut profile.split_hotkey,
        HotkeyAction::Reset => &mut profile.reset_hotkey,
        HotkeyAction::SkipSplit => &mut profile.skip_split_hotkey,
        HotkeyAction::UndoSplit => &mut profile.undo_split_hotkey,
        HotkeyAction::Pause => &mut profile.pause_hotkey,
        HotkeyAction::Screenshot => &mut profile.screenshot_hotkey,
        HotkeyAction::ToggleAutoResetImage => &mut profile.toggle_auto_reset_image_hotkey,
    }
}

/// What to do with the button picked in a message box.
type Answered = Box<dyn FnOnce(usize)>;

/// A message box waiting for its turn.
struct Message {
    text: String,
    buttons: [&'static str; 3],
    answered: Option<Answered>,
}

struct State {
    /// The profile as in its file: folders aren't resolved, so saving writes them back as they were.
    profile: UserProfile,
    path: Option<PathBuf>,
    /// The profile as last loaded or saved, to tell whether there are changes to save.
    saved: UserProfile,
    devices: Vec<DeviceInfo>,
    setting_hotkey: Option<usize>,
    held: Modifiers,
    app: AppSettings,
    started_with_gpu: bool,
    started_with: TimerConnection,
    log: VecDeque<String>,
    log_path: Option<PathBuf>,
    messages: VecDeque<Message>,
    /// The message box shown, and what to do when it's answered.
    showing_message: Option<Option<Answered>>,
}

struct Gui {
    main: MainWindow,
    settings: SettingsWindow,
    about: AboutWindow,
    log: LogWindow,
    message: MessageWindow,
    state: RefCell<State>,
    events: mpsc::Sender<Event>,
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

/// AutoSplit's `load_settings_on_open`: the only `.toml` next to the executable.
fn profile_on_open() -> Result<(PathBuf, UserProfile), String> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(exe_folder())
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default();
    found.retain(|path| path.extension().is_some_and(|extension| extension == "toml"));
    match found.as_slice() {
        [] => Err("No settings file found. One can be loaded on open if placed in the same folder as the AutoSplit \
                   executable."
            .into()),
        [path] => UserProfile::load(path).map(|profile| (path.clone(), profile)).map_err(|error| error.to_string()),
        _ => Err("Too many settings files found. Only one can be loaded on open if placed in the same folder as the \
                  AutoSplit executable."
            .into()),
    }
}

/// Picks the renderer. With the GPU, Slint's own order is used: the wgpu renderer, then drawing
/// on the CPU when that can't make a window. Without, only drawing on the CPU.
fn select_backend(gpu: bool) -> Result<()> {
    let selector = slint::BackendSelector::new().backend_name("winit".into());
    let selector = if gpu { selector } else { selector.renderer_name("software".into()) };
    selector.select().map_err(|error| anyhow::anyhow!("Couldn't open a window: {error}"))
}

/// Opens the window, and runs until it's closed (or LiveSplit closes AutoSplit).
pub fn run(options: Options) -> Result<()> {
    let app = AppSettings::load();
    let gpu = app.use_gpu();
    select_backend(gpu)?;
    let main = MainWindow::new()?;
    // Only the GPU renderers take rendering notifiers, so this tells which one Slint picked
    let drawing_with_gpu = main.window().set_rendering_notifier(|_, _| {}).is_ok();

    let (events, receiver) = mpsc::channel();
    let (mut console, log_path) = Console::with_listener(Box::new(|line| {
        let line = line.to_owned();
        with_gui(move |gui| gui.add_log_line(line));
    }));
    console.log(format_args!(
        "AutoSplit {} drawing with {}",
        env!("CARGO_PKG_VERSION"),
        match (gpu, drawing_with_gpu) {
            (_, true) => "the GPU (wgpu)",
            (true, false) => "the CPU, as the GPU couldn't be used",
            (false, false) => "the CPU",
        }
    ));

    let mut first_messages = Vec::new();
    let (path, profile) = if options.auto_controlled {
        run::log_started_by_livesplit(&mut console, log_path.as_deref());
        (None, UserProfile::default())
    } else {
        match profile_on_open() {
            Ok((path, profile)) => {
                console.log(format_args!("Loaded {}", path.display()));
                (Some(path), profile)
            }
            Err(message) => {
                first_messages.push(message);
                (None, UserProfile::default())
            }
        }
    };
    let link = if options.auto_controlled { TimerLink::AutoControlled } else { profile.timer_connection.into() };

    let gui = Rc::new(Gui {
        main,
        settings: SettingsWindow::new()?,
        about: AboutWindow::new()?,
        log: LogWindow::new()?,
        message: MessageWindow::new()?,
        state: RefCell::new(State {
            profile: profile.clone(),
            path: path.clone(),
            saved: profile.clone(),
            devices: Vec::new(),
            setting_hotkey: None,
            held: Modifiers::empty(),
            started_with_gpu: gpu,
            started_with: profile.timer_connection,
            app,
            log: VecDeque::new(),
            log_path: log_path.clone(),
            messages: VecDeque::new(),
            showing_message: None,
        }),
        events: events.clone(),
    });
    GUI.with(|cell| *cell.borrow_mut() = Some(gui.clone()));
    gui.connect();
    gui.show_profile();
    gui.about.set_version(env!("CARGO_PKG_VERSION").into());
    gui.log.set_log_path(log_path.as_ref().map(|path| path.display().to_string()).unwrap_or_default().into());

    let session_profile = resolved(&profile, path.as_deref());
    let window = gui.main.as_weak();
    let session_events = events.clone();
    let session = std::thread::Builder::new().name("session".into()).spawn(move || {
        let view = GuiView::new(
            window,
            |message| with_gui(move |gui| gui.show_message(message, ["OK", "", ""], None)),
            |profile| with_gui(move |gui| gui.profile_changed(profile)),
            |path| with_gui(move |gui| gui.profile_loaded(path)),
        );
        let mut session = Session::new(session_profile, link, console, Box::new(view), session_events.clone());
        match link {
            TimerLink::AutoControlled => {
                if let Err(error) = run::connect_to_livesplit(&session_events) {
                    session.console().error(format_args!("Couldn't talk to LiveSplit: {error}"));
                }
                session.wait_for_profile();
            }
            TimerLink::LiveSplitOne => {
                let port = session.profile().livesplit_one_port;
                if let Err(error) = session.listen_for_livesplit_one(port) {
                    let message = error.to_string();
                    session.console().error(&message);
                    with_gui(move |gui| gui.show_message(message, ["OK", "", ""], None));
                }
            }
            TimerLink::Hotkeys => session.start_sending_hotkeys(),
        }
        session.start();
        session.run(&receiver);
        // LiveSplit closed AutoSplit, or the window was closed
        let _ = slint::invoke_from_event_loop(|| {
            let _ = slint::quit_event_loop();
        });
    })?;

    gui.main.show()?;
    if options.minimized {
        gui.main.window().set_minimized(true);
    }
    for message in first_messages {
        gui.state.borrow_mut().log.push_back(format!("error: {message}"));
        gui.show_message(message, ["OK", "", ""], None);
    }
    slint::run_event_loop_until_quit()?;
    let _ = events.send(Event::Stop);
    let _ = session.join();
    GUI.with(|cell| cell.borrow_mut().take());
    Ok(())
}

/// The profile with relative folders read from its own folder, for the session.
fn resolved(profile: &UserProfile, path: Option<&Path>) -> UserProfile {
    let mut profile = profile.clone();
    profile.resolve_directories(&path.and_then(Path::parent).map_or_else(exe_folder, Path::to_owned));
    profile
}

/// AutoSplit stores folders with forward slashes and a slash at the end.
fn folder_text(path: &Path) -> String {
    format!("{}/", path.display().to_string().replace('\\', "/").trim_end_matches('/'))
}

impl Gui {
    fn send(&self, command: UiCommand) {
        let _ = self.events.send(Event::Ui(command));
    }

    /// Sends the edited profile to the session.
    fn profile_edited(&self) {
        let state = self.state.borrow();
        let profile = resolved(&state.profile, state.path.as_deref());
        let folder = profile_folder(&state);
        drop(state);
        self.send(UiCommand::SetProfile { profile: Box::new(profile), folder });
        self.show_profile();
    }

    fn connect(self: &Rc<Self>) {
        let main = &self.main;
        let weak = Rc::downgrade(self);

        macro_rules! on {
            ($window:expr, $callback:ident, ($($argument:ident),*) $gui:ident => $body:expr) => {{
                let weak = weak.clone();
                $window.$callback(move |$($argument),*| {
                    if let Some($gui) = weak.upgrade() {
                        $body;
                    }
                });
            }};
        }

        on!(main, on_browse_folder, () gui => {
            let picked = rfd::FileDialog::new().set_title("Select Split Image Directory").pick_folder();
            if let Some(folder) = picked {
                gui.state.borrow_mut().profile.split_image_directory = folder_text(&folder);
                gui.profile_edited();
            }
        });
        on!(main, on_region_edited, (x, y, width, height) gui => {
            let region = &mut gui.state.borrow_mut().profile.capture_region;
            (region.x, region.y, region.width, region.height) = (x.into(), y.into(), width.into(), height.into());
            gui.profile_edited();
        });
        on!(main, on_align_region, () gui => {
            let folder = gui.state.borrow().profile.split_image_directory.clone();
            let picked = rfd::FileDialog::new()
                .set_title("Select Reference Image")
                .set_directory(if folder.is_empty() { exe_folder() } else { PathBuf::from(folder) })
                .add_filter("PNG image", &["png"])
                .pick_file();
            if let Some(path) = picked {
                gui.send(UiCommand::AlignRegion(path));
            }
        });
        on!(main, on_take_screenshot, () gui => gui.send(UiCommand::Screenshot));
        on!(main, on_check_fps, () gui => {
            gui.main.set_fps_value("...".into());
            gui.send(UiCommand::CheckFps);
        });
        on!(main, on_start_auto_splitter, () gui => gui.send(UiCommand::StartAutoSplitter));
        on!(main, on_reset, () gui => gui.send(UiCommand::Reset));
        on!(main, on_skip_split, () gui => gui.send(UiCommand::SkipSplit));
        on!(main, on_undo_split, () gui => gui.send(UiCommand::UndoSplit));
        on!(main, on_next_image, () gui => gui.send(UiCommand::NextImage));
        on!(main, on_previous_image, () gui => gui.send(UiCommand::PreviousImage));
        on!(main, on_reload_start_image, () gui => gui.send(UiCommand::ReloadStartImage));
        on!(main, on_open_settings, () gui => gui.open_settings());
        on!(main, on_save_profile, () gui => {
            gui.save_profile(false);
        });
        on!(main, on_save_profile_as, () gui => {
            gui.save_profile(true);
        });
        on!(main, on_load_profile, () gui => gui.load_profile());
        on!(main, on_view_help, () gui => gui.open_readme());
        on!(main, on_view_log, () gui => {
            let _ = gui.log.show();
        });
        on!(main, on_about, () gui => {
            let _ = gui.about.show();
        });
        on!(self.about, on_ok, () gui => {
            let _ = gui.about.hide();
        });
        on!(self.about, on_open_link, (link) _gui => screenshot::open_file(Path::new(link.as_str())));
        on!(self.log, on_open_log_file, () gui => {
            if let Some(path) = gui.state.borrow().log_path.clone() {
                screenshot::open_file(&path);
            }
        });
        on!(self.message, on_answered, (button) gui => gui.message_answered(button as usize));

        let settings = &self.settings;
        on!(settings, on_changed, (setting) gui => gui.settings_changed(&setting));
        on!(settings, on_capture_devices_requested, () gui => gui.refresh_devices());
        on!(settings, on_browse_screenshot_directory, () gui => {
            if let Some(folder) = rfd::FileDialog::new().set_title("Select Screenshot Directory").pick_folder() {
                gui.state.borrow_mut().profile.screenshot_directory = folder_text(&folder);
                gui.profile_edited();
            }
        });
        on!(settings, on_set_hotkey, (index) gui => {
            let index = index as usize;
            let mut state = gui.state.borrow_mut();
            state.setting_hotkey = if state.setting_hotkey == Some(index) { None } else { Some(index) };
            gui.settings.set_setting_hotkey(state.setting_hotkey.map_or(-1, |index| index as i32));
        });
        on!(settings, on_hotkey_edited, (index, text) gui => {
            let (action, _) = HOTKEY_ROWS[index as usize];
            *hotkey_mut(&mut gui.state.borrow_mut().profile, action) = text.trim().to_owned();
            gui.profile_edited();
        });
        on!(settings, on_open_readme, () gui => gui.open_readme());

        // Set Hotkey: the next key pressed while the settings window has focus
        let capture = weak.clone();
        self.settings.window().on_winit_window_event(move |_, event| {
            let Some(gui) = capture.upgrade() else { return EventResult::Propagate };
            match event {
                WindowEvent::ModifiersChanged(modifiers) => {
                    gui.state.borrow_mut().held = modifiers_held(modifiers.state());
                    EventResult::Propagate
                }
                WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                    let Some(index) = gui.state.borrow().setting_hotkey else { return EventResult::Propagate };
                    gui.key_pressed(index, event);
                    EventResult::PreventDefault
                }
                _ => EventResult::Propagate,
            }
        });

        // AutoSplit's keyboard shortcuts, wherever the focus is in the main window
        let shortcuts = weak.clone();
        let mut held = Modifiers::empty();
        self.main.window().on_winit_window_event(move |_, event| {
            let Some(gui) = shortcuts.upgrade() else { return EventResult::Propagate };
            match event {
                WindowEvent::ModifiersChanged(modifiers) => {
                    held = modifiers_held(modifiers.state());
                    EventResult::Propagate
                }
                WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                    let control = held.contains(Modifiers::CONTROL);
                    let shift = held.contains(Modifiers::SHIFT);
                    let character = key_character(event).map(|c| c.to_ascii_lowercase());
                    match (control, shift, character, &event.logical_key) {
                        (true, _, Some(','), _) => gui.open_settings(),
                        (true, true, Some('s'), _) => {
                            gui.save_profile(true);
                        }
                        (true, false, Some('s'), _) => {
                            gui.save_profile(false);
                        }
                        (true, _, Some('o'), _) => gui.load_profile(),
                        (false, _, _, Key::Named(NamedKey::F1)) => gui.open_readme(),
                        _ => return EventResult::Propagate,
                    }
                    EventResult::PreventDefault
                }
                _ => EventResult::Propagate,
            }
        });

        let close = weak.clone();
        self.main.window().on_close_requested(move || {
            if let Some(gui) = close.upgrade() {
                gui.close_requested();
            }
            slint::CloseRequestResponse::KeepWindowShown
        });
    }

    /// A key pressed while the settings window waits for a hotkey.
    fn key_pressed(&self, index: usize, event: &slint::winit_030::winit::event::KeyEvent) {
        let (action, _) = HOTKEY_ROWS[index];
        // Escape clears the hotkey, like in AutoSplit and LiveSplit
        if event.logical_key == Key::Named(NamedKey::Escape) {
            *hotkey_mut(&mut self.state.borrow_mut().profile, action) = String::new();
            return self.hotkey_set();
        }
        let PhysicalKey::Code(code) = event.physical_key else { return };
        let Ok(key) = format!("{code:?}").parse::<KeyCode>() else { return };
        let character = key_character(event);
        let held = self.state.borrow().held;
        // A modifier on its own: wait for the key it goes with
        let Some(name) = hotkey_name(key, character, held) else { return };
        let mut state = self.state.borrow_mut();
        // Like AutoSplit, a hotkey only belongs to one action
        for (other, _) in HOTKEY_ROWS {
            if other != action && *hotkey_mut(&mut state.profile, other) == name {
                hotkey_mut(&mut state.profile, other).clear();
            }
        }
        *hotkey_mut(&mut state.profile, action) = name;
        drop(state);
        self.hotkey_set();
    }

    fn hotkey_set(&self) {
        self.state.borrow_mut().setting_hotkey = None;
        self.settings.set_setting_hotkey(-1);
        self.profile_edited();
    }

    fn open_settings(&self) {
        self.show_profile();
        self.refresh_devices();
        let _ = self.settings.show();
    }

    /// Lists capture devices in the background, since some take a while to answer.
    fn refresh_devices(&self) {
        std::thread::spawn(|| {
            let devices = list_devices().unwrap_or_default();
            with_gui(move |gui| {
                gui.state.borrow_mut().devices = devices;
                gui.show_profile();
            });
        });
    }

    /// Puts the profile's values in the windows.
    fn show_profile(&self) {
        let state = self.state.borrow();
        let profile = &state.profile;
        let main = &self.main;
        main.set_split_image_folder(if profile.split_image_directory.is_empty() {
            "No Folder Selected".into()
        } else {
            profile.split_image_directory.as_str().into()
        });
        let region = profile.capture_region;
        main.set_region_x(region.x as i32);
        main.set_region_y(region.y as i32);
        main.set_region_width(region.width as i32);
        main.set_region_height(region.height as i32);

        let settings = &self.settings;
        settings.set_fps_limit(profile.fps_limit as i32);
        settings.set_live_capture_region(profile.live_capture_region);
        let mut methods = vec![SharedString::from("Video Capture Device")];
        let other_method = !profile.capture_method.is_empty() && profile.capture_method != VIDEO_CAPTURE_DEVICE;
        if other_method {
            methods.push(format!("{} (not supported yet)", capture_method_name(&profile.capture_method)).into());
        }
        settings.set_capture_methods(ModelRc::new(VecModel::from(methods)));
        settings.set_capture_method_index(i32::from(other_method));
        let device_names: Vec<SharedString> =
            state.devices.iter().map(|device| format!("* {}", device.name).into()).collect();
        let selected =
            autosplit_capture::pick_device(&state.devices, &profile.capture_device_name, profile.capture_device_id)
                .and_then(|(picked, _)| state.devices.iter().position(|device| device == picked));
        settings.set_capture_device_index(
            selected.map_or(if state.devices.is_empty() { 0 } else { -1 }, |index| index as i32),
        );
        settings.set_capture_devices(ModelRc::new(VecModel::from(device_names)));
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
        settings.set_default_similarity_threshold(format!("{:.2}", profile.default_similarity_threshold).into());
        settings.set_default_delay_time(profile.default_delay_time.to_string().into());
        settings.set_default_pause_time(format!("{:.2}", profile.default_pause_time).into());
        settings.set_loop_splits(profile.loop_splits);
        settings.set_start_also_resets(profile.start_also_resets);
        settings.set_enable_auto_reset(profile.enable_auto_reset);
        let rows: Vec<HotkeyRow> = HOTKEY_ROWS
            .iter()
            .map(|&(action, name)| HotkeyRow { name: name.into(), hotkey: action.hotkey(profile).into() })
            .collect();
        settings.set_hotkeys(ModelRc::new(VecModel::from(rows)));
        settings.set_timer_connection(i32::from(profile.timer_connection == TimerConnection::LiveSplitOne));
        settings.set_livesplit_one_port(profile.livesplit_one_port.to_string().into());
        settings.set_timer_connection_changed(profile.timer_connection != state.started_with);
        settings.set_hardware_acceleration(state.app.hardware_acceleration);
        settings.set_hardware_acceleration_changed(state.app.hardware_acceleration != state.started_with_gpu);
    }

    /// Reads a setting the settings window changed into the profile.
    fn settings_changed(&self, setting: &str) {
        let settings = &self.settings;
        let mut state = self.state.borrow_mut();
        if setting == "hardware_acceleration" {
            state.app.hardware_acceleration = settings.get_hardware_acceleration();
            if let Err(error) = state.app.save() {
                drop(state);
                return self.show_message(format!("Couldn't save the setting: {error}"), ["OK", "", ""], None);
            }
            drop(state);
            return self.show_profile();
        }
        let devices = state.devices.clone();
        let profile = &mut state.profile;
        match setting {
            "fps_limit" => profile.fps_limit = settings.get_fps_limit().into(),
            "live_capture_region" => profile.live_capture_region = settings.get_live_capture_region(),
            "capture_method" => {
                if settings.get_capture_method_index() == 0 {
                    profile.capture_method = VIDEO_CAPTURE_DEVICE.into();
                }
            }
            "capture_device" => {
                if let Some(device) =
                    usize::try_from(settings.get_capture_device_index()).ok().and_then(|index| devices.get(index))
                {
                    profile.capture_device_name = device.name.clone();
                    profile.capture_device_id = device.index as i64;
                    profile.capture_method = VIDEO_CAPTURE_DEVICE.into();
                }
            }
            "screenshot_directory" => profile.screenshot_directory = settings.get_screenshot_directory().into(),
            "open_screenshot" => profile.open_screenshot = settings.get_open_screenshot(),
            "screenshot_on" => {
                profile.screenshot_on = [
                    (settings.get_screenshot_on_split(), Command::Split),
                    (settings.get_screenshot_on_start(), Command::Start),
                    (settings.get_screenshot_on_pause(), Command::Pause),
                    (settings.get_screenshot_on_reset(), Command::Reset),
                    (settings.get_screenshot_on_skip(), Command::Skip),
                    (settings.get_screenshot_on_undo(), Command::Undo),
                ]
                .into_iter()
                .filter_map(|(on, command)| on.then_some(command))
                .collect();
            }
            "default_comparison_method" => {
                profile.default_comparison_method = settings.get_default_comparison_method().into();
            }
            "default_similarity_threshold" => match settings.get_default_similarity_threshold().parse::<f64>() {
                Ok(value) if (0.0..=1.0).contains(&value) => profile.default_similarity_threshold = value,
                _ => return,
            },
            "default_delay_time" => match settings.get_default_delay_time().parse::<i64>() {
                Ok(value) if value >= 0 => profile.default_delay_time = value,
                _ => return,
            },
            "default_pause_time" => match settings.get_default_pause_time().parse::<f64>() {
                Ok(value) if value >= 0.0 => profile.default_pause_time = value,
                _ => return,
            },
            "loop_splits" => profile.loop_splits = settings.get_loop_splits(),
            "start_also_resets" => profile.start_also_resets = settings.get_start_also_resets(),
            "enable_auto_reset" => profile.enable_auto_reset = settings.get_enable_auto_reset(),
            "timer_connection" => {
                profile.timer_connection = if settings.get_timer_connection() == 1 {
                    TimerConnection::LiveSplitOne
                } else {
                    TimerConnection::Hotkeys
                };
            }
            "livesplit_one_port" => match settings.get_livesplit_one_port().parse::<u16>() {
                Ok(port) if port > 0 => profile.livesplit_one_port = port,
                _ => return,
            },
            _ => return,
        }
        drop(state);
        // Typing in a field shouldn't move its cursor, so only these refresh the window
        if matches!(setting, "timer_connection" | "capture_device" | "capture_method") {
            self.profile_edited();
        } else {
            let state = self.state.borrow();
            let profile = resolved(&state.profile, state.path.as_deref());
            let folder = profile_folder(&state);
            drop(state);
            self.send(UiCommand::SetProfile { profile: Box::new(profile), folder });
        }
    }

    /// The session changed the profile: Align Region, or the toggle auto reset image hotkey.
    fn profile_changed(&self, profile: UserProfile) {
        let mut state = self.state.borrow_mut();
        state.profile.capture_region = profile.capture_region;
        state.profile.enable_auto_reset = profile.enable_auto_reset;
        drop(state);
        self.show_profile();
    }

    /// LiveSplit sent a profile to load.
    fn profile_loaded(&self, path: PathBuf) {
        let Ok(profile) = UserProfile::load(&path) else { return };
        let mut state = self.state.borrow_mut();
        state.profile = profile.clone();
        state.saved = profile;
        state.path = Some(path);
        drop(state);
        self.show_profile();
    }

    /// AutoSplit's Save Profile: to the loaded file, or asks for one.
    fn save_profile(&self, ask: bool) -> bool {
        let path = self.state.borrow().path.clone();
        let path = match path.filter(|_| !ask) {
            Some(path) => path,
            None => {
                let picked = rfd::FileDialog::new()
                    .set_title("Save Settings As")
                    .set_directory(exe_folder())
                    .set_file_name("settings.toml")
                    .add_filter("TOML", &["toml"])
                    .save_file();
                let Some(path) = picked else { return false };
                path
            }
        };
        let profile = self.state.borrow().profile.clone();
        if let Err(error) = profile.save(&path) {
            self.show_message(format!("Couldn't save {}: {error}", path.display()), ["OK", "", ""], None);
            return false;
        }
        let mut state = self.state.borrow_mut();
        state.saved = profile;
        state.path = Some(path);
        true
    }

    fn load_profile(&self) {
        let picked = rfd::FileDialog::new()
            .set_title("Load Profile")
            .set_directory(exe_folder())
            .add_filter("TOML", &["toml"])
            .pick_file();
        let Some(path) = picked else { return };
        let loaded = match profile::load(&path, None) {
            Ok((_, warnings)) => UserProfile::load(&path).map(|profile| (profile, warnings)),
            Err(error) => return self.show_message(format!("{error:#}"), ["OK", "", ""], None),
        };
        let (profile, warnings) = match loaded {
            Ok(loaded) => loaded,
            Err(error) => return self.show_message(error.to_string(), ["OK", "", ""], None),
        };
        for warning in warnings {
            self.show_message(warning, ["OK", "", ""], None);
        }
        let mut state = self.state.borrow_mut();
        state.profile = profile.clone();
        state.saved = profile;
        state.path = Some(path);
        drop(state);
        self.profile_edited();
    }

    fn open_readme(&self) {
        let readme = exe_folder().join("README.md");
        if readme.exists() {
            screenshot::open_file(&readme);
        } else {
            screenshot::open_file(Path::new("https://github.com/Toufool/AutoSplit#readme"));
        }
    }

    /// AutoSplit asks whether to save changes before closing.
    fn close_requested(self: &Rc<Self>) {
        let state = self.state.borrow();
        if state.profile == state.saved {
            drop(state);
            let _ = slint::quit_event_loop();
            return;
        }
        let name = state
            .path
            .as_ref()
            .and_then(|path| path.file_name())
            .map_or_else(|| "Untitled".to_owned(), |name| name.to_string_lossy().into_owned());
        drop(state);
        let gui = Rc::downgrade(self);
        self.show_message(
            format!("Do you want to save changes made to settings file {name}?"),
            ["Yes", "No", "Cancel"],
            Some(Box::new(move |answer| {
                let Some(gui) = gui.upgrade() else { return };
                let quit = match answer {
                    0 => gui.save_profile(false),
                    1 => true,
                    _ => false,
                };
                if quit {
                    let _ = slint::quit_event_loop();
                }
            })),
        );
    }

    fn add_log_line(&self, line: String) {
        let mut state = self.state.borrow_mut();
        state.log.push_back(line);
        while state.log.len() > LOG_LINES {
            state.log.pop_front();
        }
        let text: Vec<&str> = state.log.iter().map(String::as_str).collect();
        self.log.set_log(text.join("\n").into());
    }

    /// Shows a message box, after the ones already waiting.
    fn show_message(&self, text: String, buttons: [&'static str; 3], answered: Option<Answered>) {
        self.state.borrow_mut().messages.push_back(Message { text, buttons, answered });
        self.next_message();
    }

    fn next_message(&self) {
        let mut state = self.state.borrow_mut();
        if state.showing_message.is_some() {
            return;
        }
        let Some(message) = state.messages.pop_front() else { return };
        state.showing_message = Some(message.answered);
        drop(state);
        let [primary, secondary, tertiary] = message.buttons;
        self.message.set_message(message.text.into());
        self.message.set_primary(primary.into());
        self.message.set_secondary(secondary.into());
        self.message.set_tertiary(tertiary.into());
        let _ = self.message.show();
    }

    fn message_answered(&self, button: usize) {
        let _ = self.message.hide();
        let answered = self.state.borrow_mut().showing_message.take().flatten();
        if let Some(answered) = answered {
            answered(button);
        }
        self.next_message();
    }
}

/// The folder the profile's relative folders are read from.
fn profile_folder(state: &State) -> PathBuf {
    state.path.as_deref().and_then(Path::parent).map_or_else(exe_folder, Path::to_owned)
}

fn modifiers_held(state: slint::winit_030::winit::keyboard::ModifiersState) -> Modifiers {
    let mut held = Modifiers::empty();
    for (on, flag) in [
        (state.control_key(), Modifiers::CONTROL),
        (state.alt_key(), Modifiers::ALT),
        (state.shift_key(), Modifiers::SHIFT),
        (state.super_key(), Modifiers::META),
    ] {
        if on {
            held |= flag;
        }
    }
    held
}

/// What the key types without modifiers, from the keyboard layout.
fn key_character(event: &slint::winit_030::winit::event::KeyEvent) -> Option<char> {
    #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
    {
        use slint::winit_030::winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
        if let Key::Character(text) = event.key_without_modifiers() {
            return text.chars().next();
        }
    }
    match &event.logical_key {
        Key::Character(text) => text.chars().next(),
        _ => None,
    }
}
