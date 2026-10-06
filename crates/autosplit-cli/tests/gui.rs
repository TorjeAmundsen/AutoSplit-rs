//! The window's logic that doesn't need a window: the profile document, message boxes, the
//! settings fields, key names and the log.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use autosplit::gui::document::{Document, folder_text, profile_on_open};
use autosplit::gui::log::{LOG_LINES, LogLines};
use autosplit::gui::messages::{Answer, Buttons, Message, MessageQueue};
use autosplit::gui::recorder::key_code;
use autosplit::gui::settings_form::{SettingEdit, apply};
use autosplit_capture::DeviceInfo;
use autosplit_core::commands::Command;
use autosplit_core::profile::{TimerConnection, UserProfile, VIDEO_CAPTURE_DEVICE};
use autosplit_input::KeyCode;
use slint::winit_030::winit::keyboard;
use test_case::test_case;

#[test_case(Path::new("C:\\Splits\\OoT") => "C:/Splits/OoT"; "backslashes")]
#[test_case(Path::new("/home/me/splits/") => "/home/me/splits"; "trailing slash")]
#[test_case(Path::new("/") => "/"; "root")]
fn stores_folders(path: &Path) -> String {
    // Act, with the expected value checked by test_case
    folder_text(path)
}

#[test_case(&[] => Err(true); "no profile")]
#[test_case(&["settings.toml"] => Ok(PathBuf::from("settings.toml")); "one profile")]
#[test_case(&["settings.toml", "notes.txt"] => Ok(PathBuf::from("settings.toml")); "one profile among other files")]
#[test_case(&["a.toml", "b.toml"] => Err(false); "two profiles")]
fn picks_the_profile_on_open(files: &[&str]) -> Result<PathBuf, bool> {
    // Arrange
    let folder = tempfile::tempdir().unwrap();
    for file in files {
        std::fs::write(folder.path().join(file), "fps_limit = 30\n").unwrap();
    }

    // Act
    let found = profile_on_open(folder.path());

    // Assert, with the expected value checked by test_case: the file name, or whether the
    // error says none was found
    found
        .map(|(path, _)| PathBuf::from(path.file_name().unwrap()))
        .map_err(|message| message.starts_with("No settings file found"))
}

#[test]
fn new_profile_has_autosplits_region() {
    // Act
    let document = Document::untitled();

    // Assert
    let region = document.profile.capture_region;
    assert_eq!((region.x, region.y, region.width, region.height), (0, 0, 640, 480));
}

#[test]
fn new_profile_is_untitled() {
    // Act
    let document = Document::untitled();

    // Assert
    assert_eq!(document.name(), "Untitled");
}

#[test]
fn edits_are_changes() {
    // Arrange
    let mut document = Document::untitled();

    // Act
    document.profile.fps_limit = 30;

    // Assert
    assert!(document.has_changes());
}

#[test]
fn saving_clears_changes() {
    // Arrange
    let mut document = Document::untitled();
    document.profile.fps_limit = 30;

    // Act
    document.saved(PathBuf::from("/splits/oot.toml"), document.to_save());

    // Assert
    assert!(!document.has_changes());
}

#[test]
fn saved_profiles_capture_from_a_device() {
    // Arrange
    let document = Document::new(UserProfile::default(), None);

    // Act
    let saved = document.to_save();

    // Assert
    assert_eq!(saved.capture_method, VIDEO_CAPTURE_DEVICE);
}

#[test_case(Some("/splits/oot.toml") => PathBuf::from("/splits"); "profile's folder")]
#[test_case(None => PathBuf::from("/exe"); "executable's folder for a new profile")]
fn reads_relative_folders_from(path: Option<&str>) -> PathBuf {
    // Arrange
    let document = Document::new(UserProfile::default(), path.map(PathBuf::from));

    // Act, with the expected value checked by test_case
    document.folder(Path::new("/exe"))
}

#[test]
fn first_message_shows_at_once() {
    // Arrange
    let mut queue = MessageQueue::default();

    // Act
    let shown = queue.push(Message::error("first"));

    // Assert
    assert_eq!(shown, Some(("first".to_owned(), Buttons::Ok)));
}

#[test]
fn second_message_waits() {
    // Arrange
    let mut queue = MessageQueue::default();
    queue.push(Message::error("first"));

    // Act
    let shown = queue.push(Message::error("second"));

    // Assert
    assert_eq!(shown, None);
}

#[test]
fn answering_shows_the_next_message() {
    // Arrange
    let mut queue = MessageQueue::default();
    queue.push(Message::error("first"));
    queue.push(Message::error("second"));

    // Act
    let (_, next) = queue.answer(Some(0));

    // Assert
    assert_eq!(next, Some(("second".to_owned(), Buttons::Ok)));
}

#[test]
fn closing_a_message_shows_the_next_one() {
    // Arrange
    let mut queue = MessageQueue::default();
    queue.push(Message::error("first"));
    queue.push(Message::error("second"));

    // Act
    let (_, next) = queue.answer(None);

    // Assert
    assert_eq!(next, Some(("second".to_owned(), Buttons::Ok)));
}

#[test_case(Some(0) => Some(Answer::Yes); "yes")]
#[test_case(Some(1) => Some(Answer::No); "no")]
#[test_case(Some(2) => Some(Answer::Cancel); "cancel")]
#[test_case(None => Some(Answer::Cancel); "closed")]
fn questions_get_their_answer(button: Option<usize>) -> Option<Answer> {
    // Arrange
    let mut queue = MessageQueue::default();
    let answer = Rc::new(Cell::new(None));
    let given = answer.clone();
    queue.push(Message::question("Save?", move |answer| given.set(Some(answer))));

    // Act
    let (answered, _) = queue.answer(button);
    if let Some((answered, reply)) = answered {
        answered(reply);
    }

    // Assert, with the expected value checked by test_case
    answer.get()
}

fn devices() -> Vec<DeviceInfo> {
    vec![DeviceInfo { index: 0, name: "Webcam".into() }, DeviceInfo { index: 3, name: "OBS Virtual Camera".into() }]
}

#[test_case(SettingEdit::FpsLimit(19) => false; "fps below 20")]
#[test_case(SettingEdit::FpsLimit(241) => false; "fps above 240")]
#[test_case(SettingEdit::FpsLimit(144) => true; "fps in range")]
#[test_case(SettingEdit::DefaultSimilarityThreshold(1.5) => false; "threshold above 1")]
#[test_case(SettingEdit::DefaultSimilarityThreshold(0.95) => true; "threshold")]
#[test_case(SettingEdit::DefaultDelayTime(-1.0) => false; "negative delay")]
#[test_case(SettingEdit::DefaultPauseTime(f64::INFINITY) => false; "endless pause")]
#[test_case(SettingEdit::LiveSplitOnePort(0.0) => false; "port 0")]
#[test_case(SettingEdit::LiveSplitOnePort(16834.0) => true; "port")]
#[test_case(SettingEdit::CaptureDevice(-1) => false; "no device picked")]
#[test_case(SettingEdit::CaptureDevice(5) => false; "device gone")]
fn validates_settings(edit: SettingEdit) -> bool {
    // Arrange
    let mut profile = UserProfile::default();

    // Act, with the expected value checked by test_case
    apply(&mut profile, edit, &devices())
}

#[test]
fn invalid_setting_leaves_the_profile() {
    // Arrange
    let mut profile = UserProfile::default();

    // Act
    apply(&mut profile, SettingEdit::DefaultSimilarityThreshold(2.0), &devices());

    // Assert
    assert_eq!(profile, UserProfile::default());
}

#[test]
fn threshold_steps_keep_two_decimals() {
    // Arrange
    let mut profile = UserProfile::default();

    // Act
    apply(&mut profile, SettingEdit::DefaultSimilarityThreshold(0.9 + 0.01 + 0.01 + 0.01), &devices());

    // Assert
    assert_eq!(profile.default_similarity_threshold, 0.93);
}

#[test]
fn picking_a_device_saves_its_name_and_number() {
    // Arrange
    let mut profile = UserProfile::default();

    // Act
    apply(&mut profile, SettingEdit::CaptureDevice(1), &devices());

    // Assert
    assert_eq!(
        (profile.capture_device_name.as_str(), profile.capture_device_id, profile.capture_method.as_str()),
        ("OBS Virtual Camera", 3, VIDEO_CAPTURE_DEVICE)
    );
}

#[test]
fn screenshot_commands_are_in_autosplits_order() {
    // Arrange
    let mut profile = UserProfile::default();
    let edit =
        SettingEdit::ScreenshotOn { start: true, split: true, reset: false, skip: true, undo: false, pause: true };

    // Act
    apply(&mut profile, edit, &devices());

    // Assert
    assert_eq!(profile.screenshot_on, [Command::Split, Command::Start, Command::Pause, Command::Skip]);
}

#[test_case(1 => TimerConnection::LiveSplitOne; "livesplit one")]
#[test_case(0 => TimerConnection::Hotkeys; "hotkeys")]
fn timer_connection(index: i32) -> TimerConnection {
    // Arrange
    let mut profile = UserProfile::default();

    // Act
    apply(&mut profile, SettingEdit::TimerConnection(index), &devices());

    // Assert, with the expected value checked by test_case
    profile.timer_connection
}

#[test_case(keyboard::KeyCode::KeyA => Some(KeyCode::KeyA); "letter")]
#[test_case(keyboard::KeyCode::Numpad1 => Some(KeyCode::Numpad1); "keypad digit")]
#[test_case(keyboard::KeyCode::NumpadDecimal => Some(KeyCode::NumpadDecimal); "keypad decimal")]
#[test_case(keyboard::KeyCode::F12 => Some(KeyCode::F12); "function key")]
#[test_case(keyboard::KeyCode::PageUp => Some(KeyCode::PageUp); "named key")]
#[test_case(keyboard::KeyCode::SuperLeft => Some(KeyCode::MetaLeft); "left windows key")]
#[test_case(keyboard::KeyCode::SuperRight => Some(KeyCode::MetaRight); "right windows key")]
#[test_case(keyboard::KeyCode::ControlLeft => Some(KeyCode::ControlLeft); "control")]
fn reads_winit_keys(code: keyboard::KeyCode) -> Option<KeyCode> {
    // Act, with the expected value checked by test_case
    key_code(code)
}

#[test]
fn log_keeps_the_newest_lines() {
    // Arrange
    let mut log = LogLines::default();

    // Act
    for line in 0..LOG_LINES + 5 {
        log.push(line.to_string());
    }

    // Assert
    assert!(log.text().starts_with("5\n6\n"));
}

#[test]
fn log_text_has_a_line_each() {
    // Arrange
    let mut log = LogLines::default();
    log.push("Loaded settings.toml".into());

    // Act
    log.push("Run started".into());

    // Assert
    assert_eq!(log.text(), "Loaded settings.toml\nRun started");
}
