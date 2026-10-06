//! AVFoundation capture. AutoSplit doesn't run on macOS, so there's nothing to match: frames
//! come from the device's active format, converted to BGRA by macOS.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AllocAnyThread, DefinedClass, define_class, msg_send};
use objc2_av_foundation::{
    AVAuthorizationStatus, AVCaptureConnection, AVCaptureDevice, AVCaptureDeviceInput, AVCaptureOutput,
    AVCaptureSession, AVCaptureSessionPresetInputPriority, AVCaptureVideoDataOutput,
    AVCaptureVideoDataOutputSampleBufferDelegate, AVMediaTypeVideo,
};
use objc2_core_media::CMSampleBuffer;
use objc2_core_video::{
    CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight, CVPixelBufferGetWidth,
    CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
    kCVPixelBufferPixelFormatTypeKey, kCVPixelFormatType_32BGRA,
};
use objc2_foundation::{NSDictionary, NSNumber, NSString};

use crate::convert::{Layout, PixelFormat};
use crate::frame::FrameSlot;
use crate::thread::{CaptureThread, find_again};
use crate::{CaptureError, DeviceInfo};

#[allow(deprecated)]
fn video_devices() -> Vec<Retained<AVCaptureDevice>> {
    let Some(video) = (unsafe { AVMediaTypeVideo }) else { return Vec::new() };
    unsafe { AVCaptureDevice::devicesWithMediaType(video) }.to_vec()
}

pub fn list_devices() -> Result<Vec<DeviceInfo>, CaptureError> {
    Ok(video_devices()
        .iter()
        .enumerate()
        .map(|(index, device)| DeviceInfo { index, name: unsafe { device.localizedName() }.to_string() })
        .collect())
}

/// Asks for camera access the first time; macOS shows its permission prompt.
fn ensure_access() -> Result<(), String> {
    let Some(video) = (unsafe { AVMediaTypeVideo }) else { return Err("video capture is unavailable".into()) };
    let denied = || {
        "Camera access is denied. Allow your terminal app (or AutoSplit) in System Settings > Privacy & Security > \
         Camera, then quit and reopen it."
            .to_owned()
    };
    match unsafe { AVCaptureDevice::authorizationStatusForMediaType(video) } {
        AVAuthorizationStatus::Authorized => Ok(()),
        AVAuthorizationStatus::NotDetermined => {
            let (sender, receiver) = mpsc::channel();
            let handler = RcBlock::new(move |granted: Bool| {
                let _ = sender.send(granted.as_bool());
            });
            unsafe { AVCaptureDevice::requestAccessForMediaType_completionHandler(video, &handler) };
            if receiver.recv().unwrap_or(false) { Ok(()) } else { Err(denied()) }
        }
        _ => Err(denied()),
    }
}

struct DelegateIvars {
    slot: Arc<FrameSlot>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "AutoSplitCaptureDelegate"]
    #[ivars = DelegateIvars]
    struct FrameDelegate;

    unsafe impl NSObjectProtocol for FrameDelegate {}

    unsafe impl AVCaptureVideoDataOutputSampleBufferDelegate for FrameDelegate {
        #[unsafe(method(captureOutput:didOutputSampleBuffer:fromConnection:))]
        fn capture_output(
            &self,
            _output: &AVCaptureOutput,
            sample_buffer: &CMSampleBuffer,
            _connection: &AVCaptureConnection,
        ) {
            let Some(pixel_buffer) = (unsafe { sample_buffer.image_buffer() }) else { return };
            let slot = &self.ivars().slot;
            unsafe { CVPixelBufferLockBaseAddress(&pixel_buffer, CVPixelBufferLockFlags::ReadOnly) };
            let (width, height) = (CVPixelBufferGetWidth(&pixel_buffer), CVPixelBufferGetHeight(&pixel_buffer));
            let stride = CVPixelBufferGetBytesPerRow(&pixel_buffer);
            let base = CVPixelBufferGetBaseAddress(&pixel_buffer) as *const u8;
            if !base.is_null() {
                let data = unsafe { std::slice::from_raw_parts(base, stride * height) };
                slot.receive(data, Layout { format: PixelFormat::Bgrx, width, height, stride });
            }
            unsafe { CVPixelBufferUnlockBaseAddress(&pixel_buffer, CVPixelBufferLockFlags::ReadOnly) };
        }
    }
);

impl FrameDelegate {
    fn new(slot: Arc<FrameSlot>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(DelegateIvars { slot });
        unsafe { msg_send![super(this), init] }
    }
}

pub fn open(device: &DeviceInfo, slot: Arc<FrameSlot>) -> Result<CaptureThread, CaptureError> {
    CaptureThread::spawn("AVFoundation capture", device, slot, |device, slot| start(device, slot.clone()), run)
}

struct Session {
    session: Retained<AVCaptureSession>,
    device: Retained<AVCaptureDevice>,
    _delegate: Retained<FrameDelegate>,
}

fn start(picked: &DeviceInfo, slot: Arc<FrameSlot>) -> Result<Session, String> {
    ensure_access()?;
    let devices = video_devices().into_iter().map(|device| {
        let name = unsafe { device.localizedName() }.to_string();
        (device, name)
    });
    let (_, device) = find_again(devices.collect(), picked).ok_or("the device is no longer connected")?;
    unsafe {
        let input = AVCaptureDeviceInput::deviceInputWithDevice_error(&device).map_err(|error| error.to_string())?;
        let session = AVCaptureSession::new();
        // Keep the device's current format instead of a preset's (often 720p)
        if session.canSetSessionPreset(AVCaptureSessionPresetInputPriority) {
            session.setSessionPreset(AVCaptureSessionPresetInputPriority);
        }
        if !session.canAddInput(&input) {
            return Err("the device can't be used for capture".into());
        }
        session.addInput(&input);

        let output = AVCaptureVideoDataOutput::new();
        let key: &NSString = kCVPixelBufferPixelFormatTypeKey.as_ref();
        let format = NSNumber::new_u32(kCVPixelFormatType_32BGRA);
        let settings = NSDictionary::<NSString, AnyObject>::from_slices(&[key], &[format.as_ref()]);
        output.setVideoSettings(Some(&settings));
        output.setAlwaysDiscardsLateVideoFrames(true);
        let delegate = FrameDelegate::new(slot);
        let queue = DispatchQueue::new("autosplit.capture", None);
        output.setSampleBufferDelegate_queue(Some(ProtocolObject::from_ref(&*delegate)), Some(&queue));
        if !session.canAddOutput(&output) {
            return Err("the device's video output can't be read".into());
        }
        session.addOutput(&output);
        session.startRunning();
        Ok(Session { session, device, _delegate: delegate })
    }
}

/// Watches for the device going away until stopped.
fn run(session: Session, stop: &AtomicBool, slot: &FrameSlot) {
    while !stop.load(Ordering::Relaxed) {
        if !unsafe { session.device.isConnected() } {
            slot.fail();
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    unsafe { session.session.stopRunning() };
}
