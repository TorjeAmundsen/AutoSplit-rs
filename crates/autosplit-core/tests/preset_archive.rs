//! Sharing presets as .zip files: what's exported, and what importing accepts and refuses.

use std::io::Write;
use std::path::Path;

use autosplit_core::preset::{PRESET_FILE, Preset, PresetError, SAVESTATE_NOTE_FILE, load_presets};
use autosplit_core::preset_archive::{MAX_FILES, export_preset, read_archive};
use test_case::test_case;

const PRESET: &str = r#"name = "Any%"
game = "Game"

[[split]]
name = "reset"
mask = "reset.png"
savestate = "savestates/start.gzs"

[[split]]
name = "Boss"
mask = "Boss.png"
savestate = "savestates/start.gzs"
"#;

fn png() -> Vec<u8> {
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::RgbaImage::new(2, 2).write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    bytes.into_inner()
}

/// A preset folder with `PRESET`'s files, and a file it doesn't use.
fn preset_folder(folder: &Path) {
    std::fs::create_dir_all(folder.join("savestates")).unwrap();
    std::fs::write(folder.join(PRESET_FILE), PRESET).unwrap();
    std::fs::write(folder.join("reset.png"), png()).unwrap();
    std::fs::write(folder.join("Boss.png"), png()).unwrap();
    std::fs::write(folder.join("savestates/start.gzs"), b"state").unwrap();
    std::fs::write(folder.join("WIP.txt"), b"not shared").unwrap();
}

/// A zip at `path` with `entries`.
fn zip_with(path: &Path, entries: &[(&str, &[u8])]) {
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    for (name, contents) in entries {
        zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(contents).unwrap();
    }
    zip.finish().unwrap();
}

/// The names in the zip at `path`, sorted.
fn entries(path: &Path) -> Vec<String> {
    let zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut names: Vec<String> = zip.file_names().map(str::to_owned).collect();
    names.sort();
    names
}

/// The names in `folder`, sorted.
fn names(folder: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder)
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn exports_hold_the_preset_and_the_files_it_uses() {
    // Arrange
    let dir = tempfile::tempdir().unwrap();
    preset_folder(&dir.path().join("Any"));
    let archive = dir.path().join("any.zip");

    // Act
    export_preset(&dir.path().join("Any"), &archive).unwrap();

    // Assert
    assert_eq!(entries(&archive), ["Boss.png", "preset.toml", "reset.png", "savestates/start.gzs"]);
    assert!(!dir.path().join("any.zip.tmp").exists());
}

#[test]
fn exports_refuse_a_missing_file() {
    // Arrange
    let dir = tempfile::tempdir().unwrap();
    preset_folder(&dir.path().join("Any"));
    std::fs::remove_file(dir.path().join("Any/Boss.png")).unwrap();
    let archive = dir.path().join("any.zip");

    // Act
    let result = export_preset(&dir.path().join("Any"), &archive);

    // Assert
    assert!(matches!(result, Err(PresetError::Unreadable { .. })));
    assert!(!archive.exists());
}

#[test]
fn imported_presets_are_the_exported_ones() {
    // Arrange
    let dir = tempfile::tempdir().unwrap();
    preset_folder(&dir.path().join("Any"));
    let archive = dir.path().join("any.zip");
    export_preset(&dir.path().join("Any"), &archive).unwrap();
    let presets = dir.path().join("presets");

    // Act
    let folder = read_archive(&archive).unwrap().install(&presets, None).unwrap();

    // Assert
    let (loaded, errors) = load_presets(&presets);
    assert!(errors.is_empty());
    assert_eq!(loaded[0].preset, Preset::parse(PRESET, Path::new("x")).unwrap());
    assert_eq!(names(&folder), ["Boss.png", "preset.toml", "reset.png", "savestates"]);
    assert_eq!(names(&folder.join("savestates")), [SAVESTATE_NOTE_FILE, "start.gzs"]);
    assert_eq!(std::fs::read(folder.join("savestates/start.gzs")).unwrap(), b"state");
    assert_eq!(names(&presets), ["Any%"]);
}

#[test]
fn bundled_presets_import_as_they_were_exported() {
    // Arrange
    let bundled = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../presets");
    let (presets, _) = load_presets(&bundled);
    let dir = tempfile::tempdir().unwrap();

    for loaded in presets {
        let archive = dir.path().join("preset.zip");

        // Act
        export_preset(&loaded.folder, &archive).unwrap();
        let folder = read_archive(&archive).unwrap().install(&dir.path().join("imported"), None).unwrap();

        // Assert
        let imported = Preset::load(&folder.join(PRESET_FILE)).unwrap();
        assert_eq!(imported, loaded.preset, "{}", loaded.preset.name);
        for split in &imported.splits {
            for path in [&split.mask, &split.savestate].into_iter().filter(|path| !path.is_empty()) {
                let (original, copy) = (std::fs::read(loaded.folder.join(path)), std::fs::read(folder.join(path)));
                assert_eq!(copy.unwrap(), original.unwrap(), "{} {path}", loaded.preset.name);
            }
        }
    }
}

#[test]
fn presets_zipped_in_their_folder_are_found() {
    // Arrange
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("any.zip");
    let png = png();
    zip_with(
        &archive,
        &[
            ("Any/preset.toml", PRESET.as_bytes()),
            ("Any/reset.png", &png),
            ("Any/Boss.png", &png),
            ("Any/savestates/start.gzs", b"state"),
        ],
    );

    // Act
    let result = read_archive(&archive);

    // Assert
    assert_eq!(result.unwrap().preset.name, "Any%");
}

#[test]
fn paths_written_with_backslashes_are_saved_with_slashes() {
    // Arrange
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("any.zip");
    let text = "name = \"Any%\"\n\n[[split]]\nname = \"Boss\"\nmask = 'masks\\Boss.png'\n";
    zip_with(&archive, &[("preset.toml", text.as_bytes()), ("masks/Boss.png", &png())]);
    let presets = dir.path().join("presets");

    // Act
    let folder = read_archive(&archive).unwrap().install(&presets, None).unwrap();

    // Assert
    let (loaded, _) = load_presets(&presets);
    assert_eq!(loaded[0].preset.splits[0].mask, "masks/Boss.png");
    assert!(folder.join("masks/Boss.png").is_file());
}

#[test_case(&[("../evil.png", b"x")]; "an entry outside the folder")]
#[test_case(&[("/evil.png", b"x")]; "an absolute entry")]
#[test_case(&[("C:evil.png", b"x")]; "an entry on a drive")]
#[test_case(&[("readme.txt", b"x")]; "no preset")]
#[test_case(&[("a/preset.toml", b"x"), ("b/preset.toml", b"x")]; "several presets")]
fn archives_that_could_write_elsewhere_or_hold_no_preset_are_refused(extra: &[(&str, &[u8])]) {
    // Arrange
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("any.zip");
    zip_with(&archive, extra);

    // Act
    let result = read_archive(&archive);

    // Assert
    assert!(matches!(result, Err(PresetError::Invalid { .. })), "{result:?}");
}

#[test_case("mask = \"../outside.png\"", true; "a mask outside the folder")]
#[test_case("mask = \"/etc/outside.png\"", true; "an absolute mask")]
#[test_case("mask = \"C:/Users/outside.png\"", true; "a mask on a drive")]
#[test_case("mask = \"missing.png\"", true; "a mask that isn't in it")]
#[test_case("mask = \"text.png\"", true; "a mask that isn't a png")]
#[test_case("treshold = 0.9", true; "an unknown key")]
#[test_case("savestate = \"../outside.gzs\"", true; "a savestate outside the folder")]
#[test_case("mask = \"Boss.png\"", false; "a mask in it")]
fn presets_that_use_files_from_elsewhere_are_refused(line: &str, refused: bool) {
    // Arrange
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("any.zip");
    let text = format!("name = \"Any%\"\n\n[[split]]\nname = \"Boss\"\n{line}\n");
    zip_with(&archive, &[("preset.toml", text.as_bytes()), ("Boss.png", &png()), ("text.png", b"not a png")]);

    // Act
    let result = read_archive(&archive);

    // Assert
    assert_eq!(result.is_err(), refused, "{result:?}");
}

#[test]
fn files_that_arent_zips_are_refused() {
    // Arrange
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("any.zip");
    std::fs::write(&archive, PRESET).unwrap();

    // Act
    let result = read_archive(&archive);

    // Assert
    assert!(matches!(result, Err(PresetError::Invalid { .. })));
}

#[test]
fn a_second_import_gets_its_own_folder() {
    // Arrange
    let dir = tempfile::tempdir().unwrap();
    preset_folder(&dir.path().join("Any"));
    let archive = dir.path().join("any.zip");
    export_preset(&dir.path().join("Any"), &archive).unwrap();
    let presets = dir.path().join("presets");
    let read = read_archive(&archive).unwrap();
    read.install(&presets, None).unwrap();

    // Act
    let folder = read.install(&presets, None).unwrap();

    // Assert
    assert_eq!(folder, presets.join("Any%_2"));
    assert_eq!(names(&presets), ["Any%", "Any%_2"]);
}

#[test]
fn replacing_puts_the_import_in_the_old_folder() {
    // Arrange
    let dir = tempfile::tempdir().unwrap();
    preset_folder(&dir.path().join("Any"));
    let archive = dir.path().join("any.zip");
    export_preset(&dir.path().join("Any"), &archive).unwrap();
    let presets = dir.path().join("presets");
    let old = presets.join("Mine");
    std::fs::create_dir_all(&old).unwrap();
    std::fs::write(old.join(PRESET_FILE), "name = \"Any%\"\n\n[[split]]\nname = \"Old\"\n").unwrap();
    std::fs::write(old.join("old.png"), png()).unwrap();

    // Act
    let folder = read_archive(&archive).unwrap().install(&presets, Some(&old)).unwrap();

    // Assert
    assert_eq!(folder, old);
    assert!(!old.join("old.png").exists());
    assert_eq!(names(&presets), ["Mine"]);
    let (loaded, _) = load_presets(&presets);
    assert_eq!(loaded[0].preset.splits.len(), 2);
}

#[test]
fn one_file_written_two_ways_is_shared_once() {
    // Arrange
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("Any");
    std::fs::create_dir_all(folder.join("masks")).unwrap();
    let text = "name = \"Any%\"\n\n[[split]]\nname = \"A\"\nmask = \"masks/Boss.png\"\n\n\
                [[split]]\nname = \"B\"\nmask = './masks\\Boss.png'\n";
    std::fs::write(folder.join(PRESET_FILE), text).unwrap();
    std::fs::write(folder.join("masks/Boss.png"), png()).unwrap();
    let archive = dir.path().join("any.zip");

    // Act
    export_preset(&folder, &archive).unwrap();
    let imported = read_archive(&archive).unwrap();

    // Assert
    assert_eq!(entries(&archive), ["masks/Boss.png", "preset.toml"]);
    let masks: Vec<&str> = imported.preset.splits.iter().map(|split| split.mask.as_str()).collect();
    assert_eq!(masks, ["masks/Boss.png", "masks/Boss.png"]);
}

#[test]
fn presets_too_big_to_import_arent_exported() {
    // Arrange
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("Any");
    std::fs::create_dir_all(folder.join("savestates")).unwrap();
    let mut text = String::from("name = \"Any%\"\n");
    for n in 0..MAX_FILES {
        text.push_str(&format!("\n[[split]]\nname = \"S{n}\"\nsavestate = \"savestates/{n}.gzs\"\n"));
        std::fs::write(folder.join(format!("savestates/{n}.gzs")), b"state").unwrap();
    }
    std::fs::write(folder.join(PRESET_FILE), text).unwrap();
    let archive = dir.path().join("any.zip");

    // Act
    let result = export_preset(&folder, &archive);

    // Assert
    assert!(matches!(result, Err(PresetError::Unwritable { .. })), "{result:?}");
    assert!(!archive.exists());
}

#[cfg(windows)]
#[test]
fn names_windows_takes_for_one_file_are_refused() {
    // Arrange
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("any.zip");
    let text = "name = \"Any%\"\n\n[[split]]\nname = \"Boss\"\nmask = \"M.png\"\nsavestate = \"m.png\"\n";
    zip_with(&archive, &[("preset.toml", text.as_bytes()), ("M.png", &png()), ("m.png", b"not a png")]);
    let presets = dir.path().join("presets");

    // Act
    let result = read_archive(&archive).unwrap().install(&presets, None);

    // Assert
    assert!(matches!(result, Err(PresetError::Unwritable { .. })), "{result:?}");
    assert!(names(&presets).is_empty());
}

#[test]
fn an_import_still_being_written_is_left_alone() {
    // Arrange
    let dir = tempfile::tempdir().unwrap();
    preset_folder(&dir.path().join("Any"));
    let archive = dir.path().join("any.zip");
    export_preset(&dir.path().join("Any"), &archive).unwrap();
    let presets = dir.path().join("presets");
    std::fs::create_dir_all(presets.join(".importing-1")).unwrap();

    // Act
    let folder = read_archive(&archive).unwrap().install(&presets, None).unwrap();

    // Assert
    assert_eq!(names(&presets), [".importing-1", "Any%"]);
    assert_eq!(folder, presets.join("Any%"));
}
