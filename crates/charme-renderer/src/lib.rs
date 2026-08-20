//! An opaque, windowless renderer for embedding frames in GUI applications.
//!
//! The public API deliberately exposes only renderer-domain types. Rendering
//! backend types and lifecycle details remain private implementation details.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod archive;
mod backend;
mod config;
mod error;
mod frame;
mod overlay;
mod pmx_import;
mod renderer;
mod scene;
mod scene_runtime;
mod scheduler;
mod selection;
mod selection_wire;
mod source;

pub use archive::discover_pmx_archive_entries;
pub use config::{BackgroundColor, OutputSize, PixelFormat, RendererConfig};
pub use error::RendererError;
pub use frame::Frame;
pub use renderer::{
    PmxLoadProgress, PmxLoadStage, Renderer, RendererNotification, ViewportSelectionAction,
};
pub use scene::{PmxMaterialSlot, PmxPrimitiveComponentInfo, PmxPrimitiveInfo, PmxSceneInfo};
pub use source::{PmxLoadRequest, PmxSourceIdentity};
