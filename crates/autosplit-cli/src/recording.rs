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
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use autosplit_core::image::BgraImage;

/// How long a recording runs before it stops by itself, so one left running doesn't fill the
/// disk.
pub const MAX_LENGTH: Duration = Duration::from_secs(10);
/// The most encoding threads, which a 1080p frame at 60 fps needs a few of.
const MAX_WORKERS: usize = 8;
/// The frame rates a recording takes, like the FPS limit in Settings, whatever a profile file says.
const FPS: std::ops::RangeInclusive<i64> = 1..=240;
/// How soon to look again when a frame is due but the capture hasn't sent a new one yet.
const NEW_FRAME_POLL: Duration = Duration::from_millis(1);

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
        if let (0, Some(error)) = (self.written, &self.error) {
            return format!("Couldn't write the screenshots into {name}: {error}");
        }
        let frames =
            if self.written == 1 { "1 screenshot".to_owned() } else { format!("{} screenshots", self.written) };
        let mut text = format!("Recorded {frames} into {name}.");
        match self.dropped {
            0 => {}
            1 => text += " 1 frame was skipped, since saving fell behind.",
            dropped => text += &format!(" {dropped} frames were skipped, since saving fell behind."),
        }
        if let Some(error) = &self.error {
            text += &format!(" Some couldn't be written: {error}");
        }
        text
    }
}

/// A recording going on. [`Recording::stop`] ends it: the frames queued are still written, and
/// then `done` gets the [`Summary`].
pub struct Recording {
    folder: PathBuf,
    started: Instant,
    interval: Duration,
    next_due: Instant,
    /// When to look for a new frame again, while one is due and the capture hasn't sent it.
    poll_at: Instant,
    last_sequence: Option<u64>,
    next_number: usize,
    dropped: Arc<AtomicUsize>,
    queue: SyncSender<(usize, BgraImage)>,
    writers: JoinHandle<()>,
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
        let fps = fps.clamp(*FPS.start(), *FPS.end()) as u32;
        let interval = Duration::from_secs(1) / fps;
        // About a second of frames, so a slow moment doesn't drop any, but no more
        let (queue, frames) = std::sync::mpsc::sync_channel(fps as usize);
        let dropped = Arc::new(AtomicUsize::new(0));
        let writers = match spawn_writers(folder.clone(), frames, dropped.clone(), done) {
            Ok(writers) => writers,
            Err(error) => {
                // Nothing's in it, and nothing will be
                let _ = std::fs::remove_dir(&folder);
                return Err(format!("Record Screenshots: couldn't start writing frames: {error}"));
            }
        };
        Ok(Self {
            folder,
            started: now,
            interval,
            next_due: now,
            poll_at: now,
            last_sequence: None,
            next_number: 1,
            dropped,
            queue,
            writers,
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
        self.next_due.max(self.poll_at).min(self.started + MAX_LENGTH)
    }

    /// Saves the frame `sequence` when one is due and it's a new frame. `image` makes the capture
    /// region's picture of it, only then.
    pub fn offer(&mut self, now: Instant, sequence: u64, image: impl FnOnce() -> Option<BgraImage>) {
        if now < self.next_due {
            return;
        }
        // Due, but the capture hasn't sent another frame yet: looked for again shortly, not
        // right away, which would keep the session busy until it comes
        if self.last_sequence == Some(sequence) {
            self.poll_at = now + NEW_FRAME_POLL;
            return;
        }
        // On a grid from the start, so frames stay evenly spaced when the loop wakes late
        let behind = (now - self.next_due).as_nanos() / self.interval.as_nanos().max(1);
        self.next_due += self.interval * (behind as u32 + 1);
        self.last_sequence = Some(sequence);
        let Some(image) = image() else { return };
        match self.queue.try_send((self.next_number, image)) {
            Ok(()) => self.next_number += 1,
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Ends the recording. The frames queued are still written by the thread this returns, which
    /// gives the summary once they are.
    pub fn stop(self) -> JoinHandle<()> {
        drop(self.queue);
        self.writers
    }
}

/// A new `recording_NNN` folder in `directory`, the first number free, made here so two
/// recordings can't take the same one.
fn claim_folder(directory: &Path) -> std::io::Result<PathBuf> {
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

/// Starts a thread that encodes and writes the frames of `frames` into `folder` on several
/// threads of its own, then gives `done` the summary.
fn spawn_writers(
    folder: PathBuf,
    frames: Receiver<(usize, BgraImage)>,
    dropped: Arc<AtomicUsize>,
    done: impl FnOnce(Summary) + Send + 'static,
) -> std::io::Result<JoinHandle<()>> {
    let workers =
        std::thread::available_parallelism().map_or(2, |count| count.get().saturating_sub(1)).clamp(1, MAX_WORKERS);
    std::thread::Builder::new().name("recording".into()).spawn(move || {
        let frames = Mutex::new(frames);
        let written = AtomicUsize::new(0);
        let error: Mutex<Option<String>> = Mutex::default();
        let note = |reason: String| {
            if let Ok(mut error) = error.lock() {
                error.get_or_insert(reason);
            }
        };
        std::thread::scope(|scope| {
            let mut started = 0;
            for index in 0..workers {
                let spawned =
                    std::thread::Builder::new().name(format!("recording {index}")).spawn_scoped(scope, || {
                        loop {
                            // Only while taking the next frame, so the others encode meanwhile
                            let next = frames.lock().map(|frames| frames.recv());
                            let Ok(Ok((number, image))) = next else { return };
                            match write_frame(&folder, number, &image) {
                                Ok(()) => {
                                    written.fetch_add(1, Ordering::Relaxed);
                                }
                                Err(reason) => note(reason),
                            }
                        }
                    });
                started += usize::from(spawned.is_ok());
            }
            if started == 0 {
                note("no thread could be started to write them".to_owned());
            }
        });
        let error = error.into_inner().ok().flatten();
        done(Summary { folder, written: written.into_inner(), dropped: dropped.load(Ordering::Relaxed), error });
    })
}

/// Writes `image` as frame `number` in `folder`, through a temporary file, so a frame cut off
/// halfway, like when the program closes, is never left looking like a whole one.
fn write_frame(folder: &Path, number: usize, image: &BgraImage) -> Result<(), String> {
    let path = folder.join(format!("{number:04}.png"));
    let temporary = folder.join(format!("{number:04}.png.tmp"));
    let png = image.encode_png().map_err(|error| error.to_string())?;
    let written = std::fs::write(&temporary, png).and_then(|()| std::fs::rename(&temporary, &path));
    written.map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        format!("{}: {error}", path.display())
    })
}
