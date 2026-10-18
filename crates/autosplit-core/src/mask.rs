//! The masks of split image presets: which part of a screenshot a split image keeps. Matches
//! AutoMask: the mask is stretched to the screenshot's size, and only pixels where it's fully
//! opaque are kept.

use std::path::Path;

use crate::image::{BgraImage, BgraView};
use crate::imgproc::{ResizeBuffers, resize_linear_exact};

/// A mask's alpha, which is all that matters of it. A PNG without transparency keeps everything.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mask {
    width: usize,
    height: usize,
    alpha: Vec<u8>,
}

impl Mask {
    pub fn open(path: &Path) -> Result<Self, image::ImageError> {
        Ok(Self::from_image(&BgraImage::open_png(path)?))
    }

    pub fn from_image(image: &BgraImage) -> Self {
        let alpha = image.data().as_chunks::<4>().0.iter().map(|pixel| pixel[3]).collect();
        Self { width: image.width(), height: image.height(), alpha }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    /// The alpha stretched to `width` by `height`, bilinearly like AutoMask.
    pub fn scaled(&self, width: usize, height: usize, buffers: &mut ResizeBuffers) -> Vec<u8> {
        let mut alpha = vec![0; width * height];
        if !self.alpha.is_empty() && !alpha.is_empty() {
            resize_linear_exact(&self.alpha, self.width, self.height, 1, width, height, buffers, &mut alpha);
        }
        alpha
    }
}

/// The split image `mask` makes of `shot`: its pixels where the mask, stretched to the shot's
/// size, is fully opaque, and transparent pixels elsewhere.
pub fn apply_mask(shot: BgraView, mask: &Mask, buffers: &mut ResizeBuffers) -> BgraImage {
    let (width, height) = (shot.width(), shot.height());
    let alpha = mask.scaled(width, height, buffers);
    let mut data = vec![0; width * height * 4];
    let rows = data.chunks_exact_mut(width * 4).zip(alpha.chunks_exact(width.max(1)));
    for (y, (out, alpha)) in rows.enumerate() {
        let pixels = out.as_chunks_mut::<4>().0.iter_mut().zip(shot.row(y).as_chunks::<4>().0).zip(alpha);
        for ((out, shot), &alpha) in pixels {
            if alpha == 255 {
                *out = [shot[0], shot[1], shot[2], 255];
            }
        }
    }
    BgraImage::from_raw(width, height, data)
}

/// A fade split's split image: `color` (RGB) where `mask` is fully opaque, at the mask's size, and
/// transparent elsewhere. Without a mask, one pixel of it, which is stretched to the whole capture.
pub fn fade_image(color: [u8; 3], mask: Option<&Mask>) -> BgraImage {
    let Some(mask) = mask else { return BgraImage::solid(color, 1, 1) };
    apply_mask(BgraImage::solid(color, mask.width, mask.height).view(), mask, &mut ResizeBuffers::default())
}
