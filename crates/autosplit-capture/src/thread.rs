use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;

use crate::frame::FrameSlot;
use crate::{CaptureError, DeviceInfo};

/// A backend's capture thread, stopped and joined when dropped.
pub(crate) struct CaptureThread {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl CaptureThread {
    /// On a new thread: `start` opens the device, and `run` then captures until `stop` is set.
    /// Returns once the device opened or failed to. A panic marks the device as failed, so it's
    /// reopened.
    pub(crate) fn spawn<S>(
        thread_name: &str,
        device: &DeviceInfo,
        slot: Arc<FrameSlot>,
        start: impl FnOnce(&DeviceInfo, &Arc<FrameSlot>) -> Result<S, String> + Send + 'static,
        run: impl FnOnce(S, &AtomicBool, &FrameSlot) + Send + 'static,
    ) -> Result<Self, CaptureError> {
        let error = |reason: String| CaptureError::Open { name: device.name.clone(), reason };
        let stop = Arc::new(AtomicBool::new(false));
        let (ready_sender, ready) = mpsc::channel();
        let (thread_stop, thread_device) = (stop.clone(), device.clone());
        let thread = std::thread::Builder::new()
            .name(thread_name.into())
            .spawn(move || {
                let captured = catch_unwind(AssertUnwindSafe(|| match start(&thread_device, &slot) {
                    Ok(started) => {
                        let _ = ready_sender.send(Ok(()));
                        run(started, &thread_stop, &slot);
                    }
                    Err(reason) => {
                        let _ = ready_sender.send(Err(reason));
                    }
                }));
                if captured.is_err() {
                    slot.fail();
                    let _ = ready_sender.send(Err("the capture thread crashed".into()));
                }
            })
            .map_err(|e| error(e.to_string()))?;
        match ready.recv() {
            Ok(Ok(())) => Ok(Self { stop, thread: Some(thread) }),
            Ok(Err(reason)) => {
                let _ = thread.join();
                Err(error(reason))
            }
            Err(_) => Err(error("the capture thread stopped".into())),
        }
    }
}

impl Drop for CaptureThread {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The device to open from a fresh listing: the one with the picked device's name, preferring
/// its index when several share the name. Devices can come and go between listing and opening.
#[cfg(any(windows, target_os = "macos"))]
pub(crate) fn find_again<T>(devices: Vec<(T, String)>, device: &DeviceInfo) -> Option<(usize, T)> {
    let mut fallback = None;
    for (index, (handle, name)) in devices.into_iter().enumerate() {
        if name != device.name {
            continue;
        }
        if index == device.index {
            return Some((index, handle));
        }
        fallback.get_or_insert((index, handle));
    }
    fallback
}
