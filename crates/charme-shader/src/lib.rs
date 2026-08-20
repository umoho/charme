//! WGSL composition and reflection for Charme material interfaces.
//!
//! The crate composes naga-oil modules, validates and lays out the resulting
//! Naga IR, associates `%{ ... }` doc-comment metadata with declarations, and
//! packs reflected uniform values into host bytes.

#![forbid(unsafe_code)]

mod composer;
mod metadata;
mod packer;
mod reflection;
mod scanner;

pub use charme_core::ParameterValue;
pub use composer::{
    ComposeStage, ShaderComposeError, ShaderComposer, ShaderDefValue, ShaderDefs, ShaderSource,
};
pub use metadata::{
    MetadataBlock, MetadataEntry, MetadataParseError, MetadataPath, MetadataValue,
    parse_metadata_block,
};
pub use packer::{ParameterBuffer, ParameterWriteError};
pub use reflection::{
    EntryPoint, InterfaceDiagnostic, InterfaceDiagnosticKind, ParameterBlock, ParameterField,
    ParameterType, ReflectError, ReflectStage, ReflectedMetadata, Resource, ResourceKind,
    ResourceUse, ScalarType, ShaderInterface, ShaderStage, TextureClass, TextureDimension,
};
pub use scanner::{
    DeclarationMetadata, ModuleMetadata, ScanDiagnostic, ScanDiagnosticKind, SourceDeclaration,
    scan_module_metadata,
};
