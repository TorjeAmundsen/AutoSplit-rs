//! Profiles written by AutoSplit (tests/fixtures/profiles, made with tomli_w) load unchanged.

use std::path::{Path, PathBuf};

use autosplit_core::commands::Command;
use autosplit_core::profile::{ProfileError, Region, UserProfile};

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
