//! MJPEG frames, decoded with libjpeg-turbo like OpenCV's `imdecode` (accurate integer DCT,
//! smooth chroma upsampling), so the pixels match AutoSplit on Linux.

use autosplit_core::image::BgraImage;

pub fn decode_to_bgra(jpeg: &[u8], dst: &mut BgraImage) -> Result<(), turbojpeg::Error> {
    let mut decompressor = turbojpeg::Decompressor::new()?;
    let header = decompressor.read_header(jpeg)?;
    dst.reshape(header.width, header.height);
    let image = turbojpeg::Image {
        pixels: dst.data_mut(),
        width: header.width,
        pitch: header.width * 4,
        height: header.height,
        format: turbojpeg::PixelFormat::BGRA,
    };
    decompressor.decompress(jpeg, image)
}
