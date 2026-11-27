//! Saving presets: files copied in, names that don't clash, and files the preset stopped using.

use std::path::Path;

use autosplit_core::compare::ComparisonMethod;
use autosplit_core::preset::{
    LIBRARY_FILE, PRESET_FILE, Preset, PresetError, PresetSplit, SAVESTATE_NOTE_FILE, delete_preset,
    duplicate_savestates, duplicates_among, keeps_own_files, load_presets, move_to_game_folder, new_preset_folder,
    sanitize_file_name, save_in_presets, save_preset, sweep_shared_files, unique_file_name,
};
use test_case::test_case;

fn split(name: &str, mask: &str) -> PresetSplit {
    PresetSplit { name: name.into(), mask: mask.into(), ..PresetSplit::default() }
}

fn preset(splits: Vec<PresetSplit>) -> Preset {
    Preset { name: "Any%".into(), game: "Game".into(), splits }
}

/// A file with `contents` at `path`, with its folders.
fn file(path: &Path, contents: &str) -> String {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
    path.display().to_string()
}

/// The files in `folder` and its folders, relative, sorted.
fn files(folder: &Path) -> Vec<String> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(folder).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.path().is_dir() {
            found.extend(files(&entry.path()).into_iter().map(|inner| format!("{name}/{inner}")));
        } else {
            found.push(name);
        }
    }
    found.sort();
    found
}

#[test_case("No Wrong Warp" => "No_Wrong_Warp"; "spaces")]
#[test_case("Any% (Glitchless): 100?" => "Any%_(Glitchless)_100"; "characters windows doesn't allow")]
#[test_case("  ...  " => "Preset"; "nothing left")]
#[test_case("Run." => "Run"; "trailing dot")]
#[test_case("con" => "con_"; "device name")]
#[test_case("COM1.txt" => "COM1_.txt"; "device name with an extension")]
#[test_case("CON.v2.gzs" => "CON_.v2.gzs"; "device name with several dots")]
fn names_work_on_every_system(name: &str) -> String {
    // Act, with the expected value checked by test_case
    sanitize_file_name(name, "Preset")
}

#[test]
fn taken_names_get_a_number() {
    // Arrange
    let taken = ["boss.png", "boss (1).png"];

    // Act
    let name = unique_file_name("boss.png", |name| taken.contains(&name));

    // Assert
    assert_eq!(name, "boss (2).png");
}

/// A preset of `game` saved in `folder`, with one mask.
fn saved(folder: &Path, game: &str) {
    file(&folder.join("boss.png"), "mask");
    save_preset(Some(folder), folder, None, &Preset { game: game.into(), ..preset(vec![split("Boss", "boss.png")]) })
        .unwrap();
}

#[test]
fn new_folders_go_in_a_folder_named_after_the_game_and_dont_reuse_existing_ones() {
    // Arrange: a game without presets yet, whose folder has an Any% in it
    let presets = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(presets.path().join("Mario_64").join("Any%")).unwrap();

    // Act
    let folder = new_preset_folder(presets.path(), "Mario 64", "Any%");

    // Assert
    assert_eq!(folder, presets.path().join("Mario_64").join("Any%_2"));
}

#[test]
fn a_preset_kept_right_in_the_presets_folder_isnt_a_game_folder() {
    // Arrange: a preset from before game folders, whose folder is named like the game
    let presets = tempfile::tempdir().unwrap();
    saved(&presets.path().join("Game"), "Game");

    // Act
    let folder = new_preset_folder(presets.path(), "Game", "Any%");

    // Assert
    assert_eq!(folder, presets.path().join("Game_2").join("Any%"));
}

#[test]
fn a_preset_whose_game_changed_moves_to_the_new_games_folder() {
    // Arrange: the only preset in its game's folder
    let presets = tempfile::tempdir().unwrap();
    let folder = presets.path().join("Old_Game").join("Any%");
    saved(&folder, "Old Game");

    // Act
    let moved = move_to_game_folder(presets.path(), &folder, "New Game").unwrap();

    // Assert: with its files, and the old game's folder it left empty is gone
    assert_eq!(moved, presets.path().join("New_Game").join("Any%"));
    assert_eq!(files(&moved), ["boss.png", PRESET_FILE]);
    assert!(!presets.path().join("Old_Game").exists());
}

#[test]
fn the_old_games_folder_stays_while_it_has_other_presets() {
    // Arrange: two presets of the old game, and the new game's folder with an Any% already
    let presets = tempfile::tempdir().unwrap();
    let folder = presets.path().join("Old_Game").join("Any%");
    saved(&folder, "Old Game");
    saved(&presets.path().join("Old_Game").join("100%"), "Old Game");
    saved(&presets.path().join("New_Game").join("Any%"), "New Game");

    // Act
    let moved = move_to_game_folder(presets.path(), &folder, "New Game").unwrap();

    // Assert: a number for the taken name, and the other preset where it was
    assert_eq!(moved, presets.path().join("New_Game").join("Any%_2"));
    assert!(presets.path().join("Old_Game").join("100%").join(PRESET_FILE).is_file());
}

#[test]
fn a_preset_already_in_its_games_folder_stays() {
    // Arrange: a game whose name only differs in what folder names leave out
    let presets = tempfile::tempdir().unwrap();
    let folder = presets.path().join("Game").join("Any%");
    saved(&folder, "Game");

    // Act
    let moved = move_to_game_folder(presets.path(), &folder, "Game?").unwrap();

    // Assert
    assert_eq!(moved, folder);
    assert!(folder.join(PRESET_FILE).is_file());
}

#[test]
fn a_preset_from_before_game_folders_moves_into_one() {
    // Arrange: right in the presets folder
    let presets = tempfile::tempdir().unwrap();
    let folder = presets.path().join("Any%");
    saved(&folder, "Old Game");

    // Act
    let moved = move_to_game_folder(presets.path(), &folder, "New Game").unwrap();

    // Assert
    assert_eq!(moved, presets.path().join("New_Game").join("Any%"));
}

#[test]
fn a_game_folder_left_with_only_finders_file_is_removed() {
    // Arrange: the old game's folder was opened in Finder, which left its .DS_Store
    let presets = tempfile::tempdir().unwrap();
    let folder = presets.path().join("Old_Game").join("Any%");
    saved(&folder, "Old Game");
    file(&presets.path().join("Old_Game").join(".DS_Store"), "finder");

    // Act
    move_to_game_folder(presets.path(), &folder, "New Game").unwrap();

    // Assert
    assert!(!presets.path().join("Old_Game").exists());
}

#[test_case("mario_64", "mario 64"; "in another case")]
#[test_case("Mario__64", "Mario  64"; "with two spaces")]
fn a_game_folder_of_the_game_typed_another_way_is_the_games_folder(on_disk: &str, typed: &str) {
    // Arrange: the game's folder, as it was named when the game was typed another way
    let presets = tempfile::tempdir().unwrap();
    saved(&presets.path().join(on_disk).join("Any%"), typed);

    // Act
    let folder = new_preset_folder(presets.path(), "Mario 64", "100%");

    // Assert: the folder as it's spelled on disk, also where names differ by case, like Linux
    assert_eq!(folder, presets.path().join(on_disk).join("100%"));
}

#[test]
fn a_preset_folder_named_like_another_in_another_case_gets_a_number() {
    // Arrange: a game's folder with an any%, which Windows and macOS take for Any%
    let presets = tempfile::tempdir().unwrap();
    saved(&presets.path().join("Mario_64").join("any%"), "Mario 64");

    // Act
    let folder = new_preset_folder(presets.path(), "Mario 64", "Any%");

    // Assert
    assert_eq!(folder, presets.path().join("Mario_64").join("Any%_2"));
}

// Only Windows refuses to move a folder with a file in it open
#[cfg(windows)]
#[test]
fn a_move_that_fails_leaves_no_new_game_folder() {
    // Arrange: one of the preset's files is open, like in an image editor
    let presets = tempfile::tempdir().unwrap();
    let folder = presets.path().join("Old_Game").join("Any%");
    saved(&folder, "Old Game");
    let _open = std::fs::File::open(folder.join("boss.png")).unwrap();

    // Act
    let moved = move_to_game_folder(presets.path(), &folder, "New Game");

    // Assert: the preset where it was, and no folder made for the new game
    assert!(matches!(moved, Err(PresetError::Unwritable { .. })), "{moved:?}");
    assert!(folder.join(PRESET_FILE).is_file());
    assert!(!presets.path().join("New_Game").exists());
}

#[test]
fn presets_load_from_game_folders_and_from_before_them() {
    // Arrange: one right in the presets folder, one in a game's folder, and a game folder's
    // leftover hidden folder
    let presets = tempfile::tempdir().unwrap();
    let old = presets.path().join("Old");
    let new = presets.path().join("Game").join("New");
    for folder in [&old, &new, &presets.path().join("Game").join(".deleting-Gone-1")] {
        saved(folder, "Game");
    }

    // Act
    let (loaded, errors) = load_presets(presets.path());

    // Assert
    let folders: Vec<&Path> = loaded.iter().map(|loaded| loaded.folder.as_path()).collect();
    assert_eq!((folders, errors.len()), (vec![new.as_path(), old.as_path()], 0));
}

#[test]
fn picked_files_are_copied_in_once_and_named_for_their_split() {
    // Arrange
    let picked = tempfile::tempdir().unwrap();
    let mask = file(&picked.path().join("mask.png"), "mask");
    let savestate = file(&picked.path().join("deku tree.gzs"), "state");
    let target = tempfile::tempdir().unwrap();
    let mut first = split("Enter Deku", &mask);
    first.savestate = savestate.clone();
    let mut second = split("ExitDeku", &mask);
    second.savestate = savestate;

    // Act
    let saved = save_preset(None, target.path(), None, &preset(vec![first, second])).unwrap();

    // Assert: one copy each, which both splits use
    let paths: Vec<(&str, &str)> =
        saved.splits.iter().map(|split| (split.mask.as_str(), split.savestate.as_str())).collect();
    assert_eq!(
        (paths, files(target.path())),
        (
            vec![("Enter_Deku.png", "savestates/deku_tree.gzs"), ("Enter_Deku.png", "savestates/deku_tree.gzs")],
            vec![
                "Enter_Deku.png".to_owned(),
                PRESET_FILE.to_owned(),
                format!("savestates/{SAVESTATE_NOTE_FILE}"),
                "savestates/deku_tree.gzs".to_owned()
            ]
        )
    );
}

#[test]
fn copies_dont_replace_files_that_are_there() {
    // Arrange
    let picked = tempfile::tempdir().unwrap();
    let mask = file(&picked.path().join("new.png"), "new");
    let target = tempfile::tempdir().unwrap();
    file(&target.path().join("Boss.png"), "someone else's");

    // Act
    let saved = save_preset(None, target.path(), None, &preset(vec![split("Boss", &mask)])).unwrap();

    // Assert
    assert_eq!(
        (saved.splits[0].mask.as_str(), std::fs::read_to_string(target.path().join("Boss.png")).unwrap()),
        ("Boss (1).png", "someone else's".to_owned())
    );
}

#[test]
fn a_replaced_mask_keeps_its_name() {
    // Arrange: the preset's Boss mask, and a new one picked for it
    let picked = tempfile::tempdir().unwrap();
    let mask = file(&picked.path().join("new.png"), "new");
    let folder = tempfile::tempdir().unwrap();
    file(&folder.path().join("Boss.png"), "old");
    save_preset(Some(folder.path()), folder.path(), None, &preset(vec![split("Boss", "Boss.png")])).unwrap();

    // Act
    let saved = save_preset(Some(folder.path()), folder.path(), None, &preset(vec![split("Boss", &mask)])).unwrap();

    // Assert
    assert_eq!(
        (saved.splits[0].mask.as_str(), std::fs::read_to_string(folder.path().join("Boss.png")).unwrap()),
        ("Boss.png", "new".to_owned())
    );
    assert_eq!(files(folder.path()), ["Boss.png", PRESET_FILE]);
}

#[test]
fn a_failed_save_puts_back_a_replaced_mask() {
    // Arrange: Boss's new mask is copied over the old one's name, then Door's mask, a folder,
    // can't be
    let picked = tempfile::tempdir().unwrap();
    let mask = file(&picked.path().join("new.png"), "new");
    let not_a_file = picked.path().join("folder.png");
    std::fs::create_dir(&not_a_file).unwrap();
    let folder = tempfile::tempdir().unwrap();
    file(&folder.path().join("Boss.png"), "old");
    save_preset(Some(folder.path()), folder.path(), None, &preset(vec![split("Boss", "Boss.png")])).unwrap();

    // Act
    let splits = vec![split("Boss", &mask), split("Door", &not_a_file.display().to_string())];
    let error = save_preset(Some(folder.path()), folder.path(), None, &preset(splits));

    // Assert
    assert_eq!(
        (error.is_err(), files(folder.path()), std::fs::read_to_string(folder.path().join("Boss.png")).unwrap()),
        (true, vec!["Boss.png".to_owned(), PRESET_FILE.to_owned()], "old".to_owned())
    );
}

#[test]
fn files_in_the_folder_stay_where_they_are() {
    // Arrange
    let folder = tempfile::tempdir().unwrap();
    file(&folder.path().join("masks/boss.png"), "mask");
    let loaded = preset(vec![split("Boss", "masks/boss.png")]);

    // Act
    let saved = save_preset(Some(folder.path()), folder.path(), None, &loaded).unwrap();

    // Assert
    assert_eq!((saved, files(folder.path())), (loaded, vec!["masks/boss.png".to_owned(), PRESET_FILE.to_owned()]));
}

#[test]
fn savestates_with_the_same_contents_in_other_files_are_found() {
    // Arrange: reset and start in two files with the same contents, and one picked from elsewhere
    let (folder, elsewhere) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    file(&folder.path().join("savestates/000 start and reset.gzs"), "link's house");
    file(&folder.path().join("savestates/000 start and reset (1).gzs"), "link's house");
    file(&folder.path().join("savestates/001 escape.gzs"), "lost woods");
    file(&folder.path().join("savestates/002 kakariko.gzs"), "kakariko!!");
    let picked = file(&elsewhere.path().join("downloaded.gzs"), "link's house");
    let with_savestate = |name: &str, savestate: &str| PresetSplit { savestate: savestate.into(), ..split(name, "") };
    let loaded = preset(vec![
        with_savestate("reset", "savestates/000 start and reset.gzs"),
        with_savestate("start_auto_splitter", "savestates/000 start and reset (1).gzs"),
        with_savestate("Escape", "savestates/001 escape.gzs"),
        with_savestate("Escape 2", &picked),
        with_savestate("Same file", "savestates/000 start and reset.gzs"),
        split("No savestate", ""),
        with_savestate("Same size", "savestates/002 kakariko.gzs"),
    ]);

    // Act
    let duplicates = duplicate_savestates(folder.path(), &loaded);

    // Assert: each with the first's, none for the one already using its file, and none for one
    // the same size as another but with other contents
    let first = "savestates/000 start and reset.gzs".to_owned();
    assert_eq!(duplicates, vec![(1, first.clone()), (3, first)]);
}

#[test]
fn files_already_there_keep_the_first_and_never_trade() {
    // Arrange: two files the images use with the same contents, another one of them, and an image
    // without a file
    let folder = tempfile::tempdir().unwrap();
    let (a, b, c) = (folder.path().join("a.gzs"), folder.path().join("b.gzs"), folder.path().join("c.gzs"));
    file(&a, "link's house");
    file(&b, "link's house");
    file(&c, "lost woods");
    let files = [Some(a.as_path()), None, Some(b.as_path()), Some(c.as_path())];

    // Act
    let duplicates = duplicates_among(&files, [a.as_path(), b.as_path(), c.as_path()]);

    // Assert: b takes a, and a keeps its own instead of taking b
    assert_eq!(duplicates, vec![(2, a.clone())]);
}

#[test]
fn saving_twice_changes_nothing() {
    // Arrange
    let picked = tempfile::tempdir().unwrap();
    let mask = file(&picked.path().join("mask.png"), "mask");
    let target = tempfile::tempdir().unwrap();
    let saved = save_preset(None, target.path(), None, &preset(vec![split("Boss", &mask)])).unwrap();
    let before = files(target.path());

    // Act
    let again = save_preset(Some(target.path()), target.path(), None, &saved).unwrap();

    // Assert
    assert_eq!((again, files(target.path())), (saved, before));
}

#[test]
fn saving_as_new_copies_everything_and_leaves_the_old_preset() {
    // Arrange
    let presets = tempfile::tempdir().unwrap();
    let old = presets.path().join("Any%");
    file(&old.join("boss.png"), "mask");
    file(&old.join("savestates/boss.gzs"), "state");
    let mut boss = split("Boss", "boss.png");
    boss.savestate = "savestates/boss.gzs".into();
    let loaded = save_preset(Some(&old), &old, None, &preset(vec![boss])).unwrap();
    let new = new_preset_folder(presets.path(), "Game", "Any%");

    // Act
    let saved = save_preset(Some(&old), &new, None, &loaded).unwrap();

    // Assert
    assert_eq!(
        (files(&new), files(&old), saved.splits[0].savestate.as_str()),
        (
            vec![
                "Boss.png".to_owned(),
                PRESET_FILE.to_owned(),
                format!("savestates/{SAVESTATE_NOTE_FILE}"),
                "savestates/boss.gzs".to_owned()
            ],
            vec![
                "boss.png".to_owned(),
                PRESET_FILE.to_owned(),
                format!("savestates/{SAVESTATE_NOTE_FILE}"),
                "savestates/boss.gzs".to_owned()
            ],
            "savestates/boss.gzs"
        )
    );
}

#[test]
fn files_the_preset_stopped_using_are_deleted() {
    // Arrange: a mask, savestate and screenshot the preset used, and files it never had
    let folder = tempfile::tempdir().unwrap();
    file(&folder.path().join("boss.png"), "mask");
    file(&folder.path().join("door.png"), "mask");
    file(&folder.path().join("notes.png"), "not the preset's");
    file(&folder.path().join("savestates/boss.gzs"), "state");
    file(&folder.path().join("savestates/stray.gzs"), "state");
    file(&folder.path().join("screenshots/boss.png"), "screen");
    file(&folder.path().join("screenshots/stray.png"), "screen");
    let mut boss = split("Boss", "boss.png");
    boss.savestate = "savestates/boss.gzs".into();
    boss.screenshot = "screenshots/boss.png".into();
    save_preset(Some(folder.path()), folder.path(), None, &preset(vec![boss, split("Door", "door.png")])).unwrap();

    // Act: the boss split is removed
    save_preset(Some(folder.path()), folder.path(), None, &preset(vec![split("Door", "door.png")])).unwrap();

    // Assert
    assert_eq!(
        files(folder.path()),
        [
            "door.png",
            "notes.png",
            PRESET_FILE,
            &format!("savestates/{SAVESTATE_NOTE_FILE}"),
            "savestates/stray.gzs",
            "screenshots/stray.png"
        ]
    );
}

#[test_case("./boss.png"; "dot slash")]
#[test_case("masks/../boss.png"; "through another folder")]
fn masks_written_another_way_are_still_used(written: &str) {
    // Arrange: a hand-written preset, which saving writes as boss.png
    let folder = tempfile::tempdir().unwrap();
    file(&folder.path().join("boss.png"), "mask");
    std::fs::create_dir(folder.path().join("masks")).unwrap();
    let loaded = preset(vec![split("Boss", written)]);
    std::fs::write(folder.path().join(PRESET_FILE), loaded.to_toml()).unwrap();

    // Act
    let saved = save_preset(Some(folder.path()), folder.path(), None, &loaded).unwrap();

    // Assert
    assert_eq!((saved.splits[0].mask.as_str(), folder.path().join("boss.png").is_file()), ("boss.png", true));
}

#[test]
fn a_failed_save_leaves_the_folder_as_it_was() {
    // Arrange: the first split's mask is copied in, then the second's, a folder, can't be
    let picked = tempfile::tempdir().unwrap();
    let mask = file(&picked.path().join("mask.png"), "mask");
    let not_a_file = picked.path().join("folder.png");
    std::fs::create_dir(&not_a_file).unwrap();
    let folder = tempfile::tempdir().unwrap();
    file(&folder.path().join("notes.txt"), "kept");

    // Act
    let splits = vec![split("Boss", &mask), split("Door", &not_a_file.display().to_string())];
    let error = save_preset(None, folder.path(), None, &preset(splits));

    // Assert
    assert_eq!((error.is_err(), files(folder.path())), (true, vec!["notes.txt".to_owned()]));
}

#[test]
fn a_failed_save_as_new_leaves_no_folder() {
    // Arrange: the second split's mask is a folder, which can't be copied
    let picked = tempfile::tempdir().unwrap();
    let mask = file(&picked.path().join("mask.png"), "mask");
    let not_a_file = picked.path().join("folder.png");
    std::fs::create_dir(&not_a_file).unwrap();
    let presets = tempfile::tempdir().unwrap();
    let new = new_preset_folder(presets.path(), "Game", "Any%");

    // Act
    let splits = vec![split("Boss", &mask), split("Door", &not_a_file.display().to_string())];
    let error = save_preset(None, &new, None, &preset(splits));

    // Assert: the game's folder made for it goes too
    assert_eq!((error.is_err(), new.exists(), presets.path().join("Game").exists()), (true, false, false));
}

#[test]
fn files_picked_for_splits_with_the_same_name_get_their_own_copies() {
    // Arrange: two different masks, and savestates whose names differ only by case
    let picked = tempfile::tempdir().unwrap();
    let (first, second) = (file(&picked.path().join("a.png"), "a"), file(&picked.path().join("b.png"), "b"));
    let mut boss = split("Boss", &first);
    boss.savestate = file(&picked.path().join("one/boss.gzs"), "1");
    let mut again = split("Boss", &second);
    again.savestate = file(&picked.path().join("two/Boss.gzs"), "2");
    let target = tempfile::tempdir().unwrap();

    // Act
    let saved = save_preset(None, target.path(), None, &preset(vec![boss, again])).unwrap();

    // Assert
    let paths: Vec<(&str, &str)> =
        saved.splits.iter().map(|split| (split.mask.as_str(), split.savestate.as_str())).collect();
    assert_eq!(paths, [("Boss.png", "savestates/boss.gzs"), ("Boss (1).png", "savestates/Boss (1).gzs")]);
}

#[test]
fn masks_of_names_that_sanitize_the_same_get_their_own_copies() {
    // Arrange: names that only differ in characters file names can't have
    let picked = tempfile::tempdir().unwrap();
    let (first, second) = (file(&picked.path().join("a.png"), "a"), file(&picked.path().join("b.png"), "b"));
    let target = tempfile::tempdir().unwrap();

    // Act
    let saved =
        save_preset(None, target.path(), None, &preset(vec![split("Boss?", &first), split("Boss:", &second)])).unwrap();

    // Assert: one name each, and each copy is its own mask
    let masks: Vec<&str> = saved.splits.iter().map(|split| split.mask.as_str()).collect();
    assert_eq!(masks, ["Boss.png", "Boss (1).png"]);
    let read = |name: &str| std::fs::read_to_string(target.path().join(name)).unwrap();
    assert_eq!((read("Boss.png"), read("Boss (1).png")), ("a".to_owned(), "b".to_owned()));
}

#[test]
fn a_preset_of_only_a_reset_image_saves() {
    // Arrange: the editor says what such a preset is missing, but saving doesn't refuse it
    let picked = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let reset = split("reset", &file(&picked.path().join("reset.png"), "r"));

    // Act
    let saved = save_preset(None, target.path(), None, &preset(vec![reset])).unwrap();

    // Assert
    assert_eq!(Preset::load(&target.path().join(PRESET_FILE)).unwrap(), saved);
    assert_eq!(files(target.path()), [PRESET_FILE, "reset.png"]);
}

#[test]
fn a_preset_with_every_field_set_reads_back_as_saved() {
    // Arrange
    let picked = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let first = split("Deku", &file(&picked.path().join("deku.png"), "d"));
    let second = PresetSplit {
        name: "Door".into(),
        with_previous: true,
        off: true,
        mask: file(&picked.path().join("door.png"), "o"),
        screenshot: file(&picked.path().join("door screen.png"), "c"),
        fade: Some("#a0a0a0".into()),
        fade_time: Some(1000),
        comparison_method: ComparisonMethod::Histograms,
        threshold: Some(0.93),
        pause: Some(1.5),
        delay: Some(250),
        dummy: true,
        below: true,
        savestate: file(&picked.path().join("door.gzs"), "s"),
        instructions: "Advance 1 frame".into(),
        // Only the editor's, never written
        remembered: Default::default(),
    };

    // Act
    let saved = save_preset(None, target.path(), None, &preset(vec![first, second.clone()])).unwrap();

    // Assert: everything but the paths, which became the copies', is as given
    let read = Preset::load(&target.path().join(PRESET_FILE)).unwrap();
    assert_eq!(read, saved);
    let expected = PresetSplit {
        mask: "Door.png".into(),
        savestate: "savestates/door.gzs".into(),
        screenshot: "screenshots/Door.png".into(),
        ..second
    };
    assert_eq!(read.splits[1], expected);
    // The mask and the screenshot share a name in their own folders, each with its contents
    let contents = |name: &str| std::fs::read_to_string(target.path().join(name)).unwrap();
    assert_eq!((contents("Door.png"), contents("screenshots/Door.png")), ("o".to_owned(), "c".to_owned()));
}

#[test]
fn missing_files_stop_the_save_before_anything_is_written() {
    // Arrange
    let target = tempfile::tempdir().unwrap();
    let missing = target.path().join("gone.png");

    // Act
    let error = save_preset(None, target.path(), None, &preset(vec![split("Boss", &missing.display().to_string())]));

    // Assert
    assert_eq!(
        (error, files(target.path())),
        (Err(PresetError::Unreadable { path: missing, reason: "the file doesn't exist".into() }), Vec::new())
    );
}

#[test]
fn deleted_presets_are_gone() {
    // Arrange: two presets
    let presets = tempfile::tempdir().unwrap();
    for name in ["Any%", "100%"] {
        let folder = presets.path().join(name);
        file(&folder.join("boss.png"), "mask");
        save_preset(Some(&folder), &folder, None, &preset(vec![split("Boss", "boss.png")])).unwrap();
    }

    // Act
    delete_preset(&presets.path().join("Any%")).unwrap();

    // Assert
    assert_eq!(files(presets.path()), ["100%/boss.png", &format!("100%/{PRESET_FILE}")]);
}

#[test]
fn what_is_left_of_a_deleted_preset_isnt_a_preset() {
    // Arrange: a delete that stopped partway, which left the renamed folder
    let presets = tempfile::tempdir().unwrap();
    let left = presets.path().join(".deleting-Any%-1");
    file(&left.join("boss.png"), "mask");
    std::fs::write(left.join(PRESET_FILE), preset(vec![split("Boss", "boss.png")]).to_toml()).unwrap();

    // Act
    let (loaded, errors) = load_presets(presets.path());

    // Assert
    assert_eq!((loaded.len(), errors.len()), (0, 0));
}

#[test]
fn deleting_again_removes_what_was_left() {
    // Arrange
    let presets = tempfile::tempdir().unwrap();
    file(&presets.path().join(".deleting-Old-1/boss.png"), "mask");
    let folder = presets.path().join("Any%");
    file(&folder.join("boss.png"), "mask");

    // Act
    delete_preset(&folder).unwrap();

    // Assert
    assert_eq!(files(presets.path()), Vec::<String>::new());
}

/// A split with a screenshot, which presets of a game share, and a mask.
fn pictured(name: &str, mask: &str, screenshot: &str) -> PresetSplit {
    PresetSplit { screenshot: screenshot.into(), ..split(name, mask) }
}

#[test]
fn screenshots_with_the_same_contents_are_shared_by_a_games_presets() {
    // Arrange: the second preset's Boss screenshot is another file with the same contents, and
    // its Door screenshot differs
    let picked = tempfile::tempdir().unwrap();
    let mask = file(&picked.path().join("mask.png"), "mask");
    let boss = file(&picked.path().join("boss.png"), "boss");
    let same = file(&picked.path().join("again/boss.png"), "boss");
    let door = file(&picked.path().join("door.png"), "door");
    let game = tempfile::tempdir().unwrap();
    let first = game.path().join("Any%");
    let second = game.path().join("100%");
    save_preset(None, &first, Some(game.path()), &preset(vec![pictured("Boss", &mask, &boss)])).unwrap();

    // Act
    let splits = vec![pictured("Boss", &mask, &same), pictured("Door", &mask, &door)];
    let saved = save_preset(None, &second, Some(game.path()), &preset(splits)).unwrap();

    // Assert
    let screenshots: Vec<&str> = saved.splits.iter().map(|split| split.screenshot.as_str()).collect();
    assert_eq!(
        (screenshots, files(&game.path().join("screenshots"))),
        (
            vec!["../screenshots/Boss.png", "../screenshots/Door.png"],
            vec!["Boss.png".to_owned(), "Door.png".to_owned()]
        )
    );
}

#[test]
fn saving_moves_a_presets_own_screenshots_and_savestates_to_its_games_folder() {
    // Arrange: a preset as saved before game folders shared files
    let game = tempfile::tempdir().unwrap();
    let folder = game.path().join("Any%");
    file(&folder.join("boss.png"), "mask");
    file(&folder.join("screenshots/Boss.png"), "boss");
    file(&folder.join("savestates/deku.gzs"), "state");
    file(&folder.join(format!("savestates/{SAVESTATE_NOTE_FILE}")), "note");
    let mut boss = pictured("Boss", "boss.png", "screenshots/Boss.png");
    boss.savestate = "savestates/deku.gzs".into();
    let loaded = preset(vec![boss]);
    save_preset(Some(&folder), &folder, None, &loaded).unwrap();

    // Act
    let saved = save_preset(Some(&folder), &folder, Some(game.path()), &loaded).unwrap();

    // Assert
    assert_eq!(
        (keeps_own_files(&loaded), keeps_own_files(&saved), files(game.path())),
        (
            true,
            false,
            vec![
                "Any%/boss.png".to_owned(),
                format!("Any%/{PRESET_FILE}"),
                format!("savestates/{SAVESTATE_NOTE_FILE}"),
                "savestates/deku.gzs".to_owned(),
                "screenshots/Boss.png".to_owned(),
            ]
        )
    );
}

#[test]
fn a_failed_save_takes_back_what_it_copied_to_the_shared_folders() {
    // Arrange: Boss's screenshot is copied to the shared folder, then Door's mask, a folder,
    // can't be
    let picked = tempfile::tempdir().unwrap();
    let mask = file(&picked.path().join("mask.png"), "mask");
    let boss = file(&picked.path().join("boss.png"), "boss");
    let not_a_file = picked.path().join("folder.png");
    std::fs::create_dir(&not_a_file).unwrap();
    let game = tempfile::tempdir().unwrap();
    file(&game.path().join("screenshots/Other.png"), "another preset's");

    // Act
    let splits = vec![pictured("Boss", &mask, &boss), split("Door", &not_a_file.display().to_string())];
    let error = save_preset(None, &game.path().join("Any%"), Some(game.path()), &preset(splits));

    // Assert
    assert_eq!((error.is_err(), files(game.path())), (true, vec!["screenshots/Other.png".to_owned()]));
}

#[test]
fn a_preset_folder_never_takes_the_name_of_the_shared_folders() {
    // Arrange
    let presets = tempfile::tempdir().unwrap();

    // Act
    let folder = new_preset_folder(presets.path(), "Game", "Screenshots");

    // Assert
    assert_eq!(folder.file_name().unwrap(), "Screenshots_2");
}

/// A games folder in `presets` with a preset using `used.png` and a library in `libraries`
/// using `library.png`, and `unused.png` and `in use.png` that neither uses.
fn shared_files(presets: &Path, libraries: &Path) -> std::path::PathBuf {
    let game = presets.join("Game");
    for name in ["used.png", "library.png", "unused.png", "in use.png"] {
        file(&game.join("screenshots").join(name), name);
    }
    file(&game.join("Any%/boss.png"), "mask");
    let any = preset(vec![pictured("Boss", "boss.png", "../screenshots/used.png")]);
    std::fs::write(game.join("Any%").join(PRESET_FILE), any.to_toml()).unwrap();
    let library = "game = \"Game\"\n\n[[split]]\nname = \"Door\"\nscreenshot = \
                   \"../../presets/Game/screenshots/library.png\"\n";
    file(&libraries.join("Game").join(LIBRARY_FILE), library);
    game
}

#[test]
fn files_nothing_uses_go_from_the_shared_folders() {
    // Arrange
    let root = tempfile::tempdir().unwrap();
    let (presets, libraries) = (root.path().join("presets"), root.path().join("splits"));
    let game = shared_files(&presets, &libraries);

    // Act
    sweep_shared_files(&game, &presets, &libraries, &[game.join("screenshots/in use.png")]);

    // Assert
    assert_eq!(
        files(&game.join("screenshots")),
        vec!["in use.png".to_owned(), "library.png".to_owned(), "used.png".to_owned()]
    );
}

#[test]
fn shared_files_stay_while_a_preset_cant_be_read() {
    // Arrange: one that may use unused.png
    let root = tempfile::tempdir().unwrap();
    let (presets, libraries) = (root.path().join("presets"), root.path().join("splits"));
    let game = shared_files(&presets, &libraries);
    file(&game.join("Broken").join(PRESET_FILE), "not = [toml");

    // Act
    sweep_shared_files(&game, &presets, &libraries, &[]);

    // Assert
    assert_eq!(files(&game.join("screenshots")).len(), 4);
}

#[test]
fn a_shared_file_never_gets_a_name_another_has_in_another_case() {
    // Arrange: the game's presets share boss.png, and this Boss has another picture. Linux would
    // take Boss.png, which is the same file as boss.png on Windows and macOS.
    let picked = tempfile::tempdir().unwrap();
    let mask = file(&picked.path().join("mask.png"), "mask");
    let boss = file(&picked.path().join("boss.png"), "this boss");
    let game = tempfile::tempdir().unwrap();
    file(&game.path().join("screenshots/boss.png"), "another boss");

    // Act
    let saved =
        save_preset(None, &game.path().join("Any%"), Some(game.path()), &preset(vec![pictured("Boss", &mask, &boss)]))
            .unwrap();

    // Assert
    assert_eq!(saved.splits[0].screenshot, "../screenshots/Boss (1).png");
}

#[test]
fn a_preset_folder_with_the_shared_folders_name_is_never_swept() {
    // Arrange: a preset from before the name was kept for the shared screenshots
    let root = tempfile::tempdir().unwrap();
    let (presets, libraries) = (root.path().join("presets"), root.path().join("splits"));
    let game = presets.join("Game");
    file(&game.join("screenshots/boss.png"), "mask");
    std::fs::write(game.join("screenshots").join(PRESET_FILE), preset(vec![split("Boss", "boss.png")]).to_toml())
        .unwrap();
    file(&game.join("screenshots/unused.png"), "nothing uses it");

    // Act
    sweep_shared_files(&game, &presets, &libraries, &[]);

    // Assert
    assert_eq!(files(&game.join("screenshots")).len(), 3);
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn a_shared_folder_spelled_in_another_case_is_used_as_it_is() {
    // Arrange: the shared folder is Screenshots, which is the same folder as screenshots here
    let game = tempfile::tempdir().unwrap();
    file(&game.path().join("Screenshots/Boss.png"), "boss");
    let folder = game.path().join("Any%");
    file(&folder.join("boss.png"), "mask");
    let loaded = preset(vec![pictured("Boss", "boss.png", "../Screenshots/Boss.png")]);

    // Act
    let saved = save_preset(Some(&folder), &folder, Some(game.path()), &loaded).unwrap();

    // Assert: no second copy
    assert_eq!(
        (saved.splits[0].screenshot.as_str(), files(&game.path().join("Screenshots"))),
        ("../Screenshots/Boss.png", vec!["Boss.png".to_owned()])
    );
}

#[test]
fn a_preset_whose_game_changed_takes_its_shared_files_to_the_new_games_folder() {
    // Arrange: a preset of A sharing A's screenshot
    let root = tempfile::tempdir().unwrap();
    let (presets, libraries) = (root.path().join("presets"), root.path().join("splits"));
    let old = presets.join("A/Any%");
    file(&old.join("boss.png"), "mask");
    file(&presets.join("A/screenshots/Boss.png"), "boss");
    let loaded = Preset { game: "A".into(), ..preset(vec![pictured("Boss", "boss.png", "../screenshots/Boss.png")]) };
    std::fs::write(old.join(PRESET_FILE), loaded.to_toml()).unwrap();

    // Act
    let saved =
        save_in_presets(&presets, &libraries, Some(&old), &old, &Preset { game: "B".into(), ..loaded }).unwrap();

    // Assert: A keeps nothing, since nothing else uses its screenshot
    assert_eq!(
        (saved.folder, saved.left_behind.is_none(), files(&presets)),
        (
            presets.join("B").join("Any%"),
            true,
            vec!["B/Any%/Boss.png".to_owned(), format!("B/Any%/{PRESET_FILE}"), "B/screenshots/Boss.png".to_owned()]
        )
    );
}

#[test_case(false => ("../screenshots/Boss.png".to_owned(), "new".to_owned()); "only this preset uses it")]
#[test_case(true => ("../screenshots/Boss (1).png".to_owned(), "old".to_owned()); "another preset uses it")]
fn a_replaced_shared_screenshot_keeps_its_name_unless_another_uses_it(shared: bool) -> (String, String) {
    // Arrange: a preset of the game with the Boss screenshot, and maybe another using it too
    let root = tempfile::tempdir().unwrap();
    let (presets, libraries) = (root.path().join("presets"), root.path().join("splits"));
    let game = presets.join("Game");
    file(&game.join("screenshots/Boss.png"), "old");
    let picked = tempfile::tempdir().unwrap();
    let screenshot = file(&picked.path().join("new.png"), "new");
    let mut folders = vec![game.join("Any%")];
    if shared {
        folders.push(game.join("100%"));
    }
    for folder in &folders {
        file(&folder.join("Boss.png"), "mask");
        let old = preset(vec![pictured("Boss", "Boss.png", "../screenshots/Boss.png")]);
        std::fs::write(folder.join(PRESET_FILE), old.to_toml()).unwrap();
    }
    let any = &folders[0];

    // Act
    let saved =
        save_in_presets(&presets, &libraries, Some(any), any, &preset(vec![pictured("Boss", "Boss.png", &screenshot)]))
            .unwrap();

    // Assert: the old screenshot is gone unless the other preset uses it
    let count = files(&game.join("screenshots")).len();
    assert_eq!(count, if shared { 2 } else { 1 });
    (saved.preset.splits[0].screenshot.clone(), std::fs::read_to_string(game.join("screenshots/Boss.png")).unwrap())
}

#[test]
fn a_failed_save_takes_back_the_shared_folder_it_made() {
    // Arrange: the game's presets don't share screenshots yet. Boss's is copied, then Door's mask,
    // a folder, can't be.
    let picked = tempfile::tempdir().unwrap();
    let mask = file(&picked.path().join("mask.png"), "mask");
    let boss = file(&picked.path().join("boss.png"), "boss");
    let not_a_file = picked.path().join("folder.png");
    std::fs::create_dir(&not_a_file).unwrap();
    let game = tempfile::tempdir().unwrap();

    // Act
    let splits = vec![pictured("Boss", &mask, &boss), split("Door", &not_a_file.display().to_string())];
    let error = save_preset(None, &game.path().join("Any%"), Some(game.path()), &preset(splits));

    // Assert
    assert_eq!((error.is_err(), game.path().join("screenshots").exists()), (true, false));
}
