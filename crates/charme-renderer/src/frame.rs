use std::sync::Arc;

use crate::{OutputSize, PixelFormat};

/// An immutable CPU image produced by the renderer.
///
/// Pixels start at the top-left corner and rows proceed from top to bottom. The
/// alpha channel is preserved from the render target. `bytes_per_row` may include
/// padding and must be used instead of assuming tightly packed rows.
#[derive(Debug, Clone)]
pub struct Frame {
    sequence: u64,
    size: OutputSize,
    pixel_format: PixelFormat,
    bytes_per_row: usize,
    pixels: Arc<[u8]>,
}

impl Frame {
    pub(crate) fn new(
        sequence: u64,
        size: OutputSize,
        pixel_format: PixelFormat,
        bytes_per_row: usize,
        pixels: Vec<u8>,
    ) -> Self {
        Self {
            sequence,
            size,
            pixel_format,
            bytes_per_row,
            pixels: pixels.into(),
        }
    }

    /// Returns the monotonically increasing frame sequence number.
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// Returns the frame dimensions in physical pixels.
    pub const fn size(&self) -> OutputSize {
        self.size
    }

    /// Returns the frame width in physical pixels.
    pub const fn width(&self) -> u32 {
        self.size.width
    }

    /// Returns the frame height in physical pixels.
    pub const fn height(&self) -> u32 {
        self.size.height
    }

    /// Returns the byte order and transfer function of the pixels.
    pub const fn pixel_format(&self) -> PixelFormat {
        self.pixel_format
    }

    /// Returns the number of bytes between the start of adjacent rows.
    pub const fn bytes_per_row(&self) -> usize {
        self.bytes_per_row
    }

    /// Borrows the complete image storage, including row padding.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// Consumes the frame and returns its shared image storage.
    pub fn into_pixels(self) -> Arc<[u8]> {
        self.pixels
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_reports_layout() {
        let frame = Frame::new(
            7,
            OutputSize::new(3, 2),
            PixelFormat::Bgra8Srgb,
            256,
            vec![0; 512],
        );

        assert_eq!(frame.sequence(), 7);
        assert_eq!(frame.width(), 3);
        assert_eq!(frame.height(), 2);
        assert_eq!(frame.bytes_per_row(), 256);
        assert_eq!(frame.pixels().len(), 512);
    }
}
