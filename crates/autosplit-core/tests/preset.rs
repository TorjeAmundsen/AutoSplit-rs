//! Split image presets and the splits library, as bundled in `presets/` and `splits/`.

use std::path::{Path, PathBuf};

use autosplit_core::fade::FadeNoise;
use autosplit_core::filename::{
    BELOW_FLAG, DUMMY_FLAG, FADE_FLAG, comparison_method_from_filename, delay_time_from_filename,
    fade_time_from_filename, filename_warnings, flags_from_filename, pause_from_filename, threshold_from_filename,
};
use autosplit_core::image::BgraImage;
use autosplit_core::preset::{
    PRESET_FILE, Preset, PresetError, PresetSplit, SPLIT_KEYS, SplitLibrary, game_key, load_libraries, load_presets,
};
use autosplit_core::profile::{ProfileError, UserProfile};
use autosplit_core::split_folder::{ValidationContext, load_split_folder};
use autosplit_core::split_image::SplitDefaults;
use test_case::test_case;

fn bundled(folder: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(folder)
}

#[test]
fn bundled_presets_load_with_all_their_files() {
    // Act
    let (presets, errors) = load_presets(&bundled("presets"));

    // Assert
    let missing: Vec<PathBuf> = presets
        .iter()
        .flat_map(|loaded| {
            let files = loaded.preset.splits.iter().flat_map(|split| [&split.mask, &split.savestate]);
            files.filter(|file| !file.is_empty()).map(|file| loaded.file(file)).collect::<Vec<_>>()
        })
        .filter(|path| !path.is_file())
        .collect();
    // Every folder with a preset file loads
    let folders = std::fs::read_dir(bundled("presets")).unwrap();
    let with_file = folders.filter(|entry| entry.as_ref().unwrap().path().join(PRESET_FILE).is_file()).count();
    assert_eq!((presets.len(), errors, missing), (with_file, vec![], vec![]));
}

#[test]
fn bundled_libraries_load_with_all_their_files() {
    // Act
    let (libraries, errors) = load_libraries(&bundled("splits"));

    // Assert
    let missing: Vec<PathBuf> = libraries
        .iter()
        .flat_map(|loaded| {
            let files = (loaded.library.splits.iter())
                .flat_map(|split| [&split.split.mask, &split.split.savestate, &split.base_image]);
            files.filter(|file| !file.is_empty()).map(|file| loaded.folder.join(file)).collect::<Vec<_>>()
        })
        .filter(|path| !path.is_file())
        .collect();
    assert_eq!((libraries.len(), errors, missing), (1, vec![], vec![]));
}

#[test]
fn bundled_library_files_have_names_every_system_tells_apart() {
    // Arrange
    let (libraries, _) = load_libraries(&bundled("splits"));

    for loaded in libraries {
        let names: Vec<String> = (std::fs::read_dir(&loaded.folder).unwrap())
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();

        // Act
        let lowercase: std::collections::HashSet<String> = names.iter().map(|name| name.to_lowercase()).collect();
        let named: Vec<&String> = (loaded.library.splits.iter())
            .flat_map(|split| [&split.split.mask, &split.split.savestate, &split.base_image])
            .filter(|name| !name.is_empty() && !names.contains(name))
            .collect();

        // Assert: no two files differ only in case, and the library names each as it's written,
        // which systems that tell case apart need
        assert_eq!((lowercase.len(), named), (names.len(), Vec::<&String>::new()), "{}", loaded.folder.display());
    }
}

#[test]
fn presets_are_sorted_by_game_then_name() {
    // Arrange
    let folder = tempfile::tempdir().unwrap();
    for (name, game) in [("b", "Game"), ("A", "Game"), ("c", "")] {
        std::fs::create_dir(folder.path().join(name)).unwrap();
        let text = format!("name = \"{name}\"\ngame = \"{game}\"\n\n[[split]]\nname = \"x\"\nmask = \"x.png\"\n");
        std::fs::write(folder.path().join(name).join("preset.toml"), text).unwrap();
    }

    // Act
    let (presets, _) = load_presets(folder.path());

    // Assert
    let names: Vec<&str> = presets.iter().map(|loaded| loaded.preset.name.as_str()).collect();
    assert_eq!(names, ["c", "A", "b"]);
}

#[test]
fn presets_of_a_game_spelled_differently_are_listed_together() {
    // Arrange: names that would sort between the spellings if they counted as other games
    let folder = tempfile::tempdir().unwrap();
    for (name, game) in [("a", "Zelda  "), ("b", "zelda"), ("c", "Zelda"), ("d", "Mario")] {
        std::fs::create_dir(folder.path().join(name)).unwrap();
        let text = format!("name = \"{name}\"\ngame = \"{game}\"\n\n[[split]]\nname = \"x\"\nmask = \"x.png\"\n");
        std::fs::write(folder.path().join(name).join("preset.toml"), text).unwrap();
    }

    // Act
    let (presets, _) = load_presets(folder.path());

    // Assert
    let names: Vec<&str> = presets.iter().map(|loaded| loaded.preset.name.as_str()).collect();
    assert_eq!(names, ["d", "a", "b", "c"]);
}

#[test_case("The Legend of Zelda: Ocarina of Time", "the legend of zelda:  ocarina of time " => true; "case and spaces")]
#[test_case("Zelda", "Mario" => false; "another game")]
fn games_are_the_same_ignoring_case_and_spaces(game: &str, other: &str) -> bool {
    // Act, with the expected value checked by test_case
    game_key(game) == game_key(other)
}

#[test]
fn a_profile_isnt_a_preset() {
    // Arrange
    let profile = UserProfile::default().to_toml();

    // Act
    let parsed = Preset::parse(&profile, Path::new("settings.toml"));

    // Assert
    assert_eq!(parsed, Err(PresetError::NotAPreset { path: "settings.toml".into() }));
}

#[test]
fn a_preset_isnt_a_profile() {
    // Arrange
    let preset = Preset { name: "Any%".into(), splits: vec![PresetSplit::default()], ..Preset::default() };

    // Act
    let parsed = UserProfile::parse(&preset.to_toml());

    // Assert
    assert_eq!(parsed, Err(ProfileError::SplitImagePreset));
}

#[test]
fn a_split_key_presets_dont_have_is_an_error() {
    // Arrange: a misspelled threshold, which would be left out and then lost on saving
    let text = "name = \"Any%\"\n\n[[split]]\nname = \"Deku\"\nmask = \"deku.png\"\ntreshold = 0.9\n";

    // Act
    let parsed = Preset::parse(text, Path::new("preset.toml"));

    // Assert
    let Err(PresetError::Invalid { reason, .. }) = parsed else { panic!("{parsed:?}") };
    assert!(reason.starts_with("split 1 (Deku) has `treshold`"), "{reason}");
}

#[test]
fn split_keys_are_the_fields_of_a_split() {
    // Arrange: every field set, so each is written
    let split = PresetSplit {
        name: "Deku".into(),
        with_previous: true,
        mask: "deku.png".into(),
        fade: Some("#000000".into()),
        fade_time: Some(1000),
        threshold: Some(0.9),
        pause: Some(1.0),
        delay: Some(100),
        dummy: true,
        below: true,
        savestate: "deku.gzs".into(),
        instructions: "Load it".into(),
        // Only the editor's, never written
        remembered: Default::default(),
    };
    let preset = Preset { name: "Any%".into(), splits: vec![split.clone()], ..Preset::default() };

    // Act
    let written: toml::Table = preset.to_toml().parse().unwrap();

    // Assert
    let table = written["split"].as_array().unwrap()[0].as_table().unwrap();
    let mut keys: Vec<&str> = table.keys().map(String::as_str).collect();
    let mut expected = SPLIT_KEYS.to_vec();
    keys.sort_unstable();
    expected.sort_unstable();
    assert_eq!(keys, expected);
    assert_eq!(Preset::parse(&preset.to_toml(), Path::new("")).unwrap().splits, vec![split]);
}

#[test]
fn presets_read_back_what_they_write() {
    // Arrange
    let (presets, _) = load_presets(&bundled("presets"));

    // Act
    let reread: Vec<Preset> =
        presets.iter().map(|loaded| Preset::parse(&loaded.preset.to_toml(), Path::new("")).unwrap()).collect();

    // Assert
    assert_eq!(reread, presets.into_iter().map(|loaded| loaded.preset).collect::<Vec<_>>());
}

const PROFILE: SplitDefaults = SplitDefaults {
    comparison_method: 1,
    similarity_threshold: 0.9,
    delay_time: 500,
    pause_time: 10.0,
    fps_limit: 60,
    fade_tolerance: 10,
    fade_noise: FadeNoise::NONE,
};

#[test_case(None; "without a profile")]
#[test_case(Some(PROFILE); "with a profile")]
fn bundled_file_names_read_back_as_their_settings(defaults: Option<SplitDefaults>) {
    // Arrange
    let (presets, _) = load_presets(&bundled("presets"));
    let reads_back = |split: &PresetSplit, name: &str| {
        let name = name.to_lowercase();
        let flags = flags_from_filename(&name);
        filename_warnings(&name).is_empty()
            && threshold_from_filename(&name) == split.threshold
            && pause_from_filename(&name) == split.pause
            && delay_time_from_filename(&name) == split.delay.map(i64::from)
            // Fade images always compare by color
            && (defaults.is_none() || split.is_fade() || comparison_method_from_filename(&name) == Some(0))
            && (flags & DUMMY_FLAG != 0) == split.dummy
            && (flags & BELOW_FLAG != 0) == split.below
            && (flags & FADE_FLAG != 0) == split.is_fade()
            && fade_time_from_filename(&name) == split.fade_time.map(i64::from)
    };

    // Act
    let mismatches: Vec<String> = presets
        .iter()
        .flat_map(|loaded| loaded.preset.splits.iter().zip(loaded.preset.file_names(defaults.as_ref())))
        .filter(|(split, name)| !reads_back(split, name))
        .map(|(_, name)| name)
        .collect();

    // Assert
    assert_eq!(mismatches, Vec::<String>::new());
}

#[test]
fn savestates_are_copied_numbered_like_the_split_images() {
    // Arrange
    let split = |name: &str, savestate: &str, instructions: &str| PresetSplit {
        name: name.into(),
        savestate: savestate.into(),
        instructions: instructions.into(),
        ..PresetSplit::default()
    };
    let splits = (0..8).map(|_| split("Filler", "", "")).chain([
        split("reset", "savestates/000 start.gzs", ""),
        split("Boss", "savestates/boss.gzs", "Advance one frame."),
    ]);
    let preset = Preset { name: "Any%".into(), splits: splits.collect(), ..Preset::default() };

    // Act
    let copies = preset.savestate_copies();

    // Assert
    assert_eq!(
        copies,
        [
            ("savestates/000 start.gzs", "08_reset.gzs".to_owned()),
            ("savestates/boss.gzs", "09_Boss_SEE_INSTRUCTIONS.gzs".to_owned())
        ]
    );
}

#[test_case("The Legend of Zelda: Ocarina of Time", "Classic Kakariko (Sworded, Gohma Clip)" => "tlozoot_classic_kakariko_sworded_gohma_clip.toml"; "game and preset")]
#[test_case("Super Mario 64", "16 Star" => "sm6_16_star.toml"; "numbers")]
#[test_case("", "No Wrong Warp" => "no_wrong_warp.toml"; "no game")]
#[test_case("Celeste", "!!!" => "c.toml"; "no words in the name")]
#[test_case("", "" => "settings.toml"; "nothing")]
fn profiles_are_named_for_the_game_and_preset(game: &str, name: &str) -> String {
    // Arrange
    let preset = Preset { name: name.into(), game: game.into(), ..Preset::default() };

    // Act, with the expected value checked by test_case
    preset.profile_file_name()
}

/// A preset of splits named `names`, with `+` before a name for a split compared with the one
/// before it.
fn grouped_preset(names: &[&str]) -> Preset {
    let splits = names.iter().map(|name| PresetSplit {
        name: name.trim_start_matches('+').into(),
        with_previous: name.starts_with('+'),
        ..PresetSplit::default()
    });
    Preset { name: "Any%".into(), splits: splits.collect(), ..Preset::default() }
}

#[test]
fn splits_compared_together_share_a_number() {
    // Arrange
    let preset = grouped_preset(&["start_auto_splitter", "reset", "reset", "Boss", "+Boss Alt", "Door"]);

    // Act
    let names = preset.file_names(None);

    // Assert: the second reset image takes no number, so there are 4
    assert_eq!(
        names,
        ["start_auto_splitter.png", "reset.png", "reset_2.png", "2_Boss.png", "2_Boss Alt.png", "3_Door.png"]
    );
}

#[test_case(&["+Boss", "Door"] => vec!["0_Boss.png", "1_Door.png"]; "first split")]
#[test_case(&["reset", "+Boss"] => vec!["reset.png", "1_Boss.png"]; "after a reset image")]
#[test_case(&["Boss", "+reset"] => vec!["0_Boss.png", "reset.png"]; "reset image")]
fn compared_with_the_split_before_needs_a_split_before(names: &[&str]) -> Vec<String> {
    // Act, with the expected value checked by test_case
    grouped_preset(names).file_names(None)
}

#[test]
fn grouped_presets_make_a_folder_that_loads_grouped() {
    // Arrange
    let preset = grouped_preset(&["start_auto_splitter", "start_auto_splitter", "reset", "reset", "A", "+B", "C"]);
    let folder = tempfile::tempdir().unwrap();
    let png = BgraImage::from_raw(4, 4, [0, 0, 255, 255].repeat(16)).encode_png().unwrap();
    for name in preset.file_names(None) {
        std::fs::write(folder.path().join(name), &png).unwrap();
    }
    let context = ValidationContext {
        split_hotkey_set: true,
        reset_hotkey_set: true,
        pause_hotkey_set: true,
        hotkeys_needed: true,
        group_split_images: true,
    };

    // Act
    let set = load_split_folder(folder.path(), &context).result.unwrap();

    // Assert
    assert_eq!((set.start.len(), set.reset.len(), set.groups), (2, 2, vec![0..2, 2..3]));
}

#[test]
fn savestates_of_several_reset_images_get_their_place() {
    // Arrange
    let mut preset = grouped_preset(&["reset", "reset", "Boss"]);
    for split in &mut preset.splits {
        split.savestate = "savestates/state.gzs".into();
    }

    // Act
    let names: Vec<String> = preset.savestate_copies().into_iter().map(|(_, name)| name).collect();

    // Assert
    assert_eq!(names, ["0_reset.gzs", "0_reset_2.gzs", "1_Boss.gzs"]);
}

#[test_case(0 => 0..1; "a split of its own")]
#[test_case(1 => 1..4; "the first of a group")]
#[test_case(2 => 1..4; "one that joins")]
#[test_case(3 => 1..4; "the last that joins")]
#[test_case(4 => 4..5; "after a group")]
#[test_case(9 => 5..5; "no such split")]
fn groups_are_the_splits_compared_together(index: usize) -> std::ops::Range<usize> {
    // Arrange
    let preset = grouped_preset(&["A", "B", "+C", "+D", "E"]);

    // Act, with the expected value checked by test_case
    preset.group(index)
}

#[test]
fn splits_compared_together_with_the_same_name_are_found() {
    // Arrange: written by hand, their savestate copies would get the same name
    let preset = grouped_preset(&["A", "Boss", "+Door", "+boss", "Boss"]);

    // Act
    let same: Vec<Option<usize>> = (0..5).map(|index| preset.same_name_in_group(index)).collect();

    // Assert: only the joined one, not the next split with the name
    assert_eq!(same, [None, None, None, Some(1), None]);
}

#[test_case("Boss." => Some(0); "a dot at the end")]
#[test_case("Boss .. " => Some(0); "dots and spaces at the end")]
#[test_case("Boss.2" => None; "a dot inside")]
fn names_windows_files_would_share_are_the_same(name: &str) -> Option<usize> {
    // Arrange: an extensionless savestate copy named Boss. would replace the one named Boss
    let preset = grouped_preset(&["Boss", &format!("+{name}")]);

    // Act
    preset.same_name_in_group(1)
}

#[test]
fn library_splits_with_then_follow_the_split_above() {
    // Arrange: a boss's intro text as a dummy, then its flash, and a split of its own
    let text = r#"
        game = "Game"

        [[split]]
        name = "Boss intro"
        mask = "intro.png"
        dummy = true
        [[split]]
        name = "Boss"
        mask = "flash.png"
        then = true
        [[split]]
        name = "Door"
        mask = "door.png"
    "#;

    // Act
    let library = SplitLibrary::parse(text, Path::new("splits.toml")).unwrap();

    // Assert
    let names: Vec<Vec<&str>> =
        (library.ready_made()).map(|steps| steps.iter().map(|step| step.split.name.as_str()).collect()).collect();
    assert_eq!(names, [vec!["Boss intro", "Boss"], vec!["Door"]]);
}

#[test]
fn a_library_cant_start_with_then() {
    // Arrange
    let text = "game = \"Game\"
[[split]]
name = \"Boss\"
mask = \"flash.png\"
then = true
";

    // Act
    let error = SplitLibrary::parse(text, Path::new("splits.toml")).unwrap_err();

    // Assert
    assert!(error.to_string().contains("no split above it to follow"), "{error}");
}

#[test_case("[[split]]
name = \"Boss\"
mask = \"boss.png\"
" => true; "no game")]
#[test_case("Game = \"Game\"
[[split]]
name = \"Boss\"
mask = \"boss.png\"
" => true; "a misspelled game")]
#[test_case("game = \"Game\"
[[split]]
name = \"Boss\"
mask = \"boss.png\"
" => false; "a game")]
fn libraries_need_a_game_and_only_their_keys(text: &str) -> bool {
    // Act, with the expected value checked by test_case: whether it's an error
    SplitLibrary::parse(text, Path::new("splits.toml")).is_err()
}

#[test_case("#a0a0a0" => Some([0xa0, 0xa0, 0xa0]); "lowercase")]
#[test_case("#FF8000" => Some([0xff, 0x80, 0x00]); "uppercase")]
#[test_case(" #000000 " => Some([0, 0, 0]); "spaces around")]
#[test_case("a0a0a0" => None; "without the hash")]
#[test_case("#a0a0a" => None; "too short")]
#[test_case("#g0a0a0" => None; "not hex")]
#[test_case("#+a+a+a" => None; "signs, which number parsing takes")]
#[test_case("#ÆØa0a0" => None; "not ascii")]
fn colors(text: &str) -> Option<[u8; 3]> {
    // Act, with the expected value checked by test_case
    autosplit_core::preset::parse_color(text)
}

#[test]
fn fade_split_without_a_mask_reads_and_names_its_image() {
    // Arrange
    let text =
        "name = \"Any%\"\n\n[[split]]\nname = \"EnterDeku\"\nfade = \"#a0a0a0\"\nfade_time = 1000\nthreshold = 0.995\n";

    // Act
    let preset = Preset::parse(text, Path::new("")).unwrap();

    // Assert: written without the mask it doesn't have
    assert!(!preset.to_toml().contains("mask"));
    let split = &preset.splits[0];
    assert_eq!(
        (split.fade_color(), split.mask.as_str(), preset.file_names(None)),
        (Some([0xa0, 0xa0, 0xa0]), "", vec!["0_EnterDeku_(0.995)_~1000~_{f}.png".to_owned()])
    );
}
