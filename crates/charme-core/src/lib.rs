//! UI- and renderer-independent domain model for Charme.

#![forbid(unsafe_code)]

mod document;
mod id;
mod persistence;
mod resource;
mod session;
mod value;

pub use document::{
    CURRENT_DOCUMENT_VERSION, CharacterFormat, CharacterSource, CharmeDocument,
    DocumentValidationError, MaterialAlphaMode, MaterialInstance, MaterialRenderState,
    MaterialSlot, ShaderSource,
};
pub use id::{DocumentId, MaterialId, MaterialSlotId, ShaderId};
pub use persistence::{
    PersistenceError, SessionPersistenceError, document_from_ron, document_to_ron, load_document,
    save_document,
};
pub use resource::{ResourcePath, ResourcePathError};
pub use session::{
    DocumentChange, EditorCommand, EditorError, EditorEvent, EditorSession, EditorSnapshot,
};
pub use value::{ParameterValue, ParameterValueKind};
