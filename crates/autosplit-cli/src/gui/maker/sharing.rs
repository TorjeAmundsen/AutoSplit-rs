//! Sharing presets: exporting one as a .zip file, and importing one someone sent.

use autosplit_core::preset::game_key;
use autosplit_core::preset_archive::{PresetArchive, export_preset, read_archive};

use super::super::preset_form::{known_game, saved_preset_name};
use super::*;

impl Gui {
    pub(super) fn connect_sharing(&self) {
        let weak = self.me.clone();
        let maker = &self.windows.maker;
        on!(weak, maker, on_import_preset, () gui => gui.pick_import());
        on!(weak, maker, on_export_preset, () gui => gui.pick_export());
    }

    /// Why the presets can't change now, or `None` when they can: while one is edited, or split
    /// images are written.
    fn sharing_waits(&self) -> Option<&'static str> {
        let maker = self.maker.borrow();
        if maker.edit.is_some() {
            Some("Save or cancel the preset being edited first.")
        } else if maker.working > 0 {
            Some("Wait until the split images are written first.")
        } else {
            None
        }
    }

    /// After a file dialog, which doesn't block the window everywhere: whether the presets can
    /// still change, and if not, says why.
    fn still_free(&self, what: &str) -> bool {
        let Some(reason) = self.sharing_waits() else { return true };
        self.show_message(Message::warning(format!("Couldn't {what} the preset. {reason}")));
        false
    }

    /// Says `text` in the bottom line, or in a message while the line shows the preset's
    /// problem instead.
    fn tell_shared(&self, text: String) {
        let window = &self.windows.maker;
        let problem = window.get_preset_problem();
        if problem.is_empty() {
            window.set_status(text.into());
        } else {
            self.show_message(Message::warning(format!("{text} It needs fixing before it's used: {problem}")));
        }
    }

    /// Asks where to save the shown preset as a .zip file, then saves it there.
    fn pick_export(self: &Rc<Self>) {
        if self.sharing_waits().is_some() {
            return;
        }
        let maker = self.maker.borrow();
        let Some(loaded) = maker.loaded() else { return };
        let (folder, name) = (loaded.folder.clone(), loaded.preset.name.clone());
        let file = loaded.preset.profile_file_name().replace(".toml", ".zip");
        drop(maker);
        let dialog = self
            .file_dialog_over("Export Preset", self.windows.maker.window())
            .set_file_name(file)
            .add_filter("Zip archive", &["zip"])
            .save_file();
        self.when_picked(dialog, move |gui, path| {
            if !gui.still_free("export") {
                return;
            }
            if let Err(error) = export_preset(&folder, &path) {
                return gui.show_message(Message::error(format!("Couldn't export the preset. {error}")));
            }
            let file = file_name_text(&path).unwrap_or_default();
            gui.tell_shared(format!("Exported the preset {name} as {file}."));
        });
    }

    /// Asks for a preset's .zip file, then imports it.
    fn pick_import(self: &Rc<Self>) {
        if self.sharing_waits().is_some() {
            return;
        }
        let dialog = self
            .file_dialog_over("Import Preset", self.windows.maker.window())
            .add_filter("Zip archive", &["zip"])
            .pick_file();
        self.when_picked(dialog, |gui, path| {
            if gui.still_free("import") {
                gui.import_preset(&path);
            }
        });
    }

    /// Imports the preset in the archive `path`. When its game already has a preset with its
    /// name, asks whether to replace that one or keep both.
    fn import_preset(self: &Rc<Self>, path: &Path) {
        let mut archive = match read_archive(path) {
            Ok(archive) => archive,
            Err(error) => return self.show_message(Message::error(format!("Couldn't import the preset. {error}"))),
        };
        let maker = self.maker.borrow();
        // A game typed like one that has presets is that game, as when saving
        if let Some(game) = known_game(&archive.preset.game, &maker.presets.games()) {
            archive.preset.game = game.to_owned();
        }
        let preset = &archive.preset;
        let same = (maker.presets.iter())
            .find(|loaded| {
                game_key(&loaded.preset.game) == game_key(&preset.game)
                    && game_key(&loaded.preset.name) == game_key(&preset.name)
            })
            .map(|loaded| loaded.folder.clone());
        let taken: Vec<String> = maker.presets.taken_names(&preset.game, None).into_iter().map(str::to_owned).collect();
        drop(maker);
        let Some(existing) = same else { return self.install_import(archive, None) };
        let (game, name) = (&archive.preset.game, &archive.preset.name);
        let game = if game.is_empty() { "The presets without a game" } else { game };
        let text = format!(
            "{game} already has a preset named {name}. Replace it with the imported one, or keep both?\n\n\
             Replacing deletes the old preset's folder with its masks and savestates, which can't be undone."
        );
        let weak = Rc::downgrade(self);
        self.show_message(Message::choice(text, ["Replace", "Keep Both", "Cancel"], 1, move |answer| {
            let Some(gui) = weak.upgrade() else { return };
            match answer {
                Answer::Yes => gui.install_import(archive, Some(existing)),
                Answer::No => {
                    let taken: Vec<&str> = taken.iter().map(String::as_str).collect();
                    let name = saved_preset_name(&archive.preset.name, &taken, true);
                    archive.preset.name = name.expect("a new preset always gets a name");
                    gui.install_import(archive, None);
                }
                Answer::Cancel => {}
            }
        }));
    }

    /// Puts the imported preset in the presets folder, in place of `replace` if there's one, and
    /// shows it.
    fn install_import(&self, archive: PresetArchive, replace: Option<PathBuf>) {
        let folder = match archive.install(&presets_folder(), replace.as_deref()) {
            Ok(folder) => folder,
            Err(error) => return self.show_message(Message::error(format!("Couldn't import the preset. {error}"))),
        };
        let errors = self.read_presets();
        let mut maker = self.maker.borrow_mut();
        // A replaced preset's masks changed under the same names
        maker.masks.clear();
        maker.live_mask = None;
        let index = maker.presets.find(&folder).unwrap_or(0);
        drop(maker);
        // Another preset than the one shown, so it starts without the shown one's grabs
        self.choose_preset(index);
        self.show_preset_errors(errors);
        self.tell_shared(format!("Imported the preset {}.", archive.preset.name));
    }
}
