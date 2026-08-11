use std::sync::Arc;

use cacao::{
    foundation::id,
    image::Image,
    objc::{class, msg_send, sel, sel_impl},
};
use charme_renderer::{Frame, PixelFormat};
use core_graphics::{
    color_space::{CGColorSpace, kCGColorSpaceSRGB},
    data_provider::CGDataProvider,
    geometry::CGSize,
    image::{CGImage, CGImageAlphaInfo, CGImageByteOrderInfo},
};
use foreign_types::ForeignType;

pub(crate) fn make_image(frame: Frame, scale: f64) -> Result<Image, String> {
    if frame.pixel_format() != PixelFormat::Bgra8Srgb {
        return Err("the cacao adapter currently requires BGRA8 sRGB frames".to_owned());
    }

    let width = frame.width() as usize;
    let height = frame.height() as usize;
    let bytes_per_row = frame.bytes_per_row();
    let pixels = frame.into_pixels();
    let provider = CGDataProvider::from_buffer(Arc::new(pixels));
    let color_space = unsafe { CGColorSpace::create_with_name(kCGColorSpaceSRGB) }
        .ok_or_else(|| "the sRGB color space is unavailable".to_owned())?;
    let bitmap_info = CGImageAlphaInfo::CGImageAlphaPremultipliedFirst as u32
        | CGImageByteOrderInfo::CGImageByteOrder32Little as u32;
    let cg_image = CGImage::new(
        width,
        height,
        8,
        32,
        bytes_per_row,
        &color_space,
        bitmap_info,
        &provider,
        true,
        0,
    );

    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let logical_size = CGSize::new(width as f64 / scale, height as f64 / scale);

    // SAFETY: `cg_image` is valid for this call and NSImage retains the image
    // representation created by the designated initializer. `init` returns an
    // owned Objective-C object, which is transferred to cacao's Image wrapper.
    let image = unsafe {
        let allocated: id = msg_send![class!(NSImage), alloc];
        let image: id = msg_send![allocated,
            initWithCGImage: cg_image.as_ptr()
            size: logical_size
        ];
        if image.is_null() {
            return Err("AppKit could not create an image from the rendered frame".to_owned());
        }
        Image::with(image)
    };

    Ok(image)
}
