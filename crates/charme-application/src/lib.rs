//! Platform-independent application orchestration for Charme frontends.
//!
//! This crate sits between native views and the domain model. It exposes
//! actions, a presentation-oriented snapshot, and a controller that owns the
//! editor session without exposing any platform UI types.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

use std::path::{Path, PathBuf};

use charme_core::{
    CharmeDocument, EditorCommand, EditorError, EditorEvent, EditorSession, EditorSnapshot,
    SessionPersistenceError,
};
use thiserror::Error;

mod shader;

pub use shader::{
    ParameterControlKind, ParameterControlSpec, ShaderInspection, inspect_shader_source,
};

/// An action that can be issued by any native frontend.
#[derive(Clone, Debug, PartialEq)]
pub enum EditorAction {
    /// Applies one semantic document command.
    Command(EditorCommand),
    /// Undoes the latest document change.
    Undo,
    /// Redoes the latest undone document change.
    Redo,
}

/// A presentation-oriented projection of the editor state.
///
/// Native frontends should use this projection for common application state
/// instead of reaching into [`EditorSession`] for menu and status decisions.
#[derive(Clone, Debug, PartialEq)]
pub struct EditorViewModel {
    /// Current project display name.
    pub document_name: String,
    /// Current project path, if the project has been saved or opened.
    pub project_path: Option<PathBuf>,
    /// Current document revision.
    pub revision: u64,
    /// Whether the document differs from its saved state.
    pub dirty: bool,
    /// Whether Undo is currently available.
    pub can_undo: bool,
    /// Whether Redo is currently available.
    pub can_redo: bool,
    /// Number of shaders in the document.
    pub shader_count: usize,
    /// Number of materials in the document.
    pub material_count: usize,
    /// Number of imported material slots in the document.
    pub material_slot_count: usize,
}

/// An application-level failure while executing an editor operation.
#[derive(Debug, Error)]
pub enum EditorControllerError {
    /// A document command or history operation was rejected.
    #[error(transparent)]
    Editor(#[from] EditorError),
    /// Project persistence failed.
    #[error(transparent)]
    Persistence(#[from] SessionPersistenceError),
}

/// Platform-independent coordinator for editor actions and state.
#[derive(Debug)]
pub struct EditorController {
    session: EditorSession,
}

impl EditorController {
    /// Creates a controller around a new unsaved project.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            session: EditorSession::new(name),
        }
    }

    /// Creates a controller around an existing validated session.
    pub fn from_session(session: EditorSession) -> Self {
        Self { session }
    }

    /// Opens a saved project.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, EditorControllerError> {
        Ok(Self {
            session: EditorSession::open(path)?,
        })
    }

    /// Returns the current domain document.
    pub fn document(&self) -> &CharmeDocument {
        self.session.document()
    }

    /// Returns the current project path.
    pub fn project_path(&self) -> Option<&Path> {
        self.session.project_path()
    }

    /// Returns a domain snapshot for consumers that need the complete state.
    pub fn snapshot(&self) -> EditorSnapshot {
        self.session.snapshot()
    }

    /// Returns the presentation projection used by native frontends.
    pub fn view_model(&self) -> EditorViewModel {
        let document = self.session.document();
        EditorViewModel {
            document_name: document.name().to_owned(),
            project_path: self.project_path().map(Path::to_path_buf),
            revision: self.session.revision(),
            dirty: self.session.is_dirty(),
            can_undo: self.session.can_undo(),
            can_redo: self.session.can_redo(),
            shader_count: document.shaders().len(),
            material_count: document.materials().len(),
            material_slot_count: document.material_slots().len(),
        }
    }

    /// Executes an action and returns the resulting document event, if any.
    pub fn dispatch(
        &mut self,
        action: EditorAction,
    ) -> Result<Option<EditorEvent>, EditorControllerError> {
        match action {
            EditorAction::Command(command) => self.apply(command),
            EditorAction::Undo => Ok(self.session.undo()?),
            EditorAction::Redo => Ok(self.session.redo()?),
        }
    }

    /// Applies one semantic document command.
    pub fn apply(
        &mut self,
        command: EditorCommand,
    ) -> Result<Option<EditorEvent>, EditorControllerError> {
        Ok(self.session.apply(command)?)
    }

    /// Undoes the latest document change.
    pub fn undo(&mut self) -> Result<Option<EditorEvent>, EditorControllerError> {
        Ok(self.session.undo()?)
    }

    /// Redoes the latest undone document change.
    pub fn redo(&mut self) -> Result<Option<EditorEvent>, EditorControllerError> {
        Ok(self.session.redo()?)
    }

    /// Saves the project to its current path.
    pub fn save(&mut self) -> Result<(), EditorControllerError> {
        Ok(self.session.save()?)
    }

    /// Saves the project to a new path.
    pub fn save_as(&mut self, path: impl Into<PathBuf>) -> Result<(), EditorControllerError> {
        Ok(self.session.save_as(path)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use charme_core::EditorCommand;

    #[test]
    fn view_model_projects_domain_state_for_native_frontends() {
        let mut controller = EditorController::new("Untitled");
        assert_eq!(controller.view_model().document_name, "Untitled");
        assert!(!controller.view_model().dirty);
        assert!(!controller.view_model().can_undo);

        controller
            .dispatch(EditorAction::Command(EditorCommand::RenameDocument(
                "Updated".to_owned(),
            )))
            .unwrap();

        let view_model = controller.view_model();
        assert_eq!(view_model.document_name, "Updated");
        assert!(view_model.dirty);
        assert!(view_model.can_undo);
    }

    #[test]
    fn actions_share_core_undo_semantics() {
        let mut controller = EditorController::new("Untitled");
        controller
            .dispatch(EditorAction::Command(EditorCommand::RenameDocument(
                "Updated".to_owned(),
            )))
            .unwrap();
        controller.dispatch(EditorAction::Undo).unwrap();

        assert_eq!(controller.document().name(), "Untitled");
        assert!(controller.view_model().can_redo);
    }
}
