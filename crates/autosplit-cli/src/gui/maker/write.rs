//! Writing split images into the split image folder and making the list's pictures of them,
//! on threads of their own.

use super::*;

/// A split image to write: what it's made from, its mask and its file.
pub(super) struct Job {
    index: usize,
    source: Source,
    mask: Arc<Mask>,
    target: PathBuf,
}

/// Moves screenshots that are in the split image folder into its `base` folder, where the
/// split image loader doesn't look. Returns how many it moved.
fn move_to_base(folder: &Path, sources: &[PathBuf]) -> Result<usize, String> {
    let folder = std::fs::canonicalize(folder).map_err(|error| error.to_string())?;
    let mut moved = 0;
    for source in sources {
        let in_folder = source.parent().and_then(|parent| std::fs::canonicalize(parent).ok()) == Some(folder.clone());
        let Some(name) = source.file_name().filter(|_| in_folder && source.is_file()) else { continue };
        let base = folder.join("base");
        std::fs::create_dir_all(&base).map_err(|error| error.to_string())?;
        std::fs::rename(source, base.join(name)).map_err(|error| error.to_string())?;
        moved += 1;
    }
    Ok(moved)
}

impl Job {
    fn run(&self) -> Result<(), String> {
        let shot = self.source.image()?;
        let image = apply_mask(shot.view(), &self.mask, &mut ResizeBuffers::default());
        let png = image.encode_png().map_err(|error| format!("Couldn't encode {}: {error}", self.target.display()))?;
        std::fs::write(&self.target, png).map_err(|error| format!("Couldn't write {}: {error}", self.target.display()))
    }
}

impl Gui {
    /// Writes the split images of `indexes` that have something to make them from, after asking
    /// about files they replace. Returns false when they can't be saved.
    pub(super) fn save_split_images(self: &Rc<Self>, indexes: Vec<usize>) -> bool {
        if self.refused_for_problems() {
            return false;
        }
        let Some(folder) = self.split_image_folder().filter(|folder| folder.is_dir()) else {
            self.show_message(Message::error(
                "Pick a split image folder that exists with Browse... first: the split images are saved there.",
            ));
            return false;
        };
        let names = self.maker_file_names();
        let sources: Vec<(usize, Source)> = {
            let maker = self.maker.borrow();
            indexes.into_iter().filter_map(|index| Some((index, maker.source(index)?.clone()))).collect()
        };
        let mut jobs = Vec::new();
        for (index, source) in sources {
            match self.maker_mask(index) {
                Ok(mask) => jobs.push(Job { index, source, mask, target: folder.join(&names[index]) }),
                Err(error) => {
                    self.show_message(Message::error(error));
                    return false;
                }
            }
        }
        let written: Vec<String> = jobs.iter().map(|job| names[job.index].clone()).collect();
        let existing: Vec<PathBuf> =
            user_files(&folder).unwrap_or_default().iter().map(|path| canonical(path)).collect();
        // Every split's screenshot, also of splits not saved now
        let sources: Vec<PathBuf> = (self.maker.borrow().splits.iter())
            .filter_map(|split| match split.source.as_ref()? {
                Source::File(path) => Some(canonical(path)),
                Source::Grabbed(_) => None,
            })
            .collect();
        let plan = plan_writes(&written, &names, &existing, &sources);
        if plan.overwrites.is_empty() && plan.stale.is_empty() {
            self.write_split_images(folder, jobs, Vec::new());
            return true;
        }
        let mut text = String::new();
        if !plan.overwrites.is_empty() {
            text += &format!("These split images are replaced:\n{}\n\n", plan.overwrites.join("\n"));
        }
        let weak = Rc::downgrade(self);
        if plan.stale.is_empty() {
            text += "Replace them?";
            self.show_message(Message::choice(text, ["Replace", "", "Cancel"], 0, move |answer| {
                if let (Some(gui), Answer::Yes) = (weak.upgrade(), answer) {
                    gui.write_split_images(folder, jobs, Vec::new());
                }
            }));
            return true;
        }
        let stale: Vec<String> = plan.stale.iter().filter_map(|path| file_name_text(path)).collect();
        text += &format!(
            "These older split images are for the same splits under other names, and the split image folder would \
             use them too:\n{}\n\nDelete them, or keep them?",
            stale.join("\n")
        );
        let stale = plan.stale;
        let labels = ["Delete Old Images", "Keep Old Images", "Cancel"];
        // Enter keeps them: deleting is never the default
        self.show_message(Message::choice(text, labels, 1, move |answer| {
            let Some(gui) = weak.upgrade() else { return };
            match answer {
                Answer::Yes => gui.write_split_images(folder, jobs, stale),
                Answer::No => gui.write_split_images(folder, jobs, Vec::new()),
                Answer::Cancel => {}
            }
        }));
        true
    }

    /// Writes split images on a thread of their own, deletes `stale` ones, and moves screenshots
    /// out of the split image folder.
    pub(super) fn write_split_images(&self, folder: PathBuf, jobs: Vec<Job>, stale: Vec<PathBuf>) {
        self.windows.maker.set_status(format!("Saving {}...", plural(jobs.len(), "split image")).into());
        let generation = self.maker.borrow().generation;
        self.start_work();
        let spawned = std::thread::Builder::new().name("split images".into()).spawn(move || {
            let results: Vec<(usize, Result<PathBuf, String>)> =
                jobs.iter().map(|job| (job.index, job.run().map(|()| job.target.clone()))).collect();
            let mut errors: Vec<String> = Vec::new();
            for path in &stale {
                if let Err(error) = std::fs::remove_file(path) {
                    errors.push(format!("Couldn't delete {}: {error}", path.display()));
                }
            }
            let files: Vec<PathBuf> = jobs
                .iter()
                .filter_map(|job| match &job.source {
                    Source::File(path) => Some(path.clone()),
                    Source::Grabbed(_) => None,
                })
                .collect();
            let moved = move_to_base(&folder, &files).unwrap_or_else(|error| {
                errors.push(format!("Couldn't move the screenshots to the base folder: {error}"));
                0
            });
            with_gui(move |gui| gui.split_images_written(generation, results, moved, errors));
        });
        if let Err(error) = spawned {
            self.end_work();
            self.show_message(Message::error(format!("Couldn't save the split images: {error}")));
        }
    }

    pub(super) fn split_images_written(
        &self,
        generation: u64,
        results: Vec<(usize, Result<PathBuf, String>)>,
        moved: usize,
        mut errors: Vec<String>,
    ) {
        self.end_work();
        let mut written = 0;
        let mut maker = self.maker.borrow_mut();
        // Another preset was chosen meanwhile: its splits aren't the ones saved
        let same_preset = maker.generation == generation;
        for (index, result) in results {
            match result {
                Ok(_) if !same_preset => written += 1,
                Ok(path) => {
                    written += 1;
                    let Some(split) = maker.splits.get_mut(index) else { continue };
                    split.saved = Some(path);
                    // A screenshot moved to the base folder is read from there
                    if let Some(Source::File(source)) = split.source.as_mut()
                        && let (Some(parent), Some(name)) = (source.parent(), source.file_name())
                        && parent.join("base").join(name).is_file()
                    {
                        *source = parent.join("base").join(name);
                    }
                }
                Err(error) => errors.push(error),
            }
        }
        drop(maker);
        let moved = if moved > 0 {
            format!(", moved {} to the base folder", plural(moved, "screenshot"))
        } else {
            String::new()
        };
        self.windows.maker.set_status(format!("Saved {}{moved}.", plural(written, "split image")).into());
        if written > 0 {
            self.windows.maker.invoke_split_images_saved();
        }
        self.send(UiCommand::ReloadSplitImages);
        self.show_maker();
        if !errors.is_empty() {
            self.show_message(Message::error(errors.join("\n\n")));
        }
    }

    /// Makes the list's pictures of the split images of `indexes` on a thread of their own, since
    /// screenshots are read from their files.
    pub(super) fn make_thumbnails(&self, indexes: Vec<usize>) {
        let maker = self.maker.borrow();
        let generation = maker.generation;
        let jobs: Vec<(usize, Source)> =
            indexes.into_iter().filter_map(|index| Some((index, maker.source(index)?.clone()))).collect();
        drop(maker);
        self.start_work();
        let spawned = std::thread::Builder::new().name("thumbnails".into()).spawn(move || {
            let made: Vec<(usize, Source, Pixels)> = jobs
                .into_iter()
                .filter_map(|(index, source)| {
                    let thumb = thumbnail(source.image().ok()?.view());
                    Some((index, source, thumb))
                })
                .collect();
            with_gui(move |gui| gui.thumbnails_made(generation, made));
        });
        if let Err(error) = spawned {
            self.end_work();
            self.add_log_line(format!("error: Couldn't make the split images' pictures: {error}"));
        }
    }

    pub(super) fn start_work(&self) {
        self.maker.borrow_mut().working += 1;
        self.windows.maker.set_working(true);
    }

    /// Done with work from `start_work`. The caller shows the window again.
    pub(super) fn end_work(&self) {
        let mut maker = self.maker.borrow_mut();
        maker.working = maker.working.saturating_sub(1);
    }

    /// Shows the pictures that are still of their split's source.
    pub(super) fn thumbnails_made(&self, generation: u64, made: Vec<(usize, Source, Pixels)>) {
        self.end_work();
        let mut maker = self.maker.borrow_mut();
        if maker.generation != generation {
            drop(maker);
            return self.show_maker();
        }
        for (index, source, thumb) in made {
            if let Some(split) = maker.splits.get_mut(index)
                && split.source.as_ref().is_some_and(|now| now.same(&source))
            {
                split.thumb = Some(slint::Image::from_rgba8(thumb));
            }
        }
        drop(maker);
        self.show_maker();
    }
}
