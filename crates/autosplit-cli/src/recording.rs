//! Recording screenshots: the capture region at the comparison frame rate, every frame saved as
//! a PNG in a folder of its own, for picking the right frame of a game without frame advance.
//!
//! Frames wait in a queue of about a second's worth while several threads encode and write them,
//! since encoding a large frame takes longer than a frame lasts. A frame that finds the queue full
//! is dropped and counted, so memory stays bounded however slow the disk is.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use autosplit_core::image::BgraImage;

/// How long a recording runs before it stops by itself, so one left running doesn't fill the
/// disk.
pub const MAX_LENGTH: Duration = Duration::from_secs(10);
/// The most encoding threads, which a 1080p frame at 60 fps needs a few of.
const MAX_WORKERS: usize = 8;

/// How a recording went, once every frame is written.
#[derive(Debug, PartialEq, Eq)]
pub struct Summary {
    pub folder: PathBuf,
    pub written: usize,
    /// Frames that found the queue full.
    pub dropped: usize,
    /// The first frame that couldn't be written, and why.
    pub error: Option<String>,
}

impl Summary {
    /// The line for the log and the window.
    pub fn message(&self) -> String {
        let name = self.folder.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let frames =
            if self.written == 1 { "1 screenshot".to_owned() } else { format!("{} screenshots", self.written) };
        let mut text = format!("Recorded {frames} into {name}.");
        if self.dropped > 0 {
            text += &format!(" {} frames were skipped, since saving fell behind.", self.dropped);
        }
        if let Some(error) = &self.error {
            text += &format!(" Some couldn't be written: {error}");
        }
        text
    }
}

/// A recording going on. Dropping it, or [`Recording::stop`], ends it: the frames queued are
/// still written, and then `done` gets the [`Summary`].
pub struct Recording {
    folder: PathBuf,
    started: Instant,
    interval: Duration,
    next_due: Instant,
    last_sequence: Option<u64>,
    next_number: usize,
    dropped: Arc<AtomicUsize>,
    queue: Option<SyncSender<(usize, BgraImage)>>,
}

impl Recording {
    /// Starts a recording into a new `recording_NNN` folder in `directory`, saving a frame every
    /// `1 / fps` seconds.
    pub fn start(
        directory: &Path,
        fps: i64,
        now: Instant,
        done: impl FnOnce(Summary) + Send + 'static,
    ) -> Result<Self, String> {
        let folder = claim_folder(directory).map_err(|error| {
            format!("Record Screenshots: couldn't create a folder in {}: {error}", directory.display())
        })?;
        let fps = fps.max(1) as usize;
        let interval = Duration::from_secs_f64(1.0 / fps as f64);
        // About a second of frames, so a slow moment doesn't drop any, but no more
        let (queue, frames) = std::sync::mpsc::sync_channel(fps.clamp(1, 240));
        let dropped = Arc::new(AtomicUsize::new(0));
        let workers =
            std::thread::available_parallelism().map_or(2, |count| count.get().saturating_sub(1)).clamp(1, MAX_WORKERS);
        spawn_writers(folder.clone(), frames, workers, dropped.clone(), done)
            .map_err(|error| format!("Record Screenshots: couldn't start writing frames: {error}"))?;
        Ok(Self {
            folder,
            started: now,
            interval,
            next_due: now,
            last_sequence: None,
            next_number: 1,
            dropped,
            queue: Some(queue),
        })
    }

    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// Whether it has run for [`MAX_LENGTH`].
    pub fn expired(&self, now: Instant) -> bool {
        now.duration_since(self.started) >= MAX_LENGTH
    }

    /// When it next wants a frame, or stops.
    pub fn next_wakeup(&self) -> Instant {
        self.next_due.min(self.started + MAX_LENGTH)
    }

    /// Saves the frame `sequence` when one is due and it's a new frame. `image` makes the capture
    /// region's picture of it, only then.
    pub fn offer(&mut self, now: Instant, sequence: u64, image: impl FnOnce() -> Option<BgraImage>) {
        if now < self.next_due || self.last_sequence == Some(sequence) {
            return;
        }
        // On a grid from the start, so frames stay evenly spaced when the loop wakes late
        while self.next_due <= now {
            self.next_due += self.interval;
        }
        self.last_sequence = Some(sequence);
        let (Some(queue), Some(image)) = (&self.queue, image()) else { return };
        match queue.try_send((self.next_number, image)) {
            Ok(()) => self.next_number += 1,
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Ends the recording. The frames queued are still written.
    pub fn stop(mut self) {
        self.queue = None;
    }
}

/// A new `recording_NNN` folder in `directory`, the first number free, made here so two
/// recordings can't take the same one.
pub fn claim_folder(directory: &Path) -> std::io::Result<PathBuf> {
    for number in 1.. {
        let folder = directory.join(format!("recording_{number:03}"));
        match std::fs::create_dir(&folder) {
            Ok(()) => return Ok(folder),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    unreachable!("some number is free")
}

/// Starts `workers` threads that encode and write the frames of `frames` into `folder`, and a
/// thread that waits for them and then gives `done` the summary.
fn spawn_writers(
    folder: PathBuf,
    frames: Receiver<(usize, BgraImage)>,
    workers: usize,
    dropped: Arc<AtomicUsize>,
    done: impl FnOnce(Summary) + Send + 'static,
) -> std::io::Result<()> {
    let frames = Arc::new(Mutex::new(frames));
    let written = Arc::new(AtomicUsize::new(0));
    let error: Arc<Mutex<Option<String>>> = Arc::default();
    let mut threads = Vec::new();
    for index in 0..workers {
        let (frames, written, error, folder) = (frames.clone(), written.clone(), error.clone(), folder.clone());
        threads.push(std::thread::Builder::new().name(format!("recording {index}")).spawn(move || {
            loop {
                // Only while taking the next frame, so the others encode meanwhile
                let next = frames.lock().map(|frames| frames.recv());
                let Ok(Ok((number, image))) = next else { return };
                let path = folder.join(format!("{number:04}.png"));
                let saved = image
                    .encode_png()
                    .map_err(|error| error.to_string())
                    .and_then(|png| std::fs::write(&path, png).map_err(|error| format!("{}: {error}", path.display())));
                match saved {
                    Ok(()) => {
                        written.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(reason) => {
                        if let Ok(mut error) = error.lock() {
                            error.get_or_insert(reason);
                        }
                    }
                }
            }
        })?);
    }
    std::thread::Builder::new().name("recording".into()).spawn(move || {
        for thread in threads {
            let _ = thread.join();
        }
        let error = error.lock().ok().and_then(|mut error| error.take());
        let (written, dropped) = (written.load(Ordering::Relaxed), dropped.load(Ordering::Relaxed));
        done(Summary { folder, written, dropped, error });
    })?;
    Ok(())
}
