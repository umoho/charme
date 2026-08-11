use std::{collections::BTreeMap, path::PathBuf};

use thiserror::Error;

use crate::{
    CharacterSource, CharmeDocument, DocumentValidationError, MaterialId, MaterialInstance,
    MaterialRenderState, MaterialSlot, MaterialSlotId, ParameterValue, ResourcePath, ShaderId,
    ShaderSource,
};

/// A semantic edit that can be issued by any platform UI.
#[derive(Clone, Debug, PartialEq)]
pub enum EditorCommand {
    /// Changes the project display name.
    RenameDocument(String),
    /// Selects or clears the character source.
    SetCharacter(Option<CharacterSource>),
    /// Adds a shader or replaces the shader with the same ID.
    UpsertShader(ShaderSource),
    /// Removes an unreferenced shader.
    RemoveShader(ShaderId),
    /// Changes a shader's display name.
    RenameShader {
        /// Shader to edit.
        shader: ShaderId,
        /// New display name.
        name: String,
    },
    /// Changes the WGSL root path for a shader.
    SetShaderPath {
        /// Shader to edit.
        shader: ShaderId,
        /// New WGSL path.
        path: ResourcePath,
    },
    /// Adds a material or replaces the material with the same ID.
    UpsertMaterial(MaterialInstance),
    /// Removes an unbound material.
    RemoveMaterial(MaterialId),
    /// Changes a material's display name.
    RenameMaterial {
        /// Material to edit.
        material: MaterialId,
        /// New display name.
        name: String,
    },
    /// Changes the shader used by a material.
    SetMaterialShader {
        /// Material to edit.
        material: MaterialId,
        /// New shader.
        shader: ShaderId,
    },
    /// Replaces material slots after importing or reimporting a character.
    ReplaceMaterialSlots(Vec<MaterialSlot>),
    /// Assigns or clears the material on one imported slot.
    BindMaterial {
        /// Slot to edit.
        slot: MaterialSlotId,
        /// New material, or `None` to clear the assignment.
        material: Option<MaterialId>,
    },
    /// Sets or removes one reflected material parameter.
    SetMaterialParameter {
        /// Material to edit.
        material: MaterialId,
        /// Dot-separated reflected field path.
        path: String,
        /// New value, or `None` to remove an override.
        value: Option<ParameterValue>,
    },
    /// Sets or removes one reflected texture binding.
    SetMaterialTexture {
        /// Material to edit.
        material: MaterialId,
        /// Reflected resource path.
        path: String,
        /// New texture, or `None` to clear the binding.
        texture: Option<ResourcePath>,
    },
    /// Replaces one material's rasterization and blending state.
    SetMaterialRenderState {
        /// Material to edit.
        material: MaterialId,
        /// New state.
        state: MaterialRenderState,
    },
}

/// Broad category of an applied document change.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DocumentChange {
    /// The entire document may have changed, for example after Undo/Redo.
    Everything,
    /// Project metadata changed.
    Metadata,
    /// Character source changed.
    Character,
    /// Shader collection changed.
    Shaders,
    /// Material collection or material values changed.
    Materials,
    /// Character material slots or their assignments changed.
    MaterialSlots,
}

/// An event emitted after a command changes the document.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EditorEvent {
    /// Monotonically increasing document revision.
    pub revision: u64,
    /// Category that consumers should refresh.
    pub change: DocumentChange,
}

/// Immutable state copied for consumption by a native UI.
#[derive(Clone, Debug, PartialEq)]
pub struct EditorSnapshot {
    /// Complete project document at this revision.
    pub document: CharmeDocument,
    /// Current project file path, if the document has been saved or opened.
    pub project_path: Option<PathBuf>,
    /// Current revision.
    pub revision: u64,
    /// Whether the document differs from the most recently saved state.
    pub dirty: bool,
}

/// Mutable application session shared semantically by all platform frontends.
#[derive(Debug)]
pub struct EditorSession {
    document: CharmeDocument,
    saved_document: CharmeDocument,
    project_path: Option<PathBuf>,
    revision: u64,
    undo_stack: Vec<CharmeDocument>,
    redo_stack: Vec<CharmeDocument>,
}

impl EditorSession {
    /// Creates a new unsaved project.
    pub fn new(name: impl Into<String>) -> Self {
        let document = CharmeDocument::new(name);
        Self {
            saved_document: document.clone(),
            document,
            project_path: None,
            revision: 0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }

    /// Creates a session around a validated document.
    pub fn from_document(
        document: CharmeDocument,
        project_path: Option<PathBuf>,
    ) -> Result<Self, EditorError> {
        document.validate()?;
        Ok(Self {
            saved_document: document.clone(),
            document,
            project_path,
            revision: 0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        })
    }

    /// Borrows the current document.
    pub const fn document(&self) -> &CharmeDocument {
        &self.document
    }

    /// Returns the current project file path.
    pub fn project_path(&self) -> Option<&std::path::Path> {
        self.project_path.as_deref()
    }

    /// Returns the current revision.
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Returns true when the document differs from the most recently saved state.
    pub fn is_dirty(&self) -> bool {
        self.document != self.saved_document
    }

    /// Returns true when an edit can be undone.
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    /// Returns true when an undone edit can be reapplied.
    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    /// Produces an owned, read-only UI snapshot.
    pub fn snapshot(&self) -> EditorSnapshot {
        EditorSnapshot {
            document: self.document.clone(),
            project_path: self.project_path.clone(),
            revision: self.revision,
            dirty: self.is_dirty(),
        }
    }

    /// Applies one semantic command transactionally.
    ///
    /// `Ok(None)` means that the command did not alter the document.
    pub fn apply(&mut self, command: EditorCommand) -> Result<Option<EditorEvent>, EditorError> {
        let previous = self.document.clone();
        let change = apply_command(&mut self.document, command)?;
        if self.document == previous {
            return Ok(None);
        }
        if let Err(error) = self.document.validate() {
            self.document = previous;
            return Err(error.into());
        }
        let Some(revision) = self.revision.checked_add(1) else {
            self.document = previous;
            return Err(EditorError::RevisionExhausted);
        };
        self.undo_stack.push(previous);
        self.redo_stack.clear();
        self.revision = revision;
        Ok(Some(EditorEvent { revision, change }))
    }

    /// Restores the document state before the latest edit.
    pub fn undo(&mut self) -> Result<Option<EditorEvent>, EditorError> {
        let Some(previous) = self.undo_stack.pop() else {
            return Ok(None);
        };
        let Some(revision) = self.revision.checked_add(1) else {
            self.undo_stack.push(previous);
            return Err(EditorError::RevisionExhausted);
        };
        self.redo_stack
            .push(std::mem::replace(&mut self.document, previous));
        self.revision = revision;
        Ok(Some(EditorEvent {
            revision,
            change: DocumentChange::Everything,
        }))
    }

    /// Reapplies the latest undone edit.
    pub fn redo(&mut self) -> Result<Option<EditorEvent>, EditorError> {
        let Some(next) = self.redo_stack.pop() else {
            return Ok(None);
        };
        let Some(revision) = self.revision.checked_add(1) else {
            self.redo_stack.push(next);
            return Err(EditorError::RevisionExhausted);
        };
        self.undo_stack
            .push(std::mem::replace(&mut self.document, next));
        self.revision = revision;
        Ok(Some(EditorEvent {
            revision,
            change: DocumentChange::Everything,
        }))
    }

    pub(crate) fn mark_saved(&mut self, path: PathBuf) {
        self.project_path = Some(path);
        self.saved_document = self.document.clone();
    }
}

fn apply_command(
    document: &mut CharmeDocument,
    command: EditorCommand,
) -> Result<DocumentChange, EditorError> {
    match command {
        EditorCommand::RenameDocument(name) => {
            document.name = name;
            Ok(DocumentChange::Metadata)
        }
        EditorCommand::SetCharacter(character) => {
            document.character = character;
            Ok(DocumentChange::Character)
        }
        EditorCommand::UpsertShader(shader) => {
            upsert(&mut document.shaders, shader, |shader| shader.id);
            Ok(DocumentChange::Shaders)
        }
        EditorCommand::RemoveShader(id) => {
            remove(&mut document.shaders, id, |shader| shader.id)
                .ok_or(EditorError::UnknownShader(id))?;
            Ok(DocumentChange::Shaders)
        }
        EditorCommand::RenameShader { shader, name } => {
            find_shader_mut(document, shader)?.name = name;
            Ok(DocumentChange::Shaders)
        }
        EditorCommand::SetShaderPath { shader, path } => {
            find_shader_mut(document, shader)?.path = path;
            Ok(DocumentChange::Shaders)
        }
        EditorCommand::UpsertMaterial(material) => {
            upsert(&mut document.materials, material, |material| material.id);
            Ok(DocumentChange::Materials)
        }
        EditorCommand::RemoveMaterial(id) => {
            remove(&mut document.materials, id, |material| material.id)
                .ok_or(EditorError::UnknownMaterial(id))?;
            Ok(DocumentChange::Materials)
        }
        EditorCommand::RenameMaterial { material, name } => {
            find_material_mut(document, material)?.name = name;
            Ok(DocumentChange::Materials)
        }
        EditorCommand::SetMaterialShader { material, shader } => {
            find_material_mut(document, material)?.shader = shader;
            Ok(DocumentChange::Materials)
        }
        EditorCommand::ReplaceMaterialSlots(slots) => {
            document.material_slots = slots;
            Ok(DocumentChange::MaterialSlots)
        }
        EditorCommand::BindMaterial { slot, material } => {
            let slot = document
                .material_slots
                .iter_mut()
                .find(|candidate| candidate.id == slot)
                .ok_or(EditorError::UnknownMaterialSlot(slot))?;
            slot.material = material;
            Ok(DocumentChange::MaterialSlots)
        }
        EditorCommand::SetMaterialParameter {
            material,
            path,
            value,
        } => {
            let material = find_material_mut(document, material)?;
            set_map_value(&mut material.parameters, path, value);
            Ok(DocumentChange::Materials)
        }
        EditorCommand::SetMaterialTexture {
            material,
            path,
            texture,
        } => {
            let material = find_material_mut(document, material)?;
            set_map_value(&mut material.textures, path, texture);
            Ok(DocumentChange::Materials)
        }
        EditorCommand::SetMaterialRenderState { material, state } => {
            find_material_mut(document, material)?.render_state = state;
            Ok(DocumentChange::Materials)
        }
    }
}

fn find_shader_mut(
    document: &mut CharmeDocument,
    id: ShaderId,
) -> Result<&mut ShaderSource, EditorError> {
    document
        .shaders
        .iter_mut()
        .find(|shader| shader.id == id)
        .ok_or(EditorError::UnknownShader(id))
}

fn find_material_mut(
    document: &mut CharmeDocument,
    id: MaterialId,
) -> Result<&mut MaterialInstance, EditorError> {
    document
        .materials
        .iter_mut()
        .find(|material| material.id == id)
        .ok_or(EditorError::UnknownMaterial(id))
}

fn upsert<T, Id: Eq>(items: &mut Vec<T>, value: T, id: impl Fn(&T) -> Id) {
    if let Some(index) = items.iter().position(|item| id(item) == id(&value)) {
        items[index] = value;
    } else {
        items.push(value);
    }
}

fn remove<T, Id: Eq>(items: &mut Vec<T>, expected: Id, id: impl Fn(&T) -> Id) -> Option<T> {
    items
        .iter()
        .position(|item| id(item) == expected)
        .map(|index| items.remove(index))
}

fn set_map_value<T>(map: &mut BTreeMap<String, T>, path: String, value: Option<T>) {
    if let Some(value) = value {
        map.insert(path, value);
    } else {
        map.remove(&path);
    }
}

/// A rejected editor command or invalid loaded document.
#[derive(Clone, Debug, Error, PartialEq)]
pub enum EditorError {
    /// The command references an unknown shader.
    #[error("unknown shader {0}")]
    UnknownShader(ShaderId),
    /// The command references an unknown material.
    #[error("unknown material {0}")]
    UnknownMaterial(MaterialId),
    /// The command references an unknown material slot.
    #[error("unknown material slot {0}")]
    UnknownMaterialSlot(MaterialSlotId),
    /// Applying the command would make the document invalid.
    #[error(transparent)]
    InvalidDocument(#[from] DocumentValidationError),
    /// The session revision counter overflowed.
    #[error("editor revision counter exhausted")]
    RevisionExhausted,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MaterialSlot, ResourcePath};

    fn session_with_material() -> (EditorSession, MaterialId, MaterialSlotId) {
        let mut session = EditorSession::new("Character");
        let shader = ShaderSource::new(
            "Toon",
            ResourcePath::project_relative("shaders/toon.wgsl").unwrap(),
        );
        let material = MaterialInstance::new("Body", shader.id());
        let slot = MaterialSlot::new(0, "Body", "Body");
        let material_id = material.id();
        let slot_id = slot.id();
        session.apply(EditorCommand::UpsertShader(shader)).unwrap();
        session
            .apply(EditorCommand::UpsertMaterial(material))
            .unwrap();
        session
            .apply(EditorCommand::ReplaceMaterialSlots(vec![slot]))
            .unwrap();
        (session, material_id, slot_id)
    }

    #[test]
    fn commands_update_revision_dirty_state_and_snapshot() {
        let (mut session, material, slot) = session_with_material();
        assert!(session.is_dirty());

        let event = session
            .apply(EditorCommand::BindMaterial {
                slot,
                material: Some(material),
            })
            .unwrap()
            .unwrap();
        assert_eq!(event.change, DocumentChange::MaterialSlots);
        assert_eq!(
            session.document().material_slot(slot).unwrap().material(),
            Some(material)
        );

        let snapshot = session.snapshot();
        assert_eq!(snapshot.revision, session.revision());
        assert!(snapshot.dirty);
    }

    #[test]
    fn invalid_commands_roll_back_the_document() {
        let (mut session, _, slot) = session_with_material();
        let before = session.snapshot();
        let missing = MaterialId::new();

        assert!(matches!(
            session.apply(EditorCommand::BindMaterial {
                slot,
                material: Some(missing),
            }),
            Err(EditorError::InvalidDocument(
                DocumentValidationError::MissingBoundMaterial { .. }
            ))
        ));
        assert_eq!(session.snapshot(), before);
    }

    #[test]
    fn applying_the_same_value_is_a_noop() {
        let mut session = EditorSession::new("Character");
        assert_eq!(
            session
                .apply(EditorCommand::RenameDocument("Character".to_owned()))
                .unwrap(),
            None
        );
        assert!(!session.is_dirty());
    }

    #[test]
    fn undo_and_redo_restore_dirty_state() {
        let mut session = EditorSession::new("Character");
        session
            .apply(EditorCommand::RenameDocument(
                "Character Lookdev".to_owned(),
            ))
            .unwrap();
        assert!(session.can_undo());
        assert!(session.is_dirty());

        let undo = session.undo().unwrap().unwrap();
        assert_eq!(undo.change, DocumentChange::Everything);
        assert_eq!(session.document().name(), "Character");
        assert!(!session.is_dirty());
        assert!(session.can_redo());

        session.redo().unwrap();
        assert_eq!(session.document().name(), "Character Lookdev");
        assert!(session.is_dirty());
    }
}
