use std::path::Path;

use image::ImageEncoder;

/// 8-bit image in BGRA channel order, rows tightly packed.
///
/// BGRA matches the cv2 Mats AutoSplit works with. The order matters: the histogram
/// method bins channels as R, G, B, and the L2 norm includes the alpha channel.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BgraImage {
    width: usize,
    height: usize,
    data: Vec<u8>,
}

impl BgraImage {
    /// Panics if `data` is not exactly `width * height * 4` bytes.
    pub fn from_raw(width: usize, height: usize, data: Vec<u8>) -> Self {
        assert_eq!(data.len(), width * height * 4, "BGRA buffer size mismatch");
        Self { width, height, data }
    }

    /// An opaque image of one color, given as RGB like `#rrggbb`.
    pub fn solid([red, green, blue]: [u8; 3], width: usize, height: usize) -> Self {
        Self::from_raw(width, height, [blue, green, red, 255].repeat(width * height))
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn data_mut(&mut self) -> &mut [u8] {
        &mut self.data
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn view(&self) -> BgraView<'_> {
        BgraView { data: &self.data, width: self.width, height: self.height, stride: self.width * 4 }
    }

    /// Resizes in place, reusing the allocation. Contents are unspecified afterwards.
    pub fn reshape(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
        self.data.resize(width * height * 4, 0);
    }

    /// Decodes a PNG. Only PNG is supported for split images.
    ///
    /// Images without an alpha channel get alpha 255, which is what AutoSplit ends up with too
    /// (it adds an opaque alpha channel after resizing). Unlike AutoSplit, grayscale and 16-bit
    /// images are converted instead of crashing.
    pub fn decode_png(bytes: &[u8]) -> Result<Self, image::ImageError> {
        let rgba = image::load_from_memory_with_format(bytes, image::ImageFormat::Png)?.into_rgba8();
        let (width, height) = (rgba.width() as usize, rgba.height() as usize);
        let mut data = rgba.into_raw();
        for pixel in data.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        Ok(Self { width, height, data })
    }

    pub fn open_png(path: &Path) -> Result<Self, image::ImageError> {
        Self::decode_png(&std::fs::read(path).map_err(image::ImageError::IoError)?)
    }

    pub fn is_png(bytes: &[u8]) -> bool {
        bytes.starts_with(b"\x89PNG\r\n\x1a\n")
    }

    /// Encodes as an RGBA PNG, like `cv2.imwrite` does for a 4-channel Mat.
    pub fn encode_png(&self) -> Result<Vec<u8>, image::ImageError> {
        let mut rgba = self.data.clone();
        for pixel in rgba.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }
        let mut out = Vec::new();
        image::codecs::png::PngEncoder::new(&mut out).write_image(
            &rgba,
            self.width as u32,
            self.height as u32,
            image::ExtendedColorType::Rgba8,
        )?;
        Ok(out)
    }
}

/// A borrowed BGRA frame whose rows may be padded, the way capture APIs hand frames out.
/// Comparisons read captures through this, so frames don't have to be copied first.
#[derive(Clone, Copy, Debug)]
pub struct BgraView<'a> {
    data: &'a [u8],
    width: usize,
    height: usize,
    /// Bytes from the start of one row to the start of the next.
    stride: usize,
}

impl<'a> BgraView<'a> {
    /// `None` if `stride` is shorter than a row or `data` is too short for the last row.
    pub fn new(data: &'a [u8], width: usize, height: usize, stride: usize) -> Option<Self> {
        (stride >= width * 4 && data.len() >= bytes_needed(height, stride, width * 4)).then_some(Self {
            data,
            width,
            height,
            stride,
        })
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// The `width * 4` bytes of row `y`, without padding.
    pub fn row(&self, y: usize) -> &'a [u8] {
        &self.data[y * self.stride..y * self.stride + self.width * 4]
    }

    /// Same as numpy slicing `image[top:bottom, left:right]`: out of range bounds are clamped.
    pub fn crop(&self, left: usize, right: usize, top: usize, bottom: usize) -> BgraView<'a> {
        let right = right.min(self.width);
        let bottom = bottom.min(self.height);
        let left = left.min(right);
        let top = top.min(bottom);
        let (width, height) = (right - left, bottom - top);
        let start = top * self.stride + left * 4;
        let data =
            if height == 0 { &[][..] } else { &self.data[start..start + self.stride * (height - 1) + width * 4] };
        BgraView { data, width, height, stride: self.stride }
    }

    pub fn crop_rect(&self, rect: Rect) -> BgraView<'a> {
        self.crop(rect.left, rect.right, rect.top, rect.bottom)
    }

    /// Copies into a tightly packed image.
    pub fn to_image(&self) -> BgraImage {
        let mut data = Vec::with_capacity(self.width * self.height * 4);
        for y in 0..self.height {
            data.extend_from_slice(self.row(y));
        }
        BgraImage::from_raw(self.width, self.height, data)
    }
}

/// Bytes that `rows` rows `stride` apart take, with `row_bytes` used in the last one.
pub fn bytes_needed(rows: usize, stride: usize, row_bytes: usize) -> usize {
    if rows == 0 { 0 } else { stride * (rows - 1) + row_bytes }
}

/// A rectangle of pixels, `right` and `bottom` exclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub left: usize,
    pub top: usize,
    pub right: usize,
    pub bottom: usize,
}

impl Rect {
    pub fn full(width: usize, height: usize) -> Self {
        Self { left: 0, top: 0, right: width, bottom: height }
    }

    pub fn width(&self) -> usize {
        self.right - self.left
    }

    pub fn height(&self) -> usize {
        self.bottom - self.top
    }

    pub fn area(&self) -> usize {
        self.width() * self.height()
    }

    pub fn contains(&self, other: &Rect) -> bool {
        self.left <= other.left && self.top <= other.top && other.right <= self.right && other.bottom <= self.bottom
    }
}
