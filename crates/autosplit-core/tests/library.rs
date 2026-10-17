//! Writing the splits library: ready-made splits added from a preset's splits, changed and
//! deleted, with their masks and base images.

use std::path::{Path, PathBuf};

use autosplit_core::image::BgraImage;
use autosplit_core::library::{
    BASE_IMAGE_SIZE, NewStep, add_ready_made, base_image, change_ready_made, delete_ready_made,
};
use autosplit_core::preset::{LIBRARY_FILE, PresetSplit, SplitLibrary, load_libraries};

const GAME: &str = "The Legend of Zelda: Ocarina of Time";

/// A mask file named `name` in `folder`, and an image of the split, 320x240 like a grab.
fn step(folder: &Path, name: &str) -> NewStep {
    let mask = folder.join(format!("{name}.png"));
    std::fs::write(&mask, BgraImage::from_raw(4, 3, vec![255; 4 * 3 * 4]).encode_png().unwrap()).unwrap();
    NewStep {
        split: PresetSplit {
            name: name.into(),
            mask: mask.display().to_string(),
            threshold: Some(0.9),
            ..PresetSplit::default()
        },
        mask,
        savestate: None,
        screenshot: BgraImage::from_raw(320, 240, vec![128; 320 * 240 * 4]),
    }
}

fn load(folder: &Path) -> SplitLibrary {
    SplitLibrary::load(&folder.join(LIBRARY_FILE)).unwrap()
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
fn base_images_are_made_small() {
    // Act
    let base = base_image(BgraImage::from_raw(320, 240, vec![10; 320 * 240 * 4]).view());

    // Assert
    assert_eq!((base.width(), base.height()), BASE_IMAGE_SIZE);
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
    change_ready_made(&folder, 0, "Final slash on Ganon", None).unwrap();

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
    change_ready_made(&folder, 0, "Just the flash", Some(vec![step(preset.path(), "Flash")])).unwrap();

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
    let again = NewStep {
        split: PresetSplit { name: "Ganon".into(), mask: mask.display().to_string(), ..PresetSplit::default() },
        mask,
        savestate: None,
        screenshot: BgraImage::from_raw(320, 240, vec![128; 320 * 240 * 4]),
    };

    // Act
    change_ready_made(&folder, 0, "Just the flash", Some(vec![again])).unwrap();

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
    let flash = NewStep {
        split: PresetSplit { name: "Flash".into(), ..PresetSplit::default() },
        mask: folder.join("Ganon.png"),
        savestate: None,
        screenshot: BgraImage::from_raw(320, 240, vec![128; 320 * 240 * 4]),
    };
    add_ready_made(splits.path(), GAME, "Any flash", vec![flash]).unwrap();

    // Act
    delete_ready_made(&folder, 0).unwrap();

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
    let error = delete_ready_made(&folder, 5).unwrap_err();

    // Assert
    assert!(error.to_string().contains("no ready-made split 6"), "{error}");
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
fn later_images_have_no_empty_description_line() {
    // Arrange
    let (_splits, _preset, folder) = with_boss();

    // Act
    let text = std::fs::read_to_string(folder.join(LIBRARY_FILE)).unwrap();

    // Assert
    assert_eq!(text.matches("description").count(), 1, "{text}");
}

#[test]
fn replaced_images_keep_their_file_names() {
    // Arrange: the boss made again from the preset's splits of the same names
    let (_splits, preset, folder) = with_boss();
    let mut intro = step(preset.path(), "Ganon intro");
    intro.split.dummy = true;

    // Act
    change_ready_made(&folder, 0, "The intro text, then the flash", Some(vec![intro, step(preset.path(), "Ganon")]))
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
    let flash = NewStep {
        split: PresetSplit { name: "Flash".into(), ..PresetSplit::default() },
        mask: folder.join("Ganon.png"),
        savestate: None,
        screenshot: BgraImage::from_raw(320, 240, vec![128; 320 * 240 * 4]),
    };
    add_ready_made(splits.path(), GAME, "Any flash", vec![flash]).unwrap();

    // Act
    change_ready_made(&folder, 0, "Just the flash", Some(vec![step(preset.path(), "Ganon")])).unwrap();

    // Assert: the boss's new mask got a name of its own
    let library = load(&folder);
    assert_eq!(
        (library.splits[0].split.mask.as_str(), library.splits[1].split.mask.as_str()),
        ("Ganon (1).png", "Ganon.png")
    );
}
