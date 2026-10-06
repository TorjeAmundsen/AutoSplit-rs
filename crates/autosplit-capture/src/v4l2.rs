//! V4L2 capture. Device indexes are the N in `/dev/videoN`, which is what AutoSplit's
//! `cv2.VideoCapture(index)` opens on Linux.
//!
//! Frames are captured at the device's current format, converted with the same math OpenCV's
//! V4L2 backend uses. Unlike OpenCV, padded rows are read with their stride.

use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use linuxvideo::format::{PixFormat, PixelFormat as Fourcc};
use linuxvideo::{BufType, CapabilityFlags, Device};

use crate::convert::{Layout, PixelFormat, Yuv420, Yuv422};
use crate::frame::FrameSlot;
use crate::thread::CaptureThread;
use crate::{CaptureError, DeviceInfo};

/// The formats OpenCV tries, in its order, when the current one isn't usable.
const TRIED_FORMATS: [(&[u8; 4], PixelFormat); 12] = [
    (b"BGR3", PixelFormat::Bgr24 { bottom_up: false }),
    (b"RGB3", PixelFormat::Rgb24),
    (b"YV12", PixelFormat::Yuv420(Yuv420::Yv12)),
    (b"YU12", PixelFormat::Yuv420(Yuv420::I420)),
    (b"YUYV", PixelFormat::Yuv422(Yuv422::Yuyv)),
    (b"UYVY", PixelFormat::Yuv422(Yuv422::Uyvy)),
    (b"NV12", PixelFormat::Yuv420(Yuv420::Nv12)),
    (b"NV21", PixelFormat::Yuv420(Yuv420::Nv21)),
    // B, G, R, X in memory; OpenCV drops the 4th byte
    (b"XR24", PixelFormat::Bgrx),
    (b"AR24", PixelFormat::Bgrx),
    (b"MJPG", PixelFormat::Mjpeg),
    (b"JPEG", PixelFormat::Mjpeg),
];

/// The format of a FOURCC, for the ones we convert: OpenCV's and grayscale.
fn pixel_format(fourcc: Fourcc) -> Option<PixelFormat> {
    match &fourcc.as_fourcc() {
        b"GREY" => Some(PixelFormat::Gray),
        code => TRIED_FORMATS.iter().find(|(tried, _)| *tried == code).map(|&(_, format)| format),
    }
}

fn device_path(index: usize) -> PathBuf {
    PathBuf::from(format!("/dev/video{index}"))
}

/// Metadata nodes, which UVC webcams add next to each camera, can't capture video.
fn captures_video(path: &Path) -> bool {
    match Device::open(path).and_then(|device| device.capabilities()) {
        Ok(capabilities) => capabilities.device_capabilities().contains(CapabilityFlags::VIDEO_CAPTURE),
        // Listed anyway, so opening it explains the problem (like missing permissions)
        Err(_) => true,
    }
}

fn all_nodes() -> Result<Vec<DeviceInfo>, CaptureError> {
    let entries = match std::fs::read_dir("/sys/class/video4linux") {
        Ok(entries) => entries,
        // The class only exists once a video device driver is loaded
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(CaptureError::List(error.to_string())),
    };
    let mut devices: Vec<DeviceInfo> = entries
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let index = entry.file_name().to_str()?.strip_prefix("video")?.parse().ok()?;
            let name = std::fs::read_to_string(entry.path().join("name")).ok()?.trim().to_owned();
            Some(DeviceInfo { index, name })
        })
        .collect();
    devices.sort_by_key(|device| device.index);
    Ok(devices)
}

pub fn list_devices() -> Result<Vec<DeviceInfo>, CaptureError> {
    Ok(all_nodes()?.into_iter().filter(|device| captures_video(&device_path(device.index))).collect())
}

pub fn open(device: &DeviceInfo, slot: Arc<FrameSlot>) -> Result<CaptureThread, CaptureError> {
    CaptureThread::spawn(
        "v4l2 capture",
        device,
        slot,
        |device, _| start(device),
        |(stream, layout), stop, slot| capture_loop(stream, layout, slot, stop),
    )
}

/// The node to open: the picked one if it still has the picked name, else another with it.
fn current_node(device: &DeviceInfo) -> Option<PathBuf> {
    let nodes = all_nodes().ok()?;
    let node = nodes
        .iter()
        .find(|node| node.index == device.index && node.name == device.name)
        .or_else(|| nodes.iter().find(|node| node.name == device.name && captures_video(&device_path(node.index))))?;
    Some(device_path(node.index))
}

fn start(device: &DeviceInfo) -> Result<(linuxvideo::stream::ReadStream, Layout), String> {
    let path = current_node(device).ok_or("the device is no longer connected")?;
    let opened = Device::open(&path).map_err(|error| error.to_string())?;
    let current = match opened.format(BufType::VIDEO_CAPTURE).map_err(|error| error.to_string())? {
        linuxvideo::format::Format::VideoCapture(format) => format,
        _ => return Err("not a video capture device".into()),
    };
    let (width, height) = (current.width(), current.height());
    let mut candidates = vec![current.pixel_format()];
    let tried = TRIED_FORMATS.iter().map(|(fourcc, _)| Fourcc::from_fourcc(**fourcc));
    candidates.extend(tried.filter(|candidate| *candidate != current.pixel_format()));

    let mut device = opened;
    let mut last_error = String::from("no supported pixel format");
    for candidate in candidates.into_iter().filter(|candidate| pixel_format(*candidate).is_some()) {
        match device.video_capture(PixFormat::new(width, height, candidate)) {
            Ok(capture) => {
                let negotiated = capture.format();
                match pixel_format(negotiated.pixel_format()) {
                    Some(format) => {
                        let (width, height) = (negotiated.width() as usize, negotiated.height() as usize);
                        let stride = (negotiated.bytes_per_line() as usize).max(width * format.bytes_per_pixel());
                        let layout = Layout { format, width, height, stride };
                        return capture.into_stream().map(|stream| (stream, layout)).map_err(|error| error.to_string());
                    }
                    None => last_error = "the driver switched to an unsupported pixel format".into(),
                }
            }
            Err(error) => last_error = error.to_string(),
        }
        device = Device::open(&path).map_err(|error| error.to_string())?;
    }
    Err(last_error)
}

/// Hands each frame to the slot straight from the driver's buffer, which the slot converts or
/// copies once. MJPEG frames are copied out first instead: decoding a whole frame can take long
/// enough on a slow CPU to hold up the driver, which only has 2 buffers.
fn capture_loop(mut stream: linuxvideo::stream::ReadStream, layout: Layout, slot: &FrameSlot, stop: &AtomicBool) {
    let fd = stream.as_raw_fd();
    let mut jpeg = Vec::new();
    while !stop.load(Ordering::Relaxed) {
        // Wait with a timeout so dropping the stream doesn't hang on a device that stopped
        let mut pollfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        let ready = unsafe { libc::poll(&mut pollfd, 1, 500) };
        if ready < 0 || pollfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
            slot.fail();
            return;
        }
        if ready == 0 {
            continue;
        }
        jpeg.clear();
        let result = stream.dequeue(|buffer| {
            if buffer.is_error() {
                return Ok(());
            }
            if layout.format == PixelFormat::Mjpeg {
                jpeg.extend_from_slice(&buffer);
            } else {
                slot.receive(&buffer, layout);
            }
            Ok(())
        });
        if result.is_err() {
            slot.fail();
            return;
        }
        if !jpeg.is_empty() {
            slot.receive(&jpeg, layout);
        }
    }
}
