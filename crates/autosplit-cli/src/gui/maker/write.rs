//! Writing split images into the split image folder and making the list's pictures of them,
//! on threads of their own.

use super::*;
use crate::console::utc_file_stamp;

/// A split image to write: what it's made from, its mask and its file. Only a fade split can be
/// without a mask, for the whole screen.
pub(super) struct Job {
    index: usize,
    /// The split's name, which finds its older images.
    split: String,
    source: Source,
    mask: Option<Arc<Mask>>,
    target: PathBuf,
}

/// What a save does with older images in the split image folder once its split images are
/// written.
pub(super) enum OldImages {
    /// Leaves them where they are.
    Keep,
    /// Saving single splits: their older images under other names, deleted for the splits whose
    /// new image was written.
    Delete(Vec<PathBuf>),
    /// Save All: every image in the folder that isn't the preset's, like ones of splits it no
    /// longer has, moved into a backup folder, but not those of splits whose image failed.
    Backup(Vec<PathBuf>),
    /// Save All: all the images [`OldImages::Backup`] moves, not only older ones of the splits
    /// saved like [`OldImages::Delete`], deleted instead.
    DeleteAll(Vec<PathBuf>),
}

/// What a save did with older images, for its status line.
pub(super) enum Cleared {
    /// How many were deleted.
    Deleted(usize),
    /// The backup folder, and how many were moved into it.
    BackedUp(PathBuf, usize),
}

/// A split image the thread wrote, or couldn't: for split `index`, made from `source`.
pub(super) struct Written {
    index: usize,
    source: Source,
    result: Result<PathBuf, String>,
}

impl Job {
    fn run(&self) -> Result<(), String> {
        let image = match (&self.source, &self.mask) {
            (Source::Fade(color), mask) => fade_image(*color, mask.as_deref()),
            (source, Some(mask)) => apply_mask(source.image()?.view(), mask, &mut ResizeBuffers::default()),
            (_, None) => return Err(format!("{} has no mask.", self.target.display())),
        };
        let png = image.encode_png().map_err(|error| format!("Couldn't encode {}: {error}", self.target.display()))?;
        std::fs::write(&self.target, png).map_err(|error| format!("Couldn't write {}: {error}", self.target.display()))
    }
}

impl Gui {
    /// Writes the split images of `indexes` that have something to make them from, after asking
    /// about files they replace. `every_split` is for Save All, which offers to back up every
    /// image that isn't the preset's, where saving single splits offers to delete their older
    /// ones. Returns false when they can't be saved.
    pub(super) fn save_split_images(self: &Rc<Self>, indexes: Vec<usize>, every_split: bool) -> bool {
        if self.refused_for_problems() {
            return false;
        }
        // Also for a save queued before they were turned off: they get no split image
        let enabled = self.maker_enabled();
        let indexes: Vec<usize> = {
            let maker = self.maker.borrow();
            indexes.into_iter().filter(|&index| !maker.is_off(&enabled, index)).collect()
        };
        if indexes.is_empty() {
            return false;
        }
        if self.queued_behind_saving(&indexes, every_split) {
            return true;
        }
        let Some(folder) = self.split_image_folder().filter(|folder| folder.is_dir()) else {
            self.show_message(Message::error(format!(
                "Pick a split image folder that exists with {} first: the split images are saved there.",
                self.folder_button()
            )));
            return false;
        };
        let names = self.maker_file_names();
        let split_names: Vec<String> = (self.maker.borrow().shown())
            .map(|preset| preset.splits.iter().map(|split| split.name.clone()).collect())
            .unwrap_or_default();
        let (sources, unsaved): (Vec<(usize, Source)>, Vec<usize>) = {
            let maker = self.maker.borrow();
            let sources: Vec<(usize, Source)> =
                indexes.iter().filter_map(|&index| Some((index, maker.source(index)?))).collect();
            // The splits that are on and not saved now, like ones not grabbed yet. All of them,
            // since Save All queued behind another save only queues the ones it can save
            let unsaved =
                maker.on_splits(&enabled).filter(|index| !sources.iter().any(|(at, _)| at == index)).collect();
            (sources, unsaved)
        };
        let mut jobs = Vec::new();
        for (index, source) in sources {
            // A fade split without a mask is the whole screen
            let mask = match source {
                Source::Fade(_) if !self.has_mask(index) => Ok(None),
                _ => self.maker_mask(index).map(Some),
            };
            match mask {
                Ok(mask) => jobs.push(Job {
                    index,
                    split: split_names[index].clone(),
                    source,
                    mask,
                    target: folder.join(&names[index]),
                }),
                Err(error) => {
                    self.show_message(Message::error(error));
                    return false;
                }
            }
        }
        let written: Vec<String> = jobs.iter().map(|job| names[job.index].clone()).collect();
        // As the folder lists them, never made canonical: a link is deleted or moved itself, not
        // the file it points to, which can be outside the folder
        let existing: Vec<PathBuf> = user_files(&folder).unwrap_or_default();
        // Every split's screenshot, also of splits not saved now
        let sources: Vec<PathBuf> = (self.maker.borrow().splits.iter())
            .filter_map(|split| match split.shot.as_ref()? {
                Source::File(path) => Some(canonical(path)),
                Source::Preset(_) | Source::Grabbed(_) | Source::Fade(_) => None,
            })
            .collect();
        let in_folder = |name: &str, path: &Path| writes_over(&folder, name, path);
        let plan = plan_writes(&written, &names, &existing, &sources, in_folder);
        // A save of every split moves all the images that aren't the preset's, whatever their
        // number, so ones of splits removed from the preset go too, but not those of splits
        // left unsaved
        let old = if every_split {
            let kept: Vec<(&str, &str)> =
                unsaved.iter().map(|&index| (names[index].as_str(), split_names[index].as_str())).collect();
            images_to_clear(&old_images(&names, &existing, &sources, in_folder), &kept)
        } else {
            plan.stale
        };
        if plan.overwrites.is_empty() && old.is_empty() {
            self.write_split_images(folder, jobs, OldImages::Keep, every_split);
            return true;
        }
        let mut text = String::new();
        if !plan.overwrites.is_empty() {
            text += &format!("These split images are replaced:\n{}\n\n", plan.overwrites.join("\n"));
        }
        let weak = Rc::downgrade(self);
        if old.is_empty() {
            text += "Replace them?";
            self.show_message(Message::choice(text, ["Replace", "", "Cancel"], 0, move |answer| {
                if let (Some(gui), Answer::Yes) = (weak.upgrade(), answer) {
                    gui.write_split_images(folder, jobs, OldImages::Keep, every_split);
                }
            }));
            return true;
        }
        let old_names = old.iter().filter_map(|path| file_name_text(path)).collect::<Vec<_>>().join("\n");
        // Before the list, which can be long enough to scroll
        let warning = "Keeping them will break your runs: AutoSplit reads every image in the folder as a split, so \
                       they add extra splits or make it split at the wrong time.";
        // Enter backs them up, which can be undone, but never deletes
        if every_split {
            text += &format!(
                "The split image folder has images that aren't the preset's split images, like ones left from \
                 an older version of the preset. {warning} Only keep them if you put them there yourself and know \
                 what they do."
            );
            // The warning doesn't hold for them, so they're named before the list
            let lone: Vec<String> =
                lone_keyword_images(&old, &names).iter().filter_map(|path| file_name_text(path)).collect();
            if !lone.is_empty() {
                let them = if lone.len() == 1 { "it" } else { "them" };
                text += &format!(
                    "\n\n{} may be your own: the preset has no reset or start image like {them}. Backing up or \
                     deleting {them} turns off resetting or starting by itself, so keep the images if you use \
                     {them}.",
                    lone.join(", ")
                );
            }
            text += &format!(
                "\n\nThe old images:\n{old_names}\n\nBack them up into a folder in the split image folder, keep \
                 them, or delete them for good?"
            );
            // Delete away from the default, so a slip from it doesn't delete
            let labels = ["Back Up", "Keep", "Delete", "Cancel"];
            self.show_message(Message::choice4(text, labels, 0, move |answer| {
                let Some(gui) = weak.upgrade() else { return };
                let chosen = match answer {
                    Answer::Yes => OldImages::Backup(old),
                    Answer::No => OldImages::Keep,
                    Answer::Other => OldImages::DeleteAll(old),
                    Answer::Cancel => return,
                };
                gui.write_split_images(folder, jobs, chosen, true);
            }));
            return true;
        }
        text += &format!(
            "The split image folder has older images of the same splits under other names, like from before a \
             threshold changed. {warning} Only keep them if you know what they do.\n\nThe older \
             images:\n{old_names}\n\nDelete them for good?"
        );
        let labels = ["Delete Old Images", "Keep Old Images", "Cancel"];
        self.show_message(Message::choice(text, labels, 1, move |answer| {
            let Some(gui) = weak.upgrade() else { return };
            match answer {
                Answer::Yes => gui.write_split_images(folder, jobs, OldImages::Delete(old), false),
                Answer::No => gui.write_split_images(folder, jobs, OldImages::Keep, false),
                Answer::Cancel | Answer::Other => {}
            }
        }));
        true
    }

    /// While split images are being saved, saves `indexes` after them instead, since two saves
    /// at once could write the same files or move the same screenshots. Returns whether it did.
    fn queued_behind_saving(&self, indexes: &[usize], every_split: bool) -> bool {
        let mut maker = self.maker.borrow_mut();
        if !maker.saving {
            return false;
        }
        let generation = maker.generation;
        let queued = maker.queued.get_or_insert_with(|| (generation, Vec::new(), false));
        // Ones queued for a preset chosen before don't count
        if queued.0 != generation {
            *queued = (generation, Vec::new(), false);
        }
        // Saved together, so once Save All queued, the save is of every split
        queued.2 |= every_split;
        for &index in indexes {
            if !queued.1.contains(&index) {
                queued.1.push(index);
            }
        }
        drop(maker);
        self.windows.maker.set_status("Saves once the split images being saved are.".into());
        true
    }

    /// Writes split images on a thread of their own, then does what `old` says with older images
    /// and moves their screenshots out of the split image folder. `every_split` is for Save All.
    pub(super) fn write_split_images(
        self: &Rc<Self>,
        folder: PathBuf,
        jobs: Vec<Job>,
        old: OldImages,
        every_split: bool,
    ) {
        // Asked about while another save started, which asks again
        let indexes: Vec<usize> = jobs.iter().map(|job| job.index).collect();
        if self.queued_behind_saving(&indexes, every_split) {
            return;
        }
        self.windows.maker.set_status(format!("Saving {}...", plural(jobs.len(), "split image")).into());
        // The preset's split images, which aren't moved when one is picked as a screenshot
        let keep: Vec<PathBuf> = self.maker_file_names().iter().map(|name| folder.join(name)).collect();
        let generation = self.maker.borrow().generation;
        self.maker.borrow_mut().saving = true;
        self.start_work();
        let spawned = std::thread::Builder::new().name("split images".into()).spawn(move || {
            let results: Vec<Written> = (jobs.iter())
                .map(|job| Written {
                    index: job.index,
                    source: job.source.clone(),
                    result: job.run().map(|()| job.target.clone()),
                })
                .collect();
            let file = |written: &Written| match &written.source {
                Source::File(path) => Some(path.clone()),
                // The preset's own file, which stays where it is
                Source::Preset(_) | Source::Grabbed(_) | Source::Fade(_) => None,
            };
            let done: Vec<PathBuf> = results.iter().filter(|written| written.result.is_ok()).filter_map(file).collect();
            let failed: Vec<PathBuf> =
                results.iter().filter(|written| written.result.is_err()).filter_map(file).collect();
            let written: Vec<PathBuf> =
                results.iter().filter_map(|written| written.result.as_ref().ok().cloned()).collect();
            let mut errors: Vec<String> = Vec::new();
            // A split whose image failed keeps its older ones, until it's saved again
            let failed_splits: Vec<(&str, &str)> = (jobs.iter().zip(&results))
                .filter(|(_, written)| written.result.is_err())
                .filter_map(|(job, _)| Some((job.target.file_name()?.to_str()?, job.split.as_str())))
                .collect();
            let mut delete = |images: &[PathBuf]| {
                let (deleted, failed) = delete_images(images);
                errors.extend(failed);
                (deleted > 0).then_some(Cleared::Deleted(deleted))
            };
            let cleared = match old {
                OldImages::Keep => None,
                OldImages::Delete(stale) => delete(&stale_to_delete(&stale, &written)),
                OldImages::DeleteAll(images) => delete(&images_to_clear(&images, &failed_splits)),
                OldImages::Backup(images) => {
                    let images = images_to_clear(&images, &failed_splits);
                    let (backup_folder, moved, error) = move_to_backup(&folder, &images, &utc_file_stamp());
                    errors.extend(error);
                    (moved > 0).then_some(Cleared::BackedUp(backup_folder, moved))
                }
            };
            let (moves, error) = move_to_base(&folder, &screenshots_to_move(&done, &failed, &keep));
            errors.extend(error);
            with_gui(move |gui| gui.split_images_written(generation, results, moves, cleared, errors));
        });
        if let Err(error) = spawned {
            self.maker.borrow_mut().saving = false;
            self.end_work();
            self.show_maker();
            self.show_message(Message::error(format!("Couldn't save the split images: {error}")));
        }
    }

    /// Marks the splits written as saved, unless their source changed meanwhile, points the
    /// splits whose screenshots moved to where they are now, then saves what was queued.
    pub(super) fn split_images_written(
        self: &Rc<Self>,
        generation: u64,
        results: Vec<Written>,
        moves: Vec<(PathBuf, PathBuf)>,
        cleared: Option<Cleared>,
        mut errors: Vec<String>,
    ) {
        self.end_work();
        let mut written = 0;
        let mut maker = self.maker.borrow_mut();
        maker.saving = false;
        // Another preset was chosen meanwhile: its splits aren't the ones saved
        let same_preset = maker.generation == generation;
        let mut saved_rows = vec![false; if same_preset { maker.splits.len() } else { 0 }];
        for Written { index, source, result } in results {
            match result {
                Ok(path) => {
                    written += 1;
                    if let Some(saved) = saved_rows.get_mut(index) {
                        *saved = true;
                    }
                    // A fade split's source is its color, which the split itself doesn't keep
                    let unchanged = maker.source(index).is_some_and(|now| now.same(&source));
                    if let Some(split) = maker.splits.get_mut(index).filter(|_| same_preset && unchanged) {
                        (split.saved, split.saved_from) = (Some(path), Some(source));
                    }
                }
                Err(error) => errors.push(error),
            }
        }
        // Every split made from a moved screenshot reads it from its new place, and its split image
        // is still made from it
        for split in maker.splits.iter_mut().filter(|_| same_preset) {
            for source in [&mut split.shot, &mut split.saved_from] {
                if let Some(Source::File(path)) = source.as_mut()
                    && let Some((_, to)) = moves.iter().find(|(from, _)| from == path)
                {
                    *path = to.clone();
                }
            }
        }
        let generation_now = maker.generation;
        let queued = maker.queued.take().filter(|(queued, _, _)| *queued == generation_now);
        let preview_saved = saved_rows.get(maker.selected).copied().unwrap_or(false);
        drop(maker);
        let mut moved = if moves.is_empty() {
            String::new()
        } else {
            format!(", moved {} to the base folder", plural(moves.len(), "screenshot"))
        };
        // What was done with old images, after the screenshots
        match cleared {
            Some(Cleared::Deleted(count)) => moved += &format!(", deleted {}", plural(count, "old image")),
            Some(Cleared::BackedUp(folder, count)) => {
                let name = file_name_text(&folder).unwrap_or_default();
                moved += &format!(", backed up {} to {name} in the split image folder", plural(count, "old image"));
            }
            None => {}
        }
        self.windows.maker.set_status(format!("Saved {}{moved}.", plural(written, "split image")).into());
        if written > 0 {
            self.windows.maker.invoke_split_images_saved(ModelRc::from(saved_rows.as_slice()), preview_saved);
        }
        self.send(UiCommand::ReloadSplitImages);
        self.show_maker();
        if !errors.is_empty() {
            self.show_message(Message::error(errors.join("\n\n")));
        }
        if let Some((_, indexes, every_split)) = queued {
            self.save_split_images(indexes, every_split);
        }
        // Once nothing's being written, so a file still being written isn't left after deleting
        if !self.maker.borrow().saving {
            let turned_off = self.maker.borrow_mut().turned_off.take();
            if let Some((_, turned_off)) = turned_off.filter(|(generation, _)| *generation == generation_now) {
                self.offer_to_delete_off_images(turned_off);
            }
        }
    }

    /// Makes the list's pictures of the split images of `indexes` on a thread of their own, since
    /// screenshots are read from their files.
    pub(super) fn make_thumbnails(&self, indexes: Vec<usize>) {
        let maker = self.maker.borrow();
        let generation = maker.generation;
        let jobs: Vec<(usize, Source)> =
            indexes.into_iter().filter_map(|index| Some((index, maker.source(index)?))).collect();
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
        let mut outdated = false;
        for (index, source, thumb) in made {
            let current = maker.source(index).is_some_and(|now| now.same(&source));
            match maker.splits.get_mut(index).filter(|_| current) {
                Some(split) => split.thumb = Some(slint::Image::from_rgba8(thumb)),
                None => outdated = true,
            }
        }
        drop(maker);
        // A split that moved or changed meanwhile, whose preset's screenshot nothing else pictures
        if outdated {
            self.picture_preset_screenshots();
        }
        self.show_maker();
    }
}
