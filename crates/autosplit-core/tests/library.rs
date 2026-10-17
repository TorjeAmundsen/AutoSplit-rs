//! Writing the splits library: ready-made splits added from a preset's splits, changed and
//! deleted, with their masks and base images.

use std::path::{Path, PathBuf};

use autosplit_core::image::BgraImage;
use autosplit_core::library::{
    BASE_IMAGE_SIZE, NewStep, add_ready_made, base_image, change_ready_made, delete_ready_made,
};
use autosplit_core::preset::{LIBRARY_FILE, LibrarySplit, PresetSplit, SplitLibrary, load_libraries};

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
        mask,
        savestate: None,
        screenshot: screenshot(),
    }
}

/// A gray picture the size of a grab.
fn screenshot() -> BgraImage {
    BgraImage::from_raw(320, 240, vec![128; 320 * 240 * 4])
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
    let folder = add_ready_made(splits.path(), GAME, "The intro text, then the flash", steps).unwrap();
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

    // Assert: it loads, one ready-made split of two images, with copies of their masks
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
}

#[test]
fn base_images_are_written_small() {
    // Arrange
    let (_splits, _preset, folder) = with_boss();

    // Act
    let base = BgraImage::open_png(&folder.join("base_Ganon.png")).unwrap();

    // Assert
    assert_eq!((base.width(), base.height()), BASE_IMAGE_SIZE);
}

#[test]
fn a_game_with_a_library_gets_another_ready_made_split() {
    // Arrange
    let (splits, preset, folder) = with_boss();

    // Act
    let added =
        add_ready_made(splits.path(), GAME, "Bottle over the head", vec![step(preset.path(), "Bottle")]).unwrap();

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
    change_ready_made(&folder, 0, &entry(&folder, 0), "Final slash on Ganon", None, &[]).unwrap();

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
    change_ready_made(&folder, 0, &entry(&folder, 0), "Just the flash", Some(vec![step(preset.path(), "Flash")]), &[])
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
    change_ready_made(&folder, 0, &entry(&folder, 0), "Just the flash", Some(vec![again]), &[]).unwrap();

    // Assert: no copy, its base image under the same name, and the intro's files gone
    assert_eq!(
        (load(&folder).splits[0].split.mask.as_str(), files(&folder)),
        ("Ganon.png", vec!["Ganon.png".to_owned(), "base_Ganon.png".to_owned(), LIBRARY_FILE.to_owned()])
    );
}

#[test]
fn deleting_a_ready_made_split_deletes_the_files_only_it_used() {
    // Arrange: a second ready-made split that uses the boss's flash mask too
    let (splits, _preset, folder) = with_boss();
    let flash = using("Flash", folder.join("Ganon.png"));
    add_ready_made(splits.path(), GAME, "Any flash", vec![flash]).unwrap();

    // Act
    delete_ready_made(&folder, 0, &entry(&folder, 0), &[]).unwrap();

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
    let error = delete_ready_made(&folder, 5, &[], &[]).unwrap_err();

    // Assert
    assert!(error.to_string().contains("changed in it since you opened it"), "{error}");
}

#[test]
fn savestates_are_copied_into_the_library_too() {
    // Arrange: a split whose savestate is in its preset's savestates folder
    let (splits, preset) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    std::fs::create_dir(preset.path().join("savestates")).unwrap();
    let savestate = preset.path().join("savestates").join("004-kokiri forest.gzs");
    std::fs::write(&savestate, b"state").unwrap();
    let mut deku = step(preset.path(), "EnterDeku");
    deku.split.savestate = "savestates/004-kokiri forest.gzs".into();
    deku.savestate = Some(savestate);

    // Act
    let folder = add_ready_made(splits.path(), GAME, "Enter the Deku Tree", vec![deku]).unwrap();

    // Assert: next to the mask, under its own name, which the library says
    let library = load(&folder);
    assert_eq!(
        (library.splits[0].split.savestate.as_str(), folder.join("004-kokiri forest.gzs").is_file()),
        ("004-kokiri forest.gzs", true)
    );
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
fn files_other_ready_made_splits_use_are_never_written_over() {
    // Arrange: a flash that uses the boss's mask, then the boss made again from a new mask
    let (splits, preset, folder) = with_boss();
    let flash = using("Flash", folder.join("Ganon.png"));
    add_ready_made(splits.path(), GAME, "Any flash", vec![flash]).unwrap();

    // Act
    change_ready_made(&folder, 0, &entry(&folder, 0), "Just the flash", Some(vec![step(preset.path(), "Ganon")]), &[])
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
    // Arrange: the boss made again, its second image's savestate missing
    let (_splits, preset, folder) = with_boss();
    let before = contents(&folder);
    let mut flash = step(preset.path(), "Ganon");
    flash.savestate = Some(preset.path().join("missing.gzs"));
    let mut intro = step(preset.path(), "Ganon intro");
    std::fs::write(&intro.mask, b"another mask").unwrap();
    intro.split.dummy = true;

    // Act
    let result = change_ready_made(&folder, 0, &entry(&folder, 0), "Again", Some(vec![intro, flash]), &[]);

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
    let result = add_ready_made(splits.path(), GAME, "Enter the Deku Tree", vec![deku]);

    // Assert: no library folder for it either
    assert_eq!((result.is_err(), files(splits.path())), (true, Vec::<String>::new()));
}

#[test]
fn a_ready_made_split_that_changed_meanwhile_isnt_changed() {
    // Arrange: the boss as it was when its window opened, then a new description written
    let (_splits, _preset, folder) = with_boss();
    let opened = entry(&folder, 0);
    change_ready_made(&folder, 0, &opened, "Changed meanwhile", None, &[]).unwrap();

    // Act
    let error = change_ready_made(&folder, 0, &opened, "From the window", None, &[]).unwrap_err();

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
    let deleted = delete_ready_made(&folder, 0, &entry(&folder, 0), &in_use);
    let replaced =
        change_ready_made(&folder, 0, &entry(&folder, 0), "Flash", Some(vec![step(preset.path(), "Flash")]), &in_use);

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
    change_ready_made(&folder, 0, &entry(&folder, 0), "Tuned", Some(steps), &in_use).unwrap();

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
    let error =
        add_ready_made(splits.path(), GAME, "Enter the Deku Tree", vec![step(preset.path(), "EnterDeku")]).unwrap_err();

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
    delete_ready_made(folder.path(), 1, &entry(folder.path(), 1), &[]).unwrap();

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
    let folder = add_ready_made(splits.path(), GAME, "Gohma", vec![upper, lower]).unwrap();

    // Assert: each its own mask and base image
    let names: Vec<String> = load(&folder).splits.iter().map(|step| step.split.mask.to_lowercase()).collect();
    assert_ne!(names[0], names[1]);
}

#[test]
fn a_new_file_named_like_another_in_another_case_gets_a_number() {
    // Arrange: a library with Gohma.png, and a split named gohma, which on Linux could otherwise
    // be written beside it and collide once the library is copied to Windows
    let (splits, preset) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let folder = add_ready_made(splits.path(), GAME, "Gohma", vec![step(preset.path(), "Gohma")]).unwrap();
    let mut lower = step(preset.path(), "gohma_low");
    lower.split.name = "gohma".into();

    // Act
    add_ready_made(splits.path(), GAME, "Gohma again", vec![lower]).unwrap();

    // Assert
    assert_eq!(load(&folder).splits[1].split.mask, "gohma (1).png");
}
