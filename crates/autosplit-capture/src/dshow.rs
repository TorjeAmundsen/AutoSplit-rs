//! DirectShow capture, replicating OpenCV 4.11's `videoInput` (`cap_dshow.cpp`), which is what
//! AutoSplit's `cv2.VideoCapture(index)` uses on Windows. Same device order, same format
//! negotiation and the same Sample Grabber setup, so Windows' own filters convert capture card
//! formats exactly like they do for AutoSplit. OBS Virtual Camera delivers NV12, converted with
//! OpenCV's math.
//!
//! All COM work happens on threads this module owns, so the caller's thread COM state (e.g. a
//! GUI's single-threaded apartment) is never touched.

#![allow(non_snake_case)]

use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::{E_POINTER, S_OK};
use windows::Win32::Media::DirectShow::{
    EC_DEVICE_LOST, IAMStreamConfig, IBaseFilter, ICaptureGraphBuilder2, ICreateDevEnum, IGraphBuilder, IMediaControl,
    IMediaEventEx, IMediaFilter, IMediaSample, PINDIR_OUTPUT, VIDEO_STREAM_CONFIG_CAPS,
};
use windows::Win32::Media::MediaFoundation::{AM_MEDIA_TYPE, VIDEOINFOHEADER};
use windows::Win32::System::Com::StructuredStorage::IPropertyBag;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize,
    IEnumMoniker, IMoniker,
};
use windows::Win32::System::Variant::{VARIANT, VariantClear};
use windows::core::{BOOL, GUID, HRESULT, IUnknown, IUnknown_Vtbl, Interface, PCWSTR, implement, interface, w};

use crate::convert::Yuv420;
use crate::convert::{Layout, PixelFormat};
use crate::frame::FrameSlot;
use crate::thread::{CaptureThread, find_again};
use crate::{CaptureError, DeviceInfo};

const fn guid(value: u128) -> GUID {
    GUID::from_u128(value)
}

/// The GUID DirectShow uses for a FOURCC media subtype.
const fn fourcc(code: &[u8; 4]) -> GUID {
    let data1 = u32::from_le_bytes(*code) as u128;
    GUID::from_u128((data1 << 96) | 0x0000_0010_8000_00aa_0038_9b71)
}

const CLSID_SYSTEM_DEVICE_ENUM: GUID = guid(0x62be5d10_60eb_11d0_bd3b_00a0c911ce86);
const CLSID_VIDEO_INPUT_DEVICE_CATEGORY: GUID = guid(0x860bb310_5d01_11d0_bd3b_00a0c911ce86);
const CLSID_CAPTURE_GRAPH_BUILDER2: GUID = guid(0xbf87b6e1_8c27_11d0_b3f0_00aa003761c5);
const CLSID_FILTER_GRAPH: GUID = guid(0xe436ebb3_524f_11ce_9f53_0020af0ba770);
// qedit.h, no longer in the Windows SDK but qedit.dll still ships with Windows
const CLSID_SAMPLE_GRABBER: GUID = guid(0xc1f400a0_3f08_11d3_9f0b_006008039e37);
const CLSID_NULL_RENDERER: GUID = guid(0xc1f400a4_3f08_11d3_9f0b_006008039e37);
const PIN_CATEGORY_CAPTURE: GUID = guid(0xfb6c4281_0353_11d1_905f_0000c0cc16ba);
const PIN_CATEGORY_PREVIEW: GUID = guid(0xfb6c4282_0353_11d1_905f_0000c0cc16ba);
const MEDIATYPE_VIDEO: GUID = fourcc(b"vids");
const FORMAT_VIDEO_INFO: GUID = guid(0x05589f80_c356_11ce_bf01_00aa0055595a);

const MEDIASUBTYPE_RGB24: GUID = guid(0xe436eb7d_524f_11ce_9f53_0020af0ba770);
const MEDIASUBTYPE_NV12: GUID = fourcc(b"NV12");

/// `videoInput::mediaSubtypes`: the order OpenCV tries subtypes in when setting a size.
const MEDIA_SUBTYPES: [GUID; 23] = [
    MEDIASUBTYPE_RGB24,
    guid(0xe436eb7e_524f_11ce_9f53_0020af0ba770), // RGB32
    guid(0xe436eb7c_524f_11ce_9f53_0020af0ba770), // RGB555
    guid(0xe436eb7b_524f_11ce_9f53_0020af0ba770), // RGB565
    fourcc(b"YUY2"),
    fourcc(b"YVYU"),
    fourcc(b"YUYV"),
    fourcc(b"IYUV"),
    fourcc(b"UYVY"),
    fourcc(b"YV12"),
    fourcc(b"YVU9"),
    fourcc(b"Y411"),
    fourcc(b"Y41P"),
    fourcc(b"Y211"),
    fourcc(b"AYUV"),
    fourcc(b"MJPG"),
    fourcc(b"Y800"),
    fourcc(b"Y8  "),
    fourcc(b"GREY"),
    fourcc(b"I420"),
    fourcc(b"BY8 "),
    fourcc(b"Y16 "),
    MEDIASUBTYPE_NV12,
];

/// Devices OpenCV forces to NV12, because they accept any format but only really send NV12.
const NV12_DEVICES: [&str; 2] = ["OBS Virtual Camera", "Streamlabs Desktop Virtual Webcam"];

#[interface("0579154A-2B53-4994-B0D0-E773148EFF85")]
unsafe trait ISampleGrabberCB: IUnknown {
    fn SampleCB(&self, sample_time: f64, sample: *mut c_void) -> HRESULT;
    fn BufferCB(&self, sample_time: f64, buffer: *mut u8, length: i32) -> HRESULT;
}

#[interface("6B652FFF-11FE-4fce-92AD-0266B5D7C78F")]
unsafe trait ISampleGrabber: IUnknown {
    fn SetOneShot(&self, one_shot: BOOL) -> HRESULT;
    fn SetMediaType(&self, media_type: *const AM_MEDIA_TYPE) -> HRESULT;
    fn GetConnectedMediaType(&self, media_type: *mut AM_MEDIA_TYPE) -> HRESULT;
    fn SetBufferSamples(&self, buffer_samples: BOOL) -> HRESULT;
    fn GetCurrentBuffer(&self, size: *mut i32, buffer: *mut i32) -> HRESULT;
    fn GetCurrentSample(&self, sample: *mut *mut c_void) -> HRESULT;
    fn SetCallback(&self, callback: *mut c_void, which_method: i32) -> HRESULT;
}

/// The current thread's membership of the multithreaded COM apartment, left when dropped.
struct ComApartment {
    initialized: bool,
}

impl ComApartment {
    fn enter() -> Self {
        Self { initialized: unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok() }
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.initialized {
            unsafe { CoUninitialize() };
        }
    }
}

/// Runs `work` on a fresh thread inside a multithreaded COM apartment.
fn with_com<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    std::thread::spawn(move || {
        let _apartment = ComApartment::enter();
        work()
    })
    .join()
    .ok()
}

/// The video input devices OpenCV can open, in its order: monikers whose property bag can't
/// be read are skipped, like `videoInput::listDevices` does.
fn monikers() -> windows::core::Result<Vec<(IMoniker, String)>> {
    unsafe {
        let device_enum: ICreateDevEnum = CoCreateInstance(&CLSID_SYSTEM_DEVICE_ENUM, None, CLSCTX_INPROC_SERVER)?;
        let mut enum_moniker: Option<IEnumMoniker> = None;
        // S_FALSE means there are no devices, and leaves the enumerator empty
        device_enum.CreateClassEnumerator(&CLSID_VIDEO_INPUT_DEVICE_CATEGORY, &mut enum_moniker, 0)?;
        let Some(enum_moniker) = enum_moniker else {
            return Ok(Vec::new());
        };
        let mut devices = Vec::new();
        loop {
            let mut moniker = [None];
            if enum_moniker.Next(&mut moniker, None) != S_OK {
                break;
            }
            let Some(moniker) = moniker[0].take() else { break };
            let Ok(bag) = moniker.BindToStorage::<_, _, IPropertyBag>(None, None) else {
                continue;
            };
            let mut name = VARIANT::default();
            let read = bag.Read(w!("FriendlyName"), &mut name, None);
            let friendly_name = if read.is_ok() { name.to_string() } else { String::new() };
            let _ = VariantClear(&mut name);
            devices.push((moniker, friendly_name));
        }
        Ok(devices)
    }
}

pub fn list_devices() -> Result<Vec<DeviceInfo>, CaptureError> {
    with_com(|| {
        monikers()
            .map(|monikers| {
                monikers.into_iter().enumerate().map(|(index, (_, name))| DeviceInfo { index, name }).collect()
            })
            .map_err(|error| error.message())
    })
    .unwrap_or_else(|| Err("listing devices crashed".into()))
    .map_err(CaptureError::List)
}

/// Hands frames from the Sample Grabber to the slot, like `SampleGrabberCallback::SampleCB`.
#[implement(ISampleGrabberCB)]
struct GrabberCallback {
    slot: Arc<FrameSlot>,
    layout: Layout,
}

impl ISampleGrabberCB_Impl for GrabberCallback_Impl {
    unsafe fn SampleCB(&self, _sample_time: f64, sample: *mut c_void) -> HRESULT {
        let Some(sample) = (unsafe { IMediaSample::from_raw_borrowed(&sample) }) else {
            return E_POINTER;
        };
        let Ok(pointer) = (unsafe { sample.GetPointer() }) else { return S_OK };
        let length = unsafe { sample.GetActualDataLength() }.max(0) as usize;
        // OpenCV drops samples of any other size
        if pointer.is_null() || length != self.layout.frame_len() {
            return S_OK;
        }
        self.slot.receive(unsafe { std::slice::from_raw_parts(pointer, length) }, self.layout);
        S_OK
    }

    unsafe fn BufferCB(&self, _sample_time: f64, _buffer: *mut u8, _length: i32) -> HRESULT {
        windows::Win32::Foundation::E_NOTIMPL
    }
}

/// Owns an `AM_MEDIA_TYPE` allocated by DirectShow (`DeleteMediaType`).
struct MediaType(*mut AM_MEDIA_TYPE);

impl MediaType {
    fn info(&mut self) -> &mut VIDEOINFOHEADER {
        unsafe { &mut *((*self.0).pbFormat as *mut VIDEOINFOHEADER) }
    }
}

impl Drop for MediaType {
    fn drop(&mut self) {
        unsafe {
            if self.0.is_null() {
                return;
            }
            let media_type = &mut *self.0;
            if media_type.cbFormat != 0 {
                CoTaskMemFree(Some(media_type.pbFormat as *const c_void));
            }
            if let Some(unknown) = media_type.pUnk.take() {
                drop(unknown);
            }
            CoTaskMemFree(Some(self.0 as *const c_void));
        }
    }
}

/// `setSizeAndSubtype`: changes size and subtype on the current media type and applies it. Like
/// OpenCV, the bitmap header's compression and bit count are left as they were.
fn set_size_and_subtype(
    config: &IAMStreamConfig,
    media_type: &mut MediaType,
    width: i32,
    height: i32,
    subtype: GUID,
) -> bool {
    unsafe {
        let Ok(previous) = config.GetFormat() else { return false };
        let previous = MediaType(previous);
        let info = media_type.info();
        info.bmiHeader.biWidth = width;
        info.bmiHeader.biHeight = height;
        info.rcSource.left = 0;
        info.rcSource.top = 0;
        info.rcTarget.left = 0;
        info.rcTarget.top = 0;
        info.rcSource.right = width;
        info.rcTarget.right = width;
        info.rcSource.bottom = height;
        info.rcTarget.bottom = height;
        let raw = &mut *media_type.0;
        raw.formattype = FORMAT_VIDEO_INFO;
        raw.majortype = MEDIATYPE_VIDEO;
        raw.subtype = subtype;
        let pixels = (width * height) as u32;
        raw.lSampleSize = if subtype == MEDIASUBTYPE_RGB24 {
            pixels * 3
        } else if [fourcc(b"YUY2"), fourcc(b"YVYU"), fourcc(b"UYVY")].contains(&subtype) {
            pixels * 2
        } else {
            0
        };
        if config.SetFormat(media_type.0).is_ok() {
            return true;
        }
        let _ = config.SetFormat(previous.0);
        *media_type = previous;
        false
    }
}

/// `findClosestSizeAndSubtype`: the stream capability whose sizes come closest.
fn closest_size_and_subtype(config: &IAMStreamConfig, width: i32, height: i32) -> Option<(i32, i32, GUID)> {
    unsafe {
        let (mut count, mut size) = (0, 0);
        config.GetNumberOfCapabilities(&mut count, &mut size).ok()?;
        if size as usize != size_of::<VIDEO_STREAM_CONFIG_CAPS>() {
            return None;
        }
        let (mut near_w, mut near_h) = (9_999_999, 9_999_999);
        let mut found = None;
        for index in 0..count {
            let mut caps = VIDEO_STREAM_CONFIG_CAPS::default();
            let mut config_type = std::ptr::null_mut();
            if config.GetStreamCaps(index, &mut config_type, &mut caps as *mut _ as *mut u8).is_err() {
                continue;
            }
            let config_type = MediaType(config_type);
            let (step_x, step_y) = (caps.OutputGranularityX, caps.OutputGranularityY);
            if step_x < 1 || step_y < 1 {
                continue;
            }
            let subtype = (*config_type.0).subtype;
            let closest = |target: i32, min: i32, max: i32, step: i32| -> (i32, bool) {
                let mut best = 999_999;
                let mut exact = false;
                let mut value = min;
                while value <= max {
                    if value == target {
                        exact = true;
                        best = value;
                    } else if (target - value).abs() < (target - best).abs() {
                        best = value;
                    }
                    value += step;
                }
                (best, exact)
            };
            let (temp_w, exact_x) = closest(width, caps.MinOutputSize.cx, caps.MaxOutputSize.cx, step_x);
            let (temp_h, exact_y) = closest(height, caps.MinOutputSize.cy, caps.MaxOutputSize.cy, step_y);
            if exact_x && exact_y {
                return Some((width, height, subtype));
            }
            if (width - temp_w).abs() + (height - temp_h).abs() < (width - near_w).abs() + (height - near_h).abs() {
                near_w = temp_w;
                near_h = temp_h;
                found = Some((near_w, near_h, subtype));
            }
        }
        found
    }
}

struct Graph {
    control: IMediaControl,
    events: IMediaEventEx,
}

/// `videoInput::start` with AutoSplit's settings: the capture size is the device's current
/// format size (AutoSplit sets it from the format DirectShow reports), trying OpenCV's subtypes
/// in order, at the device's current frame rate.
fn build_graph(moniker: &IMoniker, name: &str, slot: Arc<FrameSlot>) -> windows::core::Result<Graph> {
    unsafe {
        let builder: ICaptureGraphBuilder2 =
            CoCreateInstance(&CLSID_CAPTURE_GRAPH_BUILDER2, None, CLSCTX_INPROC_SERVER)?;
        let graph: IGraphBuilder = CoCreateInstance(&CLSID_FILTER_GRAPH, None, CLSCTX_INPROC_SERVER)?;
        let events: IMediaEventEx = graph.cast()?;
        builder.SetFiltergraph(&graph)?;
        let control: IMediaControl = graph.cast()?;

        let source: IBaseFilter = moniker.BindToObject(None, None)?;
        let wide_name: Vec<u16> = name.encode_utf16().chain([0]).collect();
        graph.AddFilter(&source, PCWSTR(wide_name.as_ptr()))?;

        let find_config = |category: &GUID| -> windows::core::Result<IAMStreamConfig> {
            let mut config: *mut c_void = std::ptr::null_mut();
            builder.FindInterface(
                Some(category),
                Some(&MEDIATYPE_VIDEO),
                &source,
                &IAMStreamConfig::IID,
                &mut config,
            )?;
            Ok(IAMStreamConfig::from_raw(config))
        };
        let config = find_config(&PIN_CATEGORY_PREVIEW).or_else(|_| find_config(&PIN_CATEGORY_CAPTURE))?;

        let mut media_type = MediaType(config.GetFormat()?);
        let current = media_type.info().bmiHeader;
        let (width, height) = first_output_size(&source).unwrap_or((current.biWidth, current.biHeight));

        let mut chosen = None;
        if NV12_DEVICES.contains(&name)
            && set_size_and_subtype(&config, &mut media_type, width, height, MEDIASUBTYPE_NV12)
        {
            chosen = Some((width, height));
        }
        if chosen.is_none() {
            chosen = MEDIA_SUBTYPES
                .iter()
                .any(|subtype| set_size_and_subtype(&config, &mut media_type, width, height, *subtype))
                .then_some((width, height));
        }
        if chosen.is_none()
            && let Some((closest_w, closest_h, subtype)) = closest_size_and_subtype(&config, width, height)
            && set_size_and_subtype(&config, &mut media_type, closest_w, closest_h, subtype)
        {
            chosen = Some((closest_w, closest_h));
        }
        let (width, height) = chosen.unwrap_or((width, height));
        let nv12 = (*media_type.0).subtype == MEDIASUBTYPE_NV12;
        drop(media_type);

        let grabber_filter: IBaseFilter = CoCreateInstance(&CLSID_SAMPLE_GRABBER, None, CLSCTX_INPROC_SERVER)?;
        graph.AddFilter(&grabber_filter, w!("Sample Grabber"))?;
        let grabber: ISampleGrabber = grabber_filter.cast()?;
        grabber.SetOneShot(false.into()).ok()?;
        grabber.SetBufferSamples(false.into()).ok()?;
        let (width, height) = (width.max(0) as usize, height.unsigned_abs() as usize);
        // RGB24 is B, G, R in DIB order, bottom row first
        let format = if nv12 { PixelFormat::Yuv420(Yuv420::Nv12) } else { PixelFormat::Bgr24 { bottom_up: true } };
        let layout = Layout::packed(format, width, height);
        let callback: ISampleGrabberCB = GrabberCallback { slot, layout }.into();
        grabber.SetCallback(callback.as_raw(), 0).ok()?;

        // NV12 stays NV12 (OpenCV converts it), everything else becomes RGB24 through
        // Windows' own conversion filters
        let grabber_type = AM_MEDIA_TYPE {
            majortype: MEDIATYPE_VIDEO,
            subtype: if nv12 { MEDIASUBTYPE_NV12 } else { MEDIASUBTYPE_RGB24 },
            formattype: FORMAT_VIDEO_INFO,
            ..Default::default()
        };
        grabber.SetMediaType(&grabber_type).ok()?;

        let renderer: IBaseFilter = CoCreateInstance(&CLSID_NULL_RENDERER, None, CLSCTX_INPROC_SERVER)?;
        graph.AddFilter(&renderer, w!("NullRenderer"))?;
        builder.RenderStream(Some(&PIN_CATEGORY_PREVIEW), &MEDIATYPE_VIDEO, &source, &grabber_filter, &renderer)?;

        // Deliver frames as soon as they arrive instead of waiting on a reference clock
        if let Ok(filter) = graph.cast::<IMediaFilter>() {
            let _ = filter.SetSyncSource(None);
        }
        control.Run()?;
        Ok(Graph { control, events })
    }
}

pub fn open(device: &DeviceInfo, slot: Arc<FrameSlot>) -> Result<CaptureThread, CaptureError> {
    CaptureThread::spawn("DirectShow capture", device, slot, start, |(graph, apartment), stop, slot| {
        watch(&graph, slot, stop);
        // The graph has to go before its COM apartment
        drop(graph);
        drop(apartment);
    })
}

/// The graph, and the COM apartment it lives in, which has to outlast it.
fn start(device: &DeviceInfo, slot: &Arc<FrameSlot>) -> Result<(Graph, ComApartment), String> {
    let apartment = ComApartment::enter();
    let graph = monikers()
        .and_then(|monikers| {
            let (_, moniker) = find_again(monikers, device)
                .ok_or_else(|| windows::core::Error::new(E_POINTER, "the device is no longer connected"))?;
            build_graph(&moniker, &device.name, slot.clone())
        })
        .map_err(|error| error.message())?;
    Ok((graph, apartment))
}

/// Watches for the device going away until stopped.
fn watch(graph: &Graph, slot: &FrameSlot, stop: &AtomicBool) {
    while !stop.load(Ordering::Relaxed) {
        let (mut code, mut param1, mut param2) = (0, 0, 0);
        // Waits for an event, or times out to check `stop`
        if unsafe { graph.events.GetEvent(&mut code, &mut param1, &mut param2, 250) }.is_ok() {
            unsafe {
                let _ = graph.events.FreeEventParams(code, param1, param2);
            }
            if code == EC_DEVICE_LOST as i32 && param2 == 0 {
                slot.fail();
            }
        }
    }
    unsafe {
        let _ = graph.control.Stop();
    }
}

/// The current size of the device's first output pin, which is where AutoSplit (through
/// pygrabber's `get_current_format`) reads the size it asks OpenCV for.
fn first_output_size(source: &IBaseFilter) -> Option<(i32, i32)> {
    unsafe {
        let pins = source.EnumPins().ok()?;
        loop {
            let mut pin = [None];
            if pins.Next(&mut pin, None) != S_OK {
                return None;
            }
            let pin = pin[0].take()?;
            if pin.QueryDirection().ok()? != PINDIR_OUTPUT {
                continue;
            }
            let config: IAMStreamConfig = pin.cast().ok()?;
            let mut format = MediaType(config.GetFormat().ok()?);
            let header = format.info().bmiHeader;
            return Some((header.biWidth, header.biHeight));
        }
    }
}
