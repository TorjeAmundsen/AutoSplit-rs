//! Loading profiles for the commands, checking their hotkeys, and screenshot names.

use autosplit::hotkeys::hotkey_problems;
use autosplit::profile::load_raw;
use autosplit::screenshot::{Saved, is_screenshot_name, save};
use autosplit_core::commands::TimerLink;
use autosplit_core::profile::UserProfile;
use test_case::test_case;

#[test]
fn missing_profile_says_so() {
    // Arrange
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("missing.toml");

    // Act
    let error = load_raw(&path).unwrap_err();

    // Assert
    assert_eq!(error.to_string(), format!("The profile {} doesn't exist.", path.display()));
}

#[test]
fn folder_instead_of_a_profile_says_so() {
    // Arrange
    let folder = tempfile::tempdir().unwrap();

    // Act
    let error = load_raw(folder.path()).unwrap_err();

    // Assert
    assert_eq!(error.to_string(), format!("{} is a folder. Give the profile's .toml file.", folder.path().display()));
}

#[test]
fn profile_loads_as_written() {
    // Arrange
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("settings.toml");
    std::fs::write(&path, "split_hotkey = \"num 1\"\nsplit_image_directory = \"splits\"\n").unwrap();

    // Act
    let (profile, warnings) = load_raw(&path).unwrap();

    // Assert: the folder isn't resolved yet
    assert_eq!(
        (profile.split_hotkey.as_str(), profile.split_image_directory.as_str(), warnings.len()),
        ("num 1", "splits", 0)
    );
}

#[test_case(UserProfile { split_hotkey: "num 1".into(), reset_hotkey: "num 1".into(), ..UserProfile::default() } => vec!["The split and reset hotkeys are both 'num 1'".to_owned()]; "same hotkey twice")]
#[test_case(UserProfile { split_hotkey: "num 1".into(), reset_hotkey: "num 3".into(), ..UserProfile::default() } => Vec::<String>::new(); "different hotkeys")]
fn hotkey_problems_find_duplicates(profile: UserProfile) -> Vec<String> {
    // Act, with the expected value checked by test_case
    hotkey_problems(&profile, TimerLink::Hotkeys)
}

#[test_case("001_SplitImage.png" => true; "screenshot")]
#[test_case("12_splitimage.PNG" => true; "other case")]
#[test_case("_SplitImage.png" => false; "no number")]
#[test_case("01_Boss_SplitImage.png" => false; "split image name")]
fn screenshot_names(name: &str) -> bool {
    // Act, with the expected value checked by test_case
    is_screenshot_name(name)
}

#[test]
fn screenshot_is_saved_into_the_folder() {
    // Arrange
    let folder = tempfile::tempdir().unwrap();
    let profile = UserProfile { split_image_directory: folder.path().display().to_string(), ..UserProfile::default() };
    let image = autosplit_core::image::BgraImage::from_raw(2, 2, vec![255; 16]);
    let (sender, receiver) = std::sync::mpsc::channel();

    // Act
    save(&profile, Some(image), move |saved| sender.send(saved).unwrap()).unwrap();
    let saved = receiver.recv().unwrap();

    // Assert: the window is told the file, and only failures are errors
    let path = folder.path().join("001_SplitImage.png");
    assert_eq!(
        (saved.path(), saved.message()),
        (Some(path.as_path()), Ok(format!("Saved screenshot {}", path.display())))
    );
}

#[test]
fn screenshot_that_wasnt_opened_is_an_error_with_its_file() {
    // Arrange
    let path = std::path::PathBuf::from("shots/001_SplitImage.png");
    let saved = Saved::NotOpened(path.clone(), "no app".into());

    // Act
    let message = saved.message();

    // Assert
    assert_eq!(
        (saved.path(), message),
        (Some(path.as_path()), Err(format!("Saved screenshot {}, but couldn't open it: no app", path.display())))
    );
}
