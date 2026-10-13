//! Split image presets and the splits library, as bundled in `presets/` and `splits/`.

use std::path::{Path, PathBuf};

use autosplit_core::filename::{
    BELOW_FLAG, DUMMY_FLAG, comparison_method_from_filename, delay_time_from_filename, filename_warnings,
    flags_from_filename, pause_from_filename, threshold_from_filename,
};
use autosplit_core::preset::{PRESET_FILE, Preset, PresetError, PresetSplit, load_libraries, load_presets};
use autosplit_core::profile::{ProfileError, UserProfile};
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
            let files = loaded.library.splits.iter().flat_map(|split| [&split.split.mask, &split.base_image]);
            files.map(|file| loaded.folder.join(file)).collect::<Vec<_>>()
        })
        .filter(|path| !path.is_file())
        .collect();
    assert_eq!((libraries.len(), errors, missing), (1, vec![], vec![]));
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
fn presets_read_back_what_they_write() {
    // Arrange
    let (presets, _) = load_presets(&bundled("presets"));

    // Act
    let reread: Vec<Preset> =
        presets.iter().map(|loaded| Preset::parse(&loaded.preset.to_toml(), Path::new("")).unwrap()).collect();

    // Assert
    assert_eq!(reread, presets.into_iter().map(|loaded| loaded.preset).collect::<Vec<_>>());
}

const PROFILE: SplitDefaults =
    SplitDefaults { comparison_method: 1, similarity_threshold: 0.9, delay_time: 500, pause_time: 10.0, fps_limit: 60 };

#[test_case(None; "without a profile")]
#[test_case(Some(PROFILE); "with a profile")]
fn bundled_file_names_read_back_as_their_settings(defaults: Option<SplitDefaults>) {
    // Arrange
    let (presets, _) = load_presets(&bundled("presets"));
    let reads_back = |split: &PresetSplit, name: &str| {
        let name = name.to_lowercase();
        let default_delay = defaults.map_or(0, |defaults| defaults.delay_time);
        let flags = flags_from_filename(&name);
        filename_warnings(&name).is_empty()
            && threshold_from_filename(&name) == Some(split.threshold())
            && pause_from_filename(&name) == split.pause
            && delay_time_from_filename(&name).unwrap_or(default_delay) == i64::from(split.delay)
            && (defaults.is_none() || comparison_method_from_filename(&name) == Some(0))
            && (flags & DUMMY_FLAG != 0) == split.dummy
            && (flags & BELOW_FLAG != 0) == split.below
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
