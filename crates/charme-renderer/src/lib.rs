//! An opaque, windowless renderer for embedding frames in GUI applications.
//!
//! The public API deliberately exposes only renderer-domain types. Rendering
//! backend types and lifecycle details remain private implementation details.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod backend;
mod config;
mod error;
mod frame;
mod renderer;

pub use config::{BackgroundColor, OutputSize, PixelFormat, RendererConfig};
pub use error::RendererError;
pub use frame::Frame;
pub use renderer::Renderer;
