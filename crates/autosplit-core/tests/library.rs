//! Writing the splits library: ready-made splits added from a preset's splits, changed and
//! deleted, with their masks and base images.

use std::path::{Path, PathBuf};

use autosplit_core::image::BgraImage;
use autosplit_core::library::{
    BASE_IMAGE_SIZE, NewStep, add_ready_made, base_image, change_ready_made, delete_ready_made, library_game_folder,
};
use autosplit_core::preset::{LIBRARY_FILE, LibrarySplit, PresetError, PresetSplit, SplitLibrary, load_libraries};

const GAME: &str = "The Legend of Zelda: Ocarina of Time";

/// A mask file named `name` in `folder`, and an image of the split, 320x240 like a grab.
fn step(folder: &Path, name: &str) -> NewStep {
    let mask = folder.join(format!("{name}.png"));
    std::fs::write(&mask, BgraImage::from_raw(4, 3, vec![255; 4 * 3 * 4]).encode_png().unwrap()).unwrap();
    let mut step = using(name, mask);
    step.split.threshold = Some(0.9);
    step
}

/// An image named `name` whose mask is `mask`, a file that's there already, like the library's.
fn using(name: &str, mask: PathBuf) -> NewStep {
    NewStep {
        split: PresetSplit { name: name.into(), mask: mask.display().to_string(), ..PresetSplit::default() },
        mask: Some(mask),
        savestate: None,
        screenshot: None,
        base_picture: screenshot(),
    }
}

/// A gray picture the size of a grab.
fn screenshot() -> BgraImage {
    BgraImage::from_raw(320, 240, vec![128; 320 * 240 * 4])
}

/// Adds a ready-made split for the game to the library in `splits`, with the presets folder in it.
fn add(splits: &Path, description: &str, steps: Vec<NewStep>) -> Result<PathBuf, PresetError> {
    add_ready_made(splits, &splits.join("presets"), GAME, description, steps)
}

/// The presets folder next to the splits folder the library in `folder` is in.
fn presets(folder: &Path) -> PathBuf {
    folder.parent().unwrap().join("presets")
}

fn load(folder: &Path) -> SplitLibrary {
    SplitLibrary::load(&folder.join(LIBRARY_FILE)).unwrap()
}

/// The `index`th ready-made split of the library in `folder` as it is, which changing it expects.
fn entry(folder: &Path, index: usize) -> Vec<LibrarySplit> {
    load(folder).ready_made().nth(index).unwrap().to_vec()
}

/// The files in `folder`, by name, sorted.
fn files(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// A library folder for the game with a boss of two images in it, and the folder the preset's
/// masks are in.
fn with_boss() -> (tempfile::TempDir, tempfile::TempDir, PathBuf) {
    let (splits, preset) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut intro = step(preset.path(), "Ganon intro");
    intro.split.dummy = true;
    let steps = vec![intro, step(preset.path(), "Ganon")];
    let folder = add(splits.path(), "The intro text, then the flash", steps).unwrap();
    (splits, preset, folder)
}

#[test]
fn base_images_are_made_small_and_opaque() {
    // Act: from a picture that's nearly see-through
    let base = base_image(BgraImage::from_raw(320, 240, vec![10; 320 * 240 * 4]).view());

    // Assert
    let opaque = base.data().iter().skip(3).step_by(4).all(|&alpha| alpha == 255);
    assert_eq!((base.width(), base.height(), opaque), (BASE_IMAGE_SIZE.0, BASE_IMAGE_SIZE.1, true));
}

#[test]
fn a_game_without_a_library_gets_one() {
    // Act
    let (splits, _preset, folder) = with_boss();

    // Assert: it loads, one ready-made split of two images, with copies of their masks and small
    // base images
    let (libraries, errors) = load_libraries(splits.path());
    let library = load(&folder);
    let steps: Vec<(&str, &str, bool, &str)> = (library.splits.iter())
        .map(|step| (step.split.name.as_str(), step.split.mask.as_str(), step.then, step.description.as_str()))
        .collect();
    assert_eq!(
        (libraries.len(), errors.len(), library.game.as_str(), library.ready_made().count(), steps),
        (
            1,
            0,
            GAME,
            1,
            vec![
                ("Ganon intro", "Ganon_intro.png", false, "The intro text, then the flash"),
                ("Ganon", "Ganon.png", true, "")
            ]
        )
    );
    assert_eq!(
        files(&folder),
        ["Ganon.png", "Ganon_intro.png", "base_Ganon.png", "base_Ganon_intro.png", LIBRARY_FILE]
    );
    let base = BgraImage::open_png(&folder.join("base_Ganon.png")).unwrap();
    assert_eq!((base.width(), base.height()), BASE_IMAGE_SIZE);
}

#[test]
fn a_game_with_a_library_gets_another_ready_made_split() {
    // Arrange
    let (splits, preset, folder) = with_boss();

    // Act
    let added = add(splits.path(), "Bottle over the head", vec![step(preset.path(), "Bottle")]).unwrap();

    // Assert
    let names: Vec<String> = load(&folder).splits.iter().map(|step| step.split.name.clone()).collect();
    assert_eq!((added, names), (folder, vec!["Ganon intro".to_owned(), "Ganon".to_owned(), "Bottle".to_owned()]));
}

#[test]
fn changing_only_the_description_keeps_the_images() {
    // Arrange
    let (_splits, _preset, folder) = with_boss();
    let before = load(&folder);

    // Act
    change_ready_made(&folder, &presets(&folder), 0, &entry(&folder, 0), "Final slash on Ganon", None, &[]).unwrap();

    // Assert
    let after = load(&folder);
    let kept = before.splits.iter().zip(&after.splits).all(|(a, b)| a.split == b.split && a.base_image == b.base_image);
    assert_eq!((kept, after.splits[0].description.as_str()), (true, "Final slash on Ganon"));
}

#[test]
fn changed_images_replace_the_old_ones_and_their_files() {
    // Arrange
    let (_splits, preset, folder) = with_boss();

    // Act
    change_ready_made(
        &folder,
        &presets(&folder),
        0,
        &entry(&folder, 0),
        "Just the flash",
        Some(vec![step(preset.path(), "Flash")]),
        &[],
    )
    .unwrap();

    // Assert
    let names: Vec<String> = load(&folder).splits.iter().map(|step| step.split.name.clone()).collect();
    assert_eq!(
        (names, files(&folder)),
        (vec!["Flash".to_owned()], vec!["Flash.png".to_owned(), "base_Flash.png".to_owned(), LIBRARY_FILE.to_owned()])
    );
}

#[test]
fn masks_in_the_library_already_are_used_where_they_are() {
    // Arrange: an image whose mask is the library's own, like after inserting it into a preset
    let (_splits, _preset, folder) = with_boss();
    let mask = folder.join("Ganon.png");
    let again = using("Ganon", mask);

    // Act
    change_ready_made(&folder, &presets(&folder), 0, &entry(&folder, 0), "Just the flash", Some(vec![again]), &[])
        .unwrap();

    // Assert: no copy, its base image under the same name, and the intro's files gone
    assert_eq!(
        (load(&folder).splits[0].split.mask.as_str(), files(&folder)),
        ("Ganon.png", vec!["Ganon.png".to_owned(), "base_Ganon.png".to_owned(), LIBRARY_FILE.to_owned()])
    );
}

#[test]
fn rewritten_files_keep_the_spelling_of_the_ones_they_replace() {
    // Arrange: a base image named in lowercase, like one written by hand
    let (_splits, _preset, folder) = with_boss();
    std::fs::rename(folder.join("base_Ganon.png"), folder.join("base_ganon.png")).unwrap();
    let text = std::fs::read_to_string(folder.join(LIBRARY_FILE)).unwrap();
    std::fs::write(folder.join(LIBRARY_FILE), text.replace("base_Ganon.png", "base_ganon.png")).unwrap();
    let again = using("Ganon", folder.join("Ganon.png"));

    // Act
    change_ready_made(&folder, &presets(&folder), 0, &entry(&folder, 0), "Just the flash", Some(vec![again]), &[])
        .unwrap();

    // Assert: the library and the folder both still say base_ganon.png
    assert_eq!(
        (load(&folder).splits[0].base_image.as_str(), files(&folder)),
        ("base_ganon.png", vec!["Ganon.png".to_owned(), "base_ganon.png".to_owned(), LIBRARY_FILE.to_owned()])
    );
}

#[test]
fn deleting_a_ready_made_split_deletes_the_files_only_it_used() {
    // Arrange: a second ready-made split that uses the boss's flash mask too
    let (splits, _preset, folder) = with_boss();
    let flash = using("Flash", folder.join("Ganon.png"));
    add(splits.path(), "Any flash", vec![flash]).unwrap();

    // Act
    delete_ready_made(&folder, &presets(&folder), 0, &entry(&folder, 0), &[]).unwrap();

    // Assert
    let names: Vec<String> = load(&folder).splits.iter().map(|step| step.split.name.clone()).collect();
    assert_eq!(
        (names, files(&folder)),
        (vec!["Flash".to_owned()], vec!["Ganon.png".to_owned(), "base_Flash.png".to_owned(), LIBRARY_FILE.to_owned()])
    );
}

#[test]
fn a_ready_made_split_that_isnt_there_is_an_error() {
    // Arrange
    let (_splits, _preset, folder) = with_boss();

    // Act
    let error = delete_ready_made(&folder, &presets(&folder), 5, &[], &[]).unwrap_err();

    // Assert
    assert!(error.to_string().contains("changed in it since you opened it"), "{error}");
}

#[test]
fn savestates_are_copied_into_the_folder_the_games_presets_share() {
    // Arrange: a split whose savestate is in its preset's savestates folder
    let (splits, preset) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::create_dir(preset.path().join("savestates")).unwrap();
    let savestate = preset.path().join("savestates").join("004-kokiri forest.gzs");
    std::fs::write(&savestate, b"state").unwrap();
    let mut deku = step(preset.path(), "EnterDeku");
    deku.split.savestate = "savestates/004-kokiri forest.gzs".into();
    deku.savestate = Some(savestate);

    // Act
    let folder = add(splits.path(), "Enter the Deku Tree", vec![deku]).unwrap();

    // Assert: under its own name, which the library says
    let library = load(&folder);
    let path = library.splits[0].split.savestate.as_str();
    assert_eq!(
        (path, std::fs::read(folder.join(path)).ok()),
        ("../presets/The_Legend_of_Zelda_Ocarina_of_Time/savestates/004-kokiri forest.gzs", Some(b"state".to_vec()))
    );
}

/// A library with one ready-made split, EnterDeku, whose preset has a screenshot for it with
/// `contents`. Returns the library's folder too.
fn with_screenshot(contents: &[u8]) -> (tempfile::TempDir, tempfile::TempDir, PathBuf) {
    let (splits, preset) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::create_dir(preset.path().join("screenshots")).unwrap();
    let shot = preset.path().join("screenshots").join("EnterDeku.png");
    std::fs::write(&shot, contents).unwrap();
    let mut deku = step(preset.path(), "EnterDeku");
    deku.split.screenshot = "screenshots/EnterDeku.png".into();
    deku.screenshot = Some(shot);
    let folder = add(splits.path(), "Enter the Deku Tree", vec![deku]).unwrap();
    (splits, preset, folder)
}

#[test]
fn screenshots_are_copied_into_the_folder_the_games_presets_share_as_they_are() {
    // Act
    let (_splits, _preset, folder) = with_screenshot(b"full size");

    // Assert: not shrunk
    let library = load(&folder);
    let path = library.splits[0].split.screenshot.as_str();
    assert_eq!(
        (path, std::fs::read(folder.join(path)).ok()),
        ("../presets/The_Legend_of_Zelda_Ocarina_of_Time/screenshots/EnterDeku.png", Some(b"full size".to_vec()))
    );
}

#[test]
fn a_screenshot_a_preset_shares_already_is_used_instead_of_a_copy() {
    // Arrange: the game's presets share a screenshot with the same contents, under another name
    let (splits, preset) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let shared = splits.path().join("presets/The_Legend_of_Zelda_Ocarina_of_Time/screenshots");
    std::fs::create_dir_all(&shared).unwrap();
    std::fs::write(shared.join("Deku.png"), b"full size").unwrap();
    let shot = preset.path().join("EnterDeku shot.png");
    std::fs::write(&shot, b"full size").unwrap();
    let mut deku = step(preset.path(), "EnterDeku");
    deku.screenshot = Some(shot);

    // Act
    let folder = add(splits.path(), "Enter the Deku Tree", vec![deku]).unwrap();

    // Assert
    assert_eq!(
        (load(&folder).splits[0].split.screenshot.clone(), files(&shared)),
        ("../presets/The_Legend_of_Zelda_Ocarina_of_Time/screenshots/Deku.png".to_owned(), vec!["Deku.png".to_owned()])
    );
}

/// A library from before libraries shared screenshots, whose Gohma keeps its own, and its folder.
fn with_own_screenshot() -> (tempfile::TempDir, PathBuf) {
    let splits = tempfile::tempdir().unwrap();
    let folder = splits.path().join("Game");
    std::fs::create_dir(&folder).unwrap();
    let png = BgraImage::from_raw(4, 3, vec![255; 4 * 3 * 4]).encode_png().unwrap();
    std::fs::write(folder.join("Gohma.png"), png).unwrap();
    std::fs::write(folder.join("screenshot_Gohma.png"), b"full size").unwrap();
    let text = "game = \"Game\"

[[split]]
name = \"Gohma\"
mask = \"Gohma.png\"
screenshot = \"screenshot_Gohma.png\"
";
    std::fs::write(folder.join(LIBRARY_FILE), text).unwrap();
    (splits, folder)
}

#[test]
fn writing_a_library_moves_the_screenshots_it_keeps_itself_to_the_shared_folder() {
    // Arrange
    let (_splits, folder) = with_own_screenshot();

    // Act: only the description changes
    change_ready_made(&folder, &presets(&folder), 0, &entry(&folder, 0), "Gohma dies", None, &[]).unwrap();

    // Assert
    let path = load(&folder).splits[0].split.screenshot.clone();
    assert_eq!(
        (path.as_str(), std::fs::read(folder.join(&path)).ok(), files(&folder)),
        (
            "../presets/Game/screenshots/Gohma.png",
            Some(b"full size".to_vec()),
            vec!["Gohma.png".to_owned(), LIBRARY_FILE.to_owned()]
        )
    );
}

#[test]
fn a_screenshot_a_preset_being_edited_takes_from_the_library_stays_until_a_later_write() {
    // Arrange: the preset being edited has the library's Gohma in it
    let (_splits, folder) = with_own_screenshot();
    let in_use = [folder.join("screenshot_Gohma.png")];

    // Act: a write about something else
    change_ready_made(&folder, &presets(&folder), 0, &entry(&folder, 0), "Gohma dies", None, &in_use).unwrap();

    // Assert
    assert_eq!(
        (load(&folder).splits[0].split.screenshot.as_str(), folder.join("screenshot_Gohma.png").is_file()),
        ("screenshot_Gohma.png", true)
    );
}

#[test]
fn deleting_a_ready_made_split_leaves_its_shared_screenshot() {
    // Arrange
    let (_splits, _preset, folder) = with_screenshot(b"full size");
    let shared = load(&folder).splits[0].split.screenshot.clone();

    // Act
    delete_ready_made(&folder, &presets(&folder), 0, &entry(&folder, 0), &[]).unwrap();

    // Assert: deleting what nothing uses anymore is for sweep_shared_files, which knows the presets
    assert_eq!((files(&folder), folder.join(shared).is_file()), (vec![LIBRARY_FILE.to_owned()], true));
}

#[test]
fn the_description_is_on_the_first_image_only() {
    // Arrange
    let (_splits, _preset, folder) = with_boss();

    // Act
    let text = std::fs::read_to_string(folder.join(LIBRARY_FILE)).unwrap();

    // Assert: one line, the first image's, and no empty one for the second
    let library = load(&folder);
    assert_eq!(
        (text.matches("description").count(), library.splits[0].description.as_str()),
        (1, "The intro text, then the flash"),
        "{text}"
    );
}

#[test]
fn replaced_images_keep_their_file_names() {
    // Arrange: the boss made again from the preset's splits of the same names
    let (_splits, preset, folder) = with_boss();
    let mut intro = step(preset.path(), "Ganon intro");
    intro.split.dummy = true;

    // Act
    change_ready_made(
        &folder,
        &presets(&folder),
        0,
        &entry(&folder, 0),
        "The intro text, then the flash",
        Some(vec![intro, step(preset.path(), "Ganon")]),
        &[],
    )
    .unwrap();

    // Assert: the same names, no copies with numbers
    assert_eq!(
        files(&folder),
        ["Ganon.png", "Ganon_intro.png", "base_Ganon.png", "base_Ganon_intro.png", LIBRARY_FILE]
    );
}

#[test]
fn renamed_images_rename_their_files() {
    // Arrange: the boss's images as they are, the flash given a name that says what it is
    let (_splits, _preset, folder) = with_boss();
    let kept = |name: &str, mask: &str| using(name, folder.join(mask));
    let steps = vec![kept("Ganon intro", "Ganon_intro.png"), kept("White flash", "Ganon.png")];

    // Act
    change_ready_made(
        &folder,
        &presets(&folder),
        0,
        &entry(&folder, 0),
        "The intro text, then the flash",
        Some(steps),
        &[],
    )
    .unwrap();

    // Assert: the flash's mask and base image under its new name, the old ones gone
    let library = load(&folder);
    assert_eq!(
        (library.splits[1].split.name.as_str(), library.splits[1].split.mask.as_str(), files(&folder)),
        (
            "White flash",
            "White_flash.png",
            vec![
                "Ganon_intro.png".to_owned(),
                "White_flash.png".to_owned(),
                "base_Ganon_intro.png".to_owned(),
                "base_White_flash.png".to_owned(),
                LIBRARY_FILE.to_owned()
            ]
        )
    );
}

#[test]
fn a_renamed_image_keeps_a_mask_another_ready_made_split_uses() {
    // Arrange: a flash that uses the boss's mask too
    let (splits, _preset, folder) = with_boss();
    add(splits.path(), "Any flash", vec![using("Flash", folder.join("Ganon.png"))]).unwrap();
    let steps =
        vec![using("Ganon intro", folder.join("Ganon_intro.png")), using("White flash", folder.join("Ganon.png"))];

    // Act
    change_ready_made(&folder, &presets(&folder), 0, &entry(&folder, 0), "Renamed", Some(steps), &[]).unwrap();

    // Assert: both still use the one mask
    let library = load(&folder);
    assert_eq!(
        (library.splits[1].split.mask.as_str(), library.splits[2].split.mask.as_str()),
        ("Ganon.png", "Ganon.png")
    );
}

#[test]
fn files_other_ready_made_splits_use_are_never_written_over() {
    // Arrange: a flash that uses the boss's mask, then the boss made again from a new mask
    let (splits, preset, folder) = with_boss();
    let flash = using("Flash", folder.join("Ganon.png"));
    add(splits.path(), "Any flash", vec![flash]).unwrap();

    // Act
    change_ready_made(
        &folder,
        &presets(&folder),
        0,
        &entry(&folder, 0),
        "Just the flash",
        Some(vec![step(preset.path(), "Ganon")]),
        &[],
    )
    .unwrap();

    // Assert: the boss's new mask got a name of its own
    let library = load(&folder);
    assert_eq!(
        (library.splits[0].split.mask.as_str(), library.splits[1].split.mask.as_str()),
        ("Ganon (1).png", "Ganon.png")
    );
}

/// The files in `folder` with what's in them, to tell that nothing changed.
fn contents(folder: &Path) -> Vec<(String, Vec<u8>)> {
    files(folder).into_iter().map(|name| (name.clone(), std::fs::read(folder.join(&name)).unwrap())).collect()
}

#[test]
fn a_replace_that_fails_changes_nothing() {
    // Arrange: the boss made again, the first image written before the second's savestate, a
    // folder, can't be copied
    let (_splits, preset, folder) = with_boss();
    let before = contents(&folder);
    let mut flash = step(preset.path(), "Ganon");
    let not_a_file = preset.path().join("flash.gzs");
    std::fs::create_dir(&not_a_file).unwrap();
    flash.savestate = Some(not_a_file);
    let mut intro = step(preset.path(), "Ganon intro");
    std::fs::write(intro.mask.as_ref().unwrap(), b"another mask").unwrap();
    intro.split.dummy = true;

    // Act
    let result =
        change_ready_made(&folder, &presets(&folder), 0, &entry(&folder, 0), "Again", Some(vec![intro, flash]), &[]);

    // Assert: the masks, base images and library as they were
    assert!(result.is_err());
    assert_eq!(contents(&folder), before);
}

#[test]
fn an_add_that_fails_leaves_nothing_behind() {
    // Arrange: a game without a library, and a split whose savestate is missing
    let (splits, preset) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut deku = step(preset.path(), "EnterDeku");
    deku.savestate = Some(preset.path().join("missing.gzs"));

    // Act
    let result = add(splits.path(), "Enter the Deku Tree", vec![deku]);

    // Assert: no library folder for it either
    assert_eq!((result.is_err(), files(splits.path())), (true, Vec::<String>::new()));
}

#[test]
fn an_add_that_fails_after_copying_a_screenshot_takes_back_the_shared_folders_it_made() {
    // Arrange: a game without presets, whose first image's screenshot is copied before the second
    // image's mask, a folder, can't be
    let (splits, preset) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut first = step(preset.path(), "EnterDeku");
    let shot = preset.path().join("shot.png");
    std::fs::write(&shot, b"full size").unwrap();
    first.screenshot = Some(shot);
    let not_a_file = preset.path().join("folder.png");
    std::fs::create_dir(&not_a_file).unwrap();
    let second = using("Deku", not_a_file);
    let presets = splits.path().join("presets");
    std::fs::create_dir(&presets).unwrap();

    // Act
    let result = add_ready_made(splits.path(), &presets, GAME, "Enter the Deku Tree", vec![first, second]);

    // Assert: the presets folder that was there stays, empty
    assert_eq!((result.is_err(), files(&presets)), (true, Vec::<String>::new()));
}

#[test]
fn a_ready_made_split_that_changed_meanwhile_isnt_changed() {
    // Arrange: the boss as it was when its window opened, then a new description written
    let (_splits, _preset, folder) = with_boss();
    let opened = entry(&folder, 0);
    change_ready_made(&folder, &presets(&folder), 0, &opened, "Changed meanwhile", None, &[]).unwrap();

    // Act
    let error = change_ready_made(&folder, &presets(&folder), 0, &opened, "From the window", None, &[]).unwrap_err();

    // Assert
    assert_eq!(
        (error.to_string().contains("changed in it since you opened it"), load(&folder).splits[0].description.as_str()),
        (true, "Changed meanwhile")
    );
}

#[test]
fn files_a_preset_being_edited_uses_are_kept() {
    // Arrange: the boss inserted into a preset that isn't saved, which uses the library's masks
    let (_splits, preset, folder) = with_boss();
    let before = contents(&folder);
    let in_use = [folder.join("Ganon_intro.png"), folder.join("Ganon.png")];

    // Act: deleting it, or replacing it with other images
    let deleted = delete_ready_made(&folder, &presets(&folder), 0, &entry(&folder, 0), &in_use);
    let replaced = change_ready_made(
        &folder,
        &presets(&folder),
        0,
        &entry(&folder, 0),
        "Flash",
        Some(vec![step(preset.path(), "Flash")]),
        &in_use,
    );

    // Assert
    assert!(deleted.unwrap_err().to_string().contains("Save the preset first"));
    assert!(replaced.unwrap_err().to_string().contains("Save the preset first"));
    assert_eq!(contents(&folder), before);
}

#[test]
fn replacing_with_the_inserted_splits_keeps_their_files() {
    // Arrange: the boss inserted and tuned in a preset, whose splits use the library's masks
    let (_splits, _preset, folder) = with_boss();
    let inserted = |name: &str, mask: &str| {
        let mut step = using(name, folder.join(mask));
        step.split.threshold = Some(0.95);
        step
    };
    let steps = vec![inserted("Ganon intro", "Ganon_intro.png"), inserted("Ganon", "Ganon.png")];
    let in_use = [folder.join("Ganon_intro.png"), folder.join("Ganon.png")];

    // Act
    change_ready_made(&folder, &presets(&folder), 0, &entry(&folder, 0), "Tuned", Some(steps), &in_use).unwrap();

    // Assert: the new threshold, the same files
    assert_eq!(
        (load(&folder).splits[0].split.threshold, files(&folder)),
        (
            Some(0.95),
            vec![
                "Ganon.png".to_owned(),
                "Ganon_intro.png".to_owned(),
                "base_Ganon.png".to_owned(),
                "base_Ganon_intro.png".to_owned(),
                LIBRARY_FILE.to_owned()
            ]
        )
    );
}

#[test]
fn adding_waits_for_a_library_that_cant_be_read() {
    // Arrange: a library folder whose file is broken, which may be this game's
    let (splits, preset) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::create_dir(splits.path().join("OcarinaOfTime")).unwrap();
    std::fs::write(
        splits.path().join("OcarinaOfTime").join(LIBRARY_FILE),
        "gmae = \"typo\"
[[split]]
",
    )
    .unwrap();

    // Act
    let error = add(splits.path(), "Enter the Deku Tree", vec![step(preset.path(), "EnterDeku")]).unwrap_err();

    // Assert: no second library beside it
    assert!(error.to_string().contains("can't be read"), "{error}");
    assert_eq!(files(splits.path()), ["OcarinaOfTime"]);
}

#[test]
fn a_file_named_in_another_case_is_still_used() {
    // Arrange: a library written by hand, whose two ready-made splits name one mask in two cases.
    // Where names don't differ by case, like on Windows, they're the same file.
    let folder = tempfile::tempdir().unwrap();
    let png = BgraImage::from_raw(4, 3, vec![255; 4 * 3 * 4]).encode_png().unwrap();
    std::fs::write(folder.path().join("Gohma.png"), png).unwrap();
    let text = "game = \"Game\"

[[split]]
name = \"Gohma\"
mask = \"Gohma.png\"

[[split]]
name = \"Gohma 2\"
mask = \"gohma.png\"
";
    std::fs::write(folder.path().join(LIBRARY_FILE), text).unwrap();

    // Act: delete the one that names it in lowercase
    delete_ready_made(folder.path(), &folder.path().join("presets"), 1, &entry(folder.path(), 1), &[]).unwrap();

    // Assert: the other one's mask is still there
    assert!(folder.path().join("Gohma.png").is_file());
}

#[test]
fn new_files_never_share_a_name_in_another_case() {
    // Arrange: two images whose names differ only in case
    let (splits, preset) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let upper = step(preset.path(), "Gohma");
    let mut lower = step(preset.path(), "gohma_low");
    lower.split.name = "gohma".into();

    // Act
    let folder = add(splits.path(), "Gohma", vec![upper, lower]).unwrap();

    // Assert: each its own mask and base image
    let library = load(&folder);
    let names: Vec<(&str, &str)> =
        (library.splits.iter()).map(|step| (step.split.mask.as_str(), step.base_image.as_str())).collect();
    assert_eq!(names, [("Gohma.png", "base_Gohma.png"), ("gohma (1).png", "base_gohma (1).png")]);
}

#[test]
fn a_new_file_named_like_another_in_another_case_gets_a_number() {
    // Arrange: a library with Gohma.png, and a split named gohma, which on Linux could otherwise
    // be written beside it and collide once the library is copied to Windows
    let (splits, preset) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let folder = add(splits.path(), "Gohma", vec![step(preset.path(), "Gohma")]).unwrap();
    let mut lower = step(preset.path(), "gohma_low");
    lower.split.name = "gohma".into();

    // Act
    add(splits.path(), "Gohma again", vec![lower]).unwrap();

    // Assert
    assert_eq!(load(&folder).splits[1].split.mask, "gohma (1).png");
}

#[test]
fn fade_step_without_a_mask_is_added_without_one() {
    // Arrange
    let directory = tempfile::tempdir().unwrap();
    let fade = PresetSplit { name: "EnterDeku".into(), fade: Some("#a0a0a0".into()), ..PresetSplit::default() };
    let step = NewStep { split: fade, mask: None, savestate: None, screenshot: None, base_picture: screenshot() };

    // Act
    let folder = add(directory.path(), "Fade into the Deku Tree", vec![step]).unwrap();

    // Assert
    let added = &load(&folder).splits[0];
    assert_eq!((added.split.mask.as_str(), added.split.fade.as_deref()), ("", Some("#a0a0a0")));
}

#[test]
fn a_librarys_shared_files_are_in_the_folder_of_its_game_as_it_spells_it() {
    // Arrange: spelled with two spaces, which is the same game as with one, in another folder
    let splits = tempfile::tempdir().unwrap();
    let folder = splits.path().join("Game");
    std::fs::create_dir(&folder).unwrap();
    let text = "game = \"Ocarina  of Time\"

[[split]]
name = \"Gohma\"
mask = \"Gohma.png\"
";
    std::fs::write(folder.join(LIBRARY_FILE), text).unwrap();

    // Act
    let game_folder = library_game_folder(&folder, &presets(&folder));

    // Assert
    assert_eq!(game_folder, Some(presets(&folder).join("Ocarina__of_Time")));
}
