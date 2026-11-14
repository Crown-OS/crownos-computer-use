//! Pixels as the compositor hands them, to PNG.

use wayland_client::protocol::wl_shm::Format;

use crate::Picture;
use crate::error::WaylandError;

/// The shm formats a capture is accepted in, most convenient first.
pub(crate) const ACCEPTED: &[Format] = &[
    Format::Abgr8888,
    Format::Xbgr8888,
    Format::Argb8888,
    Format::Xrgb8888,
];

/// Converts one tightly packed frame of `format` to RGBA.
pub(crate) fn to_rgba(
    format: Format,
    width: u32,
    height: u32,
    stride: u32,
    data: &[u8],
) -> Picture {
    let width_bytes = (width as usize) * 4;
    let mut rgba = Vec::with_capacity(width_bytes * height as usize);
    for row in 0..height as usize {
        let start = row * stride as usize;
        let Some(line) = data.get(start..start + width_bytes) else {
            break;
        };
        for pixel in line.chunks_exact(4) {
            let (Some(b0), Some(b1), Some(b2), Some(b3)) =
                (pixel.first(), pixel.get(1), pixel.get(2), pixel.get(3))
            else {
                continue;
            };
            match format {
                Format::Abgr8888 => rgba.extend([*b0, *b1, *b2, *b3]),
                Format::Xbgr8888 => rgba.extend([*b0, *b1, *b2, 255]),
                Format::Argb8888 => rgba.extend([*b2, *b1, *b0, *b3]),
                _ => rgba.extend([*b2, *b1, *b0, 255]),
            }
        }
    }
    Picture {
        width,
        height,
        rgba,
    }
}

impl Picture {
    /// # Errors
    ///
    /// Fails only when the encoder refuses the dimensions.
    pub fn to_png(&self) -> Result<Vec<u8>, WaylandError> {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()?;
        writer.write_image_data(&self.rgba)?;
        writer.finish()?;
        Ok(bytes)
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::*;

    #[test]
    fn xrgb_little_endian_becomes_rgba_and_encodes() {
        let data = [0x11u8, 0x22, 0x33, 0x00, 0, 0, 0, 0];
        let picture = to_rgba(Format::Xrgb8888, 1, 1, 8, &data);
        assert_eq!(picture.rgba, vec![0x33, 0x22, 0x11, 255]);
        let png = picture.to_png().unwrap();
        assert_eq!(&png[1..4], b"PNG");
    }
}
