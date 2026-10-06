//! Pixel format conversions to BGRA, matching what OpenCV produces for AutoSplit.
//!
//! The YUV conversions are OpenCV 4.11's `cvtColor` `COLOR_YUV2BGR_*` paths: BT.601 limited
//! range in 20-bit fixed point (`color_yuv.simd.hpp`). Alpha is always 255, like AutoSplit's
//! `COLOR_BGR2BGRA` step.
//!
//! A [`Layout`] converts an area of the frame, usually the capture region, since converting
//! pixels nobody compares is wasted work. Every pixel only depends on its own source bytes, so a
//! converted area is identical to the same area cut from a fully converted frame.
//!
//! The loops are compiled for AVX2 and SSE4.1 as well, picked at runtime, since conversion is
//! most of the CPU time capturing takes. Each loop handles one layout and one output row at a
//! time, which is what LLVM vectorizes well.

use autosplit_core::image::{BgraImage, Rect, bytes_needed};
use multiversion::multiversion;

const SHIFT: i32 = 20;
const CY: i32 = 1_220_542;
const CUB: i32 = 2_116_026;
const CUG: i32 = -409_993;
const CVG: i32 = -852_492;
const CVR: i32 = 1_673_527;
const HALF: i32 = 1 << (SHIFT - 1);

/// Chroma terms shared by the pixels of one U/V sample (`uvToRGBuv`).
#[derive(Clone, Copy)]
struct Chroma {
    r: i32,
    g: i32,
    b: i32,
}

impl Chroma {
    #[inline(always)]
    fn new(u: u8, v: u8) -> Self {
        let (u, v) = (u as i32 - 128, v as i32 - 128);
        Self { r: HALF + CVR * v, g: HALF + CVG * v + CUG * u, b: HALF + CUB * u }
    }

    /// `yRGBuvToRGBA`, written as BGRA.
    #[inline(always)]
    fn pixel(self, y: u8) -> [u8; 4] {
        let y = (y as i32 - 16).max(0) * CY;
        let channel = |uv: i32| ((y + uv) >> SHIFT).clamp(0, 255) as u8;
        [channel(self.b), channel(self.g), channel(self.r), 255]
    }
}

/// The BGRA color OpenCV turns a YUV pixel into.
pub fn yuv_to_bgra(y: u8, u: u8, v: u8) -> [u8; 4] {
    Chroma::new(u, v).pixel(y)
}

/// Layouts of 4:2:0 frames: a full size Y plane followed by quarter size chroma.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Yuv420 {
    /// Interleaved U, V after the Y plane.
    Nv12,
    /// Interleaved V, U after the Y plane.
    Nv21,
    /// U plane, then V plane (also called IYUV).
    I420,
    /// V plane, then U plane.
    Yv12,
}

/// Byte orders of packed 4:2:2 frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Yuv422 {
    /// Y0 U Y1 V (also called YUY2).
    Yuyv,
    /// U Y0 V Y1.
    Uyvy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    /// B, G, R. `bottom_up` frames store their last row first, like DirectShow's RGB24.
    Bgr24 {
        bottom_up: bool,
    },
    /// R, G, B (V4L2's RGB24).
    Rgb24,
    /// B, G, R and an unused byte.
    Bgrx,
    /// 8-bit grayscale.
    Gray,
    /// Rows `stride` apart; interleaved chroma rows `stride` apart too, separate U and V rows
    /// `stride / 2`.
    Yuv420(Yuv420),
    Yuv422(Yuv422),
    /// A JPEG per frame, always decoded whole: cropping would change pixels at the edges,
    /// where libjpeg-turbo's smooth upsampling reads neighbouring blocks. Linux only.
    Mjpeg,
}

impl PixelFormat {
    /// Bytes per pixel in the first plane, for a stride that isn't given. 0 for MJPEG.
    pub fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Bgr24 { .. } | Self::Rgb24 => 3,
            Self::Bgrx => 4,
            Self::Gray | Self::Yuv420(_) => 1,
            Self::Yuv422(_) => 2,
            Self::Mjpeg => 0,
        }
    }

    /// Whether converting `area` pixels right away is cheaper than copying the raw frame of
    /// `frame` pixels to convert later. Measured: copying runs at memory speed, YUV conversion
    /// costs about 8 times a copy per frame pixel, RGB about 2 times.
    fn converting_is_cheaper(self, area: usize, frame: usize) -> bool {
        match self {
            Self::Yuv420(_) | Self::Yuv422(_) => area <= frame / 8,
            Self::Bgr24 { .. } | Self::Rgb24 | Self::Bgrx | Self::Gray => area <= frame / 2,
            Self::Mjpeg => false,
        }
    }
}

/// What a conversion produced: the area of the frame it converted, and the frame's size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Converted {
    pub area: Rect,
    pub frame_size: (usize, usize),
}

/// How a frame's bytes are laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub format: PixelFormat,
    pub width: usize,
    pub height: usize,
    /// Bytes from one row to the next.
    pub stride: usize,
}

impl Layout {
    /// Tightly packed rows.
    pub fn packed(format: PixelFormat, width: usize, height: usize) -> Self {
        Self { format, width, height, stride: width * format.bytes_per_pixel() }
    }

    /// The bytes a whole frame takes.
    pub fn frame_len(&self) -> usize {
        let Self { format, width, height, stride } = *self;
        match format {
            PixelFormat::Yuv420(Yuv420::Nv12 | Yuv420::Nv21) => {
                stride * height + bytes_needed(height / 2, stride, width)
            }
            PixelFormat::Yuv420(Yuv420::I420 | Yuv420::Yv12) => {
                stride * height + (stride / 2) * (height / 2) + bytes_needed(height / 2, stride / 2, width / 2)
            }
            PixelFormat::Mjpeg => 0,
            _ => bytes_needed(height, stride, width * format.bytes_per_pixel()),
        }
    }

    /// Whether `area` is cheaper to convert right away than to copy for later.
    pub(crate) fn converting_is_cheaper(&self, area: Rect) -> bool {
        self.format.converting_is_cheaper(area.area(), self.width * self.height)
    }

    /// Converts `area` of the frame (grown to even coordinates for formats whose chroma covers
    /// 2 pixels), or all of it when `area` is empty or outside it. `None` when the frame is too
    /// short or has a size the format can't have.
    pub fn convert(&self, src: &[u8], area: Rect, dst: &mut BgraImage) -> Option<Converted> {
        let Self { format, width, height, stride } = *self;
        if format == PixelFormat::Mjpeg {
            return decode_jpeg(src, dst);
        }
        let even_width = matches!(format, PixelFormat::Yuv420(_) | PixelFormat::Yuv422(_));
        let even_height = matches!(format, PixelFormat::Yuv420(_));
        if (even_width && !width.is_multiple_of(2))
            || (even_height && !height.is_multiple_of(2))
            || stride < width * format.bytes_per_pixel()
            || src.len() < self.frame_len()
        {
            return None;
        }
        let area = clamp_area(area, width, height, even_width, even_height);
        match format {
            PixelFormat::Bgr24 { bottom_up: false } => rgb24_rows::<false, false>(src, height, stride, area, dst),
            PixelFormat::Bgr24 { bottom_up: true } => rgb24_rows::<true, false>(src, height, stride, area, dst),
            PixelFormat::Rgb24 => rgb24_rows::<false, true>(src, height, stride, area, dst),
            PixelFormat::Bgrx => bgrx_rows(src, stride, area, dst),
            PixelFormat::Gray => gray_rows(src, stride, area, dst),
            PixelFormat::Yuv420(layout) => {
                let (y_plane, chroma) = src.split_at(stride * height);
                let planes = Planes { y_plane, chroma, stride, plane_len: (stride / 2) * (height / 2) };
                match layout {
                    Yuv420::Nv12 => yuv420_interleaved::<false>(&planes, area, dst),
                    Yuv420::Nv21 => yuv420_interleaved::<true>(&planes, area, dst),
                    Yuv420::I420 => yuv420_planar::<false>(&planes, area, dst),
                    Yuv420::Yv12 => yuv420_planar::<true>(&planes, area, dst),
                }
            }
            PixelFormat::Yuv422(Yuv422::Yuyv) => yuv422_rows::<false>(src, stride, area, dst),
            PixelFormat::Yuv422(Yuv422::Uyvy) => yuv422_rows::<true>(src, stride, area, dst),
            PixelFormat::Mjpeg => unreachable!("decoded above"),
        }
        Some(Converted { area, frame_size: (width, height) })
    }

    /// The BGRA colors of the frame's 4 corners, without converting the rest. `None` for MJPEG,
    /// which has to be decoded whole.
    pub fn corners(&self, src: &[u8], scratch: &mut BgraImage) -> Option<[[u8; 4]; 4]> {
        if self.format == PixelFormat::Mjpeg || self.width == 0 || self.height == 0 {
            return None;
        }
        let (right, bottom) = (self.width - 1, self.height - 1);
        let mut corners = [[0; 4]; 4];
        for (corner, (x, y)) in corners.iter_mut().zip([(0, 0), (right, 0), (0, bottom), (right, bottom)]) {
            let pixel = Rect { left: x, top: y, right: x + 1, bottom: y + 1 };
            let converted = self.convert(src, pixel, scratch)?;
            let (dx, dy) = (x - converted.area.left, y - converted.area.top);
            let offset = (dy * converted.area.width() + dx) * 4;
            corner.copy_from_slice(&scratch.data()[offset..offset + 4]);
        }
        Some(corners)
    }
}

#[cfg(target_os = "linux")]
fn decode_jpeg(src: &[u8], dst: &mut BgraImage) -> Option<Converted> {
    crate::jpeg::decode_to_bgra(src, dst).ok()?;
    let (width, height) = (dst.width(), dst.height());
    Some(Converted { area: Rect::full(width, height), frame_size: (width, height) })
}

#[cfg(not(target_os = "linux"))]
fn decode_jpeg(_: &[u8], _: &mut BgraImage) -> Option<Converted> {
    None
}

/// The area to convert: `area` grown to even coordinates where the format needs them and kept
/// in the frame, or the whole frame when `area` is empty or outside it.
fn clamp_area(area: Rect, width: usize, height: usize, even_width: bool, even_height: bool) -> Rect {
    let full = Rect::full(width, height);
    if area.width() == 0 || area.height() == 0 || !full.contains(&area) {
        return full;
    }
    let down = |value: usize, even: bool| if even { value - value % 2 } else { value };
    let up = |value: usize, even: bool, limit: usize| if even { (value + value % 2).min(limit) } else { value };
    Rect {
        left: down(area.left, even_width),
        top: down(area.top, even_height),
        right: up(area.right, even_width, width),
        bottom: up(area.bottom, even_height, height),
    }
}

/// The `width` pixels of each output row.
fn output_rows(dst: &mut BgraImage, width: usize, height: usize) -> std::slice::ChunksExactMut<'_, [u8; 4]> {
    dst.reshape(width, height);
    dst.data_mut().as_chunks_mut::<4>().0.chunks_exact_mut(width.max(1))
}

struct Planes<'a> {
    y_plane: &'a [u8],
    chroma: &'a [u8],
    stride: usize,
    /// Offset of the second chroma plane, for separate planes.
    plane_len: usize,
}

/// NV12 (`SWAP` false) and NV21. `area` has even coordinates.
#[multiversion(targets("x86_64+avx2", "x86_64+sse4.1"))]
fn yuv420_interleaved<const SWAP: bool>(planes: &Planes, area: Rect, dst: &mut BgraImage) {
    let Planes { y_plane, chroma, stride, .. } = *planes;
    let (left, width) = (area.left, area.width());
    for (y, out) in (area.top..).zip(output_rows(dst, width, area.height())) {
        let luma = y_plane[y * stride + left..][..width].as_chunks::<2>().0;
        let pairs = chroma[(y / 2) * stride + left..][..width].as_chunks::<2>().0;
        for ((out, luma), uv) in out.as_chunks_mut::<2>().0.iter_mut().zip(luma).zip(pairs) {
            let chroma = if SWAP { Chroma::new(uv[1], uv[0]) } else { Chroma::new(uv[0], uv[1]) };
            *out = [chroma.pixel(luma[0]), chroma.pixel(luma[1])];
        }
    }
}

/// I420 (`SWAP` false) and YV12. `area` has even coordinates.
#[multiversion(targets("x86_64+avx2", "x86_64+sse4.1"))]
fn yuv420_planar<const SWAP: bool>(planes: &Planes, area: Rect, dst: &mut BgraImage) {
    let Planes { y_plane, chroma, stride, plane_len } = *planes;
    let (left, width) = (area.left, area.width());
    for (y, out) in (area.top..).zip(output_rows(dst, width, area.height())) {
        let luma = y_plane[y * stride + left..][..width].as_chunks::<2>().0;
        let row = (y / 2) * (stride / 2) + left / 2;
        let first = &chroma[row..][..width / 2];
        let second = &chroma[plane_len + row..][..width / 2];
        for ((out, luma), (&a, &b)) in out.as_chunks_mut::<2>().0.iter_mut().zip(luma).zip(first.iter().zip(second)) {
            let chroma = if SWAP { Chroma::new(b, a) } else { Chroma::new(a, b) };
            *out = [chroma.pixel(luma[0]), chroma.pixel(luma[1])];
        }
    }
}

/// YUYV (`UYVY` false) and UYVY. `area` has even horizontal coordinates.
#[multiversion(targets("x86_64+avx2", "x86_64+sse4.1"))]
fn yuv422_rows<const UYVY: bool>(src: &[u8], stride: usize, area: Rect, dst: &mut BgraImage) {
    for (y, out) in (area.top..).zip(output_rows(dst, area.width(), area.height())) {
        let groups = src[y * stride + area.left * 2..][..area.width() * 2].as_chunks::<4>().0;
        for (group, out) in groups.iter().zip(out.as_chunks_mut::<2>().0) {
            let (y0, y1, chroma) = if UYVY {
                (group[1], group[3], Chroma::new(group[0], group[2]))
            } else {
                (group[0], group[2], Chroma::new(group[1], group[3]))
            };
            *out = [chroma.pixel(y0), chroma.pixel(y1)];
        }
    }
}

/// 24-bit pixels; `RGB` is R, G, B order instead of B, G, R. `area` is counted from the top
/// even for `BOTTOM_UP` frames.
#[multiversion(targets("x86_64+avx2", "x86_64+sse4.1"))]
fn rgb24_rows<const BOTTOM_UP: bool, const RGB: bool>(
    src: &[u8],
    height: usize,
    stride: usize,
    area: Rect,
    dst: &mut BgraImage,
) {
    for (y, out) in (area.top..).zip(output_rows(dst, area.width(), area.height())) {
        let source_row = if BOTTOM_UP { height - 1 - y } else { y };
        let row = src[source_row * stride + area.left * 3..][..area.width() * 3].as_chunks::<3>().0;
        for (pixel, source) in out.iter_mut().zip(row) {
            *pixel = if RGB { [source[2], source[1], source[0], 255] } else { [source[0], source[1], source[2], 255] };
        }
    }
}

#[multiversion(targets("x86_64+avx2", "x86_64+sse4.1"))]
fn bgrx_rows(src: &[u8], stride: usize, area: Rect, dst: &mut BgraImage) {
    for (y, out) in (area.top..).zip(output_rows(dst, area.width(), area.height())) {
        let row = src[y * stride + area.left * 4..][..area.width() * 4].as_chunks::<4>().0;
        for (pixel, source) in out.iter_mut().zip(row) {
            *pixel = [source[0], source[1], source[2], 255];
        }
    }
}

/// The way `COLOR_GRAY2BGR` does it.
fn gray_rows(src: &[u8], stride: usize, area: Rect, dst: &mut BgraImage) {
    for (y, out) in (area.top..).zip(output_rows(dst, area.width(), area.height())) {
        for (pixel, &value) in out.iter_mut().zip(&src[y * stride + area.left..][..area.width()]) {
            *pixel = [value, value, value, 255];
        }
    }
}
