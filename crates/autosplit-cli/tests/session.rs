//! The session as the window sees it, driven by events without a window or capture device.

use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use autosplit::console::Console;
use autosplit::session::{Event, Notice, Session, Snapshot, UiCommand, View};
use autosplit_core::commands::TimerLink;
use autosplit_core::hotkeys::HotkeyAction;
use autosplit_core::image::BgraImage;
use autosplit_core::profile::{TimerConnection, UserProfile};
use autosplit_core::text::window_texts;

/// What the session showed and told the window.
#[derive(Clone, Default)]
struct Seen {
    idle: Arc<Mutex<Vec<String>>>,
    notices: Arc<Mutex<Vec<String>>>,
    current_files: Arc<Mutex<Vec<String>>>,
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
        let texts = window_texts(snapshot.status, snapshot.now, &snapshot.idle, snapshot.receiving);
        self.0.current_files.lock().unwrap().push(texts.current_file);
        self.0.idle.lock().unwrap().push(snapshot.idle);
    }

    fn notice(&mut self, notice: Notice) {
        let text = match notice {
            Notice::Dialog(message) => format!("dialog: {message}"),
            Notice::AutoResetToggled(on) => format!("auto reset {on}"),
            Notice::RegionAligned(_) => "region aligned".into(),
            Notice::ProfileLoaded { .. } => "profile loaded".into(),
            Notice::TimerLinkChanged(link) => format!("timer link {link:?}"),
        };
        self.0.notices.lock().unwrap().push(text);
    }
}

/// Runs a session for a moment with `events`, then stops it.
fn run(profile: UserProfile, events: Vec<Event>) -> Seen {
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

#[test]
fn reload_split_images_shows_the_new_folders_start_image() {
    // Arrange
    let (first, second) =
        (start_folder("start_auto_splitter_first.png"), start_folder("start_auto_splitter_second.png"));
    let profile = |folder: &tempfile::TempDir| UserProfile {
        split_hotkey: "num 1".into(),
        split_image_directory: folder.path().to_string_lossy().into_owned(),
        ..UserProfile::default()
    };
    let switch = UiCommand::SetProfile { profile: Box::new(profile(&second)), folder: ".".into(), loaded: false };

    // Act
    let seen = run(profile(&first), vec![Event::Ui(switch), Event::Ui(UiCommand::ReloadSplitImages)]);

    // Assert
    assert_eq!(seen.last_current_file(), "start_auto_splitter_second.png");
}

#[test]
fn window_texts_point_to_the_log_window() {
    // Arrange
    let profile = UserProfile { split_image_directory: "/no/such/folder".into(), ..UserProfile::default() };

    // Act
    let seen = run(profile, Vec::new());

    // Assert
    assert_eq!(seen.last_idle(), "can't start: fix the split images, then press Reload Split Images (see Help > View Log)");
}

#[test]
fn toggle_auto_reset_hotkey_tells_the_window() {
    // Arrange
    let profile = UserProfile { enable_auto_reset: true, ..UserProfile::default() };

    // Act
    let seen = run(profile, vec![Event::Hotkey(HotkeyAction::ToggleAutoResetImage)]);

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
