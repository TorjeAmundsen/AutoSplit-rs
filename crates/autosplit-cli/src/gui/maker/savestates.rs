//! Copying a preset's savestates for the practice tool, and their instructions.

use super::*;

/// The temp folder's folder the savestates are copied to for the clipboard.
const SAVESTATES_FOLDER: &str = "AutoSplit savestates";

impl Gui {
    /// Copies the preset's savestates to their folder in the temp folder, named in split order,
    /// and puts them on the clipboard, or opens the folder when the clipboard can't take files.
    /// The folder is emptied first and kept after closing, so they can still be pasted then.
    pub(super) fn copy_savestates(&self) {
        if self.refused_for_problems() {
            return;
        }
        let maker = self.maker.borrow();
        let Some(loaded) = maker.loaded() else { return };
        let copies: Vec<(PathBuf, String)> =
            loaded.preset.savestate_copies().into_iter().map(|(path, name)| (loaded.file(path), name)).collect();
        drop(maker);
        let folder = crate::platform::temp_folder().join(SAVESTATES_FOLDER);
        let mut copied = Vec::new();
        let mut missing = Vec::new();
        let emptied = match std::fs::remove_dir_all(&folder) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
            _ => std::fs::create_dir_all(&folder),
        };
        if let Err(error) = emptied {
            return self.show_message(Message::error(format!("Couldn't empty {}: {error}", folder.display())));
        }
        for (source, name) in copies {
            let target = folder.join(name);
            match std::fs::copy(&source, &target) {
                Ok(_) => copied.push(target),
                Err(_) => missing.push(source.display().to_string()),
            }
        }
        if copied.is_empty() {
            return self.show_message(Message::error("None of the preset's savestate files were found."));
        }
        let status = match self.put_files_on_clipboard(&copied) {
            true => format!("Copied {} to the clipboard.", plural(copied.len(), "savestate")),
            false => {
                self.open(&folder.display().to_string());
                let copied = plural(copied.len(), "savestate");
                format!("The clipboard can't take files here, so the folder with the {copied} opened.")
            }
        };
        self.windows.maker.set_status(status.into());
        if !missing.is_empty() {
            self.show_message(Message::warning(format!("These savestates weren't found:\n\n{}", missing.join("\n"))));
        }
    }

    /// Puts `files` on the clipboard for pasting into a file manager. Returns whether it could.
    fn put_files_on_clipboard(&self, files: &[PathBuf]) -> bool {
        // Every format Linux file managers paste, where arboard offers only one. Through X11,
        // or XWayland, which the Wayland desktops share their clipboard with.
        #[cfg(target_os = "linux")]
        if std::env::var_os("DISPLAY").is_some() && crate::file_clipboard::copy_files(files).is_ok() {
            return true;
        }
        let mut clipboard = self.clipboard.borrow_mut();
        if clipboard.is_none() {
            *clipboard = arboard::Clipboard::new().ok();
        }
        // On Windows, setting a file list adds it to what's on the clipboard, and a program pasting
        // can take the text that was there before instead
        clipboard
            .as_mut()
            .is_some_and(|clipboard| clipboard.clear().and_then(|()| clipboard.set().file_list(files)).is_ok())
    }

    /// The instructions of the preset's savestates, in a window of their own.
    pub(super) fn show_instructions(&self) {
        if self.fill_instructions() {
            self.show_window(self.windows.instructions.window());
        }
    }

    /// Keeps an open Savestate Instructions window on the chosen preset, as it's chosen, saved or
    /// deleted, and closes it when that one has none.
    pub(super) fn refresh_instructions(&self) {
        let window = self.windows.instructions.window();
        if window.is_visible() && !self.fill_instructions() {
            let _ = window.hide();
        }
    }

    /// Puts the chosen preset's savestate instructions in their window. Returns whether it has
    /// any.
    fn fill_instructions(&self) -> bool {
        let maker = self.maker.borrow();
        let Some(loaded) = maker.loaded() else { return false };
        let cards: Vec<InstructionCard> = (instruction_cards(&loaded.preset).into_iter())
            .map(|card| InstructionCard {
                split: card.split as i32,
                heading: card.heading.into(),
                savestate: card.file.into(),
                text: card.instructions.into(),
            })
            .collect();
        let title = format!("Savestate Instructions - {}", loaded.preset.name);
        // While editing, the selected split is the draft's, which these cards aren't about
        let selected = if maker.edit.is_some() { -1 } else { maker.selected as i32 };
        drop(maker);
        let any = !cards.is_empty();
        let window = &self.windows.instructions;
        // Only when they changed, so selecting another split moves the highlight instead of
        // making the cards again
        if !slint::Model::iter(&window.get_cards()).eq(cards.iter().cloned()) {
            window.set_cards(ModelRc::new(VecModel::from(cards)));
        }
        window.set_selected_split(selected);
        window.set_window_title(title.into());
        any
    }

    /// A card in Savestate Instructions was clicked: its split is selected in Make Split Images,
    /// unless a preset is being edited, which the cards aren't about.
    pub(super) fn instruction_card_clicked(&self, split: usize) {
        if self.blocked() || self.editing() {
            return;
        }
        let count = self.maker.borrow().loaded().map_or(0, |loaded| loaded.preset.splits.len());
        if split < count {
            self.choose_split(split);
        }
    }
}
