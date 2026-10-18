//! Profiles written by AutoSplit (tests/fixtures/profiles, made with tomli_w) load unchanged.

use std::path::{Path, PathBuf};

use autosplit_core::commands::Command;
use autosplit_core::image::Rect;
use autosplit_core::profile::{
    NEW_PROFILE_REGION, ProfileError, Region, RegionFit, TimerConnection, UserProfile, capture_method_name,
};
use test_case::test_case;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/profiles").join(name)
}

fn configured() -> UserProfile {
    UserProfile {
        split_hotkey: "num 1".into(),
        reset_hotkey: "num 3".into(),
        undo_split_hotkey: "num 8".into(),
        skip_split_hotkey: "num 2".into(),
        pause_hotkey: "ctrl+shift+p".into(),
        fps_limit: 144,
        capture_method: "VIDEO_CAPTURE_DEVICE".into(),
        capture_device_id: 2,
        capture_device_name: "OBS Virtual Camera".into(),
        default_comparison_method: 2,
        default_similarity_threshold: 0.96,
        default_delay_time: 150,
        default_pause_time: 2.5,
        start_also_resets: true,
        split_image_directory: "C:/Users/Runner/Splits/OoT Any%/".into(),
        screenshot_on: vec![Command::Split, Command::Reset],
        captured_window_title: "Windowed Projector (Source) - Game Capture".into(),
        capture_region: Region { x: 12, y: 34, width: 640, height: 480 },
        unknown: toml::toml! { a_future_setting = "kept" },
        ..UserProfile::default()
    }
}

#[test]
fn configured_profile_loads_every_value() {
    // Act
    let profile = UserProfile::load(&fixture("configured.toml")).unwrap();

    // Assert
    assert_eq!(profile, configured());
}

#[test]
fn boolean_screenshot_on_from_an_untouched_settings_window_loads_as_empty() {
    // Act
    let profile = UserProfile::load(&fixture("ui_defaults.toml")).unwrap();

    // Assert
    assert_eq!(profile.screenshot_on, []);
}

#[test]
fn missing_keys_fall_back_to_autosplit_default_profile() {
    // Act
    let profile = UserProfile::load(&fixture("missing_keys.toml")).unwrap();

    // Assert
    let expected = UserProfile {
        split_hotkey: "num 1".into(),
        fps_limit: 144,
        split_image_directory: "C:/Users/Runner/Splits/OoT Any%/".into(),
        capture_region: Region { x: 12, y: 34, width: 640, height: 480 },
        ..UserProfile::default()
    };
    assert_eq!(profile, expected);
}

#[test]
fn saved_profile_loads_back_identical() {
    // Arrange
    let profile = configured();

    // Act
    let reloaded = UserProfile::parse(&profile.to_toml()).unwrap();

    // Assert
    assert_eq!(reloaded, profile);
}

#[test]
fn saved_profile_keeps_autosplit_key_order() {
    // Arrange
    let expected: Vec<String> = std::fs::read_to_string(fixture("configured.toml"))
        .unwrap()
        .lines()
        .filter_map(|line| line.split_once(" = ").map(|(key, _)| key.to_owned()))
        .collect();

    // Act
    let saved: Vec<String> = configured()
        .to_toml()
        .lines()
        .filter_map(|line| line.split_once(" = ").map(|(key, _)| key.to_owned()))
        .collect();

    // Assert
    assert_eq!(saved, expected);
}

#[test]
fn integer_pause_time_from_old_profiles_loads_as_float() {
    // Act
    let profile = UserProfile::parse("default_pause_time = 10").unwrap();

    // Assert
    assert_eq!(profile.default_pause_time, 10.0);
}

#[test]
fn pickle_settings_are_reported_as_old_version() {
    // Act
    let result = UserProfile::load(Path::new("settings.pkl"));

    // Assert
    assert!(matches!(result, Err(ProfileError::OldVersion)));
}

#[test]
fn invalid_toml_is_reported_as_invalid_settings() {
    // Act
    let result = UserProfile::parse("fps_limit = = 3");

    // Assert
    assert!(matches!(result, Err(ProfileError::Invalid { .. })));
}

#[test]
fn defaults_match_a_profile_saved_from_an_untouched_settings_window() {
    // Arrange
    let saved = UserProfile::load(&fixture("ui_defaults.toml")).unwrap();

    // Act
    let defaults = UserProfile::default();

    // Assert
    // The capture method default depends on the OS, so it's taken from the saved file
    assert_eq!(UserProfile { capture_method: saved.capture_method.clone(), ..defaults }, saved);
}

#[test]
fn missing_profile_file_is_reported_as_invalid_settings() {
    // Act
    let result = UserProfile::load(Path::new("/this/profile/does/not/exist.toml"));

    // Assert
    assert!(matches!(result, Err(ProfileError::Unreadable { .. })));
}

#[test_case("timer_connection"; "timer connection")]
#[test_case("livesplit_one_port"; "livesplit one port")]
#[test_case("group_split_images"; "group split images")]
#[test_case("split_image_grid"; "split image grid")]
#[test_case("fade_color_tolerance"; "fade color tolerance")]
fn keys_not_in_autosplit_are_left_out_when_unchanged(key: &str) {
    // Act
    let saved = configured().to_toml();

    // Assert
    assert!(!saved.contains(key));
}

#[test]
fn livesplit_one_settings_load_back() {
    // Arrange
    let profile =
        UserProfile { timer_connection: TimerConnection::LiveSplitOne, livesplit_one_port: 20000, ..configured() };

    // Act
    let reloaded = UserProfile::parse(&profile.to_toml()).unwrap();

    // Assert
    assert_eq!(reloaded, profile);
}

#[test]
fn livesplit_one_is_written_as_snake_case() {
    // Arrange
    let profile = UserProfile { timer_connection: TimerConnection::LiveSplitOne, ..configured() };

    // Act
    let saved = profile.to_toml();

    // Assert
    assert!(saved.contains("timer_connection = \"livesplit_one\""));
}

#[test]
fn molok_loads_back() {
    // Arrange
    let profile = UserProfile { timer_connection: TimerConnection::Molok, ..configured() };

    // Act
    let saved = profile.to_toml();

    // Assert
    assert!(saved.contains("timer_connection = \"molok\""));
    assert_eq!(UserProfile::parse(&saved).unwrap(), profile);
}

#[test_case(Region { x: 1, y: 1, width: 2, height: 1 } => Rect { left: 1, top: 1, right: 3, bottom: 2 }; "inside the frame")]
#[test_case(Region { x: 2, y: 1, width: 100, height: 100 } => Rect { left: 2, top: 1, right: 4, bottom: 3 }; "end is clamped to the frame")]
#[test_case(Region { x: 10, y: 10, width: 5, height: 5 } => Rect { left: 3, top: 2, right: 4, bottom: 3 }; "start past the frame is moved to the last pixel")]
#[test_case(Region { x: -5, y: -5, width: 2, height: 2 } => Rect { left: 0, top: 0, right: 2, bottom: 2 }; "negative start is clamped to zero")]
#[test_case(Region { x: 0, y: 0, width: 0, height: 0 } => Rect { left: 0, top: 0, right: 0, bottom: 0 }; "empty region")]
#[test_case(Region { x: 1, y: 1, width: -3, height: 2 } => Rect { left: 1, top: 1, right: 1, bottom: 3 }; "negative size")]
fn region_rect_in_a_4x3_frame(region: Region) -> Rect {
    // Act, with the expected value checked by test_case
    region.rect(4, 3)
}

#[test_case(Region { x: 0, y: 0, width: 4, height: 3 } => RegionFit::Inside; "whole frame")]
#[test_case(Region { x: 2, y: 1, width: 4, height: 1 } => RegionFit::Partly; "past the right edge")]
#[test_case(Region { x: -1, y: 0, width: 2, height: 2 } => RegionFit::Partly; "negative start")]
#[test_case(Region { x: 0, y: 0, width: 0, height: 5 } => RegionFit::Empty; "no width")]
fn region_fit_in_a_4x3_frame(region: Region) -> RegionFit {
    // Act, with the expected value checked by test_case
    region.fit(4, 3)
}

#[test_case(UserProfile { capture_device_name: "Elgato".into(), ..UserProfile::default() } => true; "by name")]
#[test_case(UserProfile { capture_device_id: 0, ..UserProfile::default() } => true; "by number")]
#[test_case(UserProfile::default() => false; "none")]
fn has_capture_device(profile: UserProfile) -> bool {
    // Act, with the expected value checked by test_case
    profile.has_capture_device()
}

#[test_case("VIDEO_CAPTURE_DEVICE" => None; "video capture device")]
#[test_case("" => None; "not set")]
#[test_case("WINDOWS_GRAPHICS_CAPTURE" => Some("WINDOWS_GRAPHICS_CAPTURE".to_owned()); "window capture")]
fn window_capture_method(method: &str) -> Option<String> {
    // Arrange
    let profile = UserProfile { capture_method: method.into(), ..UserProfile::default() };

    // Act
    profile.window_capture_method().map(str::to_owned)
}

#[test_case("fps_limit = 60\n[capture_region]\nx = 1\ntimer_connection = \"livesplit_one\"\n" => vec!["timer_connection".to_owned()]; "key after the table")]
#[test_case("timer_connection = \"livesplit_one\"\n[capture_region]\nx = 1\n" => Vec::<String>::new(); "key before the table")]
fn finds_keys_inside_capture_region(text: &str) -> Vec<String> {
    // Act, with the expected value checked by test_case
    UserProfile::keys_inside_capture_region(text)
}

#[test_case("fps_limit = 60.0" => true; "float for a whole number")]
#[test_case("split_image_directory = \"C:\\Users\\me\"" => true; "windows path with backslashes")]
fn invalid_profile_errors_are_in_words(text: &str) -> bool {
    // Act
    let error = UserProfile::parse(text).unwrap_err().to_string();

    // Assert
    error.contains("whole number") || error.contains("single quotes")
}

#[test_case("splits", "/home/me/profiles/splits"; "relative folder")]
#[test_case("/data/splits", "/data/splits"; "absolute folder")]
#[test_case("", ""; "no folder")]
fn resolves_split_folder_against_the_profile(directory: &str, expected: &str) {
    // Arrange
    let mut profile = UserProfile { split_image_directory: directory.into(), ..UserProfile::default() };

    // Act
    profile.resolve_directories(Path::new("/home/me/profiles"));

    // Assert, as paths, since Windows joins with a backslash
    assert_eq!(Path::new(&profile.split_image_directory), Path::new(expected));
}

#[test_case("WINDOWS_GRAPHICS_CAPTURE" => "Windows Graphics Capture"; "windows graphics capture")]
#[test_case("BITBLT" => "BitBlt"; "bitblt")]
#[test_case("PRINTWINDOW_RENDERFULLCONTENT" => "Force Full Content Rendering"; "full content rendering")]
#[test_case("DESKTOP_DUPLICATION" => "Direct3D Desktop Duplication"; "desktop duplication")]
#[test_case("SOMETHING_NEW" => "SOMETHING_NEW"; "unknown method")]
fn names_capture_methods(method: &str) -> &str {
    // Act, with the expected value checked by test_case
    capture_method_name(method)
}

#[test_case(1920, 1080 => (240, 0, 1440, 1080); "a 16:9 canvas")]
#[test_case(1280, 720 => (160, 0, 960, 720); "720p")]
#[test_case(640, 480 => (0, 0, 640, 480); "already 4:3")]
#[test_case(1080, 1080 => (0, 135, 1080, 810); "narrower than 4:3")]
#[test_case(1366, 768 => (171, 0, 1024, 768); "rounded")]
fn crops_to_the_4_3_picture_in_the_middle(width: usize, height: usize) -> (i64, i64, i64, i64) {
    // Act
    let region = Region::centered_4_3(width, height);

    // Assert, with the expected value checked by test_case
    (region.x, region.y, region.width, region.height)
}

#[test_case("OBS Virtual Camera", NEW_PROFILE_REGION, (1920, 1080) => Some((1920, 1080)); "a new profile's region")]
#[test_case("OBS Virtual Camera", Region::default(), (1920, 1080) => Some((1920, 1080)); "no region")]
#[test_case("OBS Virtual Camera", Region { x: 100, y: 50, width: 640, height: 480 }, (1920, 1080) => None; "a region that was set")]
#[test_case("Game Capture HD60 S", NEW_PROFILE_REGION, (1920, 1080) => None; "another device")]
#[test_case("OBS Virtual Camera", NEW_PROFILE_REGION, (640, 480) => None; "a frame it already is")]
fn obs_virtual_camera_gets_its_whole_frame(device: &str, region: Region, frame: (usize, usize)) -> Option<(i64, i64)> {
    // Arrange
    let profile = UserProfile { capture_device_name: device.into(), capture_region: region, ..UserProfile::default() };

    // Act
    let full = profile.obs_full_frame_region(frame);

    // Assert, with the expected value checked by test_case
    full.map(|region| (region.width, region.height))
}

#[test]
fn fade_color_tolerance_loads_back() {
    // Arrange
    let profile = UserProfile { fade_color_tolerance: 24, ..configured() };

    // Act
    let reloaded = UserProfile::parse(&profile.to_toml()).unwrap();

    // Assert
    assert_eq!((reloaded.fade_color_tolerance, reloaded.split_defaults().fade_tolerance), (24, 24));
}
