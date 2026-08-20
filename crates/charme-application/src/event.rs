use std::path::PathBuf;

use crate::{EditorUpdate, ShaderInspection};

/// An asynchronous result delivered to a native frontend.
///
/// The event contains no widget or window-system types. Platform adapters may
/// attach local display information when translating it to native messages.
#[non_exhaustive]
#[derive(Debug)]
pub enum ApplicationEvent {
    /// The editor state changed after an action.
    EditorUpdated(EditorUpdate),
    /// A background shader inspection completed.
    ShaderInspected {
        /// Inspected shader path.
        path: PathBuf,
        /// Inspection result.
        result: Result<ShaderInspection, String>,
    },
    /// A user-visible application failure.
    Failed(String),
}
