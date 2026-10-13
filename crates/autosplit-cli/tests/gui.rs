//! The window's logic that doesn't need a window: the profile document, message boxes, the
//! settings fields, key names, the log and where windows open.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use autosplit::gui::document::{Document, folder_text, profile_on_open};
use autosplit::gui::log::{LOG_LINES, LogLines};
use autosplit::gui::maker_plan::{WritePlan, looks_made, plan_writes};
use autosplit::gui::messages::{Answer, Buttons, Message, MessageQueue};
use autosplit::gui::placement::{Area, beside, centered_over};
use autosplit::gui::recorder::key_code;
use autosplit::gui::settings_form::{SettingEdit, Shown, apply, pick_default_device};
use autosplit::gui::view::{KeptFrames, Picture, Pixels, Update};
use autosplit::gui::{THUMB_SIZE, thumbnail};
use autosplit_capture::DeviceInfo;
use autosplit_core::commands::{Command, TimerLink};
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
        .map(|(path, _, _)| PathBuf::from(path.file_name().unwrap()))
        .map_err(|message| message.starts_with("No settings file found"))
}

#[test]
fn profile_on_open_warns_about_misplaced_keys() {
    // Arrange
    let folder = tempfile::tempdir().unwrap();
    std::fs::write(
        folder.path().join("settings.toml"),
        "[capture_region]\nx = 0\ny = 0\nwidth = 640\nheight = 480\nfps_limit = 30\n",
    )
    .unwrap();

    // Act
    let (_, _, warnings) = profile_on_open(folder.path()).unwrap();

    // Assert
    assert_eq!(
        warnings,
        ["'fps_limit' is inside [capture_region], where it's ignored. Move it above the [capture_region] line."]
    );
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
    assert_eq!(shown.map(|shown| (shown.text, shown.buttons)), Some(("first".to_owned(), Buttons::Ok)));
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
    assert_eq!(next.map(|shown| (shown.text, shown.buttons)), Some(("second".to_owned(), Buttons::Ok)));
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
    assert_eq!(next.map(|shown| (shown.text, shown.buttons)), Some(("second".to_owned(), Buttons::Ok)));
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
#[test_case(SettingEdit::LiveSplitOnePort(65536.0) => false; "port above 65535")]
#[test_case(SettingEdit::CaptureMethod(0) => true; "video capture device")]
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

#[test_case("", -1, VIDEO_CAPTURE_DEVICE => Some(("OBS Virtual Camera".to_owned(), 3)); "no device")]
#[test_case("", -1, "" => Some(("OBS Virtual Camera".to_owned(), 3)); "no capture method")]
#[test_case("Elgato", -1, VIDEO_CAPTURE_DEVICE => None; "device not connected")]
#[test_case("", 3, VIDEO_CAPTURE_DEVICE => None; "device by number")]
#[test_case("", -1, "BITBLT" => None; "window capture method")]
fn listing_devices_picks_obs_for_a_profile_without_one(
    name: &str,
    id: i64,
    capture_method: &str,
) -> Option<(String, i64)> {
    // Arrange
    let mut profile = UserProfile {
        capture_device_name: name.into(),
        capture_device_id: id,
        capture_method: capture_method.into(),
        ..UserProfile::default()
    };

    // Act
    let picked = pick_default_device(&mut profile, &devices());

    // Assert, with the expected value checked by test_case
    picked.then_some((profile.capture_device_name, profile.capture_device_id))
}

#[test]
fn without_obs_the_first_device_is_picked() {
    // Arrange
    let mut profile = UserProfile::default();
    let devices = [DeviceInfo { index: 0, name: "Webcam".into() }, DeviceInfo { index: 1, name: "Elgato".into() }];

    // Act
    let picked = pick_default_device(&mut profile, &devices);

    // Assert
    assert_eq!((picked, profile.capture_device_name.as_str(), profile.capture_device_id), (true, "Webcam", 0));
}

#[test]
fn listing_no_devices_picks_none() {
    // Arrange
    let mut profile = UserProfile::default();

    // Act
    let picked = pick_default_device(&mut profile, &[]);

    // Assert
    assert_eq!((picked, profile), (false, UserProfile::default()));
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
#[cfg_attr(target_os = "linux", test_case(2 => TimerConnection::Molok; "molok"))]
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

#[test]
fn picking_the_unsupported_method_keeps_the_profiles() {
    // Arrange
    let mut profile = UserProfile { capture_method: "BITBLT".into(), ..UserProfile::default() };

    // Act
    apply(&mut profile, SettingEdit::CaptureMethod(1), &devices());

    // Assert
    assert_eq!(profile.capture_method, "BITBLT");
}

#[test_case(10.004 => 10.0; "rounds down")]
#[test_case(10.126 => 10.13; "rounds up to two decimals")]
fn pause_time_keeps_two_decimals(seconds: f64) -> f64 {
    // Arrange
    let mut profile = UserProfile::default();

    // Act
    apply(&mut profile, SettingEdit::DefaultPauseTime(seconds), &devices());

    // Assert, with the expected value checked by test_case
    profile.default_pause_time
}

#[test]
fn choices_have_their_labels_and_default() {
    // Arrange
    let mut queue = MessageQueue::default();

    // Act
    let shown = queue.push(Message::choice("Delete them?", ["Delete", "Keep", "Cancel"], 1, |_| {})).unwrap();

    // Assert
    assert_eq!((shown.labels, shown.default), (["Delete", "Keep", "Cancel"], 1));
}

#[test]
fn errors_answered_with_any_button_say_ok() {
    // Arrange
    let mut queue = MessageQueue::default();
    queue.push(Message::error("broken"));

    // Act
    let (answered, next) = queue.answer(Some(2));

    // Assert
    assert_eq!((answered.is_none(), next), (true, None));
}

fn shown(capture_method: &str, devices: &[DeviceInfo], scanning: bool) -> Shown {
    let profile = UserProfile {
        capture_method: capture_method.into(),
        capture_device_name: "OBS Virtual Camera".into(),
        ..UserProfile::default()
    };
    Shown::new(&profile, devices, scanning, TimerLink::Hotkeys)
}

#[test_case(VIDEO_CAPTURE_DEVICE, false, false => ""; "devices listed")]
#[test_case(VIDEO_CAPTURE_DEVICE, true, false => "Scanning for existing devices..."; "scanning")]
#[test_case(VIDEO_CAPTURE_DEVICE, false, true => "No device found."; "none found")]
#[test_case("BITBLT", false, false => "Select \"Video Capture Device\" above"; "window capture method")]
fn device_placeholder(capture_method: &str, scanning: bool, no_devices: bool) -> &'static str {
    // Arrange
    let devices = if no_devices { Vec::new() } else { devices() };

    // Act, with the expected value checked by test_case
    shown(capture_method, &devices, scanning).device_placeholder
}

#[test_case(VIDEO_CAPTURE_DEVICE => (vec!["Video Capture Device (see below)".to_owned()], 0); "video capture device")]
#[test_case("BITBLT" => (vec!["Video Capture Device (see below)".to_owned(), "BitBlt (not supported)".to_owned()], 1); "window capture method")]
fn capture_methods(capture_method: &str) -> (Vec<String>, i32) {
    // Act
    let shown = shown(capture_method, &devices(), false);

    // Assert, with the expected value checked by test_case
    (shown.capture_methods, shown.capture_method_index)
}

#[test]
fn selects_the_profiles_device() {
    // Act
    let shown = shown(VIDEO_CAPTURE_DEVICE, &devices(), false);

    // Assert
    assert_eq!(shown.device_index, 1);
}

#[test]
fn missing_device_is_listed_as_not_connected() {
    // Arrange
    let webcam = [DeviceInfo { index: 0, name: "Webcam".into() }];

    // Act
    let shown = shown(VIDEO_CAPTURE_DEVICE, &webcam, false);

    // Assert
    assert_eq!(
        (shown.device_names, shown.device_index),
        (vec!["Webcam".to_owned(), "OBS Virtual Camera (not connected)".to_owned()], 1)
    );
}

#[test_case(TimerConnection::Hotkeys => 0; "hotkeys")]
#[test_case(TimerConnection::LiveSplitOne => 1; "livesplit one")]
#[cfg_attr(target_os = "linux", test_case(TimerConnection::Molok => 2; "molok"))]
#[cfg_attr(not(target_os = "linux"), test_case(TimerConnection::Molok => 0; "molok shows hotkeys off linux"))]
fn timer_connection_index(connection: TimerConnection) -> i32 {
    // Arrange
    let profile = UserProfile { timer_connection: connection, ..UserProfile::default() };

    // Act, with the expected value checked by test_case
    Shown::new(&profile, &[], false, TimerLink::from(connection)).timer_connection
}

#[test]
fn hotkeys_note_when_livesplit_controls_the_timer() {
    // Act
    let shown = Shown::new(&UserProfile::default(), &[], false, TimerLink::AutoControlled);

    // Assert
    assert_eq!(
        shown.hotkeys_note,
        "LiveSplit or LiveSplit One controls the timer, so only the screenshot and auto reset hotkeys are used."
    );
}

fn pixels(red: u8) -> Picture {
    let mut pixels = Pixels::new(1, 1);
    pixels.make_mut_bytes().copy_from_slice(&[red, 0, 0, 255]);
    Picture::Show(pixels)
}

fn red(picture: &Picture) -> Option<u8> {
    match picture {
        Picture::Show(pixels) => Some(pixels.as_bytes()[0]),
        Picture::Clear => Some(0),
        Picture::Keep => None,
    }
}

#[test_case(pixels(1), Picture::Keep => Some(1); "keeps the older picture")]
#[test_case(pixels(1), pixels(2) => Some(2); "newer picture wins")]
#[test_case(pixels(1), Picture::Clear => Some(0); "newer clear wins")]
#[test_case(Picture::Keep, Picture::Keep => None; "nothing to show")]
fn merging_updates_keeps_the_newest_picture(older: Picture, newer: Picture) -> Option<u8> {
    // Arrange
    let older = Update { live: older, ..Update::default() };
    let newer = Update { live: newer, ..Update::default() };

    // Act
    let merged = newer.after(older);

    // Assert, with the expected value checked by test_case
    red(&merged.live)
}

#[test]
fn merging_updates_keeps_the_newest_tiles() {
    // Arrange
    let older = Update {
        texts: None,
        live: Picture::Keep,
        current: Picture::Keep,
        tiles: [pixels(1), pixels(2), pixels(3), Picture::Keep],
        ..Update::default()
    };
    let newer = Update {
        texts: None,
        live: Picture::Keep,
        current: Picture::Keep,
        tiles: [Picture::Keep, pixels(4), Picture::Clear, Picture::Keep],
        ..Update::default()
    };

    // Act
    let merged = newer.after(older);

    // Assert
    assert_eq!(merged.tiles.each_ref().map(red), [Some(1), Some(4), Some(0), None]);
}

#[test_case(1, 1 => Some(2); "same request merges")]
#[test_case(1, 2 => None; "an older request's part is dropped")]
fn merging_updates_drops_the_testers_part_of_an_older_request(older: u64, newer: u64) -> Option<u8> {
    // Arrange
    let older = Update { tester_live: pixels(2), tester_request: older, ..Update::default() };
    let newer = Update { tester_request: newer, ..Update::default() };

    // Act
    let merged = newer.after(older);

    // Assert, with the expected value checked by test_case
    red(&merged.tester_live)
}

fn kept(cleared: bool, captions: &[&str]) -> KeptFrames {
    let frame = || Some(Pixels::new(1, 1));
    KeptFrames { cleared, added: captions.iter().map(|caption| (frame(), (*caption).to_owned())).collect() }
}

#[test_case(kept(false, &["a"]), kept(false, &["b"]) => (false, "a b".to_owned()); "newer frames come after")]
#[test_case(kept(true, &["a"]), kept(false, &["b"]) => (true, "a b".to_owned()); "older clear stays")]
#[test_case(kept(false, &["a"]), kept(true, &["b"]) => (true, "b".to_owned()); "newer clear drops the older frames")]
fn merging_kept_frames(older: KeptFrames, newer: KeptFrames) -> (bool, String) {
    // Act
    let merged = newer.after(older);

    // Assert, with the expected value checked by test_case
    (merged.cleared, merged.added.into_iter().map(|(_, caption)| caption).collect::<Vec<_>>().join(" "))
}

#[test_case(TimerConnection::LiveSplitOne, TimerLink::Hotkeys => true; "switch waiting for the run")]
#[test_case(TimerConnection::Hotkeys, TimerLink::Hotkeys => false; "same connection")]
#[test_case(TimerConnection::LiveSplitOne, TimerLink::AutoControlled => false; "livesplit controls the timer")]
fn timer_connection_waiting(connection: TimerConnection, link: TimerLink) -> bool {
    // Arrange
    let profile = UserProfile { timer_connection: connection, ..UserProfile::default() };

    // Act, with the expected value checked by test_case
    Shown::new(&profile, &[], false, link).timer_connection_waiting
}

const SECOND_SCREEN: Area = Area { x: 1920, y: 0, width: 2560, height: 1440 };

#[test_case(Area { x: 2500, y: 400, width: 600, height: 500 }, None => (2600, 450); "centered over the main window")]
#[test_case(Area { x: 2500, y: 400, width: 600, height: 500 }, Some(SECOND_SCREEN) => (2600, 450); "on the main window's screen")]
#[test_case(Area { x: 1920, y: 0, width: 200, height: 100 }, Some(SECOND_SCREEN) => (1920, 0); "kept inside the top left")]
#[test_case(Area { x: 4280, y: 1240, width: 200, height: 200 }, Some(SECOND_SCREEN) => (4080, 1040); "kept inside the bottom right")]
fn opens_over_the_main_window(main: Area, screen: Option<Area>) -> (i32, i32) {
    centered_over(main, 400, 400, screen)
}

#[test]
fn window_bigger_than_the_screen_keeps_its_corner_on_it() {
    assert_eq!(centered_over(SECOND_SCREEN, 3000, 2000, Some(SECOND_SCREEN)), (1920, 0));
}

#[test_case(Area { x: 2000, y: 100, width: 800, height: 450 } => (2800, 100); "on the right")]
#[test_case(Area { x: 3600, y: 100, width: 800, height: 450 } => (2600, 100); "on the left when the right is off screen")]
#[test_case(Area { x: 2600, y: 1200, width: 800, height: 450 } => (3400, 1040); "kept inside the bottom")]
#[test_case(Area { x: 2400, y: 100, width: 1900, height: 450 } => (2850, 125); "centered when neither side fits")]
fn tester_opens_beside_the_main_window(main: Area) -> (i32, i32) {
    beside(main, 1000, 400, Some(SECOND_SCREEN))
}

fn plan(names: &[&str], existing: &[&str]) -> WritePlan {
    plan_with_sources(names, existing, &[])
}

fn plan_with_sources(names: &[&str], existing: &[&str], sources: &[&str]) -> WritePlan {
    let names: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
    let existing: Vec<PathBuf> = existing.iter().map(PathBuf::from).collect();
    let sources: Vec<PathBuf> = sources.iter().map(PathBuf::from).collect();
    plan_writes(&names, &existing, &sources)
}

#[test]
fn saving_replaces_files_with_the_same_name() {
    // Act
    let plan = plan(&["05_Bottle_(0.93).png"], &["05_Bottle_(0.93).png", "06_Boss_(0.9).png"]);

    // Assert
    assert_eq!(plan, WritePlan { overwrites: vec!["05_Bottle_(0.93).png".into()], stale: vec![] });
}

#[test_case(&["05_Bottle_(0.93).png"], &["05_Bottle_(0.9).png"] => vec![PathBuf::from("05_Bottle_(0.9).png")]; "another threshold")]
#[test_case(&["05_Bottle_(0.93).png"], &["5_Bottle_(0.93).png"] => vec![PathBuf::from("5_Bottle_(0.93).png")]; "fewer digits")]
#[test_case(&["reset_(0.96).png"], &["reset_(0.9)_[1].png"] => vec![PathBuf::from("reset_(0.9)_[1].png")]; "reset image")]
#[test_case(&["05_Bottle_(0.93).png"], &["06_Boss_(0.9).png", "notes.txt", "base"] => Vec::<PathBuf>::new(); "other splits and files")]
fn saving_finds_older_images_of_the_same_splits(names: &[&str], existing: &[&str]) -> Vec<PathBuf> {
    // Act, with the expected value checked by test_case
    plan(names, existing).stale
}

#[test]
fn screenshots_saved_from_are_not_older_images() {
    // Arrange: screenshots count from 001, so 002 looks like split 02's
    let names = ["01_Deku_(0.95).png", "02_Bottle_(0.93).png"];
    let existing = ["001_shot.png", "002_shot.png", "02_Bottle_(0.9).png"];

    // Act
    let plan = plan_with_sources(&names, &existing, &["001_shot.png", "002_shot.png"]);

    // Assert
    assert_eq!(plan.stale, vec![PathBuf::from("02_Bottle_(0.9).png")]);
}

#[test_case("002_SplitImage.png"; "screenshot name")]
#[test_case("002_splitimage.PNG"; "screenshot name in other case")]
fn screenshots_are_not_older_images(screenshot: &str) {
    // Act
    let plan = plan(&["02_Bottle_(0.93).png"], &[screenshot]);

    // Assert
    assert_eq!(plan.stale, Vec::<PathBuf>::new());
}

#[test]
fn thumbnails_are_the_whole_shot_shrunk_and_opaque() {
    // Arrange: blue on the left, green on the right, half transparent
    let shot = autosplit_core::image::BgraImage::from_raw(
        4,
        2,
        (0..8).flat_map(|i| if i % 4 < 2 { [255, 0, 0, 128] } else { [0, 255, 0, 128] }).collect(),
    );

    // Act
    let thumb = thumbnail(shot.view());

    // Assert: the corners keep their colors, nothing's masked away
    let pixels = thumb.as_slice();
    let (width, height) = THUMB_SIZE;
    let rgba = |index: usize| [pixels[index].r, pixels[index].g, pixels[index].b, pixels[index].a];
    assert_eq!(
        ((thumb.width() as usize, thumb.height() as usize), rgba(0), rgba(width * height - 1)),
        (THUMB_SIZE, [0, 0, 255, 255], [0, 255, 0, 255])
    );
}

#[test_case("001_SplitImage.png" => false; "screenshot")]
#[test_case("013_SplitImage - Kopi.png" => false; "copied screenshot")]
#[test_case("boss fight.png" => false; "any picture")]
#[test_case("05_Bottle_(0.93).png" => true; "made split image")]
#[test_case("reset.png" => true; "reset image")]
#[test_case("02_Dummy_{d}.png" => true; "flags only")]
#[test_case("03_EnterDeku.png" => true; "number and name, default threshold")]
#[test_case("Copy of 004_splitimage.PNG" => false; "screenshot inside a longer name")]
#[test_case("Copy of 001_SplitImage.png" => false; "copy of a screenshot")]
#[test_case("Copy of 000_SplitImage(1).png" => false; "numbered copy, which reads as a threshold")]
#[test_case("000_SplitImage (2) [3].png" => false; "copy with more brackets")]
fn screenshots_dont_look_like_split_images(name: &str) -> bool {
    // Act, with the expected value checked by test_case
    looks_made(name)
}
