//! The session as the window sees it, driven by events without a window or capture device.

use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use autosplit::console::Console;
use autosplit::screenshot::Saved;
use autosplit::session::{Event, Notice, Session, Snapshot, UiCommand, View};
use autosplit_core::commands::TimerLink;
use autosplit_core::hotkeys::HotkeyAction;
use autosplit_core::image::BgraImage;
use autosplit_core::profile::{TimerConnection, UserProfile};
use autosplit_core::text::window_texts;

/// The Live Tester's image and threshold, while it's open.
type Tested = Option<(String, f64)>;

/// What the session showed and told the window.
#[derive(Clone, Default)]
struct Seen {
    idle: Arc<Mutex<Vec<String>>>,
    notices: Arc<Mutex<Vec<String>>>,
    current_files: Arc<Mutex<Vec<String>>>,
    current_messages: Arc<Mutex<Vec<String>>>,
    testers: Arc<Mutex<Vec<Tested>>>,
}

impl Seen {
    fn last_idle(&self) -> String {
        self.idle.lock().unwrap().last().cloned().unwrap_or_default()
    }

    fn notices(&self) -> Vec<String> {
        self.notices.lock().unwrap().clone()
    }

    fn last_current_file(&self) -> String {
        self.current_files.lock().unwrap().last().cloned().unwrap_or_default()
    }

    fn last_tester(&self) -> Tested {
        self.testers.lock().unwrap().last().cloned().flatten()
    }
}

struct FakeView(Seen);

impl View for FakeView {
    fn interval(&self, _profile: &UserProfile) -> Duration {
        Duration::from_millis(5)
    }

    fn wants_frames(&self, _profile: &UserProfile) -> bool {
        false
    }

    fn update(&mut self, snapshot: Snapshot<'_>) {
        let texts = window_texts(snapshot.status, snapshot.now, &snapshot.idle, snapshot.receiving, true);
        self.0.current_files.lock().unwrap().push(texts.current_file);
        self.0.current_messages.lock().unwrap().push(texts.current_message);
        self.0.idle.lock().unwrap().push(snapshot.idle);
        let tester = snapshot.tester.map(|tester| (tester.image().display_name(), tester.threshold()));
        self.0.testers.lock().unwrap().push(tester);
    }

    fn notice(&mut self, notice: Notice) {
        let text = match notice {
            Notice::Dialog(message) => format!("dialog: {message}"),
            Notice::AutoResetToggled(on) => format!("auto reset {on}"),
            Notice::ProfileLoaded { .. } => "profile loaded".into(),
            Notice::TimerLinkChanged(link) => format!("timer link {link:?}"),
            Notice::Grabbed(image) => format!("grabbed {:?}", image.map(|image| image.width())),
            Notice::ScreenshotSaved(path) => format!("screenshot {}", path.display()),
            Notice::FullFrameRegion(region) => format!("full frame {}x{}", region.width, region.height),
            Notice::CroppedRegion(region) => format!("cropped {}x{}", region.width, region.height),
        };
        self.0.notices.lock().unwrap().push(text);
    }
}

/// Runs a session for a moment with `events`, then stops it.
fn run(profile: UserProfile, events: Vec<Event>) -> Seen {
    run_after(profile, || {}, events)
}

/// Runs a session for a moment with `events`, after the session set itself up and then
/// `meanwhile` ran, then stops it.
fn run_after(profile: UserProfile, meanwhile: impl FnOnce(), events: Vec<Event>) -> Seen {
    let seen = Seen::default();
    let (sender, receiver) = mpsc::channel();
    let mut session = Session::new(
        profile,
        TimerLink::Hotkeys,
        Console::silent(),
        Some(Box::new(FakeView(seen.clone()))),
        sender.clone(),
    );
    session.start();
    meanwhile();
    for event in events {
        sender.send(event).unwrap();
    }
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        let _ = sender.send(Event::Stop);
    });
    session.run(&receiver);
    seen
}

/// A folder with one split image.
fn split_folder() -> tempfile::TempDir {
    let folder = tempfile::tempdir().unwrap();
    let image = BgraImage::from_raw(8, 8, [10, 20, 30, 255].repeat(64));
    std::fs::write(folder.path().join("001_split.png"), image.encode_png().unwrap()).unwrap();
    folder
}

/// A folder with a start image named `start` and one split image.
fn start_folder(start: &str) -> tempfile::TempDir {
    let folder = split_folder();
    let image = BgraImage::from_raw(8, 8, [40, 50, 60, 255].repeat(64));
    std::fs::write(folder.path().join(start), image.encode_png().unwrap()).unwrap();
    folder
}

/// A profile for `folder` with a split hotkey.
fn folder_profile(folder: &tempfile::TempDir) -> UserProfile {
    UserProfile {
        split_hotkey: "num 1".into(),
        split_image_directory: folder.path().to_string_lossy().into_owned(),
        ..UserProfile::default()
    }
}

/// Replaces the folder's start image with one named `start`, after the session loaded it.
fn rename_start(folder: &tempfile::TempDir, start: &str) -> impl FnOnce() {
    let (from, to) = (folder.path().join("start_auto_splitter_first.png"), folder.path().join(start));
    move || std::fs::rename(from, to).unwrap()
}

#[test]
fn reload_split_images_reads_the_folder_again() {
    // Arrange
    let folder = start_folder("start_auto_splitter_first.png");
    let changed = rename_start(&folder, "start_auto_splitter_second.png");

    // Act
    let seen = run_after(folder_profile(&folder), changed, vec![Event::Ui(UiCommand::ReloadSplitImages)]);

    // Assert
    assert_eq!(seen.last_current_file(), "start_auto_splitter_second.png");
}

#[test]
fn folder_isnt_read_again_without_reload() {
    // Arrange
    let folder = start_folder("start_auto_splitter_first.png");
    let changed = rename_start(&folder, "start_auto_splitter_second.png");

    // Act
    let seen = run_after(folder_profile(&folder), changed, Vec::new());

    // Assert
    assert_eq!(seen.last_current_file(), "start_auto_splitter_first.png");
}

#[test]
fn another_folder_shows_its_start_image() {
    // Arrange
    let (first, second) =
        (start_folder("start_auto_splitter_first.png"), start_folder("start_auto_splitter_second.png"));
    let switch =
        UiCommand::SetProfile { profile: Box::new(folder_profile(&second)), folder: ".".into(), loaded: false };

    // Act
    let seen = run(folder_profile(&first), vec![Event::Ui(switch)]);

    // Assert
    assert_eq!(seen.last_current_file(), "start_auto_splitter_second.png");
}

#[test]
fn setting_a_needed_hotkey_makes_the_folder_usable() {
    // Arrange: a reset image needs the reset hotkey
    let folder = start_folder("reset.png");
    let with_reset = UserProfile { reset_hotkey: "num 3".into(), ..folder_profile(&folder) };
    let set = UiCommand::SetProfile { profile: Box::new(with_reset), folder: ".".into(), loaded: false };

    // Act
    let seen = run(folder_profile(&folder), vec![Event::Ui(set)]);

    // Assert: the folder can be used. Whether the run can start also depends on the hotkeys
    // being heard, which CI on Linux can't, without access to the keyboards.
    let idle = seen.last_idle();
    assert!(!idle.contains("fix the split images"), "{idle}");
    assert!(idle == "ready: press num 1 to start" || idle.contains("hotkeys don't work"), "{idle}");
}

#[test]
fn endless_start_pause_keeps_the_session_running() {
    // Arrange: the split hotkey starts the run, which pauses for the start image's [inf]
    let folder = start_folder("start_auto_splitter_[inf].png");
    let profile = UserProfile {
        split_hotkey: "num 1".into(),
        split_image_directory: folder.path().to_string_lossy().into_owned(),
        ..UserProfile::default()
    };

    // Act
    let seen = run(profile, vec![Event::Hotkey(HotkeyAction::Split, Instant::now())]);

    // Assert
    let messages = seen.current_messages.lock().unwrap().clone();
    assert!(messages.iter().any(|message| message.starts_with("None (Paused).")), "{messages:?}");
}

#[test]
fn window_texts_point_to_the_log_window() {
    // Arrange
    let profile = UserProfile { split_image_directory: "/no/such/folder".into(), ..UserProfile::default() };

    // Act
    let seen = run(profile, Vec::new());

    // Assert
    assert_eq!(
        seen.last_idle(),
        "can't start: fix the split images, then press Reload Split Images (see Help > View Log)"
    );
}

#[test]
fn toggle_auto_reset_hotkey_tells_the_window() {
    // Arrange
    let profile = UserProfile { enable_auto_reset: true, ..UserProfile::default() };

    // Act
    let seen = run(profile, vec![Event::Hotkey(HotkeyAction::ToggleAutoResetImage, Instant::now())]);

    // Assert
    assert_eq!(seen.notices(), ["auto reset false"]);
}

#[test]
fn starting_without_a_split_hotkey_shows_one_dialog() {
    // Arrange
    let folder = split_folder();
    let profile =
        UserProfile { split_image_directory: folder.path().to_string_lossy().into_owned(), ..UserProfile::default() };

    // Act
    let seen = run(profile, vec![Event::Ui(UiCommand::StartAutoSplitter)]);

    // Assert
    assert_eq!(seen.notices(), ["dialog: No split hotkey is set, so the timer can't be started."]);
}

#[test]
fn starting_without_a_split_hotkey_leaves_the_folder_usable() {
    // Arrange
    let folder = split_folder();
    let profile =
        UserProfile { split_image_directory: folder.path().to_string_lossy().into_owned(), ..UserProfile::default() };

    // Act
    let seen = run(profile, vec![Event::Ui(UiCommand::StartAutoSplitter)]);

    // Assert
    assert_eq!(seen.last_idle(), "can't start: the profile has no split hotkey");
}

#[test]
fn timer_connection_switches_between_runs() {
    // Arrange
    let profile = UserProfile {
        timer_connection: TimerConnection::LiveSplitOne,
        // Any free port
        livesplit_one_port: 0,
        ..UserProfile::default()
    };
    let folder = std::env::temp_dir();
    let set = Event::Ui(UiCommand::SetProfile { profile: Box::new(profile), folder, loaded: false });

    // Act
    let seen = run(UserProfile::default(), vec![set]);

    // Assert
    assert_eq!(seen.notices(), ["timer link LiveSplitOne"]);
}

/// Starts a run with the split hotkey, then asks for LiveSplit One.
fn switch_during_a_run(folder: &tempfile::TempDir) -> Vec<Event> {
    let profile = UserProfile {
        timer_connection: TimerConnection::LiveSplitOne,
        // Any free port
        livesplit_one_port: 0,
        ..folder_profile(folder)
    };
    let set = Event::Ui(UiCommand::SetProfile { profile: Box::new(profile), folder: ".".into(), loaded: false });
    vec![Event::Hotkey(HotkeyAction::Split, Instant::now()), set]
}

#[test]
fn timer_connection_waits_for_the_run_to_end() {
    // Arrange
    let folder = split_folder();

    // Act
    let seen = run(folder_profile(&folder), switch_during_a_run(&folder));

    // Assert
    assert_eq!(seen.notices(), Vec::<String>::new());
}

#[test]
fn timer_connection_switches_when_the_run_ends() {
    // Arrange
    let folder = split_folder();
    let mut events = switch_during_a_run(&folder);
    events.push(Event::Ui(UiCommand::Reset));

    // Act
    let seen = run(folder_profile(&folder), events);

    // Assert
    assert_eq!(seen.notices(), ["timer link LiveSplitOne"]);
}

fn open_tester(path: std::path::PathBuf) -> Event {
    Event::Ui(UiCommand::OpenTester { path, request: 1 })
}

#[test]
fn live_tester_uses_the_profiles_defaults() {
    // Arrange
    let folder = split_folder();
    let profile = |threshold| UserProfile { default_similarity_threshold: threshold, ..UserProfile::default() };
    let set = Event::Ui(UiCommand::SetProfile { profile: Box::new(profile(0.5)), folder: ".".into(), loaded: false });

    // Act
    let seen = run(profile(0.9), vec![open_tester(folder.path().join("001_split.png")), set]);

    // Assert
    assert_eq!(seen.last_tester(), Some(("001_split.png".into(), 0.5)));
}

#[test]
fn closing_the_live_tester_drops_its_image() {
    // Arrange
    let folder = split_folder();
    let close = Event::Ui(UiCommand::CloseTester);

    // Act
    let seen = run(UserProfile::default(), vec![open_tester(folder.path().join("001_split.png")), close]);

    // Assert
    assert_eq!(seen.last_tester(), None);
}

#[test]
fn live_tester_with_an_unusable_image_shows_a_dialog() {
    // Arrange
    let folder = tempfile::tempdir().unwrap();
    let image = folder.path().join("notes.png");
    std::fs::write(&image, "not an image").unwrap();

    // Act
    let seen = run(UserProfile::default(), vec![open_tester(image)]);

    // Assert
    assert_eq!(
        (seen.notices(), seen.last_tester()),
        (vec!["dialog: 'notes.png' isn't a PNG file. Only PNG split images are supported.".to_owned()], None)
    );
}

// The shortcut is Cmd+, on macOS, like its other shortcuts
#[cfg(target_os = "macos")]
const NO_CAPTURE: &str = "grabbed Err(\"There's no capture to grab. Check the capture device in Settings (Cmd+,).\")";
#[cfg(not(target_os = "macos"))]
const NO_CAPTURE: &str = "grabbed Err(\"There's no capture to grab. Check the capture device in Settings (Ctrl+,).\")";

#[test]
fn grab_without_a_capture_says_so() {
    // Act
    let seen = run(UserProfile::default(), vec![Event::Ui(UiCommand::SetMakerOpen(true)), Event::Ui(UiCommand::Grab)]);

    // Assert
    assert_eq!(seen.notices(), [NO_CAPTURE]);
}

#[test]
fn screenshot_hotkey_grabs_while_making_split_images() {
    // Act
    let events =
        vec![Event::Ui(UiCommand::SetMakerOpen(true)), Event::Hotkey(HotkeyAction::Screenshot, Instant::now())];
    let seen = run(UserProfile::default(), events);

    // Assert
    assert_eq!(seen.notices(), [NO_CAPTURE]);
}

#[test]
fn saved_screenshots_are_shown_and_failures_are_errors() {
    // Arrange
    let path = std::path::PathBuf::from("001_SplitImage.png");
    let events = vec![
        Event::Screenshot(Saved::Written(path.clone())),
        Event::Screenshot(Saved::NotOpened(path, "no app".into())),
        Event::Screenshot(Saved::Failed("Screenshot: couldn't write it".into())),
    ];

    // Act
    let seen = run(UserProfile::default(), events);

    // Assert
    assert_eq!(
        seen.notices(),
        [
            "screenshot 001_SplitImage.png",
            "screenshot 001_SplitImage.png",
            "dialog: Saved screenshot 001_SplitImage.png, but couldn't open it: no app",
            "dialog: Screenshot: couldn't write it",
        ]
    );
}

#[test]
fn folder_isnt_read_while_making_split_images() {
    // Arrange
    let folder = start_folder("start_auto_splitter_first.png");
    let changed = rename_start(&folder, "start_auto_splitter_second.png");
    let events = vec![Event::Ui(UiCommand::SetMakerOpen(true)), Event::Ui(UiCommand::ReloadSplitImages)];

    // Act
    let seen = run_after(folder_profile(&folder), changed, events);

    // Assert
    assert_eq!(seen.last_current_file(), "start_auto_splitter_first.png");
}

#[test]
fn folder_is_read_once_making_split_images_ends() {
    // Arrange
    let folder = start_folder("start_auto_splitter_first.png");
    let changed = rename_start(&folder, "start_auto_splitter_second.png");
    let events = vec![
        Event::Ui(UiCommand::SetMakerOpen(true)),
        Event::Ui(UiCommand::ReloadSplitImages),
        Event::Ui(UiCommand::SetMakerOpen(false)),
    ];

    // Act
    let seen = run_after(folder_profile(&folder), changed, events);

    // Assert
    assert_eq!(seen.last_current_file(), "start_auto_splitter_second.png");
}

#[test]
fn half_made_folder_shows_no_error_while_making_split_images() {
    // Arrange: only the start and reset images saved so far, which aren't enough for a run
    let folder = tempfile::tempdir().unwrap();
    let image = BgraImage::from_raw(8, 8, [40, 50, 60, 255].repeat(64)).encode_png().unwrap();
    std::fs::write(folder.path().join("start_auto_splitter_(0.95).png"), &image).unwrap();
    std::fs::write(folder.path().join("reset_(0.95).png"), &image).unwrap();
    let events = vec![Event::Ui(UiCommand::SetMakerOpen(true)), Event::Ui(UiCommand::ReloadSplitImages)];

    // Act
    let seen = run(folder_profile(&folder), events);

    // Assert
    assert!(seen.notices().iter().all(|notice| !notice.starts_with("dialog")), "{:?}", seen.notices());
}
