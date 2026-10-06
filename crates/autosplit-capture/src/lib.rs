//! Video capture devices (capture cards, OBS Virtual Camera, webcams), producing the same
//! pixels AutoSplit gets from OpenCV.
//!
//! Windows uses DirectShow like OpenCV's backend, Linux V4L2, macOS AVFoundation.

#[doc(hidden)]
pub mod convert;
mod frame;
#[cfg(target_os = "linux")]
mod jpeg;
#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
mod thread;

#[cfg(target_os = "macos")]
mod avfoundation;
#[cfg(windows)]
mod dshow;
#[cfg(target_os = "linux")]
mod v4l2;

use std::sync::Arc;

use autosplit_core::profile::Region;
use std::time::Instant;

pub use frame::{FRAME_TIMEOUT, Frame, is_obs_placeholder};

#[doc(hidden)]
pub use frame::FrameSlot;

#[cfg(target_os = "macos")]
use avfoundation as backend;
#[cfg(windows)]
use dshow as backend;
#[cfg(target_os = "linux")]
use v4l2 as backend;

/// A capture device. `index` is its position in the system's device list, which is what
/// AutoSplit profiles store as `capture_device_id` (DirectShow order on Windows).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    pub index: usize,
    pub name: String,
}

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("Couldn't list capture devices: {0}")]
    List(String),
    #[error("Couldn't open '{name}': {reason}")]
    Open { name: String, reason: String },
    #[error("Video capture devices aren't supported on this platform.")]
    Unsupported,
}

/// Lists the capture devices, in the system's order.
pub fn list_devices() -> Result<Vec<DeviceInfo>, CaptureError> {
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    return backend::list_devices();
    #[allow(unreachable_code)]
    Err(CaptureError::Unsupported)
}

/// How [`pick_device`] found the device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickedBy {
    Name,
    /// No device has the saved name, so the saved index was used.
    Index,
}

/// The profile's device: the one named `name`, or the one at `index` when none has that name.
pub fn pick_device<'a>(devices: &'a [DeviceInfo], name: &str, index: i64) -> Option<(&'a DeviceInfo, PickedBy)> {
    if !name.is_empty()
        && let Some(device) = devices.iter().find(|device| device.name == name)
    {
        return Some((device, PickedBy::Name));
    }
    let index = usize::try_from(index).ok()?;
    devices.iter().find(|device| device.index == index).map(|device| (device, PickedBy::Index))
}

/// An open capture device, capturing on a background thread until dropped.
pub struct Capture {
    device: DeviceInfo,
    slot: Arc<FrameSlot>,
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    _thread: thread::CaptureThread,
}

impl Capture {
    pub fn open(device: &DeviceInfo) -> Result<Self, CaptureError> {
        let slot = Arc::new(FrameSlot::default());
        #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
        return Ok(Self { device: device.clone(), slot: slot.clone(), _thread: backend::open(device, slot)? });
        #[allow(unreachable_code)]
        {
            let _ = slot;
            Err(CaptureError::Unsupported)
        }
    }

    pub fn device(&self) -> &DeviceInfo {
        &self.device
    }

    /// The newest frame, or `None` while no frame arrived within [`FRAME_TIMEOUT`].
    pub fn latest(&self) -> Option<Arc<Frame>> {
        self.slot.latest(Instant::now())
    }

    /// Whether a frame arrived within [`FRAME_TIMEOUT`].
    pub fn receiving(&self) -> bool {
        self.slot.receiving(Instant::now())
    }

    /// The part of frames that's compared, so only that part is converted. `None` converts
    /// whole frames. Use [`Frame::region_view`] to read it.
    pub fn set_region(&self, region: Option<Region>) {
        self.slot.set_region(region);
    }

    /// Whether frames will be compared soon. While they aren't, frames are kept unconverted,
    /// which saves most of the CPU time capturing takes; [`Capture::latest`] still converts the
    /// newest one when asked.
    pub fn set_wanted(&self, wanted: bool) {
        self.slot.set_wanted(wanted);
    }

    /// OBS Virtual Camera sent its "not started" placeholder.
    pub fn placeholder_seen(&self) -> bool {
        self.slot.placeholder_seen()
    }

    /// The device stopped working, e.g. it was unplugged. Reopen it to recover.
    pub fn failed(&self) -> bool {
        self.slot.failed()
    }
}
