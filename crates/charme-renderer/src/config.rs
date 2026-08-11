/// The dimensions, in physical pixels, of rendered frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OutputSize {
    /// Frame width in physical pixels.
    pub width: u32,
    /// Frame height in physical pixels.
    pub height: u32,
}

impl OutputSize {
    /// Creates a new output size in physical pixels.
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub(crate) const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }
}

/// The byte order and transfer function used by a rendered frame.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PixelFormat {
    /// Four 8-bit channels in blue, green, red, alpha byte order using sRGB.
    Bgra8Srgb,
    /// Four 8-bit channels in red, green, blue, alpha byte order using sRGB.
    Rgba8Srgb,
}

/// The opaque color used to clear the rendered image.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BackgroundColor {
    /// Linear red component in the inclusive range `0.0..=1.0`.
    pub red: f32,
    /// Linear green component in the inclusive range `0.0..=1.0`.
    pub green: f32,
    /// Linear blue component in the inclusive range `0.0..=1.0`.
    pub blue: f32,
}

impl BackgroundColor {
    /// Black.
    pub const BLACK: Self = Self::rgb(0.0, 0.0, 0.0);
    /// White.
    pub const WHITE: Self = Self::rgb(1.0, 1.0, 1.0);

    /// Creates an opaque RGB clear color.
    pub const fn rgb(red: f32, green: f32, blue: f32) -> Self {
        Self { red, green, blue }
    }

    pub(crate) fn is_valid(self) -> bool {
        [self.red, self.green, self.blue]
            .into_iter()
            .all(|component| component.is_finite() && (0.0..=1.0).contains(&component))
    }
}

/// Configuration used to create a [`Renderer`](crate::Renderer).
#[derive(Debug, Clone, PartialEq)]
pub struct RendererConfig {
    pub(crate) output_size: OutputSize,
    pub(crate) pixel_format: PixelFormat,
    pub(crate) background: BackgroundColor,
}

impl RendererConfig {
    /// Creates a configuration with the requested physical-pixel dimensions.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            output_size: OutputSize::new(width, height),
            ..Self::default()
        }
    }

    /// Selects the output pixel format.
    pub fn pixel_format(mut self, pixel_format: PixelFormat) -> Self {
        self.pixel_format = pixel_format;
        self
    }

    /// Selects the opaque clear color.
    pub fn background(mut self, background: BackgroundColor) -> Self {
        self.background = background;
        self
    }

    /// Returns the configured output size.
    pub const fn output_size(&self) -> OutputSize {
        self.output_size
    }

    /// Returns the configured pixel format.
    pub const fn selected_pixel_format(&self) -> PixelFormat {
        self.pixel_format
    }

    /// Returns the configured clear color.
    pub const fn selected_background(&self) -> BackgroundColor {
        self.background
    }
}

impl Default for RendererConfig {
    fn default() -> Self {
        Self {
            output_size: OutputSize::new(1280, 720),
            pixel_format: PixelFormat::Bgra8Srgb,
            background: BackgroundColor::rgb(0.15, 0.15, 0.18),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_preserves_public_configuration() {
        let config = RendererConfig::new(640, 480)
            .pixel_format(PixelFormat::Rgba8Srgb)
            .background(BackgroundColor::WHITE);

        assert_eq!(config.output_size(), OutputSize::new(640, 480));
        assert_eq!(config.selected_pixel_format(), PixelFormat::Rgba8Srgb);
        assert_eq!(config.selected_background(), BackgroundColor::WHITE);
    }

    #[test]
    fn either_zero_dimension_is_empty() {
        assert!(OutputSize::new(0, 100).is_empty());
        assert!(OutputSize::new(100, 0).is_empty());
        assert!(!OutputSize::new(1, 1).is_empty());
    }
}
