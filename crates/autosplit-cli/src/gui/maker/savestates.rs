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
        let folder = std::env::temp_dir().join(SAVESTATES_FOLDER);
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
        let mut clipboard = self.clipboard.borrow_mut();
        if clipboard.is_none() {
            *clipboard = arboard::Clipboard::new().ok();
        }
        // On Windows, setting a file list adds it to what's on the clipboard, and a program pasting
        // can take the text that was there before instead
        let set =
            clipboard.as_mut().map(|clipboard| clipboard.clear().and_then(|()| clipboard.set().file_list(&copied)));
        drop(clipboard);
        let status = match set {
            Some(Ok(())) => format!("Copied {} savestates to the clipboard.", copied.len()),
            _ => {
                self.open(&folder.display().to_string());
                format!(
                    "The clipboard can't take files here, so the {} savestates are in the folder that opened.",
                    copied.len()
                )
            }
        };
        self.windows.maker.set_status(status.into());
        if !missing.is_empty() {
            self.show_message(Message::warning(format!("These savestates weren't found:\n\n{}", missing.join("\n"))));
        }
    }

    /// The instructions of the preset's savestates, in a window of their own.
    pub(super) fn show_instructions(&self) {
        let maker = self.maker.borrow();
        let Some(loaded) = maker.loaded() else { return };
        let width = loaded.preset.splits.len().to_string().len();
        let cards: Vec<InstructionCard> = loaded
            .preset
            .splits
            .iter()
            .enumerate()
            .filter(|(_, split)| !split.savestate.is_empty() && !split.instructions.is_empty())
            .map(|(index, split)| InstructionCard {
                heading: format!("{index:0width$}. {}", split.name).into(),
                savestate: split.savestate.as_str().into(),
                text: split.instructions.as_str().into(),
            })
            .collect();
        let title = format!("Savestate Instructions - {}", loaded.preset.name);
        drop(maker);
        let window = &self.windows.instructions;
        window.set_cards(ModelRc::new(VecModel::from(cards)));
        window.set_window_title(title.into());
        self.show_window(window.window());
    }
}
