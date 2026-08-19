//! Pure mesh-topology algorithms used by Charme.
//!
//! The crate deliberately does not depend on Bevy, PMX, or a renderer. It
//! operates on indexed triangle ranges so callers can preserve their own
//! vertex attributes and material metadata.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod components;

pub use components::{
    Connectivity, MeshComponent, PrimitiveRange, PrimitiveSplit, SplitError, split_primitive,
    split_primitive_with_connectivity,
};
