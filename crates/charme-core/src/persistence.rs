use std::{
    fs, io,
    path::{Path, PathBuf},
};

use ron::ser::PrettyConfig;
use thiserror::Error;

use crate::{CharmeDocument, DocumentValidationError, EditorError, EditorSession};

/// Serializes a validated document as readable RON.
pub fn document_to_ron(document: &CharmeDocument) -> Result<String, PersistenceError> {
    document.validate()?;
    ron::ser::to_string_pretty(document, PrettyConfig::default())
        .map_err(|error| PersistenceError::Serialize(error.to_string()))
}

/// Parses and validates a document from RON.
pub fn document_from_ron(source: &str) -> Result<CharmeDocument, PersistenceError> {
    let document: CharmeDocument =
        ron::from_str(source).map_err(|error| PersistenceError::Deserialize(error.to_string()))?;
    document.validate()?;
    Ok(document)
}

/// Saves a document to a project file.
pub fn save_document(path: &Path, document: &CharmeDocument) -> Result<(), PersistenceError> {
    let source = document_to_ron(document)?;
    let temporary = temporary_path(path);
    fs::write(&temporary, source).map_err(|source| PersistenceError::Io {
        path: temporary.clone(),
        source,
    })?;
    if let Err(source) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(PersistenceError::Io {
            path: path.to_path_buf(),
            source,
        });
    }
    Ok(())
}

/// Loads and validates a document from a project file.
pub fn load_document(path: &Path) -> Result<CharmeDocument, PersistenceError> {
    let source = fs::read_to_string(path).map_err(|source| PersistenceError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    document_from_ron(&source)
}

impl EditorSession {
    /// Opens a saved Charme project as a clean session.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, SessionPersistenceError> {
        let path = path.into();
        let document = load_document(&path)?;
        Self::from_document(document, Some(path)).map_err(SessionPersistenceError::Editor)
    }

    /// Saves to the current project path.
    pub fn save(&mut self) -> Result<(), SessionPersistenceError> {
        let path = self
            .project_path()
            .map(Path::to_path_buf)
            .ok_or(SessionPersistenceError::MissingProjectPath)?;
        save_document(&path, self.document())?;
        self.mark_saved(path);
        Ok(())
    }

    /// Saves to a new project path and makes it the current path.
    pub fn save_as(&mut self, path: impl Into<PathBuf>) -> Result<(), SessionPersistenceError> {
        let path = path.into();
        save_document(&path, self.document())?;
        self.mark_saved(path);
        Ok(())
    }
}

fn temporary_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "charme-project".to_owned());
    path.with_file_name(format!(".{file_name}.tmp"))
}

/// A serialization, validation, or file-system failure.
#[derive(Debug, Error)]
pub enum PersistenceError {
    /// Document serialization failed.
    #[error("failed to serialize Charme document: {0}")]
    Serialize(String),
    /// Document deserialization failed.
    #[error("failed to deserialize Charme document: {0}")]
    Deserialize(String),
    /// The loaded or saved document is structurally invalid.
    #[error(transparent)]
    InvalidDocument(#[from] DocumentValidationError),
    /// A file operation failed.
    #[error("file operation failed for {}: {source}", path.display())]
    Io {
        /// Path being accessed.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },
}

/// A project persistence failure at the editor-session layer.
#[derive(Debug, Error)]
pub enum SessionPersistenceError {
    /// `save` was requested before the project had a file path.
    #[error("the project has no path; use save_as first")]
    MissingProjectPath,
    /// Document or file persistence failed.
    #[error(transparent)]
    Persistence(#[from] PersistenceError),
    /// Session construction failed.
    #[error(transparent)]
    Editor(#[from] EditorError),
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::{
        EditorCommand, MaterialInstance, MaterialSlot, ParameterValue, ResourcePath, ShaderSource,
    };

    fn populated_session() -> EditorSession {
        let mut session = EditorSession::new("Paimon");
        let shader = ShaderSource::new(
            "Character Toon",
            ResourcePath::project_relative("shaders/character.wgsl").unwrap(),
        );
        let mut material = MaterialInstance::new("Face", shader.id());
        material
            .parameters
            .insert("face.threshold".to_owned(), ParameterValue::F32(0.25));
        let mut slot = MaterialSlot::new(0, "脸", "Face");
        slot.material = Some(material.id());
        session.apply(EditorCommand::UpsertShader(shader)).unwrap();
        session
            .apply(EditorCommand::UpsertMaterial(material))
            .unwrap();
        session
            .apply(EditorCommand::ReplaceMaterialSlots(vec![slot]))
            .unwrap();
        session
    }

    #[test]
    fn ron_round_trip_preserves_ids_paths_and_values() {
        let session = populated_session();
        let encoded = document_to_ron(session.document()).unwrap();
        let decoded = document_from_ron(&encoded).unwrap();

        assert_eq!(&decoded, session.document());
        assert!(encoded.contains("face.threshold"));
        assert!(encoded.contains("shaders/character.wgsl"));
    }

    #[test]
    fn save_as_and_open_update_session_state() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("charme_core_{unique}.charme.ron"));
        let mut session = populated_session();
        assert!(session.is_dirty());

        session.save_as(&path).unwrap();
        assert!(!session.is_dirty());
        assert_eq!(session.project_path(), Some(path.as_path()));

        let opened = EditorSession::open(&path).unwrap();
        assert!(!opened.is_dirty());
        assert_eq!(opened.document(), session.document());
        fs::remove_file(path).unwrap();
    }
}
