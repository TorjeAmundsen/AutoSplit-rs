//! Split image folder validation reports AutoSplit's errors, in AutoSplit's priority order.

use std::path::Path;

use autosplit_core::filename::{FilenameWarning, Setting};
use autosplit_core::image::BgraImage;
use autosplit_core::profile::UserProfile;
use autosplit_core::split_folder::{
    ValidationContext, ValidationError, load_profile_folder_to_start, load_split_folder,
};
use autosplit_core::split_image::{LoadError, LoadWarning};
use tempfile::TempDir;
use test_case::test_case;

const ALL_HOTKEYS: ValidationContext = ValidationContext {
    split_hotkey_set: true,
    reset_hotkey_set: true,
    pause_hotkey_set: true,
    hotkeys_needed: true,
    group_split_images: false,
};
const NO_HOTKEYS: ValidationContext = ValidationContext {
    split_hotkey_set: false,
    reset_hotkey_set: false,
    pause_hotkey_set: false,
    hotkeys_needed: true,
    group_split_images: false,
};
const WITHOUT_HOTKEYS: ValidationContext = ValidationContext { hotkeys_needed: false, ..NO_HOTKEYS };
const GROUPED: ValidationContext = ValidationContext { group_split_images: true, ..ALL_HOTKEYS };

fn folder(names: &[&str]) -> TempDir {
    let directory = tempfile::tempdir().unwrap();
    let png = BgraImage::from_raw(2, 2, vec![255; 16]).encode_png().unwrap();
    for name in names {
        let content = if name.ends_with(".png") { png.clone() } else { b"not an image".to_vec() };
        std::fs::write(directory.path().join(name), content).unwrap();
    }
    directory
}

fn validation_error(names: &[&str], context: ValidationContext) -> Option<ValidationError> {
    let directory = folder(names);
    load_split_folder(directory.path(), &context).result.err()
}

#[test_case(&["start_auto_splitter.png"], ALL_HOTKEYS => Some(ValidationError::NoSplitImage); "only a start image")]
#[test_case(&["start_auto_splitter.png", "1.png"], NO_HOTKEYS => Some(ValidationError::StartWithoutSplitHotkey); "start image without split hotkey")]
#[test_case(&["reset.png", "1.png"], NO_HOTKEYS => Some(ValidationError::ResetWithoutResetHotkey); "reset image without reset hotkey")]
#[test_case(&["1_{p}.png"], NO_HOTKEYS => Some(ValidationError::PauseWithoutPauseHotkey("1_{p}.png".into())); "pause flag without pause hotkey")]
#[test_case(&["1.png", "reset.png", "reset_2.png"], ALL_HOTKEYS => Some(ValidationError::MultipleKeywordImages("reset")); "two reset images")]
#[test_case(&["1.png", "start_auto_splitter.png", "start_auto_splitter_2.png"], ALL_HOTKEYS => Some(ValidationError::MultipleKeywordImages("start_auto_splitter")); "two start images")]
#[test_case(&["1.png", "2.bmp"], ALL_HOTKEYS => matches Some(ValidationError::InvalidImage(LoadError::NotPng(_))); "non png file")]
#[test_case(&["1.png", "2.txt"], ALL_HOTKEYS => matches Some(ValidationError::InvalidImage(LoadError::OcrUnsupported(_))); "ocr split file")]
#[test_case(&["1.png", "notes_txt"], ALL_HOTKEYS => matches Some(ValidationError::InvalidImage(LoadError::NotPng(_))); "txt without a dot isn't an ocr file")]
#[test_case(&["1.png", "notes"], ALL_HOTKEYS => matches Some(ValidationError::InvalidImage(LoadError::NotPng(_))); "file without extension")]
#[test_case(&["2_{p}.bmp", "3_{p}.png"], NO_HOTKEYS => matches Some(ValidationError::InvalidImage(_)); "invalid image reported before missing hotkeys of later images")]
#[test_case(&["preset_menu.png", "1.png"], NO_HOTKEYS => None; "preset isn't the reset keyword")]
#[test_case(&["reset.bmp", "1.png"], ALL_HOTKEYS => matches Some(ValidationError::InvalidImage(LoadError::NotPng(_))); "unusable reset image")]
#[test_case(&["start_auto_splitter.bmp", "1.png"], ALL_HOTKEYS => matches Some(ValidationError::InvalidImage(LoadError::NotPng(_))); "unusable start image")]
#[test_case(&["xstart_auto_splitter.png", "1.png"], NO_HOTKEYS => None; "start keyword must be its own word")]
#[test_case(&["start_auto_splitter.png", "reset.png", "1_{p}.png"], WITHOUT_HOTKEYS => None; "a timer told without hotkeys needs none")]
#[test_case(&["1.png", ".hidden.bmp"], ALL_HOTKEYS => None; "hidden files are ignored")]
#[test_case(&["1_a.png", "1_b.png", "1_c.png", "1_d.png", "2.png"], GROUPED => None; "four images for one split")]
#[test_case(&["1_a.png", "1_b.png", "01_c.png", "1_d.png", "1_e.png"], GROUPED => Some(ValidationError::GroupTooLarge("01".into())); "five images for one split")]
#[test_case(&["1.png", "boss.png"], GROUPED => Some(ValidationError::NoNumberPrefix("boss.png".into())); "grouped image without a number")]
#[test_case(&["1.png", "boss.png"], ALL_HOTKEYS => None; "ungrouped images need no number")]
#[test_case(&["1_a.png", "1_b_@2@.png"], GROUPED => Some(ValidationError::GroupLoopsDiffer("1".into())); "different loop counts in a group")]
#[test_case(&["start_auto_splitter.png", "reset.png", "1.png"], GROUPED => None; "start and reset images need no number")]
fn folder_validation(names: &[&str], context: ValidationContext) -> Option<ValidationError> {
    // Arrange, Act
    validation_error(names, context)
}

#[test]
fn empty_folder_path_is_no_folder_selected() {
    // Act
    let result = load_split_folder(Path::new(""), &ALL_HOTKEYS).result;

    // Assert
    assert_eq!(result.err(), Some(ValidationError::NoSplitImageDirectory));
}

#[test]
fn missing_folder_is_invalid() {
    // Arrange
    let path = Path::new("/this/folder/does/not/exist");

    // Act
    let result = load_split_folder(path, &ALL_HOTKEYS).result;

    // Assert
    assert_eq!(result.err(), Some(ValidationError::InvalidDirectory(path.to_owned())));
}

#[test]
fn split_images_are_in_file_name_order() {
    // Arrange
    let directory = folder(&["b.png", "a_(0.9).png", "C.png", "reset.png", "10.png", "2.png"]);

    // Act
    let set = load_split_folder(directory.path(), &ALL_HOTKEYS).result.unwrap();

    // Assert
    let names: Vec<&str> = set.splits.iter().map(|image| image.filename.as_str()).collect();
    assert_eq!(names, ["10.png", "2.png", "c.png", "a_(0.9).png", "b.png"]);
}

#[test]
fn grouped_split_images_are_in_number_then_name_order() {
    // Arrange
    let directory = folder(&["10_a.png", "2_b.png", "02_c.png", "1.png", "reset.png"]);

    // Act
    let set = load_split_folder(directory.path(), &GROUPED).result.unwrap();

    // Assert
    let names: Vec<&str> = set.splits.iter().map(|image| image.filename.as_str()).collect();
    assert_eq!((names, set.groups), (vec!["1.png", "02_c.png", "2_b.png", "10_a.png"], vec![0..1, 1..3, 3..4]));
}

#[test]
fn ungrouped_split_images_are_one_per_split() {
    // Arrange
    let directory = folder(&["1_a.png", "1_b.png"]);

    // Act
    let set = load_split_folder(directory.path(), &ALL_HOTKEYS).result.unwrap();

    // Assert
    assert_eq!(set.groups, [0..1, 1..2]);
}

#[test]
fn non_png_file_reports_a_load_error() {
    // Arrange
    let directory = folder(&["1.png", "2.jpg"]);

    // Act
    let load_errors = load_split_folder(directory.path(), &ALL_HOTKEYS).load_errors;

    // Assert
    assert_eq!(load_errors, [LoadError::NotPng(directory.path().join("2.jpg"))]);
}

#[test]
fn ocr_split_file_reports_that_ocr_is_unsupported() {
    // Arrange
    let directory = folder(&["1.png", "2.txt"]);

    // Act
    let load_errors = load_split_folder(directory.path(), &ALL_HOTKEYS).load_errors;

    // Assert
    assert_eq!(load_errors, [LoadError::OcrUnsupported(directory.path().join("2.txt"))]);
}

#[test_case(true => Some(ValidationError::NoSplitHotkey); "needs a split hotkey")]
#[test_case(false => None; "a timer told without hotkeys needs none")]
fn starting_without_a_split_hotkey(hotkeys_needed: bool) -> Option<ValidationError> {
    // Arrange
    let directory = folder(&["1.png"]);
    let profile =
        UserProfile { split_image_directory: directory.path().to_string_lossy().into(), ..UserProfile::default() };

    // Act
    let result = load_profile_folder_to_start(&profile, hotkeys_needed).result;

    // Assert, with the expected value checked by test_case
    result.err()
}

#[test]
fn ignored_filename_values_are_reported_without_failing_the_folder() {
    // Arrange
    let directory = folder(&["1_(1.5).png"]);

    // Act
    let load = load_split_folder(directory.path(), &ALL_HOTKEYS);

    // Assert
    let expected = LoadWarning {
        path: directory.path().join("1_(1.5).png"),
        warning: FilenameWarning::OutOfRange { setting: Setting::Threshold, value: "1.5".into() },
    };
    assert_eq!((load.result.is_ok(), load.warnings), (true, vec![expected]));
}
