//! Saving presets: files copied in, names that don't clash, and files the preset stopped using.

use std::path::Path;

use autosplit_core::compare::ComparisonMethod;
use autosplit_core::preset::{
    PRESET_FILE, Preset, PresetError, PresetSplit, SAVESTATE_NOTE_FILE, delete_preset, duplicate_savestates,
    duplicates_among, load_presets, new_preset_folder, same_contents_as, sanitize_file_name, save_preset,
    unique_file_name,
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

#[test]
fn new_folders_dont_reuse_existing_ones() {
    // Arrange
    let presets = tempfile::tempdir().unwrap();
    std::fs::create_dir(presets.path().join("Any%")).unwrap();

    // Act
    let folder = new_preset_folder(presets.path(), "Any%");

    // Assert
    assert_eq!(folder, presets.path().join("Any%_2"));
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
    let saved = save_preset(None, target.path(), &preset(vec![first, second])).unwrap();

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
    let saved = save_preset(None, target.path(), &preset(vec![split("Boss", &mask)])).unwrap();

    // Assert
    assert_eq!(
        (saved.splits[0].mask.as_str(), std::fs::read_to_string(target.path().join("Boss.png")).unwrap()),
        ("Boss (1).png", "someone else's".to_owned())
    );
}

#[test]
fn files_in_the_folder_stay_where_they_are() {
    // Arrange
    let folder = tempfile::tempdir().unwrap();
    file(&folder.path().join("masks/boss.png"), "mask");
    let loaded = preset(vec![split("Boss", "masks/boss.png")]);

    // Act
    let saved = save_preset(Some(folder.path()), folder.path(), &loaded).unwrap();

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
fn a_file_with_the_same_contents_is_found_among_others() {
    // Arrange: one with other contents of the same size, the file itself, then the match
    let folder = tempfile::tempdir().unwrap();
    let picked = folder.path().join("picked.gzs");
    file(&picked, "link's house");
    let (other, same) = (folder.path().join("other.gzs"), folder.path().join("000.gzs"));
    file(&other, "lost woods!!");
    file(&same, "link's house");
    let files = [other.as_path(), picked.as_path(), same.as_path()];

    // Act
    let found = (same_contents_as(&picked, files), same_contents_as(&other, files));

    // Assert
    assert_eq!(found, (Some(same.as_path()), None));
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
    let saved = save_preset(None, target.path(), &preset(vec![split("Boss", &mask)])).unwrap();
    let before = files(target.path());

    // Act
    let again = save_preset(Some(target.path()), target.path(), &saved).unwrap();

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
    let loaded = save_preset(Some(&old), &old, &preset(vec![boss])).unwrap();
    let new = new_preset_folder(presets.path(), "Any%");

    // Act
    let saved = save_preset(Some(&old), &new, &loaded).unwrap();

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
    // Arrange: a mask and savestate the preset used, and files it never had
    let folder = tempfile::tempdir().unwrap();
    file(&folder.path().join("boss.png"), "mask");
    file(&folder.path().join("door.png"), "mask");
    file(&folder.path().join("notes.png"), "not the preset's");
    file(&folder.path().join("savestates/boss.gzs"), "state");
    file(&folder.path().join("savestates/stray.gzs"), "state");
    let mut boss = split("Boss", "boss.png");
    boss.savestate = "savestates/boss.gzs".into();
    save_preset(Some(folder.path()), folder.path(), &preset(vec![boss, split("Door", "door.png")])).unwrap();

    // Act: the boss split is removed
    save_preset(Some(folder.path()), folder.path(), &preset(vec![split("Door", "door.png")])).unwrap();

    // Assert
    assert_eq!(
        files(folder.path()),
        ["door.png", "notes.png", PRESET_FILE, &format!("savestates/{SAVESTATE_NOTE_FILE}"), "savestates/stray.gzs"]
    );
}

#[test_case("./boss.png"; "dot slash")]
#[test_case(".//boss.png"; "doubled slash")]
#[test_case("masks/../boss.png"; "through another folder")]
fn masks_written_another_way_are_still_used(written: &str) {
    // Arrange: a hand-written preset, which saving writes as boss.png
    let folder = tempfile::tempdir().unwrap();
    file(&folder.path().join("boss.png"), "mask");
    std::fs::create_dir(folder.path().join("masks")).unwrap();
    let loaded = preset(vec![split("Boss", written)]);
    std::fs::write(folder.path().join(PRESET_FILE), loaded.to_toml()).unwrap();

    // Act
    let saved = save_preset(Some(folder.path()), folder.path(), &loaded).unwrap();

    // Assert
    assert_eq!((saved.splits[0].mask.as_str(), folder.path().join("boss.png").is_file()), ("boss.png", true));
}

#[test]
fn a_failed_save_leaves_the_folder_as_it_was() {
    // Arrange: the first split's mask would be copied in, the second's is gone
    let picked = tempfile::tempdir().unwrap();
    let mask = file(&picked.path().join("mask.png"), "mask");
    let gone = picked.path().join("gone.png").display().to_string();
    let folder = tempfile::tempdir().unwrap();
    file(&folder.path().join("notes.txt"), "kept");

    // Act
    let error = save_preset(None, folder.path(), &preset(vec![split("Boss", &mask), split("Door", &gone)]));

    // Assert
    assert_eq!((error.is_err(), files(folder.path())), (true, vec!["notes.txt".to_owned()]));
}

#[test]
fn a_failed_save_as_new_leaves_no_folder() {
    // Arrange
    let picked = tempfile::tempdir().unwrap();
    let mask = file(&picked.path().join("mask.png"), "mask");
    let gone = picked.path().join("gone.png").display().to_string();
    let presets = tempfile::tempdir().unwrap();
    let new = new_preset_folder(presets.path(), "Any%");

    // Act
    let error = save_preset(None, &new, &preset(vec![split("Boss", &mask), split("Door", &gone)]));

    // Assert
    assert_eq!((error.is_err(), new.exists()), (true, false));
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
    let saved = save_preset(None, target.path(), &preset(vec![boss, again])).unwrap();

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
        save_preset(None, target.path(), &preset(vec![split("Boss?", &first), split("Boss:", &second)])).unwrap();

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
    let saved = save_preset(None, target.path(), &preset(vec![reset])).unwrap();

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
        mask: file(&picked.path().join("door.png"), "o"),
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
    let saved = save_preset(None, target.path(), &preset(vec![first, second.clone()])).unwrap();

    // Assert: everything but the paths, which became the copies', is as given
    let read = Preset::load(&target.path().join(PRESET_FILE)).unwrap();
    assert_eq!(read, saved);
    let expected = PresetSplit { mask: "Door.png".into(), savestate: "savestates/door.gzs".into(), ..second };
    assert_eq!(read.splits[1], expected);
}

#[test]
fn missing_files_stop_the_save_before_anything_is_written() {
    // Arrange
    let target = tempfile::tempdir().unwrap();
    let missing = target.path().join("gone.png");

    // Act
    let error = save_preset(None, target.path(), &preset(vec![split("Boss", &missing.display().to_string())]));

    // Assert
    assert_eq!(
        (error, files(target.path())),
        (Err(PresetError::Unreadable { path: missing, reason: "the file doesn't exist".into() }), Vec::new())
    );
}

#[test]
fn the_savestate_folder_says_not_to_copy_from_it() {
    // Arrange: a preset without savestates, and one with
    let picked = tempfile::tempdir().unwrap();
    let mask = file(&picked.path().join("mask.png"), "mask");
    let mut boss = split("Boss", &mask);
    boss.savestate = file(&picked.path().join("boss.gzs"), "state");
    let (without, with) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());

    // Act
    save_preset(None, without.path(), &preset(vec![split("Boss", &mask)])).unwrap();
    save_preset(None, with.path(), &preset(vec![boss])).unwrap();

    // Assert
    let note = |folder: &Path| folder.join("savestates").join(SAVESTATE_NOTE_FILE).is_file();
    assert_eq!((note(without.path()), note(with.path())), (false, true));
}

#[test]
fn deleted_presets_are_gone() {
    // Arrange: two presets
    let presets = tempfile::tempdir().unwrap();
    for name in ["Any%", "100%"] {
        let folder = presets.path().join(name);
        file(&folder.join("boss.png"), "mask");
        save_preset(Some(&folder), &folder, &preset(vec![split("Boss", "boss.png")])).unwrap();
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
